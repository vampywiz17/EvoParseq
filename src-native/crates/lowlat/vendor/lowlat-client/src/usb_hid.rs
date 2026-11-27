//! Shared optional USB HID device lifecycle; independent of video decode.
#![allow(
    clippy::indexing_slicing,
    reason = "fixed local records; peer records are length checked"
)]
#[path = "hid_mouse_input.rs"]
mod input;
#[path = "hid_keyboard_input.rs"]
mod keyboard;
pub(crate) const FRAME: usize = 1945;
const RECORD: usize = 1570;
const MAKER: &str = "EvoParseq";
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Mouse,
    Keyboard,
    Both,
}
struct Layout {
    id: u32,
    device: &'static [u8],
    config: &'static [u8],
    report: &'static [u8],
    product: &'static str,
    serial: &'static str,
    input_bytes: usize,
}
impl Kind {
    fn layout(self) -> Layout {
        match self {
            Self::Mouse => Layout {
                id: input::ID,
                device: input::DEVICE,
                config: input::CONFIG,
                report: input::REPORT,
                product: "Experimental USB HID mouse",
                serial: "MOUSE-DUAL",
                input_bytes: 8,
            },
            Self::Keyboard => Layout {
                id: keyboard::ID,
                device: keyboard::DEVICE,
                config: keyboard::CONFIG,
                report: keyboard::REPORT,
                product: "Experimental USB HID keyboard",
                serial: "KEYBOARD",
                input_bytes: keyboard::BYTES,
            },
            Self::Both => Layout {
                id: 0xffff_f008,
                device: &BOTH_DEVICE,
                config: &BOTH_CONFIG,
                report: &BOTH_REPORT,
                product: "Experimental USB HID mouse and keyboard",
                serial: "INPUT-DUAL-MOUSE",
                input_bytes: keyboard::BYTES + 1,
            },
        }
    }
}
const fn copy_descriptor<const N: usize>(source: &[u8]) -> [u8; N] {
    let mut r = [0; N];
    let mut i = 0;
    while i < N {
        r[i] = source[i];
        i += 1;
    }
    r
}
const BOTH_DEVICE: [u8; 18] = {
    let mut r = copy_descriptor::<18>(keyboard::DEVICE);
    r[10] = 8;
    r
};
const BOTH_REPORT: [u8; input::REPORT.len() + keyboard::REPORT.len() + 2] = {
    let mut r = [0; input::REPORT.len() + keyboard::REPORT.len() + 2];
    let mut i = 0;
    while i < input::REPORT.len() {
        r[i] = input::REPORT[i];
        i += 1;
    }
    let mut dst = i;
    i = 0;
    while i < keyboard::REPORT.len() {
        if i == 6 {
            r[dst] = 0x85;
            r[dst + 1] = 2;
            dst += 2;
        }
        r[dst] = keyboard::REPORT[i];
        dst += 1;
        i += 1;
    }
    r
};
const BOTH_CONFIG: [u8; 34] = {
    let mut r = copy_descriptor::<34>(keyboard::CONFIG);
    r[25] = BOTH_REPORT.len() as u8;
    r[26] = (BOTH_REPORT.len() >> 8) as u8;
    r
};
#[derive(Debug, Clone, Copy)]
struct Report {
    bytes: [u8; 30],
    len: usize,
}
impl Report {
    fn new(bytes: &[u8]) -> Self {
        let mut r = Self {
            bytes: [0; 30],
            len: bytes.len(),
        };
        r.bytes[..bytes.len()].copy_from_slice(bytes);
        r
    }
    fn numbered(mut self, id: u8) -> Self {
        self.bytes.copy_within(..self.len, 1);
        self.bytes[0] = id;
        self.len += 1;
        self
    }
}
#[derive(Debug, Default)]
struct Pair {
    mouse: input::Input,
    keyboard: keyboard::Input,
    keyboard_next: bool,
}
impl Pair {
    fn next(&mut self, now: u64) -> Option<Report> {
        let report = if self.keyboard_next {
            self.keyboard
                .next(now)
                .map(|r| r.numbered(2))
                .or_else(|| self.mouse.next(now))
        } else {
            self.mouse
                .next(now)
                .or_else(|| self.keyboard.next(now).map(|r| r.numbered(2)))
        };
        if let Some(r) = &report {
            self.keyboard_next = r.bytes[0] != 2;
        }
        report
    }
    fn release(&mut self) -> Option<Report> {
        self.mouse.reset();
        self.keyboard.reset();
        self.mouse
            .release()
            .or_else(|| self.keyboard.release().map(|r| r.numbered(2)))
    }
    fn start(&mut self) {
        self.mouse.start();
        self.keyboard.start();
    }
    fn failed(&self) -> bool {
        self.mouse.failed() || self.keyboard.failed()
    }
    fn command(&mut self, op: u8, a: u32, b: u32, c: u32) {
        if mouse_opcode(op) || op == 24 {
            self.mouse.command(op, a, b, c);
        }
        if op == 0 || op == 24 {
            self.keyboard.command(op, a, b, c);
        }
    }
}
#[derive(Debug)]
enum Input {
    Mouse(input::Input),
    Keyboard(keyboard::Input),
    Both(Box<Pair>),
}
impl Default for Input {
    fn default() -> Self {
        Self::Mouse(input::Input::default())
    }
}
// The lifecycle calls the same small report interface on either device.
macro_rules! forward {
    ($($name:ident($($arg:ident:$ty:ty),*) -> $ret:ty;)*) => {$ (
        fn $name(&mut self, $($arg:$ty),*) -> $ret {
            match self {
                Self::Mouse(v) => v.$name($($arg),*),
                Self::Keyboard(v) => v.$name($($arg),*),
                Self::Both(v) => v.$name($($arg),*),
            }
        }
    )*};
}
impl Input {
    fn kind(&self) -> Kind {
        match self {
            Self::Mouse(_) => Kind::Mouse,
            Self::Keyboard(_) => Kind::Keyboard,
            Self::Both(_) => Kind::Both,
        }
    }
    forward! {
        start() -> ();
        failed() -> bool;
        next(now: u64) -> Option<Report>;
        release() -> Option<Report>;
        command(op: u8, a: u32, b: u32, c: u32) -> ();
    }
}
pub(crate) fn mouse_opcode(opcode: u8) -> bool {
    matches!(opcode, 1 | 2 | 3 | 26)
}
fn put(record: &mut [u8], offset: usize, value: u32) {
    record[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn word(record: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([record[offset], record[offset + 1]])
}
fn dword(record: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(record[offset..offset + 4].try_into().unwrap_or_default())
}
fn frame(record: &[u8; RECORD], id: u32) -> [u8; FRAME] {
    let mut out = [0; FRAME];
    out[..4].copy_from_slice(&1932u32.to_be_bytes());
    out[4..8].copy_from_slice(&1u32.to_be_bytes());
    out[8..12].copy_from_slice(&id.to_be_bytes());
    out[12] = 32;
    put(&mut out, 13, 14);
    put(&mut out, 17, 1570);
    out[21..21 + RECORD].copy_from_slice(record);
    out
}
fn identity(layout: &Layout) -> [u8; FRAME] {
    let mut r = [0; RECORD];
    put(&mut r, 0, 15);
    r[4..22].copy_from_slice(layout.device);
    r[22..22 + MAKER.len()].copy_from_slice(MAKER.as_bytes());
    r[277..277 + layout.product.len()].copy_from_slice(layout.product.as_bytes());
    r[532..536].copy_from_slice(&[4, 3, 9, 4]);
    r[542..542 + layout.config.len()].copy_from_slice(layout.config);
    put(&mut r, 1566, 34);
    frame(&r, layout.id)
}
fn configuration_for(config: &[u8], id: u32) -> [u8; FRAME] {
    let mut r = [0; RECORD];
    put(&mut r, 0, 11);
    r[8..17].copy_from_slice(&config[..9]);
    r[17..26].copy_from_slice(&config[9..18]);
    r[26..33].copy_from_slice(&config[27..34]);
    put(&mut r, 17 + 180, 1); // one alternate, one interface, one endpoint
    frame(&r, id)
}
fn string(value: &str) -> Vec<u8> {
    let mut bytes = vec![0, 3];
    for ch in value.encode_utf16() {
        bytes.extend_from_slice(&ch.to_le_bytes());
    }
    bytes[0] = u8::try_from(bytes.len()).unwrap_or(0);
    bytes
}

fn control(r: &[u8], layout: &Layout, input: &mut Input) -> [u8; FRAME] {
    let (device, config, report) = (layout.device, layout.config, layout.report);
    let mut reply = [0; RECORD];
    put(&mut reply, 0, 10);
    put(&mut reply, 1036, dword(r, 1042));
    let length = usize::from(word(r, 1034));
    let value = word(r, 6);
    let index = word(r, 8);
    let request = (r[4], r[5], value, index);
    let numbered = input.kind() == Kind::Both;
    let report_id: u16 = if numbered { 2 } else { 0 };
    let k = match input {
        Input::Keyboard(k) => Some(k),
        Input::Both(p) => Some(&mut p.keyboard),
        Input::Mouse(_) => None,
    };
    let keyboard_reply = if let Some(k) = k {
        match request {
            (0xa1, 1, v, 0) if v == 0x100 | report_id => {
                let mut bytes = k.report().to_vec();
                if numbered {
                    bytes.insert(0, 2);
                }
                Some(Some(bytes))
            }
            (0xa1, 1, v, 0) if v == 0x200 | report_id => Some(Some(if numbered {
                vec![2, k.leds]
            } else {
                vec![k.leds]
            })),
            (0x21, 9, v, 0) if v == 0x200 | report_id => {
                let prefix = usize::from(numbered);
                if length == 1 + prefix && (!numbered || r[10] == 2) {
                    k.leds = r[10 + prefix] & 0x1f;
                    Some(Some(r[10..10 + length].to_vec()))
                } else {
                    Some(None)
                }
            }
            (0xa1, 2, v, 0) if v == report_id || v == 0 => Some(Some(vec![k.idle])),
            (0x21, 10, _, 0) if (value & 255 == report_id || value & 255 == 0) && length == 0 => {
                k.idle = (value >> 8) as u8;
                Some(Some(vec![]))
            }
            // This descriptor offers report protocol only, never boot protocol.
            (0x21, 11, 0, 0) => Some(None),
            _ => None,
        }
    } else {
        None
    };
    let data = keyboard_reply.unwrap_or_else(|| match request {
        (0x80, 6, 0x100, 0) => Some(device.to_vec()),
        (0x80, 6, 0x200, 0) => Some(config.to_vec()),
        (0x80, 6, 0x300, _) => Some(vec![4, 3, 9, 4]),
        (0x80, 6, 0x301, 0x409) => Some(string(MAKER)),
        (0x80, 6, 0x302, 0x409) => Some(string(layout.product)),
        (0x80, 6, 0x303, 0x409) => Some(string(layout.serial)),
        (0x81, 6, 0x2100, 0) => Some(config[18..27].to_vec()),
        (0x81, 6, 0x2200, 0) => Some(report.to_vec()),
        (0x80, 8, 0, 0) => Some(vec![1]),  // GET_CONFIGURATION
        (0x81, 10, 0, 0) => Some(vec![0]), // GET_INTERFACE
        (0x80..=0x82, 0, 0, _) => Some(vec![0, 0]), // GET_STATUS
        (0x00, 9, 1, 0) | (0x01, 11, 0, 0) => Some(vec![]),
        (0xa1, 2, 0, 0) => Some(vec![0]),
        (0xa1, 3, 0, 0) => Some(vec![1]),
        (0x21, 10, _, 0) | (0x21, 11, 0..=1, 0) => Some(vec![]),
        _ => None,
    });
    let supported = length <= 1024 && data.is_some();
    if let Some(data) = data.filter(|_| supported) {
        let count = data.len().min(length);
        reply[8..8 + count].copy_from_slice(&data[..count]);
        // Native libusb_control_transfer returns the transferred count in BOTH
        // the status and length fields (negative status only denotes failure).
        put(&mut reply, 4, u32::try_from(count).unwrap_or(0));
        put(&mut reply, 1032, u32::try_from(count).unwrap_or(0));
    } else {
        put(&mut reply, 4, (-9i32) as u32);
    }
    frame(&reply, layout.id)
}

#[derive(Debug, Default)]
pub(crate) struct Device {
    start: Option<f64>,
    announced: bool,
    removed: bool,
    error: u32,
    elapsed: u64,
    input: Input,
    pending: std::collections::VecDeque<u32>,
    ready: bool,
    last_report: Option<u64>,
}
impl Device {
    pub(crate) fn new(kind: Kind) -> Self {
        Self {
            input: match kind {
                Kind::Mouse => Input::Mouse(input::Input::default()),
                Kind::Keyboard => Input::Keyboard(keyboard::Input::default()),
                Kind::Both => Input::Both(Box::default()),
            },
            ..Self::default()
        }
    }
    pub(crate) fn tick(&mut self, now: f64) -> Option<[u8; FRAME]> {
        let start = *self.start.get_or_insert(now);
        self.elapsed = (now - start).max(0.0) as u64;
        if self.removed {
            return None;
        }
        if self.input.failed() {
            self.fail(3);
        }
        if self.announced && !self.ready && self.elapsed >= 12000 {
            self.fail(1);
        }
        if self.error != 0 {
            return self.remove();
        }
        if !self.announced && self.elapsed >= 2000 {
            self.announced = true;
            return Some(identity(&self.input.kind().layout()));
        }
        if self.pending.is_empty()
            || self
                .last_report
                .is_some_and(|t| self.elapsed.saturating_sub(t) < 4)
        {
            return None;
        }
        let report = self.input.next(self.elapsed)?;
        self.last_report = Some(self.elapsed);
        self.complete(report)
    }
    pub(crate) fn incoming(&mut self, bytes: &[u8]) -> Option<[u8; FRAME]> {
        if !self.announced
            || self.removed
            || bytes.len() != FRAME
            || bytes[12] != 32
            || bytes[..4] != 1932u32.to_be_bytes()
            || bytes[8..12] != self.input.kind().layout().id.to_be_bytes()
            || dword(bytes, 13) != 14
            || dword(bytes, 17) != 1570
        {
            return None;
        }
        let r = &bytes[21..21 + RECORD];
        match dword(r, 0) {
            2 => {
                let l = self.input.kind().layout();
                Some(configuration_for(l.config, l.id))
            }
            1 => Some(control(r, &self.input.kind().layout(), &mut self.input)),
            7 => {
                if r[4] != 0x81
                    || r[5] != 3
                    || !(self.input.kind().layout().input_bytes as u32..=1550)
                        .contains(&dword(r, 16))
                    || self.pending.len() >= 8
                {
                    self.fail(2);
                    return self.remove();
                }
                self.pending.push_back(dword(r, 8));
                if !self.ready {
                    self.input.start();
                    self.ready = true;
                }
                None
            }
            _ => {
                self.fail(5);
                self.remove()
            }
        }
    }
    fn fail(&mut self, code: u32) {
        if self.error == 0 {
            self.error = code;
        }
    }
    pub(crate) fn error(&self) -> u32 {
        self.error
    }
    pub(crate) fn active(&mut self) -> bool {
        self.announced && !self.removed && self.error == 0 && self.ready
    }
    pub(crate) fn external_input(&mut self, opcode: u8, a: u32, b: u32, c: u32) {
        if self.active() || (!self.removed && self.error == 0 && opcode == 3 && a == 3) {
            self.input.command(opcode, a, b, c);
        }
    }
    pub(crate) fn release_input(&mut self) -> Option<[u8; FRAME]> {
        if self.pending.is_empty() {
            self.input.command(24, 0, 0, 0);
            return None;
        }
        let report = self.input.release()?;
        self.complete(report)
    }
    fn complete(&mut self, report: Report) -> Option<[u8; FRAME]> {
        let id = self.pending.pop_front()?;
        let mut r = [0; RECORD];
        put(&mut r, 0, 8);
        r[4] = 0x81;
        r[5] = 3;
        put(&mut r, 8, id);
        put(&mut r, 16, report.len as u32);
        r[20..20 + report.len].copy_from_slice(&report.bytes[..report.len]);
        Some(frame(&r, self.input.kind().layout().id))
    }
    pub(crate) fn remove(&mut self) -> Option<[u8; FRAME]> {
        if !self.announced || self.removed {
            return None;
        }
        if let Some(release) = self.release_input() {
            return Some(release);
        }
        self.removed = true;
        let mut r = [0; RECORD];
        put(&mut r, 0, 16);
        Some(frame(&r, self.input.kind().layout().id))
    }
    pub(crate) fn sent(&mut self, success: bool) {
        if !success {
            self.fail(4);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hid_pair_uses_one_device_and_correlates_both_report_types_and_release() {
        let mut d = Device::new(Kind::Both);
        assert!(d.tick(0.0).is_none());
        let identity = d.tick(2000.0).unwrap();
        let layout = Kind::Both.layout();
        assert_eq!(&identity[8..12], &layout.id.to_be_bytes());
        assert_eq!(layout.config[25] as usize, layout.report.len());
        assert_eq!(
            layout.report.windows(2).filter(|w| *w == [0x85, 1]).count(),
            1
        );
        assert_eq!(
            layout.report.windows(2).filter(|w| *w == [0x85, 2]).count(),
            1
        );
        assert!(
            d.tick(2000.0).is_none(),
            "must not announce a second device"
        );
        for id in 101..=108 {
            let mut r = [0; RECORD];
            put(&mut r, 0, 7);
            r[4] = 0x81;
            r[5] = 3;
            put(&mut r, 8, id);
            put(&mut r, 16, 32);
            assert!(d.incoming(&frame(&r, layout.id)).is_none());
        }
        let initial = d.tick(2000.0).unwrap();
        assert_eq!(
            (dword(&initial, 29), dword(&initial, 37), initial[41]),
            (101, 30, 2)
        );
        d.external_input(3, 2, 100, 200);
        d.external_input(1, 1, 1, 0);
        d.external_input(0, 225, 0, 1);
        d.external_input(0, 4, 0, 1);
        for (time, id, report_id, len) in [
            (2004.0, 102, 1, 5),
            (2008.0, 103, 2, 30),
            (2012.0, 104, 3, 8),
            (2016.0, 105, 2, 30),
        ] {
            let f = d.tick(time).unwrap();
            assert_eq!(&f[8..12], &layout.id.to_be_bytes());
            assert_eq!((dword(&f, 29), f[41], dword(&f, 37)), (id, report_id, len));
            if id == 105 {
                assert_eq!(&f[42..44], &[2, 16]);
            }
        }
        // Standard output report includes the keyboard's report ID.
        let mut r = [0; RECORD];
        put(&mut r, 0, 1);
        r[4] = 0x21;
        r[5] = 9;
        r[6..8].copy_from_slice(&0x202u16.to_le_bytes());
        r[10] = 2;
        r[11] = 3;
        r[1034..1036].copy_from_slice(&2u16.to_le_bytes());
        put(&mut r, 1042, 77);
        let ack = d.incoming(&frame(&r, layout.id)).unwrap();
        assert_eq!((dword(&ack, 25), dword(&ack, 21 + 1036)), (2, 77));
        for (id, report_id) in [(106, 3), (107, 2)] {
            let f = d.release_input().unwrap();
            assert_eq!((dword(&f, 29), f[41]), (id, report_id));
            if report_id == 3 {
                assert_eq!(f[42], 0);
            } else {
                assert!(f[42..71].iter().all(|b| *b == 0));
            }
        }
        let removal = d.remove().unwrap();
        assert_eq!(dword(&removal, 21), 16);
        assert!(!d.active());
    }
    #[test]
    fn hid_mouse_enumeration_preserves_zero_ids_and_stalls_unknown_requests() {
        let mut m = Device::default();
        assert!(m.tick(0.0).is_none());
        let identity = m.tick(2000.0).unwrap();
        assert_eq!(&identity[8..12], &input::ID.to_be_bytes());
        assert_eq!(input::CONFIG[25], input::REPORT.len() as u8);
        let mut request = [0; RECORD];
        put(&mut request, 0, 1);
        request[4] = 0x81;
        request[5] = 6;
        request[6..8].copy_from_slice(&0x2200u16.to_le_bytes());
        request[1034..1036].copy_from_slice(&1024u16.to_le_bytes());
        let reply = m.incoming(&frame(&request, input::ID)).unwrap();
        assert_eq!(dword(&reply, 21 + 1036), 0);
        assert_eq!(dword(&reply, 21 + 1032), input::REPORT.len() as u32);
        assert_eq!(&reply[29..29 + input::REPORT.len()], input::REPORT);
        request[4] = 0x80;
        request[6..8].copy_from_slice(&0x03eeu16.to_le_bytes());
        assert_eq!(
            dword(&m.incoming(&frame(&request, input::ID)).unwrap(), 25) as i32,
            -9
        );
        assert_eq!(m.error(), 0);
        assert!(m.tick(12000.0).is_some());
        assert_eq!(m.error(), 1);
        assert!(!m.active());
    }
    #[test]
    fn hid_mouse_no_diagnostic_lifetime_limit_and_rejects_bad_reads() {
        let mut m = Device::default();
        m.tick(0.0);
        m.tick(2000.0);
        let mut r = [0; RECORD];
        put(&mut r, 0, 7);
        r[4] = 0x81;
        r[5] = 3;
        put(&mut r, 16, 8);
        assert!(m.incoming(&frame(&r, input::ID)).is_none());
        assert!(m.active());
        assert!(m.tick(100000000.0).is_none());
        assert!(m.active());
        r[4] = 1;
        assert!(m.incoming(&frame(&r, input::ID)).is_some());
        assert_eq!(m.error(), 2);
        assert!(!m.active());
    }
}
