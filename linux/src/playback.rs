//! Bounded compressed-input handoff for the Linux video adapter.
use anyhow::{Result, ensure};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

pub struct Playback {
    stopped: AtomicBool,
    packets: Mutex<(VecDeque<Vec<u8>>, usize)>,
    ui: eframe::egui::Context,
}

impl Playback {
    pub fn new(ui: eframe::egui::Context) -> Arc<Self> {
        Arc::new(Self {
            stopped: AtomicBool::new(false),
            packets: Mutex::new((VecDeque::new(), 0)),
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
            packets.0.len() < 8
                && bytes.len() <= 8 * 1024 * 1024
                && packets.1 + bytes.len() <= 16 * 1024 * 1024,
            "The bounded compressed video queue filled; reconnect to recover references"
        );
        packets.1 += bytes.len();
        packets.0.push_back(bytes.to_vec());
        drop(packets);
        self.ui.request_repaint();
        Ok(())
    }
    fn next(&self) -> Option<Vec<u8>> {
        let mut packets = self.packets.lock().unwrap_or_else(|e| e.into_inner());
        let packet = packets.0.pop_front()?;
        packets.1 -= packet.len();
        Some(packet)
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
            .0
            .len();
        let mut consumed = 0;
        for _ in 0..pending {
            if self.stopped() {
                break;
            }
            let Some(packet) = self.next() else { break };
            // No queue lock is held while executing native codec operations.
            decode(&packet)?;
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
