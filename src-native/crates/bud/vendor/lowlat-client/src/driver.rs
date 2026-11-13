//! The session, driven one pass at a time: what a client says when the path
//! comes up, what it does with each channel, and how far behind its reader is.
//!
//! Sans-IO. The shell thread calls [`Driver::turn`] once per pass with the
//! endpoint and the clock; a test calls it with a simulated endpoint and a
//! fake clock. Nothing here reads time, owns a socket or spawns a thread.
//!
//! **The receive ring is this thread's, so the catch-up is too.** Access
//! units are taken off the video channel here and handed to the application
//! through a pool by index; when the reader is more than one message
//! behind, this looks ahead through the messages the ring holds for keyframe
//! metadata whose picture has also arrived and skips to it, so a lagging
//! reader recovers in one step. Nothing is skipped over a gap, and nothing is
//! skipped when no keyframe is ahead (docs/10-client.md section 3).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use lowlat_common::events;
use lowlat_common::pool::{self, Pool};
use lowlat_common::spsc::Ring;
use lowlat_core::audio;
use lowlat_core::channel::Arrivals;
use lowlat_core::control::{self, CONTROL_CHANNEL, Control, op};
use lowlat_core::endpoint::Endpoint;
use lowlat_core::init::{self, Init};
use lowlat_core::relay::{self, Failure, Relay};
use lowlat_core::session::{Health, Session};
use lowlat_core::video::{self, METADATA_LEN, VIDEO_HEADER_LEN};
use lowlat_core::{Error, conn};

use crate::event::{Event, Outcome};
use crate::sound::{self, Packets};
use crate::{AUDIO_CHANNEL, UNIT_BYTES, UNIT_SLOTS, VIDEO_CHANNEL};

/// The longest inbound control message that will be taken: the user-data
/// ceiling plus its header. A longer one cannot be consumed and the channel
/// cannot advance past it, so it ends the session.
const MAX_INBOUND: usize = control::USER_DATA_MAX + control::CONTROL_HEADER_LEN;

/// The streams a peer holds. Only the first is ever sent pictures.
const STREAMS: usize = 3;

/// The pool's tag on a unit that is keyframe metadata rather than a picture.
/// The tag's other 31 bits are the unit's arrival stamp.
pub const TAG_METADATA: u32 = 1;

/// An arrival stamp: microseconds since the loop's epoch in 31 bits, so it
/// wraps every 35 minutes. Only an age is ever read from one, and no unit
/// waits anywhere near that long.
const STAMP_MASK: u32 = 0x7fff_ffff;

/// The stamp for a pass at `now_ms` on the loop's epoch.
pub fn stamp_us(now_ms: f64) -> u32 {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "microseconds since the loop's epoch, wrapping as the stamp does"
    )]
    let us = (now_ms.max(0.0) * 1000.0) as u64;
    u32::try_from(us & u64::from(STAMP_MASK)).unwrap_or(0)
}

/// How long before `now_ms` a stamp was taken, on the same epoch.
pub fn stamp_age_us(now_ms: f64, stamp: u32) -> u32 {
    stamp_us(now_ms).wrapping_sub(stamp) & STAMP_MASK
}

/// How often the client reports its decode times: on the clock, not per
/// picture, so a still desktop still reports and the round-trip estimate,
/// which samples only on acknowledged sends, stays alive.
pub const REPORT_INTERVAL_MS: f64 = 2000.0;

/// The media kinds of a latency report.
const KIND_VIDEO: u32 = 1;
const KIND_AUDIO: u32 = 2;

/// The channels the metrics describe: control, video, sound, by number.
pub const CHANNELS: usize = 3;

/// How often the recent-loss figure takes a sample, and the weight of each
/// sample in it: a thirtieth, so the figure reads over about thirty seconds.
const LOSS_SAMPLE_MS: f64 = 1000.0;
const LOSS_WEIGHT: f64 = 1.0 / 30.0;

/// Received access units, on their way to the decoder.
///
/// One producer (the session's thread) and one consumer. The pool holds the
/// bytes and the ring carries slot indices, so a unit is copied once, off the
/// receive ring, and never again.
#[derive(Debug, Clone)]
pub struct Units {
    #[cfg(windows)]
    secondary: Arc<std::sync::Mutex<[Option<Units>; 2]>>,
    pool: Arc<Pool>,
    ring: Arc<Ring<u32, UNIT_SLOTS>>,
    /// The consumer's wait word: bumped on every unit handed over.
    word: Arc<AtomicU32>,
}

impl Default for Units {
    fn default() -> Self {
        Self::new()
    }
}

impl Units {
    pub fn new() -> Self {
        Self {
            #[cfg(windows)]
            secondary: Default::default(),
            pool: Arc::new(Pool::new(UNIT_SLOTS, UNIT_BYTES)),
            ring: Arc::new(Ring::new()),
            word: Arc::new(AtomicU32::new(0)),
        }
    }

    #[cfg(windows)]
    pub fn secondary(&self, stream: usize, create: bool) -> Option<Self> {
        let mut streams = self.secondary.lock().unwrap_or_else(|e| e.into_inner());
        let slot = streams.get_mut(stream.checked_sub(1)?)?;
        if create && slot.is_none() {
            *slot = Some(Self::new());
        }
        slot.clone()
    }

    /// Wait up to `timeout` for a unit to be handed over, or for a wake.
    /// The consumer's; it rechecks `take` after.
    pub fn wait(&self, timeout: core::time::Duration) {
        let word = self.word.load(Ordering::Acquire);
        if self.ring.is_empty() {
            lowlat_common::wait::wait(&self.word, word, timeout);
        }
    }

    /// Wake the consumer without a unit: teardown, or something else to
    /// look at.
    pub fn wake(&self) {
        self.word.fetch_add(1, Ordering::Release);
        lowlat_common::wait::notify_all(&self.word);
    }

    /// The next unit, in order, or `None` while nothing waits. Held until
    /// dropped; the slot goes back then.
    pub fn take(&self) -> Option<Unit<'_>> {
        let index = self.ring.pop()?;
        let frame = self.pool.claim(index)?;
        Some(Unit { frame })
    }

    /// Units handed over and not yet taken.
    pub fn queued(&self) -> usize {
        self.ring.len()
    }

    /// Let go of every unit handed over and not yet taken: they belong to a
    /// stream that has ended. The consumer's side, taken while no consumer
    /// runs.
    pub fn clear(&self) {
        while self.take().is_some() {}
    }

    /// Hand `content` over as the session's thread does, for the decode
    /// thread's tests.
    #[cfg(test)]
    pub(crate) fn hand_over(&self, content: &[u8]) {
        let mut writer = self.pool.acquire().expect("a free slot");
        assert!(writer.fill_with(|slot| {
            slot[..content.len()].copy_from_slice(content);
            Some(content.len())
        }));
        assert_eq!(writer.publish(0, &[&self.ring]), 1);
        self.word.fetch_add(1, Ordering::Release);
        lowlat_common::wait::notify_one(&self.word);
    }
}

/// One access unit, header included, as it came off the wire.
#[derive(Debug)]
pub struct Unit<'a> {
    frame: pool::Frame<'a>,
}

impl Unit<'_> {
    /// The message content: the ten-byte video header and the bitstream.
    pub fn bytes(&self) -> &[u8] {
        self.frame.bytes()
    }

    /// When the unit was taken off the wire: the stamp of the pass that
    /// completed it.
    pub fn stamp(&self) -> u32 {
        self.frame.tag() >> 1
    }

    /// Keyframe metadata rather than a picture.
    pub fn metadata(&self) -> bool {
        self.frame.tag() & TAG_METADATA != 0
    }
}

/// How far behind the reader is: what the deferred decisions are decided on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Lag {
    /// Messages arrived and not yet consumed: waiting in the receive ring plus
    /// handed over and not yet taken.
    pub behind: u32,
    /// How long there has been something unconsumed, in milliseconds.
    pub behind_ms: u32,
}

/// What the loop publishes about itself, for the seam to read from any thread.
#[derive(Debug, Default)]
pub struct Telemetry {
    #[cfg(feature = "usb-hid-input")]
    pub hid_mouse_active: AtomicBool,
    #[cfg(feature = "usb-hid-input")]
    pub hid_mouse_error: AtomicU32,
    #[cfg(feature = "usb-hid-input")]
    pub hid_keyboard_active: AtomicBool,
    #[cfg(feature = "usb-hid-input")]
    pub hid_keyboard_error: AtomicU32,
    /// UDP payload bytes, including protocol/crypto/relay traffic.
    pub udp_received: AtomicU64,
    pub udp_sent: AtomicU64,
    /// The seam's state word: 0 connecting, 1 established, 2 over.
    pub state: AtomicU32,
    /// The status the host's disconnect carried, or zero.
    pub disconnect: AtomicU32,
    pub pictures: AtomicU64,
    pub metadata: AtomicU64,
    pub skipped: AtomicU64,
    pub video_bytes: AtomicU64,
    pub audio_packets: AtomicU64,
    pub audio_bytes: AtomicU64,
    pub control_in: AtomicU64,
    pub control_out: AtomicU64,
    pub behind: AtomicU32,
    pub behind_ms: AtomicU32,
    pub rtt_ms: AtomicU32,
    /// Set by the decoder's consumer when its feed asked for a keyframe; the
    /// loop sends the request on its next pass and clears it.
    pub request: AtomicBool,
    pub fallback_420: AtomicBool,
    /// The declaration: what the application asked, and what went out after
    /// the mask; and the stream as decoded, the layout's code or zero.
    pub asked_flags: AtomicU32,
    pub declared_flags: AtomicU32,
    pub stream_format: AtomicU32,
    /// The host's own encode time for the stream, as it last reported it,
    /// in microseconds.
    pub encode_us: AtomicU32,
    /// Input reports refused because the ring to the session thread was
    /// full, counted where they were dropped.
    pub input_dropped: AtomicU32,
    /// Compressed audio packets dropped because the pool is full.
    pub audio_dropped: AtomicU32,
    /// This client's number on the host's roster; zero until told.
    pub number: AtomicU32,
    /// Per channel, what a receiver can count (docs/10-client.md section 9):
    /// fragments accepted, those that arrived behind a later one, duplicates
    /// and out-of-window drops, the negatives sent, bytes and messages taken
    /// off the channel, and the recent-loss figure as the bits of an `f32`.
    pub fragments: [AtomicU64; CHANNELS],
    pub late: [AtomicU64; CHANNELS],
    pub duplicates: [AtomicU64; CHANNELS],
    pub out_of_window: [AtomicU64; CHANNELS],
    pub nacks_sent: [AtomicU64; CHANNELS],
    pub bytes: [AtomicU64; CHANNELS],
    pub messages: [AtomicU64; CHANNELS],
    pub loss_30s: [AtomicU32; CHANNELS],
    /// How long the session has been established, in milliseconds.
    pub connected_ms: AtomicU32,
    /// The relayed address a relay attempt offered, as [`pack_relayed`]
    /// writes it; zero until the relay has one, and for a direct attempt.
    pub relayed: AtomicU64,
    /// Whether the path goes through the relay.
    pub path_relayed: AtomicBool,
    /// Established relay carrier: 0 UDP, 1 TCP, 2 TLS.
    pub relay_transport: AtomicU32,
}

/// A relayed address in one word, so it is published without a lock: the
/// IPv4 address in the low thirty-two bits, the port above it, and bit 48 set
/// so that no address at all reads as zero. The relayed family is IPv4.
pub fn pack_relayed(addr: std::net::SocketAddrV4) -> u64 {
    u64::from(addr.ip().to_bits()) | (u64::from(addr.port()) << 32) | (1 << 48)
}

/// The address [`pack_relayed`] wrote, if it wrote one.
pub fn unpack_relayed(word: u64) -> Option<std::net::SocketAddrV4> {
    if word & (1 << 48) == 0 {
        return None;
    }
    let ip = u32::try_from(word & 0xFFFF_FFFF).ok()?;
    let port = u16::try_from((word >> 32) & 0xFFFF).ok()?;
    Some(std::net::SocketAddrV4::new(
        std::net::Ipv4Addr::from_bits(ip),
        port,
    ))
}

/// What a failed relay ends the session with.
fn relay_outcome(relay: &Relay<'_>) -> Option<Outcome> {
    match relay.state() {
        relay::State::Failed(Failure::Unreachable) => Some(Outcome::RelayUnreachable),
        relay::State::Failed(Failure::Refused) => Some(Outcome::RelayRefused),
        relay::State::Failed(_) => Some(Outcome::RelayLost),
        _ => None,
    }
}

/// One session's driver.
#[derive(Debug)]
pub struct Driver {
    /// Successful application enqueues since the shell last drained output.
    output_queued: bool,
    #[cfg(feature = "usb-hid-input")]
    hid_device: Option<crate::usb_hid::Device>,
    #[cfg(feature = "usb-hid-input")]
    hid_mouse_enabled: bool,
    #[cfg(feature = "usb-hid-input")]
    hid_keyboard_enabled: bool,
    init: Init,
    units: Units,
    emit: events::Sender<Event>,
    telemetry: Arc<Telemetry>,
    established: bool,
    stalled_said: bool,
    /// When the departure went out, if it has.
    leaving: Option<f64>,
    /// When the next latency report is due.
    report_due_ms: Option<f64>,
    inbound: Vec<u8>,
    /// Sound packets on their way to the application, and the scratch a
    /// packet is taken into when the pool has no room for it.
    packets: Packets,
    dropped_sound: Vec<u8>,
    /// What this peer has sent and been sent, one count per opcode.
    received: [u32; 256],
    sent: [u32; 256],
    /// The generation the host announced per stream, and its encode time.
    generation: [Option<u32>; STREAMS],
    encode_latency_us: [u32; STREAMS],
    /// This client's number on the host's roster, once told.
    number: Option<u32>,
    /// Since when the reader has had something unconsumed.
    behind_since: Option<f64>,
    lag: Lag,
    pictures: u64,
    metadata: u64,
    skipped: u64,
    video_bytes: u64,
    audio_packets: u64,
    audio_bytes: u64,
    audio_dropped: u32,
    first_audio_said: bool,
    /// Bytes taken off the control channel; the other two channels' are
    /// counted where their messages are taken.
    control_bytes: u64,
    /// When the session was established, and the recent-loss sampler: when
    /// it last sampled, the arrivals it saw then, and the average per channel.
    established_ms: f64,
    loss_sampled_ms: f64,
    loss_seen: [Arrivals; CHANNELS],
    loss_ewma: [f64; CHANNELS],
}

impl Driver {
    pub fn new(
        init: Init,
        units: Units,
        packets: Packets,
        emit: events::Sender<Event>,
        telemetry: Arc<Telemetry>,
    ) -> Self {
        Self {
            output_queued: false,
            #[cfg(feature = "usb-hid-input")]
            hid_device: None,
            #[cfg(feature = "usb-hid-input")]
            hid_mouse_enabled: false,
            #[cfg(feature = "usb-hid-input")]
            hid_keyboard_enabled: false,
            init,
            units,
            packets,
            emit,
            telemetry,
            established: false,
            stalled_said: false,
            leaving: None,
            report_due_ms: None,
            inbound: vec![0u8; MAX_INBOUND],
            dropped_sound: vec![0u8; sound::PACKET_BYTES],
            received: [0; 256],
            sent: [0; 256],
            generation: [None; STREAMS],
            encode_latency_us: [0; STREAMS],
            number: None,
            behind_since: None,
            lag: Lag::default(),
            pictures: 0,
            metadata: 0,
            skipped: 0,
            video_bytes: 0,
            audio_packets: 0,
            audio_bytes: 0,
            audio_dropped: 0,
            first_audio_said: false,
            control_bytes: 0,
            established_ms: 0.0,
            loss_sampled_ms: 0.0,
            loss_seen: [Arrivals::default(); CHANNELS],
            loss_ewma: [0.0; CHANNELS],
        }
    }

    pub fn established(&self) -> bool {
        self.established
    }

    pub(crate) fn take_output_queued(&mut self) -> bool {
        std::mem::take(&mut self.output_queued)
    }

    /// Messages received, one count per opcode.
    pub fn received(&self) -> &[u32; 256] {
        &self.received
    }

    /// Messages sent, one count per opcode.
    pub fn sent(&self) -> &[u32; 256] {
        &self.sent
    }

    pub fn lag(&self) -> Lag {
        self.lag
    }

    pub fn skipped(&self) -> u64 {
        self.skipped
    }

    pub fn pictures(&self) -> u64 {
        self.pictures
    }

    pub fn audio_packets(&self) -> u64 {
        self.audio_packets
    }

    pub fn generation(&self, stream: usize) -> Option<u32> {
        self.generation.get(stream).copied().flatten()
    }

    pub fn encode_latency_us(&self, stream: usize) -> u32 {
        self.encode_latency_us.get(stream).copied().unwrap_or(0)
    }

    /// One pass. `Some` is the end of the session, and the last thing this
    /// driver decides.
    pub fn turn(
        &mut self,
        endpoint: &mut Endpoint<'_, Session<'_>>,
        now_ms: f64,
    ) -> Option<Outcome> {
        // The relay's end is its own, typed, and comes before the punch's:
        // an attempt whose relay failed can only time out after it.
        if let Some(outcome) = endpoint.relay().and_then(relay_outcome) {
            lowlat_common::log_info!("client: the relay ended the attempt, outcome={outcome:?}");
            return Some(outcome);
        }
        match endpoint.conn().state() {
            conn::State::Established(addr) if !self.established => {
                // The pair the session will claim for its life, both halves:
                // a destination with a source it cannot reach is otherwise
                // visible only as every datagram refused.
                match endpoint.conn().local() {
                    Some(from) => {
                        lowlat_common::log_info!("client: path established, to={addr} from={from}");
                    }
                    None => {
                        lowlat_common::log_info!("client: path established, to={addr} from=any")
                    }
                }
                self.established = true;
                self.established_ms = now_ms;
                self.telemetry
                    .path_relayed
                    .store(endpoint.relay().is_some(), Ordering::Relaxed);
                self.loss_sampled_ms = now_ms;
                self.telemetry.state.store(1, Ordering::Relaxed);
                self.start(endpoint.session());
                #[cfg(feature = "usb-hid-input")]
                {
                    self.hid_device = match (self.hid_mouse_enabled, self.hid_keyboard_enabled) {
                        (true, true) => {
                            Some(crate::usb_hid::Device::new(crate::usb_hid::Kind::Both))
                        }
                        (true, false) => Some(crate::usb_hid::Device::default()),
                        (false, true) => {
                            Some(crate::usb_hid::Device::new(crate::usb_hid::Kind::Keyboard))
                        }
                        (false, false) => None,
                    };
                }
                self.report_due_ms = Some(now_ms + REPORT_INTERVAL_MS);
                self.emit.send(Event::Established { addr });
            }
            conn::State::Failed(failure) => {
                lowlat_common::log_info!("client: punch failed, outcome={failure:?}");
                return Some(Outcome::ConnectivityFailed);
            }
            _ => {}
        }
        if !self.established {
            return None;
        }

        #[cfg(feature = "usb-hid-input")]
        {
            let frame = self.hid_device.as_mut().and_then(|d| d.tick(now_ms));
            self.hid_send(endpoint.session(), frame);
            let (active, error) = self
                .hid_device
                .as_mut()
                .map_or((false, 0), |d| (d.active(), d.error()));
            for (enabled, a, e) in [
                (
                    self.hid_mouse_enabled,
                    &self.telemetry.hid_mouse_active,
                    &self.telemetry.hid_mouse_error,
                ),
                (
                    self.hid_keyboard_enabled,
                    &self.telemetry.hid_keyboard_active,
                    &self.telemetry.hid_keyboard_error,
                ),
            ] {
                a.store(enabled && active, Ordering::Release);
                e.store(if enabled { error } else { 0 }, Ordering::Release);
            }
        }

        if let Some(outcome) = self.check_health(endpoint, now_ms) {
            return Some(outcome);
        }
        if self.telemetry.request.swap(false, Ordering::AcqRel) {
            if self.telemetry.fallback_420.load(Ordering::Acquire) {
                self.init.flags &= !init::FLAG_COLOR444;
                self.telemetry
                    .declared_flags
                    .store(self.init.flags, Ordering::Release);
            }
            self.request_keyframe(endpoint.session());
        }
        if self.report_due_ms.is_some_and(|due| now_ms >= due) {
            self.report(endpoint.session());
            self.report_due_ms = Some(now_ms + REPORT_INTERVAL_MS);
        }
        if let Some(outcome) = self.drain_control(endpoint.session()) {
            return Some(outcome);
        }
        if let Some(outcome) = self.drain_video(endpoint.session(), now_ms) {
            return Some(outcome);
        }
        #[cfg(windows)]
        if let Some(outcome) = self.drain_secondary(endpoint.session(), now_ms) {
            return Some(outcome);
        }
        if let Some(outcome) = self.drain_audio(endpoint.session(), now_ms) {
            return Some(outcome);
        }
        self.measure_lag(endpoint.session(), now_ms);
        self.sample_loss(endpoint.session(), now_ms);
        self.publish(endpoint.session(), now_ms);
        None
    }

    /// Once a second, fold each channel's late arrivals over its arrivals
    /// since the last sample into the recent-loss average.
    fn sample_loss(&mut self, session: &Session<'_>, now_ms: f64) {
        if now_ms - self.loss_sampled_ms < LOSS_SAMPLE_MS {
            return;
        }
        self.loss_sampled_ms = now_ms;
        for (channel, (seen, ewma)) in self
            .loss_seen
            .iter_mut()
            .zip(self.loss_ewma.iter_mut())
            .enumerate()
        {
            let now = session
                .recv_arrivals(u8::try_from(channel).unwrap_or(u8::MAX))
                .unwrap_or_default();
            let fragments = now.fragments.saturating_sub(seen.fragments);
            let late = now.late.saturating_sub(seen.late);
            *seen = now;
            // Nothing arrived, nothing was lost: a quiet channel reads as a
            // clean one rather than holding its last figure for ever.
            #[allow(clippy::cast_precision_loss, reason = "counts per second")]
            let sample = if fragments == 0 {
                0.0
            } else {
                (late as f64 / fragments as f64).min(1.0)
            };
            *ewma += (sample - *ewma) * LOSS_WEIGHT;
        }
    }

    /// The departure: opcode 10 with a zero status, which is what a client
    /// sends when it leaves cleanly. The session stays up for a grace so the
    /// message reaches the host; [`Driver::left`] says when that is over.
    pub fn leave(&mut self, session: &mut Session<'_>, now_ms: f64) {
        if self.leaving.is_none() {
            #[cfg(feature = "usb-hid-input")]
            self.finish_hid_input(session);
            self.send_control(
                session,
                &Control {
                    a0: 0,
                    a1: 0,
                    a2: 0,
                    opcode: op::DISCONNECT,
                    body: &[],
                },
            );
            self.leaving = Some(now_ms);
        }
    }

    /// True once a departure has had its grace.
    pub fn left(&self, now_ms: f64) -> bool {
        self.leaving
            .is_some_and(|at| now_ms - at >= crate::event::LEAVE_GRACE_MS)
    }

    /// An application message for the host.
    pub fn send_user_data(&mut self, session: &mut Session<'_>, id: u32, text: &[u8]) -> bool {
        let len = control::string_body_len(text.len());
        if len > control::USER_DATA_MAX {
            return false;
        }
        let mut header = [0u8; control::CONTROL_HEADER_LEN];
        let control = Control {
            a0: u32::try_from(len).unwrap_or(u32::MAX),
            a1: id,
            a2: 0,
            opcode: op::USER_DATA,
            body: &[],
        };
        if control::encode_header(&mut header, &control).is_err() {
            return false;
        }
        // The body is the text and its terminator, which the message framing
        // takes as two slices; the terminator rides as a one-byte tail.
        let mut body = Vec::with_capacity(len);
        body.extend_from_slice(text);
        body.push(0);
        let sent = session
            .send_message(CONTROL_CHANNEL, &header, &body)
            .is_ok();
        if sent {
            self.count_sent(op::USER_DATA);
        }
        sent
    }

    /// What a client says when the path comes up, in order: the
    /// initialization, the diagnostics opcode with every bit clear, and the
    /// declaration for each secondary stream. Stream 0 declares through the
    /// initialization and sends no declaration of its own; nothing waits on an
    /// acknowledgement.
    fn start(&mut self, session: &mut Session<'_>) {
        let mut body = [0u8; 512];
        let Ok(len) = init::encode(&mut body, &self.init) else {
            return;
        };
        let init = Control {
            a0: u32::try_from(len).unwrap_or(u32::MAX),
            a1: 0,
            a2: 0,
            opcode: op::INIT,
            body: body.get(..len).unwrap_or(&[]),
        };
        self.send_control(session, &init);
        self.send_control(
            session,
            &Control {
                a0: 0,
                a1: 0,
                a2: 0,
                opcode: op::DIAGNOSTICS,
                body: &[],
            },
        );
        for stream in [2, 1] {
            self.send_control(
                session,
                &Control {
                    a0: stream,
                    a1: self.init.flags,
                    a2: 0,
                    opcode: op::ENCODER_CONFIG,
                    body: &[],
                },
            );
        }
    }

    /// The one request a client makes of a host: a keyframe for stream 0,
    /// after its decoder was torn down. An encoder rebuild on an established
    /// host, so it is sent only when the feed decided it must be.
    fn request_keyframe(&mut self, session: &mut Session<'_>) {
        self.send_control(
            session,
            &Control {
                a0: 0,
                a1: self.init.flags,
                a2: 1,
                opcode: op::ENCODER_CONFIG,
                body: &[],
            },
        );
    }

    /// The decode times, both kinds, in the argument order a client uses:
    /// the figure first, then the kind. Zero until something has been timed,
    /// and sent anyway.
    fn report(&mut self, session: &mut Session<'_>) {
        // No SDK decoder is timed in this seam. Preserve the initial zero
        // reports and their protocol cadence; do not invent decode timings.
        for (us, kind) in [(0, KIND_VIDEO), (0, KIND_AUDIO)] {
            self.send_control(
                session,
                &Control {
                    a0: us,
                    a1: kind,
                    a2: 0,
                    opcode: op::ENCODE_LATENCY,
                    body: &[],
                },
            );
        }
    }

    pub(crate) fn send_external(&mut self, session: &mut Session<'_>, bytes: &[u8]) {
        if let Ok(mut message) = control::parse(bytes) {
            #[cfg(feature = "usb-hid-input")]
            if self.route_hid_input(&message) {
                return;
            }
            #[cfg(windows)]
            if message.opcode == 27 && message.a1 != 0 && (1..3).contains(&message.a0) {
                // Allocate an additional compressed pool only on an explicit
                // application request. Other platforms retain one stream.
                let _ = self.units.secondary(message.a0 as usize, true);
            }
            if message.opcode == op::ENCODER_CONFIG
                && (message.a0 == 0 || (cfg!(windows) && message.a0 < 3))
            {
                message.a1 = negotiated_encoder_flags(message.a1, self.init.flags);
            }
            self.send_control(session, &message);
        }
    }

    #[cfg(feature = "usb-hid-input")]
    pub(crate) fn enable_hid_input(&mut self, mouse: bool, keyboard: bool) {
        self.hid_mouse_enabled = mouse;
        self.hid_keyboard_enabled = keyboard;
    }
    #[cfg(feature = "usb-hid-input")]
    fn route_hid_input(&mut self, message: &Control<'_>) -> bool {
        let selected = (crate::usb_hid::mouse_opcode(message.opcode) && self.hid_mouse_enabled)
            || (message.opcode == op::KEYBOARD && self.hid_keyboard_enabled);
        if selected || message.opcode == op::RELEASE {
            if let Some(device) = &mut self.hid_device {
                device.external_input(message.opcode, message.a0, message.a1, message.a2);
            }
        }
        selected
    }
    #[cfg(feature = "usb-hid-input")]
    fn hid_send(&mut self, session: &mut Session<'_>, frame: Option<[u8; crate::usb_hid::FRAME]>) {
        if let Some(frame) = frame {
            let (header, body) = frame.split_at(13);
            let success = session.send_message(CONTROL_CHANNEL, header, body).is_ok();
            self.output_queued |= success;
            let error = if let Some(device) = self.hid_device.as_mut() {
                device.sent(success);
                device.error()
            } else {
                0
            };
            if self.hid_mouse_enabled {
                self.telemetry
                    .hid_mouse_error
                    .store(error, Ordering::Release);
            }
            if self.hid_keyboard_enabled {
                self.telemetry
                    .hid_keyboard_error
                    .store(error, Ordering::Release);
            }
        }
    }
    #[cfg(feature = "usb-hid-input")]
    pub(crate) fn finish_hid_input(&mut self, session: &mut Session<'_>) {
        self.telemetry
            .hid_mouse_active
            .store(false, Ordering::Release);
        self.telemetry
            .hid_keyboard_active
            .store(false, Ordering::Release);
        // A combined device may have two held reports. Drain both before removal.
        for _ in 0..2 {
            let release = self.hid_device.as_mut().and_then(|d| d.release_input());
            if release.is_none() {
                break;
            }
            self.hid_send(session, release);
        }
        let remove = self.hid_device.as_mut().and_then(|d| d.remove());
        self.hid_send(session, remove);
    }

    fn send_control(&mut self, session: &mut Session<'_>, control: &Control<'_>) {
        let mut header = [0u8; control::CONTROL_HEADER_LEN];
        if control::encode_header(&mut header, control).is_err() {
            return;
        }
        if session
            .send_message(CONTROL_CHANNEL, &header, control.body)
            .is_ok()
        {
            self.output_queued = true;
            self.count_sent(control.opcode);
        }
    }

    fn count_sent(&mut self, opcode: u8) {
        if let Some(count) = self.sent.get_mut(usize::from(opcode)) {
            *count = count.saturating_add(1);
        }
    }

    fn check_health(
        &mut self,
        endpoint: &Endpoint<'_, Session<'_>>,
        now_ms: f64,
    ) -> Option<Outcome> {
        match endpoint.health(now_ms) {
            Health::Dead => Some(Outcome::PeerGone),
            Health::Undeliverable => Some(Outcome::Undeliverable),
            Health::Stalled => {
                if !self.stalled_said {
                    self.stalled_said = true;
                    lowlat_common::log_warn!(
                        "client: stalled, nothing received for {:.0} ms",
                        lowlat_core::session::LIVENESS_SOFT_MS
                    );
                }
                None
            }
            Health::Alive => {
                self.stalled_said = false;
                None
            }
        }
    }

    /// Everything the host said on the control channel.
    ///
    /// **A take that fails is terminal.** The channel only advances when a
    /// message is consumed, so a message that cannot be would be read again
    /// on every pass for ever.
    fn drain_control(&mut self, session: &mut Session<'_>) -> Option<Outcome> {
        // Borrowed out for the pass so the handler below may take the rest
        // of the driver; the buffer is put back on every way out.
        let mut inbound = core::mem::take(&mut self.inbound);
        let outcome = self.drain_control_into(session, &mut inbound);
        self.inbound = inbound;
        outcome
    }

    fn drain_control_into(
        &mut self,
        session: &mut Session<'_>,
        inbound: &mut [u8],
    ) -> Option<Outcome> {
        loop {
            let len = match session.take_message(CONTROL_CHANNEL, inbound) {
                None => return None,
                Some(Ok(len)) => len,
                Some(Err(error)) => {
                    lowlat_common::log_warn!("client: control message refused, error={error:?}");
                    return Some(Outcome::Unreadable);
                }
            };
            self.control_bytes = self.control_bytes.saturating_add(len as u64);
            let Ok(message) = control::parse(inbound.get(..len).unwrap_or(&[])) else {
                continue;
            };
            if let Some(count) = self.received.get_mut(usize::from(message.opcode)) {
                if *count == 0 {
                    lowlat_common::log_info!(
                        "client: first op={} ({}) a0={} a1={} a2={} body={}",
                        message.opcode,
                        op::name(message.opcode),
                        message.a0,
                        message.a1,
                        message.a2,
                        message.body.len()
                    );
                }
                *count = count.saturating_add(1);
            }
            #[cfg(feature = "usb-hid-input")]
            {
                let frame = self
                    .hid_device
                    .as_mut()
                    .and_then(|d| d.incoming(inbound.get(..len).unwrap_or_default()));
                self.hid_send(session, frame);
            }
            {
                self.emit.send(Event::ExternalControl {
                    bytes: inbound.get(..len).unwrap_or_default().to_vec(),
                });
            }
            if let Some(outcome) = self.on_control(&message) {
                return Some(outcome);
            }
        }
    }

    /// Session-owned control only. Original frames are forwarded to the
    /// retained application UI without cursor/input/gamepad reprocessing.
    fn on_control(&mut self, message: &Control<'_>) -> Option<Outcome> {
        match message.opcode {
            op::DISCONNECT => {
                let status = message.a0 as i32;
                self.telemetry
                    .disconnect
                    .store(message.a0, Ordering::Relaxed);
                if status != 0 {
                    return Some(Outcome::Disconnected(status));
                }
            }
            op::ENCODE_LATENCY => {
                // Only kind 1 reports video encode time. Audio (kind 2) also
                // names stream 0 and must not overwrite the video's figure.
                // Zero carries no usable timing; retain this connection's
                // last real video report instead of making it flicker.
                if message.a0 == 1
                    && message.a1 != 0
                    && let Some(slot) = self.encode_latency_us.get_mut(message.a2 as usize)
                {
                    *slot = message.a1;
                }
            }
            op::ENCODER_GENERATION => {
                if let Some(slot) = self.generation.get_mut(message.a0 as usize) {
                    *slot = Some(message.a1);
                }
            }
            op::GUEST_LIST => {
                self.number = Some(message.a1);
                self.telemetry.number.store(message.a1, Ordering::Relaxed);
            }
            _ => {}
        }
        None
    }

    #[cfg(windows)]
    fn drain_secondary(&mut self, session: &mut Session<'_>, now_ms: f64) -> Option<Outcome> {
        for stream in 1..3 {
            let Some(units) = self.units.secondary(stream, false) else {
                continue;
            };
            let channel = (stream + 2) as u8;
            // Bounded per-pass work; never starve the primary, sound or input.
            for _ in 0..UNIT_SLOTS {
                let pending = session.pending_messages(channel);
                if pending == 0 {
                    break;
                }
                if pending > 1 {
                    self.catch_up(session, pending, channel);
                }
                let Some(mut writer) = units.pool.acquire() else {
                    break;
                };
                let mut refused = false;
                let filled = writer.fill_with(|slot| match session.take_message(channel, slot) {
                    Some(Ok(len)) => Some(len),
                    Some(Err(_)) => {
                        refused = true;
                        None
                    }
                    None => None,
                });
                if refused {
                    return Some(Outcome::Unreadable);
                }
                if !filled {
                    break;
                }
                let metadata = video::parse(writer.written()).is_ok_and(|h| h.metadata);
                let tag = (stamp_us(now_ms) << 1) | if metadata { TAG_METADATA } else { 0 };
                let _ = writer.publish(tag, &[&units.ring]);
                units.word.fetch_add(1, Ordering::Release);
                lowlat_common::wait::notify_one(&units.word);
            }
        }
        None
    }

    fn drain_video(&mut self, session: &mut Session<'_>, now_ms: f64) -> Option<Outcome> {
        loop {
            let pending = session.pending_messages(VIDEO_CHANNEL);
            if pending == 0 {
                return None;
            }
            if pending > 1 {
                self.catch_up(session, pending, VIDEO_CHANNEL);
            }
            let mut writer = self.units.pool.acquire()?;
            let mut refused: Option<Error> = None;
            let filled = writer.fill_with(|slot| match session.take_message(VIDEO_CHANNEL, slot) {
                Some(Ok(len)) => Some(len),
                Some(Err(error)) => {
                    refused = Some(error);
                    None
                }
                None => None,
            });
            if let Some(error) = refused {
                lowlat_common::log_warn!("client: video message refused, error={error:?}");
                return Some(Outcome::Unreadable);
            }
            if !filled {
                return None;
            }
            let content = writer.written();
            let len = content.len();
            let header = video::parse(content).ok();
            let is_metadata = header.is_some_and(|header| header.metadata);
            let tag = (stamp_us(now_ms) << 1) | if is_metadata { TAG_METADATA } else { 0 };
            // **The pool never refuses here.** The ring is as deep as the
            // pool has slots, so a slot that was free has a place in it.
            if writer.publish(tag, &[&self.units.ring]) == 0 {
                self.skipped = self.skipped.saturating_add(1);
                continue;
            }
            self.units.word.fetch_add(1, Ordering::Release);
            lowlat_common::wait::notify_one(&self.units.word);
            self.video_bytes = self.video_bytes.saturating_add(len as u64);
            if is_metadata {
                self.metadata = self.metadata.saturating_add(1);
            } else {
                self.pictures = self.pictures.saturating_add(1);
            }
        }
    }

    /// The keyframe-aligned catch-up over messages that have arrived.
    ///
    /// Reads the head of each pending message for keyframe metadata whose
    /// keyframe is the next message and has arrived whole, takes the last
    /// such, and discards everything before it. Nothing else is ever
    /// skipped: a reader behind by a backlog of predicted pictures decodes
    /// them in order, because none of them can be skipped to.
    fn catch_up(&mut self, session: &mut Session<'_>, pending: u32, channel: u8) {
        let mut head = [0u8; METADATA_LEN];
        let mut target: Option<u32> = None;
        let mut pictures_before = 0u32;
        let mut pictures_scanned = 0u32;
        for n in 0..pending {
            let Some(len) = session.peek_message(channel, n, &mut head) else {
                break;
            };
            let Ok(header) = video::parse(head.get(..len).unwrap_or(&[])) else {
                pictures_scanned = pictures_scanned.saturating_add(1);
                continue;
            };
            if !header.metadata {
                pictures_scanned = pictures_scanned.saturating_add(1);
                continue;
            }
            let announces_keyframe = video::parse_metadata(head.get(..len).unwrap_or(&[]))
                .is_ok_and(|metadata| metadata.keyframe);
            // The picture it announces must be the next message, complete,
            // and a picture: a second metadata message there is a keyframe
            // that has not arrived.
            let mut next = [0u8; VIDEO_HEADER_LEN];
            let next_is_picture = n + 1 < pending
                && session
                    .peek_message(channel, n + 1, &mut next)
                    .is_some_and(|len| {
                        video::parse(next.get(..len).unwrap_or(&[]))
                            .is_ok_and(|header| !header.metadata)
                    });
            if announces_keyframe && next_is_picture && n > 0 {
                target = Some(n);
                pictures_before = pictures_scanned;
            }
        }
        if let Some(n) = target {
            let skipped = session.skip_messages(channel, n);
            lowlat_common::log_info!(
                "client: keyframe {n} messages ahead, skipping to it, discarded={skipped}"
            );
            self.skipped = self.skipped.saturating_add(u64::from(pictures_before));
        }
    }

    /// Every sound packet, into the pool for the application, stamped with
    /// when it arrived.
    ///
    /// **A full pool drops the packet rather than leaving it.** Left in the
    /// receive ring it would sit behind everything the host sends after it,
    /// and a reader that is not calling would play it late when it did; the
    /// packet is taken off the ring and counted instead.
    fn drain_audio(&mut self, session: &mut Session<'_>, now_ms: f64) -> Option<Outcome> {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "milliseconds since the loop's epoch, wrapping as a stamp"
        )]
        let arrived = now_ms.max(0.0) as u32;
        loop {
            let mut refused: Option<Error> = None;
            let mut take = |slot: &mut [u8]| match session.take_message(AUDIO_CHANNEL, slot) {
                Some(Ok(len)) => Some(len),
                Some(Err(error)) => {
                    refused = Some(error);
                    None
                }
                None => None,
            };
            let len = if let Some(mut writer) = self.packets.writer() {
                if !writer.fill_with(&mut take) {
                    if let Some(error) = refused {
                        lowlat_common::log_warn!("client: sound packet refused, error={error:?}");
                        return Some(Outcome::Unreadable);
                    }
                    return None;
                }
                let len = writer.written().len();
                if !self.first_audio_said {
                    self.first_audio_said = true;
                    if let Ok(header) = audio::parse(writer.written()) {
                        lowlat_common::log_info!(
                            "client: sound mask={} samples={} rate={} codec={:?} channels={}",
                            header.mask,
                            header.samples,
                            header.rate,
                            header.codec,
                            header.channels
                        );
                    }
                }
                self.packets.publish(writer, arrived);
                len
            } else {
                let Some(len) = take(&mut self.dropped_sound) else {
                    if let Some(error) = refused {
                        lowlat_common::log_warn!("client: sound packet refused, error={error:?}");
                        return Some(Outcome::Unreadable);
                    }
                    return None;
                };
                self.audio_dropped = self.audio_dropped.saturating_add(1);
                if self.audio_dropped == 1 {
                    lowlat_common::log_warn!("client: sound pool full, packet dropped");
                }
                len
            };
            self.audio_packets = self.audio_packets.saturating_add(1);
            self.audio_bytes = self.audio_bytes.saturating_add(len as u64);
        }
    }

    /// How far behind the reader is: what waits in the ring plus what was
    /// handed over and not taken, and for how long there has been anything.
    fn measure_lag(&mut self, session: &Session<'_>, now_ms: f64) {
        let pending = session.pending_messages(VIDEO_CHANNEL);
        let queued = u32::try_from(self.units.queued()).unwrap_or(u32::MAX);
        let behind = pending.saturating_add(queued);
        let since = if behind == 0 {
            None
        } else {
            Some(self.behind_since.unwrap_or(now_ms))
        };
        self.behind_since = since;
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a non-negative duration in whole milliseconds, saturated"
        )]
        let behind_ms = since.map_or(0, |at| {
            (now_ms - at).max(0.0).min(f64::from(u32::MAX)) as u32
        });
        self.lag = Lag { behind, behind_ms };
    }

    fn publish(&self, session: &Session<'_>, now_ms: f64) {
        let t = &self.telemetry;
        // Per channel, what the ring and the session counted and what this
        // driver took off each.
        let messages = [
            self.received.iter().map(|c| u64::from(*c)).sum(),
            self.pictures.saturating_add(self.metadata),
            self.audio_packets,
        ];
        let bytes = [self.control_bytes, self.video_bytes, self.audio_bytes];
        for channel in 0..CHANNELS {
            let index = u8::try_from(channel).unwrap_or(u8::MAX);
            let arrivals = session.recv_arrivals(index).unwrap_or_default();
            let drops = session.recv_drops(index).unwrap_or_default();
            let store = |slot: &[AtomicU64; CHANNELS], value: u64| {
                if let Some(cell) = slot.get(channel) {
                    cell.store(value, Ordering::Relaxed);
                }
            };
            store(&t.fragments, arrivals.fragments);
            store(&t.late, arrivals.late);
            store(&t.duplicates, drops.duplicate);
            store(&t.out_of_window, drops.out_of_window);
            store(&t.nacks_sent, session.nacks_sent(index));
            store(&t.bytes, bytes.get(channel).copied().unwrap_or(0));
            store(&t.messages, messages.get(channel).copied().unwrap_or(0));
            #[allow(clippy::cast_possible_truncation, reason = "a ratio in 0..1")]
            if let (Some(cell), Some(ewma)) = (t.loss_30s.get(channel), self.loss_ewma.get(channel))
            {
                cell.store((*ewma as f32).to_bits(), Ordering::Relaxed);
            }
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "milliseconds since the session came up, saturated"
        )]
        t.connected_ms.store(
            (now_ms - self.established_ms)
                .max(0.0)
                .min(f64::from(u32::MAX)) as u32,
            Ordering::Relaxed,
        );
        t.pictures.store(self.pictures, Ordering::Relaxed);
        t.metadata.store(self.metadata, Ordering::Relaxed);
        t.skipped.store(self.skipped, Ordering::Relaxed);
        t.video_bytes.store(self.video_bytes, Ordering::Relaxed);
        t.audio_packets.store(self.audio_packets, Ordering::Relaxed);
        t.audio_bytes.store(self.audio_bytes, Ordering::Relaxed);
        t.audio_dropped.store(self.audio_dropped, Ordering::Relaxed);
        t.control_in.store(
            self.received.iter().map(|c| u64::from(*c)).sum(),
            Ordering::Relaxed,
        );
        t.control_out.store(
            self.sent.iter().map(|c| u64::from(*c)).sum(),
            Ordering::Relaxed,
        );
        t.behind.store(self.lag.behind, Ordering::Relaxed);
        t.behind_ms.store(self.lag.behind_ms, Ordering::Relaxed);
        t.encode_us
            .store(self.encode_latency_us(0), Ordering::Relaxed);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a round trip in whole milliseconds, saturated"
        )]
        t.rtt_ms.store(
            session.srtt_ms().max(0.0).min(f64::from(u32::MAX)) as u32,
            Ordering::Relaxed,
        );
    }
}

fn negotiated_encoder_flags(external: u32, initial: u32) -> u32 {
    const MASK: u32 =
        init::FLAG_HEVC | init::FLAG_COLOR444 | init::FLAG_FULL_RANGE | init::FLAG_10BIT;
    (external & !MASK) | (initial & MASK)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_flush_hint_tracks_only_accepted_control_messages() {
        let (emit, _) = events::queue();
        let mut driver = Driver::new(
            crate::config::Config::default().init(&crate::config::Caps::default()),
            Units::new(),
            Packets::new(),
            emit,
            Arc::new(Telemetry::default()),
        );
        let mut session = Session::new(
            lowlat_core::envelope::Envelope::from_key(&[0x77; 32]).unwrap(),
            1,
            0.0,
        );
        let control = Control {
            opcode: op::MOUSE_BUTTON,
            a0: 1,
            a1: 1,
            a2: 0,
            body: &[],
        };
        // A missing channel rejects the enqueue, so there is nothing to flush.
        driver.send_control(&mut session, &control);
        assert!(!driver.take_output_queued());
        let mut bodies = vec![0u8; 256 * 64];
        let mut slots = vec![lowlat_core::send::SendSlot::default(); 64];
        session
            .attach_send(
                CONTROL_CHANNEL,
                lowlat_core::send::SendRing::new(&mut bodies, &mut slots, 256, CONTROL_CHANNEL)
                    .unwrap(),
            )
            .unwrap();
        driver.send_control(&mut session, &control);
        assert!(driver.take_output_queued());
        assert!(!driver.take_output_queued());
        let before = driver.sent()[usize::from(op::MOUSE_BUTTON)];
        let oversized = vec![0u8; 256 * 65];
        driver.send_control(
            &mut session,
            &Control {
                body: &oversized,
                ..control
            },
        );
        assert_eq!(driver.sent()[usize::from(op::MOUSE_BUTTON)], before);
        assert!(!driver.take_output_queued());
    }

    #[cfg(feature = "usb-hid-input")]
    #[test]
    fn usb_hid_live_route_never_duplicates_or_falls_back() {
        let (emit, _) = events::queue();
        let mut driver = Driver::new(
            crate::config::Config::default().init(&crate::config::Caps::default()),
            Units::new(),
            Packets::new(),
            emit,
            Arc::new(Telemetry::default()),
        );
        for live in [false, true] {
            driver.hid_mouse_enabled = live;
            for opcode in [
                op::MOUSE_BUTTON,
                op::MOUSE_WHEEL,
                op::MOUSE_MOTION,
                op::MOUSE_MOTION_STREAM,
            ] {
                assert_eq!(
                    driver.route_hid_input(&Control {
                        opcode,
                        a0: 1,
                        a1: 1,
                        a2: 0,
                        body: &[]
                    }),
                    live
                );
            }
            for opcode in [
                op::KEYBOARD,
                op::RELEASE,
                op::GAMEPAD_BUTTON,
                op::ENCODER_CONFIG,
            ] {
                assert!(!driver.route_hid_input(&Control {
                    opcode,
                    a0: 0,
                    a1: 0,
                    a2: 0,
                    body: &[]
                }));
            }
        }
    }

    #[test]
    fn external_encoder_requests_keep_negotiated_chroma_depth_and_range() {
        for initial in [0, 8, 0x09, 0x0b, 0x19, 0x1b] {
            for external in [0, 1, 2, 8, 16, 0x1b, 0x40, 0x5b] {
                let flags = negotiated_encoder_flags(external, initial);
                assert_eq!(flags & 0x1b, initial);
                assert_eq!(flags & !0x1b, external & !0x1b);
            }
        }
        assert_eq!(negotiated_encoder_flags(0x1b, 0x19), 0x19);
    }
    #[test]
    fn only_video_encode_reports_update_the_current_connections_timing() {
        let make = || {
            let (emit, _) = events::queue();
            Driver::new(
                crate::config::Config::default().init(&crate::config::Caps::default()),
                Units::new(),
                Packets::new(),
                emit,
                Arc::new(Telemetry::default()),
            )
        };
        let mut driver = make();
        assert_eq!(driver.encode_latency_us(0), 0);
        for (kind, reported, stream, expected) in [
            (2, 9000, 0, 0),
            (0, 8000, 0, 0),
            (1, 3400, 0, 3400),
            (2, 19000, 0, 3400),
            (1, 0, 0, 3400),
            (1, 4200, 0, 4200),
            (99, 7000, 0, 4200),
            (1, 9000, u32::MAX, 4200),
            (2, 0, 0, 4200),
            (1, 0, 0, 4200),
        ] {
            let _ = driver.on_control(&Control {
                a0: kind,
                a1: reported,
                a2: stream,
                opcode: op::ENCODE_LATENCY,
                body: &[],
            });
            assert_eq!(driver.encode_latency_us(0), expected);
        }
        assert_eq!(
            make().encode_latency_us(0),
            0,
            "new connection must not inherit old timing"
        );
    }

    /// An age reads across the stamp's wrap: a stamp taken a millisecond
    /// before its 31 bits run out, read two after, is three milliseconds old.
    #[test]
    fn an_age_reads_across_the_stamps_wrap() {
        let wrap_ms = f64::from(STAMP_MASK) / 1000.0;
        let stamp = stamp_us(wrap_ms - 1.0);
        assert!(stamp_us(wrap_ms + 2.0) < stamp, "the stamp did not wrap");
        assert_eq!(stamp_age_us(wrap_ms + 2.0, stamp), 3000);
        assert_eq!(stamp_age_us(5.0, stamp_us(5.0)), 0);
    }

    /// Units a stream left handed over and untaken are let go by a clear,
    /// and every slot is the producer's again for the stream after it.
    #[test]
    fn a_clear_lets_go_of_the_units_left_behind() {
        let units = Units::new();
        let publish = |units: &Units, byte: u8| {
            let mut writer = units.pool.acquire().expect("a free slot");
            assert!(writer.fill_with(|slot| {
                slot[..4].fill(byte);
                Some(4)
            }));
            assert_eq!(writer.publish(0, &[&units.ring]), 1);
        };
        for byte in 0..UNIT_SLOTS as u8 {
            publish(&units, byte);
        }
        assert_eq!(units.queued(), UNIT_SLOTS);

        units.clear();
        assert_eq!(units.queued(), 0);
        assert!(units.take().is_none(), "a unit outlived the clear");
        // Every slot came back: the next stream fills all of them.
        for byte in 0..UNIT_SLOTS as u8 {
            publish(&units, 10 + byte);
        }
        assert_eq!(
            units.take().expect("the next stream's first").bytes(),
            &[10; 4]
        );
    }
}
