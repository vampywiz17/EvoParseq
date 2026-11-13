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
    pub badge: Option<Rect>,
    buttons: u32,
    keyboard: std::sync::Arc<crate::native_loop::KeyboardBridge>,
    wheel: Vec2,
    grabbed: bool,
    confined: bool,
    unlock_after_present: bool,
    last_absolute: Option<Pos2>,
    capture_anchor: Option<Pos2>,
    restored_position: Option<Pos2>,
    peer_restore: Option<(usize, i32, i32)>,
}
impl DesktopInput {
    pub fn keyboard(&self) -> std::sync::Arc<crate::native_loop::KeyboardBridge> {
        self.keyboard.clone()
    }
    pub fn new(keyboard: std::sync::Arc<crate::native_loop::KeyboardBridge>) -> Self {
        Self {
            keyboard,
            ..Default::default()
        }
    }
    pub fn session(&self, playback: &std::sync::Arc<Playback>) {
        self.keyboard.session(playback);
    }
    pub fn refresh_hz(&self) -> Option<f64> {
        let mhz = self
            .keyboard
            .refresh_millihertz
            .load(std::sync::atomic::Ordering::Relaxed);
        (mhz > 0).then_some(f64::from(mhz) / 1000.)
    }
    pub fn release(&mut self, ctx: &egui::Context, playback: &Playback) {
        playback
            .clipboard_allowed
            .store(false, std::sync::atomic::Ordering::Release);
        playback.controller_focus(false);
        playback.push_input(Input::Focus(false));
        self.enabled = false;
        self.buttons = 0;
        self.keyboard.unbind(playback);
        self.wheel = Vec2::ZERO;
        self.grabbed = false;
        self.confined = false;
        self.unlock_after_present = false;
        self.capture_anchor = None;
        self.restored_position = None;
        self.peer_restore = None;
        playback.take_cursor_restore();
        self.controls = None;
        self.badge = None;
        Self::restore_cursor(ctx);
    }
    pub fn enable(&mut self, playback: &std::sync::Arc<Playback>) {
        if !self.enabled && playback.input_ready() && self.video.is_some() {
            self.enabled = true;
            playback.activate_input();
            playback
                .clipboard_allowed
                .store(true, std::sync::atomic::Ordering::Release);
            playback.clipboard_read();
            self.keyboard.bind(Some(playback.clone()));
            playback.push_input(Input::Focus(true));
        }
    }
    pub fn cursor(&mut self, ctx: &egui::Context, playback: &Playback) {
        if let Some((stream, x, y)) = self.peer_restore.take() {
            if self.enabled && ctx.input(|i| i.focused) {
                if let Some(position) = playback.cursor_endpoint(stream, x, y) {
                    ctx.send_viewport_cmd_to(
                        if stream == 0 {
                            egui::ViewportId::ROOT
                        } else {
                            crate::display_routes::viewport(stream)
                        },
                        egui::ViewportCommand::CursorPosition(position),
                    );
                }
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
            ctx.set_cursor_icon(egui::CursorIcon::Default);
        }
        let grab = self.enabled && playback.capture() && ctx.input(|i| i.focused);
        if grab && self.unlock_after_present {
            self.unlock_after_present = false;
            self.restored_position = None;
        }
        if self.grabbed
            && !grab
            && self.enabled
            && ctx.input(|i| i.focused)
            && !self.unlock_after_present
        {
            if let Some((stream, x, y)) = playback.take_cursor_restore() {
                if stream != playback.stream() {
                    if playback.cursor_endpoint(stream, x, y).is_some() {
                        // Commit the source unlock before sending a peer warp on
                        // the next presentation. Viewport command order is not global.
                        ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(
                            egui::CursorGrab::None,
                        ));
                        ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(false));
                        self.peer_restore = Some((stream, x, y));
                        self.grabbed = false;
                        self.confined = false;
                        ctx.set_cursor_icon(egui::CursorIcon::None);
                        ctx.request_repaint();
                        return;
                    }
                    // A disappeared peer must not warp inside the wrong window.
                } else if let Some(rect) = self.video {
                    let viewport = crate::viewport::Viewport {
                        source: playback.input_geometry(),
                        client: (rect.width().round() as u32, rect.height().round() as u32),
                    };
                    if let Some((x, y)) = viewport.unmap(x, y) {
                        let position = rect.min + egui::vec2(x as f32, y as f32);
                        // Wayland permits position hints only while the pointer is locked.
                        // Keep it locked through this presentation's surface commit, then
                        // unlock on the next frame. No recurring warp in absolute mode.
                        ctx.send_viewport_cmd(egui::ViewportCommand::CursorPosition(position));
                        ctx.set_cursor_icon(egui::CursorIcon::None);
                        self.restored_position = Some(position);
                        self.unlock_after_present = true;
                        ctx.request_repaint();
                        return;
                    }
                }
            }
        }
        if self.grabbed != grab {
            self.grabbed = grab;
            self.unlock_after_present = false;
            if grab {
                self.capture_anchor = self.last_absolute;
                self.restored_position = None;
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(!grab));
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(if grab {
                egui::CursorGrab::Locked
            } else {
                egui::CursorGrab::None
            }));
        }
        let confined = self.enabled
            && !grab
            && ctx.input(|i| i.focused)
            && matches!(
                playback
                    .immersive
                    .load(std::sync::atomic::Ordering::Acquire),
                1 | 3
            );
        if self.confined != confined {
            self.confined = confined;
            if !grab {
                ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(if confined {
                    egui::CursorGrab::Confined
                } else {
                    egui::CursorGrab::None
                }));
            }
        }
        // Keep None while locked, but let ordinary UI hover choose its cursor.
        // Repeated native visibility commands would overwrite Wayland CSD resize
        // cursors even though the capture state has not changed.
        if grab {
            ctx.set_cursor_icon(egui::CursorIcon::None);
        }
    }
    pub fn restore_cursor(ctx: &egui::Context) {
        ctx.set_cursor_icon(egui::CursorIcon::Default);
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(egui::CursorGrab::None));
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
    }
    fn inside(&self, pos: Pos2) -> bool {
        self.video.is_some_and(|rect| rect.contains(pos))
            && !self.controls.is_some_and(|rect| rect.contains(pos))
            && !self.badge.is_some_and(|rect| rect.contains(pos))
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
        let paste = raw
            .events
            .iter()
            .any(|event| matches!(event, Event::Paste(_)))
            && self.keyboard.take_paste();
        raw.events.retain(|event| {
            if let Event::PointerButton {
                pos, pressed: true, ..
            } = event
            {
                if !self.enabled && self.inside(*pos) {
                    self.enable(playback);
                }
            }
            if self.enabled && matches!(event, Event::PointerButton {pos, pressed:true,..} if self.controls.is_some_and(|r|r.contains(*pos)) || self.badge.is_some_and(|r|r.contains(*pos))) {
                self.release(ctx,playback);
            }
            if !self.enabled { return true; }
            match event {
                Event::Paste(text) => {
                    if text.len()<=crate::clipboard_sync::MAX_TEXT && !text.contains('\0') {
                        playback.push_session(crate::session_controls::Command::Clipboard{text:text.clone(),paste});
                    }
                    false
                }
                Event::Key { .. }
                | Event::Copy
                | Event::Cut
                | Event::Text(_)
                | Event::Ime(_) => false,
                Event::PointerMoved(pos)
                    if !self.grabbed && (self.inside(*pos) || self.buttons != 0) =>
                {
                    // Do not send the stale pre-lock location or the compositor's
                    // restoration echo back to the host as a new absolute move.
                    if self.capture_anchor == Some(*pos) || self.restored_position == Some(*pos) { return true; }
                    self.capture_anchor = None;
                    self.restored_position = None;
                    self.last_absolute = Some(*pos);
                    let local = *pos - self.video.unwrap().min;
                    self.pointer(playback, local, false, true, (0, 0));
                    // Keep egui's hover/leave bookkeeping current as well.
                    true
                }
                Event::MouseMoved(delta) if self.grabbed && playback.capture() => {
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
                        !self.grabbed
                            && self.capture_anchor != Some(*pos)
                            && self.restored_position != Some(*pos),
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
    fn linux_session_control_click_releases_host_without_forwarding_local_mouse() {
        let ctx = egui::Context::default();
        let playback = Playback::new(ctx.clone());
        playback.set_input_ready(true);
        let mut adapter = DesktopInput {
            enabled: true,
            video: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 600.))),
            badge: Some(Rect::from_min_size(egui::pos2(16., 16.), Vec2::splat(48.))),
            ..Default::default()
        };
        let mut raw = egui::RawInput::default();
        raw.events.push(Event::PointerButton {
            pos: egui::pos2(30., 30.),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::default(),
        });
        adapter.events(&ctx, &mut raw, &playback);
        assert!(!adapter.enabled);
        assert_eq!(raw.events.len(), 1, "local overlay must receive its click");
        let mut batch = std::collections::VecDeque::new();
        playback.input.take(&mut batch).unwrap();
        assert_eq!(batch.len(), 1);
        assert!(matches!(
            batch.pop_front(),
            Some(crate::input_handoff::Event::Desktop(Input::Focus(false)))
        ));
    }
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

#[cfg(test)]
mod cursor_tests {
    use super::*;
    #[test]
    fn linux_cursor_restore_clears_both_egui_icon_and_native_visibility() {
        let ctx = egui::Context::default();
        let playback = Playback::new(ctx.clone());
        playback.set_capture(true);
        let mut adapter = DesktopInput {
            enabled: true,
            ..Default::default()
        };
        let locked = ctx.run(egui::RawInput::default(), |ctx| {
            adapter.cursor(ctx, &playback)
        });
        assert_eq!(locked.platform_output.cursor_icon, egui::CursorIcon::None);
        let restored = ctx.run(egui::RawInput::default(), |ctx| {
            adapter.release(ctx, &playback);
            DesktopInput::restore_cursor(ctx);
        });
        assert_eq!(
            restored.platform_output.cursor_icon,
            egui::CursorIcon::Default
        );
        let commands = &restored
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .unwrap()
            .commands;
        assert!(
            commands
                .iter()
                .any(|cmd| matches!(cmd, egui::ViewportCommand::CursorVisible(true)))
        );
        assert!(commands.iter().any(|cmd| matches!(
            cmd,
            egui::ViewportCommand::CursorGrab(egui::CursorGrab::None)
        )));
    }
}

#[cfg(test)]
mod ordinary_cursor_tests {
    use super::*;
    #[test]
    fn linux_unlocked_cursor_preserves_widget_cursor_and_does_not_override_decorations() {
        let ctx = egui::Context::default();
        let playback = Playback::new(ctx.clone());
        let mut adapter = DesktopInput::default();
        for _ in 0..3 {
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                ctx.set_cursor_icon(egui::CursorIcon::Text);
                adapter.cursor(ctx, &playback);
            });
            assert_eq!(output.platform_output.cursor_icon, egui::CursorIcon::Text);
            assert!(
                !output.viewport_output[&egui::ViewportId::ROOT]
                    .commands
                    .iter()
                    .any(|cmd| matches!(
                        cmd,
                        egui::ViewportCommand::CursorVisible(_)
                            | egui::ViewportCommand::CursorGrab(_)
                    ))
            );
        }
    }
}

#[cfg(test)]
mod drag_restore_tests {
    use super::*;
    #[test]
    fn linux_drag_exit_commits_position_hint_before_unlock_and_ignores_stale_anchor() {
        let ctx = egui::Context::default();
        let playback = Playback::new(ctx.clone());
        playback.set_input_ready(true);
        playback.set_input_geometry((1920, 1080));
        let mut adapter = DesktopInput {
            enabled: true,
            video: Some(Rect::from_min_size(
                egui::pos2(10., 24.),
                egui::vec2(960., 540.),
            )),
            last_absolute: Some(egui::pos2(100., 100.)),
            ..Default::default()
        };
        playback.host_cursor(true, 180, 152);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            adapter.cursor(ctx, &playback)
        });
        playback.host_cursor(false, 1440, 810);
        let hint = ctx.run(egui::RawInput::default(), |ctx| {
            adapter.cursor(ctx, &playback)
        });
        let commands = &hint.viewport_output[&egui::ViewportId::ROOT].commands;
        assert!(commands.iter().any(|cmd| matches!(cmd,egui::ViewportCommand::CursorPosition(pos) if *pos==egui::pos2(730.,429.))));
        assert!(!commands.iter().any(|cmd| matches!(
            cmd,
            egui::ViewportCommand::CursorGrab(egui::CursorGrab::None)
        )));
        assert!(adapter.grabbed);
        let unlock = ctx.run(egui::RawInput::default(), |ctx| {
            adapter.cursor(ctx, &playback)
        });
        assert!(
            unlock.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .any(|cmd| matches!(
                    cmd,
                    egui::ViewportCommand::CursorGrab(egui::CursorGrab::None)
                ))
        );
        assert!(!adapter.grabbed);
        let mut raw = egui::RawInput::default();
        raw.events = vec![
            Event::PointerMoved(egui::pos2(100., 100.)),
            Event::PointerMoved(egui::pos2(730., 429.)),
        ];
        adapter.events(&ctx, &mut raw, &playback);
        let mut batch = std::collections::VecDeque::new();
        playback.input.take(&mut batch).unwrap();
        assert!(batch.is_empty());
        raw.events = vec![Event::PointerMoved(egui::pos2(735., 430.))];
        adapter.events(&ctx, &mut raw, &playback);
        playback.input.take(&mut batch).unwrap();
        assert_eq!(batch.len(), 1);
        playback.host_cursor(false, 1000, 500);
        let idle = ctx.run(egui::RawInput::default(), |ctx| {
            adapter.cursor(ctx, &playback)
        });
        assert!(
            !idle.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .any(|cmd| matches!(cmd, egui::ViewportCommand::CursorPosition(_)))
        );
    }
}

#[cfg(test)]
mod immersive_tests {
    use super::*;
    #[test]
    fn linux_mouse_immersive_confines_without_inventing_relative_camera_deltas() {
        let ctx = egui::Context::default();
        let playback = Playback::new(ctx.clone());
        playback
            .immersive
            .store(3, std::sync::atomic::Ordering::Release);
        let mut input = DesktopInput::default();
        input.enabled = true;
        let mut raw = egui::RawInput::default();
        raw.focused = true;
        let out = ctx.run(raw, |ctx| input.cursor(ctx, &playback));
        assert!(!input.grabbed);
        assert!(
            out.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .any(|c| matches!(
                    c,
                    egui::ViewportCommand::CursorGrab(egui::CursorGrab::Confined)
                ))
        );
    }
}

#[cfg(test)]
mod peer_cursor_tests {
    use super::*;
    #[test]
    fn linux_drag_endpoint_uses_destination_geometry_without_releasing_host_buttons() {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let displays = std::sync::Arc::new(crate::display_routes::Displays::default());
        let primary = Playback::new(ctx.clone());
        primary.attach_displays(&displays, 0);
        *displays.primary.lock().unwrap() = std::sync::Arc::downgrade(&primary);
        displays.open(1, &ctx).unwrap();
        let peer = displays.get(1).unwrap().playback.clone();
        for target in [&primary, &peer] {
            target.set_input_ready(true);
            target.set_input_geometry((1920, 1080));
        }
        primary.set_video_rect(Some(Rect::from_min_size(
            egui::pos2(0., 24.),
            egui::vec2(1920., 1080.),
        )));
        peer.set_video_rect(Some(Rect::from_min_size(
            egui::pos2(10., 24.),
            egui::vec2(960., 540.),
        )));
        let mut adapter = DesktopInput {
            enabled: true,
            buttons: 1,
            video: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1920., 1080.))),
            ..Default::default()
        };
        primary.host_cursor(true, 20, 30);
        let _ = ctx.run(Default::default(), |ctx| adapter.cursor(ctx, &primary));
        primary.release_peer_cursor(1, 1440, 810);
        let unlock = ctx.run(Default::default(), |ctx| adapter.cursor(ctx, &primary));
        assert!(
            unlock.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .any(|cmd| matches!(
                    cmd,
                    egui::ViewportCommand::CursorGrab(egui::CursorGrab::None)
                ))
        );
        let out = ctx.run(Default::default(), |ctx| {
            ctx.show_viewport_deferred(
                crate::display_routes::viewport(1),
                Default::default(),
                |_, _| {},
            );
            adapter.cursor(ctx, &primary);
        });
        assert!(out.viewport_output[&crate::display_routes::viewport(1)].commands.iter().any(|cmd| matches!(cmd,egui::ViewportCommand::CursorPosition(p) if *p==egui::pos2(730.,429.))));
        assert!(
            !out.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .any(|cmd| matches!(cmd, egui::ViewportCommand::CursorPosition(_)))
        );
        assert_eq!(adapter.buttons, 1);
        let mut input = std::collections::VecDeque::new();
        primary.input.take(&mut input).unwrap();
        assert!(input.is_empty());
        primary.release_peer_cursor(1, 100, 100);
        assert!(primary.take_cursor_restore().is_none());
        peer.host_cursor(true, 20, 30);
        peer.release_peer_cursor(0, 1440, 810);
        assert_eq!(
            peer.cursor_endpoint(0, 1440, 810),
            Some(egui::pos2(1440., 834.))
        );
        displays.close(1);
        assert!(primary.cursor_endpoint(1, 20, 30).is_none());
    }
}
