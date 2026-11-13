//! Supported eframe external-event-loop adapter. Only physical key translation
//! lives here; egui/winit continue to own windowing, pointers and rendering.
use crate::{desktop_input::Event as Input, playback::Playback};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU32, Ordering},
};
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, ElementState, StartCause, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::{KeyCode, ModifiersState, PhysicalKey},
    window::WindowId,
};

// The external event-loop API reports renderer/window initialization errors via
// log, while run_app itself can return Ok. Forward only framework errors; never
// enable transport/authentication logging or persist raw messages in JSON logs.
// Vulkan adapter errors explain failed surface queries before eframe's summary.
struct FrameworkErrors;
impl log::Log for FrameworkErrors {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() == log::Level::Error
            && (metadata.target().starts_with("eframe::")
                || metadata.target() == "wgpu_hal::vulkan::adapter")
    }
    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!("{}: {}", record.target(), record.args());
            crate::event_log::record(
                "ERROR",
                "desktop_framework_failed",
                serde_json::json!({"component": if record.target().starts_with("eframe::") {
                    "eframe"
                } else {
                    "vulkan_surface"
                }}),
            );
        }
    }
    fn flush(&self) {}
}
pub fn report_framework_errors() {
    static LOGGER: FrameworkErrors = FrameworkErrors;
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Error);
    }
}

#[derive(Default)]
pub struct KeyboardBridge {
    target: Mutex<Option<Arc<Playback>>>,
    session: Mutex<Option<std::sync::Weak<Playback>>>,
    released: AtomicBool,
    pub refresh_millihertz: AtomicU32,
    paste: Mutex<Option<std::time::Instant>>,
}
impl KeyboardBridge {
    pub fn session(&self, playback: &Arc<Playback>) {
        *self.session.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::downgrade(playback));
    }
    pub fn bind(&self, playback: Option<Arc<Playback>>) {
        *self.target.lock().unwrap_or_else(|e| e.into_inner()) = playback;
    }
    pub fn unbind(&self, playback: &Playback) {
        let mut target = self.target.lock().unwrap_or_else(|e| e.into_inner());
        if target
            .as_ref()
            .is_some_and(|p| std::ptr::eq(p.as_ref(), playback))
        {
            *target = None;
        }
    }
    pub fn take_release(&self) -> bool {
        self.released.swap(false, Ordering::AcqRel)
    }
    fn target(&self) -> Option<Arc<Playback>> {
        self.target
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .filter(|p| !p.stopped())
    }
    pub fn take_paste(&self) -> bool {
        self.paste
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(1))
    }
    pub fn release(&self) {
        *self.paste.lock().unwrap_or_else(|e| e.into_inner()) = None;
        if let Some(playback) = self
            .session
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
        {
            playback.controller_focus(false);
            playback.clipboard_allowed.store(false, Ordering::Release);
        }
        if let Some(playback) = self.target() {
            playback.clipboard_allowed.store(false, Ordering::Release);
            playback.controller_focus(false);
            playback.push_input(Input::Focus(false));
            playback.repaint();
        }
        self.bind(None);
        self.released.store(true, Ordering::Release);
    }
}
pub struct Application<'a> {
    inner: eframe::EframeWinitApplication<'a>,
    pub keyboard: Arc<KeyboardBridge>,
    modifiers: ModifiersState,
    root: Option<WindowId>,
    pasting: bool,
}
impl<'a> Application<'a> {
    pub fn new(inner: eframe::EframeWinitApplication<'a>, keyboard: Arc<KeyboardBridge>) -> Self {
        Self {
            inner,
            keyboard,
            modifiers: ModifiersState::empty(),
            root: None,
            pasting: false,
        }
    }
    fn display(&self, event_loop: &ActiveEventLoop) {
        // Winit exposes cached compositor monitor data. Without a public
        // eframe root-window accessor, only one connected output is unambiguous.
        let mut monitors = event_loop.available_monitors();
        let first = monitors.next();
        let hz = if monitors.next().is_none() {
            first.and_then(|m| m.refresh_rate_millihertz())
        } else {
            None
        };
        self.keyboard
            .refresh_millihertz
            .store(hz.unwrap_or(0), Ordering::Relaxed);
    }
    fn key(&mut self, key: PhysicalKey, down: bool) -> bool {
        let Some(playback) = self.keyboard.target() else {
            return false;
        };
        let PhysicalKey::Code(code) = key else {
            return true;
        };
        if code == KeyCode::KeyM
            && down
            && self.modifiers.control_key()
            && self.modifiers.shift_key()
        {
            self.keyboard.release();
            return true;
        }
        if code == KeyCode::KeyV {
            if down
                && self.modifiers.control_key()
                && !self.modifiers.alt_key()
                && !self.modifiers.super_key()
            {
                self.pasting = true;
                *self
                    .keyboard
                    .paste
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(std::time::Instant::now());
                playback.clipboard_read();
                // Public RequestPaste uses the existing clipboard adapter and
                // is independent of the local keyboard layout. Forward Ctrl+V
                // only after its text arrives through the existing UI handoff.
                return true;
            }
            if !down && self.pasting {
                self.pasting = false;
                return true;
            }
        }
        if let Some(usage) = usage(code) {
            let mods = if (224..=231).contains(&usage) {
                0
            } else {
                i32::from(self.modifiers.shift_key())
                    | (i32::from(self.modifiers.control_key()) << 1)
                    | (i32::from(self.modifiers.alt_key()) << 2)
                    | (i32::from(self.modifiers.super_key()) << 3)
            };
            playback.push_input(Input::Key { usage, mods, down });
        }
        true
    }
}
impl ApplicationHandler<eframe::UserEvent> for Application<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.inner.resumed(event_loop);
        self.display(event_loop);
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, window: WindowId, event: WindowEvent) {
        let root = *self.root.get_or_insert(window);
        if window == root {
            if matches!(
                &event,
                WindowEvent::Moved(_)
                    | WindowEvent::Resized(_)
                    | WindowEvent::ScaleFactorChanged { .. }
            ) {
                self.display(event_loop);
            }
        }
        match &event {
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::Focused(false) | WindowEvent::CloseRequested => self.keyboard.release(),
            WindowEvent::KeyboardInput { event, .. }
                if self.key(event.physical_key, event.state == ElementState::Pressed) =>
            {
                return;
            }
            _ => {}
        }
        self.inner.window_event(event_loop, window, event);
    }
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        self.inner.new_events(event_loop, cause);
    }
    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: eframe::UserEvent) {
        self.inner.user_event(event_loop, event);
    }
    fn device_event(&mut self, event_loop: &ActiveEventLoop, device: DeviceId, event: DeviceEvent) {
        self.inner.device_event(event_loop, device, event);
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.inner.about_to_wait(event_loop);
    }
    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.keyboard.release();
        self.inner.suspended(event_loop);
    }
    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        self.keyboard.release();
        self.inner.exiting(event_loop);
    }
    fn memory_warning(&mut self, event_loop: &ActiveEventLoop) {
        self.inner.memory_warning(event_loop);
    }
}
fn usage(code: KeyCode) -> Option<i32> {
    let name = match code {
        KeyCode::KeyA => "KeyA",
        KeyCode::KeyB => "KeyB",
        KeyCode::KeyC => "KeyC",
        KeyCode::KeyD => "KeyD",
        KeyCode::KeyE => "KeyE",
        KeyCode::KeyF => "KeyF",
        KeyCode::KeyG => "KeyG",
        KeyCode::KeyH => "KeyH",
        KeyCode::KeyI => "KeyI",
        KeyCode::KeyJ => "KeyJ",
        KeyCode::KeyK => "KeyK",
        KeyCode::KeyL => "KeyL",
        KeyCode::KeyM => "KeyM",
        KeyCode::KeyN => "KeyN",
        KeyCode::KeyO => "KeyO",
        KeyCode::KeyP => "KeyP",
        KeyCode::KeyQ => "KeyQ",
        KeyCode::KeyR => "KeyR",
        KeyCode::KeyS => "KeyS",
        KeyCode::KeyT => "KeyT",
        KeyCode::KeyU => "KeyU",
        KeyCode::KeyV => "KeyV",
        KeyCode::KeyW => "KeyW",
        KeyCode::KeyX => "KeyX",
        KeyCode::KeyY => "KeyY",
        KeyCode::KeyZ => "KeyZ",
        KeyCode::Digit0 => "Digit0",
        KeyCode::Digit1 => "Digit1",
        KeyCode::Digit2 => "Digit2",
        KeyCode::Digit3 => "Digit3",
        KeyCode::Digit4 => "Digit4",
        KeyCode::Digit5 => "Digit5",
        KeyCode::Digit6 => "Digit6",
        KeyCode::Digit7 => "Digit7",
        KeyCode::Digit8 => "Digit8",
        KeyCode::Digit9 => "Digit9",
        KeyCode::F1 => "F1",
        KeyCode::F2 => "F2",
        KeyCode::F3 => "F3",
        KeyCode::F4 => "F4",
        KeyCode::F5 => "F5",
        KeyCode::F6 => "F6",
        KeyCode::F7 => "F7",
        KeyCode::F8 => "F8",
        KeyCode::F9 => "F9",
        KeyCode::F10 => "F10",
        KeyCode::F11 => "F11",
        KeyCode::F12 => "F12",
        KeyCode::F13 => "F13",
        KeyCode::F14 => "F14",
        KeyCode::F15 => "F15",
        KeyCode::F16 => "F16",
        KeyCode::F17 => "F17",
        KeyCode::F18 => "F18",
        KeyCode::F19 => "F19",
        KeyCode::F20 => "F20",
        KeyCode::F21 => "F21",
        KeyCode::F22 => "F22",
        KeyCode::F23 => "F23",
        KeyCode::F24 => "F24",
        KeyCode::Enter => "Enter",
        KeyCode::Escape => "Escape",
        KeyCode::Backspace => "Backspace",
        KeyCode::Tab => "Tab",
        KeyCode::Space => "Space",
        KeyCode::Minus => "Minus",
        KeyCode::Equal => "Equal",
        KeyCode::BracketLeft => "BracketLeft",
        KeyCode::BracketRight => "BracketRight",
        KeyCode::Backslash => "Backslash",
        KeyCode::Semicolon => "Semicolon",
        KeyCode::Quote => "Quote",
        KeyCode::Backquote => "Backquote",
        KeyCode::Comma => "Comma",
        KeyCode::Period => "Period",
        KeyCode::Slash => "Slash",
        KeyCode::CapsLock => "CapsLock",
        KeyCode::PrintScreen => "PrintScreen",
        KeyCode::ScrollLock => "ScrollLock",
        KeyCode::Pause => "Pause",
        KeyCode::Insert => "Insert",
        KeyCode::Home => "Home",
        KeyCode::PageUp => "PageUp",
        KeyCode::Delete => "Delete",
        KeyCode::End => "End",
        KeyCode::PageDown => "PageDown",
        KeyCode::ArrowRight => "ArrowRight",
        KeyCode::ArrowLeft => "ArrowLeft",
        KeyCode::ArrowDown => "ArrowDown",
        KeyCode::ArrowUp => "ArrowUp",
        KeyCode::NumLock => "NumLock",
        KeyCode::NumpadDivide => "NumpadDivide",
        KeyCode::NumpadMultiply => "NumpadMultiply",
        KeyCode::NumpadSubtract => "NumpadSubtract",
        KeyCode::NumpadAdd => "NumpadAdd",
        KeyCode::NumpadEnter => "NumpadEnter",
        KeyCode::Numpad1 => "Numpad1",
        KeyCode::Numpad2 => "Numpad2",
        KeyCode::Numpad3 => "Numpad3",
        KeyCode::Numpad4 => "Numpad4",
        KeyCode::Numpad5 => "Numpad5",
        KeyCode::Numpad6 => "Numpad6",
        KeyCode::Numpad7 => "Numpad7",
        KeyCode::Numpad8 => "Numpad8",
        KeyCode::Numpad9 => "Numpad9",
        KeyCode::Numpad0 => "Numpad0",
        KeyCode::NumpadDecimal => "NumpadDecimal",
        KeyCode::IntlBackslash => "IntlBackslash",
        KeyCode::ContextMenu => "ContextMenu",
        KeyCode::NumpadEqual => "NumpadEqual",
        KeyCode::ControlLeft => "ControlLeft",
        KeyCode::ShiftLeft => "ShiftLeft",
        KeyCode::AltLeft => "AltLeft",
        KeyCode::SuperLeft => "MetaLeft",
        KeyCode::ControlRight => "ControlRight",
        KeyCode::ShiftRight => "ShiftRight",
        KeyCode::AltRight => "AltRight",
        KeyCode::SuperRight => "MetaRight",
        _ => return None,
    };
    crate::desktop_input::usage(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linux_physical_keys_preserve_iso_numpad_and_right_hand_modifiers() {
        for (key, expected) in [
            (KeyCode::IntlBackslash, 100),
            (KeyCode::NumpadEnter, 88),
            (KeyCode::Enter, 40),
            (KeyCode::AltRight, 230),
            (KeyCode::ControlRight, 228),
            (KeyCode::KeyY, 28),
            (KeyCode::KeyZ, 29),
        ] {
            assert_eq!(usage(key), Some(expected));
        }
    }
}

#[cfg(test)]
mod focus_tests {
    #[test]
    fn linux_framework_error_output_excludes_authentication_transport_and_non_errors() {
        use log::Log as _;
        let enabled = |target, level| {
            super::FrameworkErrors
                .enabled(&log::Metadata::builder().target(target).level(level).build())
        };
        assert!(enabled("eframe::native::run", log::Level::Error));
        assert!(enabled("wgpu_hal::vulkan::adapter", log::Level::Error));
        assert!(!enabled("wgpu_hal::vulkan::adapter", log::Level::Info));
        assert!(!enabled(
            "wgpu_hal::vulkan::adapter_extra",
            log::Level::Error
        ));
        assert!(!enabled("eframe::native::run", log::Level::Info));
        assert!(!enabled("webrtc", log::Level::Error));
        assert!(!enabled("reqwest", log::Level::Error));
        assert!(!enabled("account", log::Level::Error));
    }
    #[test]
    fn linux_focus_loss_revokes_controller_and_clipboard_without_mouse_capture() {
        let p = crate::playback::Playback::new(eframe::egui::Context::default());
        let bridge = super::KeyboardBridge::default();
        bridge.session(&p);
        p.controller_focus(true);
        p.clipboard_allowed
            .store(true, std::sync::atomic::Ordering::Release);
        bridge.release();
        assert!(!p.pad_focus.load(std::sync::atomic::Ordering::Acquire));
        assert!(
            !p.clipboard_allowed
                .load(std::sync::atomic::Ordering::Acquire)
        );
    }
}
