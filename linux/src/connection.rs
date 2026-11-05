use crate::{
    account,
    account_signaling::Socket,
    attempt::{Attempt, Output, StunProvider},
    audio_pipewire, audio_stream, control, native_wire,
    video_stream::VideoProtocol,
};
use anyhow::{Context, Result, bail, ensure};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    WebRtc,
    Bud,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::WebRtc => "WebRTC",
            Self::Bud => "BUD",
        }
    }
}

pub fn run_stream(
    session: &account::Session,
    peer: &str,
    mode: Mode,
    playback: &Arc<crate::playback::Playback>,
    telemetry: &Arc<crate::telemetry::Session>,
    settings: &Arc<crate::connection_settings::Manager>,
) -> Result<String> {
    // Unlock portal-backed secrets on this blocking account worker before the
    // async ICE resolver runs. Keyring UI never runs on a media/render thread.
    let (preferences, error) = settings.view();
    ensure!(error.is_none(), "Fix connection settings before connecting");
    preferences.validate()?;
    playback.bind_input_consumer();
    crate::event_log::record(
        "INFO",
        "connection_started",
        serde_json::json!({"transport":mode.label()}),
    );
    let socket = Socket::open(session)?;
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce).map_err(|_| anyhow::anyhow!("Could not generate connection ID"))?;
    let id: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
    let result = match mode {
        Mode::WebRtc => run_webrtc(&socket, peer, &id, playback, telemetry, settings),
        Mode::Bud => run_bud(&socket, peer, &id, playback, telemetry, settings),
    };
    (socket.writer)(
        native_wire::envelope("offer_cancel", peer, &id, serde_json::json!({})).to_string(),
    );
    crate::event_log::record(
        if result.is_ok() { "INFO" } else { "ERROR" },
        "connection_finished",
        serde_json::json!({"transport":mode.label(),"success":result.is_ok()}),
    );
    result
}

fn create_webrtc_attempt(
    id: &str,
    output: Output,
    config: control::Config,
    playback: &Arc<crate::playback::Playback>,
    settings: Option<Arc<dyn crate::rtc_configuration::NetworkSettings>>,
    audio_budget: Option<u32>,
) -> Result<Attempt> {
    // Match the existing Windows/Android Parsec host compatibility policy.
    // This permits legacy RSA key sizes only; signature and SDP fingerprint
    // checks remain enabled; this policy is independent of rendering state.
    let attempt = Attempt::spawn_configured(
        id,
        output,
        Some(config),
        StunProvider::Parsec,
        true,
        settings,
    )?;
    attempt.start_native_media(Some(playback.clone()), audio_budget);
    Ok(attempt)
}

fn run_webrtc(
    socket: &Socket,
    peer: &str,
    id: &str,
    playback: &Arc<crate::playback::Playback>,
    telemetry: &Arc<crate::telemetry::Session>,
    settings: &Arc<crate::connection_settings::Manager>,
) -> Result<String> {
    let writer = socket.writer.clone();
    let target = peer.to_owned();
    let offer_id = id.to_owned();
    let output = Output::new(Arc::new(move |description| {
        Ok(description.is_some_and(|description| {
            writer(native_wire::offer(
                &target,
                &offer_id,
                &description.credentials,
            ))
        }))
    }));
    let config = control::Config {
        video_protocol: VideoProtocol {
            version: 1,
            message_size: 11,
            version_offset: 0,
            flag_offset: 4,
        },
        allow_hevc: playback.hevc(),
        ten_bit: false,
        chroma_444: false,
    };
    let audio_config =
        audio_pipewire::Config::new(&settings.view().0, crate::audio_layout::Layout::STEREO);
    let mut attempt = create_webrtc_attempt(
        id,
        output,
        config.clone(),
        playback,
        Some(settings.clone()),
        Some(audio_config.maximum_ms),
    )?;
    attempt.set_linux_telemetry(telemetry.bus.clone(), telemetry.generation);
    let receiver = attempt
        .audio_receiver()
        .context("Audio pipeline unavailable")?;
    let mut input = Input::default();
    let mut diagnostics = crate::telemetry::Control::default();
    let result = observe(
        Mode::WebRtc,
        receiver,
        playback,
        telemetry,
        audio_config,
        |signaling_closed| {
            if !*signaling_closed {
                for _ in 0..64 {
                    match socket.next() {
                        Ok(Some(text)) => {
                            ensure!(
                                !native_wire::incoming(&text, id, &attempt)?,
                                "The host closed the connection"
                            );
                        }
                        Ok(None) => break,
                        Err(_) if attempt.control_ready() => {
                            *signaling_closed = true;
                            break;
                        }
                        Err(error) => return Err(error),
                    }
                }
            }
            while let Some(event) = attempt.pop_event() {
                if event["type"] == 8 {
                    ensure!(
                        (socket.writer)(native_wire::candidate(&event, peer, id)),
                        "Could not send a local candidate"
                    );
                }
            }
            if let Some(code) = attempt.host_status() {
                bail!("The host refused or ended the connection (status {code})");
            }
            if let Some(generation) = playback.take_keyframe_request()? {
                if let Err(error) = attempt.send_binary(0, config.keyframe_request()) {
                    playback.keyframe_request_failed(generation);
                    return Err(error);
                }
            }
            for _ in 0..16 {
                let Some((channel, text, payload)) = attempt.pop_binary() else {
                    break;
                };
                if channel == 0 && !text {
                    diagnostics.receive(&payload, telemetry);
                    if let Ok(message) = control::decode(&payload) {
                        if let control::Message::Status(code) = &message {
                            ensure!(*code == 0, "The host ended the connection (status {code})");
                        }
                        input.host_control(&message, playback);
                    }
                }
            }
            if let Some(stage) = attempt.failure_stage() {
                bail!("WebRTC connection failed ({stage})");
            }
            diagnostics.update(attempt.control_ready(), telemetry, |bytes| {
                attempt.send_binary(0, bytes)
            })?;
            input.pump(
                playback,
                attempt.control_ready(),
                |bytes| attempt.send_binary(0, bytes::Bytes::copy_from_slice(bytes)),
                |command| {
                    diagnostics.command(command, telemetry, playback.input_geometry(), |bytes| {
                        attempt.send_binary(0, bytes)
                    })
                },
            )?;
            Ok((attempt.control_ready(), attempt.video_snapshot(), false))
        },
    );
    let result = result.map_err(|error| {
        let state = attempt.snapshot();
        let message = format!(
            "{error}. Offer ready: {}; remote description: {}; ICE candidates (local/remote): {}/{}; open channels: {}; transport: {}; native media: {}; failure channel: {}; control receipts: {} / {} bytes; max C/V/A: {}",
            state["offer_ready"],
            state["remote_description_set"],
            state["local_candidates"],
            state["remote_candidates"],
            state["channels_open"],
            state["transport_states"],
            state["unavailable_media_bypasses_control_queue"],
            state["failure_channel"],
            state["queued_messages"],
            state["queued_message_bytes"],
            state["channel_max_message_bytes"],
        );
        eprintln!("{message}");
        anyhow::anyhow!(message)
    });
    input.release(playback, |bytes| {
        attempt.send_binary(0, bytes::Bytes::copy_from_slice(bytes))
    });
    diagnostics.stop(telemetry, |bytes| attempt.send_binary(0, bytes));
    attempt.disconnect();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_playback_keeps_host_certificate_policy_and_native_media_delivery() {
        let playback = crate::playback::Playback::new(eframe::egui::Context::default());
        let output = Output::new(Arc::new(|_| Ok(true)));
        let config = control::Config {
            video_protocol: VideoProtocol {
                version: 1,
                message_size: 11,
                version_offset: 0,
                flag_offset: 4,
            },
            allow_hevc: false,
            ten_bit: false,
            chroma_444: false,
        };
        let mut attempt = create_webrtc_attempt(
            "linux-certificate-policy",
            output,
            config,
            &playback,
            None,
            Some(100),
        )
        .unwrap();
        let report = attempt.snapshot();
        assert_eq!(report["legacy_rsa_1024_enabled"], true);
        assert_eq!(report["unavailable_media_bypasses_control_queue"], true);
        attempt.cancel();
        attempt.wait_finished(Duration::from_secs(12)).unwrap();
        assert!(
            playback.stopped(),
            "the real native sink must be bound to the connection"
        );
    }
}

fn run_bud(
    socket: &Socket,
    peer: &str,
    id: &str,
    playback: &Arc<crate::playback::Playback>,
    telemetry: &Arc<crate::telemetry::Session>,
    settings: &Arc<crate::connection_settings::Manager>,
) -> Result<String> {
    use evoparseq_bud::{
        bud::{Event, Route},
        signaling::Signaling,
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (route, mut relays, preferences) = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(15), crate::bud_routes::route(settings))
            .await
            .context("BUD route resolution timed out")?
    })?;
    let layout = if preferences.experimental_surround_audio {
        crate::audio_device::default_layout().unwrap_or(crate::audio_layout::Layout::STEREO)
    } else {
        crate::audio_layout::Layout::STEREO
    };
    let audio_config = audio_pipewire::Config::new(&preferences, layout);
    let mut wire_id = id.to_owned();
    let mut answered_at: Option<Instant> = None;
    let mut deadline = Instant::now() + Duration::from_secs(30);
    let mut direct = matches!(&route, Route::Direct { .. });
    let mut retries = 0;
    let mut session = crate::bud_ingress::session(
        playback.hevc(),
        false,
        false,
        preferences.allow_raw_audio,
        layout,
    )?;
    session.wake_consumer(std::thread::current());
    let credentials = session.offer(&wire_id, route)?;
    let mut source = session.take_media()?;
    let mut signaling = Signaling::new(peer, &wire_id)?;
    ensure!(
        (socket.writer)(signaling.offer(&credentials).to_string()),
        "Could not send BUD offer"
    );
    let audio = audio_stream::Pipeline::start_with_layout(Some(audio_config.maximum_ms), layout);
    let sender = audio.sender().context("Audio pipeline unavailable")?;
    let mut ingress = crate::bud_ingress::VideoState::new(playback.hevc(), false, false);
    let mut established = false;
    let mut input = Input::default();
    let mut diagnostics = crate::telemetry::Control::default();
    let result = observe(
        Mode::Bud,
        audio.native_receiver(),
        playback,
        telemetry,
        audio_config,
        |signaling_closed| {
            if !*signaling_closed {
                for _ in 0..64 {
                    match socket.next() {
                        Ok(Some(text)) => {
                            signaling.receive(&text, &mut session)?;
                            let value: serde_json::Value = serde_json::from_str(&text)?;
                            if value["action"] == "answer_relay"
                                && value["payload"]["attempt_id"].as_str() == Some(wire_id.as_str())
                            {
                                answered_at.get_or_insert_with(Instant::now);
                            }
                        }
                        Ok(None) => break,
                        Err(_) if established => {
                            *signaling_closed = true;
                            break;
                        }
                        Err(error) => return Err(error),
                    }
                }
            }
            let mut ended = false;
            for _ in 0..64 {
                let Some(event) = session.poll()? else { break };
                if let Some(candidate) = signaling.candidate(&event) {
                    ensure!(
                        (socket.writer)(candidate.to_string()),
                        "Could not send BUD candidate"
                    );
                }
                match event {
                    Event::Established { .. } => established = true,
                    Event::ExternalControl { bytes } => {
                        diagnostics.receive(&bytes, telemetry);
                        if let Ok(message) = control::decode(&bytes) {
                            if let control::Message::Status(code) = &message {
                                ensure!(
                                    *code == 0,
                                    "The host ended the connection (status {code})"
                                );
                            }
                            input.host_control(&message, playback);
                        }
                    }
                    Event::Ended { outcome } => {
                        ensure!(!established, "BUD connection ended: {outcome:?}");
                        ended = true;
                    }
                    Event::IoFailure { stage, kind, .. } => {
                        ensure!(!established, "BUD IO failed: {} ({kind:?})", stage.name());
                        ended = true;
                    }
                    _ => {}
                }
            }
            if !established
                && (ended
                    || Instant::now() >= deadline
                    || (direct
                        && answered_at.is_some_and(|at| at.elapsed() >= Duration::from_secs(4))))
            {
                ensure!(retries < 32, "BUD configured route attempts exhausted");
                if relays.is_none() {
                    relays = Some(runtime.block_on(async {
                        tokio::time::timeout(
                            Duration::from_secs(15),
                            crate::bud_routes::relay_route(settings, &preferences),
                        )
                        .await
                        .context("BUD relay resolution timed out")?
                    })?);
                }
                if playback.stopped() {
                    return Ok((false, ingress.snapshot(), false));
                }
                let route = relays
                    .as_mut()
                    .and_then(|routes| routes.pop_front())
                    .context("BUD configured routes exhausted")?;
                session.close();
                ensure!(
                    (socket.writer)(
                        native_wire::envelope(
                            "offer_cancel",
                            peer,
                            &wire_id,
                            serde_json::json!({})
                        )
                        .to_string()
                    ),
                    "Could not withdraw BUD offer"
                );
                wire_id = crate::bud_routes::native_attempt_id()?;
                session = crate::bud_ingress::session(
                    playback.hevc(),
                    false,
                    false,
                    preferences.allow_raw_audio,
                    layout,
                )?;
                session.wake_consumer(std::thread::current());
                let credentials = session.offer(&wire_id, route)?;
                source = session.take_media()?;
                signaling = Signaling::new(peer, &wire_id)?;
                ensure!(
                    (socket.writer)(signaling.offer(&credentials).to_string()),
                    "Could not send BUD relay offer"
                );
                direct = false;
                retries += 1;
                answered_at = None;
                deadline = Instant::now() + Duration::from_secs(30);
                return Ok((false, ingress.snapshot(), true));
            }
            if let Some(generation) = playback.take_keyframe_request()? {
                ingress.wait_for_keyframe();
                if !session.request_keyframe_format(false) {
                    playback.keyframe_request_failed(generation);
                    anyhow::bail!("BUD could not request a video resume keyframe");
                }
            }
            diagnostics.update(established, telemetry, |bytes| session.send_frame(&bytes))?;
            if telemetry.sample_due() {
                let bud = session.snapshot();
                let framing = ingress.snapshot();
                let mut sample = crate::stats::Sample {
                    state: if bud.connected {
                        "connected"
                    } else {
                        "connecting"
                    }
                    .into(),
                    rtt_ms: bud.rtt_ms,
                    received: Some(bud.udp_received),
                    sent: Some(bud.udp_sent),
                    audio_bytes: bud.channels[2].bytes,
                    codec: framing.codec,
                    profile: framing.sps_profile_idc,
                    hevc_advertised: playback.hevc(),
                    ..Default::default()
                };
                sample.bud = Some(bud);
                telemetry.bus.publish(telemetry.generation, sample);
            }
            let session_ref = std::cell::RefCell::new(&mut session);
            input.pump(
                playback,
                established,
                |bytes| session_ref.borrow_mut().send_frame(bytes),
                |command| {
                    diagnostics.command(command, telemetry, playback.input_geometry(), |bytes| {
                        session_ref.borrow_mut().send_frame(&bytes)
                    })
                },
            )?;
            let pumped =
                crate::bud_ingress::pump(&mut source, &mut ingress, playback.as_ref(), &sender)?;
            Ok((established, ingress.snapshot(), pumped.more_pending()))
        },
    );
    input.release(playback, |bytes| session.send_frame(bytes));
    diagnostics.stop(telemetry, |bytes| session.send_frame(&bytes));
    session.close();
    (socket.writer)(
        native_wire::envelope("offer_cancel", peer, &wire_id, serde_json::json!({})).to_string(),
    );
    result
}

fn observe(
    mode: Mode,
    receiver: audio_stream::NativeReceiver,
    playback: &Arc<crate::playback::Playback>,
    telemetry: &Arc<crate::telemetry::Session>,
    audio_config: audio_pipewire::Config,
    mut poll: impl FnMut(&mut bool) -> Result<(bool, crate::video_stream::Snapshot, bool)>,
) -> Result<String> {
    let _output = audio_pipewire::Output::start(&receiver, telemetry.output.clone(), audio_config)?;
    telemetry.set_audio(receiver);
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut signaling_closed = false;
    let mut keyframe_received = false;
    let mut connected = false;
    let bud = mode == Mode::Bud;
    let mode = mode.label();
    let mut ready_at = None;
    loop {
        if playback.stopped() {
            return Ok(format!("{mode} stream disconnected"));
        }
        let (ready, video, more_pending) = poll(&mut signaling_closed)?;
        connected |= ready;
        let now = Instant::now();
        if ready {
            ready_at.get_or_insert(now);
        }
        keyframe_received |= ready && video.parameter_sets_and_idr_observed;
        if !keyframe_received
            && if bud {
                ready_at.is_some_and(|at| now.duration_since(at) >= Duration::from_secs(20))
            } else {
                now >= deadline
            }
        {
            if connected {
                bail!("{mode} connected, but no complete video keyframe arrived within 20 seconds");
            }
            bail!("{mode} connection timed out");
        }
        if !more_pending {
            std::thread::park_timeout(Duration::from_millis(10));
        }
    }
}

impl crate::bud_ingress::VideoTarget for crate::playback::Playback {
    fn encoder_rebuilt(&self) {
        self.encoder_rebuilt();
    }
    fn submit_oriented(
        &self,
        bytes: &[u8],
        info: crate::video_stream::PacketInfo,
        rotation: u8,
    ) -> Result<bool> {
        ensure!(
            rotation == 0,
            "Rotated video is not supported in the Linux playback preview"
        );
        self.submit_packet(bytes, info)
    }
}

/// Transport binding only: state, framing, release policy and geometry are shared.
#[derive(Default)]
struct Input {
    engine: crate::desktop_input::Engine,
    events: std::collections::VecDeque<crate::input_handoff::Event>,
}
impl Input {
    fn host_control(&mut self, message: &control::Message, playback: &crate::playback::Playback) {
        if let control::Message::Buffer { event, .. } = message {
            let cursor = &event["cursor"];
            if cursor.is_object() {
                let relative = cursor["relative"].as_bool().unwrap_or(false);
                let hidden = cursor["hidden"].as_bool().unwrap_or(false);
                self.engine.host_cursor(
                    relative,
                    hidden,
                    cursor["positionX"].as_i64().unwrap_or(0) as i32,
                    cursor["positionY"].as_i64().unwrap_or(0) as i32,
                );
                playback.host_cursor(
                    crate::desktop_input::relative_capture(relative, hidden),
                    cursor["positionX"].as_i64().unwrap_or(0) as i32,
                    cursor["positionY"].as_i64().unwrap_or(0) as i32,
                );
            }
        }
    }
    fn pump(
        &mut self,
        playback: &crate::playback::Playback,
        ready: bool,
        mut send: impl FnMut(&[u8]) -> Result<()>,
        mut session_command: impl FnMut(crate::session_controls::Command) -> Result<()>,
    ) -> Result<()> {
        playback.set_input_ready(ready);
        if !ready {
            return Ok(());
        }
        self.engine.geometry(playback.input_geometry());
        if let Err(error) = playback.input.take(&mut self.events) {
            let _ = self.engine.event(
                crate::desktop_input::Event::Release,
                Instant::now(),
                &mut send,
            );
            return Err(error);
        }
        while let Some(event) = self.events.pop_front() {
            match event {
                crate::input_handoff::Event::Desktop(event) => {
                    self.engine.event(event, Instant::now(), &mut send)?
                }
                crate::input_handoff::Event::Session(command) => session_command(command)?,
                _ => {}
            }
        }
        Ok(())
    }
    fn release(
        &mut self,
        playback: &crate::playback::Playback,
        send: impl FnMut(&[u8]) -> Result<()>,
    ) {
        playback.set_input_ready(false);
        playback.set_capture(false);
        let _ = self
            .engine
            .event(crate::desktop_input::Event::Release, Instant::now(), send);
    }
}

#[cfg(test)]
mod input_tests {
    use super::*;
    use crate::desktop_input::{Event, Pointer};
    #[test]
    fn linux_normal_input_uses_shared_key_pointer_and_release_frames() {
        let playback = crate::playback::Playback::new(eframe::egui::Context::default());
        playback.set_input_geometry((1920, 1080));
        let mut input = Input::default();
        let mut cursor = vec![0; 34];
        cursor[12] = 9;
        cursor[32..34].copy_from_slice(&256i16.to_be_bytes());
        input.host_control(&control::decode(&cursor).unwrap(), &playback);
        assert!(playback.capture());
        playback.push_input(Event::Focus(true));
        playback.push_input(Event::Key {
            usage: 4,
            mods: 0,
            down: true,
        });
        playback.push_input(Event::Pointer(Pointer {
            x: 3.,
            y: -2.,
            relative: true,
            moved: true,
            viewport: (1280, 720),
            buttons: 1,
            wheel: (0, 0),
        }));
        let mut frames = Vec::new();
        input
            .pump(
                &playback,
                true,
                |bytes| {
                    frames.push(bytes.to_vec());
                    Ok(())
                },
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(frames[0], control::header(0, 4, 0, 1));
        assert_eq!(frames[1], control::header(3, 1, 3, -2));
        assert_eq!(frames[2], control::header(1, 1, 1, 0));
        playback.push_input(Event::Focus(false));
        input
            .pump(
                &playback,
                true,
                |bytes| {
                    frames.push(bytes.to_vec());
                    Ok(())
                },
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(frames.last().unwrap(), &control::header(24, 0, 0, 0));
        let before = frames.len();
        playback.push_input(Event::Key {
            usage: 4,
            mods: 0,
            down: true,
        });
        input
            .pump(
                &playback,
                true,
                |bytes| {
                    frames.push(bytes.to_vec());
                    Ok(())
                },
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(
            frames.len(),
            before,
            "focus loss must gate subsequent input"
        );
        input.release(&playback, |bytes| {
            frames.push(bytes.to_vec());
            Ok(())
        });
        assert!(!playback.capture() && !playback.input_ready());
        assert_eq!(frames.last().unwrap(), &control::header(24, 0, 0, 0));
    }
    #[test]
    fn linux_input_overload_releases_host_instead_of_losing_button_edges() {
        let playback = crate::playback::Playback::new(eframe::egui::Context::default());
        let mut input = Input::default();
        for _ in 0..129 {
            playback.push_input(Event::Key {
                usage: 4,
                mods: 0,
                down: true,
            });
        }
        let mut frames = Vec::new();
        assert!(
            input
                .pump(
                    &playback,
                    true,
                    |bytes| {
                        frames.push(bytes.to_vec());
                        Ok(())
                    },
                    |_| Ok(())
                )
                .is_err()
        );
        assert_eq!(frames, vec![control::header(24, 0, 0, 0).to_vec()]);
    }
}
