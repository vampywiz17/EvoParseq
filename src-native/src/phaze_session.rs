//! Platform-independent Phaze session policy and encoded-video ingress.
use crate::{
    session_preferences::{Resolution, Video},
    video_stream::{Inspector, PacketInfo, Snapshot},
    viewport::Viewport,
};
use anyhow::{ensure, Result};
use evoparseq_phaze::{ClientControl, Input, PeerState, VideoConfig};

pub fn supported_video(mut video: Video) -> Video {
    video.resolution = Resolution::Host;
    video.constant_fps = None;
    video.rotated = None;
    video
}

pub fn normalized(viewport: Option<Viewport>, x: i32, y: i32) -> Option<(i32, i32)> {
    let [left, top, right, bottom] = viewport?.rect()?;
    let axis = |value: i32, start: i32, end: i32| {
        ((i64::from(value) - i64::from(start)) * 65535 / i64::from(end - start - 1).max(1))
            .clamp(0, 65535) as i32
    };
    Some((axis(x, left, right), axis(y, top, bottom)))
}

pub fn button(
    viewport: Option<Viewport>,
    down: bool,
    button: i32,
    x: i32,
    y: i32,
) -> Option<Input> {
    let &button = [1, 3, 2, 4, 5].get(button as usize)?;
    // The Phaze client checks the video bounds on a press. A release can
    // finish a drag outside the video or while its geometry is changing.
    if down {
        let [left, top, right, bottom] = viewport?.rect()?;
        if !(left..right).contains(&x) || !(top..bottom).contains(&y) {
            return None;
        }
    }
    Some(Input::Button { button, x, y, down })
}

pub struct Stream {
    config: VideoConfig,
    pending: Option<(u32, VideoConfig)>,
    sent: Option<VideoConfig>,
    inspector: Inspector,
    payload_bytes: Option<u64>,
}

impl Stream {
    pub fn new(config: VideoConfig) -> Self {
        let mut inspector = Inspector::default();
        inspector.enable_hevc(config.hevc);
        Self {
            config,
            pending: None,
            sent: None,
            inspector,
            payload_bytes: None,
        }
    }

    pub fn config(&self) -> VideoConfig {
        self.config
    }

    pub fn validate_update(&self, video: &Video) -> Result<VideoConfig> {
        video.validate()?;
        ensure!(
            video.resolution == Resolution::Host
                && video.constant_fps.is_none()
                && video.rotated.is_none(),
            "Unsupported Phaze stream option"
        );
        let config = VideoConfig {
            mbps: video.bandwidth_mbps.unwrap_or(self.config.mbps),
            fps: video.fps.unwrap_or(self.config.fps),
            ..self.config
        };
        config.validate()?;
        Ok(config)
    }

    /// Commit only after the client's preference store accepted the update.
    pub fn apply(&mut self, config: VideoConfig, output: Option<u32>) {
        self.config = config;
        if let Some(output) = output {
            self.select(output);
        }
    }

    pub fn select(&mut self, output: u32) {
        self.pending = Some((output, self.config));
    }

    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }

    pub fn take_request(&mut self) -> Option<(u32, VideoConfig)> {
        self.pending.take()
    }

    pub fn request_sent(&mut self, output: u32, config: VideoConfig) {
        self.sent = Some(config);
        crate::event_log::record(
            "INFO",
            "phaze_stream_request_sent",
            serde_json::json!({"output":output,"maximum_mbps":config.mbps,"fps":config.fps}),
        );
    }

    pub fn sent(&self) -> Option<VideoConfig> {
        self.sent
    }

    pub fn prepare_recovery(&mut self, fallback_420: bool) {
        if fallback_420 {
            self.config.chroma_444 = false;
            if let Some((_, config)) = &mut self.pending {
                config.chroma_444 = false;
            }
        }
    }

    pub fn inspect(&mut self, encoded: &[u8], reset: bool) -> Option<PacketInfo> {
        if !encoded.is_empty() {
            let bytes = self.payload_bytes.get_or_insert(0);
            *bytes = bytes.saturating_add(encoded.len() as u64);
        }
        if reset {
            self.inspector = Inspector::default();
            self.inspector.enable_hevc(self.config.hevc);
        }
        self.inspector.receive(encoded)
    }

    pub fn video(&self) -> Snapshot {
        self.inspector.snapshot()
    }

    pub fn payload_bytes(&self) -> Option<u64> {
        self.payload_bytes
    }
}

pub fn log_bandwidth_feedback(lower_rate: f64, upper_rate: f64) {
    crate::event_log::record(
        "INFO",
        "phaze_bandwidth_feedback_started",
        serde_json::json!({"lower_bytes_per_second":lower_rate,"upper_bytes_per_second":upper_rate}),
    );
}

#[derive(Default)]
pub struct PhazePeers {
    pub local_id: Option<u32>,
    pub controller: Option<u32>,
    pub state: PeerState,
}

impl PhazePeers {
    pub fn admin(&self) -> bool {
        self.state
            .guests
            .as_deref()
            .unwrap_or_default()
            .iter()
            .any(|guest| Some(guest.id) == self.local_id && guest.admin)
    }

    pub fn allows(&self, control: &ClientControl) -> bool {
        if !self.admin() {
            return false;
        }
        let guests = self.state.guests.as_deref().unwrap_or_default();
        match control {
            ClientControl::SecureAttention => true,
            ClientControl::SetControl(id) => *id == 0 || guests.iter().any(|guest| guest.id == *id),
            ClientControl::RemoveGuest(id) => {
                Some(*id) != self.local_id && guests.iter().any(|guest| guest.id == *id)
            }
            ClientControl::RespondToOffer { attempt_id, .. } => self
                .state
                .offers
                .as_deref()
                .unwrap_or_default()
                .iter()
                .any(|offer| offer.attempt_id == *attempt_id),
            ClientControl::Clipboard(_) => false,
        }
    }
}

/// Try configured relay servers in order. Once allocated, the Phaze session
/// uses that relay exclusively. The platform supplies its optional TLS roots.
pub async fn connect_relay<S: evoparseq_phaze::Sink>(
    remote: &evoparseq_phaze::Session,
    peer: &str,
    sink: &mut S,
    servers: Vec<webrtc::ice_transport::ice_server::RTCIceServer>,
    native_roots: impl Fn() -> Vec<Vec<u8>>,
    set_carrier: impl Fn(&mut S, &'static str, bool),
) -> Result<()> {
    use evoparseq_phaze::turn::{Carrier, Relay};
    use webrtc::ice::url::{ProtoType, SchemeType, Url};
    for server in servers {
        for raw in server.urls {
            let url = Url::parse_url(&raw)?;
            let (carrier, name) = match (url.scheme, url.proto) {
                (SchemeType::Turn, ProtoType::Udp) => (Carrier::Udp, "TURN / UDP"),
                (SchemeType::Turn, ProtoType::Tcp) => (Carrier::Tcp, "TURN / TCP"),
                (SchemeType::Turns, ProtoType::Tcp) => (Carrier::Tls, "TURN / TLS"),
                _ => continue,
            };
            set_carrier(sink, name, false);
            sink.stage("allocating_turn");
            let trusted_roots: Vec<_> = if carrier == Carrier::Tls {
                native_roots()
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
            set_carrier(sink, name, true);
            let result = evoparseq_phaze::connect_via(remote, peer, sink, &relay).await;
            relay.close().await;
            return result;
        }
    }
    anyhow::bail!(
        "No configured TURN server could allocate a Phaze relay. Check the URL and credentials."
    )
}
