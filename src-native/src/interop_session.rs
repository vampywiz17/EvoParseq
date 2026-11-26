//! Platform-independent Phaze session policy and encoded-video ingress.
use crate::{
    session_preferences::{Resolution, Video},
    video_stream::{Inspector, PacketInfo, Snapshot},
    viewport::Viewport,
};
use anyhow::{ensure, Result};
use evoparseq_host_interop::{ClientControl, Input, PeerState, VideoConfig};

#[cfg(not(target_os = "android"))]
use crate::display_stream::DESKTOP_DISPLAYS;

#[cfg(not(target_os = "android"))]
#[derive(Default)]
pub struct SecondaryDisplays {
    slots: [Option<Secondary>; DESKTOP_DISPLAYS - 1],
}

#[cfg(not(target_os = "android"))]
struct Secondary {
    output: u32,
    video: Stream,
    opened: std::time::Instant,
    started: bool,
    requested: bool,
    closing: bool,
}

#[cfg(not(target_os = "android"))]
impl SecondaryDisplays {
    pub fn available(&self, output: u32, primary: Option<u32>) -> bool {
        primary.is_some() && primary != Some(output) && self.slot(output).is_none()
    }

    pub fn slot(&self, output: u32) -> Option<usize> {
        self.slots
            .iter()
            .position(|slot| slot.as_ref().is_some_and(|screen| screen.output == output))
            .map(|index| index + 1)
    }

    pub fn is_open(&self) -> bool {
        self.slots.iter().any(Option::is_some)
    }

    pub fn vacant(&self) -> Option<usize> {
        self.slots
            .iter()
            .position(Option::is_none)
            .map(|index| index + 1)
    }

    pub fn open(&mut self, slot: usize, output: u32, config: VideoConfig) {
        self.slots[slot - 1] = Some(Secondary {
            output,
            video: Stream::new(config),
            opened: std::time::Instant::now(),
            started: false,
            requested: false,
            closing: false,
        });
    }

    pub fn output(&self, slot: usize) -> Option<u32> {
        let screen = self.slots.get(slot.checked_sub(1)?)?.as_ref()?;
        (!screen.closing).then_some(screen.output)
    }

    pub fn ready(&self, slot: usize) -> bool {
        self.output(slot).is_some()
            && self.slots[slot - 1]
                .as_ref()
                .is_some_and(|screen| screen.started)
    }

    pub fn timed_out(&self, slot: usize) -> bool {
        self.slots[slot - 1].as_ref().is_some_and(|screen| {
            screen.requested
                && !screen.started
                && screen.opened.elapsed() >= std::time::Duration::from_secs(20)
        })
    }

    pub fn close(&mut self, slot: usize) {
        if let Some(screen) = self.slots[slot - 1].as_mut() {
            screen.closing = true;
        }
    }

    pub fn lost(&mut self, output: u32) -> Option<usize> {
        let slot = self.slot(output)?;
        self.slots[slot - 1] = None;
        Some(slot)
    }

    pub fn inspect(&mut self, slot: usize, bytes: &[u8], reset: bool) -> Option<PacketInfo> {
        let screen = self.slots[slot - 1].as_mut()?;
        if screen.closing {
            return None;
        }
        let info = screen.video.inspect(bytes, reset)?;
        screen.started |= info.picture;
        Some(info)
    }

    pub fn command(&mut self) -> Option<evoparseq_host_interop::DisplayCommand> {
        use evoparseq_host_interop::DisplayCommand;
        for slot in &mut self.slots {
            let Some(screen) = slot else { continue };
            if screen.closing {
                let output = screen.output;
                let requested = screen.requested;
                *slot = None;
                if requested {
                    return Some(DisplayCommand::Close(output));
                }
            } else if !screen.requested {
                screen.requested = true;
                screen.opened = std::time::Instant::now();
                return Some(DisplayCommand::Open(screen.output, screen.video.config()));
            }
        }
        None
    }
}

pub fn supported_video(mut video: Video) -> Video {
    video.resolution = Resolution::Host;
    video.constant_fps = None;
    video.rotated = None;
    video
}

pub fn normalized(viewport: Option<Viewport>, x: i32, y: i32) -> Option<(i32, i32)> {
    let [left, top, right, bottom] = viewport?.rect()?;
    Some((
        normalized_axis(i64::from(x) - i64::from(left), (right - left) as u32),
        normalized_axis(i64::from(y) - i64::from(top), (bottom - top) as u32),
    ))
}

pub fn normalized_axis(value: i64, extent: u32) -> i32 {
    (value * 65535 / i64::from(extent.saturating_sub(1).max(1))).clamp(0, 65535) as i32
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

pub fn log_parallel_display_probe(status: &'static str) {
    crate::event_log::record(
        "INFO",
        "phaze_parallel_display_probe",
        serde_json::json!({"status":status}),
    );
}

pub fn log_display_video_header(output: u32, match_mask: u8) {
    crate::event_log::record(
        "INFO",
        "phaze_display_video_header_observed",
        serde_json::json!({"output":output,"matching_word_mask":match_mask}),
    );
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
pub async fn connect_relay<S: evoparseq_host_interop::Sink>(
    remote: &evoparseq_host_interop::Session,
    peer: &str,
    sink: &mut S,
    servers: Vec<webrtc::ice_transport::ice_server::RTCIceServer>,
    native_roots: impl Fn() -> Vec<Vec<u8>>,
    set_carrier: impl Fn(&mut S, &'static str, bool),
) -> Result<()> {
    use evoparseq_host_interop::turn::{Carrier, Relay};
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
                        "interop_turn_allocation_failed",
                        serde_json::json!({"carrier":name,"reason":error.to_string()}),
                    );
                    continue;
                }
            };
            set_carrier(sink, name, true);
            let result = evoparseq_host_interop::connect_via(remote, peer, sink, &relay).await;
            relay.close().await;
            return result;
        }
    }
    anyhow::bail!(
        "No configured TURN server could allocate a Phaze relay. Check the URL and credentials."
    )
}

#[cfg(all(test, not(target_os = "android")))]
#[test]
fn secondary_display_slots_keep_output_identity_until_closed() {
    use evoparseq_host_interop::DisplayCommand;

    let mut displays = SecondaryDisplays::default();
    let config = VideoConfig::default();
    assert!(!displays.available(7, None));
    assert!(!displays.available(6, Some(6)));
    assert!(displays.available(7, Some(6)));
    displays.open(1, 7, config);
    assert!(!displays.available(7, Some(6)));
    assert_eq!(displays.vacant(), Some(2));
    assert!(matches!(
        displays.command(),
        Some(DisplayCommand::Open(7, _))
    ));
    assert!(displays.command().is_none());
    displays.open(2, 9, config);
    displays.open(3, 5, config);
    assert_eq!(displays.vacant(), None);
    assert_eq!(displays.lost(9), Some(2));
    assert_eq!(displays.output(1), Some(7));
    assert_eq!(displays.output(3), Some(5));
    displays.close(1);
    assert_eq!(displays.output(1), None);
    assert!(matches!(displays.command(), Some(DisplayCommand::Close(7))));
    assert_eq!(displays.vacant(), Some(1));
}

#[cfg(all(test, not(target_os = "android")))]
#[test]
fn secondary_video_waits_for_its_own_picture_and_a_closing_slot_discards_frames() {
    use evoparseq_host_interop::DisplayCommand;
    use std::time::{Duration, Instant};

    let mut displays = SecondaryDisplays::default();
    let config = VideoConfig::default();
    assert!(!displays.is_open());
    displays.open(1, 99, config);
    assert!(!displays.ready(1));
    let earlier = Instant::now() - Duration::from_secs(21);
    displays.slots[0].as_mut().unwrap().opened = earlier;
    assert!(
        !displays.timed_out(1),
        "timeout starts when the request is sent"
    );
    assert!(matches!(
        displays.command(),
        Some(DisplayCommand::Open(99, _))
    ));
    assert!(!displays.timed_out(1));
    assert!(displays.inspect(1, &[0], true).is_none());
    assert!(displays.is_open());
    displays.slots[0].as_mut().unwrap().opened = earlier;
    assert!(displays.timed_out(1));
    displays.close(1);
    assert!(displays.inspect(1, &[0], false).is_none());
    assert!(matches!(
        displays.command(),
        Some(DisplayCommand::Close(99))
    ));
    assert!(!displays.is_open());

    displays.open(2, 7, config);
    displays.close(2);
    assert!(
        displays.command().is_none(),
        "an unsent request has nothing to stop"
    );
    assert!(!displays.is_open());
}
