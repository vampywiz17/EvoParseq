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
    time::Duration,
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
    manager: Arc<Manager>,
    config: evoparseq_phaze::VideoConfig,
    pending: Option<(u32, evoparseq_phaze::VideoConfig)>,
    received: u64,
    sent: u64,
    rtt: Option<Duration>,
    audio_bytes: u64,
    video_payload_bytes: Option<u64>,
    sent_video_config: Option<evoparseq_phaze::VideoConfig>,
    raw_audio: bool,
    audio_codec: Option<evoparseq_phaze::AudioCodec>,
    authenticated: bool,
    video: crate::video_windows::Pipeline,
    generation: u64,
    inspector: crate::video_stream::Inspector,
    audio: Option<Arc<crate::audio_windows::Playback>>,
    keys: evoparseq_phaze::Keyboard,
    events: VecDeque<Event>,
    input_logged: [u8; 4],
    route: &'static str,
    carrier: &'static str,
}
impl View {
    fn publish(&self, state: &str) {
        self.window.stats.publish(
            self.generation,
            crate::stats::Sample {
                phaze: true,
                state: state.into(),
                protocol: format!("Phaze / {}", self.carrier),
                dtls: if self.authenticated {
                    "Authenticated DTLS 1.2 (legacy non-EMS)"
                } else {
                    "Not reported"
                }
                .into(),
                video: Some(self.video.snapshot()),
                codec: self.inspector.snapshot().codec,
                hevc_advertised: self.config.hevc,
                received: self.authenticated.then_some(self.received),
                sent: self.authenticated.then_some(self.sent),
                rtt_ms: self.rtt.map(|elapsed| elapsed.as_secs_f64() * 1000.0),
                audio_bytes: self.audio_bytes,
                video_payload_bytes: self.video_payload_bytes,
                phaze_video_request: self
                    .sent_video_config
                    .map(|config| (config.mbps, config.fps)),
                audio_ready: self.audio.is_some(),
                audio_codec: self.audio_codec.map(|codec| codec.label()),
                audio_layout: self
                    .audio
                    .as_ref()
                    .map(|_| crate::audio_layout::Layout::STEREO),
                profile: self.inspector.snapshot().sps_profile_idc,
                route: if self.authenticated {
                    self.route
                } else {
                    "Unknown"
                },
                turn_protocol: (self.authenticated && self.route == "TURN")
                    .then(|| self.carrier.trim_start_matches("TURN / ").to_owned()),
                ..Default::default()
            },
        );
    }
}
impl evoparseq_phaze::Sink for View {
    fn raw_audio(&self) -> bool {
        self.raw_audio
    }
    fn video_config(&self) -> evoparseq_phaze::VideoConfig {
        self.config
    }
    fn stream_request(&mut self) -> Option<(u32, evoparseq_phaze::VideoConfig)> {
        self.pending.take()
    }
    fn traffic(&mut self, received: u64, sent: u64) {
        self.received = received;
        self.sent = sent;
        self.publish("connected");
    }
    fn rtt(&mut self, elapsed: Option<Duration>) {
        self.rtt = elapsed;
    }
    fn stream_request_sent(&mut self, output: u32, config: evoparseq_phaze::VideoConfig) {
        use super::session_model::{Notification, Status, View as _};
        self.sent_video_config = Some(config);
        crate::event_log::record(
            "INFO",
            "phaze_stream_request_sent",
            serde_json::json!({"output":output,"maximum_mbps":config.mbps,"fps":config.fps}),
        );
        self.window
            .session
            .displays
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .current = Some(output.to_string());
        self.window.notify(Notification::DisplaySelected);
        self.window
            .notify(Notification::Status(Status::RequestSent));
    }
    fn stage(&mut self, stage: &'static str) {
        if stage == "dtls_authenticated" {
            self.authenticated = true;
            crate::event_log::record(
                "INFO",
                "phaze_route_selected",
                serde_json::json!({"route":self.route,"carrier":self.carrier,"authenticated":true}),
            );
        }
        crate::event_log::record(
            "INFO",
            "phaze_connection_stage",
            serde_json::json!({"stage":stage}),
        );
        self.publish(&format!("Phaze: {stage}"));
    }
    fn outputs(&mut self, outputs: &[evoparseq_phaze::Output]) {
        use super::session_model::{Notification, View as _};
        {
            let mut displays = self
                .window
                .session
                .displays
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            displays.outputs = outputs
                .iter()
                .filter(|o| o.active)
                .map(|o| super::host_displays::Output {
                    id: o.id.to_string(),
                    name: o.name.clone(),
                    adapter: String::new(),
                })
                .collect();
            displays.loading = false;
        }
        self.window.notify(Notification::DisplaysChanged);
        crate::event_log::record(
            "INFO",
            "phaze_displays_received",
            serde_json::json!({"count":outputs.len()}),
        );
    }
    fn video(&mut self, encoded: &[u8], reset: bool) -> Result<()> {
        if !encoded.is_empty() {
            let bytes = self.video_payload_bytes.get_or_insert(0);
            *bytes = bytes.saturating_add(encoded.len() as u64);
        }
        if reset {
            self.video.encoder_rebuilt();
            self.inspector = Default::default();
            self.inspector.enable_hevc(self.config.hevc);
        }
        if let Some(info) = self.inspector.receive(encoded) {
            self.video.submit(encoded, info);
        }
        Ok(())
    }
    fn audio(
        &mut self,
        pcm: &[i16],
        codec: evoparseq_phaze::AudioCodec,
        payload_bytes: usize,
    ) -> Result<()> {
        self.audio_bytes = self.audio_bytes.saturating_add(payload_bytes as u64);
        self.audio_codec = Some(codec);
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
                serde_json::json!({"codec":codec.label(),"sample_rate":48000,"channels":2}),
            );
        }
        if let Some(playback) = &self.audio {
            playback.queue(pcm);
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
                Event::NativeSession(command) => {
                    use super::session_model::{Command, View as _};
                    let result = (|| -> Result<()> {
                        match command {
                            Command::Apply(video) => {
                                video.validate()?;
                                anyhow::ensure!(
                                    video.resolution
                                        == crate::session_preferences::Resolution::Host
                                        && video.constant_fps.is_none()
                                        && video.rotated.is_none(),
                                    "Unsupported Phaze stream option"
                                );
                                let config = evoparseq_phaze::VideoConfig {
                                    mbps: video.bandwidth_mbps.unwrap_or(self.config.mbps),
                                    fps: video.fps.unwrap_or(self.config.fps),
                                    ..self.config
                                };
                                config.validate()?;
                                self.manager.update_native(|p| p.video = video.clone())?;
                                self.window.set_input_state(crate::input_state::State {
                                    mode: video.mode(),
                                    detached: self.window.input_detached(),
                                    ..Default::default()
                                });
                                self.window.set_fullscreen(video.fullscreen);
                                if let Some(output) = output {
                                    self.pending = Some((output, config));
                                    self.config = config;
                                }
                            }
                            Command::SelectDisplay(id) => {
                                self.window
                                    .session
                                    .displays
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .validate_selection(&id, false)?;
                                self.keys.reset();
                                commands.clear();
                                commands.push(Input::Release);
                                self.pending = Some((id.parse()?, self.config));
                            }
                            Command::HideButton => {
                                self.manager
                                    .update_native(|p| p.hide_session_button = true)?;
                                self.window.native_refresh_overlay();
                            }
                            _ => anyhow::bail!("Unsupported Phaze control"),
                        }
                        Ok(())
                    })();
                    if let Err(error) = result {
                        crate::event_log::record(
                            "WARN",
                            "phaze_control_failed",
                            serde_json::json!({"reason":error.to_string()}),
                        );
                        self.window.result(false);
                    } else if self.pending.is_none() {
                        self.window.result(true);
                    }
                }
                event => {
                    let kind = match &event {
                        Event::Motion(..) | Event::RelativeMotion(..) => Some((0, "motion")),
                        Event::Button(true, ..) => Some((1, "button")),
                        Event::Scroll(..) => Some((2, "scroll")),
                        Event::Key(true, ..) => Some((3, "keyboard")),
                        _ => None,
                    };
                    let viewport = self.window.presented_viewport();
                    let allowed = output.is_some() && self.window.native_input_allowed();
                    let before = commands.len();
                    translate(event, &mut self.keys, viewport, allowed, commands)?;
                    if let Some((index, kind)) = kind {
                        let queued = commands.len() != before;
                        let bit = 1 << u8::from(queued);
                        if self.input_logged[index] & bit == 0 && crate::event_log::enabled() {
                            self.input_logged[index] |= bit;
                            crate::event_log::record(
                                "INFO",
                                "phaze_input_dispatch",
                                serde_json::json!({
                                    "kind":kind,
                                    "queued":queued,
                                    "output_selected":output.is_some(),
                                    "input_allowed":allowed,
                                    "viewport_available":viewport.is_some(),
                                    "input_detached":self.window.input_detached(),
                                }),
                            );
                        }
                    }
                }
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
    window.session.phaze.store(true, Ordering::Release);
    let saved = manager.view().0;
    window.set_input_state(crate::input_state::State {
        mode: saved.native_core.video.mode(),
        ..Default::default()
    });
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
    let mut inspector = crate::video_stream::Inspector::default();
    inspector.enable_hevc(saved.prefer_hevc);
    let mut view = View {
        window: window.clone(),
        manager: manager.clone(),
        config: evoparseq_phaze::VideoConfig {
            hevc: saved.prefer_hevc,
            chroma_444: saved.prefer_hevc && saved.experimental_chroma_444,
            mbps: saved.native_core.video.bandwidth_mbps.unwrap_or(10),
            fps: saved.native_core.video.fps.unwrap_or(60),
        },
        pending: None,
        received: 0,
        sent: 0,
        rtt: None,
        audio_bytes: 0,
        video_payload_bytes: None,
        sent_video_config: None,
        raw_audio: saved.allow_raw_audio,
        audio_codec: None,
        authenticated: false,
        video,
        generation,
        inspector,
        audio: None,
        keys: Default::default(),
        events: VecDeque::new(),
        input_logged: [0; 4],
        route: "Direct",
        carrier: "direct UDP",
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
            result=connect(&remote,peer,&manager,&mut view)=>result,
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

async fn connect(
    remote: &evoparseq_phaze::Session,
    peer: &str,
    manager: &Manager,
    view: &mut View,
) -> Result<()> {
    if !manager.view().0.force_relay {
        return evoparseq_phaze::connect(remote, peer, view).await;
    }
    use evoparseq_phaze::turn::{Carrier, Relay};
    use webrtc::ice::url::{ProtoType, SchemeType, Url};
    let configuration = manager.resolve_configuration().await?;
    for server in configuration.ice_servers {
        for raw in server.urls {
            let url = Url::parse_url(&raw)?;
            let (carrier, name) = match (url.scheme, url.proto) {
                (SchemeType::Turn, ProtoType::Udp) => (Carrier::Udp, "TURN / UDP"),
                (SchemeType::Turn, ProtoType::Tcp) => (Carrier::Tcp, "TURN / TCP"),
                (SchemeType::Turns, ProtoType::Tcp) => (Carrier::Tls, "TURN / TLS"),
                _ => continue,
            };
            view.carrier = name;
            evoparseq_phaze::Sink::stage(view, "allocating_turn");
            let trusted_roots: Vec<_> = if carrier == Carrier::Tls {
                rustls_native_certs::load_native_certs()
                    .certs
                    .into_iter()
                    .map(|c| c.as_ref().to_vec())
                    .collect()
            } else {
                Vec::new()
            };
            let relay = match Relay::allocate_trusted(
                &url.host,
                url.port,
                carrier,
                &server.username,
                &server.credential,
                &trusted_roots,
            )
            .await
            {
                Ok(relay) => relay,
                Err(error) => {
                    crate::event_log::record(
                        "WARN",
                        "phaze_turn_allocation_failed",
                        serde_json::json!({"carrier":name,"reason":error.to_string()}),
                    );
                    continue;
                }
            };
            view.route = "TURN";
            view.carrier = name;
            let result = evoparseq_phaze::connect_via(remote, peer, view, &relay).await;
            relay.close().await;
            return result;
        }
    }
    anyhow::bail!(
        "No configured TURN server could allocate a Phaze relay. Check the URL and credentials."
    )
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
    fn offline_session_controls_reuse_preferences_and_validate_advertised_outputs() -> Result<()> {
        use super::super::session_model::Command;
        use evoparseq_phaze::Sink as _;
        let directory = std::env::temp_dir().join(format!(
            "evoparseq-phaze-controls-{}",
            crate::bud_attempt::native_attempt_id()?
        ));
        std::fs::create_dir(&directory)?;
        let manager = Manager::open(directory.join("settings.json"));
        {
            let window = Window::create_native()?;
            let _lifecycle = SessionWindow(window.clone());
            window.session.phaze.store(true, Ordering::Release);
            window.install_connection_settings(manager.clone());
            let mut view = View {
                video: crate::video_windows::Pipeline::start(
                    window.clone(),
                    crate::video_output::DisplayCatchUpPolicy::Fixed,
                ),
                generation: window.stats.begin(),
                window: window.clone(),
                manager: manager.clone(),
                config: Default::default(),
                pending: None,
                received: 0,
                sent: 0,
                rtt: None,
                audio_bytes: 0,
                video_payload_bytes: None,
                sent_video_config: None,
                raw_audio: false,
                audio_codec: None,
                authenticated: false,
                inspector: Default::default(),
                audio: None,
                keys: Default::default(),
                events: VecDeque::new(),
                input_logged: [0; 4],
                route: "Direct",
                carrier: "direct UDP",
            };
            let output = |id| evoparseq_phaze::Output {
                id,
                name: format!("Fixture {id}"),
                width: 1920,
                height: 1080,
                active: true,
                primary: id == 7,
            };
            view.outputs(&[output(7), output(8)]);
            view.stream_request_sent(7, view.config);
            view.stage("dtls_authenticated");
            view.traffic(2048, 1024);
            assert!(view.authenticated);
            assert_eq!((view.received, view.sent), (2048, 1024));
            // Exercise the actual publisher -> shared rates -> cards boundary.
            // GPU counters here are synthetic; no decoder/hardware claim is made.
            let mut before = window
                .stats
                .read()
                .context("Missing first telemetry sample")?;
            before.video_payload_bytes = Some(4_000);
            view.video_payload_bytes = Some(129_000);
            view.traffic(127048, 63524);
            let mut current = window
                .stats
                .read()
                .context("Missing second telemetry sample")?;
            before.at = current.at - Duration::from_secs(1);
            for (sample, frames) in [(&mut before, 0), (&mut current, 60)] {
                let video = sample.video.as_mut().unwrap();
                video.decoder_initialized = true;
                video.frames_decoded = frames;
            }
            let mut rates = crate::stats::Rates::default();
            rates.update(&before);
            let values = rates.update(&current);
            assert_eq!(values.incoming, Some(1.0));
            assert_eq!(values.outgoing, Some(0.5));
            assert_eq!(values.fps, Some(60.0));
            assert_eq!(values.video_payload_mbps, Some(1.0));
            let timing = window.stats.host_timing.snapshot(view.generation);
            let cards = crate::stats_view::Presentation::new(
                &current,
                &values,
                (None, None, None),
                None,
                &timing,
            )
            .cards;
            assert!(cards.contains(&("Decoded video", "60.0 fps".into())));
            assert!(cards.contains(&("Incoming traffic", "1.0 Mbps".into())));
            assert!(cards.contains(&("Outgoing traffic", "0.5 Mbps".into())));
            assert!(cards.contains(&("Connection RTT", "—".into())));
            assert!(!view.raw_audio());
            view.raw_audio = true;
            assert!(view.raw_audio());
            view.audio_codec = Some(evoparseq_phaze::AudioCodec::Aac);
            view.rtt(Some(Duration::from_micros(12500)));
            view.traffic(127048, 63524);
            let measured = window.stats.read().context("Missing RTT sample")?;
            assert_eq!(measured.rtt_ms, Some(12.5));
            assert_eq!(measured.audio_codec, Some("AAC-LC"));
            assert!(crate::stats_view::Presentation::new(
                &measured,
                &values,
                (None, None, None),
                None,
                &timing,
            )
            .cards
            .contains(&("Connection RTT", "12.5 ms".into())));
            view.rtt(None);
            view.traffic(127048, 63524);
            assert_eq!(window.stats.read().unwrap().rtt_ms, None);
            window.events.drain();
            let video = crate::session_preferences::Video {
                bandwidth_mbps: Some(40),
                fps: Some(144),
                ..Default::default()
            };
            window
                .events
                .push(Event::NativeSession(Command::Apply(video.clone())));
            let mut commands = Vec::new();
            assert!(view.poll(Some(7), &mut commands)?);
            let (id, config) = view.stream_request().context("Missing stream update")?;
            assert_eq!((id, config.mbps, config.fps), (7, 40, 144));
            assert_eq!(view.sent_video_config, Some(Default::default()));
            view.stream_request_sent(id, config);
            view.traffic(127048, 63524);
            let measured = window
                .stats
                .read()
                .context("Missing stream request sample")?;
            let video_stats = crate::stats_view::Presentation::new(
                &measured,
                &values,
                (None, None, None),
                None,
                &timing,
            )
            .select(crate::stats_view::Tab::Video);
            for (label, value) in [
                ("Requested maximum bitrate", "40.0 Mbps"),
                ("Requested frame rate", "144.0 fps"),
                ("Received video payload bitrate", "1.0 Mbps"),
            ] {
                assert!(video_stats.rows.contains(&(label.into(), value.into())));
            }
            assert_eq!(manager.view().0.native_core.video, video);
            window
                .events
                .push(Event::NativeSession(Command::SelectDisplay("99".into())));
            assert!(view.poll(Some(7), &mut commands)?);
            assert!(view.stream_request().is_none());
            window
                .events
                .push(Event::NativeSession(Command::SelectDisplay("8".into())));
            assert!(view.poll(Some(7), &mut commands)?);
            assert_eq!(view.stream_request(), Some((8, config)));
            assert_eq!(commands.last(), Some(&Input::Release));
            view.stream_request_sent(8, config);
            assert_eq!(
                window.session.displays.lock().unwrap().current.as_deref(),
                Some("8")
            );
            window.events.push(Event::NativeSession(Command::Apply(
                crate::session_preferences::Video {
                    rotated: Some(true),
                    ..video.clone()
                },
            )));
            assert!(view.poll(Some(8), &mut commands)?);
            assert!(view.stream_request().is_none());
            assert_eq!(manager.view().0.native_core.video, video);
            window
                .events
                .push(Event::NativeSession(Command::HideButton));
            assert!(view.poll(Some(8), &mut commands)?);
            assert!(manager.native_session_button_hidden());
            window
                .events
                .push(Event::NativeSession(Command::Disconnect));
            assert!(!view.poll(Some(8), &mut commands)?);
        }
        std::fs::remove_dir_all(directory)?;
        Ok(())
    }
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
