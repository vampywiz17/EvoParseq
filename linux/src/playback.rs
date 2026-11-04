//! Bounded compressed-input handoff for the Linux video adapter.
use crate::video_output::{MAX_BYTES, MAX_FRAMES};
use anyhow::{Result, ensure};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

struct Packet {
    bytes: Vec<u8>,
    queued: Instant,
}

struct Pending {
    packets: VecDeque<Packet>,
    bytes: usize,
    received: u64,
    received_bytes: u64,
    decoded: u64,
    began: Instant,
    active: Option<Instant>,
    last_decode: Duration,
    peak_decode: Duration,
    peak_wait: Duration,
}

impl Default for Pending {
    fn default() -> Self {
        Self {
            packets: VecDeque::new(),
            bytes: 0,
            received: 0,
            received_bytes: 0,
            decoded: 0,
            began: Instant::now(),
            active: None,
            last_decode: Duration::ZERO,
            peak_decode: Duration::ZERO,
            peak_wait: Duration::ZERO,
        }
    }
}

impl Pending {
    fn failure(&self, incoming: usize, now: Instant) -> String {
        let reason = if incoming > 8 * 1024 * 1024 {
            "input size"
        } else if self.bytes + incoming > MAX_BYTES {
            "byte limit"
        } else {
            "packet limit"
        };
        let oldest = self
            .packets
            .front()
            .map_or(Duration::ZERO, |p| now.saturating_duration_since(p.queued));
        let active = self
            .active
            .map(|start| now.saturating_duration_since(start).as_secs_f64() * 1000.0);
        let elapsed = now.saturating_duration_since(self.began).as_secs_f64();
        let bitrate = if elapsed > 0.0 {
            self.received_bytes as f64 * 8.0 / elapsed / 1_000_000.0
        } else {
            0.0
        };
        let arrival_span = self
            .packets
            .front()
            .zip(self.packets.back())
            .map_or(Duration::ZERO, |(first, last)| {
                last.queued.saturating_duration_since(first.queued)
            });
        format!(
            "Video queue full ({reason}): {} packets, {} KiB + {} KiB incoming; oldest {:.1} ms, peak wait {:.1} ms, arrival span {:.1} ms; received {}, decoded {}; decode last/peak {:.1}/{:.1} ms, active {} ms; elapsed {:.1} s, average video input {:.1} Mbit/s",
            self.packets.len(),
            self.bytes / 1024,
            incoming / 1024,
            oldest.as_secs_f64() * 1000.0,
            self.peak_wait.as_secs_f64() * 1000.0,
            arrival_span.as_secs_f64() * 1000.0,
            self.received,
            self.decoded,
            self.last_decode.as_secs_f64() * 1000.0,
            self.peak_decode.as_secs_f64() * 1000.0,
            active.map_or_else(|| "none".into(), |ms| format!("{ms:.1}")),
            elapsed,
            bitrate
        )
    }
}

pub struct Playback {
    stopped: AtomicBool,
    packets: Mutex<Pending>,
    ui: eframe::egui::Context,
}

impl Playback {
    pub fn new(ui: eframe::egui::Context) -> Arc<Self> {
        Arc::new(Self {
            stopped: AtomicBool::new(false),
            packets: Mutex::new(Pending::default()),
            ui,
        })
    }
    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
    }
    pub fn submit(&self, bytes: &[u8]) -> Result<()> {
        let mut packets = self.packets.lock().unwrap_or_else(|e| e.into_inner());
        ensure!(
            packets.packets.len() < MAX_FRAMES
                && bytes.len() <= 8 * 1024 * 1024
                && packets.bytes + bytes.len() <= MAX_BYTES,
            "{}",
            packets.failure(bytes.len(), Instant::now())
        );
        packets.bytes += bytes.len();
        packets.received += 1;
        packets.received_bytes = packets.received_bytes.saturating_add(bytes.len() as u64);
        packets.packets.push_back(Packet {
            bytes: bytes.to_vec(),
            queued: Instant::now(),
        });
        drop(packets);
        self.ui.request_repaint();
        Ok(())
    }
    fn next(&self) -> Option<Vec<u8>> {
        let mut packets = self.packets.lock().unwrap_or_else(|e| e.into_inner());
        let packet = packets.packets.pop_front()?;
        let now = Instant::now();
        packets.bytes -= packet.bytes.len();
        packets.peak_wait = packets
            .peak_wait
            .max(now.saturating_duration_since(packet.queued));
        packets.active = Some(now);
        Some(packet.bytes)
    }

    /// Decode the bounded backlog present at the start of this UI update.
    /// Presentation happens afterwards, so it cannot throttle decoding to one
    /// packet per display refresh. New arrivals wait for the next update;
    /// compressed references are never discarded to catch up.
    pub fn decode_pending(&self, mut decode: impl FnMut(&[u8]) -> Result<()>) -> Result<usize> {
        let pending = self
            .packets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .packets
            .len();
        let mut consumed = 0;
        for _ in 0..pending {
            if self.stopped() {
                break;
            }
            let Some(packet) = self.next() else { break };
            // No queue lock is held while executing native codec operations.
            let result = decode(&packet);
            {
                let mut state = self.packets.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(start) = state.active.take() {
                    state.last_decode = start.elapsed();
                    state.peak_decode = state.peak_decode.max(state.last_decode);
                }
                state.decoded += u64::from(result.is_ok());
            }
            result?;
            consumed += 1;
        }
        Ok(consumed)
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        self.stop();
    }
}
