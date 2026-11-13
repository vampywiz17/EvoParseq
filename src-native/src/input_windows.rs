//! Win32 messages become the existing common events, synchronously and without
//! another queue. Capture/focus ownership remains in the native Window backend.
use crate::native_core::session_model::Event;
use std::{collections::BTreeSet, sync::Mutex};
use windows_sys::Win32::{
    Foundation::{LPARAM, WPARAM},
    UI::{
        Input::{KeyboardAndMouse::*, *},
        WindowsAndMessaging::*,
    },
};

pub const CAPTURED_KEY: u32 = WM_APP + 13;

pub struct KeyMessage {
    pub code: &'static str,
    pub down: bool,
    pub repeated: bool,
    pub modifiers: i32,
}
pub unsafe fn key_message(message: u32, wp: WPARAM, lp: LPARAM) -> Option<KeyMessage> {
    let captured = message == CAPTURED_KEY;
    if !captured && !matches!(message, WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP) {
        return None;
    }
    let code = code(
        if captured {
            (wp & 255) as u32
        } else {
            wp as u32
        },
        ((lp >> 16) & 255) as u8,
        lp & (1 << 24) != 0,
    )?;
    let down = if captured {
        lp & (1 << 31) == 0
    } else {
        matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN)
    };
    let repeated = lp & (1 << 30) != 0;
    // This metadata is for local shortcut routing, not remote key delivery.
    // Repeats, releases and non-command keys need no additional modifier query.
    Some(KeyMessage {
        code,
        down,
        repeated,
        modifiers: if captured {
            (lp & 63) as i32
        } else if down && !repeated && crate::input::shortcut_binding(code).is_some() {
            modifiers()
        } else {
            0
        },
    })
}

pub(crate) fn code(vk: u32, scan: u8, extended: bool) -> Option<&'static str> {
    let special = match vk {
        0x08 => "Backspace",
        0x09 => "Tab",
        0x0D if extended => "NumpadEnter",
        0x0D => "Enter",
        0x10 if scan == 0x36 => "ShiftRight",
        0xA1 => "ShiftRight",
        0x10 | 0xA0 => "ShiftLeft",
        0x11 if extended => "ControlRight",
        0xA3 => "ControlRight",
        0x11 | 0xA2 => "ControlLeft",
        0x12 if extended => "AltRight",
        0xA5 => "AltRight",
        0x12 | 0xA4 => "AltLeft",
        0x14 => "CapsLock",
        0x1B => "Escape",
        0x20 => "Space",
        0x21 if extended => "PageUp",
        0x22 if extended => "PageDown",
        0x23 if extended => "End",
        0x24 if extended => "Home",
        0x25 if extended => "ArrowLeft",
        0x26 if extended => "ArrowUp",
        0x27 if extended => "ArrowRight",
        0x28 if extended => "ArrowDown",
        0x2D if extended => "Insert",
        0x2E if extended => "Delete",
        0x5B => "MetaLeft",
        0x5C => "MetaRight",
        0x5D => "ContextMenu",
        0x90 => "NumLock",
        0x91 => "ScrollLock",
        0x13 => "Pause",
        0x2C => "PrintScreen",
        0x6F => "NumpadDivide",
        _ => "",
    };
    if !special.is_empty() {
        return Some(special);
    }
    if (0x70..=0x87).contains(&vk) {
        return Some(
            [
                "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "F13",
                "F14", "F15", "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24",
            ][(vk - 0x70) as usize],
        );
    }
    if extended {
        return None;
    }
    Some(match scan {
        0x02 => "Digit1",
        0x03 => "Digit2",
        0x04 => "Digit3",
        0x05 => "Digit4",
        0x06 => "Digit5",
        0x07 => "Digit6",
        0x08 => "Digit7",
        0x09 => "Digit8",
        0x0A => "Digit9",
        0x0B => "Digit0",
        0x0C => "Minus",
        0x0D => "Equal",
        0x10 => "KeyQ",
        0x11 => "KeyW",
        0x12 => "KeyE",
        0x13 => "KeyR",
        0x14 => "KeyT",
        0x15 => "KeyY",
        0x16 => "KeyU",
        0x17 => "KeyI",
        0x18 => "KeyO",
        0x19 => "KeyP",
        0x1A => "BracketLeft",
        0x1B => "BracketRight",
        0x1E => "KeyA",
        0x1F => "KeyS",
        0x20 => "KeyD",
        0x21 => "KeyF",
        0x22 => "KeyG",
        0x23 => "KeyH",
        0x24 => "KeyJ",
        0x25 => "KeyK",
        0x26 => "KeyL",
        0x27 => "Semicolon",
        0x28 => "Quote",
        0x29 => "Backquote",
        0x2B => "Backslash",
        0x2C => "KeyZ",
        0x2D => "KeyX",
        0x2E => "KeyC",
        0x2F => "KeyV",
        0x30 => "KeyB",
        0x31 => "KeyN",
        0x32 => "KeyM",
        0x33 => "Comma",
        0x34 => "Period",
        0x35 => "Slash",
        0x37 => "NumpadMultiply",
        0x47 => "Numpad7",
        0x48 => "Numpad8",
        0x49 => "Numpad9",
        0x4A => "NumpadSubtract",
        0x4B => "Numpad4",
        0x4C => "Numpad5",
        0x4D => "Numpad6",
        0x4E => "NumpadAdd",
        0x4F => "Numpad1",
        0x50 => "Numpad2",
        0x51 => "Numpad3",
        0x52 => "Numpad0",
        0x53 => "NumpadDecimal",
        0x56 => "IntlBackslash",
        _ => return None,
    })
}

fn emit_key(
    wp: WPARAM,
    lp: LPARAM,
    down: bool,
    keys: &Mutex<BTreeSet<&'static str>>,
    modifiers: impl FnOnce(&BTreeSet<&'static str>) -> i32,
    emit: impl FnOnce(Event),
) {
    let Some(code) = code(wp as u32, ((lp >> 16) & 255) as u8, lp & (1 << 24) != 0) else {
        return;
    };
    let mut pressed = keys.lock().unwrap_or_else(|e| e.into_inner());
    if down {
        pressed.insert(code);
    } else {
        pressed.remove(code);
    }
    // Keep the existing lock and event order: update pressed keys, read this
    // event's modifier source, then publish to the existing FIFO.
    emit(Event::Key(down, code, modifiers(&pressed)));
}
pub unsafe fn key(
    message: u32,
    wp: WPARAM,
    lp: LPARAM,
    keys: &Mutex<BTreeSet<&'static str>>,
    emit: impl FnOnce(Event),
) {
    emit_key(
        wp,
        lp,
        matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN),
        keys,
        |pressed| {
            modifiers()
                | if pressed.contains("MetaLeft") || pressed.contains("MetaRight") {
                    8
                } else {
                    0
                }
        },
        emit,
    );
}
pub fn captured_key(
    wp: WPARAM,
    lp: LPARAM,
    keys: &Mutex<BTreeSet<&'static str>>,
    emit: impl FnOnce(Event),
) {
    // Hook modifiers describe the original event; do not replace them with a
    // later UI-thread GetKeyState result (Alt may already have been released).
    emit_key(
        wp & 255,
        lp,
        lp & (1 << 31) == 0,
        keys,
        |_| (lp & 63) as i32,
        emit,
    );
}
pub fn captured_key_data(key: &KBDLLHOOKSTRUCT, down: bool, mods: usize) -> LPARAM {
    (((key.scanCode & 255) << 16)
        | mods as u32
        | if key.flags & LLKHF_EXTENDED != 0 {
            1 << 24
        } else {
            0
        }
        | if down { 0 } else { 1 << 31 }) as isize
}
pub fn position(lp: LPARAM) -> (i32, i32) {
    (lp as i16 as i32, (lp >> 16) as i16 as i32)
}
/// Encode the signed 16-bit coordinates carried by WM_MOUSEMOVE; zero is
/// reserved for no pending warp. Negative peer-window coordinates stay valid.
pub fn motion_echo(x: i32, y: i32) -> u64 {
    (u64::from(x as u16) << 16 | u64::from(y as u16)) + 1
}
pub fn motion(lp: LPARAM, warp_echo: u64) -> Option<Event> {
    let (x, y) = position(lp);
    (warp_echo == 0 || warp_echo != motion_echo(x, y)).then_some(Event::Motion(x, y))
}
pub fn button(message: u32, wp: WPARAM, lp: LPARAM) -> Option<Event> {
    let (down, button) = match message {
        WM_LBUTTONDOWN | WM_LBUTTONUP => (message == WM_LBUTTONDOWN, 0),
        WM_MBUTTONDOWN | WM_MBUTTONUP => (message == WM_MBUTTONDOWN, 1),
        WM_RBUTTONDOWN | WM_RBUTTONUP => (message == WM_RBUTTONDOWN, 2),
        WM_XBUTTONDOWN | WM_XBUTTONUP => (
            message == WM_XBUTTONDOWN,
            if (wp >> 16) & 0xffff == 1 { 3 } else { 4 },
        ),
        _ => return None,
    };
    let (x, y) = position(lp);
    Some(Event::Button(down, button, x, y))
}
pub fn scroll(message: u32, wp: WPARAM) -> Option<Event> {
    let delta = (wp >> 16) as i16 as i32;
    match message {
        WM_MOUSEWHEEL => Some(Event::Scroll(0, -delta)),
        WM_MOUSEHWHEEL => Some(Event::Scroll(delta, 0)),
        _ => None,
    }
}
pub fn text(wp: WPARAM, decoder: &mut crate::input::TextDecoder) -> Option<Event> {
    decoder.push(wp as u16).map(|ch| Event::Text(ch as u32))
}
pub fn raw_motion(mouse: &RAWMOUSE, relative: bool) -> Option<Event> {
    (relative && mouse.usFlags & MOUSE_MOVE_ABSOLUTE == 0)
        .then_some(Event::RelativeMotion(mouse.lLastX, mouse.lLastY))
}
pub unsafe fn raw_input(lp: LPARAM, deliver: impl FnOnce(&RAWMOUSE)) {
    let mut raw: RAWINPUT = std::mem::zeroed();
    let mut size = std::mem::size_of::<RAWINPUT>() as u32;
    let read = GetRawInputData(
        lp as HRAWINPUT,
        RID_INPUT,
        (&mut raw as *mut RAWINPUT).cast(),
        &mut size,
        std::mem::size_of::<RAWINPUTHEADER>() as u32,
    );
    if read != u32::MAX
        && raw.header.dwType == RIM_TYPEMOUSE
        && read
            >= std::mem::size_of::<RAWINPUTHEADER>() as u32 + std::mem::size_of::<RAWMOUSE>() as u32
    {
        deliver(&raw.data.mouse);
    }
}

pub unsafe fn modifiers() -> i32 {
    let mut mods = 0;
    for (vk, bit) in [(VK_SHIFT, 1), (VK_CONTROL, 2), (VK_MENU, 4)] {
        if GetKeyState(vk as i32) < 0 {
            mods |= bit;
        }
    }
    if GetKeyState(VK_LWIN as i32) < 0 || GetKeyState(VK_RWIN as i32) < 0 {
        mods |= 8;
    }
    if GetKeyState(VK_CAPITAL as i32) & 1 != 0 {
        mods |= 16;
    }
    if GetKeyState(VK_NUMLOCK as i32) & 1 != 0 {
        mods |= 32;
    }
    mods
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hook_keys_preserve_sided_modifiers_repeat_flags_and_capture_metadata() {
        for (vk, scan, extended, expected) in [
            (VK_LSHIFT, 0x2a, false, "ShiftLeft"),
            (VK_RSHIFT, 0x36, false, "ShiftRight"),
            (VK_LCONTROL, 0x1d, false, "ControlLeft"),
            (VK_RCONTROL, 0x1d, true, "ControlRight"),
            (VK_LMENU, 0x38, false, "AltLeft"),
            (VK_RMENU, 0x38, true, "AltRight"),
        ] {
            let key = KBDLLHOOKSTRUCT {
                vkCode: u32::from(vk),
                scanCode: scan,
                flags: if extended { LLKHF_EXTENDED } else { 0 },
                ..Default::default()
            };
            let lp = captured_key_data(&key, true, 63) | (1 << 30);
            let wp = (17 << 8) | usize::from(vk);
            let message = unsafe { key_message(CAPTURED_KEY, wp, lp) }.unwrap();
            assert_eq!(
                (
                    message.code,
                    message.down,
                    message.repeated,
                    message.modifiers
                ),
                (expected, true, true, 63)
            );
            captured_key(wp, lp, &Default::default(), |event| {
                assert!(matches!(event, Event::Key(true, code, 63) if code == expected));
            });
        }
        assert!(unsafe { key_message(WM_MOUSEMOVE, 0, 0) }.is_none());
    }
    fn point(x: i16, y: i16) -> LPARAM {
        (u32::from(x as u16) | (u32::from(y as u16) << 16)) as isize
    }
    #[test]
    fn signed_motion_and_wheel_axes_keep_the_existing_common_event_contract() {
        for (x, y) in [(-32768i16, 32767i16), (-100, -200), (0, 0), (320, 200)] {
            assert!(matches!(motion(point(x, y), 0), Some(Event::Motion(a, b))
                if a == i32::from(x) && b == i32::from(y)));
        }
        for x in [i16::MIN, -1, 0, i16::MAX] {
            for y in [i16::MIN, -1, 0, i16::MAX] {
                let echo = motion_echo(i32::from(x), i32::from(y));
                assert_ne!(echo, 0);
                assert!(motion(point(x, y), echo).is_none());
                assert!(motion(point(x.wrapping_add(1), y), echo).is_some());
            }
        }
        let echo = motion_echo(-50, 25);
        assert!(motion(point(-50, 25), echo).is_none());
        assert!(matches!(
            motion(point(-49, 25), echo),
            Some(Event::Motion(-49, 25))
        ));
        for delta in [-32768i16, -120, 1, 120, 32767] {
            let wp = usize::from(delta as u16) << 16;
            assert!(
                matches!(scroll(WM_MOUSEWHEEL, wp), Some(Event::Scroll(0, n)) if n == -i32::from(delta))
            );
            assert!(
                matches!(scroll(WM_MOUSEHWHEEL, wp), Some(Event::Scroll(n, 0)) if n == i32::from(delta))
            );
        }
        assert!(scroll(WM_MOUSEMOVE, 0).is_none());
    }
    #[test]
    fn all_five_buttons_keep_indices_positions_and_release_pairs() {
        for (down, up, wp, button_index) in [
            (WM_LBUTTONDOWN, WM_LBUTTONUP, 0, 0),
            (WM_MBUTTONDOWN, WM_MBUTTONUP, 0, 1),
            (WM_RBUTTONDOWN, WM_RBUTTONUP, 0, 2),
            (WM_XBUTTONDOWN, WM_XBUTTONUP, 1 << 16, 3),
            (WM_XBUTTONDOWN, WM_XBUTTONUP, 2 << 16, 4),
        ] {
            for (message, pressed) in [(down, true), (up, false)] {
                assert!(
                    matches!(button(message, wp, point(-12, 340)), Some(Event::Button(d, b, -12, 340))
                    if d == pressed && b == button_index)
                );
            }
        }
        assert!(button(WM_MOUSEMOVE, 0, 0).is_none());
    }
    #[test]
    fn captured_keys_keep_original_modifiers_extended_scans_and_publication_order() {
        let keys = Mutex::new(BTreeSet::from(["MetaLeft"]));
        let tab = KBDLLHOOKSTRUCT {
            vkCode: u32::from(VK_TAB),
            scanCode: 0x0f,
            flags: LLKHF_ALTDOWN,
            ..Default::default()
        };
        let mut events = Vec::new();
        captured_key(
            tab.vkCode as usize,
            captured_key_data(&tab, true, 4),
            &keys,
            |event| {
                assert!(
                    keys.try_lock().is_err(),
                    "Keep the existing pressed-key lock through publication"
                );
                events.push(event);
            },
        );
        assert!(keys.lock().unwrap().contains("Tab"));
        captured_key(
            tab.vkCode as usize,
            captured_key_data(&tab, false, 0),
            &keys,
            |event| events.push(event),
        );
        assert!(matches!(
            events.as_slice(),
            [Event::Key(true, "Tab", 4), Event::Key(false, "Tab", 0)]
        ));
        assert!(!keys.lock().unwrap().contains("Tab"));
        let ctrl = KBDLLHOOKSTRUCT {
            vkCode: u32::from(VK_CONTROL),
            scanCode: 0x1d,
            flags: LLKHF_EXTENDED,
            ..Default::default()
        };
        captured_key(
            ctrl.vkCode as usize,
            captured_key_data(&ctrl, true, 2),
            &keys,
            |event| assert!(matches!(event, Event::Key(true, "ControlRight", 2))),
        );
        let _held = keys.lock().unwrap();
        // Unknown keys were discarded before taking the mutex in the old path.
        captured_key(0, 0, &keys, |_| panic!("Unknown key must not be emitted"));
    }
    #[test]
    fn raw_relative_motion_keeps_both_axes_reversal_and_absolute_filtering() {
        for (x, y) in [(-400, 0), (1, 0), (0, -300), (0, 1), (0, 0)] {
            let mouse = RAWMOUSE {
                lLastX: x,
                lLastY: y,
                ..Default::default()
            };
            assert!(
                matches!(raw_motion(&mouse, true), Some(Event::RelativeMotion(a, b)) if a == x && b == y)
            );
            assert!(raw_motion(&mouse, false).is_none());
        }
        let absolute = RAWMOUSE {
            usFlags: MOUSE_MOVE_ABSOLUTE,
            lLastX: 65535,
            lLastY: 65535,
            ..Default::default()
        };
        assert!(raw_motion(&absolute, true).is_none());
    }
    #[test]
    fn character_adapter_preserves_utf16_and_filters_control_duplicates() {
        let mut decoder = crate::input::TextDecoder::default();
        assert!(text(0xd83e, &mut decoder).is_none());
        assert!(matches!(
            text(0xdd80, &mut decoder),
            Some(Event::Text(0x1f980))
        ));
        assert!(matches!(
            text('ő' as usize, &mut decoder),
            Some(Event::Text(0x151))
        ));
        for control in [0, 9, 13, 22] {
            assert!(text(control, &mut decoder).is_none());
        }
        assert!(text(0xd83e, &mut decoder).is_none());
        decoder = Default::default(); // The window resets this at focus loss.
        assert!(text(0xdd80, &mut decoder).is_none());
    }
    #[test]
    fn keys_preserve_physical_position_and_navigation() {
        assert_eq!(code(0x09, 0x0F, false), Some("Tab"));
        assert_eq!(code(0x5A, 0x15, false), Some("KeyY")); // Hungarian layout
        assert_eq!(code(0x56, 0x2F, false), Some("KeyV"));
        assert_eq!(code(0x11, 0x1D, true), Some("ControlRight"));
        assert_eq!(code(0x0D, 0x1C, true), Some("NumpadEnter"));
        assert_eq!(code(0x25, 0x4B, false), Some("Numpad4"));
        assert_eq!(code(0x25, 0x4B, true), Some("ArrowLeft"));
        assert_eq!(code(0, 0, false), None);
    }
}
