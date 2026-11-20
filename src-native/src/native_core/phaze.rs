//! Windows adapter only: independent Rust protocol -> existing media and input backends.
use super::account::{phaze_runtime as block_on, Session};
#[cfg(test)]
#[path = "phaze_turn.rs"]
mod turn_research;
use crate::platform::Desktop as _;
use crate::{
    connection_settings::Manager,
    phaze_session::{self, normalized, Stream},
    window::{Event, Window},
};
use anyhow::Result;
use std::{
    collections::VecDeque,
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};

struct View {
    window: Arc<Window>,
    manager: Arc<Manager>,
    stream: Stream,
    received: u64,
    sent: u64,
    rtt: Option<Duration>,
    audio_bytes: u64,
    raw_audio: bool,
    hid_devices: (bool, bool),
    audio_codec: Option<evoparseq_phaze::AudioCodec>,
    authenticated: bool,
    last_clipboard: String,
    pending_clipboard: Option<crate::clipboard_sync::Pending>,
    video: crate::video_windows::Pipeline,
    generation: u64,
    audio: Option<Arc<crate::audio_windows::Playback>>,
    keys: evoparseq_phaze::Keyboard,
    events: VecDeque<Event>,
    input_logged: [Option<&'static str>; 4],
    needs_initial_position: bool,
    interval_motion: u32,
    interval_buttons: [u32; 2],
    interval_keys: [u32; 2],
    last_input_delivery: evoparseq_phaze::InputDelivery,
    interval_pictures: u32,
    route: &'static str,
    carrier: &'static str,
}
impl View {
    fn publish(&self, state: &str) {
        let video = self.video.snapshot();
        let stream = self.stream.video();
        if state == "connected" && video.decoder_initialized && crate::event_log::enabled() {
            crate::event_log::record(
                "INFO",
                "phaze_video_flow",
                serde_json::json!({
                    "motion_commands":self.interval_motion,
                    "button_presses_received":self.interval_buttons[0],
                    "button_presses_queued":self.interval_buttons[1],
                    "key_presses_received":self.interval_keys[0],
                    "key_presses_queued":self.interval_keys[1],
                    "received_pictures":self.interval_pictures,
                    "decoded_total":video.frames_decoded,
                    "presented_total":video.frames_presented,
                    "dropped_total":video.frames_dropped,
                    "skipped_display_total":video.display_updates_skipped,
                    "decode_queue_depth":video.queue_depth,
                    "present_busy_total":video.presentations_busy,
                    "present_interval_ms":video.local_timing.present_interval.latest_ms,
                    "present_interval_peak_ms":video.local_timing.present_interval.peak_ms,
                }),
            );
        }
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
                video: Some(video),
                codec: stream.codec,
                hevc_advertised: self.stream.config().hevc,
                received: self.authenticated.then_some(self.received),
                sent: self.authenticated.then_some(self.sent),
                rtt_ms: self.rtt.map(|elapsed| elapsed.as_secs_f64() * 1000.0),
                audio_bytes: self.audio_bytes,
                video_payload_bytes: self.stream.payload_bytes(),
                phaze_video_request: self.stream.sent().map(|config| (config.mbps, config.fps)),
                audio_ready: self.audio.is_some(),
                audio_codec: self.audio_codec.map(|codec| codec.label()),
                audio_layout: self
                    .audio
                    .as_ref()
                    .map(|_| crate::audio_layout::Layout::STEREO),
                profile: stream.sps_profile_idc,
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
    fn usb_hid_devices(&self) -> (bool, bool) {
        self.hid_devices
    }

    fn usb_probe_enabled(&self) -> bool {
        std::env::var_os("EVOPARSEQ_PHAZE_USB_PROBE").is_some_and(|value| value == "1")
    }

    fn usb_probe_event(&mut self, event: &str, kind: Option<u8>, bytes: usize) {
        crate::event_log::record(
            "INFO",
            "phaze_usb_probe",
            serde_json::json!({"event":event,"kind":kind,"bytes":bytes}),
        );
    }

    fn peer_state(&mut self, local_id: u32, state: evoparseq_phaze::PeerState) {
        let mut peers = self
            .window
            .session
            .phaze_peers
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        peers.local_id = Some(local_id);
        peers.state = state;
    }
    fn host_flags(&mut self, flags: u32) {
        crate::event_log::record(
            "INFO",
            "phaze_host_capabilities",
            serde_json::json!({"usb_available": flags & 16 != 0}),
        );
    }
    fn controller(&mut self, id: u32) {
        let mut peers = self
            .window
            .session
            .phaze_peers
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if peers.controller != Some(id) {
            peers.controller = Some(id);
            crate::event_log::record(
                "INFO",
                "phaze_controller_changed",
                serde_json::json!({"local_has_control": peers.local_id.map(|local| id == local)}),
            );
        }
    }
    fn clipboard_text(&mut self, text: &str) {
        let desktop = crate::platform_windows::NativeDesktop(self.window.handle() as usize);
        crate::clipboard_sync::receive(
            &mut self.last_clipboard,
            &mut self.pending_clipboard,
            text,
            self.window.clipboard_write_allowed(),
            Instant::now(),
            |value| desktop.write_text(value),
        );
    }
    fn control_sent(&mut self, control: &evoparseq_phaze::ClientControl, sent: bool) {
        use super::session_model::{Notification, Status, View as _};
        use evoparseq_phaze::ClientControl;
        match control {
            ClientControl::Clipboard(text) => {
                if sent {
                    self.last_clipboard.clone_from(text);
                }
            }
            ClientControl::SecureAttention => self.window.notify(Notification::Status(if sent {
                Status::AttentionSent
            } else {
                Status::AttentionFailed
            })),
            _ => self.window.notify(Notification::Status(if sent {
                Status::ControlSent
            } else {
                Status::Failed
            })),
        }
    }
    fn raw_audio(&self) -> bool {
        self.raw_audio
    }
    fn video_config(&self) -> evoparseq_phaze::VideoConfig {
        self.stream.config()
    }
    fn stream_request(&mut self) -> Option<(u32, evoparseq_phaze::VideoConfig)> {
        self.stream.take_request()
    }
    fn take_video_recovery(&mut self) -> bool {
        let Some((_, fallback_420)) = self.video.take_keyframe_request() else {
            return false;
        };
        self.stream.prepare_recovery(fallback_420);
        true
    }
    fn video_recovery_sent(&mut self, output: u32) {
        crate::event_log::record(
            "INFO",
            "phaze_video_recovery_sent",
            serde_json::json!({"output": output}),
        );
    }
    fn traffic(&mut self, received: u64, sent: u64) {
        self.received = received;
        self.sent = sent;
        self.publish("connected");
        self.interval_motion = 0;
        self.interval_buttons = [0; 2];
        self.interval_keys = [0; 2];
        self.interval_pictures = 0;
    }
    fn rtt(&mut self, elapsed: Option<Duration>) {
        self.rtt = elapsed;
    }
    fn input_delivery(&mut self, delivery: evoparseq_phaze::InputDelivery) {
        if !crate::event_log::enabled() || delivery == self.last_input_delivery {
            return;
        }
        self.last_input_delivery = delivery;
        crate::event_log::record(
            "INFO",
            "phaze_input_transport",
            serde_json::json!({
                "sent_total":delivery.sent,
                "acknowledged_total":delivery.acknowledged,
                "retransmitted_total":delivery.retransmitted,
                "oldest_unacknowledged_attempt_ms":delivery.oldest_unacknowledged_attempt
                    .map(|age| age.as_secs_f64() * 1000.),
            }),
        );
    }
    fn bandwidth_feedback_started(&mut self, lower_rate: f64, upper_rate: f64) {
        crate::phaze_session::log_bandwidth_feedback(lower_rate, upper_rate);
    }
    fn cursor_mode(&mut self, relative: bool, visible: bool) {
        self.window.set_host_relative_mouse(relative);
        self.window
            .set_cursor(crate::cursor::Request::Visible(visible));
        crate::event_log::record(
            "INFO",
            "phaze_cursor_visibility",
            serde_json::json!({"visible": visible, "relative": relative}),
        );
    }
    fn cursor_image(&mut self, image: evoparseq_phaze::CursorImage<'_>) {
        use crate::cursor::{Frame, Request};
        use evoparseq_phaze::CursorFormat;
        let (width, height) = image.size;
        let (x, y) = image.hotspot;
        let frame = match image.format {
            CursorFormat::Png => Frame::native_png(image.bytes, x as i32, y as i32),
            CursorFormat::Rgba => {
                Frame::rgba(image.bytes.to_vec(), width, height, x as i32, y as i32)
            }
            CursorFormat::Monochrome => {
                Frame::monochrome(image.bytes.to_vec(), width, height, x, y)
            }
        };
        match frame {
            Ok(frame) => self.window.set_cursor(Request::Image(Some(frame))),
            Err(_) => crate::event_log::record(
                "WARN",
                "phaze_cursor_image_invalid",
                serde_json::json!({}),
            ),
        }
    }
    fn stream_request_sent(&mut self, output: u32, config: evoparseq_phaze::VideoConfig) {
        use super::session_model::{Notification, Status, View as _};
        self.stream.request_sent(output, config);
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
            displays.hid_mouse = self.hid_devices.0;
        }
        self.window.notify(Notification::DisplaysChanged);
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
        if let Some(info) = self.stream.inspect(encoded, reset) {
            self.interval_pictures += u32::from(info.picture);
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
        controls: &mut Vec<evoparseq_phaze::ClientControl>,
    ) -> Result<bool> {
        use super::session_model::{Command, Notification, Status, View as _};
        use evoparseq_phaze::{ClientControl, Input, MAX_CLIPBOARD_BYTES};
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
        let desktop = crate::platform_windows::NativeDesktop(self.window.handle() as usize);
        if self
            .events
            .iter()
            .any(|event| matches!(event, Event::Clipboard | Event::Focus(false)))
        {
            self.pending_clipboard = None;
        }
        if self.pending_clipboard.is_some() {
            crate::clipboard_sync::retry(
                &mut self.last_clipboard,
                &mut self.pending_clipboard,
                self.window.clipboard_write_allowed(),
                Instant::now(),
                |value| desktop.write_text(value),
            );
        }
        while let Some(event) = self.events.pop_front() {
            match event {
                Event::Shortcut(crate::input::Shortcut::Disconnect)
                | Event::NativeSession(super::session_model::Command::Disconnect) => {
                    return Ok(false);
                }
                Event::Shortcut(crate::input::Shortcut::Fullscreen) => {
                    self.window.set_fullscreen(!self.window.native_fullscreen())
                }
                Event::Shortcut(crate::input::Shortcut::ReleaseInput) => {
                    self.keys.reset();
                    commands.push(Input::Release);
                    self.window.toggle_input_detached();
                }
                Event::Clipboard => {
                    self.window.take_clipboard_event();
                    if output.is_some() && self.window.clipboard_allowed() {
                        if let Some(text) = desktop.read_text().filter(|text| {
                            text.len() <= MAX_CLIPBOARD_BYTES && text != &self.last_clipboard
                        }) {
                            controls.push(ClientControl::Clipboard(text));
                        }
                    }
                }
                Event::NativeSession(Command::SecureAttention)
                | Event::PhazeControl(ClientControl::SecureAttention) => {
                    let control = ClientControl::SecureAttention;
                    if self
                        .window
                        .session
                        .phaze_peers
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .allows(&control)
                    {
                        controls.push(control);
                    } else {
                        self.window
                            .notify(Notification::Status(Status::AttentionFailed));
                    }
                }
                Event::PhazeControl(control) => {
                    if self
                        .window
                        .session
                        .phaze_peers
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .allows(&control)
                    {
                        controls.push(control);
                    } else {
                        self.window.result(false);
                    }
                }
                Event::NativeSession(command) => {
                    let result = (|| -> Result<()> {
                        match command {
                            Command::Apply(video) => {
                                let config = self.stream.validate_update(&video)?;
                                self.manager.update_native(|p| p.video = video.clone())?;
                                self.window.set_input_state(crate::input_state::State {
                                    mode: video.mode(),
                                    detached: self.window.input_detached(),
                                    ..Default::default()
                                });
                                self.window.set_fullscreen(video.fullscreen);
                                self.stream.apply(config, output);
                            }
                            Command::SelectDisplay(id) => {
                                self.window
                                    .session
                                    .displays
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .validate_selection(&id, self.hid_devices.0)?;
                                self.keys.reset();
                                commands.clear();
                                commands.push(Input::Release);
                                self.stream.select(id.parse()?);
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
                    } else if !self.stream.pending() {
                        self.window.result(true);
                    }
                }
                Event::Focus(true) => self.needs_initial_position = true,
                event => {
                    let reset = match event {
                        Event::Focus(false) => Some("focus_lost"),
                        Event::HidInputRelease => Some("input_released"),
                        _ => None,
                    };
                    if let Some(reason) = reset.filter(|_| crate::event_log::enabled()) {
                        crate::event_log::record(
                            "INFO",
                            "phaze_input_reset",
                            serde_json::json!({"reason":reason}),
                        );
                    }
                    let kind = match &event {
                        Event::Motion(..) | Event::RelativeMotion(..) => Some((0, "motion")),
                        Event::Button(true, ..) => Some((1, "button")),
                        Event::Scroll(..) => Some((2, "scroll")),
                        Event::Key(true, ..) => Some((3, "keyboard")),
                        _ => None,
                    };
                    let viewport = self.window.presented_viewport();
                    let blocked_by = if output.is_some() {
                        self.window.native_input_blocker()
                    } else {
                        Some("no_selected_output")
                    };
                    let allowed = blocked_by.is_none();
                    let before = commands.len();
                    translate(event, &mut self.keys, viewport, allowed, commands)?;
                    if self.needs_initial_position && commands.len() != before {
                        align_initial_pointer(
                            &mut self.needs_initial_position,
                            self.window.native_relative_mouse(),
                            viewport,
                            commands,
                            before,
                        );
                    }
                    if let Some((index, kind)) = kind {
                        let queued = commands.len() != before;
                        match index {
                            0 => self.interval_motion += u32::from(queued),
                            1 => {
                                self.interval_buttons[0] += 1;
                                self.interval_buttons[1] += u32::from(queued);
                            }
                            3 => {
                                self.interval_keys[0] += 1;
                                self.interval_keys[1] += u32::from(queued);
                            }
                            _ => {}
                        }
                        let dispatch = Some(blocked_by.unwrap_or(if queued {
                            "queued"
                        } else {
                            "not_translated"
                        }));
                        if self.input_logged[index] != dispatch && crate::event_log::enabled() {
                            self.input_logged[index] = dispatch;
                            crate::event_log::record(
                                "INFO",
                                "phaze_input_dispatch",
                                serde_json::json!({
                                    "kind":kind,
                                    "queued":queued,
                                    "output_selected":output.is_some(),
                                    "input_allowed":allowed,
                                    "blocked_by":blocked_by,
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
    let mut view = View {
        window: window.clone(),
        manager: manager.clone(),
        stream: Stream::new(evoparseq_phaze::VideoConfig {
            hevc: saved.prefer_hevc,
            chroma_444: saved.prefer_hevc && saved.experimental_chroma_444,
            mbps: saved.native_core.video.bandwidth_mbps.unwrap_or(10),
            fps: saved.native_core.video.fps.unwrap_or(60),
        }),
        received: 0,
        sent: 0,
        rtt: None,
        audio_bytes: 0,
        raw_audio: saved.allow_raw_audio,
        hid_devices: saved.phaze_input_mode.devices(),
        audio_codec: None,
        authenticated: false,
        last_clipboard: String::new(),
        pending_clipboard: None,
        video,
        generation,
        audio: None,
        keys: Default::default(),
        events: VecDeque::new(),
        input_logged: [None; 4],
        needs_initial_position: true,
        interval_motion: 0,
        interval_buttons: [0; 2],
        interval_keys: [0; 2],
        last_input_delivery: Default::default(),
        interval_pictures: 0,
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
    phaze_session::connect_relay(
        remote,
        peer,
        view,
        manager.resolve_configuration().await?.ice_servers,
        || {
            rustls_native_certs::load_native_certs()
                .certs
                .into_iter()
                .map(|cert| cert.as_ref().to_vec())
                .collect()
        },
        |view, name, allocated| {
            view.carrier = name;
            if allocated {
                view.route = "TURN";
            }
        },
    )
    .await
}

fn align_initial_pointer(
    pending: &mut bool,
    relative: bool,
    viewport: Option<crate::viewport::Viewport>,
    commands: &mut Vec<evoparseq_phaze::Input>,
    first: usize,
) {
    use evoparseq_phaze::Input;
    match commands[first] {
        Input::Motion {
            relative: false, ..
        } => *pending = false,
        Input::Button {
            down: true, x, y, ..
        } if !relative => {
            if let Some((x, y)) = normalized(viewport, x, y) {
                commands.insert(
                    first,
                    Input::Motion {
                        x,
                        y,
                        relative: false,
                    },
                );
                *pending = false;
            }
        }
        _ => {}
    }
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
        Event::Button(down, button, x, y) => {
            commands.extend(phaze_session::button(viewport, down, button, x, y));
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
    use anyhow::Context;
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
                stream: Stream::new(Default::default()),
                received: 0,
                sent: 0,
                rtt: None,
                audio_bytes: 0,
                raw_audio: false,
                hid_devices: (false, false),
                audio_codec: None,
                authenticated: false,
                last_clipboard: String::new(),
                pending_clipboard: None,
                audio: None,
                keys: Default::default(),
                events: VecDeque::new(),
                input_logged: [None; 4],
                needs_initial_position: true,
                interval_motion: 0,
                interval_buttons: [0; 2],
                interval_keys: [0; 2],
                last_input_delivery: Default::default(),
                interval_pictures: 0,
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
            view.stage("waiting_outputs");
            window.events.drain();
            let initial_video = crate::session_preferences::Video {
                bandwidth_mbps: Some(28),
                fps: Some(90),
                ..Default::default()
            };
            window
                .events
                .push(Event::NativeSession(Command::Apply(initial_video.clone())));
            let mut commands = Vec::new();
            let mut controls = Vec::new();
            assert!(view.poll(None, &mut commands, &mut controls)?);
            assert!(view.stream_request().is_none());
            assert_eq!(manager.view().0.native_core.video, initial_video);
            view.outputs(&[output(7), output(8)]);
            let initial_config = view.video_config();
            let request = initial_config.request(7)?;
            assert_eq!(&request[9..13], &28_u32.to_be_bytes());
            assert_eq!(&request[13..17], &90_u32.to_be_bytes());
            view.stream_request_sent(7, initial_config);
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
            view.video(&[0; 129_000], false)?;
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
            assert!(view.poll(Some(7), &mut commands, &mut controls)?);
            let (id, config) = view.stream_request().context("Missing stream update")?;
            assert_eq!((id, config.mbps, config.fps), (7, 40, 144));
            assert_eq!(view.stream.sent(), Some(initial_config));
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
            assert!(view.poll(Some(7), &mut commands, &mut controls)?);
            assert!(view.stream_request().is_none());
            window
                .events
                .push(Event::NativeSession(Command::SelectDisplay("8".into())));
            assert!(view.poll(Some(7), &mut commands, &mut controls)?);
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
            assert!(view.poll(Some(8), &mut commands, &mut controls)?);
            assert!(view.stream_request().is_none());
            assert_eq!(manager.view().0.native_core.video, video);

            use evoparseq_phaze::{ClientControl, CursorFormat, CursorImage};
            view.host_flags(16);
            view.bandwidth_feedback_started(750_000.0, 875_000.0);
            view.video_recovery_sent(8);
            view.cursor_mode(false, false);
            assert!(!window.native_relative_mouse());
            view.cursor_mode(true, true);
            assert!(window.native_relative_mouse());
            view.cursor_mode(false, true);
            assert!(!window.native_relative_mouse());
            let pixel = [200, 100, 50, 128];
            for (format, bytes) in [
                (CursorFormat::Rgba, pixel.as_slice()),
                (CursorFormat::Monochrome, &[0x80, 0x00]),
                (CursorFormat::Png, &[]),
                (CursorFormat::Rgba, &[]),
            ] {
                view.cursor_image(CursorImage {
                    format,
                    size: (1, 1),
                    hotspot: (0, 0),
                    bytes,
                });
            }
            view.video(&[0, 0, 0, 1, 0x09, 0xf0], true)?;
            assert_eq!(view.stream.payload_bytes(), Some(129_006));
            assert_eq!(view.interval_pictures, 0);
            view.control_sent(&ClientControl::Clipboard("fixture".into()), false);
            assert!(view.last_clipboard.is_empty());
            view.control_sent(&ClientControl::Clipboard("fixture".into()), true);
            assert_eq!(view.last_clipboard, "fixture");
            view.control_sent(&ClientControl::SecureAttention, false);
            view.control_sent(&ClientControl::SecureAttention, true);
            view.control_sent(&ClientControl::SetControl(8), false);
            view.control_sent(&ClientControl::SetControl(8), true);

            window.events.drain();
            window
                .events
                .push(Event::NativeSession(Command::SecureAttention));
            window
                .events
                .push(Event::PhazeControl(ClientControl::SetControl(8)));
            assert!(view.poll(Some(8), &mut commands, &mut controls)?);
            assert!(controls.is_empty());
            view.peer_state(
                7,
                serde_json::from_value(serde_json::json!({
                    "guests": [
                        {"id": 7, "user_name": "Local", "has_control": true, "admin": true},
                        {"id": 8, "user_name": "Guest", "has_control": false, "admin": false}
                    ]
                }))?,
            );
            window
                .events
                .push(Event::NativeSession(Command::SecureAttention));
            window
                .events
                .push(Event::PhazeControl(ClientControl::SetControl(8)));
            window
                .events
                .push(Event::PhazeControl(ClientControl::RemoveGuest(7)));
            assert!(view.poll(Some(8), &mut commands, &mut controls)?);
            assert!(matches!(
                &controls[..],
                [ClientControl::SecureAttention, ClientControl::SetControl(8)]
            ));
            controls.clear();

            commands.clear();
            window.events.push(Event::RelativeMotion(10, 20));
            window.events.push(Event::Scroll(1, 2));
            window.events.push(Event::HidInputRelease);
            assert!(view.poll(None, &mut commands, &mut controls)?);
            assert_eq!(commands, [Input::Release]);
            window
                .events
                .push(Event::NativeSession(Command::HideButton));
            assert!(view.poll(Some(8), &mut commands, &mut controls)?);
            assert!(manager.native_session_button_hidden());
            window
                .events
                .push(Event::NativeSession(Command::Disconnect));
            assert!(!view.poll(Some(8), &mut commands, &mut controls)?);

            // An idle decoder has no recovery to dispatch. A recovery that
            // requires 4:2:0 updates both a reopen and a concurrent user request.
            assert!(!view.take_video_recovery());
            view.stream.apply(
                evoparseq_phaze::VideoConfig {
                    hevc: true,
                    chroma_444: true,
                    ..view.stream.config()
                },
                Some(8),
            );
            view.stream.prepare_recovery(false);
            assert!(view.video_config().chroma_444);
            view.stream.prepare_recovery(true);
            assert!(!view.video_config().chroma_444);
            assert_eq!(view.stream_request(), Some((8, view.video_config())));
            view.controller(42);
            view.controller(42);
            assert_eq!(
                window.session.phaze_peers.lock().unwrap().controller,
                Some(42)
            );

            block_on(async {
                let server = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
                let address = server.local_addr()?;
                let mut settings = manager.view().0;
                settings.force_relay = true;
                settings.turn_urls = vec![format!("turn:{address}?transport=tcp")];
                settings.custom_username = "fixture".into();
                manager.save(settings, "fixture-password", "", false, false)?;
                let remote = evoparseq_phaze::Session::from_token("offline-fixture")?;
                let close_connection = async {
                    let (socket, _) = server.accept().await?;
                    drop(socket);
                    Ok::<_, anyhow::Error>(())
                };
                let (error, closed) = tokio::time::timeout(Duration::from_secs(10), async {
                    tokio::join!(
                        connect(&remote, "offline-host", &manager, &mut view),
                        close_connection,
                    )
                })
                .await?;
                closed?;
                assert_eq!(view.carrier, "TURN / TCP");
                assert!(error
                    .unwrap_err()
                    .to_string()
                    .contains("No configured TURN server"));
                Ok(())
            })?;
        }
        std::fs::remove_dir_all(directory)?;
        Ok(())
    }
    #[test]
    fn first_click_aligns_only_an_unpositioned_absolute_pointer() {
        use evoparseq_phaze::Input;
        let viewport = Some(crate::viewport::Viewport {
            source: (101, 101),
            client: (101, 101),
        });
        let down = Input::Button {
            button: 1,
            x: 50,
            y: 25,
            down: true,
        };
        let position = Input::Motion {
            x: 32767,
            y: 16383,
            relative: false,
        };
        let mut pending = true;
        let mut commands = vec![down];
        align_initial_pointer(&mut pending, true, viewport, &mut commands, 0);
        assert!(pending);
        assert_eq!(commands, [down]);
        align_initial_pointer(&mut pending, false, viewport, &mut commands, 0);
        assert!(!pending);
        assert_eq!(commands, [position, down]);

        pending = true;
        commands = vec![position];
        align_initial_pointer(&mut pending, false, viewport, &mut commands, 0);
        assert!(!pending);
        assert_eq!(commands, [position]);
    }
    #[test]
    fn phaze_click_sends_only_the_button_and_focus_loss_releases() -> Result<()> {
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
        assert_eq!(
            commands,
            [Input::Button {
                button: 2,
                x: 512,
                y: 384,
                down: true,
            }]
        );
        translate(
            Event::Button(true, 0, 512, 20),
            &mut keys,
            viewport,
            true,
            &mut commands,
        )?;
        assert_eq!(commands.len(), 1);
        translate(
            Event::Button(false, 2, 512, 20),
            &mut keys,
            None,
            true,
            &mut commands,
        )?;
        assert_eq!(
            commands[1],
            Input::Button {
                button: 2,
                x: 512,
                y: 20,
                down: false
            }
        );
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
