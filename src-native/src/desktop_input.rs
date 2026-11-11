//! Platform-independent physical-key vocabulary and Parsec keyboard framing.
use crate::control::header;
pub fn usage(code: &str) -> Option<i32> {
    let bytes = code.as_bytes();
    if code.starts_with("Key") && bytes.len() == 4 && bytes[3].is_ascii_uppercase() {
        return Some(i32::from(bytes[3] - b'A') + 4);
    }
    if code.starts_with("Digit") && bytes.len() == 6 && bytes[5].is_ascii_digit() {
        return Some(if bytes[5] == b'0' {
            39
        } else {
            i32::from(bytes[5] - b'1') + 30
        });
    }
    if let Some(n) = code.strip_prefix('F').and_then(|s| s.parse::<i32>().ok()) {
        if (1..=12).contains(&n) {
            return Some(57 + n);
        }
        if (13..=24).contains(&n) {
            return Some(91 + n);
        }
    }
    Some(match code {
        "Enter" => 40,
        "Escape" => 41,
        "Backspace" => 42,
        "Tab" => 43,
        "Space" => 44,
        "Minus" => 45,
        "Equal" => 46,
        "BracketLeft" => 47,
        "BracketRight" => 48,
        "Backslash" => 49,
        "Semicolon" => 51,
        "Quote" => 52,
        "Backquote" => 53,
        "Comma" => 54,
        "Period" => 55,
        "Slash" => 56,
        "CapsLock" => 57,
        "PrintScreen" => 70,
        "ScrollLock" => 71,
        "Pause" => 72,
        "Insert" => 73,
        "Home" => 74,
        "PageUp" => 75,
        "Delete" => 76,
        "End" => 77,
        "PageDown" => 78,
        "ArrowRight" => 79,
        "ArrowLeft" => 80,
        "ArrowDown" => 81,
        "ArrowUp" => 82,
        "NumLock" => 83,
        "NumpadDivide" => 84,
        "NumpadMultiply" => 85,
        "NumpadSubtract" => 86,
        "NumpadAdd" => 87,
        "NumpadEnter" => 88,
        "Numpad1" => 89,
        "Numpad2" => 90,
        "Numpad3" => 91,
        "Numpad4" => 92,
        "Numpad5" => 93,
        "Numpad6" => 94,
        "Numpad7" => 95,
        "Numpad8" => 96,
        "Numpad9" => 97,
        "Numpad0" => 98,
        "NumpadDecimal" => 99,
        "IntlBackslash" => 100,
        "ContextMenu" => 101,
        "NumpadEqual" => 103,
        "ControlLeft" => 224,
        "ShiftLeft" => 225,
        "AltLeft" => 226,
        "MetaLeft" => 227,
        "ControlRight" => 228,
        "ShiftRight" => 229,
        "AltRight" => 230,
        "MetaRight" => 231,
        _ => return None,
    })
}

#[derive(Default)]
pub struct Keyboard {
    pub(crate) modifiers: i32,
}
impl Keyboard {
    pub fn frame(&mut self, key: i32, compact: i32, down: bool) -> Option<bytes::Bytes> {
        if !(1..=255).contains(&key) {
            return None;
        }
        if (224..=231).contains(&key) {
            let bit = [64, 1, 256, 1024, 128, 2, 512, 2048][(key - 224) as usize];
            if down {
                self.modifiers |= bit;
            } else {
                self.modifiers &= !bit;
            }
        }
        let mut mods = self.modifiers;
        // A modifier may already be held when focus/session starts.
        for (source, bits, fallback) in [(1, 3, 1), (2, 192, 64), (4, 768, 256), (8, 3072, 1024)] {
            if compact & source != 0 && mods & bits == 0 {
                mods |= fallback;
            }
        }
        if compact & 16 != 0 {
            mods |= 8192;
        }
        if compact & 32 != 0 {
            mods |= 4096;
        }

        Some(header(0, key, mods, i32::from(down)))
    }
    pub fn reset(&mut self) {
        self.modifiers = 0;
    }
}

/// Stock cursor contract, shared by Windows and Android: hidden also requests capture.
pub fn relative_capture(relative: bool, hidden: bool) -> bool {
    relative || hidden
}
#[cfg(any(target_os = "android", target_os = "linux", test))]
mod pointing {
    use super::*;
    use anyhow::Result;
    use std::time::{Duration, Instant};
    /// Explicit UI selection has priority over broad device capability flags.
    /// Other standard remotes remain usable; their adapter must distinguish
    /// directional sources from a separate gamepad or physical keyboard.
    #[derive(Default)]
    pub struct RemoteSource {
        enabled: bool,
        owner: Option<i32>,
    }
    impl RemoteSource {
        pub fn configure(&mut self, enabled: bool, owner: Option<i32>) {
            self.enabled = enabled;
            self.owner = if enabled { owner } else { None };
        }
        pub fn remove(&mut self, device: i32) {
            if self.owner == Some(device) {
                self.owner = None;
            }
        }
        pub fn accepts(&self, device: i32, directional_source: bool) -> bool {
            self.enabled && (self.owner == Some(device) || directional_source)
        }
    }
    #[derive(Clone, Copy, Debug)]
    pub struct Pointer {
        pub x: f64,
        pub y: f64,
        pub relative: bool,
        pub moved: bool,
        pub viewport: (u32, u32),
        pub buttons: u32,
        pub wheel: (i32, i32),
    }
    impl Pointer {
        pub fn merge(&mut self, next: Self) -> bool {
            if self.buttons != next.buttons
                || self.viewport != next.viewport
                || self.relative != next.relative
                || !self.moved
                || !next.moved
                || self.wheel != (0, 0)
                || next.wheel != (0, 0)
            {
                return false;
            }
            if self.relative {
                let (x, y) = (self.x + next.x, self.y + next.y);
                if !x.is_finite() || !y.is_finite() {
                    return false;
                }
                self.x = x;
                self.y = y;
            } else {
                self.x = next.x;
                self.y = next.y;
            }
            true
        }
    }
    #[derive(Clone, Copy, Debug)]
    pub enum Event {
        Focus(bool),
        Release,
        Viewport(u32, u32),
        Key { usage: i32, mods: i32, down: bool },
        Pointer(Pointer),
        RemoteMode(bool, u32),
        RemoteKey { key: u8, down: bool, cancel: bool },
    }
    pub struct Engine {
        keys: Keyboard,
        active: bool,
        mode: bool,
        physical: bool,
        source: (u32, u32),
        client: (u32, u32),
        x: f64,
        y: f64,
        relative: bool,
        hidden: bool,
        buttons: u32,
        sent_buttons: u32,
        directions: u8,
        center: Option<Instant>,
        drag: bool,
        right: bool,
        long: Duration,
        last: Instant,
        held_since: Instant,
        remainder: (f64, f64),
        cursor_initialized: bool,
        pending_position: Option<(i32, i32)>,
    }
    impl Default for Engine {
        fn default() -> Self {
            let now = Instant::now();
            Self {
                keys: Keyboard::default(),
                active: false,
                mode: false,
                physical: false,
                source: (0, 0),
                client: (0, 0),
                x: 0.,
                y: 0.,
                relative: false,
                hidden: false,
                buttons: 0,
                sent_buttons: 0,
                directions: 0,
                center: None,
                drag: false,
                right: false,
                long: Duration::from_millis(500),
                last: now,
                held_since: now,
                remainder: (0., 0.),
                cursor_initialized: false,
                pending_position: None,
            }
        }
    }
    impl Engine {
        pub fn geometry(&mut self, source: (u32, u32)) {
            if self.source == source {
                return;
            }
            let (nx, ny) = if self.source.0 > 0 && self.source.1 > 0 {
                (self.x / self.source.0 as f64, self.y / self.source.1 as f64)
            } else {
                (0.5, 0.5)
            };
            self.source = source;
            self.x = nx * source.0 as f64;
            self.y = ny * source.1 as f64;
            self.restore_position();
        }
        pub fn host_cursor(&mut self, relative: bool, hidden: bool, x: i32, y: i32) {
            let relative = relative_capture(relative, hidden);
            if !self.cursor_initialized || (self.relative && !relative) {
                self.pending_position = Some((x, y));
                self.restore_position();
            }
            self.cursor_initialized = true;
            if self.relative != relative {
                self.remainder = (0., 0.);
            }
            self.relative = relative;
            self.hidden = hidden;
        }
        fn restore_position(&mut self) {
            if self.source.0 > 0 && self.source.1 > 0 {
                if let Some((x, y)) = self.pending_position.take() {
                    if x >= 0 && y >= 0 && x as u32 <= self.source.0 && y as u32 <= self.source.1 {
                        self.x = x as f64;
                        self.y = y as f64;
                    }
                }
            }
        }
        fn abs(&self) -> bytes::Bytes {
            header(3, 0, self.x.round() as i32, self.y.round() as i32)
        }
        fn mouse_buttons(
            &mut self,
            extra: u32,
            mut send: impl FnMut(&[u8]) -> Result<()>,
        ) -> Result<()> {
            let mask = self.buttons | if self.drag { 1 } else { 0 } | extra;
            for (bit, wire) in [(1, 1), (2, 3), (4, 2), (8, 4), (16, 5)] {
                if (self.sent_buttons ^ mask) & bit != 0 {
                    send(&header(1, wire, i32::from(mask & bit != 0), 0))?;
                }
            }
            self.sent_buttons = mask;
            Ok(())
        }
        fn click(&mut self, bit: u32, mut send: impl FnMut(&[u8]) -> Result<()>) -> Result<()> {
            if self.source.0 == 0 || self.source.1 == 0 {
                return Ok(());
            }
            if !self.relative {
                send(&self.abs())?;
            }
            self.mouse_buttons(bit, &mut send)?;
            self.mouse_buttons(0, send)
        }
        fn release(&mut self, mut send: impl FnMut(&[u8]) -> Result<()>) -> Result<()> {
            self.keys.reset();
            self.buttons = 0;
            self.sent_buttons = 0;
            self.directions = 0;
            self.center = None;
            self.drag = false;
            self.right = false;
            self.remainder = (0., 0.);
            send(&header(24, 0, 0, 0))
        }
        pub fn event(
            &mut self,
            event: Event,
            now: Instant,
            mut send: impl FnMut(&[u8]) -> Result<()>,
        ) -> Result<()> {
            match event {
                Event::Viewport(w, h) => {
                    self.client = (w, h);
                    return Ok(());
                }
                Event::Focus(active) => {
                    if self.active && !active {
                        self.release(&mut send)?;
                    }
                    self.active = active;
                    self.last = now;
                    return Ok(());
                }
                Event::Release => return self.release(send),
                Event::RemoteKey { cancel: true, .. } => {
                    self.directions = 0;
                    self.center = None;
                    self.drag = false;
                    self.right = false;
                    return self.mouse_buttons(0, send);
                }
                Event::RemoteMode(on, ms) => {
                    if self.drag {
                        self.drag = false;
                        self.mouse_buttons(0, &mut send)?;
                    }
                    self.mode = on;
                    self.directions = 0;
                    self.center = None;
                    self.right = false;
                    self.long = Duration::from_millis(u64::from(ms.clamp(200, 2000)));
                    self.last = now;
                    return Ok(());
                }
                _ if !self.active => return Ok(()),
                _ => {}
            }
            self.tick(now, &mut send)?;
            match event {
                Event::Key { usage, mods, down } => {
                    if let Some(frame) = self.keys.frame(usage, mods, down) {
                        send(&frame)?;
                    }
                }
                Event::Pointer(p) => {
                    self.physical = true;
                    self.client = p.viewport;
                    if self.source.0 == 0 || self.source.1 == 0 {
                        return Ok(());
                    }
                    if p.moved && p.x.is_finite() && p.y.is_finite() {
                        if p.relative && self.relative {
                            let x = p.x + self.remainder.0;
                            let y = p.y + self.remainder.1;
                            let dx = x.round().clamp(i32::MIN as f64, i32::MAX as f64) as i32;
                            let dy = y.round().clamp(i32::MIN as f64, i32::MAX as f64) as i32;
                            self.remainder = (x - f64::from(dx), y - f64::from(dy));
                            if dx != 0 || dy != 0 {
                                send(&header(3, 1, dx, dy))?;
                            }
                        } else {
                            let view = crate::viewport::Viewport {
                                source: self.source,
                                client: self.client,
                            };
                            if p.relative {
                                if let Some([l, t, r, b]) = view.rect() {
                                    self.x += p.x * self.source.0 as f64 / (r - l) as f64;
                                    self.y += p.y * self.source.1 as f64 / (b - t) as f64;
                                }
                            } else if let Some((x, y)) =
                                view.map(p.x.round() as i32, p.y.round() as i32)
                            {
                                self.x = x as f64;
                                self.y = y as f64;
                            }
                            self.clamp();
                            send(&self.abs())?;
                        }
                    }
                    self.buttons = p.buttons & 31;
                    self.mouse_buttons(0, &mut send)?;
                    if p.wheel != (0, 0) {
                        send(&header(2, p.wheel.0, p.wheel.1, 0))?;
                    }
                }
                Event::RemoteKey { key, down, .. } if self.mode => {
                    if key < 4 {
                        if down {
                            if self.directions == 0 {
                                self.held_since = now;
                                self.last = now;
                            }
                            self.directions |= 1 << key;
                            if self.center.is_some() && !self.right && !self.drag {
                                self.drag = true;
                                send(&self.abs())?;
                                self.mouse_buttons(0, &mut send)?;
                            }
                        } else {
                            self.directions &= !(1 << key);
                        }
                    } else if key == 4 {
                        if down && self.center.is_none() {
                            self.center = Some(now);
                            self.right = false;
                            if self.directions != 0 {
                                self.drag = true;
                                send(&self.abs())?;
                                self.mouse_buttons(0, &mut send)?;
                            }
                        } else if !down && self.center.take().is_some() {
                            if self.drag {
                                self.drag = false;
                                self.mouse_buttons(0, &mut send)?;
                            } else if !self.right {
                                self.click(1, &mut send)?;
                            }
                            self.right = false;
                        }
                    }
                }
                _ => {}
            }
            Ok(())
        }
        fn clamp(&mut self) {
            self.x = self.x.clamp(0., self.source.0 as f64);
            self.y = self.y.clamp(0., self.source.1 as f64);
        }
        pub fn tick(
            &mut self,
            now: Instant,
            mut send: impl FnMut(&[u8]) -> Result<()>,
        ) -> Result<()> {
            let dt = now
                .saturating_duration_since(self.last)
                .as_secs_f64()
                .min(0.05);
            self.last = now;
            if !self.active || !self.mode || self.source.0 == 0 || self.source.1 == 0 {
                return Ok(());
            }
            if self
                .center
                .is_some_and(|t| now.saturating_duration_since(t) >= self.long)
                && !self.drag
                && !self.right
            {
                self.click(2, &mut send)?;
                self.right = true;
            }
            if self.directions != 0 {
                let x = i32::from(self.directions & 8 != 0) - i32::from(self.directions & 4 != 0);
                let y = i32::from(self.directions & 2 != 0) - i32::from(self.directions & 1 != 0);
                if x != 0 || y != 0 {
                    let speed = self.source.1 as f64
                        * if now.saturating_duration_since(self.held_since)
                            > Duration::from_millis(700)
                        {
                            1.1
                        } else {
                            0.35
                        };
                    let diagonal = if x != 0 && y != 0 {
                        std::f64::consts::FRAC_1_SQRT_2
                    } else {
                        1.
                    };
                    let (dx, dy) = (
                        x as f64 * speed * dt * diagonal,
                        y as f64 * speed * dt * diagonal,
                    );
                    if self.relative {
                        send(&header(3, 1, dx.round() as i32, dy.round() as i32))?;
                    } else {
                        self.x += dx;
                        self.y += dy;
                        self.clamp();
                        send(&self.abs())?;
                    }
                }
            }
            Ok(())
        }
        pub fn ticking(&self) -> bool {
            self.active
                && self.mode
                && (self.directions != 0 || (self.center.is_some() && !self.right && !self.drag))
        }
        pub fn pointer(&self) -> u64 {
            if !self.active
                || (!self.mode && !self.physical)
                || self.hidden
                || self.relative
                || self.source.0 == 0
                || self.source.1 == 0
            {
                return 0;
            }
            let client = if self.client.0 == 0 || self.client.1 == 0 {
                self.source
            } else {
                self.client
            };
            let Some((x, y)) = crate::viewport::Viewport {
                source: self.source,
                client,
            }
            .unmap(self.x.round() as i32, self.y.round() as i32) else {
                return 0;
            };
            (1 << 32)
                | ((x as u64 * 65535 / client.0 as u64) & 65535)
                | (((y as u64 * 65535 / client.1 as u64) & 65535) << 16)
        }
    }
}
#[cfg(any(target_os = "android", target_os = "linux", test))]
pub use pointing::{Engine, Event, Pointer, RemoteSource};

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    fn setup() -> Engine {
        let mut e = Engine::default();
        e.geometry((1920, 1080));
        let n = Instant::now();
        e.event(Event::Focus(true), n, |_| Ok(())).unwrap();
        e.event(Event::RemoteMode(true, 500), n, |_| Ok(()))
            .unwrap();
        e
    }
    fn key(key: u8, down: bool) -> Event {
        Event::RemoteKey {
            key,
            down,
            cancel: false,
        }
    }
    #[test]
    fn remote_owner_routes_navigation_independently_of_event_source() {
        let mut source = RemoteSource::default();
        assert!(!source.accepts(1, true));
        source.configure(true, Some(1));
        assert!(source.accepts(1, true));
        assert!(source.accepts(1, false)); // Same remote reports an arrow as keyboard input.
        assert!(!source.accepts(2, false)); // The adapter identifies a separate gamepad.
        assert!(!source.accepts(3, false)); // A separate keyboard keeps its arrows.
        assert!(source.accepts(4, true)); // Another remote / composite directional device.
        source.remove(2);
        assert!(source.accepts(1, false));
        source.remove(1);
        assert!(!source.accepts(1, false)); // Reused OS device ID is not the old remote.
        source.configure(false, Some(1));
        assert!(!source.accepts(1, true));
        source.configure(true, None);
        assert!(!source.accepts(3, false));
        assert!(source.accepts(-1, true)); // Android virtual TV remote.
        assert!(!source.accepts(2, false));
    }
    #[test]
    fn each_remote_direction_moves_until_released() {
        for (key, dx, dy) in [(0, 0, -1), (1, 0, 1), (2, -1, 0), (3, 1, 0)] {
            let mut engine = setup();
            let now = Instant::now();
            let mut frames = Vec::new();
            let mut send = |frame: &[u8]| {
                frames.push(frame.to_vec());
                Ok(())
            };
            engine.event(self::key(key, true), now, &mut send).unwrap();
            engine
                .tick(now + Duration::from_millis(20), &mut send)
                .unwrap();
            engine
                .event(
                    self::key(key, false),
                    now + Duration::from_millis(30),
                    &mut send,
                )
                .unwrap();
            let stopped = engine.pointer();
            engine
                .tick(now + Duration::from_millis(80), &mut send)
                .unwrap();
            assert_eq!(stopped, engine.pointer());
            assert!(!engine.ticking());
            let last = frames.last().unwrap();
            assert_eq!(last[12], 3);
            let x = i32::from_be_bytes(last[4..8].try_into().unwrap());
            let y = i32::from_be_bytes(last[8..12].try_into().unwrap());
            assert_eq!((x - 960).signum(), dx);
            assert_eq!((y - 540).signum(), dy);
        }
    }
    #[test]
    fn viewport_and_focus_gate_remote_motion_timer() {
        let mut e = setup();
        let n = Instant::now();
        e.event(Event::Viewport(1024, 768), n, |_| Ok(())).unwrap();
        assert!(!e.ticking());
        e.event(key(3, true), n, |_| Ok(())).unwrap();
        assert!(e.ticking());
        e.tick(n + Duration::from_millis(20), |_| Ok(())).unwrap();
        let position = e.pointer();
        assert_ne!(position, 0);
        let y = (position >> 16) & 65535;
        assert!((32000..33500).contains(&y));
        e.event(Event::Focus(false), n + Duration::from_millis(30), |_| {
            Ok(())
        })
        .unwrap();
        assert!(!e.ticking());
        assert_eq!(e.pointer(), 0);
    }
    #[test]
    fn short_double_and_long_ok_never_mix_mouse_buttons() {
        let mut e = setup();
        let n = Instant::now();
        let mut frames = Vec::new();
        let mut send = |f: &[u8]| {
            frames.push(f.to_vec());
            Ok(())
        };
        for t in [0, 150] {
            e.event(key(4, true), n + Duration::from_millis(t), &mut send)
                .unwrap();
            e.event(key(4, false), n + Duration::from_millis(t + 50), &mut send)
                .unwrap();
        }
        e.event(key(4, true), n + Duration::from_millis(300), &mut send)
            .unwrap();
        e.tick(n + Duration::from_millis(900), &mut send).unwrap();
        e.event(key(4, false), n + Duration::from_millis(950), &mut send)
            .unwrap();
        let buttons: Vec<_> = frames
            .iter()
            .filter(|f| f[12] == 1)
            .map(|f| {
                (
                    i32::from_be_bytes(f[..4].try_into().unwrap()),
                    i32::from_be_bytes(f[4..8].try_into().unwrap()),
                )
            })
            .collect();
        assert_eq!(
            buttons,
            vec![(1, 1), (1, 0), (1, 1), (1, 0), (3, 1), (3, 0)]
        );
    }
    #[test]
    fn ok_direction_drags_and_cancel_does_not_become_right_click() {
        let mut e = setup();
        let n = Instant::now();
        let mut frames = Vec::new();
        let mut send = |f: &[u8]| {
            frames.push(f.to_vec());
            Ok(())
        };
        e.event(key(4, true), n, &mut send).unwrap();
        e.event(key(3, true), n + Duration::from_millis(20), &mut send)
            .unwrap();
        e.tick(n + Duration::from_millis(40), &mut send).unwrap();
        e.tick(n + Duration::from_millis(900), &mut send).unwrap();
        e.event(key(4, false), n + Duration::from_millis(920), &mut send)
            .unwrap();
        e.event(
            Event::Focus(false),
            n + Duration::from_millis(930),
            &mut send,
        )
        .unwrap();
        assert!(frames.iter().any(|f| f[12] == 1 && f[3] == 1 && f[7] == 1));
        assert!(frames.iter().any(|f| f[12] == 1 && f[3] == 1 && f[7] == 0));
        assert!(!frames.iter().any(|f| f[12] == 1 && f[3] == 3));
        assert_eq!(frames.last().unwrap()[12], 24);
        let mut e = setup();
        let mut frames = Vec::new();
        let mut send = |f: &[u8]| {
            frames.push(f.to_vec());
            Ok(())
        };
        e.event(key(4, true), n, &mut send).unwrap();
        e.event(
            Event::RemoteKey {
                key: 4,
                down: false,
                cancel: true,
            },
            n + Duration::from_secs(1),
            &mut send,
        )
        .unwrap();
        assert!(frames.is_empty());
    }
    #[test]
    fn relative_camera_and_desktop_endpoint_restore_match_windows_contract() {
        let mut e = setup();
        e.host_cursor(true, false, 100, 100);
        assert_eq!(e.pointer(), 0);
        let n = Instant::now();
        let mut frames = Vec::new();
        let mut send = |f: &[u8]| {
            frames.push(f.to_vec());
            Ok(())
        };
        e.event(
            Event::Pointer(Pointer {
                x: 7.,
                y: -3.,
                relative: true,
                moved: true,
                viewport: (2560, 1440),
                buttons: 0,
                wheel: (0, 0),
            }),
            n,
            &mut send,
        )
        .unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0], header(3, 1, 7, -3).as_ref());
        e.host_cursor(false, false, 1800, 900);
        assert_ne!(e.pointer(), 0);
        let restored = e.pointer();
        e.host_cursor(false, false, 0, 0);
        assert_eq!(restored, e.pointer());
        e.host_cursor(false, true, 0, 0);
        assert_eq!(e.pointer(), 0);
    }
    #[test]
    fn physical_mouse_chords_and_keyboard_release_preserve_all_edges() {
        let mut e = setup();
        let n = Instant::now();
        let mut frames = Vec::new();
        let mut send = |f: &[u8]| {
            frames.push(f.to_vec());
            Ok(())
        };
        for buttons in [1, 3, 2, 0] {
            e.event(
                Event::Pointer(Pointer {
                    x: 0.,
                    y: 0.,
                    relative: true,
                    moved: false,
                    viewport: (1920, 1080),
                    buttons,
                    wheel: (0, 0),
                }),
                n,
                &mut send,
            )
            .unwrap();
        }
        e.event(
            Event::Key {
                usage: 230,
                mods: 4,
                down: true,
            },
            n,
            &mut send,
        )
        .unwrap();
        e.event(Event::Release, n, &mut send).unwrap();
        e.event(
            Event::Key {
                usage: 4,
                mods: 0,
                down: true,
            },
            n,
            &mut send,
        )
        .unwrap();
        assert_eq!(
            frames[..4].iter().map(|f| (f[3], f[7])).collect::<Vec<_>>(),
            vec![(1, 1), (3, 1), (1, 0), (3, 0)]
        );
        assert_eq!(&frames[4][4..8], &512i32.to_be_bytes());
        assert_eq!(&frames[6][4..8], &0i32.to_be_bytes());
    }
}
