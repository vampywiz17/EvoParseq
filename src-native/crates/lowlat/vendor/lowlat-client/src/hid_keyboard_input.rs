//! Physical HID usages. Character interpretation belongs to the host layout.
use super::Report;
use std::collections::VecDeque;

pub(super) const ID: u32 = 0xffff_f005;
pub(super) const BYTES: usize = 29;
pub(super) const DEVICE: &[u8] = &[
    18, 1, 0, 2, 0, 0, 0, 64, 0xff, 0xff, 5, 0xf0, 0, 1, 1, 2, 3, 1,
];
// Report protocol: eight modifiers and 224 key bits; no six-key limit.
// Standard LED output is delivered through control SET_REPORT, not an OUT EP.
pub(super) const REPORT: &[u8] = &[
    0x05, 1, 0x09, 6, 0xa1, 1, 0x05, 7, 0x19, 0xe0, 0x29, 0xe7, 0x15, 0, 0x25, 1, 0x75, 1, 0x95, 8,
    0x81, 2, 0x19, 0, 0x29, 0xdf, 0x75, 1, 0x95, 0xe0, 0x81, 2, 0x05, 8, 0x19, 1, 0x29, 5, 0x95, 5,
    0x75, 1, 0x91, 2, 0x95, 1, 0x75, 3, 0x91, 3, 0xc0,
];
pub(super) const CONFIG: &[u8] = &[
    9,
    2,
    34,
    0,
    1,
    1,
    0,
    0x80,
    50,
    9,
    4,
    0,
    0,
    1,
    3,
    0,
    0,
    0,
    9,
    0x21,
    0x11,
    1,
    0,
    1,
    0x22,
    REPORT.len() as u8,
    0,
    7,
    5,
    0x81,
    3,
    32,
    0,
    4,
];

#[derive(Debug, Default)]
pub(super) struct Input {
    reports: VecDeque<[u8; BYTES]>,
    keys: [u8; BYTES],
    sent: [u8; BYTES],
    ready: bool,
    failed: bool,
    last_sent: Option<u64>,
    pub(super) leds: u8,
    pub(super) idle: u8,
}
impl Input {
    pub(super) fn ready(&self) -> bool {
        self.ready && !self.failed
    }
    pub(super) fn failed(&self) -> bool {
        self.failed
    }
    pub(super) fn start(&mut self) {
        if !self.ready {
            self.reports.push_back([0; BYTES]);
        }
        self.ready = true;
    }
    pub(super) fn report(&self) -> [u8; BYTES] {
        self.keys
    }
    pub(super) fn command(&mut self, opcode: u8, usage: u32, _mods: u32, pressed: u32) {
        if opcode == 24 {
            self.reset();
            return;
        }
        if opcode != 0 || !self.ready() || pressed > 1 {
            return;
        }
        // Zero/rollover usages are not physical keys. No Unicode-to-key guesses.
        if !(4..=231).contains(&usage) {
            return;
        }
        let (index, bit) = if usage >= 224 {
            (0, (usage - 224) as u8)
        } else {
            (1 + usage as usize / 8, (usage % 8) as u8)
        };
        let before = self.keys;
        if pressed != 0 {
            self.keys[index] |= 1 << bit;
        } else {
            self.keys[index] &= !(1 << bit);
        }
        // The host keyboard stack repeats held keys. Ignore client typematic.
        if self.keys == before {
            return;
        }
        if self.reports.len() >= 128 {
            self.failed = true;
            return;
        }
        self.reports.push_back(self.keys);
    }
    pub(super) fn next(&mut self, now: u64) -> Option<Report> {
        if self.failed || self.last_sent.is_some_and(|t| now.saturating_sub(t) < 4) {
            return None;
        }
        let keys = if let Some(keys) = self.reports.pop_front() {
            keys
        } else if self.idle != 0
            && self
                .last_sent
                .is_some_and(|t| now.saturating_sub(t) >= u64::from(self.idle) * 4)
        {
            self.keys
        } else {
            return None;
        };
        self.sent = keys;
        self.last_sent = Some(now);
        Some(Report::new(&keys))
    }
    pub(super) fn reset(&mut self) {
        self.keys = [0; BYTES];
        self.reports.clear();
        if self.sent != [0; BYTES] {
            self.reports.push_back(self.keys);
        }
    }
    pub(super) fn release(&mut self) -> Option<Report> {
        self.reset();
        if self.sent == [0; BYTES] {
            return None;
        }
        self.sent = [0; BYTES];
        self.reports.clear();
        Some(Report::new(&self.sent))
    }
}
