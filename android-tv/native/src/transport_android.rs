//! Android transport lifecycle; codecs and host settings stay shared.
use crate::{
    attempt::{Attempt, Output, StunProvider},
    audio_stream, lowlat_ingress, control, native_wire,
    native_wire::SignalingWrite,
    video_android,
};
use anyhow::{ensure, Result};
use bytes::Bytes;
use evoparseq_lowlat::{
    lowlat::{Event, MediaSource, Route, Session},
    signaling::Signaling,
};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};

pub enum Transport {
    WebRtc(Box<Attempt>),
    Lowlat(Box<Lowlat>),
}
pub struct Lowlat {
    session: Session,
    signaling: Signaling,
    source: MediaSource,
    audio: audio_stream::Pipeline,
    sender: audio_stream::Sender,
    video: Arc<video_android::Pipeline>,
    ingress: lowlat_ingress::VideoState,
    more_media: bool,
    controls: VecDeque<Bytes>,
    started: Instant,
    established: bool,
    network: Arc<crate::network::Network>,
    config: control::Config,
    writer: SignalingWrite,
    peer: String,
    base_id: String,
    wire_id: String,
    routes: Option<VecDeque<Route>>,
    retries: usize,
    audio_settings: crate::audio_preferences::Settings,
    audio_layout: crate::audio_layout::Layout,
}
impl Transport {
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        id: &str,
        peer: &str,
        writer: SignalingWrite,
        config: &control::Config,
        lowlat: bool,
        video: Arc<video_android::Pipeline>,
        network: Arc<crate::network::Network>,
        audio_settings: crate::audio_preferences::Settings,
        audio_layout: crate::audio_layout::Layout,
    ) -> Result<Self> {
        if !lowlat {
            let target = peer.to_owned();
            let offer_id = id.to_owned();
            let output = Output::new(Arc::new(move |description| {
                Ok(description.is_some_and(|d| {
                    writer(native_wire::offer(&target, &offer_id, &d.credentials))
                }))
            }));
            let attempt = Attempt::spawn_configured(
                id,
                output,
                Some(config.clone()),
                StunProvider::Parsec,
                true,
                Some(network),
            )?;
            attempt.start_native_media(video, audio_settings.maximum());
            return Ok(Self::WebRtc(Box::new(attempt)));
        }
        let mut session = lowlat_ingress::session(
            config.allow_hevc,
            config.ten_bit,
            false,
            audio_settings.pcm,
            audio_layout,
        )?;
        session.wake_consumer(std::thread::current());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let (route, routes) = if network.force_relay {
            let mut routes = runtime.block_on(async {
                crate::lowlat_routes::turn_relays(network.servers().await?).await
            })?;
            (
                routes
                    .pop_front()
                    .ok_or_else(|| anyhow::anyhow!("No TURN route"))?,
                Some(routes),
            )
        } else {
            let urls = network
                .stun_urls
                .iter()
                .map(|s| webrtc::ice::url::Url::parse_url(s))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let addresses = runtime.block_on(futures_util::future::join_all(urls.into_iter().map(
                |u| async move {
                    tokio::time::timeout(
                        Duration::from_secs(3),
                        tokio::net::lookup_host((u.host.as_str(), u.port)),
                    )
                    .await
                    .ok()
                    .and_then(|v| v.ok())
                    .map(|v| v.collect::<Vec<_>>())
                    .unwrap_or_default()
                },
            )));
            (
                Route::Direct {
                    stun: addresses.into_iter().flatten().collect(),
                },
                None,
            )
        };
        let credentials = session.offer(id, route)?;
        let source = session.take_media()?;
        let signaling = Signaling::new(peer, id)?;
        ensure!(
            writer(signaling.offer(&credentials).to_string()),
            "Could not send LOWLAT offer"
        );
        let audio =
            audio_stream::Pipeline::start_with_layout(Some(audio_settings.maximum()), audio_layout);
        let sender = audio
            .sender()
            .ok_or_else(|| anyhow::anyhow!("Audio unavailable"))?;
        Ok(Self::Lowlat(Box::new(Lowlat {
            session,
            signaling,
            source,
            audio,
            sender,
            video,
            ingress: lowlat_ingress::VideoState::new(config.allow_hevc, config.ten_bit, false),
            more_media: false,
            controls: VecDeque::new(),
            started: Instant::now(),
            established: false,
            network,
            config: config.clone(),
            writer,
            peer: peer.into(),
            base_id: id.into(),
            wire_id: id.into(),
            routes,
            retries: 0,
            audio_settings,
            audio_layout,
        })))
    }
    pub fn receive(&mut self, text: &str, id: &str) -> Result<()> {
        match self {
            Self::WebRtc(attempt) => ensure!(
                !native_wire::incoming(text, id, attempt)?,
                "The host closed the connection"
            ),
            Self::Lowlat(lowlat) => lowlat.signaling.receive(text, &mut lowlat.session)?,
        }
        Ok(())
    }
    pub fn poll(&mut self, writer: &SignalingWrite, peer: &str, id: &str) -> Result<()> {
        match self {
            Self::WebRtc(attempt) => {
                for _ in 0..64 {
                    let Some(event) = attempt.pop_event() else {
                        break;
                    };
                    if event["type"] == 8 {
                        ensure!(
                            writer(native_wire::candidate(&event, peer, id)),
                            "Could not send network candidate"
                        );
                    }
                }
            }
            Self::Lowlat(lowlat) => {
                for _ in 0..64 {
                    let Some(event) = lowlat.session.poll()? else {
                        break;
                    };
                    if let Some(candidate) = lowlat.signaling.candidate(&event) {
                        ensure!(
                            writer(candidate.to_string()),
                            "Could not send LOWLAT candidate"
                        );
                    }
                    match event {
                        Event::Established { .. } => lowlat.established = true,
                        Event::ExternalControl { bytes } => {
                            ensure!(lowlat.controls.len() < 64, "LOWLAT control queue full");
                            lowlat.controls.push_back(Bytes::from(bytes));
                        }
                        Event::Ended { outcome } => {
                            if !lowlat.established && lowlat.retry()? {
                                break;
                            }
                            anyhow::bail!("LOWLAT connection ended: {outcome:?}")
                        }
                        Event::IoFailure { stage, kind, .. } => {
                            anyhow::bail!("LOWLAT IO failed: {} ({kind:?})", stage.name())
                        }
                        _ => {}
                    }
                }
                ensure!(
                    lowlat.established || lowlat.started.elapsed() < Duration::from_secs(30),
                    "LOWLAT connection timed out"
                );
                let pumped = lowlat_ingress::pump(
                    &mut lowlat.source,
                    &mut lowlat.ingress,
                    lowlat.video.as_ref(),
                    &lowlat.sender,
                )?;
                lowlat.more_media = pumped.more_pending();
            }
        }
        Ok(())
    }
    pub fn more_media_pending(&self) -> bool {
        matches!(self, Self::Lowlat(lowlat) if lowlat.more_media)
    }
    pub fn audio_receiver(&self) -> Option<audio_stream::NativeReceiver> {
        match self {
            Self::WebRtc(a) => a.audio_receiver(),
            Self::Lowlat(b) => Some(b.audio.native_receiver()),
        }
    }
    pub fn control_ready(&self) -> bool {
        match self {
            Self::WebRtc(a) => a.control_ready(),
            Self::Lowlat(b) => b.established,
        }
    }
    pub fn pop_binary(&mut self) -> Option<(u16, bool, Bytes)> {
        match self {
            Self::WebRtc(a) => a.pop_binary(),
            Self::Lowlat(b) => b.controls.pop_front().map(|b| (0, false, b)),
        }
    }
    pub fn send_binary(&mut self, channel: u16, bytes: Bytes) -> Result<()> {
        match self {
            Self::WebRtc(a) => a.send_binary(channel, bytes),
            Self::Lowlat(b) => {
                ensure!(channel == 0, "Invalid LOWLAT control channel");
                b.session.send_frame(&bytes)
            }
        }
    }
    pub fn keyframe(&self, config: &control::Config) -> Result<()> {
        match self {
            Self::WebRtc(a) => a.send_binary(0, config.keyframe_request()),
            Self::Lowlat(b) => {
                ensure!(
                    b.session.request_keyframe_format(false),
                    "LOWLAT keyframe request failed"
                );
                Ok(())
            }
        }
    }
    pub fn host_status(&self) -> Option<i32> {
        match self {
            Self::WebRtc(a) => a.host_status(),
            Self::Lowlat(_) => None,
        }
    }
    pub fn failure_stage(&self) -> Option<crate::attempt::FailureStage> {
        match self {
            Self::WebRtc(a) => a.failure_stage(),
            Self::Lowlat(_) => None,
        }
    }
    pub fn network_diagnostics(&self) -> Value {
        match self {
            Self::WebRtc(a) => a.network_diagnostics(),
            Self::Lowlat(b) => {
                let s = b.session.snapshot();
                json!({"transport":if s.connected { format!("LOWLAT / {}",s.relay_transport.unwrap_or("udp").to_uppercase()) } else { "LOWLAT / connecting".into() }, "state":if s.connected {"Connected"} else {"Connecting"}, "received_bytes":[s.udp_received], "rtt_ms":s.rtt_ms,"route":if !s.connected { "Unverified" } else if s.relay { "Relay — TURN in use" } else { "Direct — no TURN" }})
            }
        }
    }
    pub fn wire_id<'a>(&'a self, fallback: &'a str) -> &'a str {
        match self {
            Self::WebRtc(_) => fallback,
            Self::Lowlat(b) => &b.wire_id,
        }
    }
    pub fn disconnect(&mut self) {
        match self {
            Self::WebRtc(a) => a.disconnect(),
            Self::Lowlat(b) => b.session.close(),
        }
    }
}

impl Lowlat {
    fn retry(&mut self) -> Result<bool> {
        if self.network.provider == "direct" {
            return Ok(false);
        }
        if self.routes.is_none() {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            self.routes = Some(runtime.block_on(async {
                crate::lowlat_routes::turn_relays(self.network.servers().await?).await
            })?);
        }
        let Some(route) = self.routes.as_mut().and_then(|r| r.pop_front()) else {
            return Ok(false);
        };
        (self.writer)(
            native_wire::envelope("offer_cancel", &self.peer, &self.wire_id, json!({})).to_string(),
        );
        self.session.close();
        let mut session = lowlat_ingress::session(
            self.config.allow_hevc,
            self.config.ten_bit,
            false,
            self.audio_settings.pcm,
            self.audio_layout,
        )?;
        session.wake_consumer(std::thread::current());
        self.retries += 1;
        self.wire_id = format!("{}-relay-{}", self.base_id, self.retries);
        let credentials = session.offer(&self.wire_id, route)?;
        self.source = session.take_media()?;
        self.signaling = Signaling::new(&self.peer, &self.wire_id)?;
        ensure!(
            (self.writer)(self.signaling.offer(&credentials).to_string()),
            "Could not send relay offer"
        );
        self.session = session;
        self.started = Instant::now();
        self.controls.clear();
        self.ingress =
            lowlat_ingress::VideoState::new(self.config.allow_hevc, self.config.ten_bit, false);
        self.more_media = false;
        Ok(true)
    }
}

impl lowlat_ingress::VideoTarget for video_android::Pipeline {
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
            "Rotated LOWLAT video is not supported on Android TV yet"
        );
        Ok(self.submit(bytes, info))
    }
}
