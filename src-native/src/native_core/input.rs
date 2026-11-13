//! Physical USB usages, as in lowlat examples/client/keys.h. Reuse our wire
//! framing, viewport and HID adapter; characters remain the host layout's.
use super::transport::Attempt;
use crate::window::{Event, Window};
use anyhow::Result;

use crate::desktop_input::usage;

fn primary_allows(event: &Event, ready: bool) -> bool {
    ready
        || !matches!(
            event,
            Event::Motion(..) | Event::RelativeMotion(..) | Event::Button(..) | Event::Scroll(..)
        )
}

// Side-specific modifier bits from lowlat.h, not the browser's compact mask.
#[derive(Default)]
pub struct Input {
    keys: crate::desktop_input::Keyboard,
}
impl Input {
    fn frame(
        &mut self,
        event: &Event,
        viewport: Option<crate::viewport::Viewport>,
    ) -> Option<bytes::Bytes> {
        use crate::control::header;
        Some(match event {
            Event::Key(down, code, compact) => {
                let key = usage(code)?;
                self.keys.frame(key, *compact, *down)?
            }
            Event::Button(down, button, ..) if (0..=4).contains(button) => {
                header(1, *button + 1, i32::from(*down), 0)
            }
            // Window's vertical sign matches its browser callback; the native
            // lowlat/stock-host contract is Win32's wheel sign instead.
            Event::Scroll(x, y) => header(2, *x, y.saturating_neg(), 0),
            Event::Motion(x, y) => {
                let (x, y) = viewport?.map(*x, *y)?;
                header(3, 0, x, y)
            }
            Event::RelativeMotion(x, y) => header(3, 1, *x, *y),
            Event::RelativeMode(active) => header(3, 3, i32::from(*active), 0),
            Event::Focus(false) | Event::HidInputRelease => {
                self.keys.reset();
                header(24, 0, 0, 0)
            }
            _ => return None,
        })
    }
    pub fn send(&mut self, attempt: &Attempt, window: &Window, event: &Event) -> Result<()> {
        if matches!(event, Event::RelativeMode(_)) && !attempt.hid_mouse_enabled() {
            return Ok(());
        }
        // No mouse packet may target an unverified captured output. Device
        // enumeration, key forwarding and release commands remain independent.
        if attempt.hid_mouse_enabled()
            && !primary_allows(
                event,
                window
                    .session
                    .hid_primary_ready
                    .load(std::sync::atomic::Ordering::Acquire),
            )
        {
            return Ok(());
        }
        let viewport = window.presented_viewport();
        let Some(mut packet) = self.frame(event, viewport) else {
            return Ok(());
        };
        let stream = window
            .display_stream
            .load(std::sync::atomic::Ordering::Acquire);
        if stream != 0 && packet[12] == 3 {
            let Ok(motion) = crate::display_stream::motion(stream, &packet) else {
                return Ok(());
            };
            packet = motion;
        }
        if window.native_input_allowed()
            || matches!(event, Event::Focus(false) | Event::HidInputRelease)
        {
            attempt.native_input(&packet, viewport)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unverified_primary_blocks_mouse_but_keeps_releases_and_keyboard() {
        for event in [
            Event::Motion(1, 2),
            Event::RelativeMotion(3, 4),
            Event::Button(true, 0, 0, 0),
            Event::Scroll(0, 120),
        ] {
            assert!(!primary_allows(&event, false));
            assert!(primary_allows(&event, true));
        }
        for event in [
            Event::Key(true, "KeyA", 0),
            Event::Focus(false),
            Event::HidInputRelease,
            Event::RelativeMode(false),
        ] {
            assert!(primary_allows(&event, false));
        }
    }
    #[test]
    fn physical_usages_preserve_hungarian_iso_key_and_modifiers() {
        for (code, expected) in [
            ("KeyY", 28),
            ("KeyZ", 29),
            ("IntlBackslash", 100),
            ("AltRight", 230),
            ("ControlRight", 228),
            ("Digit0", 39),
            ("F24", 115),
        ] {
            assert_eq!(usage(code), Some(expected));
        }
        assert_eq!(usage("unrecognized"), None);
    }
    #[test]
    fn native_buttons_wheel_and_modifiers_match_lowlat_wire_values() {
        let mut input = Input::default();
        assert_eq!(
            input.frame(&Event::Button(true, 0, 0, 0), None).unwrap(),
            crate::control::header(1, 1, 1, 0)
        );
        assert_eq!(
            input.frame(&Event::Button(false, 2, 0, 0), None).unwrap(),
            crate::control::header(1, 3, 0, 0)
        );
        assert_eq!(
            input.frame(&Event::Scroll(120, -120), None).unwrap(),
            crate::control::header(2, 120, 120, 0)
        );
        assert_eq!(
            input
                .frame(&Event::Key(true, "ControlRight", 2), None)
                .unwrap(),
            crate::control::header(0, 228, 128, 1)
        );
        assert_eq!(
            input.frame(&Event::Key(true, "KeyA", 18), None).unwrap(),
            crate::control::header(0, 4, 128 | 8192, 1)
        );
        assert_eq!(
            input.frame(&Event::Focus(false), None).unwrap(),
            crate::control::header(24, 0, 0, 0)
        );
        assert_eq!(input.keys.modifiers, 0);
    }
}
