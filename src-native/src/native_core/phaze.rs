//! Windows adapter only: independent Rust protocol -> existing media and input backends.
use super::account::Session;
#[cfg(test)]
#[path = "phaze_turn.rs"]
mod turn_research;
use crate::{
    connection_settings::Manager,
    window::{Event, Window},
};
use anyhow::{Context, Result};
use std::{
    collections::VecDeque,
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};

pub(super) fn block_on<T>(future: impl std::future::Future<Output = Result<T>>) -> Result<T> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("Could not initialize Phaze runtime")?
        .block_on(future)
}
struct View {
    window: Arc<Window>,
    video: crate::video_windows::Pipeline,
    generation: u64,
    last_report: Instant,
    inspector: crate::video_stream::Inspector,
    audio: Option<Arc<crate::audio_windows::Playback>>,
    pcm: Vec<i16>,
    keys: evoparseq_phaze::Keyboard,
    events: VecDeque<Event>,
}
impl View {
    fn publish(&self, state: &str) {
        self.window.stats.publish(
            self.generation,
            crate::stats::Sample {
                state: state.into(),
                protocol: "Phaze / direct UDP".into(),
                dtls: "Authenticated DTLS 1.2 (legacy non-EMS)".into(),
                video: Some(self.video.snapshot()),
                codec: Some(crate::video_stream::Codec::H264),
                profile: self.inspector.snapshot().sps_profile_idc,
                route: "Direct",
                ..Default::default()
            },
        );
    }
}
impl evoparseq_phaze::Sink for View {
    fn stage(&mut self, stage: &'static str) {
        crate::event_log::record(
            "INFO",
            "phaze_connection_stage",
            serde_json::json!({"stage":stage}),
        );
        self.window.stats.publish(
            self.generation,
            crate::stats::Sample {
                state: format!("Phaze: {stage}"),
                protocol: "Phaze / direct UDP".into(),
                dtls: if stage == "dtls_authenticated" {
                    "Authenticated DTLS 1.2 (legacy non-EMS)".into()
                } else {
                    "Not reported".into()
                },
                ..Default::default()
            },
        );
    }
    fn outputs(&mut self, outputs: &[evoparseq_phaze::Output]) {
        crate::event_log::record(
            "INFO",
            "phaze_displays_received",
            serde_json::json!({"count":outputs.len()}),
        );
    }
    fn video(&mut self, encoded: &[u8], reset: bool) -> Result<()> {
        if reset {
            self.video.encoder_rebuilt();
        }
        if let Some(info) = self.inspector.receive(encoded) {
            self.video.submit(encoded, info);
        }
        if self.last_report.elapsed() >= Duration::from_millis(500) {
            self.publish("Connected (Phaze, experimental)");
            self.last_report = Instant::now();
        }
        Ok(())
    }
    fn audio(&mut self, bytes: &[u8]) -> Result<()> {
        if self.audio.is_none() {
            let playback = Arc::new(crate::audio_windows::Playback::create_layout(
                self.window.clone(),
                50,
                100,
                crate::audio_layout::Layout::STEREO,
            )?);
            self.window.stats.set_audio(Arc::downgrade(&playback));
            self.audio = Some(playback);
            crate::event_log::record(
                "INFO",
                "phaze_audio_started",
                serde_json::json!({"codec":"PCM s16le","sample_rate":48000,"channels":2}),
            );
        }
        self.pcm.clear();
        self.pcm.extend(
            bytes
                .as_chunks::<2>()
                .0
                .iter()
                .copied()
                .map(i16::from_le_bytes),
        );
        if let Some(playback) = &self.audio {
            playback.queue(&self.pcm);
        }
        Ok(())
    }
    fn poll(
        &mut self,
        output: Option<u32>,
        commands: &mut Vec<evoparseq_phaze::Input>,
    ) -> Result<bool> {
        use evoparseq_phaze::Input;
        if self.window.closing.load(Ordering::Acquire) || self.window.stop.stopped() {
            return Ok(false);
        }
        if let Some(stage) = self
            .window
            .video_report
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|r| r.failure_stage)
        {
            anyhow::bail!("Phaze video failed: {stage}");
        }
        self.window.events.take_batch(&mut self.events);
        while let Some(event) = self.events.pop_front() {
            match event {
                Event::Shortcut(crate::input::Shortcut::Disconnect)
                | Event::NativeSession(super::session_model::Command::Disconnect) => {
                    return Ok(false)
                }
                Event::Shortcut(crate::input::Shortcut::Fullscreen) => {
                    self.window.set_fullscreen(!self.window.native_fullscreen())
                }
                Event::Shortcut(crate::input::Shortcut::ReleaseInput) => {
                    self.keys.reset();
                    commands.push(Input::Release);
                    self.window.toggle_input_detached();
                }
                event => translate(
                    event,
                    &mut self.keys,
                    self.window.presented_viewport(),
                    output.is_some() && self.window.native_input_allowed(),
                    commands,
                )?,
            }
        }
        Ok(true)
    }
}
// Declared before the media view so the decoder worker joins before HWND teardown,
// including error/cancellation paths.
struct SessionWindow(Arc<Window>);
impl Drop for SessionWindow {
    fn drop(&mut self) {
        self.0.end_session();
        self.0.request_stop();
        self.0.close();
    }
}
pub(super) fn run(session: &Session, peer: &str, manager: Arc<Manager>) -> Result<()> {
    anyhow::ensure!(
        session.service() == super::account::Service::Phaze,
        "Phaze requires a Phaze account."
    );
    let window = Window::create_native()?;
    let _lifecycle = SessionWindow(window.clone());
    window.remote_controls.store(false, Ordering::Release);
    window.set_input_state(Default::default());
    window.install_connection_settings(manager.clone());
    window.set_fullscreen(manager.view().0.native_core.video.fullscreen);
    window.events.set_consumer(std::thread::current());
    unsafe {
        let title: Vec<u16> = "EvoParseq — Phaze (experimental)"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW(
            window.handle(),
            title.as_ptr(),
        );
    }
    let video = crate::video_windows::Pipeline::start(
        window.clone(),
        crate::video_output::DisplayCatchUpPolicy::Fixed,
    );
    let generation = window.stats.begin();
    let mut view = View {
        window: window.clone(),
        video,
        generation,
        last_report: Instant::now(),
        inspector: Default::default(),
        audio: None,
        pcm: Vec::new(),
        keys: Default::default(),
        events: VecDeque::new(),
    };
    let remote = evoparseq_phaze::Session::from_token(session.token())?;
    let result = block_on(async {
        let local = async {
            let mut tick = tokio::time::interval(Duration::from_millis(50));
            loop {
                tick.tick().await;
                if window.closing.load(Ordering::Acquire) || window.stop.stopped() {
                    return Ok(());
                }
            }
        };
        tokio::select! {
            result=evoparseq_phaze::connect(&remote,peer,&mut view)=>result,
            result=local=>result,
        }
    });
    if let Err(error) = &result {
        crate::event_log::record(
            "ERROR",
            "phaze_session_failed",
            serde_json::json!({"reason":error.to_string()}),
        );
    }
    result
}

fn normalized(viewport: Option<crate::viewport::Viewport>, x: i32, y: i32) -> Option<(i32, i32)> {
    let viewport = viewport?;
    let [left, top, right, bottom] = viewport.rect()?;
    let axis = |v: i32, start: i32, end: i32| {
        ((i64::from(v) - i64::from(start)) * 65535 / i64::from(end - start - 1).max(1))
            .clamp(0, 65535) as i32
    };
    Some((axis(x, left, right), axis(y, top, bottom)))
}

fn translate(
    event: Event,
    keys: &mut evoparseq_phaze::Keyboard,
    viewport: Option<crate::viewport::Viewport>,
    allowed: bool,
    commands: &mut Vec<evoparseq_phaze::Input>,
) -> Result<()> {
    use evoparseq_phaze::Input;
    match event {
        Event::Focus(false) | Event::HidInputRelease => {
            keys.reset();
            commands.push(Input::Release);
        }
        Event::InputOverflow => anyhow::bail!("Input queue overloaded; reconnect to continue."),
        _ if !allowed => {}
        Event::Key(down, code, compact) => {
            if let Some(command) = crate::desktop_input::usage(code)
                .and_then(|usage| keys.key(usage as u16, compact, down))
            {
                commands.push(command);
            }
        }
        Event::Motion(x, y) => {
            if let Some((x, y)) = normalized(viewport, x, y) {
                commands.push(Input::Motion {
                    x,
                    y,
                    relative: false,
                });
            }
        }
        Event::RelativeMotion(x, y) => commands.push(Input::Motion {
            x,
            y,
            relative: true,
        }),
        Event::Button(down, button, x, y) if (0..=4).contains(&button) => {
            if let Some((nx, ny)) = normalized(viewport, x, y) {
                if down {
                    commands.push(Input::Motion {
                        x: nx,
                        y: ny,
                        relative: false,
                    });
                }
                commands.push(Input::Button {
                    button: [1, 3, 2, 4, 5][button as usize],
                    x,
                    y,
                    down,
                });
            }
        }
        Event::Scroll(x, y) => commands.push(Input::Scroll {
            x,
            y: y.saturating_neg(),
        }),
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use evoparseq_phaze::{Input, Keyboard};
    #[test]
    fn activating_click_positions_before_press_and_focus_loss_releases() -> Result<()> {
        let viewport = Some(crate::viewport::Viewport {
            source: (1920, 1080),
            client: (1024, 768),
        });
        let mut keys = Keyboard::default();
        let mut commands = Vec::new();
        translate(
            Event::Button(true, 2, 512, 384),
            &mut keys,
            viewport,
            true,
            &mut commands,
        )?;
        assert!(matches!(
            commands[0],
            Input::Motion {
                relative: false,
                ..
            }
        ));
        assert!(matches!(
            commands[1],
            Input::Button {
                button: 2,
                down: true,
                ..
            }
        ));
        translate(
            Event::Key(true, "ControlRight", 0),
            &mut keys,
            viewport,
            true,
            &mut commands,
        )?;
        translate(Event::Focus(false), &mut keys, None, false, &mut commands)?;
        assert_eq!(commands.last(), Some(&Input::Release));
        commands.clear();
        translate(
            Event::Key(true, "KeyA", 0),
            &mut keys,
            viewport,
            true,
            &mut commands,
        )?;
        assert!(matches!(commands[0], Input::Key { modifiers: 0, .. }));
        commands.clear();
        translate(
            Event::Button(true, 0, 10, 20),
            &mut keys,
            None,
            true,
            &mut commands,
        )?;
        translate(
            Event::Key(true, "KeyB", 0),
            &mut keys,
            viewport,
            false,
            &mut commands,
        )?;
        assert!(commands.is_empty());
        Ok(())
    }
    #[test]
    fn letterboxed_motion_uses_phaze_normalized_coordinates() {
        let viewport = Some(crate::viewport::Viewport {
            source: (1920, 1080),
            client: (1024, 768),
        });
        assert_eq!(normalized(viewport, 0, 96), Some((0, 0)));
        assert_eq!(normalized(viewport, 1023, 671), Some((65535, 65535)));
        assert_eq!(normalized(None, 100, 100), None);
    }
}
