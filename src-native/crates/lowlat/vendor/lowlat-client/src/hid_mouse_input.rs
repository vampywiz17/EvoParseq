//! Absolute desktop positions and raw relative mouse reports; bounded queues.
use super::Report;
use std::collections::VecDeque;
pub(super) const ID: u32 = 0xffff_f007;
pub(super) const DEVICE: &[u8] = &[
    18, 1, 0, 2, 0, 0, 0, 64, 0xff, 0xff, 7, 0xf0, 0, 1, 1, 2, 3, 1,
];
pub(super) use lowlat_core::hid::MOUSE_REPORT as REPORT;
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
    8,
    0,
    4,
];
#[derive(Debug, Default, Clone, Copy)]
struct Sample {
    buttons: u8,
    edge: bool,
    point: Option<(u16, u16)>,
    delta: (i64, i64),
    vertical: i64,
    horizontal: i64,
}
#[derive(Debug, Default)]
pub(super) struct Input {
    samples: VecDeque<Sample>,
    position: Option<(u16, u16)>,
    buttons: u8,
    sent_buttons: u8,
    wheel_remainder: (i64, i64),
    relative: bool,
    ready: bool,
    failed: bool,
    last_sent: Option<u64>,
}
impl Input {
    pub(super) fn start(&mut self) {
        self.ready = true;
    }
    pub(super) fn failed(&self) -> bool {
        self.failed
    }
    fn enqueue(&mut self, sample: Sample) {
        if let Some(last) = self.samples.back_mut().filter(|last| {
            last.buttons == sample.buttons
                && !last.edge
                && !sample.edge
                && last.point.is_some() == sample.point.is_some()
        }) {
            if sample.point.is_some() {
                last.point = sample.point;
            }
            last.delta.0 += sample.delta.0;
            last.delta.1 += sample.delta.1;
            last.vertical += sample.vertical;
            last.horizontal += sample.horizontal;
            if last.delta.0.abs() > 1_048_576
                || last.delta.1.abs() > 1_048_576
                || last.vertical.abs() > 8192
                || last.horizontal.abs() > 8192
            {
                self.failed = true;
            }
        } else if self.samples.len() < 64 {
            self.samples.push_back(sample);
        } else {
            self.failed = true;
        }
    }
    pub(super) fn command(&mut self, opcode: u8, a: u32, b: u32, c: u32) {
        if opcode == 24 {
            self.reset();
            return;
        }
        // Private adapter mode command, never forwarded as Parsec input. Keep
        // host intent even if enumeration has not supplied its first IN read.
        if opcode == 3 && a == 3 && b <= 1 && c == 0 {
            let relative = b != 0;
            if relative != self.relative {
                self.relative = relative;
                self.position = None;
                // Preserve every button edge and wheel, discard stale motion.
                self.samples
                    .retain(|s| s.edge || s.vertical != 0 || s.horizontal != 0);
                for s in &mut self.samples {
                    s.delta = (0, 0);
                }
            }
            return;
        }
        if !self.ready || self.failed {
            return;
        }
        let mut s = Sample {
            buttons: self.buttons,
            ..Default::default()
        };
        match opcode {
            1 => {
                let bit = match a {
                    1 => 1,
                    2 => 4,
                    3 => 2,
                    4 => 8,
                    5 => 16,
                    _ => return,
                };
                if b > 1 {
                    return;
                }
                if b != 0 {
                    self.buttons |= bit;
                } else {
                    self.buttons &= !bit;
                }
                s.buttons = self.buttons;
                s.edge = true;
            }
            2 => {
                self.wheel_remainder.0 += i64::from(a as i32);
                self.wheel_remainder.1 += i64::from(b as i32);
                s.horizontal = self.wheel_remainder.0 / 120;
                s.vertical = self.wheel_remainder.1 / 120;
                self.wheel_remainder.0 %= 120;
                self.wheel_remainder.1 %= 120;
                if s.horizontal == 0 && s.vertical == 0 {
                    return;
                }
            }
            3 if a == 2 && !self.relative && b <= 32767 && c <= 32767 => {
                let point = (b as u16, c as u16);
                if self.position == Some(point) {
                    return;
                }
                self.position = Some(point);
                s.point = Some(point);
            }
            3 if a == 1 && self.relative => {
                s.delta = (i64::from(b as i32), i64::from(c as i32));
                if s.delta == (0, 0) {
                    return;
                }
            }
            _ => return,
        }
        if s.delta.0.abs() > 1_048_576
            || s.delta.1.abs() > 1_048_576
            || s.horizontal.abs() > 8192
            || s.vertical.abs() > 8192
        {
            self.failed = true;
            return;
        }
        self.enqueue(s);
    }
    fn relative_packet(buttons: u8, x: i16, y: i16, vertical: u8, horizontal: u8) -> Report {
        let (x, y) = (x.to_le_bytes(), y.to_le_bytes());
        Report::new(&[3, buttons, x[0], x[1], y[0], y[1], vertical, horizontal])
    }
    pub(super) fn next(&mut self, now: u64) -> Option<Report> {
        if self.failed || self.last_sent.is_some_and(|t| now.saturating_sub(t) < 4) {
            return None;
        }
        let s = self.samples.front_mut()?;
        let report = if let Some((x, y)) = s.point.take() {
            let (x, y) = (x.to_le_bytes(), y.to_le_bytes());
            Report::new(&[1, x[0], x[1], y[0], y[1]])
        } else {
            let take = |v: &mut i64, limit: i64| {
                let n = (*v).clamp(-limit, limit);
                *v -= n;
                n
            };
            let r = Self::relative_packet(
                s.buttons,
                take(&mut s.delta.0, 32767) as i16,
                take(&mut s.delta.1, 32767) as i16,
                take(&mut s.vertical, 127) as i8 as u8,
                take(&mut s.horizontal, 127) as i8 as u8,
            );
            self.sent_buttons = s.buttons;
            r
        };
        self.last_sent = Some(now);
        if s.delta == (0, 0) && s.vertical == 0 && s.horizontal == 0 {
            self.samples.pop_front();
        }
        Some(report)
    }
    pub(super) fn reset(&mut self) {
        self.buttons = 0;
        self.position = None;
        self.wheel_remainder = (0, 0);
        self.samples.clear();
        if self.ready && self.sent_buttons != 0 {
            self.samples.push_back(Sample::default());
        }
    }
    pub(super) fn release(&mut self) -> Option<Report> {
        self.reset();
        if self.sent_buttons == 0 {
            return None;
        }
        self.sent_buttons = 0;
        Some(Self::relative_packet(0, 0, 0, 0, 0))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn ready() -> Input {
        let mut i = Input::default();
        i.start();
        assert!(i.next(0).is_none());
        i
    }
    fn bytes(i: &mut Input, t: u64) -> Vec<u8> {
        let r = i.next(t).unwrap();
        r.bytes[..r.len].to_vec()
    }
    #[test]
    fn hid_mouse_click_drag_release_keep_edge_positions() {
        let mut i = ready();
        i.command(3, 2, 100, 200);
        i.command(1, 1, 1, 0);
        i.command(3, 2, 30000, 32767);
        i.command(1, 1, 0, 0);
        assert_eq!(bytes(&mut i, 4), [1, 100, 0, 200, 0]);
        assert_eq!(bytes(&mut i, 8), [3, 1, 0, 0, 0, 0, 0, 0]);
        assert_eq!(bytes(&mut i, 12), [1, 48, 117, 255, 127]);
        assert_eq!(bytes(&mut i, 16), [3, 0, 0, 0, 0, 0, 0, 0]);
    }
    #[test]
    fn hid_mouse_two_button_hold_survives_camera_changes_and_independent_releases() {
        for (first, second, first_mask, remaining) in [(1, 3, 1, 2), (3, 1, 2, 1)] {
            let mut i = ready();
            i.command(1, first, 1, 0);
            assert_eq!(bytes(&mut i, 4), [3, first_mask, 0, 0, 0, 0, 0, 0]);
            i.command(1, second, 1, 0);
            assert_eq!(bytes(&mut i, 8), [3, 3, 0, 0, 0, 0, 0, 0]);
            i.command(3, 3, 1, 0);
            i.command(3, 1, 20, (-10i32) as u32);
            assert_eq!(bytes(&mut i, 12), [3, 3, 20, 0, 246, 255, 0, 0]);
            // No repeated reports or synthetic releases just because a hold is long.
            assert!(i.next(10_000).is_none());
            i.command(3, 3, 0, 0);
            i.command(3, 2, 100, 200);
            assert_eq!(bytes(&mut i, 10_004), [1, 100, 0, 200, 0]);
            i.command(1, first, 0, 0);
            assert_eq!(bytes(&mut i, 10_008), [3, remaining, 0, 0, 0, 0, 0, 0]);
            i.command(3, 3, 1, 0);
            i.command(3, 1, 1, 0);
            assert_eq!(bytes(&mut i, 10_012), [3, remaining, 1, 0, 0, 0, 0, 0]);
            i.command(1, second, 0, 0);
            assert_eq!(bytes(&mut i, 10_016), [3, 0, 0, 0, 0, 0, 0, 0]);
        }
    }
    #[test]
    fn hid_mouse_wheel_target_and_focus_release_discard_stale_motion() {
        let mut i = ready();
        i.command(3, 2, 500, 600);
        let _ = bytes(&mut i, 4);
        i.command(2, 60, (-60i32) as u32, 0);
        assert!(i.samples.is_empty());
        i.command(2, 60, (-60i32) as u32, 0);
        i.command(3, 2, 700, 800);
        assert_eq!(bytes(&mut i, 8), [3, 0, 0, 0, 0, 0, 255, 1]);
        let _ = bytes(&mut i, 12);
        i.command(1, 3, 1, 0);
        assert_eq!(bytes(&mut i, 16)[1], 2);
        i.command(3, 2, 900, 1000);
        let release = i.release().unwrap();
        assert_eq!(&release.bytes[..release.len], [3, 0, 0, 0, 0, 0, 0, 0]);
        assert!(
            i.samples
                .iter()
                .all(|s| s.point.is_none() && s.delta == (0, 0))
        );
    }
    #[test]
    fn hid_mouse_bounds_and_long_lived_pacing() {
        let mut i = ready();
        i.command(3, 2, 1, 1);
        let now = u64::from(u32::MAX) + 100;
        assert!(i.next(now).is_some());
        i.command(3, 2, 2, 2);
        assert!(i.next(now + 1).is_none());
        assert!(i.next(now + 4).is_some());
        for n in 0..65 {
            i.command(1, 1, n % 2, 0);
        }
        assert!(i.failed());
        let mut i = ready();
        i.command(3, 1, 1, 1);
        assert!(i.samples.is_empty());
    }
    #[test]
    fn hid_mouse_relative_switch_preserves_buttons_signed_deltas_and_focus_release() {
        let mut i = Input::default();
        i.command(3, 3, 1, 0);
        i.start();
        i.command(1, 3, 1, 0);
        assert_eq!(bytes(&mut i, 4), [3, 2, 0, 0, 0, 0, 0, 0]);
        i.command(3, 1, (-40000i32) as u32, 40000);
        i.command(3, 1, (-5i32) as u32, 7);
        assert_eq!(bytes(&mut i, 8), [3, 2, 1, 128, 255, 127, 0, 0]);
        assert_eq!(bytes(&mut i, 12), [3, 2, 186, 227, 72, 28, 0, 0]);
        // Never emit absolute moves in camera mode; never repeat an old delta.
        i.command(3, 2, 32000, 100);
        assert!(i.next(16).is_none());
        i.command(2, 0, 120, 0);
        assert_eq!(bytes(&mut i, 16), [3, 2, 0, 0, 0, 0, 1, 0]);
        i.command(3, 1, 100, 200);
        i.command(3, 3, 0, 0);
        assert!(i.next(20).is_none());
        i.command(3, 2, 100, 200);
        assert_eq!(bytes(&mut i, 20), [1, 100, 0, 200, 0]);
        // Button owner/report never changes when switching axes or returning to UI.
        i.command(1, 3, 0, 0);
        assert_eq!(bytes(&mut i, 24), [3, 0, 0, 0, 0, 0, 0, 0]);
        i.command(3, 3, 1, 0);
        i.command(1, 1, 1, 0);
        let _ = bytes(&mut i, 28);
        i.command(3, 1, 123, 456);
        i.command(24, 0, 0, 0);
        assert_eq!(bytes(&mut i, 32), [3, 0, 0, 0, 0, 0, 0, 0]);
        assert!(i.next(36).is_none());
    }
}
