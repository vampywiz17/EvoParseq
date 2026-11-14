//! Linux wakeup/focus adapter around the existing shared compressed video queue.
use crate::{
    video_output::{MAX_BYTES, MAX_FRAMES, Queue},
    video_stream::{PacketInfo, ParameterCache},
};
use anyhow::{Result, ensure};
use std::{
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};

#[derive(Default)]
struct Pending {
    queue: Queue,
    parameters: ParameterCache,
    suspended: bool,
    decoder_generation: u64,
    initialization: Option<Vec<u8>>,
}

impl Pending {
    fn check(&self) -> Result<()> {
        ensure!(
            self.queue.report.failure_stage.is_none(),
            "Linux video stopped: {} (queue peak {}, decode peak {:.1} ms, queue wait peak {:.1} ms)",
            self.queue.report.failure_stage.unwrap_or("unknown"),
            self.queue.report.queue_peak_depth,
            self.queue.report.local_timing.decoder_service.peak_ms,
            self.queue.report.local_timing.compressed_wait.peak_ms
        );
        Ok(())
    }
}

type GpuWake = Box<dyn Fn() -> bool + Send + Sync>;

pub struct Playback {
    displays: OnceLock<(std::sync::Weak<crate::display_routes::Displays>, usize)>,
    gpu_wakeup: Mutex<Option<GpuWake>>,
    stopped: AtomicBool,
    hevc: AtomicBool,
    ten_bit: AtomicBool,
    packets: Mutex<Pending>,
    ui: eframe::egui::Context,
    pub(crate) input: crate::input_handoff::Inbox,
    consumer: OnceLock<std::thread::Thread>,
    input_geometry: AtomicU64,
    input_ready: AtomicBool,
    capture: AtomicBool,
    pub(crate) pad_focus: AtomicBool,
    pub(crate) clipboard_allowed: AtomicBool,
    clipboard_output: Mutex<Option<String>>,
    clipboard_pending: AtomicBool,
    pub(crate) immersive: std::sync::atomic::AtomicU8,
    cursor_restore: Mutex<Option<(usize, i32, i32)>>,
    video_rect: Mutex<Option<eframe::egui::Rect>>,
    cursor_state: AtomicU64,
    pointer_buttons: std::sync::atomic::AtomicU32,
}

impl Playback {
    pub fn has_queued_video(&self) -> bool {
        self.packets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .queue
            .report
            .queue_depth
            > 0
    }
    pub fn defer_gpu(&self, ready: impl Fn() -> bool + Send + Sync + 'static) {
        let mut wake = self.gpu_wakeup.lock().unwrap_or_else(|e| e.into_inner());
        if wake.is_none() {
            *wake = Some(Box::new(ready));
        }
    }
    /// Existing transport/control loop only; no new worker, timer or render.
    /// Poll exclusively while bounded GPU backpressure left compressed input.
    pub fn poll_gpu(&self) {
        let mut wake = self.gpu_wakeup.lock().unwrap_or_else(|e| e.into_inner());
        if wake.as_ref().is_some_and(|ready| ready()) {
            *wake = None;
            self.ui.request_repaint();
        }
    }

    pub fn new(ui: eframe::egui::Context) -> Arc<Self> {
        Arc::new(Self {
            displays: OnceLock::new(),
            gpu_wakeup: Mutex::new(None),
            stopped: AtomicBool::new(false),
            hevc: AtomicBool::new(false),
            ten_bit: AtomicBool::new(false),
            packets: Mutex::new(Pending::default()),
            ui,
            input: Default::default(),
            consumer: OnceLock::new(),
            input_geometry: AtomicU64::new(0),
            input_ready: AtomicBool::new(false),
            capture: AtomicBool::new(false),
            pad_focus: AtomicBool::new(false),
            clipboard_allowed: AtomicBool::new(false),
            clipboard_output: Mutex::new(None),
            clipboard_pending: AtomicBool::new(false),
            immersive: std::sync::atomic::AtomicU8::new(0),
            cursor_restore: Mutex::new(None),
            video_rect: Mutex::new(None),
            cursor_state: AtomicU64::new(0),
            pointer_buttons: std::sync::atomic::AtomicU32::new(0),
        })
    }
    pub fn set_hevc(&self, enabled: bool) {
        self.hevc.store(enabled, Ordering::Relaxed);
    }
    pub fn set_ten_bit(&self, enabled: bool) {
        self.ten_bit.store(enabled, Ordering::Relaxed);
    }
    pub fn ten_bit(&self) -> bool {
        self.ten_bit.load(Ordering::Relaxed)
    }
    pub fn hevc(&self) -> bool {
        self.hevc.load(Ordering::Relaxed)
    }
    pub fn video_snapshot(&self) -> crate::video_output::Snapshot {
        self.packets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .queue
            .report
            .clone()
    }
    pub fn repaint(&self) {
        self.ui.request_repaint();
        if self.stream() != 0 {
            self.ui
                .request_repaint_of(crate::display_routes::viewport(self.stream()));
        }
    }
    pub fn attach_displays(&self, displays: &Arc<crate::display_routes::Displays>, stream: usize) {
        let _ = self.displays.set((Arc::downgrade(displays), stream));
    }
    pub fn displays(&self) -> Option<Arc<crate::display_routes::Displays>> {
        self.displays.get()?.0.upgrade()
    }
    pub fn stream(&self) -> usize {
        self.displays.get().map_or(0, |d| d.1)
    }
    pub fn activate_input(&self) {
        if let Some(displays) = self.displays() {
            displays.owner.store(self.stream() + 1, Ordering::Release);
        }
    }
    pub fn owns_input(&self) -> bool {
        self.displays()
            .is_none_or(|d| d.owner.load(Ordering::Acquire) == self.stream() + 1)
    }
    pub fn clipboard_write(&self, text: &str) -> bool {
        if !self.clipboard_allowed.load(Ordering::Acquire) || self.stopped() {
            return false;
        }
        *self
            .clipboard_output
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(text.to_owned());
        self.clipboard_pending.store(true, Ordering::Release);
        self.ui.request_repaint();
        true // One pending native output; latest value supersedes stale text.
    }
    pub fn finish_clipboard(&self, focused: bool) {
        if !self.clipboard_pending.swap(false, Ordering::AcqRel) {
            return;
        }
        let text = self
            .clipboard_output
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if focused && self.clipboard_allowed.load(Ordering::Acquire) && !self.stopped() {
            if let Some(text) = text {
                self.ui.copy_text(text);
            }
        }
    }
    pub fn clipboard_read(&self) {
        self.ui.send_viewport_cmd_to(
            if self.stream() == 0 {
                eframe::egui::ViewportId::ROOT
            } else {
                crate::display_routes::viewport(self.stream())
            },
            eframe::egui::ViewportCommand::RequestPaste,
        );
        self.repaint();
    }
    pub fn controller_focus(&self, active: bool) {
        self.pad_focus.store(active, Ordering::Release);
    }
    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(consumer) = self.consumer.get() {
            consumer.unpark();
        }
    }

    pub fn push_session(&self, command: crate::session_controls::Command) {
        self.input.push(command);
        if let Some(consumer) = self.consumer.get() {
            consumer.unpark();
        }
    }
    pub fn push_input(&self, event: crate::desktop_input::Event) {
        self.input.push(event);
        if let Some(consumer) = self.consumer.get() {
            consumer.unpark();
        }
    }
    pub fn bind_input_consumer(&self) {
        let _ = self.consumer.set(std::thread::current());
    }
    pub fn set_input_geometry(&self, size: (u32, u32)) {
        self.input_geometry.store(
            (u64::from(size.0) << 32) | u64::from(size.1),
            Ordering::Release,
        );
    }
    pub fn input_geometry(&self) -> (u32, u32) {
        let size = self.input_geometry.load(Ordering::Acquire);
        ((size >> 32) as u32, size as u32)
    }
    pub fn set_input_ready(&self, ready: bool) {
        self.input_ready.store(ready, Ordering::Release);
    }
    pub fn input_ready(&self) -> bool {
        self.input_ready.load(Ordering::Acquire)
    }
    pub fn set_capture(&self, capture: bool) {
        if self.capture.swap(capture, Ordering::AcqRel) != capture {
            self.ui.request_repaint();
        }
    }
    pub fn host_cursor(&self, capture: bool, x: i32, y: i32) {
        self.cursor_state.store(
            (1 << 33)
                | (u64::from(capture) << 32)
                | (u64::from(x as u16) << 16)
                | u64::from(y as u16),
            Ordering::Release,
        );
        let mut restore = self
            .cursor_restore
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if self.capture() && !capture {
            *restore = Some((self.stream(), x, y));
        } else if capture {
            *restore = None;
        }
        self.set_capture(capture);
    }
    pub fn take_cursor_restore(&self) -> Option<(usize, i32, i32)> {
        self.cursor_restore
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }
    pub fn cursor_state(&self) -> Option<(bool, i32, i32)> {
        let state = self.cursor_state.load(Ordering::Acquire);
        (state & (1 << 33) != 0).then_some((
            state & (1 << 32) != 0,
            ((state >> 16) & 65535) as i32,
            (state & 65535) as i32,
        ))
    }
    pub fn set_video_rect(&self, rect: Option<eframe::egui::Rect>) {
        *self.video_rect.lock().unwrap_or_else(|e| e.into_inner()) = rect;
    }
    pub fn cursor_endpoint(&self, stream: usize, x: i32, y: i32) -> Option<eframe::egui::Pos2> {
        if stream != self.stream() {
            let displays = self.displays()?;
            let target = if stream == 0 {
                displays
                    .primary
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .upgrade()?
            } else {
                displays.get(stream)?.playback.clone()
            };
            return target.cursor_endpoint(stream, x, y);
        }
        let rect = (*self.video_rect.lock().unwrap_or_else(|e| e.into_inner()))?;
        let (x, y) = crate::viewport::Viewport {
            source: self.input_geometry(),
            client: (rect.width().round() as u32, rect.height().round() as u32),
        }
        .unmap(x, y)?;
        Some(rect.min + eframe::egui::vec2(x as f32, y as f32))
    }
    pub fn pointer_buttons(&self, buttons: u32) {
        let buttons = buttons & 31;
        let previous = self.pointer_buttons.swap(buttons, Ordering::AcqRel);
        if previous != 0 && buttons == 0 {
            let restore = *self
                .cursor_restore
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if let Some((stream, x, y)) = restore.filter(|(stream, _, _)| *stream != self.stream())
            {
                self.finish_peer_cursor(stream, x, y);
            }
        }
    }
    pub fn peer_cursor(&self) -> Option<(usize, i32, i32)> {
        (*self
            .cursor_restore
            .lock()
            .unwrap_or_else(|e| e.into_inner()))
        .filter(|(stream, _, _)| *stream != self.stream())
    }
    pub fn hide_peer_cursor(&self, stream: usize) {
        let mut restore = self
            .cursor_restore
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if restore.is_some_and(|(target, _, _)| target == stream && target != self.stream()) {
            *restore = None;
            drop(restore);
            self.repaint();
        }
    }
    fn finish_peer_cursor(&self, stream: usize, x: i32, y: i32) {
        self.cursor_state.store(
            (1 << 33) | (u64::from(x as u16) << 16) | u64::from(y as u16),
            Ordering::Release,
        );
        self.set_capture(false);
        crate::event_log::record(
            "INFO",
            "cursor_peer_drag_released",
            serde_json::json!({"stream":self.stream(),"visible_stream":stream}),
        );
        self.repaint();
    }
    pub fn release_peer_cursor(&self, stream: usize, x: i32, y: i32) {
        if stream == self.stream() || !self.capture() {
            return;
        }
        *self
            .cursor_restore
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some((stream, x, y));
        if self.pointer_buttons.load(Ordering::Acquire) == 0 {
            self.finish_peer_cursor(stream, x, y);
        }
        self.repaint();
        if let Some(displays) = self.displays() {
            let target = if stream == 0 {
                displays
                    .primary
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .upgrade()
            } else {
                displays.get(stream).map(|c| c.playback.clone())
            };
            if let Some(target) = target {
                target.repaint();
            }
        }
    }
    /// Only a host-reported visible peer of the active captured drag is drawn.
    /// The source retains raw motion until button-up; no early native warp.
    pub fn peer_drag_position(&self) -> Option<eframe::egui::Pos2> {
        let displays = self.displays()?;
        let source = displays.owner.load(Ordering::Acquire).checked_sub(1)?;
        let source = if source == 0 {
            displays
                .primary
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .upgrade()?
        } else {
            displays.get(source)?.playback.clone()
        };
        if !source.capture() || source.pointer_buttons.load(Ordering::Acquire) == 0 {
            return None;
        }
        let (stream, x, y) = source.peer_cursor()?;
        (stream == self.stream())
            .then(|| self.cursor_endpoint(stream, x, y))
            .flatten()
    }
    pub fn capture(&self) -> bool {
        self.capture.load(Ordering::Acquire)
    }

    pub fn submit_packet(&self, bytes: &[u8], info: PacketInfo) -> Result<bool> {
        self.submit_oriented(bytes, info, 0)
    }
    pub fn submit_oriented(&self, bytes: &[u8], info: PacketInfo, rotation: u8) -> Result<bool> {
        ensure!(rotation <= 3, "Invalid video rotation");
        let mut state = self.packets.lock().unwrap_or_else(|e| e.into_inner());
        state.check()?;
        ensure!(
            bytes.len() <= MAX_BYTES,
            "Linux encoded video exceeds the compressed video byte budget"
        );
        if info.parameters {
            state.parameters.observe(bytes, info.codec);
        }
        if self.stopped() {
            return Ok(false);
        }
        if state.suspended {
            state.queue.report.frames_dropped = state.queue.report.frames_dropped.saturating_add(1);
            return Ok(false);
        }
        if state.queue.report.queue_depth >= MAX_FRAMES
            || state.queue.report.queue_bytes + bytes.len() > MAX_BYTES
        {
            // The UI may be throttled while another window covers it. Retire
            // the entire obsolete reference chain, not arbitrary delta frames.
            // Do not start a recovery deadline until the UI can consume again.
            state.queue.clear();
            state.initialization = None;
            state.queue.report.frames_dropped = state.queue.report.frames_dropped.saturating_add(1);
            state.suspended = true;
            drop(state);
            self.ui.request_repaint();
            return Ok(false);
        }
        let mut buffer = state.queue.take_buffer();
        buffer.extend_from_slice(bytes);
        let accepted =
            state
                .queue
                .submit_native_codec(buffer, info.idr, info.parameters, 0, info.codec);
        if accepted {
            state.queue.set_last_packet(rotation, info.picture);
            if state.decoder_generation != state.queue.reset_generation
                && state.initialization.is_none()
            {
                // Snapshot at the first accepted packet, not at UI consumption:
                // later queued parameter updates must not initialize an older IDR.
                state.initialization = state.parameters.bytes();
            }
        }
        state.check()?;
        drop(state);
        if accepted {
            self.ui.request_repaint();
        }
        Ok(accepted)
    }

    pub fn encoder_rebuilt(&self) {
        let mut state = self.packets.lock().unwrap_or_else(|e| e.into_inner());
        if state.queue.encoder_rebuilt(Instant::now()) {
            state.initialization = None;
        }
    }

    /// A reference-chain restart is deferred while the compositor throttles
    /// the background UI. Audio and network/control polling remain independent.
    pub fn resume_if_ready(&self, focused: bool) -> Result<()> {
        let mut state = self.packets.lock().unwrap_or_else(|e| e.into_inner());
        state.check()?;
        if focused && state.suspended && !self.stopped() {
            state.suspended = false;
            state.queue.reset_generation = state.queue.reset_generation.wrapping_add(1);
            state.queue.report.recovery_reason = Some("presentation_suspended");
            // Existing shared IDR gate, request rate limit and five-second
            // deadline; a hidden window is not a decoder-failure attempt.
            state.queue.begin_format_change(Instant::now(), true);
        }
        Ok(())
    }

    pub fn take_keyframe_request(&self) -> Result<Option<u64>> {
        self.take_keyframe_request_at(Instant::now())
    }
    pub(crate) fn take_keyframe_request_at(&self, now: Instant) -> Result<Option<u64>> {
        let mut state = self.packets.lock().unwrap_or_else(|e| e.into_inner());
        if state.suspended || self.stopped() {
            return Ok(None);
        }
        let request = state
            .queue
            .take_keyframe_request(now)
            .map(|_| state.queue.reset_generation);
        state.check()?;
        Ok(request)
    }
    pub fn keyframe_request_failed(&self, generation: u64) {
        self.packets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .queue
            .request_failed(generation);
    }
    #[cfg(test)]
    pub fn status(&self) -> Option<&'static str> {
        let state = self.packets.lock().unwrap_or_else(|e| e.into_inner());
        if state.suspended {
            Some("Video paused: return to this window to resume. Audio remains active.")
        } else if state.queue.report.recovering {
            Some("Resuming video: waiting for a new keyframe...")
        } else {
            None
        }
    }

    /// Consume only the initial bounded backlog. Every accepted reference is
    /// decoded; normal presentation afterwards uses the newest output.
    #[cfg(test)]
    pub fn decode_pending(&self, mut decode: impl FnMut(&[u8]) -> Result<()>) -> Result<usize> {
        self.decode_pending_codec(|bytes, _| decode(bytes))
    }
    #[cfg(test)]
    pub fn decode_pending_codec(
        &self,
        mut decode: impl FnMut(&[u8], crate::video_stream::Codec) -> Result<()>,
    ) -> Result<usize> {
        self.decode_pending_oriented(|bytes, codec, _| decode(bytes, codec))
    }
    #[cfg(test)]
    pub fn decode_pending_oriented(
        &self,
        decode: impl FnMut(&[u8], crate::video_stream::Codec, u8) -> Result<()>,
    ) -> Result<usize> {
        self.decode_pending_limited(usize::MAX, decode)
    }

    #[cfg(test)]
    pub fn decode_pending_limited(
        &self,
        limit: usize,
        decode: impl FnMut(&[u8], crate::video_stream::Codec, u8) -> Result<()>,
    ) -> Result<usize> {
        let remaining = std::cell::Cell::new(limit);
        self.decode_pending_ready(
            || {
                let count = remaining.get();
                if count == 0 {
                    false
                } else {
                    remaining.set(count - 1);
                    true
                }
            },
            decode,
        )
    }
    pub fn decode_pending_ready(
        &self,
        mut ready: impl FnMut() -> bool,
        mut decode: impl FnMut(&[u8], crate::video_stream::Codec, u8) -> Result<()>,
    ) -> Result<usize> {
        let pending = self
            .packets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .queue
            .report
            .queue_depth;
        let mut consumed = 0;
        for _ in 0..pending {
            if !ready() {
                break;
            }
            if self.stopped() {
                break;
            }
            let (mut frame, generation) = {
                let mut state = self.packets.lock().unwrap_or_else(|e| e.into_inner());
                state.check()?;
                if state.suspended {
                    break;
                }
                let Some(mut frame) = state.queue.pop() else {
                    break;
                };
                let generation = state.queue.reset_generation;
                if state.decoder_generation != generation {
                    let mut initialization = state.initialization.take().ok_or_else(|| {
                        anyhow::anyhow!("Video resume requires cached codec parameters")
                    })?;
                    initialization.extend_from_slice(&frame.bytes);
                    state
                        .queue
                        .recycle(std::mem::replace(&mut frame.bytes, initialization));
                    state.decoder_generation = generation;
                }
                (frame, generation)
            };
            let started = Instant::now();
            let result = decode(&frame.bytes, frame.codec, frame.rotation);
            {
                let mut state = self.packets.lock().unwrap_or_else(|e| e.into_inner());
                state
                    .queue
                    .report
                    .local_timing
                    .decoder_service
                    .record(started.elapsed());
                if result.is_ok() {
                    state.queue.report.frames_decoded = state
                        .queue
                        .report
                        .frames_decoded
                        .saturating_add(u64::from(frame.picture));
                    if frame.idr && generation == state.queue.reset_generation && !state.suspended {
                        state.queue.recovered();
                    }
                }
                state.queue.recycle(std::mem::take(&mut frame.bytes));
            }
            result?;
            consumed += 1;
        }
        Ok(consumed)
    }

    #[cfg(test)]
    pub fn submit(&self, bytes: &[u8]) -> Result<()> {
        self.submit_packet(
            bytes,
            PacketInfo {
                picture: true,
                codec: crate::video_stream::Codec::H264,
                idr: true,
                parameters: false,
            },
        )
        .map(|_| ())
    }
}

impl crate::bud_ingress::VideoTarget for Playback {
    fn encoder_rebuilt(&self) {
        self.encoder_rebuilt();
    }
    fn submit_oriented(&self, bytes: &[u8], info: PacketInfo, rotation: u8) -> Result<bool> {
        self.submit_oriented(bytes, info, rotation)
    }
}

impl crate::video_output::NativeSink for Playback {
    fn submit(&self, bytes: &[u8], info: PacketInfo) -> bool {
        self.submit_packet(bytes, info).unwrap_or(false)
    }
    fn stop(&self) {
        Playback::stop(self);
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod rotation_tests {
    #[test]
    fn linux_rotation_stays_attached_to_its_encoded_packet() {
        let p = super::Playback::new(eframe::egui::Context::default());
        let info = crate::video_stream::PacketInfo {
            picture: true,
            codec: crate::video_stream::Codec::H264,
            idr: true,
            parameters: false,
        };
        p.submit_oriented(&[0, 0, 1, 0x65, 1], info, 3).unwrap();
        let mut rotations = Vec::new();
        p.decode_pending_oriented(|_, _, rotation| {
            rotations.push(rotation);
            Ok(())
        })
        .unwrap();
        assert_eq!(rotations, [3]);
    }
}

#[cfg(test)]
mod clipboard_output_tests {
    #[test]
    fn linux_pending_clipboard_is_latest_only_and_revoked_before_native_output() {
        let ctx = eframe::egui::Context::default();
        let p = super::Playback::new(ctx.clone());
        p.clipboard_allowed
            .store(true, std::sync::atomic::Ordering::Release);
        assert!(p.clipboard_write("one"));
        assert!(p.clipboard_write("two"));
        p.clipboard_allowed
            .store(false, std::sync::atomic::Ordering::Release);
        let out = ctx.run(Default::default(), |_| p.finish_clipboard(true));
        assert!(out.platform_output.commands.is_empty());
        p.clipboard_allowed
            .store(true, std::sync::atomic::Ordering::Release);
        p.clipboard_write("three");
        p.clipboard_write("four");
        let out = ctx.run(Default::default(), |_| p.finish_clipboard(true));
        assert!(
            matches!(out.platform_output.commands.as_slice(),[eframe::egui::OutputCommand::CopyText(text)] if text=="four")
        );
    }
}
