//! egui/Wayland event translation only. Shared Rust owns input state and framing.
use crate::{
    desktop_input::{Event as Input, Pointer},
    playback::Playback,
};
use eframe::egui::{self, Event, Pos2, Rect, Vec2};

#[derive(Default)]
pub struct DesktopInput {
    pub enabled: bool,
    pub video: Option<Rect>,
    pub controls: Option<Rect>,
    buttons: u32,
    keyboard: std::sync::Arc<crate::native_loop::KeyboardBridge>,
    wheel: Vec2,
    grabbed: bool,
}
impl DesktopInput {
    pub fn new(keyboard: std::sync::Arc<crate::native_loop::KeyboardBridge>) -> Self {
        Self {
            keyboard,
            ..Default::default()
        }
    }
    pub fn release(&mut self, ctx: &egui::Context, playback: &Playback) {
        playback.push_input(Input::Focus(false));
        self.enabled = false;
        self.buttons = 0;
        self.keyboard.bind(None);
        self.wheel = Vec2::ZERO;
        self.grabbed = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(egui::CursorGrab::None));
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
    }
    pub fn enable(&mut self, playback: &std::sync::Arc<Playback>) {
        if !self.enabled && playback.input_ready() && self.video.is_some() {
            self.enabled = true;
            self.keyboard.bind(Some(playback.clone()));
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
    pub fn events(
        &mut self,
        ctx: &egui::Context,
        raw: &mut egui::RawInput,
        playback: &std::sync::Arc<Playback>,
    ) {
        if self.keyboard.take_release() {
            self.release(ctx, playback);
        }
        if !raw.focused || !playback.input_ready() || playback.stopped() {
            if self.enabled {
                self.release(ctx, playback);
            }
            return;
        }
        raw.events.retain(|event| {
            if let Event::PointerButton {
                pos, pressed: true, ..
            } = event
            {
                if !self.enabled && self.inside(*pos) {
                    self.enable(playback);
                }
            }
            if !self.enabled {
                return true;
            }
            match event {
                Event::Key { .. }
                | Event::Copy
                | Event::Cut
                | Event::Paste(_)
                | Event::Text(_)
                | Event::Ime(_) => false,
                Event::PointerMoved(pos)
                    if !self.grabbed && (self.inside(*pos) || self.buttons != 0) =>
                {
                    let local = *pos - self.video.unwrap().min;
                    self.pointer(playback, local, false, true, (0, 0));
                    false
                }
                Event::MouseMoved(delta) if self.grabbed => {
                    self.pointer(playback, *delta, true, true, (0, 0));
                    false
                }
                Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    ..
                } if self.inside(*pos) || self.buttons != 0 || self.grabbed => {
                    let bit = match button {
                        egui::PointerButton::Primary => 1,
                        egui::PointerButton::Secondary => 2,
                        egui::PointerButton::Middle => 4,
                        egui::PointerButton::Extra1 => 8,
                        egui::PointerButton::Extra2 => 16,
                    };
                    if *pressed {
                        self.buttons |= bit;
                    } else {
                        self.buttons &= !bit;
                    }
                    self.pointer(
                        playback,
                        *pos - self.video.unwrap().min,
                        false,
                        !self.grabbed,
                        (0, 0),
                    );
                    false
                }
                Event::MouseWheel { unit, delta, .. } => {
                    let scale = match unit {
                        egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => 120.,
                        egui::MouseWheelUnit::Point => 3.,
                    };
                    self.wheel += *delta * scale;
                    let ticks = (self.wheel.x.trunc() as i32, self.wheel.y.trunc() as i32);
                    self.wheel -= Vec2::new(ticks.0 as f32, ticks.1 as f32);
                    self.pointer(playback, Vec2::ZERO, self.grabbed, false, ticks);
                    false
                }
                _ => true,
            }
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use egui::Modifiers;
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
        adapter.keyboard.release();
        raw.events.clear();
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
}
