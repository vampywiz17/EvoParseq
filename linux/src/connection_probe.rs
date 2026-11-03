use crate::{
    account,
    account_signaling::Socket,
    attempt::{Attempt, Output, StunProvider},
    control, native_wire,
    video_stream::VideoProtocol,
};
use anyhow::{Context, Result, bail, ensure};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

pub fn run(session: &account::Session, peer: &str) -> Result<String> {
    let socket = Socket::open(session)?;
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce).map_err(|_| anyhow::anyhow!("Could not generate connection ID"))?;
    let id: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
    let writer = socket.writer.clone();
    let target = peer.to_owned();
    let offer_id = id.clone();
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
        Attempt::spawn_configured(&id, output, Some(config), StunProvider::Parsec, true, None)?;
    let result = wait_for_video(&socket, &attempt, peer, &id);
    attempt.disconnect();
    (socket.writer)(
        native_wire::envelope("offer_cancel", peer, &id, serde_json::json!({})).to_string(),
    );
    result
}

fn wait_for_video(socket: &Socket, attempt: &Attempt, peer: &str, id: &str) -> Result<String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut signaling_closed = false;
    while Instant::now() < deadline {
        if !signaling_closed {
            for _ in 0..64 {
                match socket.next() {
                    Ok(Some(text)) => {
                        ensure!(
                            !native_wire::incoming(&text, id, attempt)?,
                            "The host closed the connection"
                        );
                    }
                    Ok(None) => break,
                    Err(_) if attempt.control_ready() => {
                        signaling_closed = true;
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
        let video = attempt.video_snapshot();
        if attempt.control_ready() && video.parameter_sets_and_idr_observed {
            let codec = video
                .codec
                .context("The host video format is unavailable")?;
            return Ok(format!(
                "WebRTC connected. Received {} video and a keyframe.",
                codec.name()
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if attempt.control_ready() {
        bail!("WebRTC connected, but no complete video keyframe arrived within 20 seconds");
    }
    bail!("WebRTC connection timed out")
}
