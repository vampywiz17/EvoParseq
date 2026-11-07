//! Documented macOS path: Annex B H.264/HEVC -> CMSampleBuffer ->
//! AVSampleBufferDisplayLayer. The layer decodes with VideoToolbox and presents
//! through Core Animation; the application never reads decoded pixels back.
//! The layer does not report whether VideoToolbox used hardware, so
//! `hardware_decode` stays unknown. This is not the application's Zero copy mode.
// ponytail: the layer owns decode and display pacing; move to
// VTDecompressionSession + Metal if measured latency needs finer control.
use crate::video_output::{DisplayCatchUpPolicy, Queue, Snapshot};
use crate::video_stream::{Codec, PacketInfo};
use objc2::{msg_send, rc::Retained, runtime::AnyObject};
use std::{
    ffi::c_void,
    ptr,
    sync::{Arc, Mutex},
    time::Instant,
};

mod ffi {
    use std::ffi::c_void;
    pub type OSStatus = i32;

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct CGSize {
        pub width: f64,
        pub height: f64,
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        pub static kCFBooleanTrue: *const c_void;
        pub fn CFRelease(object: *const c_void);
        pub fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
        pub fn CFDictionarySetValue(dict: *const c_void, key: *const c_void, value: *const c_void);
    }

    #[link(name = "CoreMedia", kind = "framework")]
    extern "C" {
        pub static kCMSampleAttachmentKey_DisplayImmediately: *const c_void;
        pub fn CMVideoFormatDescriptionCreateFromH264ParameterSets(
            allocator: *const c_void,
            count: usize,
            pointers: *const *const u8,
            sizes: *const usize,
            nal_length_size: i32,
            out: *mut *mut c_void,
        ) -> OSStatus;
        pub fn CMVideoFormatDescriptionCreateFromHEVCParameterSets(
            allocator: *const c_void,
            count: usize,
            pointers: *const *const u8,
            sizes: *const usize,
            nal_length_size: i32,
            extensions: *const c_void,
            out: *mut *mut c_void,
        ) -> OSStatus;
        pub fn CMVideoFormatDescriptionGetPresentationDimensions(
            format: *mut c_void,
            use_pixel_aspect_ratio: u8,
            use_clean_aperture: u8,
        ) -> CGSize;
        pub fn CMBlockBufferCreateWithMemoryBlock(
            allocator: *const c_void,
            memory: *mut c_void,
            length: usize,
            block_allocator: *const c_void,
            custom_source: *const c_void,
            offset: usize,
            data_length: usize,
            flags: u32,
            out: *mut *mut c_void,
        ) -> OSStatus;
        pub fn CMBlockBufferReplaceDataBytes(
            source: *const c_void,
            destination: *mut c_void,
            offset: usize,
            length: usize,
        ) -> OSStatus;
        pub fn CMSampleBufferCreateReady(
            allocator: *const c_void,
            data: *mut c_void,
            format: *mut c_void,
            samples: isize,
            timing_entries: isize,
            timing: *const c_void,
            size_entries: isize,
            sizes: *const usize,
            out: *mut *mut c_void,
        ) -> OSStatus;
        pub fn CMSampleBufferGetSampleAttachmentsArray(
            sample: *mut c_void,
            create: u8,
        ) -> *const c_void;
    }

    #[link(name = "VideoToolbox", kind = "framework")]
    extern "C" {
        pub fn VTIsHardwareDecodeSupported(codec: u32) -> u8;
    }

    // Linking AVFoundation registers AVSampleBufferDisplayLayer with the runtime.
    #[link(name = "AVFoundation", kind = "framework")]
    extern "C" {
        pub static AVLayerVideoGravityResizeAspect: &'static objc2_foundation::NSString;
    }
}

const ASSURE_MEMORY_NOW: u32 = 1;
const HEVC_CODEC_TYPE: u32 = u32::from_be_bytes(*b"hvc1");
/// AVQueuedSampleBufferRenderingStatusFailed.
const LAYER_FAILED: isize = 2;

/// CMSampleBufferRef as the Objective-C runtime encodes it, for msg_send checks.
#[repr(C)]
struct OpaqueSampleBuffer([u8; 0]);
unsafe impl objc2::encode::RefEncode for OpaqueSampleBuffer {
    const ENCODING_REF: objc2::encode::Encoding = objc2::encode::Encoding::Pointer(
        &objc2::encode::Encoding::Struct("opaqueCMSampleBuffer", &[]),
    );
}

/// Owned Core Foundation object, released on drop.
struct Cf(*mut c_void);
impl Drop for Cf {
    fn drop(&mut self) {
        unsafe { ffi::CFRelease(self.0) }
    }
}
// Core Media format descriptions and sample buffers are immutable once created.
unsafe impl Send for Cf {}

/// Annex B NAL units, without start codes or trailing zero padding.
fn nal_units(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut offset = 0;
    std::iter::from_fn(move || {
        while bytes.get(offset) == Some(&0) {
            offset += 1;
        }
        if bytes.get(offset) != Some(&1) {
            return None;
        }
        let begin = offset + 1;
        let mut end = begin;
        while end < bytes.len() && !bytes[end..].starts_with(&[0, 0, 1]) {
            end += 1;
        }
        offset = end;
        let mut nal = &bytes[begin..end];
        while let [rest @ .., 0] = nal {
            nal = rest;
        }
        Some(nal)
    })
}

/// Parameter-set slot (VPS/SPS/PPS order) for a NAL, or None for picture data.
fn parameter_slot(codec: Codec, nal: &[u8]) -> Option<usize> {
    let header = *nal.first()?;
    match codec {
        Codec::H264 => match header & 31 {
            7 => Some(1),
            8 => Some(2),
            _ => None,
        },
        Codec::Hevc => match (header >> 1) & 63 {
            t @ 32..=34 => Some(usize::from(t - 32)),
            _ => None,
        },
    }
}

/// Turns Annex B packets into length-prefixed CMSampleBuffers, rebuilding the
/// format description whenever the stream's parameter sets change. The one
/// copy into the compressed sample is required by Core Media's AVCC layout.
#[derive(Default)]
struct Converter {
    parameters: [Option<Vec<u8>>; 3],
    format: Option<Cf>,
    sample: Vec<u8>,
    /// New parameter sets replaced a running format; the layer must be
    /// flushed before the next key picture.
    reconfigured: bool,
}

impl Converter {
    /// Ok(None): the packet carried only parameter sets, or no format exists yet.
    fn convert(&mut self, bytes: &[u8], codec: Codec) -> Result<Option<Cf>, String> {
        let mut changed = false;
        self.sample.clear();
        for nal in nal_units(bytes) {
            if nal.is_empty() {
                continue;
            }
            if let Some(slot) = parameter_slot(codec, nal) {
                if self.parameters[slot].as_deref() != Some(nal) {
                    self.parameters[slot] = Some(nal.to_vec());
                    changed = true;
                }
            } else {
                let length = u32::try_from(nal.len()).map_err(|_| "nal size")?;
                self.sample.extend_from_slice(&length.to_be_bytes());
                self.sample.extend_from_slice(nal);
            }
        }
        if changed {
            if let Some(format) = self.create_format(codec)? {
                self.reconfigured |= self.format.replace(format).is_some();
            }
        }
        let Some(format) = &self.format else {
            return Ok(None);
        };
        if self.sample.is_empty() {
            return Ok(None);
        }
        unsafe {
            let mut block = ptr::null_mut();
            let status = ffi::CMBlockBufferCreateWithMemoryBlock(
                ptr::null(),
                ptr::null_mut(),
                self.sample.len(),
                ptr::null(),
                ptr::null(),
                0,
                self.sample.len(),
                ASSURE_MEMORY_NOW,
                &mut block,
            );
            if status != 0 {
                return Err(format!("CMBlockBufferCreateWithMemoryBlock {status}"));
            }
            let block = Cf(block);
            let status = ffi::CMBlockBufferReplaceDataBytes(
                self.sample.as_ptr().cast(),
                block.0,
                0,
                self.sample.len(),
            );
            if status != 0 {
                return Err(format!("CMBlockBufferReplaceDataBytes {status}"));
            }
            let mut sample = ptr::null_mut();
            let size = self.sample.len();
            let status = ffi::CMSampleBufferCreateReady(
                ptr::null(),
                block.0,
                format.0,
                1,
                0,
                ptr::null(),
                1,
                &size,
                &mut sample,
            );
            if status != 0 {
                return Err(format!("CMSampleBufferCreateReady {status}"));
            }
            let sample = Cf(sample);
            // Live stream: show each picture as soon as it is decoded, no timeline.
            let attachments = ffi::CMSampleBufferGetSampleAttachmentsArray(sample.0, 1);
            if !attachments.is_null() {
                ffi::CFDictionarySetValue(
                    ffi::CFArrayGetValueAtIndex(attachments, 0),
                    ffi::kCMSampleAttachmentKey_DisplayImmediately,
                    ffi::kCFBooleanTrue,
                );
            }
            Ok(Some(sample))
        }
    }

    fn create_format(&self, codec: Codec) -> Result<Option<Cf>, String> {
        let first = match codec {
            Codec::H264 => 1,
            Codec::Hevc => 0,
        };
        let sets: Option<Vec<&[u8]>> = self.parameters[first..]
            .iter()
            .map(|p| p.as_deref())
            .collect();
        // Wait until the full parameter set is known; a partial set is not decodable.
        let Some(sets) = sets else {
            return Ok(None);
        };
        let pointers: Vec<*const u8> = sets.iter().map(|s| s.as_ptr()).collect();
        let sizes: Vec<usize> = sets.iter().map(|s| s.len()).collect();
        let mut format = ptr::null_mut();
        let status = unsafe {
            match codec {
                Codec::H264 => ffi::CMVideoFormatDescriptionCreateFromH264ParameterSets(
                    ptr::null(),
                    sets.len(),
                    pointers.as_ptr(),
                    sizes.as_ptr(),
                    4,
                    &mut format,
                ),
                Codec::Hevc => ffi::CMVideoFormatDescriptionCreateFromHEVCParameterSets(
                    ptr::null(),
                    sets.len(),
                    pointers.as_ptr(),
                    sizes.as_ptr(),
                    4,
                    ptr::null(),
                    &mut format,
                ),
            }
        };
        if status != 0 {
            return Err(format!("format description {status}"));
        }
        Ok(Some(Cf(format)))
    }

    fn dimensions(&self) -> Option<(u32, u32)> {
        let format = self.format.as_ref()?;
        let size =
            unsafe { ffi::CMVideoFormatDescriptionGetPresentationDimensions(format.0, 1, 1) };
        Some((size.width as u32, size.height as u32))
    }
}

/// Creates the display layer; called on the main thread by the window.
pub fn create_layer() -> Option<Retained<AnyObject>> {
    let class = objc2::runtime::AnyClass::get(c"AVSampleBufferDisplayLayer")?;
    let layer: Retained<AnyObject> = unsafe { msg_send![class, new] };
    let _: () =
        unsafe { msg_send![&*layer, setVideoGravity: ffi::AVLayerVideoGravityResizeAspect] };
    Some(layer)
}

trait Layer {
    fn status(&self) -> isize;
    fn error_code(&self) -> isize;
    fn is_ready(&self) -> bool;
    fn flush(&mut self);
    fn enqueue(&mut self, sample: &Cf);
}

impl Layer for &AnyObject {
    fn status(&self) -> isize {
        unsafe { msg_send![*self, status] }
    }
    fn error_code(&self) -> isize {
        let error: Option<Retained<AnyObject>> = unsafe { msg_send![*self, error] };
        error.map(|e| unsafe { msg_send![&*e, code] }).unwrap_or(0)
    }
    fn is_ready(&self) -> bool {
        unsafe { msg_send![*self, isReadyForMoreMediaData] }
    }
    fn flush(&mut self) {
        let _: () = unsafe { msg_send![*self, flush] };
    }
    fn enqueue(&mut self, sample: &Cf) {
        let _: () =
            unsafe { msg_send![*self, enqueueSampleBuffer: sample.0.cast::<OpaqueSampleBuffer>()] };
    }
}

#[derive(Default)]
struct State {
    queue: Queue,
    converter: Converter,
    shown: bool,
}

impl State {
    fn submit(&mut self, layer: &mut impl Layer, bytes: &[u8], info: PacketInfo) -> bool {
        let q = &mut self.queue;
        if q.stopped || q.report.failure_stage.is_some() {
            q.report.frames_dropped = q.report.frames_dropped.saturating_add(1);
            return false;
        }
        if bytes.len() > q.max_frame_bytes {
            q.fail("video-input-too-large", None);
            q.report.frames_dropped = q.report.frames_dropped.saturating_add(1);
            return false;
        }
        if q.report.codec.is_some_and(|codec| codec != info.codec) {
            q.fail("video-unexpected-codec-change", None);
            return false;
        }
        q.report.codec = Some(info.codec);

        let mut flushed = false;
        if layer.status() == LAYER_FAILED {
            let code = layer.error_code();
            crate::event_log::record(
                "WARN",
                "video_layer_failed",
                serde_json::json!({"code": code}),
            );
            layer.flush();
            flushed = true;
            // The decoder lost its references; resume from the next key picture.
            if !q.report.recovering && !q.recover(Instant::now()) {
                q.report.frames_dropped = q.report.frames_dropped.saturating_add(1);
                return false;
            }
        }

        if q.report.waiting_for_idr && !info.idr && !info.parameters {
            q.report.frames_dropped = q.report.frames_dropped.saturating_add(1);
            return false;
        }
        let sample = match self.converter.convert(bytes, info.codec) {
            Ok(sample) => sample,
            Err(error) => {
                q.fail("video-sample-create", Some(error));
                return false;
            }
        };
        if self.converter.format.is_some() {
            q.report.decoder_initialized = true;
            if let Some((width, height)) = self.converter.dimensions() {
                q.report.width = Some(width);
                q.report.height = Some(height);
            }
        }
        if std::mem::take(&mut self.converter.reconfigured) {
            // The host changed the encoded format (e.g. a display switch or a
            // fullscreen app transition). Retire queued frames from the old
            // format while keeping the last image, and wait for a key picture.
            if !flushed {
                layer.flush();
                flushed = true;
            }
            q.begin_format_change(Instant::now(), !info.idr);
            crate::event_log::record(
                "INFO",
                "video_format_changed",
                serde_json::json!({"width":q.report.width,"height":q.report.height}),
            );
        }
        // Parameter sets alone never end the wait for a key picture; a delta
        // picture attached to parameter sets is still dropped.
        if q.report.waiting_for_idr && !info.idr {
            if sample.is_some() {
                q.report.frames_dropped = q.report.frames_dropped.saturating_add(1);
                return false;
            }
            return true;
        }
        let Some(sample) = sample else {
            return true;
        };
        if !layer.is_ready() {
            // The layer's decode queue is full (slow decoding or a burst after
            // congestion). Discard stale queued buffers while keeping the last
            // displayed frame; only a key picture can resume after a flush.
            if !flushed {
                layer.flush();
            }
            if !info.idr || !layer.is_ready() {
                q.drop_backlog(Instant::now());
                q.report.frames_dropped = q.report.frames_dropped.saturating_add(1);
                return false;
            }
        }
        if info.idr {
            q.report.decoder_initialized = true;
            q.report.waiting_for_idr = false;
            if q.report.recovering {
                q.recovered();
            }
        }
        layer.enqueue(&sample);
        q.report.frames_submitted = q.report.frames_submitted.saturating_add(1);
        self.shown = true;
        true
    }

    fn encoder_rebuilt(&mut self, layer: &mut impl Layer) -> bool {
        if self.queue.encoder_rebuilt(Instant::now()) {
            self.converter = Converter::default();
            layer.flush();
            return true;
        }
        false
    }
}

pub struct Pipeline {
    state: Mutex<State>,
    window: Arc<crate::window::Window>,
    generation: u64,
}

impl Pipeline {
    /// The layer decodes and paces display itself; the catch-up policy applies
    /// to the Windows presentation queue only.
    pub fn start(window: Arc<crate::window::Window>, _policy: DisplayCatchUpPolicy) -> Self {
        let generation = window.begin_video();
        let mut queue = Queue::default();
        queue.report.enabled = true;
        queue.report.decoder = Some("VideoToolbox (AVSampleBufferDisplayLayer)");
        queue.report.renderer = Some("Core Animation");
        queue.report.presentation_mode = Some("Display immediately");
        if window.video_layer() == 0 {
            queue.fail("video-layer-create", None);
        }
        Self {
            state: Mutex::new(State {
                queue,
                converter: Converter::default(),
                shown: false,
            }),
            window,
            generation,
        }
    }

    fn layer(&self) -> &AnyObject {
        // The window retains its layer for its whole lifetime, and we hold the window.
        unsafe { &*(self.window.video_layer() as *const AnyObject) }
    }

    pub fn submit(&self, bytes: &[u8], info: PacketInfo) -> bool {
        self.submit_oriented(bytes, info, 0)
    }

    // ponytail: host rotation is not applied to the layer yet; add a layer
    // transform plus viewport mapping when a rotated host is validated.
    pub fn submit_oriented(&self, bytes: &[u8], info: PacketInfo, _rotation: u8) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let was_shown = state.shown;
        let mut layer = self.layer();
        let accepted = state.submit(&mut layer, bytes, info);
        if !was_shown && state.shown {
            self.window.set_video_ready_for(self.generation, true);
        }
        *self
            .window
            .video_report
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(state.queue.report.clone());
        accepted
    }

    /// Explicit BUD encoder-generation boundary: drop old parameter sets and
    /// the layer's references, then wait for the new stream's key picture.
    #[cfg(feature = "bud")]
    pub fn encoder_rebuilt(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut layer = self.layer();
        if state.encoder_rebuilt(&mut layer) {
            *self
                .window
                .video_report
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Some(state.queue.report.clone());
        }
    }

    pub fn take_keyframe_request(&self) -> Option<(u64, bool)> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let failed_before = state.queue.report.failure_stage.is_some();
        let request = state
            .queue
            .take_keyframe_request(Instant::now())
            .map(|_| (state.queue.reset_generation, false));
        if !failed_before && state.queue.report.failure_stage.is_some() {
            *self
                .window
                .video_report
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Some(state.queue.report.clone());
        }
        request
    }

    pub fn recovery_request_failed(&self, generation: u64) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.queue.request_failed(generation);
        *self
            .window
            .video_report
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(state.queue.report.clone());
    }

    pub fn snapshot(&self) -> Snapshot {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .queue
            .report
            .clone()
    }

    /// No application-owned decoder worker can stall on this path.
    pub fn native_failure(&self) -> Option<&'static str> {
        None
    }

    pub fn stop(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.queue.stopped {
            return;
        }
        state.queue.stopped = true;
        state.queue.clear();
        state.converter = Converter::default();
        state.queue.report.worker_finished = true;
        state.queue.report.resources_released = true;
        *self
            .window
            .video_report
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(state.queue.report.clone());
        drop(state);
        self.window.finish_video(self.generation);
        let window = self.window.clone();
        dispatch2::DispatchQueue::main().exec_async(move || {
            let layer = window.video_layer();
            if layer != 0 {
                let _: () =
                    unsafe { msg_send![&*(layer as *const AnyObject), flushAndRemoveImage] };
            }
        });
    }
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn hevc_available(_gpu: Option<&crate::connection_settings::GpuPreference>) -> bool {
    unsafe { ffi::VTIsHardwareDecodeSupported(HEVC_CODEC_TYPE) != 0 }
}

/// Main 10 output through the layer has not been validated, so it is not offered.
pub fn hevc_ten_bit_available(_gpu: Option<&crate::connection_settings::GpuPreference>) -> bool {
    false
}

/// HEVC 4:4:4 is a Windows vendor-decoder path only.
pub fn hevc_444_available(
    _gpu: Option<&crate::connection_settings::GpuPreference>,
    _ten_bit: bool,
) -> bool {
    false
}

#[cfg(test)]
pub(crate) fn display_extents(width: u32, height: u32, rotation: u8) -> (u32, u32) {
    if rotation & 1 != 0 {
        (height, width)
    } else {
        (width, height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CMTime {
        value: i64,
        timescale: i32,
        flags: u32,
        epoch: i64,
    }
    type Callback = extern "C" fn(*mut c_void, *mut c_void, i32, u32, *mut c_void, CMTime, CMTime);
    #[repr(C)]
    struct CallbackRecord {
        callback: Callback,
        refcon: *mut c_void,
    }
    #[link(name = "VideoToolbox", kind = "framework")]
    extern "C" {
        fn VTDecompressionSessionCreate(
            allocator: *const c_void,
            format: *mut c_void,
            decoder_specification: *const c_void,
            image_attributes: *const c_void,
            callback: *const CallbackRecord,
            out: *mut *mut c_void,
        ) -> i32;
        fn VTDecompressionSessionDecodeFrame(
            session: *mut c_void,
            sample: *mut c_void,
            flags: u32,
            frame_refcon: *mut c_void,
            info_out: *mut u32,
        ) -> i32;
        fn VTDecompressionSessionWaitForAsynchronousFrames(session: *mut c_void) -> i32;
        fn VTDecompressionSessionInvalidate(session: *mut c_void);
    }

    extern "C" fn decoded(
        refcon: *mut c_void,
        _frame: *mut c_void,
        status: i32,
        flags: u32,
        image: *mut c_void,
        _pts: CMTime,
        _duration: CMTime,
    ) {
        const FRAME_DROPPED: u32 = 1 << 1;
        if status == 0 && (!image.is_null() || flags & FRAME_DROPPED == 0) {
            unsafe { *(refcon as *mut u32) += 1 };
        }
    }

    /// The lowlat fixture as Annex B access units.
    fn hevc_units() -> Vec<Vec<u8>> {
        let mut fixture: &[u8] = include_bytes!("../crates/video/fixtures/synthetic-720p-hevc.bin");
        let mut units = Vec::new();
        while !fixture.is_empty() {
            let len = u32::from_le_bytes(fixture[..4].try_into().unwrap()) as usize;
            units.push(fixture[4..4 + len].to_vec());
            fixture = &fixture[4 + len..];
        }
        units
    }

    /// Feeds converted samples to VideoToolbox, the decoder behind the display
    /// layer, and returns how many pictures came out (offline test readback only).
    #[test]
    fn hevc_fixture_samples_decode() {
        let units = hevc_units();
        let mut converter = Converter::default();
        let mut session: *mut c_void = ptr::null_mut();
        let mut pictures = 0u32;
        let record = CallbackRecord {
            callback: decoded,
            refcon: (&mut pictures as *mut u32).cast(),
        };
        for unit in &units {
            let Some(sample) = converter.convert(unit, Codec::Hevc).unwrap() else {
                continue;
            };
            if session.is_null() {
                let format = converter.format.as_ref().unwrap().0;
                let status = unsafe {
                    VTDecompressionSessionCreate(
                        ptr::null(),
                        format,
                        ptr::null(),
                        ptr::null(),
                        &record,
                        &mut session,
                    )
                };
                assert_eq!(status, 0, "VTDecompressionSessionCreate");
            }
            let status = unsafe {
                VTDecompressionSessionDecodeFrame(
                    session,
                    sample.0,
                    0,
                    ptr::null_mut(),
                    ptr::null_mut(),
                )
            };
            assert_eq!(status, 0, "VTDecompressionSessionDecodeFrame");
        }
        unsafe {
            VTDecompressionSessionWaitForAsynchronousFrames(session);
            VTDecompressionSessionInvalidate(session);
            ffi::CFRelease(session);
        }
        assert_eq!(converter.dimensions(), Some((1280, 720)));
        assert_eq!(pictures, units.len() as u32);
    }

    /// Exercises the exact layer messages the pipeline sends (selectors and
    /// argument encodings); a wrong one raises instead of returning.
    #[test]
    fn display_layer_accepts_samples() {
        let layer = create_layer().expect("AVSampleBufferDisplayLayer");
        let mut converter = Converter::default();
        for unit in hevc_units().iter().take(8) {
            if let Some(sample) = converter.convert(unit, Codec::Hevc).unwrap() {
                let _: () = unsafe {
                    msg_send![&*layer, enqueueSampleBuffer: sample.0.cast::<OpaqueSampleBuffer>()]
                };
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
        let status: isize = unsafe { msg_send![&*layer, status] };
        let error: Option<Retained<AnyObject>> = unsafe { msg_send![&*layer, error] };
        assert_ne!(status, LAYER_FAILED, "{error:?}");
        let _: () = unsafe { msg_send![&*layer, flush] };
        let _: () = unsafe { msg_send![&*layer, flushAndRemoveImage] };
    }

    #[test]
    fn only_replaced_parameter_sets_request_a_flush() {
        let units = hevc_units();
        let mut converter = Converter::default();
        for unit in &units {
            converter.convert(unit, Codec::Hevc).unwrap();
        }
        assert!(!converter.reconfigured, "the first format is not a change");
        converter.convert(&units[0], Codec::Hevc).unwrap();
        assert!(!converter.reconfigured, "repeated parameter sets");
        let sps = include_bytes!("../crates/video/fixtures/hevc-crop.sps");
        converter.convert(sps, Codec::Hevc).unwrap();
        assert!(converter.reconfigured);
        assert_ne!(converter.dimensions(), Some((1280, 720)));
    }

    #[test]
    fn annex_b_split_drops_start_codes_and_padding() {
        let bytes = [
            0, 0, 0, 1, 0x67, 1, 2, 0, 0, 0, 1, 0x68, 3, 0, 0, 1, 0x65, 4,
        ];
        let nals: Vec<&[u8]> = nal_units(&bytes).collect();
        assert_eq!(nals, [&[0x67, 1, 2][..], &[0x68, 3], &[0x65, 4]]);
        assert_eq!(parameter_slot(Codec::H264, nals[0]), Some(1));
        assert_eq!(parameter_slot(Codec::H264, nals[1]), Some(2));
        assert_eq!(parameter_slot(Codec::H264, nals[2]), None);
    }

    struct TestLayer {
        status: isize,
        error_code: isize,
        ready: bool,
        ready_after_flush: bool,
        flushes: usize,
        enqueued: usize,
    }

    impl Default for TestLayer {
        fn default() -> Self {
            Self {
                status: 0,
                error_code: 0,
                ready: true,
                ready_after_flush: true,
                flushes: 0,
                enqueued: 0,
            }
        }
    }

    impl Layer for TestLayer {
        fn status(&self) -> isize {
            self.status
        }
        fn error_code(&self) -> isize {
            self.error_code
        }
        fn is_ready(&self) -> bool {
            self.ready
        }
        fn flush(&mut self) {
            self.flushes += 1;
            if self.status == LAYER_FAILED {
                self.status = 0;
            }
            if self.ready_after_flush {
                self.ready = true;
            }
        }
        fn enqueue(&mut self, _sample: &Cf) {
            self.enqueued += 1;
        }
    }

    fn inspect_all(units: &[Vec<u8>]) -> Vec<PacketInfo> {
        let mut inspector = crate::video_stream::Inspector::default();
        inspector.enable_hevc(true);
        units
            .iter()
            .map(|unit| inspector.receive(unit).expect("valid HEVC access unit"))
            .collect()
    }

    #[test]
    fn backpressure_flushes_layer_requests_keyframe_and_recovers_on_idr() {
        let units = hevc_units();
        let infos = inspect_all(&units);
        let mut state = State::default();
        let mut layer = TestLayer::default();
        assert!(state.submit(&mut layer, &units[0], infos[0]));
        assert!(state.submit(&mut layer, &units[1], infos[1]));
        assert_eq!(layer.enqueued, 2);
        assert_eq!(state.queue.report.frames_submitted, 2);
        assert!(state.shown);

        // Slow decoding or a post-congestion burst fills the display layer:
        // flush queued frames, drop the delta chain, and request a fresh IDR.
        layer.ready = false;
        assert!(!state.submit(&mut layer, &units[2], infos[2]));
        assert_eq!(layer.flushes, 1);
        assert_eq!(layer.enqueued, 2);
        assert!(state.queue.report.waiting_for_idr);
        assert!(state.queue.report.recovering);
        assert_eq!(state.queue.report.frames_dropped, 1);
        assert_eq!(
            state.queue.take_keyframe_request(Instant::now()),
            Some(Codec::Hevc)
        );

        // Even after the flush makes the layer ready again, deltas following
        // the dropped reference frame must not be enqueued before an IDR.
        assert!(!state.submit(&mut layer, &units[3], infos[3]));
        assert_eq!(layer.enqueued, 2);
        assert_eq!(state.queue.report.frames_dropped, 2);

        // If the layer is again not ready when the recovery IDR arrives,
        // flushing clears the stale queue so the IDR can be enqueued immediately.
        layer.ready = false;
        assert!(state.submit(&mut layer, &units[0], infos[0]));
        assert_eq!(layer.flushes, 2);
        assert_eq!(layer.enqueued, 3);
        assert!(!state.queue.report.waiting_for_idr);
        assert!(!state.queue.report.recovering);
        assert_eq!(state.queue.report.frames_submitted, 3);

        // Repeated backpressure episodes across a session (and multiple deltas
        // arriving during a single backpressure episode) must not exhaust the
        // fatal decoder-error budget and freeze video.
        for _ in 0..4 {
            layer.ready = false;
            layer.ready_after_flush = false;
            assert!(!state.submit(&mut layer, &units[1], infos[1]));
            assert!(!state.submit(&mut layer, &units[2], infos[2]));
            assert!(state.queue.report.failure_stage.is_none());
            layer.ready_after_flush = true;
            assert!(state.submit(&mut layer, &units[0], infos[0]));
            assert!(state.queue.report.failure_stage.is_none());
            assert!(!state.queue.report.waiting_for_idr);
        }

        // A valid BUD keyframe access unit larger than WebRTC's 1 MiB
        // MAX_CHANNEL_MESSAGE ceiling (up to BUD's 4000 * 1193 ring bound)
        // must not trigger video-input-too-large and freeze the video layer.
        assert!(state.encoder_rebuilt(&mut layer));
        let mut large_bud_idr = units[0].clone();
        large_bud_idr.resize(crate::control::MAX_CHANNEL_MESSAGE + 64 * 1024, 0);
        assert!(state.submit(&mut layer, &large_bud_idr, infos[0]));
        assert!(state.queue.report.failure_stage.is_none());
        assert!(state.submit(&mut layer, &units[1], infos[1]));
    }

    #[test]
    fn display_and_stream_reconfiguration_flush_and_wait_for_idr_without_spending_error_budget() {
        let units = hevc_units();
        let infos = inspect_all(&units);
        let crop_sps = include_bytes!("../crates/video/fixtures/hevc-crop.sps");
        let crop_info = PacketInfo {
            picture: false,
            codec: Codec::Hevc,
            idr: false,
            parameters: true,
        };
        let mut state = State::default();
        let mut layer = TestLayer::default();
        assert!(state.submit(&mut layer, &units[0], infos[0]));
        assert!(state.submit(&mut layer, &units[1], infos[1]));
        assert_eq!(
            (state.queue.report.width, state.queue.report.height),
            (Some(1280), Some(720))
        );

        // Repeat host display/resolution changes more than 3 times: valid
        // format transitions must never exhaust the error-recovery budget.
        let mut cropped_idr = Vec::new();
        for nal in nal_units(&units[0]) {
            if parameter_slot(Codec::Hevc, nal) == Some(1) {
                cropped_idr.extend_from_slice(crop_sps);
            } else {
                cropped_idr.extend_from_slice(&[0, 0, 0, 1]);
                cropped_idr.extend_from_slice(nal);
            }
        }
        for _ in 0..4 {
            // 1. Out-of-band parameter-set change before the new IDR arrives.
            assert!(state.submit(&mut layer, crop_sps, crop_info));
            assert!(state.queue.report.waiting_for_idr);
            assert!(state.queue.report.recovering);
            assert_ne!(
                (state.queue.report.width, state.queue.report.height),
                (Some(1280), Some(720))
            );
            assert!(!state.submit(&mut layer, &units[2], infos[2]));

            // A packet carrying both parameters and a delta picture while
            // waiting for IDR keeps the parameters but drops the delta.
            let mut param_plus_delta = crop_sps.to_vec();
            param_plus_delta.extend_from_slice(&units[2]);
            assert!(!state.submit(
                &mut layer,
                &param_plus_delta,
                PacketInfo {
                    picture: true,
                    codec: Codec::Hevc,
                    idr: false,
                    parameters: true,
                },
            ));

            // Matching IDR completes the format transition.
            assert!(state.submit(&mut layer, &cropped_idr, infos[0]));
            assert!(!state.queue.report.waiting_for_idr);
            assert!(!state.queue.report.recovering);
            assert!(state.queue.report.failure_stage.is_none());

            // 2. In-band parameter-set + IDR change in the same access unit.
            let flushes_before = layer.flushes;
            assert!(state.submit(&mut layer, &units[0], infos[0]));
            assert_eq!(layer.flushes, flushes_before + 1);
            assert!(!state.queue.report.waiting_for_idr);
            assert!(!state.queue.report.recovering);
            assert_eq!(
                (state.queue.report.width, state.queue.report.height),
                (Some(1280), Some(720))
            );
            assert!(state.queue.report.failure_stage.is_none());
        }

        // Explicit encoder rebuild (BUD display/mode switch) flushes the layer,
        // clears cached parameter sets, drops deltas, and resumes on a new IDR.
        let flushes_before = layer.flushes;
        assert!(state.encoder_rebuilt(&mut layer));
        assert_eq!(layer.flushes, flushes_before + 1);
        assert!(state.queue.report.waiting_for_idr);
        assert!(!state.submit(&mut layer, &units[1], infos[1]));
        assert!(state.submit(&mut layer, &units[0], infos[0]));
        assert!(!state.queue.report.waiting_for_idr);
        assert_eq!(
            (state.queue.report.width, state.queue.report.height),
            (Some(1280), Some(720))
        );
    }

    #[test]
    fn layer_failure_and_fullscreen_resize_recover_on_idr() {
        let units = hevc_units();
        let infos = inspect_all(&units);
        let mut state = State::default();
        let mut layer = TestLayer::default();
        assert!(state.submit(&mut layer, &units[0], infos[0]));
        assert!(state.submit(&mut layer, &units[1], infos[1]));

        // 1. Layer fails before a delta arrives: flush, enter recovery, drop delta.
        layer.status = LAYER_FAILED;
        assert!(!state.submit(&mut layer, &units[2], infos[2]));
        assert_eq!(layer.status, 0);
        assert_eq!(layer.flushes, 1);
        assert!(state.queue.report.waiting_for_idr);
        assert!(state.queue.report.recovering);
        assert_eq!(state.queue.report.frames_dropped, 1);

        // 2. If the layer is in LAYER_FAILED when the recovery IDR itself arrives
        // (e.g. asynchronous failure from an earlier frame or fullscreen transition),
        // flushing the layer allows that IDR to recover immediately without
        // dropping the keyframe or spending another recovery attempt.
        layer.status = LAYER_FAILED;
        assert!(state.submit(&mut layer, &units[0], infos[0]));
        assert_eq!(layer.status, 0);
        assert_eq!(layer.flushes, 2);
        assert!(!state.queue.report.waiting_for_idr);
        assert!(!state.queue.report.recovering);
        assert_eq!(state.queue.report.frames_submitted, 3);
        assert!(state.submit(&mut layer, &units[1], infos[1]));

        // 3. Exercise the real AVSampleBufferDisplayLayer through State::submit
        // across fullscreen-sized bounds changes and stream reconfiguration.
        let real_layer = create_layer().expect("AVSampleBufferDisplayLayer");
        let mut real_ref: &AnyObject = &real_layer;
        let mut live_state = State::default();
        for (unit, info) in units.iter().zip(infos.iter()).take(4) {
            assert!(live_state.submit(&mut real_ref, unit, *info));
        }
        // Simulate windowed -> fullscreen -> windowed layer bounds transitions.
        for (width, height) in [(2560.0, 1440.0), (1280.0, 720.0)] {
            let bounds = objc2_foundation::NSRect::new(
                objc2_foundation::NSPoint::new(0.0, 0.0),
                objc2_foundation::NSSize::new(width, height),
            );
            let _: () = unsafe { msg_send![real_ref, setBounds: bounds] };
            for (unit, info) in units.iter().zip(infos.iter()).skip(4).take(2) {
                assert!(live_state.submit(&mut real_ref, unit, *info));
            }
        }
        // Reconfigure stream format on the real layer and recover on IDR.
        let crop_sps = include_bytes!("../crates/video/fixtures/hevc-crop.sps");
        assert!(live_state.submit(
            &mut real_ref,
            crop_sps,
            PacketInfo {
                picture: false,
                codec: Codec::Hevc,
                idr: false,
                parameters: true,
            },
        ));
        assert!(live_state.queue.report.waiting_for_idr);
        assert!(live_state.submit(&mut real_ref, &units[0], infos[0]));
        assert!(!live_state.queue.report.waiting_for_idr);
        assert_ne!(real_ref.status(), LAYER_FAILED);
        let _: () = unsafe { msg_send![real_ref, flushAndRemoveImage] };
    }
}
