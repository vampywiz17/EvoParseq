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
    let socket = Socket::open(session)?;
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce).map_err(|_| anyhow::anyhow!("Could not generate connection ID"))?;
    let id: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
    let result = match mode {
        Mode::WebRtc => run_webrtc(&socket, peer, &id),
        Mode::Bud => run_bud(&socket, peer, &id),
    };
    (socket.writer)(
        native_wire::envelope("offer_cancel", peer, &id, serde_json::json!({})).to_string(),
    );
    result
}

fn run_webrtc(socket: &Socket, peer: &str, id: &str) -> Result<String> {
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
    let mut attempt =
        Attempt::spawn_configured(id, output, Some(config), StunProvider::Parsec, true, None)?;
    attempt.start_native_media();
    let receiver = attempt
        .audio_receiver()
        .context("Audio pipeline unavailable")?;
    let result = observe(Mode::WebRtc, receiver, |signaling_closed| {
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
        Ok((attempt.control_ready(), attempt.video_snapshot()))
    });
    attempt.disconnect();
    result
}

fn run_bud(socket: &Socket, peer: &str, id: &str) -> Result<String> {
    use evoparseq_bud::{
        bud::{Event, Route, Session},
        media,
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
    let mut session = Session::external_video(false)?;
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
    let mut inspector = crate::video_stream::Inspector::default();
    let mut established = false;
    let result = observe(Mode::Bud, audio.native_receiver(), |signaling_closed| {
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
        for _ in 0..32 {
            if source
                .with_audio(|bytes| {
                    sender.submit(media::opus(bytes)?);
                    Ok(())
                })?
                .is_none()
            {
                break;
            }
        }
        for _ in 0..32 {
            if source
                .with_video(|bytes| {
                    if let media::Video::AccessUnit { annex_b, .. } = media::video(bytes)? {
                        inspector.receive(annex_b);
                    }
                    Ok(())
                })?
                .is_none()
            {
                break;
            }
        }
        Ok((established, inspector.snapshot()))
    });
    session.close();
    result
}

fn observe(
    mode: Mode,
    receiver: audio_stream::NativeReceiver,
    mut poll: impl FnMut(&mut bool) -> Result<(bool, crate::video_stream::Snapshot)>,
) -> Result<String> {
    let output = audio_pipewire::Output::start(&receiver);
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut signaling_closed = false;
    let mut playback = None;
    let mut connected = false;
    let mode = mode.label();
    loop {
        let (ready, video) = poll(&mut signaling_closed)?;
        connected |= ready;
        let now = Instant::now();
        if playback.is_none() && ready && video.parameter_sets_and_idr_observed {
            let codec = video
                .codec
                .context("The host video format is unavailable")?;
            playback = Some((codec, now + Duration::from_secs(8)));
        }
        if let Some((codec, until)) = playback {
            if now >= until {
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
        std::thread::park_timeout(Duration::from_millis(10));
    }
}
