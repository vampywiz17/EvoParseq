use crate::account::{self, Session};
use jni::{
    objects::{JByteArray, JObject, JString, JValue},
    sys::jstring,
    JNIEnv,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use zeroize::Zeroizing;

// Account/stream JNI commands run on the view model worker. Controller calls
// only enqueue bounded input or register the platform output callback.
// Only one foreground account is supported. Tokens cross JNI only as transient
// byte arrays for the Android Keystore adapter, never UI strings or diagnostics.
static SESSION: Mutex<Option<Arc<Session>>> = Mutex::new(None);

fn reply(env: &mut JNIEnv<'_>, value: Value) -> jstring {
    match env.new_string(value.to_string()) {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(), // JNI leaves the allocation exception pending.
    }
}

fn text(env: &mut JNIEnv<'_>, value: &JString<'_>) -> jni::errors::Result<String> {
    Ok(env.get_string(value)?.into())
}

#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_signIn(
    mut env: JNIEnv,
    _object: JObject,
    email: JString,
    password: JString,
    tfa: JString,
) -> jstring {
    let result = (|| -> anyhow::Result<Value> {
        let login = account::Login {
            email: text(&mut env, &email)?,
            password: Zeroizing::new(text(&mut env, &password)?),
            tfa: Zeroizing::new(text(&mut env, &tfa)?),
        };
        let mut slot = SESSION
            .lock()
            .map_err(|_| anyhow::anyhow!("Account unavailable."))?;
        anyhow::ensure!(slot.is_none(), "Sign out before signing in again.");
        *slot = Some(Arc::new(account::login(&login)?));
        Ok(json!({"ok":true}))
    })();
    reply(
        &mut env,
        result.unwrap_or_else(|e| json!({"error":e.to_string()})),
    )
}

#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_rememberSession(
    mut env: JNIEnv,
    _object: JObject,
    vault: JObject,
) -> jstring {
    let result = (|| -> anyhow::Result<Value> {
        let slot = SESSION
            .lock()
            .map_err(|_| anyhow::anyhow!("Account unavailable"))?;
        let session = slot
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Sign in first"))?;
        let token = env.byte_array_from_slice(session.token().as_bytes())?;
        env.call_method(&vault, "save", "([B)V", &[JValue::Object(token.as_ref())])?;
        session.remembered();
        Ok(json!({"ok":true}))
    })();
    // Clear pending platform exceptions; never return their text or token data.
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    reply(&mut env, result.unwrap_or_else(|_| json!({"error":"Signed in, but secure storage failed. Sign-in will not be remembered."})))
}

#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_restoreSession(
    mut env: JNIEnv,
    _object: JObject,
    token: JByteArray,
) -> jstring {
    let result = (|| -> anyhow::Result<Value> {
        anyhow::ensure!(
            env.get_array_length(&token)? <= 4096,
            "Invalid saved sign-in"
        );
        let bytes = Zeroizing::new(env.convert_byte_array(&token)?);
        let session = Session::from_token(std::str::from_utf8(&bytes)?)?;
        let mut slot = SESSION
            .lock()
            .map_err(|_| anyhow::anyhow!("Account unavailable"))?;
        anyhow::ensure!(slot.is_none(), "Already signed in");
        *slot = Some(Arc::new(session));
        Ok(json!({"ok":true}))
    })();
    reply(
        &mut env,
        result.unwrap_or_else(|_| json!({"error":"Saved sign-in unavailable. Sign in again."})),
    )
}

#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_hosts(
    mut env: JNIEnv,
    _object: JObject,
) -> jstring {
    let result = (|| -> anyhow::Result<Value> {
        let slot = SESSION
            .lock()
            .map_err(|_| anyhow::anyhow!("Account unavailable."))?;
        let session = slot
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Sign in first."))?;
        let hosts = account::hosts(session)?;
        Ok(
            json!({"ok":true,"hosts":hosts.iter().map(|h| json!({"peer":h.peer,"name":h.name})).collect::<Vec<_>>()}),
        )
    })();
    reply(
        &mut env,
        result.unwrap_or_else(|e| json!({"error":e.to_string(),"expired":e.downcast_ref::<account::ExpiredSession>().is_some()})),
    )
}

#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_signOut(_env: JNIEnv, _object: JObject) {
    stop_setup();
    crate::stream::stop();
    if let Ok(mut slot) = SESSION.lock() {
        // Explicit logout revokes remembered sessions too; process exit does not.
        if let Some(session) = slot.take() {
            session.revoke();
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_startStream(
    mut env: JNIEnv,
    _object: JObject,
    surface: JObject,
    peer: JString,
    preferences: JString,
    choices: JString,
    width: jni::sys::jint,
    height: jni::sys::jint,
) -> jstring {
    let result = (|| -> anyhow::Result<Value> {
        stop_setup();
        let session = SESSION
            .lock()
            .map_err(|_| anyhow::anyhow!("Account unavailable"))?
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Sign in first"))?;
        let peer = text(&mut env, &peer)?;
        let prefs = crate::preferences::Preferences::parse(&text(&mut env, &preferences)?)?;
        let choices = text(&mut env, &choices)?;
        anyhow::ensure!(choices.len() <= 4096, "Decoder choices exceed limit");
        let choices: crate::video_android::Choices = serde_json::from_str(&choices)?;
        let size = (u32::try_from(width)?, u32::try_from(height)?);
        anyhow::ensure!(
            (1..=16384).contains(&size.0) && (1..=16384).contains(&size.1),
            "Invalid Surface size"
        );
        // Acquires a reference independent of this JNI local object. The decoder
        // worker retains it through MediaCodec stop/delete, even on UI teardown.
        let window = unsafe {
            ndk_sys::ANativeWindow_fromSurface(env.get_raw().cast(), surface.as_raw().cast())
        };
        anyhow::ensure!(!window.is_null(), "Video Surface unavailable");
        crate::stream::start(
            session,
            peer,
            prefs,
            crate::video_android::NativeWindow(window),
            choices,
            size,
        )?;
        Ok(json!({"ok":true}))
    })();
    reply(
        &mut env,
        result.unwrap_or_else(|e| json!({"error":e.to_string()})),
    )
}

#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_stopStream(_env: JNIEnv, _object: JObject) {
    stop_setup();
    crate::stream::stop();
}

#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_streamStatus(
    mut env: JNIEnv,
    _object: JObject,
) -> jstring {
    reply(&mut env, crate::stream::status())
}

#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_settings(
    mut env: JNIEnv,
    _object: JObject,
    current: JString,
    field: JString,
    index: jni::sys::jint,
) -> jstring {
    let result = (|| -> anyhow::Result<Value> {
        let current = text(&mut env, &current)?;
        let mut value = if current.is_empty() {
            crate::preferences::Preferences::default()
        } else {
            crate::preferences::Preferences::parse(&current)?
        };
        let field = text(&mut env, &field)?;
        if !field.is_empty() {
            value.select(&field, usize::try_from(index)?)?;
        }
        Ok(value.view())
    })();
    // Local malformed settings must not leak their raw content to the screen.
    reply(
        &mut env,
        result.unwrap_or_else(|_| json!({"error":"Could not read or change TV settings."})),
    )
}

#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_releaseSession(
    _env: JNIEnv,
    _object: JObject,
) {
    stop_setup();
    crate::stream::stop();
    if let Ok(mut slot) = SESSION.lock() {
        drop(slot.take());
    }
}

static LOCAL_SETUP: Mutex<Option<crate::local_setup::Setup>> = Mutex::new(None);
fn stop_setup() {
    if let Some(mut setup) = LOCAL_SETUP.lock().unwrap_or_else(|e| e.into_inner()).take() {
        setup.stop();
    }
}

#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_startLocalSetup(
    mut env: JNIEnv,
    _object: JObject,
    ip: JString,
) -> jstring {
    let result = (|| -> anyhow::Result<Value> {
        anyhow::ensure!(
            !crate::stream::active(),
            "Disconnect the stream before local setup"
        );
        stop_setup();
        let setup = crate::local_setup::Setup::start(text(&mut env, &ip)?.parse()?)?;
        let report = setup.snapshot();
        *LOCAL_SETUP.lock().unwrap_or_else(|e| e.into_inner()) = Some(setup);
        Ok(report)
    })();
    reply(&mut env,result.unwrap_or_else(|_|json!({"error":"Could not start local HTTPS setup. Connect the TV to a private LAN and try again."})))
}
#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_localSetupStatus(
    mut env: JNIEnv,
    _object: JObject,
) -> jstring {
    let report = LOCAL_SETUP
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|s| s.snapshot())
        .unwrap_or_else(|| json!({"closed":true}));
    reply(&mut env, report)
}
#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_stopLocalSetup(
    _env: JNIEnv,
    _object: JObject,
) {
    stop_setup();
}
#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_confirmLocalSetup(
    mut env: JNIEnv,
    _object: JObject,
    vault: JObject,
) -> jstring {
    let result = (|| -> anyhow::Result<Value> {
        let network = LOCAL_SETUP
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|s| s.pending())
            .ok_or_else(|| anyhow::anyhow!("No pending network settings"))?;
        let raw = Zeroizing::new(serde_json::to_vec(&network)?);
        let bytes = env.byte_array_from_slice(&raw)?;
        env.call_method(&vault, "save", "([B)V", &[JValue::Object(bytes.as_ref())])?;
        let report = json!({"ok":true,"network":network.summary()});
        *crate::network::CURRENT
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(network));
        stop_setup();
        Ok(report)
    })();
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    reply(
        &mut env,
        result.unwrap_or_else(
            |_| json!({"error":"Could not securely save network settings. Nothing was applied."}),
        ),
    )
}
#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_restoreNetwork(
    mut env: JNIEnv,
    _object: JObject,
    bytes: JByteArray,
) -> jstring {
    let result = (|| -> anyhow::Result<Value> {
        anyhow::ensure!(
            env.get_array_length(&bytes)? <= 32768,
            "Network settings exceed limit"
        );
        let raw = Zeroizing::new(env.convert_byte_array(&bytes)?);
        let n = crate::network::Network::parse(&raw)?;
        let report = json!({"ok":true,"network":n.summary()});
        *crate::network::CURRENT
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(n));
        Ok(report)
    })();
    reply(
        &mut env,
        result.unwrap_or_else(
            |_| json!({"error":"Saved network settings unavailable. Configure the relay again."}),
        ),
    )
}
#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_resetNetwork(
    mut env: JNIEnv,
    _object: JObject,
) -> jstring {
    stop_setup();
    *crate::network::CURRENT
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = None;
    reply(
        &mut env,
        json!({"ok":true,"network":crate::network::current().summary()}),
    )
}

#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_setOverlayVisible(
    _env: JNIEnv,
    _object: JObject,
    visible: jni::sys::jboolean,
) {
    crate::overlay::REQUESTED.store(visible != 0, std::sync::atomic::Ordering::Relaxed);
}

#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_controllerAdapter(
    env: JNIEnv,
    _object: JObject,
    adapter: JObject,
) {
    if crate::gamepad_android::bind(&env, adapter).is_err() {
        let _ = env.exception_clear();
    }
}
#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_controllerFocus(
    _env: JNIEnv,
    _object: JObject,
    active: jni::sys::jboolean,
) {
    crate::gamepad_android::focus(active != 0);
}
#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_controllerButton(
    _env: JNIEnv,
    _object: JObject,
    id: i32,
    code: i32,
    down: jni::sys::jboolean,
) -> jni::sys::jboolean {
    u8::from(crate::gamepad_android::button(id, code, down != 0))
}
#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_controllerRemoved(
    _env: JNIEnv,
    _object: JObject,
    id: i32,
) {
    REMOTE_MOUSE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(id);
    crate::stream::controller_event(crate::gamepad::Event::Remove(id));
}
#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_controllerAxes(
    _env: JNIEnv,
    _object: JObject,
    id: i32,
    x: f32,
    y: f32,
    rx: f32,
    ry: f32,
    lt: f32,
    rt: f32,
    hx: f32,
    hy: f32,
    fx: f32,
    fy: f32,
    frx: f32,
    fry: f32,
    flt: f32,
    frt: f32,
) {
    crate::gamepad_android::axes(
        id,
        [x, y, rx, ry, lt, rt, hx, hy],
        [fx, fy, frx, fry, flt, frt],
    );
}

static REMOTE_MOUSE: std::sync::LazyLock<std::sync::Mutex<crate::desktop_input::RemoteSource>> =
    std::sync::LazyLock::new(Default::default);
#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_remoteMouseMode(
    _env: JNIEnv,
    _o: JObject,
    on: jni::sys::jboolean,
    long_ms: i32,
    device: i32,
) {
    REMOTE_MOUSE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .configure(on != 0, (device >= 0).then_some(device));
    crate::stream::input_event(crate::desktop_input::Event::RemoteMode(
        on != 0,
        long_ms.max(0) as u32,
    ));
}
#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_remoteMouseKey(
    _env: JNIEnv,
    _o: JObject,
    code: i32,
    down: jni::sys::jboolean,
    cancel: jni::sys::jboolean,
    device: i32,
    directional_source: jni::sys::jboolean,
) -> jni::sys::jboolean {
    if !crate::gamepad_android::ACTIVE.load(std::sync::atomic::Ordering::Acquire)
        || !REMOTE_MOUSE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .accepts(device, directional_source != 0)
    {
        return 0;
    }
    let key = match code {
        19 => 0,
        20 => 1,
        21 => 2,
        22 => 3,
        23 | 66 | 160 => 4,
        _ => return 0,
    };
    crate::stream::input_event(crate::desktop_input::Event::RemoteKey {
        key,
        down: down != 0,
        cancel: cancel != 0,
    });
    1
}
#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_desktopKey(
    _env: JNIEnv,
    _o: JObject,
    scan: i32,
    code: i32,
    meta: i32,
    down: jni::sys::jboolean,
    cancel: jni::sys::jboolean,
    repeat: i32,
) -> i32 {
    if !crate::gamepad_android::ACTIVE.load(std::sync::atomic::Ordering::Acquire) {
        return 0;
    }
    if cancel != 0 {
        crate::stream::input_event(crate::desktop_input::Event::Release);
        return 1;
    }
    let Some(usage) = crate::keyboard::usage(scan, code) else {
        return 0;
    };
    let mods = crate::keyboard::modifiers(meta);
    let shortcut = match usage {
        16 => Some("KeyM"),
        18 => Some("KeyO"),
        21 => Some("KeyR"),
        _ => None,
    }
    .and_then(|k| crate::input::shortcut(k, mods));
    if down != 0
        && matches!(
            shortcut,
            Some(crate::input::Shortcut::Menu | crate::input::Shortcut::ReleaseInput)
        )
    {
        if repeat != 0 {
            return 1;
        }
        crate::stream::input_event(crate::desktop_input::Event::Release);
        return 2;
    }
    crate::stream::input_event(crate::desktop_input::Event::Key {
        usage,
        mods,
        down: down != 0,
    });
    1
}
#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_desktopViewport(
    _env: JNIEnv,
    _o: JObject,
    w: i32,
    h: i32,
) {
    crate::stream::input_event(crate::desktop_input::Event::Viewport(
        w.max(0) as u32,
        h.max(0) as u32,
    ));
}
#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_desktopRelease(_env: JNIEnv, _o: JObject) {
    crate::stream::input_event(crate::desktop_input::Event::Release);
}
#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_desktopPointer(
    _env: JNIEnv,
    _o: JObject,
    x: f64,
    y: f64,
    relative: jni::sys::jboolean,
    moved: jni::sys::jboolean,
    w: i32,
    h: i32,
    buttons: i32,
    wx: i32,
    wy: i32,
) {
    if !crate::gamepad_android::ACTIVE.load(std::sync::atomic::Ordering::Acquire) {
        return;
    }
    crate::stream::input_event(crate::desktop_input::Event::Pointer(
        crate::desktop_input::Pointer {
            x,
            y,
            relative: relative != 0,
            moved: moved != 0,
            viewport: (w.max(0) as u32, h.max(0) as u32),
            buttons: buttons as u32,
            wheel: (wx, wy),
        },
    ));
}

// Runs only on the existing TV account worker; never on a media callback.
static UPDATE: Mutex<Option<crate::release_apk::Candidate>> = Mutex::new(None);
#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_checkUpdate(
    mut env: JNIEnv,
    _object: JObject,
    installed: JString,
    prerelease: jni::sys::jboolean,
) -> jstring {
    let result = (|| -> anyhow::Result<Value> {
        *UPDATE.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let candidate = crate::release_apk::check(&text(&mut env, &installed)?, prerelease != 0)?;
        let value = json!({"ok":true,"release":candidate});
        *UPDATE.lock().unwrap_or_else(|e| e.into_inner()) = candidate;
        Ok(value)
    })();
    reply(
        &mut env,
        result.unwrap_or_else(
            |_| json!({"ok":false,"error":"Could not check GitHub releases. Try again later."}),
        ),
    )
}
#[no_mangle]
pub extern "system" fn Java_app_evoparseq_tv_NativeCore_downloadUpdate(
    mut env: JNIEnv,
    _object: JObject,
    destination: JString,
) -> jstring {
    let result = (|| -> anyhow::Result<Value> {
        let candidate = UPDATE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Check for updates first"))?;
        crate::release_apk::download(
            &candidate,
            std::path::Path::new(&text(&mut env, &destination)?),
        )?;
        Ok(json!({"ok":true,"version":candidate.version}))
    })();
    reply(&mut env,result.unwrap_or_else(|_|json!({"ok":false,"error":"APK download or integrity check failed. Check for updates and retry."})))
}
