//! USB HID device state shared by Phaze platform adapters.
use crate::Input;
use std::collections::VecDeque;

pub(crate) const MOUSE_ID: u32 = 1;
pub(crate) const KEYBOARD_ID: u32 = 2;
// USB-IF reserved testing identity. Never impersonate a physical peripheral.
pub(crate) const VID: u16 = 0xffff;
const MAX_REPORTS: usize = 128;
const MAX_READS: usize = 64;
const STALL: i32 = -32;

// HID 1.11 report descriptors. Mouse report 1 is position only. Report 2
// owns the buttons in both coordinate modes, including during drags.
const MOUSE_REPORT: &[u8] = &[
    0x05, 1, 0x09, 2, 0xa1, 1, 0x85, 1, 0x05, 1, 0x09, 0x30, 0x09, 0x31, 0x15, 0, 0x26, 0xff, 0x7f,
    0x75, 16, 0x95, 2, 0x81, 2, 0x85, 2, 0x05, 9, 0x19, 1, 0x29, 5, 0x15, 0, 0x25, 1, 0x75, 1,
    0x95, 5, 0x81, 2, 0x75, 3, 0x95, 1, 0x81, 3, 0x05, 1, 0x09, 0x30, 0x09, 0x31, 0x16, 1, 0x80,
    0x26, 0xff, 0x7f, 0x75, 16, 0x95, 2, 0x81, 6, 0x09, 0x38, 0x15, 0x81, 0x25, 127, 0x75, 8, 0x95,
    1, 0x81, 6, 0x05, 0x0c, 0x0a, 0x38, 2, 0x81, 6, 0xc0,
];
const KEYBOARD_REPORT: &[u8] = &[
    0x05, 1, 0x09, 6, 0xa1, 1, 0x05, 7, 0x19, 0xe0, 0x29, 0xe7, 0x15, 0, 0x25, 1, 0x75, 1, 0x95, 8,
    0x81, 2, 0x19, 0, 0x29, 0xdf, 0x95, 0xe0, 0x81, 2, 0x05, 8, 0x19, 1, 0x29, 5, 0x95, 5, 0x91, 2,
    0x95, 3, 0x91, 3, 0xc0,
];

#[derive(Clone, Copy)]
struct Report {
    bytes: [u8; 29],
    len: usize,
}

impl Report {
    fn new(bytes: &[u8]) -> Self {
        let mut report = Self {
            bytes: [0; 29],
            len: bytes.len(),
        };
        report.bytes[..bytes.len()].copy_from_slice(bytes);
        report
    }

    fn data(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

pub(crate) struct Device {
    pub(crate) id: u32,
    pub(crate) opened: bool,
    configured: bool,
    last_report: Report,
    keys: [u8; 29],
    buttons: u8,
    wheel: [i32; 2],
    idle: u8,
    reports: VecDeque<Report>,
    reads: VecDeque<[u8; 33]>,
}

impl Device {
    pub(crate) fn new(id: u32) -> Self {
        let mut device = Self {
            id,
            opened: false,
            configured: false,
            last_report: Report::new(&[]),
            keys: [0; 29],
            buttons: 0,
            wheel: [0; 2],
            idle: 0,
            reports: VecDeque::new(),
            reads: VecDeque::new(),
        };
        device.release();
        device
    }

    pub(crate) fn name(&self) -> &'static str {
        if self.id == MOUSE_ID {
            "EvoParseq Mouse"
        } else {
            "EvoParseq Keyboard"
        }
    }

    pub(crate) fn product(&self) -> u16 {
        0xe000 + self.id as u16
    }

    fn descriptor(&self) -> &'static [u8] {
        if self.id == MOUSE_ID {
            MOUSE_REPORT
        } else {
            KEYBOARD_REPORT
        }
    }

    pub(crate) fn device_descriptor(&self) -> [u8; 18] {
        let mut bytes = [18, 1, 0, 2, 0, 0, 0, 64, 0, 0, 0, 0, 0, 1, 1, 2, 0, 1];
        bytes[8..10].copy_from_slice(&VID.to_le_bytes());
        bytes[10..12].copy_from_slice(&self.product().to_le_bytes());
        bytes
    }

    pub(crate) fn configuration(&self) -> [u8; 34] {
        let mut bytes = [
            9, 2, 34, 0, 1, 1, 0, 0x80, 50, 9, 4, 0, 0, 1, 3, 0, 0, 0, 9, 0x21, 0x11, 1, 0, 1,
            0x22, 0, 0, 7, 5, 0x81, 3, 32, 0, 4,
        ];
        bytes[25..27].copy_from_slice(&(self.descriptor().len() as u16).to_le_bytes());
        bytes
    }

    pub(crate) fn open(&mut self) {
        self.close();
        self.opened = true;
    }

    pub(crate) fn close(&mut self) {
        self.opened = false;
        self.configured = false;
        self.reads.clear();
        self.idle = 0;
        self.release();
    }

    pub(crate) fn release(&mut self) {
        self.keys.fill(0);
        self.buttons = 0;
        self.wheel = [0; 2];
        self.reports.clear();
        self.last_report = if self.id == MOUSE_ID {
            Report::new(&[2, 0, 0, 0, 0, 0, 0, 0])
        } else {
            Report::new(&self.keys)
        };
        self.reports.push_back(self.last_report);
    }

    pub(crate) fn input(&mut self, input: Input) {
        if !self.opened {
            return;
        }
        if self.reports.len() == MAX_REPORTS {
            self.release();
            return;
        }
        let report = match input {
            Input::Key { usage, down, .. } => {
                let (index, bit) = match usage {
                    0xe0..=0xe7 => (0, usage - 0xe0),
                    4..=0xdf => (1 + usize::from(usage / 8), usage % 8),
                    _ => return,
                };
                let old = self.keys[index];
                self.keys[index] = (old & !(1 << bit)) | (u8::from(down) << bit);
                if self.keys[index] == old {
                    return;
                }
                Report::new(&self.keys)
            }
            Input::Motion {
                x,
                y,
                relative: false,
            } => {
                let mut bytes = [1, 0, 0, 0, 0];
                bytes[1..3].copy_from_slice(&((x.clamp(0, 65535) / 2) as u16).to_le_bytes());
                bytes[3..5].copy_from_slice(&((y.clamp(0, 65535) / 2) as u16).to_le_bytes());
                if self.reports.back().is_some_and(|report| report.len == 5) {
                    self.reports.pop_back();
                }
                Report::new(&bytes)
            }
            Input::Motion {
                x,
                y,
                relative: true,
            } => self.mouse_report(x, y, 0, 0),
            Input::Button { button, down, .. } => {
                let bit = match button {
                    1 => 0,
                    3 => 1,
                    2 => 2,
                    4 => 3,
                    5 => 4,
                    _ => return,
                };
                self.buttons = (self.buttons & !(1 << bit)) | (u8::from(down) << bit);
                self.mouse_report(0, 0, 0, 0)
            }
            Input::Scroll { x, y } => {
                self.wheel[0] = self.wheel[0].saturating_add(x);
                self.wheel[1] = self.wheel[1].saturating_sub(y);
                let [x, y] = self.wheel.map(|units| (units / 120).clamp(-127, 127));
                if x == 0 && y == 0 {
                    return;
                }
                self.wheel[0] -= x * 120;
                self.wheel[1] -= y * 120;
                self.mouse_report(0, 0, x, y)
            }
            Input::Release => {
                self.release();
                return;
            }
        };
        self.reports.push_back(report);
    }

    fn mouse_report(&self, x: i32, y: i32, horizontal: i32, vertical: i32) -> Report {
        let axis = |value: i32| (value.clamp(-32767, 32767) as i16).to_le_bytes();
        let [xl, xh] = axis(x);
        let [yl, yh] = axis(y);
        Report::new(&[
            2,
            self.buttons,
            xl,
            xh,
            yl,
            yh,
            vertical as u8,
            horizontal as u8,
        ])
    }

    pub(crate) fn enqueue_read(&mut self, req: [u8; 33]) -> bool {
        if self.opened && self.reads.len() < MAX_READS {
            self.reads.push_back(req);
            true
        } else {
            false
        }
    }

    pub(crate) fn complete(&mut self) -> Option<([u8; 33], Vec<u8>)> {
        if self.reads.is_empty() {
            return None;
        }
        let report = self.reports.pop_front()?;
        let request = self.reads.pop_front()?;
        self.last_report = report;
        let mut payload = vec![0; 4];
        payload.extend_from_slice(report.data());
        Some((request, payload))
    }

    pub(crate) fn control(&mut self, setup: [u8; 8], data: &[u8]) -> Vec<u8> {
        let value = u16::from_le_bytes([setup[2], setup[3]]);
        let [index, descriptor] = value.to_le_bytes();
        let interface = u16::from_le_bytes([setup[4], setup[5]]);
        let length = u16::from_le_bytes([setup[6], setup[7]]) as usize;
        let descriptor_data = match (setup[0], setup[1]) {
            (0x80 | 0x81, 6) => match descriptor {
                1 if index == 0 => Some(self.device_descriptor().to_vec()),
                2 if index == 0 => Some(self.configuration().to_vec()),
                3 => match index {
                    0 => Some(vec![4, 3, 9, 4]),
                    1 => Some(string_descriptor("EvoParseq")),
                    2 => Some(string_descriptor(self.name())),
                    _ => None,
                },
                0x21 if index == 0 && interface == 0 => Some(self.configuration()[18..27].to_vec()),
                0x22 if index == 0 && interface == 0 => Some(self.descriptor().to_vec()),
                _ => None,
            },
            (0x80..=0x82, 0) => Some(vec![0, 0]),
            (0, 5) if value <= 127 => Some(Vec::new()),
            (0, 9) if value <= 1 => {
                self.configured = value == 1;
                if !self.configured {
                    self.release();
                }
                Some(Vec::new())
            }
            (0x80, 8) => Some(vec![u8::from(self.configured)]),
            (0x81, 10) if interface == 0 => Some(vec![0]),
            (1, 11) if value == 0 && interface == 0 => Some(Vec::new()),
            (0x21, 10) if interface == 0 => {
                self.idle = descriptor;
                Some(Vec::new())
            }
            (0xa1, 2) if interface == 0 => Some(vec![self.idle]),
            (0xa1, 1) if interface == 0 && descriptor == 1 => {
                let mouse_position = self.id == MOUSE_ID && index == 1;
                Some(if mouse_position {
                    vec![1, 0, 0, 0, 0]
                } else {
                    self.last_report.data().to_vec()
                })
            }
            (0x21, 9)
                if self.id == KEYBOARD_ID
                    && interface == 0
                    && value == 0x0200
                    && data.len() == 1
                    && length == 1 =>
            {
                Some(Vec::new())
            }
            _ => None,
        };
        let mut payload = setup.to_vec();
        let status = if descriptor_data.is_some() { 0 } else { STALL };
        payload.extend_from_slice(&status.to_le_bytes());
        let mut bytes = descriptor_data.unwrap_or_default();
        bytes.truncate(length);
        let transferred = if setup[0] & 0x80 == 0 && status == 0 {
            data.len()
        } else {
            bytes.len()
        };
        payload.extend_from_slice(&(transferred as u32).to_le_bytes());
        payload.extend(bytes);
        payload
    }
}

fn string_descriptor(text: &str) -> Vec<u8> {
    let mut bytes = vec![0, 3];
    bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
    bytes[0] = bytes.len() as u8;
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_supply_neutral_reports_then_pressed_and_released_state() {
        let mut keyboard = Device::new(KEYBOARD_ID);
        keyboard.open();
        for _ in 0..3 {
            assert!(keyboard.enqueue_read([0; 33]));
        }
        assert_eq!(keyboard.complete().unwrap().1, [0; 33]);
        for down in [true, false] {
            keyboard.input(Input::Key {
                usage: 4,
                scan: 30,
                modifiers: 0,
                down,
            });
            let completion = keyboard.complete().unwrap().1;
            assert_eq!(completion[5], u8::from(down) << 4);
        }
        let mut mouse = Device::new(MOUSE_ID);
        mouse.open();
        assert!(mouse.enqueue_read([0; 33]));
        assert_eq!(&mouse.complete().unwrap().1[4..], &[2, 0, 0, 0, 0, 0, 0, 0]);
        assert!(mouse.enqueue_read([0; 33]));
        mouse.input(Input::Button {
            button: 3,
            down: true,
            x: 0,
            y: 0,
        });
        assert_eq!(mouse.complete().unwrap().1[5], 2);
    }
}
