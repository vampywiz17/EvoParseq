//! egui/Wayland event translation only. Shared Rust owns input state and framing.
use crate::{
    desktop_input::{Event as Input, Pointer},
    playback::Playback,
};
use eframe::egui::{self, Event, Key, Modifiers, Pos2, Rect, Vec2};

#[derive(Default)]
pub struct DesktopInput {
    pub enabled: bool,
    pub video: Option<Rect>,
    pub controls: Option<Rect>,
    buttons: u32,
    modifiers: u8,
    wheel: Vec2,
    grabbed: bool,
}
impl DesktopInput {
    pub fn release(&mut self, ctx: &egui::Context, playback: &Playback) {
        playback.push_input(Input::Focus(false));
        self.enabled = false;
        self.buttons = 0;
        self.modifiers = 0;
        self.wheel = Vec2::ZERO;
        self.grabbed = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(egui::CursorGrab::None));
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
    }
    pub fn enable(&mut self, playback: &Playback) {
        if !self.enabled && playback.input_ready() && self.video.is_some() {
            self.enabled = true;
            playback.push_input(Input::Focus(true));
        }
    }
    pub fn cursor(&mut self, ctx: &egui::Context, playback: &Playback) {
        let grab = self.enabled && playback.capture() && ctx.input(|i| i.focused);
        if self.grabbed != grab {
            self.grabbed = grab;
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(if grab {
                egui::CursorGrab::Locked
            } else {
                egui::CursorGrab::None
            }));
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(!grab));
        }
    }
    fn inside(&self, pos: Pos2) -> bool {
        self.video.is_some_and(|rect| rect.contains(pos))
            && !self.controls.is_some_and(|rect| rect.contains(pos))
    }
    fn pointer(
        &self,
        playback: &Playback,
        pos: Vec2,
        relative: bool,
        moved: bool,
        wheel: (i32, i32),
    ) {
        let Some(rect) = self.video else { return };
        playback.push_input(Input::Pointer(Pointer {
            x: f64::from(pos.x),
            y: f64::from(pos.y),
            relative,
            moved,
            viewport: (
                rect.width().round().max(1.) as u32,
                rect.height().round().max(1.) as u32,
            ),
            buttons: self.buttons,
            wheel,
        }));
    }
    fn modifiers(&mut self, playback: &Playback, next: Modifiers) -> i32 {
        let bits = u8::from(next.shift)
            | (u8::from(next.ctrl) << 1)
            | (u8::from(next.alt) << 2)
            | (u8::from(next.mac_cmd) << 3);
        // egui exposes aggregate modifier state, not left/right modifier keys.
        for (bit, usage) in [(1, 225), (2, 224), (4, 226), (8, 227)] {
            if (self.modifiers ^ bits) & bit != 0 {
                playback.push_input(Input::Key {
                    usage,
                    mods: 0,
                    down: bits & bit != 0,
                });
            }
        }
        self.modifiers = bits;
        i32::from(bits)
    }
    pub fn events(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput, playback: &Playback) {
        if !raw.focused || !playback.input_ready() || playback.stopped() {
            if self.enabled {
                self.release(ctx, playback);
            }
            return;
        }
        raw.events.retain(|event| {
            if matches!(event, Event::Key { key: Key::M, pressed: true, modifiers, .. } if modifiers.ctrl && modifiers.shift) && self.enabled {
                self.release(ctx, playback);
                return false;
            }
            if let Event::PointerButton { pos, pressed: true, .. } = event {
                if !self.enabled && self.inside(*pos) { self.enable(playback); }
            }
            if !self.enabled { return true; }
            match event {
                Event::Key { physical_key, key, pressed, modifiers, .. } => {
                    let mods = self.modifiers(playback, *modifiers);
                    if let Some(usage) = usage(physical_key.unwrap_or(*key)) {
                        playback.push_input(Input::Key { usage, mods, down: *pressed });
                    }
                    false
                }
                Event::Copy | Event::Cut | Event::Paste(_) => {
                    let key = match event { Event::Copy => 6, Event::Cut => 27, _ => 25 };
                    // Forward the shortcut only. Never synchronize clipboard contents.
                    playback.push_input(Input::Key { usage: key, mods: 2, down: true });
                    playback.push_input(Input::Key { usage: key, mods: 2, down: false });
                    false
                }
                Event::Text(_) | Event::Ime(_) => false,
                Event::PointerMoved(pos) if !self.grabbed && (self.inside(*pos) || self.buttons != 0) => {
                    let local = *pos - self.video.unwrap().min;
                    self.pointer(playback, local, false, true, (0,0));
                    false
                }
                Event::MouseMoved(delta) if self.grabbed => {
                    self.pointer(playback, *delta, true, true, (0,0));
                    false
                }
                Event::PointerButton { pos, button, pressed, modifiers } if self.inside(*pos) || self.buttons != 0 || self.grabbed => {
                    self.modifiers(playback, *modifiers);
                    let bit = match button { egui::PointerButton::Primary => 1, egui::PointerButton::Secondary => 2, egui::PointerButton::Middle => 4, egui::PointerButton::Extra1 => 8, egui::PointerButton::Extra2 => 16 };
                    if *pressed { self.buttons |= bit; } else { self.buttons &= !bit; }
                    self.pointer(playback, *pos - self.video.unwrap().min, false, !self.grabbed, (0,0));
                    false
                }
                Event::MouseWheel { unit, delta, .. } => {
                    let scale = match unit { egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => 120., egui::MouseWheelUnit::Point => 3. };
                    self.wheel += *delta * scale;
                    let ticks = (self.wheel.x.trunc() as i32, self.wheel.y.trunc() as i32);
                    self.wheel -= Vec2::new(ticks.0 as f32, ticks.1 as f32);
                    self.pointer(playback, Vec2::ZERO, self.grabbed, false, ticks);
                    false
                }
                _ => true,
            }
        });
        if self.enabled {
            self.modifiers(playback, raw.modifiers);
        }
    }
}
fn usage(key: Key) -> Option<i32> {
    let name = key.name();
    if name.len() == 1 {
        let code = name.as_bytes()[0];
        if code.is_ascii_uppercase() {
            return Some(i32::from(code - b'A') + 4);
        }
        if code.is_ascii_digit() {
            return Some(if code == b'0' {
                39
            } else {
                i32::from(code - b'1') + 30
            });
        }
    }
    Some(match key {
        Key::ArrowDown => 81,
        Key::ArrowUp => 82,
        Key::ArrowLeft => 80,
        Key::ArrowRight => 79,
        Key::Backtick => 53,
        Key::Backslash | Key::Pipe => 49,
        Key::Slash | Key::Questionmark => 56,
        Key::OpenBracket | Key::OpenCurlyBracket => 47,
        Key::CloseBracket | Key::CloseCurlyBracket => 48,
        Key::Minus => 45,
        Key::Equals | Key::Plus => 46,
        Key::Comma => 54,
        Key::Period => 55,
        Key::Semicolon | Key::Colon => 51,
        Key::Quote => 52,
        _ => return crate::desktop_input::usage(name),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linux_input_release_hotkey_drops_pressed_buttons_and_focus_loss_releases_host() {
        let ctx = egui::Context::default();
        let playback = Playback::new(ctx.clone());
        playback.set_input_ready(true);
        let mut adapter = DesktopInput {
            video: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 600.))),
            ..Default::default()
        };
        let mut raw = egui::RawInput::default();
        raw.events.push(Event::PointerButton {
            pos: egui::pos2(100., 100.),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::default(),
        });
        adapter.events(&ctx, &mut raw, &playback);
        assert!(adapter.enabled);
        raw.events = vec![Event::Key {
            key: Key::M,
            physical_key: Some(Key::M),
            pressed: true,
            repeat: false,
            modifiers: Modifiers {
                ctrl: true,
                shift: true,
                ..Default::default()
            },
        }];
        adapter.events(&ctx, &mut raw, &playback);
        assert!(!adapter.enabled);
        let mut batch = std::collections::VecDeque::new();
        playback.input.take(&mut batch).unwrap();
        assert!(matches!(
            batch.back(),
            Some(crate::input_handoff::Event::Desktop(Input::Focus(false)))
        ));
        assert!(!batch.iter().any(|e| matches!(
            e,
            crate::input_handoff::Event::Desktop(Input::Key { usage: 16, .. })
        )));
        adapter.enable(&playback);
        raw.focused = false;
        adapter.events(&ctx, &mut raw, &playback);
        assert!(!adapter.enabled);
    }
    #[test]
    fn linux_physical_key_translation_does_not_use_text_or_keyboard_layout() {
        assert_eq!(usage(Key::Y), Some(28));
        assert_eq!(usage(Key::Z), Some(29));
        assert_eq!(usage(Key::OpenBracket), Some(47));
        assert_eq!(usage(Key::F24), Some(115));
    }
}
