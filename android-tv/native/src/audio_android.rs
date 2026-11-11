//! AAudio is only the output adapter; PCM queues, buffering and retry policy are shared Rust.
use crate::{
    audio_layout::Layout,
    audio_preferences::{Inventory, Settings},
    audio_recovery::Recovery,
    audio_stream::{CallbackReader, NativeReceiver},
};
use anyhow::Result;
use ndk_sys as ndk;
use std::{
    cell::UnsafeCell,
    ffi::c_void,
    ptr,
    sync::{
        atomic::{AtomicBool, AtomicI32, Ordering},
        OnceLock,
    },
    time::Instant,
};

// These public symbols were added in API 32. Resolve them at runtime so the
// same APK continues to load on API 29-31 without a newer-symbol link dependency.
struct MaskApi {
    set: unsafe extern "C" fn(*mut ndk::AAudioStreamBuilder, u32),
    get: unsafe extern "C" fn(*mut ndk::AAudioStream) -> u32,
}
fn masks() -> Option<&'static MaskApi> {
    static API: OnceLock<Option<MaskApi>> = OnceLock::new();
    API.get_or_init(|| unsafe {
        let set = libc::dlsym(
            libc::RTLD_DEFAULT,
            c"AAudioStreamBuilder_setChannelMask".as_ptr(),
        );
        let get = libc::dlsym(libc::RTLD_DEFAULT, c"AAudioStream_getChannelMask".as_ptr());
        if set.is_null() || get.is_null() {
            None
        } else {
            Some(MaskApi {
                set: std::mem::transmute::<
                    *mut c_void,
                    unsafe extern "C" fn(*mut ndk::AAudioStreamBuilder, u32),
                >(set),
                get: std::mem::transmute::<
                    *mut c_void,
                    unsafe extern "C" fn(*mut ndk::AAudioStream) -> u32,
                >(get),
            })
        }
    })
    .as_ref()
}
struct Callback {
    reader: UnsafeCell<Option<CallbackReader>>,
    layout: Layout,
    minimum_frames: usize,
    stopping: AtomicBool,
    error: AtomicI32,
}
unsafe extern "C" fn data(
    _stream: *mut ndk::AAudioStream,
    user: *mut c_void,
    output: *mut c_void,
    frames: i32,
) -> i32 {
    if user.is_null() || output.is_null() || frames <= 0 {
        return ndk::AAUDIO_CALLBACK_RESULT_STOP as i32;
    }
    let state = &*user.cast::<Callback>();
    let Some(samples) = (frames as usize).checked_mul(usize::from(state.layout.channels)) else {
        return ndk::AAUDIO_CALLBACK_RESULT_STOP as i32;
    };
    let output = std::slice::from_raw_parts_mut(output.cast::<i16>(), samples);
    if state.stopping.load(Ordering::Acquire) {
        output.fill(0);
        return ndk::AAUDIO_CALLBACK_RESULT_STOP as i32;
    }
    // AAudio serializes data callbacks. The control thread accesses the reader
    // only before start or after a successful close: no live concurrent mutation.
    if let Some(reader) = (&mut *state.reader.get()).as_mut() {
        if reader
            .render_layout(output, state.layout, state.minimum_frames)
            .invalid_layout
        {
            state
                .error
                .store(ndk::AAUDIO_ERROR_INVALID_FORMAT, Ordering::Release);
            return ndk::AAUDIO_CALLBACK_RESULT_STOP as i32;
        }
    } else {
        output.fill(0);
    }
    ndk::AAUDIO_CALLBACK_RESULT_CONTINUE as i32
}
unsafe extern "C" fn error(_stream: *mut ndk::AAudioStream, user: *mut c_void, code: i32) {
    if let Some(state) = user.cast::<Callback>().as_ref() {
        state.error.store(code, Ordering::Release);
    }
}
pub struct Output {
    stream: *mut ndk::AAudioStream,
    callback: Option<Box<Callback>>,
    receiver: Option<NativeReceiver>,
    recovery: Recovery,
    next: Instant,
    restorations: u64,
    last_error: i32,
    negotiated: Layout,
}
#[derive(serde::Serialize)]
pub struct Snapshot {
    state: &'static str,
    recovering: bool,
    restorations: u64,
    last_error: i32,
    channels: u8,
    channel_mask: u32,
    negotiated_channels: u8,
    frames_written: i64,
    frames_read: i64,
    xruns: i32,
    buffer_frames: i32,
    capacity_frames: i32,
    device_id: i32,
}
impl Output {
    pub fn prepare(settings: Settings, inventory: &Inventory, bud: bool) -> Self {
        let mut output = Self {
            stream: ptr::null_mut(),
            callback: Some(Box::new(Callback {
                reader: UnsafeCell::new(None),
                layout: Layout::STEREO,
                minimum_frames: settings.minimum_frames(),
                stopping: AtomicBool::new(false),
                error: AtomicI32::new(0),
            })),
            receiver: None,
            recovery: Recovery::default(),
            next: Instant::now(),
            restorations: 0,
            last_error: 0,
            negotiated: Layout::STEREO,
        };
        if let Err(code) = output.open(Layout::STEREO) {
            output.failed(code);
            return output;
        }
        if bud && settings.surround && masks().is_some() {
            let device = unsafe { ndk::AAudioStream_getDeviceId(output.stream) };
            let layout = inventory.layout(device);
            if layout != Layout::STEREO && output.close() {
                if output.open(layout).is_ok()
                    && unsafe { ndk::AAudioStream_getDeviceId(output.stream) } == device
                {
                    output.negotiated = layout;
                } else if output.close() {
                    if let Err(code) = output.open(Layout::STEREO) {
                        output.failed(code);
                    }
                }
            }
        }
        output
    }
    pub fn layout(&self) -> Layout {
        self.negotiated
    }
    pub fn attach(&mut self, receiver: NativeReceiver) -> Result<()> {
        if let Some(callback) = self.callback.as_mut() {
            *callback.reader.get_mut() = Some(receiver.callback_reader()?);
        }
        self.receiver = Some(receiver);
        if !self.stream.is_null() && !self.recovery.recovering() {
            let code = unsafe { ndk::AAudioStream_requestStart(self.stream) };
            if code < 0 {
                self.failed(code);
            }
        }
        Ok(())
    }
    // Called on the existing session/control thread, never a real-time callback.
    pub fn tick(&mut self, now: Instant) {
        let Some(callback) = self.callback.as_ref() else {
            return;
        };
        let code = callback.error.swap(0, Ordering::AcqRel);
        if code != 0 {
            self.failed(code);
        }
        if !self.recovery.recovering() || now < self.next {
            return;
        }
        if !self.close() {
            return;
        }
        if let Some(receiver) = &self.receiver {
            receiver.discard_pending();
        }
        if let Some(reader) = self
            .callback
            .as_mut()
            .and_then(|c| c.reader.get_mut().as_mut())
        {
            reader.restart();
        }
        let mut result = self.open(self.negotiated);
        if result.is_err() && self.negotiated != Layout::STEREO && self.close() {
            result = self.open(Layout::STEREO);
        }
        if result.is_ok() {
            let code = unsafe { ndk::AAudioStream_requestStart(self.stream) };
            if code < 0 {
                result = Err(code);
            }
        }
        match result {
            Ok(()) => {
                self.restorations += 1;
                self.recovery.restored();
                crate::event_log::record(
                    "INFO",
                    "android_audio_restored",
                    serde_json::json!({"channels":self.callback.as_ref().unwrap().layout.channels}),
                );
            }
            Err(code) => {
                self.last_error = code;
                self.next = now + self.recovery.failed();
            }
        }
    }
    fn failed(&mut self, code: i32) {
        self.last_error = code;
        if self.recovery.begin() {
            self.recovery.endpoint_changed();
            crate::event_log::record(
                "WARN",
                "android_audio_recovering",
                serde_json::json!({"code":code}),
            );
        }
        self.next = Instant::now() + self.recovery.failed();
    }
    fn open(&mut self, layout: Layout) -> std::result::Result<(), i32> {
        let callback = self
            .callback
            .as_mut()
            .ok_or(ndk::AAUDIO_ERROR_INVALID_STATE)?;
        callback.layout = layout;
        callback.stopping.store(false, Ordering::Release);
        callback.error.store(0, Ordering::Release);
        unsafe {
            let mut builder = ptr::null_mut();
            let code = ndk::AAudio_createStreamBuilder(&mut builder);
            if code < 0 || builder.is_null() {
                return Err(code.min(-1));
            }
            ndk::AAudioStreamBuilder_setDirection(builder, ndk::AAUDIO_DIRECTION_OUTPUT as i32);
            ndk::AAudioStreamBuilder_setSharingMode(
                builder,
                ndk::AAUDIO_SHARING_MODE_SHARED as i32,
            );
            ndk::AAudioStreamBuilder_setPerformanceMode(
                builder,
                ndk::AAUDIO_PERFORMANCE_MODE_LOW_LATENCY as i32,
            );
            ndk::AAudioStreamBuilder_setUsage(builder, ndk::AAUDIO_USAGE_GAME as i32);
            ndk::AAudioStreamBuilder_setContentType(builder, ndk::AAUDIO_CONTENT_TYPE_MOVIE as i32);
            ndk::AAudioStreamBuilder_setFormat(builder, ndk::AAUDIO_FORMAT_PCM_I16);
            ndk::AAudioStreamBuilder_setChannelCount(builder, i32::from(layout.channels));
            if layout != Layout::STEREO {
                let Some(api) = masks() else {
                    ndk::AAudioStreamBuilder_delete(builder);
                    return Err(ndk::AAUDIO_ERROR_INVALID_FORMAT);
                };
                (api.set)(builder, layout.mask);
            }
            ndk::AAudioStreamBuilder_setSampleRate(builder, 48000);
            let user = (&mut **callback as *mut Callback).cast();
            ndk::AAudioStreamBuilder_setDataCallback(builder, Some(data), user);
            ndk::AAudioStreamBuilder_setErrorCallback(builder, Some(error), user);
            let code = ndk::AAudioStreamBuilder_openStream(builder, &mut self.stream);
            ndk::AAudioStreamBuilder_delete(builder);
            if code < 0 || self.stream.is_null() {
                return Err(code.min(-1));
            }
            if ndk::AAudioStream_getSampleRate(self.stream) != 48000
                || ndk::AAudioStream_getChannelCount(self.stream) != i32::from(layout.channels)
                || ndk::AAudioStream_getFormat(self.stream) != ndk::AAUDIO_FORMAT_PCM_I16
                || (layout != Layout::STEREO
                    && masks().is_none_or(|api| (api.get)(self.stream) != layout.mask))
            {
                return Err(ndk::AAUDIO_ERROR_INVALID_FORMAT);
            }
            let burst = ndk::AAudioStream_getFramesPerBurst(self.stream);
            if burst > 0 {
                let code = ndk::AAudioStream_setBufferSizeInFrames(
                    self.stream,
                    burst
                        .saturating_mul(2)
                        .min(ndk::AAudioStream_getBufferCapacityInFrames(self.stream)),
                );
                if code < 0 {
                    return Err(code);
                }
            }
        }
        Ok(())
    }
    // A failed close cannot prove callbacks stopped. Retain their userData and
    // disable this output rather than reuse/free it or affect the video session.
    fn close(&mut self) -> bool {
        if self.callback.is_none() {
            return false;
        }
        if self.stream.is_null() {
            return true;
        }
        self.callback
            .as_ref()
            .unwrap()
            .stopping
            .store(true, Ordering::Release);
        let stream = std::mem::replace(&mut self.stream, ptr::null_mut());
        unsafe {
            ndk::AAudioStream_requestStop(stream);
            let code = ndk::AAudioStream_close(stream);
            if code < 0 {
                self.last_error = code;
                self.recovery.begin();
                Box::leak(self.callback.take().unwrap());
                crate::event_log::record(
                    "ERROR",
                    "android_audio_close_failed",
                    serde_json::json!({"code":code}),
                );
                return false;
            }
        }
        true
    }
    pub fn snapshot(&self) -> Snapshot {
        let layout = self
            .callback
            .as_ref()
            .map(|c| c.layout)
            .unwrap_or(Layout::STEREO);
        let mut s = Snapshot {
            state: if self.callback.is_none() {
                "Unavailable"
            } else if self.recovery.recovering() {
                "Reconnecting"
            } else {
                "Playing"
            },
            recovering: self.recovery.recovering(),
            restorations: self.restorations,
            last_error: self.last_error,
            channels: layout.channels,
            channel_mask: layout.mask,
            negotiated_channels: self.negotiated.channels,
            frames_written: 0,
            frames_read: 0,
            xruns: 0,
            buffer_frames: 0,
            capacity_frames: 0,
            device_id: 0,
        };
        if !self.stream.is_null() {
            unsafe {
                s.frames_written = ndk::AAudioStream_getFramesWritten(self.stream);
                s.frames_read = ndk::AAudioStream_getFramesRead(self.stream);
                s.xruns = ndk::AAudioStream_getXRunCount(self.stream);
                s.buffer_frames = ndk::AAudioStream_getBufferSizeInFrames(self.stream);
                s.capacity_frames = ndk::AAudioStream_getBufferCapacityInFrames(self.stream);
                s.device_id = ndk::AAudioStream_getDeviceId(self.stream);
            }
        }
        s
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        self.close();
    }
}
