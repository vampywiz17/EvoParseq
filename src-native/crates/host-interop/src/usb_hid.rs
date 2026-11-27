//! USB HID device state shared by Phaze platform adapters.
use crate::Input;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

pub(crate) const MOUSE_ID: u32 = 1;
pub(crate) const KEYBOARD_ID: u32 = 2;
// Synthetic identity, following the existing EvoParseq LOWLAT HID convention.
pub(crate) const VID: u16 = 0xffff;
const MAX_REPORTS: usize = 128;
const MAX_READS: usize = 64;
const STALL: i32 = -32;

use lowlat_core::hid::MOUSE_REPORT;
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
    pub(crate) announced: bool,
    configured: bool,
    position: Report,
    last_sent: Option<Instant>,
    keys: [u8; 29],
    buttons: u8,
    wheel: [i32; 2],
    idle: u8,
    leds: u8,
    reports: VecDeque<Report>,
    reads: VecDeque<[u8; 33]>,
}

impl Device {
    pub(crate) fn new(id: u32) -> Self {
        let mut device = Self {
            id,
            opened: false,
            announced: false,
            configured: false,
            position: Report::new(&[1, 0, 0, 0, 0]),
            last_sent: None,
            keys: [0; 29],
            buttons: 0,
            wheel: [0; 2],
            idle: 0,
            leds: 0,
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
        self.leds = 0;
        self.last_sent = None;
        self.release();
    }

    pub(crate) fn release(&mut self) {
        self.keys.fill(0);
        self.buttons = 0;
        self.wheel = [0; 2];
        self.reports.clear();
        self.reports.push_back(self.state_report());
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
                self.position = Report::new(&bytes);
                self.position
            }
            Input::Motion {
                x,
                y,
                relative: true,
            } => self.mouse_report(x, y, 0, 0),
            Input::Button { button, down, .. } => {
                if !(1..=5).contains(&button) {
                    return;
                }
                let bit = button - 1;
                self.buttons = (self.buttons & !(1 << bit)) | (u8::from(down) << bit);
                self.mouse_report(0, 0, 0, 0)
            }
            Input::Scroll { x, y } => {
                self.wheel[0] = self.wheel[0].saturating_add(x);
                self.wheel[1] = self.wheel[1].saturating_add(y);
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
            3,
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

    fn state_report(&self) -> Report {
        if self.id == MOUSE_ID {
            self.mouse_report(0, 0, 0, 0)
        } else {
            Report::new(&self.keys)
        }
    }

    pub(crate) fn complete(&mut self) -> Option<([u8; 33], Vec<u8>, bool)> {
        if self.reads.is_empty() {
            return None;
        }
        let report = self.reports.pop_front().or_else(|| {
            (self.id == KEYBOARD_ID
                && self.idle != 0
                && self.last_sent.is_some_and(|time| {
                    time.elapsed() >= Duration::from_millis(u64::from(self.idle) * 4)
                }))
            .then(|| self.state_report())
        })?;
        let request = self.reads.pop_front()?;
        let initial = self.last_sent.is_none();
        if initial || self.id == KEYBOARD_ID && self.idle != 0 {
            self.last_sent = Some(Instant::now());
        }
        // The Phaze interrupt completion uses a signed transfer count.
        // Nonnegative values are the number of data bytes, not a status code.
        let mut payload = (report.len as i32).to_le_bytes().to_vec();
        payload.extend_from_slice(report.data());
        Some((request, payload, initial))
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
            (0xa1, 1) if interface == 0 && descriptor == 1 => match (self.id, index) {
                (MOUSE_ID, 1) => Some(self.position.data().to_vec()),
                (MOUSE_ID, 3) | (KEYBOARD_ID, 0) => Some(self.state_report().data().to_vec()),
                _ => None,
            },
            (0xa1, 1) if self.id == KEYBOARD_ID && interface == 0 && value == 0x0200 => {
                Some(vec![self.leds])
            }
            (0x21, 9)
                if self.id == KEYBOARD_ID
                    && interface == 0
                    && value == 0x0200
                    && data.len() == 1
                    && length == 1 =>
            {
                self.leds = data[0] & 0x1f;
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
        let neutral = keyboard.complete().unwrap().1;
        assert_eq!(&neutral[..4], &29i32.to_le_bytes());
        assert_eq!(&neutral[4..], &[0; 29]);
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
        assert_eq!(&mouse.complete().unwrap().1[4..], &[3, 0, 0, 0, 0, 0, 0, 0]);
        for (button, mask) in [(1, 1), (2, 2), (3, 4)] {
            for down in [true, false] {
                assert!(mouse.enqueue_read([0; 33]));
                mouse.input(Input::Button {
                    button,
                    down,
                    x: 0,
                    y: 0,
                });
                assert_eq!(mouse.complete().unwrap().1[5], if down { mask } else { 0 });
            }
        }
        for direction in [1, -1] {
            assert!(mouse.enqueue_read([0; 33]));
            mouse.input(Input::Scroll {
                x: 0,
                y: direction * 120,
            });
            assert_eq!(mouse.complete().unwrap().1[10], direction as u8);
        }
    }

    fn setup(request_type: u8, request: u8, value: u16, interface: u16, length: u16) -> [u8; 8] {
        let [value_low, value_high] = value.to_le_bytes();
        let [interface_low, interface_high] = interface.to_le_bytes();
        let [length_low, length_high] = length.to_le_bytes();
        [
            request_type,
            request,
            value_low,
            value_high,
            interface_low,
            interface_high,
            length_low,
            length_high,
        ]
    }

    fn status(payload: &[u8]) -> i32 {
        i32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]])
    }

    fn data(payload: &[u8]) -> &[u8] {
        &payload[16..]
    }

    #[test]
    fn control_transfers_serve_descriptors_configuration_and_indicator_state() {
        let mut keyboard = Device::new(KEYBOARD_ID);
        let mut mouse = Device::new(MOUSE_ID);

        let descriptor = mouse.control(setup(0x80, 6, 0x0100, 0, 18), &[]);
        assert_eq!(status(&descriptor), 0);
        assert_eq!(data(&descriptor), mouse.device_descriptor().as_slice());

        let configuration = mouse.control(setup(0x80, 6, 0x0200, 0, 34), &[]);
        assert_eq!(data(&configuration), mouse.configuration().as_slice());

        let hid = mouse.control(setup(0x81, 6, 0x2100, 0, 9), &[]);
        assert_eq!(data(&hid), &mouse.configuration()[18..27]);

        let report = mouse.control(setup(0x81, 6, 0x2200, 0, 4), &[]);
        assert_eq!(data(&report), &mouse.descriptor()[..4]);

        let string = mouse.control(setup(0x81, 6, 0x0300, 0, 4), &[]);
        assert_eq!(data(&string), &[4u8, 3, 9, 4][..]);

        let language = string_descriptor("EvoParseq");
        let read = mouse.control(setup(0x81, 6, 0x0301, 0, language.len() as u16), &[]);
        assert_eq!(data(&read), language.as_slice());

        let product = string_descriptor(mouse.name());
        let read = mouse.control(setup(0x81, 6, 0x0302, 0, product.len() as u16), &[]);
        assert_eq!(data(&read), product.as_slice());

        let unknown = mouse.control(setup(0x81, 6, 0x2300, 0, 4), &[]);
        assert_eq!(status(&unknown), -32);

        let status_read = mouse.control(setup(0x82, 0, 0, 0, 2), &[]);
        assert_eq!(data(&status_read), &[0u8, 0][..]);

        let address = mouse.control(setup(0, 5, 1, 0, 0), &[]);
        assert_eq!(status(&address), 0);

        let configured = mouse.control(setup(0, 9, 1, 0, 0), &[]);
        assert_eq!(status(&configured), 0);
        let current = mouse.control(setup(0x80, 8, 0, 0, 1), &[]);
        assert_eq!(data(&current), &[1u8][..]);

        mouse.control(setup(0, 9, 0, 0, 0), &[]);
        let current = mouse.control(setup(0x80, 8, 0, 0, 1), &[]);
        assert_eq!(data(&current), &[0u8][..]);

        let interface = mouse.control(setup(0x81, 10, 0, 0, 1), &[]);
        assert_eq!(data(&interface), &[0u8][..]);
        let interface_out = mouse.control(setup(1, 11, 0, 0, 0), &[]);
        assert_eq!(status(&interface_out), 0);

        keyboard.control(setup(0x21, 10, 0x7d00, 0, 0), &[]);
        let idle = keyboard.control(setup(0xa1, 2, 0, 0, 1), &[]);
        assert_eq!(data(&idle), &[125u8][..]);

        let missing = keyboard.control(setup(0xa1, 1, 0x0101, 0, 29), &[]);
        assert_eq!(status(&missing), -32);
        let keyboard_state = keyboard.control(setup(0xa1, 1, 0x0100, 0, 29), &[]);
        assert_eq!(data(&keyboard_state), &[0u8; 29][..]);
        let mouse_position = mouse.control(setup(0xa1, 1, 0x0101, 0, 5), &[]);
        assert_eq!(data(&mouse_position), &[1u8, 0, 0, 0, 0][..]);
        let mouse_state = mouse.control(setup(0xa1, 1, 0x0103, 0, 8), &[]);
        assert_eq!(data(&mouse_state).len(), 8);

        let leds = keyboard.control(setup(0xa1, 1, 0x0200, 0, 1), &[]);
        assert_eq!(data(&leds), &[0u8][..]);
        let set_leds = keyboard.control(setup(0x21, 9, 0x0200, 0, 1), &[0x1f]);
        assert_eq!(status(&set_leds), 0);
        let leds = keyboard.control(setup(0xa1, 1, 0x0200, 0, 1), &[]);
        assert_eq!(data(&leds), &[0x1fu8][..]);
    }

    #[test]
    fn closed_devices_ignore_input_and_reads() {
        let mut device = Device::new(MOUSE_ID);
        let queued = device.reports.len();
        device.input(Input::Release);
        assert_eq!(device.reports.len(), queued);
        assert!(!device.enqueue_read([0; 33]));
        assert!(device.complete().is_none());
    }

    #[test]
    fn pointer_reports_cover_absolute_relative_scroll_and_release() {
        let mut mouse = Device::new(MOUSE_ID);
        mouse.open();
        let queued = mouse.reports.len();
        mouse.input(Input::Key {
            usage: 3,
            scan: 0,
            modifiers: 0,
            down: true,
        });
        assert_eq!(mouse.reports.len(), queued);

        mouse.input(Input::Motion {
            x: 40000,
            y: -10,
            relative: false,
        });
        assert_eq!(
            mouse.reports.back().map(Report::data),
            Some(&[1u8, 0x20, 0x4e, 0, 0][..])
        );
        mouse.input(Input::Motion {
            x: 200,
            y: 100,
            relative: false,
        });
        assert_eq!(
            mouse.reports.back().map(Report::data),
            Some(&[1u8, 100, 0, 50, 0][..])
        );

        mouse.input(Input::Motion {
            x: -5,
            y: 9,
            relative: true,
        });
        assert_eq!(
            mouse.reports.back().map(Report::data),
            Some(&[3u8, 0, 0xfb, 0xff, 9, 0, 0, 0][..])
        );

        mouse.input(Input::Button {
            button: 6,
            x: 0,
            y: 0,
            down: true,
        });
        assert_eq!(
            mouse.reports.back().map(Report::data),
            Some(&[3u8, 0, 0xfb, 0xff, 9, 0, 0, 0][..])
        );

        mouse.input(Input::Button {
            button: 5,
            x: 0,
            y: 0,
            down: true,
        });
        assert_eq!(
            mouse.reports.back().map(Report::data),
            Some(&[3u8, 16, 0, 0, 0, 0, 0, 0][..])
        );

        let queued = mouse.reports.len();
        mouse.input(Input::Scroll { x: 60, y: 0 });
        assert_eq!(mouse.reports.len(), queued);
        mouse.input(Input::Scroll { x: 60, y: 0 });
        assert_eq!(
            mouse.reports.back().map(Report::data),
            Some(&[3u8, 16, 0, 0, 0, 0, 0, 1][..])
        );

        mouse.input(Input::Release);
        assert_eq!(mouse.reports.len(), 1);
        assert_eq!(
            mouse.reports.back().map(Report::data),
            Some(&[3u8, 0, 0, 0, 0, 0, 0, 0][..])
        );
    }

    #[test]
    fn queued_reports_and_reads_stay_bounded() {
        let mut mouse = Device::new(MOUSE_ID);
        mouse.open();
        for _ in 1..MAX_REPORTS {
            mouse.input(Input::Scroll { x: 0, y: 120 });
        }
        assert_eq!(mouse.reports.len(), MAX_REPORTS);
        mouse.input(Input::Scroll { x: 0, y: 120 });
        assert_eq!(mouse.reports.len(), 1);

        for _ in 0..MAX_READS {
            assert!(mouse.enqueue_read([0; 33]));
        }
        assert!(!mouse.enqueue_read([0; 33]));
        assert!(mouse.complete().is_some());
    }

    #[test]
    fn idle_keyboard_resends_its_state_without_new_input() {
        let mut keyboard = Device::new(KEYBOARD_ID);
        keyboard.open();
        assert!(keyboard.enqueue_read([0; 33]));
        assert!(keyboard.complete().is_some());
        keyboard.reports.clear();
        keyboard.idle = 1;
        std::thread::sleep(Duration::from_millis(10));
        assert!(keyboard.enqueue_read([0; 33]));
        let (_, payload, initial) = keyboard.complete().expect("idle repeat");
        assert!(!initial);
        assert_eq!(payload.len(), 4 + 29);
    }
}
