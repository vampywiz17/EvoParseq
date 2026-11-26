//! Wayland/Vulkan/PipeWire binding for the shared Phaze session.
use crate::{account, connection_settings::Manager, interop_client, playback::Playback, telemetry};
use crate::{
    desktop_input::Event as Desktop, display_stream::DESKTOP_DISPLAYS, input_handoff::DragPhase,
    interop_session::SecondaryDisplays,
};
use anyhow::Result;
use evoparseq_host_interop::{ClientControl, DisplayCommand, InputBatch, VideoConfig};
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
    playback.local_cursor.store(true, Ordering::Release);
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
            secondary: Default::default(),
            owner: None,
            drag_origin: None,
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
        saved.phaze_display_layout,
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
    telemetry.displays.clear();
    playback.set_phaze_cursor_mode(false, true);
    playback.local_cursor.store(false, Ordering::Release);
    playback.set_cursor_image(None);
    result?;
    Ok("Phaze stream disconnected".into())
}

struct Adapter<'a> {
    playback: &'a Playback,
    telemetry: &'a telemetry::Session,
    manager: &'a Manager,
    desktop: interop_client::DesktopInput,
    secondary: SecondaryDisplays,
    owner: Option<usize>,
    drag_origin: Option<usize>,
    events: VecDeque<crate::input_handoff::Event>,
    clipboard: crate::clipboard_sync::State,
    peers: crate::interop_session::PhazePeers,
    next: Instant,
    authenticated: bool,
    paste_after_send: bool,
    paste_ready: bool,
}

impl Adapter<'_> {
    fn desktop_event(&mut self, slot: usize, event: Desktop, input: &mut InputBatch) -> Result<()> {
        if slot == 0 && !self.secondary.is_open() {
            match event {
                Desktop::Focus(true) => self.owner = Some(0),
                Desktop::Focus(false) => self.owner = None,
                _ => {}
            }
            input.target(None);
            return self
                .desktop
                .event(event, self.playback.input_geometry(), input);
        }
        let target = (slot != 0).then(|| self.playback.display(slot)).flatten();
        let (geometry, output, mode) = if slot == 0 {
            (
                self.playback.input_geometry(),
                None,
                self.playback.phaze_pointer_mode(),
            )
        } else if let (Some(target), Some(output)) = (target, self.secondary.output(slot)) {
            if !self.secondary.ready(slot) && !matches!(event, Desktop::Focus(_) | Desktop::Release)
            {
                return Ok(());
            }
            (
                target.input_geometry(),
                Some(output),
                target.phaze_pointer_mode(),
            )
        } else {
            return Ok(());
        };
        if self.drag_origin.is_some() && !matches!(event, Desktop::Release) {
            return Ok(());
        }
        if matches!(event, Desktop::Focus(true)) {
            if self.owner.is_some_and(|owner| owner != slot) {
                input.target(None);
                self.desktop.event(Desktop::Release, geometry, input)?;
            }
            self.owner = Some(slot);
        } else if self.owner != Some(slot) {
            return Ok(());
        }
        input.target(output);
        self.desktop.cursor_mode(mode.0, mode.1);
        self.desktop.event(event, geometry, input)?;
        if matches!(event, Desktop::Focus(false) | Desktop::Release) {
            self.owner = None;
            self.drag_origin = None;
        }
        Ok(())
    }

    fn drag_event(
        &mut self,
        slot: usize,
        mut pointer: crate::desktop_input::Pointer,
        phase: DragPhase,
        input: &mut InputBatch,
    ) -> Result<()> {
        if phase == DragPhase::Begin {
            if self.drag_origin.is_some() || self.owner != Some(slot) {
                return Ok(());
            }
            self.drag_origin = Some(slot);
        }
        let Some(origin) = self.drag_origin else {
            return Ok(());
        };
        let other = (slot != 0).then(|| self.playback.display(slot)).flatten();
        let target = if slot == 0 {
            Some(self.playback)
        } else {
            other.as_deref()
        };
        let output = self.secondary.output(slot);
        let geometry = if phase == DragPhase::Cancel {
            None
        } else if slot == 0 {
            (!self.playback.phaze_pointer_mode().0).then(|| self.playback.input_geometry())
        } else if output.is_some() && self.secondary.ready(slot) {
            target
                .filter(|p| !p.stopped() && !p.phaze_pointer_mode().0)
                .map(|p| p.input_geometry())
        } else {
            None
        };
        let cancelled = geometry.is_none();
        if let Some(geometry) = geometry {
            input.target(output);
            pointer.buttons = u32::from(phase != DragPhase::Drop);
            pointer.relative = false;
            self.desktop
                .event(Desktop::Pointer(pointer), geometry, input)?;
        } else {
            input.target(None);
            self.desktop
                .event(Desktop::Release, self.playback.input_geometry(), input)?;
        }
        if cancelled || phase == DragPhase::Drop {
            self.drag_origin = None;
            self.clipboard.revoke();
            let source = (origin != 0)
                .then(|| self.playback.display(origin))
                .flatten();
            let source = if origin == 0 {
                Some(self.playback)
            } else {
                source.as_deref()
            };
            if let Some(source) = source {
                source.clipboard_allowed.store(false, Ordering::Release);
                source.deactivate_input();
                source.native_drag.store(false, Ordering::Release);
            }
            // Wayland DnD can consume key releases. Retire them before the
            // destination resumes ordinary input, after the pointer button up.
            input.target(None);
            self.desktop
                .event(Desktop::Release, self.playback.input_geometry(), input)?;
            if !cancelled && let Some(target) = target {
                self.desktop
                    .event(Desktop::Focus(true), target.input_geometry(), input)?;
                self.owner = Some(slot);
                target.activate_input();
                target.drag_activation.store(true, Ordering::Release);
                target.repaint();
            } else {
                self.owner = None;
            }
            crate::event_log::record(
                "INFO",
                "phaze_display_drag_finished",
                serde_json::json!({"source_stream":origin,"target_stream":slot,"cancelled":cancelled}),
            );
        }
        Ok(())
    }
}

impl interop_client::Platform for Adapter<'_> {
    fn cursor_mode(&mut self, relative: bool, visible: bool) {
        self.desktop.cursor_mode(relative, visible);
        self.playback.set_phaze_cursor_mode(relative, visible);
    }

    fn cursor_image(&mut self, image: evoparseq_host_interop::CursorImage<'_>) {
        match crate::cursor_pixels::Pixels::decode(image) {
            Ok(image) => self
                .playback
                .set_cursor_image(Some(std::sync::Arc::new(image))),
            Err(error) => {
                self.playback.set_cursor_image(None);
                crate::event_log::record(
                    "WARN",
                    "phaze_cursor_image_rejected",
                    serde_json::json!({"error": error.to_string()}),
                );
            }
        }
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

    fn display_command(&mut self) -> Option<DisplayCommand> {
        self.secondary.command()
    }

    fn secondary_video(&mut self, output: u32, encoded: &[u8], reset: bool) {
        if let Some(slot) = self.secondary.slot(output) {
            if let Some(channel) = self.telemetry.displays.get(slot) {
                if reset {
                    channel.playback.encoder_rebuilt();
                }
                if let Some(info) = self.secondary.inspect(slot, encoded, reset) {
                    if channel.playback.submit_packet(encoded, info).is_err() {
                        channel.playback.stop();
                    }
                }
            }
        }
    }

    fn secondary_cursor_mode(&mut self, output: u32, relative: bool, visible: bool) {
        if let Some(channel) = self
            .secondary
            .slot(output)
            .and_then(|slot| self.telemetry.displays.get(slot))
        {
            channel.playback.set_phaze_cursor_mode(relative, visible);
        }
    }

    fn secondary_lost(&mut self, output: u32) {
        if let Some(slot) = self.secondary.lost(output) {
            self.telemetry.displays.close(slot);
        }
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
        input: &mut InputBatch,
        controls: &mut Vec<ClientControl>,
    ) -> Result<bool> {
        use crate::{
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
        for slot in 1..DESKTOP_DISPLAYS {
            if !self.secondary.is_open() {
                break;
            }
            if let Some(channel) = self.telemetry.displays.get(slot) {
                channel.playback.poll_gpu();
                channel.playback.set_input_ready(self.secondary.ready(slot));
                if channel.playback.stopped()
                    || channel.playback.video_snapshot().failure_stage.is_some()
                    || self.secondary.timed_out(slot)
                {
                    self.secondary.close(slot);
                    self.telemetry.displays.close(slot);
                }
            }
        }
        if self
            .owner
            .is_some_and(|slot| slot != 0 && self.secondary.output(slot).is_none())
        {
            self.desktop
                .event(Desktop::Release, self.playback.input_geometry(), input)?;
            self.owner = None;
            self.drag_origin = None;
        }
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
                    self.desktop_event(0, event, input)?;
                }
                Event::DisplayDesktop { stream, event } => {
                    if matches!(event, Desktop::Focus(true)) && self.owner != Some(stream) {
                        self.clipboard.revoke();
                        paste_now = false;
                        self.paste_after_send = false;
                        controls.retain(|control| !matches!(control, ClientControl::Clipboard(_)));
                    }
                    self.desktop_event(stream, event, input)?
                }
                Event::DisplayDrag {
                    stream,
                    pointer,
                    phase,
                } => {
                    self.drag_event(stream, pointer, phase, input)?;
                    if phase == DragPhase::Drop || phase == DragPhase::Cancel {
                        paste_now = false;
                        self.paste_after_send = false;
                        controls.retain(|control| !matches!(control, ClientControl::Clipboard(_)));
                    }
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
                    input.target(None);
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
                            Command::AddPhazeDisplay(id) => {
                                anyhow::ensure!(
                                    session.hid_devices == (false, false),
                                    "Additional displays require normal input"
                                );
                                anyhow::ensure!(
                                    self.secondary.available(id, output),
                                    "Display is already open"
                                );
                                anyhow::ensure!(
                                    host.outputs
                                        .iter()
                                        .any(|output| output.id == id.to_string()),
                                    "Phaze display is unavailable"
                                );
                                let slot = self.secondary.vacant().ok_or_else(|| {
                                    anyhow::anyhow!("No additional display slot is available")
                                })?;
                                let config = session.stream.config();
                                self.telemetry.displays.open_phaze(
                                    slot,
                                    id,
                                    self.playback.ui_context(),
                                    config.hevc,
                                )?;
                                self.secondary.open(slot, id, config);
                            }
                            Command::SelectDisplay(id) => {
                                anyhow::ensure!(
                                    self.secondary.slot(id.parse()?).is_none(),
                                    "Display is already open"
                                );
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
        input.target(None);
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

#[cfg(test)]
#[test]
fn linux_phaze_drop_and_cancel_restore_input_after_wayland_drag() -> Result<()> {
    use crate::{connection::Mode, desktop_input::Pointer};
    use evoparseq_host_interop::Input;
    use std::sync::Arc;

    let ctx = eframe::egui::Context::default();
    let manager = Manager::open(std::env::temp_dir().join(format!(
        "evoparseq-phaze-drag-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos(),
    )));
    for (origin, destination, phase) in [
        (0, 0, DragPhase::Drop),
        (1, 1, DragPhase::Drop),
        (0, 1, DragPhase::Drop),
        (1, 0, DragPhase::Drop),
        (0, 1, DragPhase::Cancel),
        (1, 0, DragPhase::Cancel),
    ] {
        let telemetry = telemetry::Session::new(Mode::Phaze);
        let root = Playback::new(ctx.clone());
        root.attach_displays(&telemetry.displays, 0);
        *telemetry.displays.primary.lock().unwrap() = Arc::downgrade(&root);
        telemetry.displays.open_phaze(1, 29, &ctx, false)?;
        let peer = telemetry.displays.get(1).unwrap().playback.clone();
        root.set_input_geometry((1920, 1080));
        peer.set_input_geometry((1280, 720));
        let mut adapter = Adapter {
            playback: &root,
            telemetry: &telemetry,
            manager: &manager,
            desktop: Default::default(),
            secondary: Default::default(),
            owner: None,
            drag_origin: None,
            events: Default::default(),
            clipboard: Default::default(),
            peers: Default::default(),
            next: Instant::now(),
            authenticated: true,
            paste_after_send: false,
            paste_ready: false,
        };
        adapter.secondary.open(
            1,
            29,
            VideoConfig {
                hevc: false,
                ..Default::default()
            },
        );
        assert!(
            adapter
                .secondary
                .inspect(1, &[0, 0, 1, 0x65, 0x80], false)
                .is_some()
        );
        let source = if origin == 0 { &root } else { &peer };
        let target = if destination == 0 { &root } else { &peer };
        source.activate_input();
        source.native_drag.store(true, Ordering::Release);
        source.clipboard_allowed.store(true, Ordering::Release);
        let pointer = Pointer {
            x: 200.,
            y: 120.,
            relative: false,
            moved: true,
            viewport: (640, 360),
            buttons: 1,
            wheel: (0, 0),
        };
        let mut input = InputBatch::default();
        adapter.desktop_event(origin, Desktop::Focus(true), &mut input)?;
        adapter.drag_event(origin, pointer, DragPhase::Begin, &mut input)?;
        adapter.drag_event(destination, pointer, DragPhase::Motion, &mut input)?;
        adapter.drag_event(destination, pointer, phase, &mut input)?;
        assert!(!source.native_drag.load(Ordering::Acquire));
        assert!(!source.clipboard_allowed.load(Ordering::Acquire));
        assert!(adapter.drag_origin.is_none());
        if phase == DragPhase::Cancel {
            assert!(!source.input_active());
            assert_eq!(adapter.owner, None);
            assert!(input.iter().any(|event| *event == Input::Release));
        } else {
            assert!(target.input_active());
            assert!(target.drag_activation.load(Ordering::Acquire));
            assert_eq!(adapter.owner, Some(destination));
            let release = input
                .iter()
                .position(|event| matches!(event, Input::Button { down: false, .. }))
                .unwrap();
            assert!(
                input
                    .iter()
                    .skip(release + 1)
                    .any(|event| *event == Input::Release)
            );
            assert_eq!(
                input
                    .iter()
                    .filter(|event| matches!(event, Input::Button { down: true, .. }))
                    .count(),
                1
            );
            input.clear();
            adapter.desktop_event(destination, Desktop::Pointer(pointer), &mut input)?;
            assert!(
                input
                    .iter()
                    .any(|event| matches!(event, Input::Button { down: true, .. })),
                "the click following a completed drag must reach the host"
            );
            if origin != destination {
                adapter.desktop_event(origin, Desktop::Focus(false), &mut input)?;
                assert_eq!(adapter.owner, Some(destination));
                assert!(
                    !input.iter().any(|event| *event == Input::Release),
                    "late source focus loss must not cancel the new destination click"
                );
            }
        }
        telemetry.displays.clear();
    }
    Ok(())
}
