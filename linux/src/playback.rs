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

pub struct Playback {
    stopped: AtomicBool,
    packets: Mutex<Pending>,
    ui: eframe::egui::Context,
    pub(crate) input: crate::input_handoff::Inbox,
    consumer: OnceLock<std::thread::Thread>,
    input_geometry: AtomicU64,
    input_ready: AtomicBool,
    capture: AtomicBool,
    cursor_restore: Mutex<Option<(i32, i32)>>,
}

impl Playback {
    pub fn new(ui: eframe::egui::Context) -> Arc<Self> {
        Arc::new(Self {
            stopped: AtomicBool::new(false),
            packets: Mutex::new(Pending::default()),
            ui,
            input: Default::default(),
            consumer: OnceLock::new(),
            input_geometry: AtomicU64::new(0),
            input_ready: AtomicBool::new(false),
            capture: AtomicBool::new(false),
            cursor_restore: Mutex::new(None),
        })
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
        let mut restore = self
            .cursor_restore
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if self.capture() && !capture {
            *restore = Some((x, y));
        } else if capture {
            *restore = None;
        }
        self.set_capture(capture);
    }
    pub fn take_cursor_restore(&self) -> Option<(i32, i32)> {
        self.cursor_restore
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }
    pub fn capture(&self) -> bool {
        self.capture.load(Ordering::Acquire)
    }

    pub fn submit_packet(&self, bytes: &[u8], info: PacketInfo) -> Result<bool> {
        let mut state = self.packets.lock().unwrap_or_else(|e| e.into_inner());
        state.check()?;
        ensure!(
            bytes.len() <= crate::control::MAX_CHANNEL_MESSAGE,
            "Linux video input exceeds the shared message limit"
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
        let accepted = state
            .queue
            .submit_codec(buffer, info.idr, info.parameters, 0, info.codec);
        if accepted {
            state.queue.set_last_packet(0, info.picture);
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
    pub fn decode_pending(&self, mut decode: impl FnMut(&[u8]) -> Result<()>) -> Result<usize> {
        let pending = self
            .packets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .queue
            .report
            .queue_depth;
        let mut consumed = 0;
        for _ in 0..pending {
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
            let result = decode(&frame.bytes);
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
