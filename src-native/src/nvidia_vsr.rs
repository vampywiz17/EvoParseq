//! Optional RTX Video SDK 1.1 D3D11 adapter. No CPU pixel access.
//! The public C ABI is bound directly; no NVIDIA sample implementation is copied.
use windows::Win32::Graphics::{Direct3D11::*, Dxgi::Common::*};

pub struct Surfaces {
    pub input: ID3D11Texture2D,
    pub output: ID3D11Texture2D,
    clear: ID3D11RenderTargetView,
    key: (u32, u32, u32, u32, DXGI_FORMAT),
}

impl Surfaces {
    unsafe fn new(
        device: &ID3D11Device,
        source: (u32, u32),
        target: (u32, u32),
        format: DXGI_FORMAT,
    ) -> anyhow::Result<Self> {
        let texture = |size: (u32, u32), flags: u32| -> windows::core::Result<ID3D11Texture2D> {
            let mut resource = None;
            device.CreateTexture2D(
                &D3D11_TEXTURE2D_DESC {
                    Width: size.0,
                    Height: size.1,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: format,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: flags,
                    ..Default::default()
                },
                None,
                Some(&mut resource),
            )?;
            resource.ok_or_else(windows::core::Error::from_thread)
        };
        let input = texture(
            source,
            (D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE).0 as u32,
        )?;
        let output = texture(
            target,
            (D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_UNORDERED_ACCESS).0
                as u32,
        )?;
        let mut clear = None;
        device.CreateRenderTargetView(&output, None, Some(&mut clear))?;
        Ok(Self {
            input,
            output,
            clear: clear.ok_or_else(windows::core::Error::from_thread)?,
            key: (source.0, source.1, target.0, target.1, format),
        })
    }
}

pub struct Vsr {
    sdk: sdk::Instance,
    surfaces: Option<Surfaces>,
    quality: u8,
    evaluated: bool,
}
impl Vsr {
    pub unsafe fn new(device: &ID3D11Device, quality: u8) -> anyhow::Result<Self> {
        anyhow::ensure!((1..=4).contains(&quality), "Invalid RTX VSR quality");
        Ok(Self {
            sdk: sdk::Instance::new(device)?,
            surfaces: None,
            quality,
            evaluated: false,
        })
    }
    /// One source-sized conversion target and one UAV output, reused across frames.
    /// The caller skips VSR when the viewport does not upscale in both dimensions.
    pub unsafe fn prepare(
        &mut self,
        device: &ID3D11Device,
        source: (u32, u32),
        target: (u32, u32),
        format: DXGI_FORMAT,
    ) -> anyhow::Result<&Surfaces> {
        let key = (source.0, source.1, target.0, target.1, format);
        if self.surfaces.as_ref().is_none_or(|s| s.key != key) {
            self.surfaces = Some(Surfaces::new(device, source, target, format)?);
        }
        Ok(self.surfaces.as_ref().unwrap())
    }
    pub fn warming(&self) -> bool {
        !self.evaluated
    }
    pub unsafe fn evaluate(
        &mut self,
        context: &ID3D11DeviceContext,
        target: [i32; 4],
    ) -> anyhow::Result<()> {
        let surfaces = self
            .surfaces
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("VSR surfaces missing"))?;
        context.ClearRenderTargetView(&surfaces.clear, &[0.0, 0.0, 0.0, 1.0]);
        let result = self.sdk.evaluate(context, surfaces, target, self.quality);
        self.evaluated = result.is_ok();
        result
    }
}

mod sdk {
    use super::*;
    use std::{
        ffi::{c_char, c_void, CStr, OsString},
        os::windows::ffi::{OsStrExt, OsStringExt},
        path::{Path, PathBuf},
        ptr::null_mut,
        sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use windows::{core::Interface, Win32::Graphics::Direct3D10::ID3D10Multithread};

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryExW(path: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
        fn FreeLibrary(module: *mut c_void) -> i32;
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
        fn GetModuleFileNameW(module: *mut c_void, path: *mut u16, length: u32) -> u32;
    }
    const LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR: u32 = 0x100;
    const LOAD_LIBRARY_SEARCH_DEFAULT_DIRS: u32 = 0x1000;

    // NVIDIA NGX public API uses a cdecl log callback on Windows x64.
    // It can be called from arbitrary SDK threads. The event logger is bounded
    // and asynchronous; never block GPU threads on file I/O.
    type SdkLogCallback = unsafe extern "C" fn(*const c_char, u32, u32);
    static NGX_INIT_LOG_ACTIVE: AtomicBool = AtomicBool::new(false);
    static NGX_INIT_LOG_COUNT: AtomicUsize = AtomicUsize::new(0);
    const NGX_INIT_LOG_LIMIT: usize = 48;

    unsafe extern "C" fn ngx_init_log(message: *const c_char, level: u32, component: u32) {
        if message.is_null()
            || !NGX_INIT_LOG_ACTIVE.load(Ordering::Acquire)
            || !crate::event_log::enabled()
            || NGX_INIT_LOG_COUNT.fetch_add(1, Ordering::Relaxed) >= NGX_INIT_LOG_LIMIT
        {
            return;
        }
        // Callbacks may run on NVIDIA worker threads. Do not unwind across FFI.
        let _ = std::panic::catch_unwind(|| {
            let raw = unsafe { CStr::from_ptr(message) }.to_string_lossy();
            let snippet: String = raw
                .chars()
                .filter(|ch| !ch.is_control() || *ch == ' ')
                .take(320)
                .collect();
            crate::event_log::record(
                "INFO",
                "video_ngx_sdk_log",
                serde_json::json!({"level":level,"component":component,"message":snippet}),
            );
        });
    }

    /// Query already-loaded modules only. Never preload a DLL merely to probe it.
    /// Report path equality, not potentially private filesystem paths.
    unsafe fn module_state(name: &str, expected: Option<&Path>) -> (bool, Option<bool>) {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let handle = GetModuleHandleW(wide.as_ptr());
        if handle.is_null() {
            return (false, None);
        }
        let Some(expected) = expected else {
            return (true, None);
        };
        let mut filename = [0u16; 1024];
        let count =
            GetModuleFileNameW(handle, filename.as_mut_ptr(), filename.len() as u32) as usize;
        if count == 0 || count >= filename.len() {
            return (true, None);
        }
        let found = PathBuf::from(OsString::from_wide(&filename[..count]));
        let matches = match (
            std::fs::canonicalize(found),
            std::fs::canonicalize(expected),
        ) {
            (Ok(a), Ok(b)) => Some(a == b),
            _ => None,
        };
        (true, matches)
    }

    unsafe fn log_module_probe(root: &Path, phase: &'static str) {
        if !crate::event_log::enabled() {
            return;
        }
        let (runtime_loaded, runtime_is_verified) =
            module_state("nvngx_vsr.dll", Some(&root.join("nvngx_vsr.dll")));
        let (core_loaded, _) = module_state("_nvngx.dll", None);
        crate::event_log::record(
            "INFO",
            "video_ngx_module_probe",
            serde_json::json!({
                "phase":phase,
                "runtime_loaded":runtime_loaded,
                "runtime_matches_verified_file":runtime_is_verified,
                "ngx_driver_core_loaded":core_loaded,
                "search_directory":"verified_user_vsr_cache",
                "bridge_abi":3,
                "sdk_version":"1.1.0",
                "ngx_api_version":"0x15"
            }),
        );
    }

    #[repr(C)]
    #[derive(Default)]
    struct CreateDiagnostics {
        stage: u32,
        sdk_result: u32,
        available: i32,
    }
    type Create = unsafe extern "C" fn(
        *mut c_void,
        *mut c_void,
        *const u16,
        *mut *mut c_void,
        *mut CreateDiagnostics,
        Option<SdkLogCallback>,
    ) -> u32;
    type Evaluate = unsafe extern "C" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        *const i32,
        u32,
    ) -> u32;
    type Destroy = unsafe extern "C" fn(*mut c_void);
    type Abi = unsafe extern "C" fn() -> u32;

    struct Library(*mut c_void);
    impl Drop for Library {
        fn drop(&mut self) {
            unsafe {
                FreeLibrary(self.0);
            }
        }
    }
    impl Library {
        unsafe fn open(path: &std::path::Path) -> anyhow::Result<Self> {
            let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let module = LoadLibraryExW(
                wide.as_ptr(),
                null_mut(),
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
            );
            anyhow::ensure!(
                !module.is_null(),
                "Cannot load the verified NVIDIA NGX bridge"
            );
            Ok(Self(module))
        }
        unsafe fn symbol<T: Copy>(&self, symbol: &'static [u8]) -> anyhow::Result<T> {
            let ptr = GetProcAddress(self.0, symbol.as_ptr());
            anyhow::ensure!(!ptr.is_null(), "NVIDIA NGX bridge ABI symbol missing");
            Ok(std::mem::transmute_copy(&ptr))
        }
    }

    struct ContextLock(ID3D10Multithread);
    impl ContextLock {
        unsafe fn enter(device: &ID3D11Device) -> anyhow::Result<Self> {
            let lock: ID3D10Multithread = device.cast()?;
            lock.Enter();
            Ok(Self(lock))
        }
    }
    impl Drop for ContextLock {
        fn drop(&mut self) {
            unsafe {
                self.0.Leave();
            }
        }
    }

    pub struct Instance {
        device: ID3D11Device,
        _library: Library,
        session: *mut c_void,
        evaluate: Evaluate,
        destroy: Destroy,
    }
    impl Instance {
        pub unsafe fn new(device: &ID3D11Device) -> anyhow::Result<Self> {
            let root = crate::vsr_runtime::validated_directory()?;
            let library = Library::open(&crate::vsr_runtime::validated_bridge_path()?)?;
            let abi: Abi = library.symbol(b"evoparseq_ngx_bridge_abi\0")?;
            anyhow::ensure!(abi() == 3, "Unsupported NVIDIA NGX bridge ABI");
            let create: Create = library.symbol(b"evoparseq_ngx_create\0")?;
            let evaluate: Evaluate = library.symbol(b"evoparseq_ngx_evaluate\0")?;
            let destroy: Destroy = library.symbol(b"evoparseq_ngx_destroy\0")?;
            let path: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
            let context = device.GetImmediateContext()?;
            let _lock = ContextLock::enter(device)?;
            log_module_probe(&root, "before_init");
            NGX_INIT_LOG_COUNT.store(0, Ordering::Relaxed);
            let logging = crate::event_log::enabled();
            NGX_INIT_LOG_ACTIVE.store(logging, Ordering::Release);
            let mut session = null_mut();
            let mut diagnostics = CreateDiagnostics::default();
            // Preserve the original direct-SDK init inputs: app=0,
            // API=0x15, D3D11 device/context and the same verified path list.
            let created = create(
                device.as_raw(),
                context.as_raw(),
                path.as_ptr(),
                &mut session,
                &mut diagnostics,
                if logging { Some(ngx_init_log) } else { None },
            );
            NGX_INIT_LOG_ACTIVE.store(false, Ordering::Release);
            log_module_probe(&root, "after_init");
            if created != 1 || session.is_null() {
                let stage = match diagnostics.stage {
                    1 => "invalid bridge ABI input",
                    2 => "NGX D3D11 initialization",
                    3 => "NGX D3D11 capability parameters",
                    4 => "NGX VSR availability query",
                    5 => "VSR.Available is false",
                    6 => "NGX VSR feature creation",
                    _ => "unknown NGX bridge initialization failure",
                };
                anyhow::bail!(
                    "NVIDIA NGX VSR failed at {stage}: status 0x{:08X}, available={}",
                    diagnostics.sdk_result,
                    diagnostics.available
                );
            }
            Ok(Self {
                device: device.clone(),
                _library: library,
                session,
                evaluate,
                destroy,
            })
        }
        pub unsafe fn evaluate(
            &self,
            context: &ID3D11DeviceContext,
            surfaces: &Surfaces,
            rect: [i32; 4],
            quality: u8,
        ) -> anyhow::Result<()> {
            let _lock = ContextLock::enter(&self.device)?;
            anyhow::ensure!(
                (self.evaluate)(
                    self.session,
                    context.as_raw(),
                    surfaces.input.as_raw(),
                    surfaces.output.as_raw(),
                    surfaces.key.0,
                    surfaces.key.1,
                    rect.as_ptr(),
                    u32::from(quality)
                ) == 1,
                "NVIDIA NGX VSR frame evaluation failed"
            );
            Ok(())
        }
    }
    impl Drop for Instance {
        fn drop(&mut self) {
            unsafe {
                let _lock = ContextLock::enter(&self.device).ok();
                (self.destroy)(self.session);
            }
        }
    }
}
