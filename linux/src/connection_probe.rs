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

#[derive(Clone, Copy)]
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

pub fn run(session: &account::Session, peer: &str, mode: Mode) -> Result<String> {
    run_session(session, peer, mode, None)
}

pub fn run_stream(
    session: &account::Session,
    peer: &str,
    mode: Mode,
    playback: &Arc<crate::playback::Playback>,
) -> Result<String> {
    run_session(session, peer, mode, Some(playback))
}

fn run_session(
    session: &account::Session,
    peer: &str,
    mode: Mode,
    playback: Option<&Arc<crate::playback::Playback>>,
) -> Result<String> {
    let socket = Socket::open(session)?;
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce).map_err(|_| anyhow::anyhow!("Could not generate connection ID"))?;
    let id: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
    let result = match mode {
        Mode::WebRtc => run_webrtc(&socket, peer, &id, playback),
        Mode::Bud => run_bud(&socket, peer, &id, playback),
    };
    (socket.writer)(
        native_wire::envelope("offer_cancel", peer, &id, serde_json::json!({})).to_string(),
    );
    result
}

fn create_webrtc_attempt(
    id: &str,
    output: Output,
    config: control::Config,
    playback: Option<&Arc<crate::playback::Playback>>,
) -> Result<Attempt> {
    // Match the existing Windows/Android Parsec host compatibility policy.
    // This permits legacy RSA key sizes only; signature and SDP fingerprint
    // checks remain enabled. Probe/playback must not select different policies.
    let attempt =
        Attempt::spawn_configured(id, output, Some(config), StunProvider::Parsec, true, None)?;
    attempt.start_native_media(
        playback
            .cloned()
            .map(|sink| sink as Arc<dyn crate::video_output::NativeSink>),
    );
    Ok(attempt)
}

fn run_webrtc(
    socket: &Socket,
    peer: &str,
    id: &str,
    playback: Option<&Arc<crate::playback::Playback>>,
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
        allow_hevc: false,
        ten_bit: false,
        chroma_444: false,
    };
    let mut attempt = create_webrtc_attempt(id, output, config.clone(), playback)?;
    let receiver = attempt
        .audio_receiver()
        .context("Audio pipeline unavailable")?;
    let result = observe(Mode::WebRtc, receiver, playback, |signaling_closed| {
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
        if let Some(playback) = playback {
            if let Some(generation) = playback.take_keyframe_request()? {
                if let Err(error) = attempt.send_binary(0, config.keyframe_request()) {
                    playback.keyframe_request_failed(generation);
                    return Err(error);
                }
            }
        }
        for _ in 0..16 {
            let Some((channel, text, payload)) = attempt.pop_binary() else {
                break;
            };
            if channel == 0 && !text {
                if let Ok(control::Message::Status(code)) = control::decode(&payload) {
                    ensure!(code == 0, "The host ended the connection (status {code})");
                }
            }
        }
        if let Some(stage) = attempt.failure_stage() {
            bail!("WebRTC connection failed ({stage})");
        }
        Ok((attempt.control_ready(), attempt.video_snapshot(), false))
    });
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
    attempt.disconnect();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_probe_and_play_use_the_same_host_certificate_policy() {
        let playback = crate::playback::Playback::new(eframe::egui::Context::default());
        for sink in [None, Some(&playback)] {
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
            let mut attempt =
                create_webrtc_attempt("linux-certificate-policy", output, config, sink).unwrap();
            let report = attempt.snapshot();
            assert_eq!(report["legacy_rsa_1024_enabled"], true);
            assert_eq!(report["unavailable_media_bypasses_control_queue"], true);
            attempt.cancel();
            attempt.wait_finished(Duration::from_secs(12)).unwrap();
        }
    }
}

fn run_bud(
    socket: &Socket,
    peer: &str,
    id: &str,
    playback: Option<&Arc<crate::playback::Playback>>,
) -> Result<String> {
    use evoparseq_bud::{
        bud::{Event, Route},
        signaling::Signaling,
    };

    let (_, url) = StunProvider::Parsec
        .server()
        .context("Missing STUN server")?;
    let url = webrtc::ice::url::Url::parse_url(url)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let stun = runtime.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(3),
            tokio::net::lookup_host((url.host.as_str(), url.port)),
        )
        .await
        .ok()
        .and_then(|result| result.ok())
        .map(|addresses| addresses.collect())
        .unwrap_or_default()
    });
    let mut session = crate::bud_ingress::session(
        false,
        false,
        false,
        false,
        crate::audio_layout::Layout::STEREO,
    )?;
    session.wake_consumer(std::thread::current());
    let credentials = session.offer(id, Route::Direct { stun })?;
    let mut source = session.take_media()?;
    let signaling = Signaling::new(peer, id)?;
    ensure!(
        (socket.writer)(signaling.offer(&credentials).to_string()),
        "Could not send BUD offer"
    );
    let audio = audio_stream::Pipeline::start_with_budget(Some(100));
    let sender = audio.sender().context("Audio pipeline unavailable")?;
    let mut ingress = crate::bud_ingress::VideoState::new(false, false, false);
    let mut established = false;
    let result = observe(
        Mode::Bud,
        audio.native_receiver(),
        playback,
        |signaling_closed| {
            if !*signaling_closed {
                for _ in 0..64 {
                    match socket.next() {
                        Ok(Some(text)) => signaling.receive(&text, &mut session)?,
                        Ok(None) => break,
                        Err(_) if established => {
                            *signaling_closed = true;
                            break;
                        }
                        Err(error) => return Err(error),
                    }
                }
            }
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
                        if let Ok(control::Message::Status(code)) = control::decode(&bytes) {
                            ensure!(code == 0, "The host ended the connection (status {code})");
                        }
                    }
                    Event::Ended { outcome } => bail!("BUD connection ended: {outcome:?}"),
                    Event::IoFailure { stage, kind, .. } => {
                        bail!("BUD IO failed: {} ({kind:?})", stage.name());
                    }
                    _ => {}
                }
            }
            if let Some(playback) = playback {
                if let Some(generation) = playback.take_keyframe_request()? {
                    ingress.wait_for_keyframe();
                    if !session.request_keyframe_format(false) {
                        playback.keyframe_request_failed(generation);
                        anyhow::bail!("BUD could not request a video resume keyframe");
                    }
                }
            }
            let pumped = crate::bud_ingress::pump(&mut source, &mut ingress, &playback, &sender)?;
            Ok((established, ingress.snapshot(), pumped.more_pending()))
        },
    );
    session.close();
    result
}

fn observe(
    mode: Mode,
    receiver: audio_stream::NativeReceiver,
    playback: Option<&Arc<crate::playback::Playback>>,
    mut poll: impl FnMut(&mut bool) -> Result<(bool, crate::video_stream::Snapshot, bool)>,
) -> Result<String> {
    let output = audio_pipewire::Output::start(&receiver);
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut signaling_closed = false;
    let mut keyframe = None;
    let mut connected = false;
    let mode = mode.label();
    loop {
        if playback.is_some_and(|playback| playback.stopped()) {
            return Ok(format!("{mode} stream disconnected"));
        }
        let (ready, video, more_pending) = poll(&mut signaling_closed)?;
        connected |= ready;
        let now = Instant::now();
        if keyframe.is_none() && ready && video.parameter_sets_and_idr_observed {
            let codec = video
                .codec
                .context("The host video format is unavailable")?;
            keyframe = Some((codec, now + Duration::from_secs(8)));
        }
        if let Some((codec, until)) = keyframe {
            if playback.is_none() && now >= until {
                let frames = receiver.snapshot().pcm_frames_consumed;
                let sound = match output.and_then(audio_pipewire::Output::finish) {
                    Ok(()) if frames > 0 => format!("PipeWire played {frames} PCM frames."),
                    Ok(()) => "No audio frames reached PipeWire during the test.".into(),
                    Err(error) => format!("PipeWire output failed: {error:#}"),
                };
                return Ok(format!(
                    "{mode} connected. Received {} video and a keyframe. {sound}",
                    codec.name()
                ));
            }
        } else if now >= deadline {
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

// The connection probe shares all framing/gating policy but has no decoder.
impl crate::bud_ingress::VideoTarget for Option<&Arc<crate::playback::Playback>> {
    fn encoder_rebuilt(&self) {
        if let Some(playback) = self {
            playback.encoder_rebuilt();
        }
    }
    fn submit_oriented(
        &self,
        bytes: &[u8],
        info: crate::video_stream::PacketInfo,
        rotation: u8,
    ) -> Result<bool> {
        match self {
            Some(playback) => {
                ensure!(
                    rotation == 0,
                    "Rotated video is not supported in the Linux playback preview"
                );
                playback.submit_packet(bytes, info)
            }
            None => Ok(true),
        }
    }
}
