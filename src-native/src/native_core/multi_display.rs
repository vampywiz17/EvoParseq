//! Additional Windows display windows on the existing connection. No audio,
//! signaling, GPU interop, frame composition or duplicate stream decoders.
use super::{input::Input, pointer::Pointer, session_model::Event, transport::Attempt};
use crate::{connection_settings::Manager, window::Window};
use anyhow::{ensure, Result};
use std::sync::{Arc, Mutex};

use crate::display_stream::{enable, STREAMS};

pub unsafe fn add_menu(window: &Window) {
    use windows_sys::Win32::{Foundation::POINT, UI::WindowsAndMessaging::*};
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return;
    }
    {
        let displays = window
            .session
            .displays
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for (stream, output) in displays.outputs.iter().enumerate().take(STREAMS).skip(1) {
            let disabled = window.displays.get(stream).is_some()
                || displays.current.as_deref() == Some(&output.id);
            AppendMenuW(
                menu,
                MF_STRING | if disabled { MF_GRAYED } else { 0 },
                stream,
                super::ui::wide(&output.name).as_ptr(),
            );
        }
    }
    let mut position = POINT { x: 0, y: 0 };
    GetCursorPos(&mut position);
    let selected = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_NONOTIFY,
        position.x,
        position.y,
        0,
        window.handle(),
        std::ptr::null(),
    );
    DestroyMenu(menu);
    if (1..STREAMS as i32).contains(&selected) {
        window.events.push(Event::NativeSession(
            crate::session_controls::Command::AddDisplay(selected as usize),
        ));
    }
}

pub struct Screen {
    pub window: Arc<Window>,
    pub video: Arc<crate::video_windows::Pipeline>,
    pub inspector: Arc<Mutex<crate::video_stream::Inspector>>,
    ingress: Mutex<crate::bud_ingress::VideoState>,
    input: Mutex<Input>,
    batch: Mutex<std::collections::VecDeque<Event>>,
    pointer: Mutex<Pointer>,
    flags: std::sync::atomic::AtomicI32,
    opened: std::time::Instant,
    failed: std::sync::atomic::AtomicBool,
}

#[derive(Default)]
pub struct Displays {
    screens: Mutex<[Option<Arc<Screen>>; STREAMS - 1]>,
    primary: Mutex<std::sync::Weak<Window>>,
    focus: std::sync::atomic::AtomicUsize,
    visible_cursor_stream: std::sync::atomic::AtomicUsize,
    pub flags: std::sync::atomic::AtomicI32,
}

impl Displays {
    pub fn set_primary(&self, window: std::sync::Weak<Window>) {
        *self.primary.lock().unwrap_or_else(|e| e.into_inner()) = window;
    }
    pub fn owns_input(&self) -> bool {
        let screens = self
            .screens
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        screens
            .iter()
            .flatten()
            .any(|s| s.window.native_input_allowed())
            || (screens.iter().any(Option::is_some)
                && self
                    .primary
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .upgrade()
                    .is_some_and(|w| w.native_input_allowed()))
    }
    pub(super) fn synchronize_input(
        &self,
        attempt: &Attempt,
        primary_input: &mut Input,
    ) -> Result<()> {
        use std::sync::atomic::Ordering;
        let screens = self
            .screens
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if screens.iter().all(Option::is_none) {
            self.focus.store(0, Ordering::Release);
            return Ok(());
        }
        let primary = self
            .primary
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .upgrade();
        let owner = if primary.is_some_and(|w| w.native_input_allowed()) {
            1
        } else {
            screens
                .iter()
                .position(|s| s.as_ref().is_some_and(|s| s.window.native_input_allowed()))
                .map_or(0, |i| i + 2)
        };
        if self.focus.swap(owner, Ordering::AcqRel) != owner {
            // Release once before processing any window's keys. A late focus-loss
            // event from the old window must not release the new window's keys.
            attempt.send(&crate::control::header(24, 0, 0, 0))?;
            *primary_input = Default::default();
            for screen in screens.iter().flatten() {
                *screen.input.lock().unwrap_or_else(|e| e.into_inner()) = Default::default();
            }
        }
        Ok(())
    }

    pub fn get(&self, stream: usize) -> Option<Arc<Screen>> {
        self.screens
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(stream.checked_sub(1)?)?
            .clone()
    }

    pub(super) fn open(
        &self,
        stream: usize,
        attempt: &Attempt,
        manager: Arc<Manager>,
    ) -> Result<()> {
        ensure!(
            (1..STREAMS).contains(&stream),
            "Invalid additional display stream"
        );
        ensure!(
            !attempt.hid_mouse_enabled()
                && !(attempt.name() == "BUD" && manager.view().0.hid_keyboard_enabled()),
            "Additional displays require normal mouse input"
        );
        ensure!(self.get(stream).is_none(), "Display is already open");
        let window = Window::create_native()?;
        window
            .display_stream
            .store(stream, std::sync::atomic::Ordering::Release);
        window.install_connection_settings(manager.clone());
        window.events.set_consumer(std::thread::current());
        window.set_input_state(crate::input_state::State {
            mode: manager.view().0.native_core.video.mode(),
            ..Default::default()
        });
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW(
                window.handle(),
                super::ui::wide(&format!("EvoParseq - Display {}", stream + 1)).as_ptr(),
            );
        }
        let video = Arc::new(crate::video_windows::Pipeline::start(
            window.clone(),
            if attempt.name() == "BUD" {
                crate::video_output::DisplayCatchUpPolicy::Fixed
            } else {
                crate::video_output::DisplayCatchUpPolicy::Adaptive
            },
        ));
        let mut inspector = crate::video_stream::Inspector::default();
        inspector.configure(crate::video_stream::VideoProtocol {
            version: 1,
            message_size: 11,
            version_offset: 0,
            flag_offset: 4,
        });
        let flags = self.flags.load(std::sync::atomic::Ordering::Acquire);
        inspector.enable_hevc(flags & 1 != 0);
        let screen = Arc::new(Screen {
            window,
            video,
            inspector: Arc::new(Mutex::new(inspector)),
            ingress: Mutex::new(crate::bud_ingress::VideoState::new(
                flags & 1 != 0,
                flags & 0x10 != 0,
                flags & 2 != 0,
            )),
            input: Default::default(),
            batch: Default::default(),
            pointer: Default::default(),
            flags: std::sync::atomic::AtomicI32::new(
                self.flags.load(std::sync::atomic::Ordering::Acquire),
            ),
            opened: std::time::Instant::now(),
            failed: std::sync::atomic::AtomicBool::new(false),
        });
        self.screens.lock().unwrap_or_else(|e| e.into_inner())[stream - 1] = Some(screen);
        self.changed();
        let result = (|| {
            attempt.send(&crate::control::header(
                13,
                stream as i32,
                self.flags.load(std::sync::atomic::Ordering::Acquire),
                0,
            ))?;
            attempt.send(&enable(stream, true)?)
        })();
        if result.is_err() {
            self.close(stream);
        }
        result
    }

    fn changed(&self) {
        if let Some(primary) = self
            .primary
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .upgrade()
        {
            super::session_model::View::notify(
                primary.as_ref(),
                super::session_model::Notification::DisplaysChanged,
            );
        }
    }

    pub fn close(&self, stream: usize) {
        let screen = stream.checked_sub(1).and_then(|i| {
            self.screens
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_mut(i)
                .and_then(Option::take)
        });
        if let Some(screen) = screen {
            screen.video.stop();
            screen.window.end_session();
            screen.window.request_stop();
            screen.window.close();
            self.changed();
        }
    }

    pub fn clear(&self) {
        self.visible_cursor_stream
            .store(0, std::sync::atomic::Ordering::Release);
        for stream in 1..STREAMS {
            self.close(stream);
        }
    }

    pub fn cursor_mode(
        &self,
        stream: usize,
        relative: bool,
        hidden: bool,
        suppressed: bool,
        position: (i32, i32),
    ) {
        use std::sync::atomic::Ordering;
        if stream >= STREAMS {
            return;
        }
        if relative || hidden || suppressed {
            self.visible_cursor_stream.store(0, Ordering::Release);
            return;
        }
        // Reconcile only a mode/stream transition, not every pointer movement.
        if self.visible_cursor_stream.load(Ordering::Acquire) == stream + 1 {
            return;
        }
        let screens = self
            .screens
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        // Ignore closed/unknown outputs and keep the single-display contract.
        if screens.iter().all(Option::is_none) || (stream != 0 && screens[stream - 1].is_none()) {
            return;
        }
        let primary = self
            .primary
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .upgrade();
        let target = if stream == 0 {
            primary.clone()
        } else {
            screens[stream - 1].as_ref().map(|s| s.window.clone())
        };
        let Some(target) = target else {
            return;
        };
        self.visible_cursor_stream
            .store(stream + 1, Ordering::Release);
        if let Some(primary) = primary {
            primary.release_peer_cursor(&target, position);
        }
        for screen in screens.iter().flatten() {
            screen.window.release_peer_cursor(&target, position);
        }
    }
    pub fn pointer(&self, stream: usize, bytes: &[u8]) -> Result<()> {
        if let Some(screen) = self.get(stream) {
            screen
                .pointer
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .receive(bytes, &screen.window)?;
        }
        Ok(())
    }

    pub fn pump_bud(&self, source: &mut evoparseq_bud::bud::MediaSource) -> Result<()> {
        for stream in 1..STREAMS {
            let screen = self.get(stream);
            for _ in 0..16 {
                if source
                    .with_secondary_video(stream, |bytes| {
                        if let Some(screen) = &screen {
                            if screen
                                .ingress
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .receive(bytes, screen.video.as_ref())
                                .is_err()
                            {
                                screen
                                    .failed
                                    .store(true, std::sync::atomic::Ordering::Release);
                            }
                        }
                        Ok(())
                    })?
                    .is_none()
                {
                    break;
                }
            }
        }
        Ok(())
    }

    pub(super) fn poll(&self, attempt: &Attempt) -> Result<()> {
        for stream in 1..STREAMS {
            let Some(screen) = self.get(stream) else {
                continue;
            };
            let mut events = screen.batch.lock().unwrap_or_else(|e| e.into_inner());
            screen.window.events.take_batch(&mut events);
            let snapshot = screen.video.snapshot();
            let mut closing = screen.failed.load(std::sync::atomic::Ordering::Acquire)
                || screen.window.stop.stopped()
                || snapshot.failure_stage.is_some();
            if screen.opened.elapsed() > std::time::Duration::from_secs(10)
                && snapshot.frames_decoded == 0
            {
                closing = true;
            }
            for event in events.drain(..) {
                match event {
                    Event::NativeSession(crate::session_controls::Command::Apply(video)) => {
                        if let Some(primary) = self
                            .primary
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .upgrade()
                        {
                            primary.events.push(Event::NativeSession(
                                crate::session_controls::Command::DisplayResolution {
                                    stream,
                                    resolution: video.resolution,
                                    size: screen.window.native_display_size(),
                                },
                            ));
                        }
                    }
                    Event::NativeSession(crate::session_controls::Command::Disconnect)
                    | Event::Shortcut(crate::input::Shortcut::Disconnect) => closing = true,
                    Event::Shortcut(crate::input::Shortcut::Fullscreen) => screen
                        .window
                        .set_fullscreen(!screen.window.native_fullscreen()),
                    Event::Shortcut(crate::input::Shortcut::ReleaseInput) => {
                        attempt.send(&crate::control::header(24, 0, 0, 0))?;
                        *screen.input.lock().unwrap_or_else(|e| e.into_inner()) =
                            Default::default();
                        screen.window.toggle_input_detached();
                        screen.window.native_toggle_overlay();
                    }
                    Event::NativeSession(_) | Event::Shortcut(_) | Event::Clipboard => {}
                    Event::Focus(false) if self.owns_input() => {
                        *screen.input.lock().unwrap_or_else(|e| e.into_inner()) =
                            Default::default();
                    }
                    Event::InputOverflow => {
                        closing = true;
                    }
                    _ => screen
                        .input
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .send(attempt, &screen.window, &event)?,
                }
            }
            if let Some((_, fallback)) = screen.video.take_keyframe_request() {
                screen
                    .ingress
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .wait_for_keyframe();
                if fallback {
                    screen
                        .flags
                        .fetch_and(!2, std::sync::atomic::Ordering::AcqRel);
                }
                let flags = screen.flags.load(std::sync::atomic::Ordering::Acquire);
                attempt.send(&crate::control::header(13, stream as i32, flags, 1))?;
            }
            if closing {
                crate::event_log::record(
                    "INFO",
                    "additional_display_ended",
                    serde_json::json!({"stream":stream,"decoded":snapshot.frames_decoded}),
                );
                if self.focus.load(std::sync::atomic::Ordering::Acquire) == stream + 1 {
                    let _ = screen.input.lock().unwrap_or_else(|e| e.into_inner()).send(
                        attempt,
                        &screen.window,
                        &Event::Focus(false),
                    );
                }
                attempt.send(&enable(stream, false)?)?;
                self.close(stream);
            }
        }
        Ok(())
    }
}

impl Drop for Displays {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    #[ignore = "native window fixture; run alone after the other foreground tests"]
    fn additional_display_native_lifecycle() {
        use crate::{input::Shortcut, session_controls::Command, session_preferences::Resolution};
        let primary = Window::create_native().unwrap();
        struct Close(Arc<Window>);
        impl Drop for Close {
            fn drop(&mut self) {
                self.0.displays.clear();
                self.0.close();
            }
        }
        let _close = Close(primary.clone());
        // A unique, nonexistent profile: this fixture never reads user settings.
        let manager = Manager::open(std::env::temp_dir().join(format!(
            "evoparseq-display-{}-{}.json", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        )));
        let displays = &primary.displays;
        let (attempt, packets) = Attempt::recorded_commands();
        displays.flags.store(9, Ordering::Release);
        for stream in [0, STREAMS, usize::MAX] {
            assert!(displays.open(stream, &attempt, manager.clone()).is_err());
            assert!(displays.get(stream).is_none());
            displays.close(stream);
        }
        for stream in 1..STREAMS {
            displays.open(stream, &attempt, manager.clone()).unwrap();
            assert_eq!(
                packets(),
                vec![
                    crate::control::header(13, stream as i32, 9, 0),
                    enable(stream, true).unwrap()
                ]
            );
            assert!(displays.open(stream, &attempt, manager.clone()).is_err());
        }
        let second = displays.get(1).unwrap();
        let third = displays.get(2).unwrap();
        assert!(!Arc::ptr_eq(&second.video, &third.video));
        assert_eq!(second.window.display_stream.load(Ordering::Acquire), 1);
        assert_eq!(third.window.display_stream.load(Ordering::Acquire), 2);
        assert_eq!(second.video.snapshot().frames_decoded, 0);
        assert_eq!(third.video.snapshot().frames_decoded, 0);

        let mut input = Input::default();
        displays.synchronize_input(&attempt, &mut input).unwrap();
        assert!(packets().is_empty());
        displays.focus.store(3, Ordering::Release);
        displays.synchronize_input(&attempt, &mut input).unwrap();
        assert_eq!(packets(), vec![crate::control::header(24, 0, 0, 0)]);
        displays.synchronize_input(&attempt, &mut input).unwrap();
        assert!(packets().is_empty());

        // Visible endpoints reconcile only live display transitions. Hidden,
        // suppressed and relative reports must not make an endpoint visible.
        for stream in [0, 1, 2] {
            displays.cursor_mode(stream, false, false, false, (20, 30));
            assert_eq!(
                displays.visible_cursor_stream.load(Ordering::Acquire),
                stream + 1
            );
            displays.cursor_mode(stream, false, false, false, (40, 50));
        }
        for mode in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            displays.cursor_mode(1, mode.0, mode.1, mode.2, (0, 0));
            assert_eq!(displays.visible_cursor_stream.load(Ordering::Acquire), 0);
        }
        displays.cursor_mode(STREAMS, false, false, false, (0, 0));
        assert!(displays.pointer(1, &[]).is_err());
        assert!(displays.pointer(0, &[]).is_ok());

        // Resolution requests return to the one primary host-settings owner;
        // secondary windows cannot expose statistics or change other settings.
        let resolution = Resolution::Fixed(1280, 720);
        let mut primary_events = std::collections::VecDeque::new();
        primary.events.take_batch(&mut primary_events);
        primary_events.clear();
        second
            .window
            .events
            .push(Event::NativeSession(Command::Apply(
                crate::session_preferences::Video {
                    resolution,
                    ..Default::default()
                },
            )));
        for event in [
            Event::Clipboard,
            Event::Shortcut(Shortcut::Statistics),
            Event::NativeSession(Command::SecureAttention),
            Event::CursorReady,
        ] {
            second.window.events.push(event);
        }
        displays.poll(&attempt).unwrap();
        primary.events.take_batch(&mut primary_events);
        assert!(primary_events.iter().any(|event| matches!(event,
            Event::NativeSession(Command::DisplayResolution { stream: 1, resolution: r, size })
                if *r == resolution && *size == second.window.native_display_size())));
        assert!(packets().is_empty());
        second
            .window
            .events
            .push(Event::Shortcut(Shortcut::ReleaseInput));
        displays.poll(&attempt).unwrap();
        assert_eq!(packets(), vec![crate::control::header(24, 0, 0, 0)]);

        // An overflow closes only its additional display, using stream-specific
        // control. The other worker and primary window remain alive.
        second.window.events.push(Event::InputOverflow);
        displays.poll(&attempt).unwrap();
        assert_eq!(packets(), vec![enable(1, false).unwrap()]);
        assert!(displays.get(1).is_none());
        assert!(displays.get(2).is_some());
        assert!(!primary.stop.stopped());
        third
            .window
            .events
            .push(Event::NativeSession(Command::Disconnect));
        displays.poll(&attempt).unwrap();
        assert_eq!(packets(), vec![enable(2, false).unwrap()]);
        assert!(displays.get(2).is_none());
        displays.synchronize_input(&attempt, &mut input).unwrap();
        assert_eq!(displays.focus.load(Ordering::Acquire), 0);
        displays.clear();
        assert_eq!(displays.visible_cursor_stream.load(Ordering::Acquire), 0);
        assert!(!primary.stop.stopped());

        // A closed command receiver must roll back a partially opened window.
        // Wait only for these empty decoder workers to retire before reusing slots.
        for screen in [&second, &third] {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while !screen.video.snapshot().worker_finished {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        let (closed, receiver) = Attempt::recorded_commands();
        drop(receiver);
        assert!(displays.open(1, &closed, manager).is_err());
        assert!(displays.get(1).is_none());
        assert!(!primary.stop.stopped());
    }
}
