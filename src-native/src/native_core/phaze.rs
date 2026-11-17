//! Windows adapter only: independent Rust protocol -> existing video pipeline.
use super::account::Session;
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
            self.publish("Connected (Phaze, experimental video only)");
            self.last_report = Instant::now();
        }
        Ok(())
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
    window.set_input_state(crate::input_state::State {
        detached: true,
        ..Default::default()
    });
    window.install_connection_settings(manager.clone());
    window.set_fullscreen(manager.view().0.native_core.video.fullscreen);
    window.events.set_consumer(std::thread::current());
    unsafe {
        let title: Vec<u16> = "EvoParseq — Phaze (experimental video only)"
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
    };
    let remote = evoparseq_phaze::Session::from_token(session.token())?;
    let result = block_on(async {
        let local = async {
            let mut tick = tokio::time::interval(Duration::from_millis(50));
            let mut events = VecDeque::new();
            loop {
                tick.tick().await;
                if window.closing.load(Ordering::Acquire) || window.stop.stopped() {
                    return Ok(());
                }
                if let Some(stage) = window
                    .video_report
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                    .and_then(|r| r.failure_stage)
                {
                    anyhow::bail!("Phaze video failed: {stage}");
                }
                window.events.take_batch(&mut events);
                while let Some(event) = events.pop_front() {
                    match event {
                        Event::Shortcut(crate::input::Shortcut::Disconnect)
                        | Event::NativeSession(super::session_model::Command::Disconnect) => {
                            window.request_stop()
                        }
                        Event::Shortcut(crate::input::Shortcut::Fullscreen) => {
                            window.set_fullscreen(!window.native_fullscreen())
                        }
                        _ => {} // No Parsec-formatted input/control is sent to a Phaze host.
                    }
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
