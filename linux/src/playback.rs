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
    pub fn next(&self) -> Option<Vec<u8>> {
        let mut packets = self.packets.lock().unwrap_or_else(|e| e.into_inner());
        let packet = packets.0.pop_front()?;
        packets.1 -= packet.len();
        Some(packet)
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        self.stop();
    }
}
