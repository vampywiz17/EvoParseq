//! Wayland/Vulkan/PipeWire binding for the shared Phaze session.
use crate::{account, connection_settings::Manager, interop_client, playback::Playback, telemetry};
use anyhow::Result;
use evoparseq_host_interop::{ClientControl, Input, VideoConfig};
use std::{
    collections::VecDeque,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

pub fn run(
    account: &account::Session,
    peer: &str,
    playback: &Playback,
    telemetry: &telemetry::Session,
    manager: &Manager,
) -> Result<String> {
    let saved = manager.view().0;
    let mut audio = crate::audio_pipewire::Config::new(&saved, crate::audio_layout::Layout::STEREO);
    if !audio.custom {
        audio.minimum_ms = 50;
        audio.maximum_ms = 100;
    }
    let pcm = crate::audio_stream::DecodedPcm::stereo(audio.maximum_ms);
    let receiver = pcm.receiver();
    let _output = crate::audio_pipewire::Output::start(&receiver, telemetry.output.clone(), audio)?;
    telemetry.set_audio(receiver);
    let mut client = interop_client::Client::new(
        Adapter {
            playback,
            telemetry,
            manager,
            desktop: Default::default(),
            events: VecDeque::new(),
            clipboard: Default::default(),
            peers: Default::default(),
            next: Instant::now(),
            authenticated: false,
            paste_after_send: false,
            paste_ready: false,
        },
        VideoConfig {
            hevc: playback.hevc(),
            chroma_444: false, // The existing Vulkan Video path supports 4:2:0.
            fps: saved.native_core.video.fps.unwrap_or(60),
            mbps: saved.native_core.video.bandwidth_mbps.unwrap_or(10),
        },
        pcm,
        saved.allow_raw_audio,
    )
    .with_hid_devices(saved.phaze_input_mode.devices());
    let result = account::phaze_runtime(async {
        let servers = if saved.force_relay {
            Some(manager.resolve_configuration().await?.ice_servers)
        } else {
            None
        };
        let stopped = async {
            while !playback.stopped() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Ok(())
        };
        tokio::select! {
            biased;
            result = client.connect(account.token(), peer, servers, || {
                rustls_native_certs::load_native_certs()
                    .certs.into_iter().map(|cert| cert.as_ref().to_vec()).collect()
            }) => result,
            result = stopped => result,
        }
    });
    playback.set_phaze_cursor_mode(false, true);
    result?;
    Ok("Phaze stream disconnected".into())
}

struct Adapter<'a> {
    playback: &'a Playback,
    telemetry: &'a telemetry::Session,
    manager: &'a Manager,
    desktop: interop_client::DesktopInput,
    events: VecDeque<crate::input_handoff::Event>,
    clipboard: crate::clipboard_sync::State,
    peers: crate::interop_session::PhazePeers,
    next: Instant,
    authenticated: bool,
    paste_after_send: bool,
    paste_ready: bool,
}

impl interop_client::Platform for Adapter<'_> {
    fn cursor_mode(&mut self, relative: bool, visible: bool) {
        self.desktop.cursor_mode(relative, visible);
        self.playback.set_phaze_cursor_mode(relative, visible);
    }

    fn stage(&mut self, stage: &'static str) {
        self.authenticated |= stage == "dtls_authenticated";
        crate::event_log::record(
            "INFO",
            "phaze_connection_stage",
            serde_json::json!({"stage":stage}),
        );
    }

    fn outputs(&mut self, outputs: &[evoparseq_host_interop::Output]) {
        crate::event_log::record(
            "INFO",
            "phaze_displays_received",
            serde_json::json!({
                "count": outputs.len(),
                "active": outputs.iter().filter(|output| output.active).count(),
                "active_primary": outputs.iter().filter(|output| output.active && output.primary).count()
            }),
        );
        let mut host = self
            .telemetry
            .host
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        host.outputs = outputs
            .iter()
            .filter(|o| o.active)
            .map(|output| crate::host_displays::Output {
                id: output.id.to_string(),
                name: output.name.clone(),
                adapter: String::new(),
            })
            .collect();
        host.revision = host.revision.wrapping_add(1);
    }

    fn stream_request_sent(&mut self, output: u32, _: VideoConfig) {
        let mut host = self
            .telemetry
            .host
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        host.phaze_output = Some(output.to_string());
        host.pending = false;
        host.status = crate::session_controls::Status::RequestSent.text().into();
        host.revision = host.revision.wrapping_add(1);
        self.next = Instant::now();
        self.playback.repaint();
    }

    fn video(&mut self, encoded: &[u8], info: crate::video_stream::PacketInfo) -> Result<()> {
        self.playback.submit_packet(encoded, info)?;
        Ok(())
    }

    fn reset_video(&mut self) {
        self.playback.encoder_rebuilt();
    }
    fn take_recovery(&mut self) -> bool {
        self.playback
            .take_keyframe_request()
            .is_ok_and(|request| request.is_some())
    }

    fn peer_state(&mut self, local_id: u32, state: evoparseq_host_interop::PeerState) {
        self.peers.local_id = Some(local_id);
        self.peers.state = state;
        self.telemetry
            .host
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .windows = self.peers.admin();
    }

    fn clipboard(&mut self, text: &str) {
        let allowed = self.playback.clipboard_allowed.load(Ordering::Acquire);
        self.clipboard
            .receive(text, allowed, |text| self.playback.clipboard_write(text));
    }

    fn control_sent(&mut self, control: &ClientControl, sent: bool) {
        if let ClientControl::Clipboard(text) = control {
            if sent {
                self.clipboard.local_sent(text);
            }
            self.paste_ready |= std::mem::take(&mut self.paste_after_send) && sent;
        }
    }

    fn poll(
        &mut self,
        session: &mut interop_client::State,
        output: Option<u32>,
        input: &mut Vec<Input>,
        controls: &mut Vec<ClientControl>,
    ) -> Result<bool> {
        use crate::{
            desktop_input::Event as Desktop,
            input_handoff::Event,
            session_controls::{Command, Status},
        };
        if self.playback.stopped() {
            return Ok(false);
        }
        self.playback.poll_gpu();
        if let Some(stage) = self.playback.video_snapshot().failure_stage {
            anyhow::bail!("Phaze video stopped: {stage}");
        }
        self.playback.set_input_ready(output.is_some());
        self.playback.input.take(&mut self.events)?;
        let geometry = self.playback.input_geometry();
        let mut paste_now = std::mem::take(&mut self.paste_ready);
        while let Some(event) = self.events.pop_front() {
            match event {
                Event::Desktop(event) | Event::DisplayDesktop { stream: 0, event } => {
                    if matches!(event, Desktop::Focus(false) | Desktop::Release) {
                        self.clipboard.revoke();
                        paste_now = false;
                        self.paste_after_send = false;
                        controls.retain(|control| !matches!(control, ClientControl::Clipboard(_)));
                    }
                    self.desktop.event(event, geometry, input)?;
                }
                Event::Session(Command::Disconnect) => return Ok(false),
                Event::Session(Command::Clipboard { text, paste }) if output.is_some() => {
                    let allowed = self.playback.clipboard_allowed.load(Ordering::Acquire);
                    if allowed {
                        if text.len() <= evoparseq_host_interop::MAX_CLIPBOARD_BYTES
                            && !text.contains('\0')
                            && self.clipboard.local_changed(&text)
                        {
                            controls
                                .retain(|control| !matches!(control, ClientControl::Clipboard(_)));
                            controls.push(ClientControl::Clipboard(text));
                            self.paste_after_send |= paste;
                        } else {
                            paste_now |= paste;
                        }
                    }
                }
                Event::Session(command) => {
                    let mut host = self
                        .telemetry
                        .host
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    let result = (|| -> Result<()> {
                        match command {
                            Command::Apply(video) => {
                                let config = session.stream.validate_update(&video)?;
                                self.manager
                                    .update_native(|profile| profile.video = video.clone())?;
                                self.playback
                                    .immersive
                                    .store(video.immersive, Ordering::Release);
                                session.stream.apply(config, output);
                            }
                            Command::SelectDisplay(id) => {
                                // Reuse the Windows policy: USB HID mouse targets the
                                // host's primary display, not another selected output.
                                crate::host_displays::Displays {
                                    outputs: host.outputs.clone(),
                                    ..Default::default()
                                }
                                .validate_selection(&id, session.hid_devices.0)?;
                                self.desktop.event(Desktop::Release, geometry, input)?;
                                session.stream.select(id.parse()?);
                            }
                            Command::SecureAttention if self.peers.admin() => {
                                controls.push(ClientControl::SecureAttention)
                            }
                            _ => anyhow::bail!("Unsupported Phaze control"),
                        }
                        Ok(())
                    })();
                    host.pending = session.stream.pending();
                    host.status = match result {
                        Ok(()) if host.pending => Status::Waiting.text().into(),
                        Ok(()) => String::new(),
                        Err(error) => error.to_string(),
                    };
                }
                _ => {}
            }
        }
        if paste_now && self.playback.clipboard_allowed.load(Ordering::Acquire) {
            for down in [true, false] {
                self.desktop.event(
                    Desktop::Key {
                        usage: 25,
                        mods: 2,
                        down,
                    },
                    geometry,
                    input,
                )?;
            }
        }
        if Instant::now() >= self.next {
            self.next = Instant::now() + Duration::from_millis(250);
            let video = session.stream.video();
            self.telemetry.bus.publish(
                self.telemetry.generation,
                crate::stats::Sample {
                    phaze: true,
                    state: if output.is_some() {
                        "connected"
                    } else {
                        "connecting"
                    }
                    .into(),
                    protocol: format!("Phaze / {}", session.carrier),
                    route: if self.authenticated {
                        session.route
                    } else {
                        "Unknown"
                    },
                    turn_protocol: (self.authenticated && session.route == "TURN")
                        .then(|| session.carrier.trim_start_matches("TURN / ").to_owned()),
                    received: self.authenticated.then_some(session.received),
                    sent: self.authenticated.then_some(session.sent),
                    rtt_ms: session.rtt.map(|rtt| rtt.as_secs_f64() * 1000.),
                    audio_bytes: session.audio_bytes,
                    codec: video.codec,
                    hevc_advertised: session.stream.config().hevc,
                    profile: video.sps_profile_idc,
                    video_payload_bytes: session.stream.payload_bytes(),
                    phaze_video_request: session.stream.sent().map(|c| (c.mbps, c.fps)),
                    ..Default::default()
                },
            );
            self.playback.repaint();
        }
        Ok(true)
    }
}
