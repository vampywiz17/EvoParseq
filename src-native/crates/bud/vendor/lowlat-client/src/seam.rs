//! Transport-only EvoParseq seam derived from the pinned lowlat client.
//! Media stays compressed until the application's existing decoders consume it.
use crate::config::{Caps, Config, Port};
use crate::driver::{Telemetry, Units};
use crate::event::{Arrival, Ask, Error, Event, LEAVE_GRACE_MS, Peer, Transport};
use crate::input::{RING_DEPTH, Request};
use crate::sound::{self, Packets};
use lowlat_common::{events, spsc::Ring};
use lowlat_core::{conn::Kind, envelope::Cipher};
use lowlat_crypto::Credentials;
use lowlat_net::{Guest, Wake};
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock, mpsc};
use std::time::Duration;

struct Attempt {
    id: String,
    config: Config,
    ours: Credentials,
    pending: Vec<Arrival>,
    inject: Option<mpsc::Sender<Arrival>>,
    ask: Option<mpsc::Sender<Ask>>,
    requests: Option<Arc<Ring<Request, RING_DEPTH>>>,
    thread: Option<Guest>,
    epoch: Arc<OnceLock<lowlat_common::clock::Time>>,
}

pub struct Client {
    attempt: Option<Attempt>,
    emit: events::Sender<Event>,
    events: events::Receiver<Event>,
    telemetry: Arc<Telemetry>,
    units: Units,
    packets: Packets,
    caps: Caps,
    external_consumer: Option<std::thread::Thread>,
    external_audio_taken: bool,
}

impl Client {
    pub fn new_external_video(caps: Caps, _port: Port) -> Result<Self, Error> {
        if !caps.h264 {
            return Err(Error::Unsupported);
        }
        let (emit, events) = events::queue();
        Ok(Self {
            attempt: None,
            emit,
            events,
            telemetry: Arc::default(),
            units: Units::new(),
            packets: Packets::new(),
            caps,
            external_consumer: None,
            external_audio_taken: false,
        })
    }

    pub fn new_attempt(
        &mut self,
        id: &str,
        config: Config,
        transport: Transport,
    ) -> Result<Credentials, Error> {
        if transport != Transport::Bud {
            return Err(Error::Transport);
        }
        if !cfg!(feature = "turn-stream")
            && config
                .relay
                .as_ref()
                .is_some_and(|relay| relay.transport != crate::config::RelayTransport::Udp)
        {
            return Err(Error::Transport);
        }
        if self.attempt.is_some() {
            return Err(Error::Busy);
        }
        let mut ours = lowlat_crypto::credentials().map_err(|_| Error::Crypto)?;
        self.telemetry = Arc::default();
        self.emit.clear();
        self.units.clear();
        self.packets.clear();
        self.external_audio_taken = false;
        if config.legacy_cipher {
            ours.aes256.clear();
        }
        self.attempt = Some(Attempt {
            id: id.into(),
            config,
            ours: ours.clone(),
            pending: Vec::new(),
            inject: None,
            ask: None,
            requests: None,
            thread: None,
            epoch: Arc::new(OnceLock::new()),
        });
        Ok(ours)
    }

    pub fn take_external_audio_source(&mut self) -> Result<sound::EncodedAudioSource, Error> {
        if self.attempt.is_none() {
            return Err(Error::NoSession);
        }
        if self.external_audio_taken {
            return Err(Error::Busy);
        }
        self.external_audio_taken = true;
        Ok(sound::EncodedAudioSource::new(self.packets.clone()))
    }

    /// Call after dropping the sole media consumer, outside UI/session locks.
    pub fn end_connection(&mut self, id: &str) {
        let Some(mut attempt) = self.attempt.take_if(|a| a.id == id) else {
            return;
        };
        if let (Some(ask), Some(thread)) = (&attempt.ask, &mut attempt.thread) {
            if ask.send(Ask::Leave).is_ok() {
                let _ = thread.wake_handle().notify();
                let began = lowlat_common::clock::Time::now();
                while thread.alive()
                    && lowlat_common::clock::elapsed_ms(began) < LEAVE_GRACE_MS * 2.0
                {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            thread.stop();
        }
        self.units.clear();
        self.packets.clear();
        self.packets.wake();
    }
    pub fn add_candidate(&mut self, id: &str, addr: SocketAddr, sync: bool, kind: Kind) {
        let Some(attempt) = self.attempt.as_mut().filter(|attempt| attempt.id == id) else {
            return;
        };
        // The marker's address is never read, so the family is asked of real
        // candidates only.
        if !sync && !attempt.config.admits(addr) {
            lowlat_common::log_info!(
                "client: attempt={} declined an IPv6 candidate, addr={}",
                id,
                addr
            );
            return;
        }
        let arrival = if sync {
            Arrival::PeerReady
        } else {
            Arrival::Candidate(addr, kind)
        };
        match (&attempt.inject, &attempt.thread) {
            (Some(inject), Some(thread)) => {
                if inject.send(arrival).is_ok() {
                    let _ = thread.wake_handle().notify();
                }
            }
            _ => attempt.pending.push(arrival),
        }
    }

    pub fn begin_p2p(&mut self, id: &str, theirs: &Peer) -> Result<(), Error> {
        let attempt = self
            .attempt
            .as_mut()
            .filter(|attempt| attempt.id == id)
            .ok_or(Error::UnknownAttempt)?;
        if attempt.thread.is_some() {
            return Err(Error::AlreadyBegun);
        }

        // **Keyed from the answer's block.** The host seals both directions
        // with its own material: its media key when both ends offered one,
        // its certificate digest under the legacy cipher when either did not.
        // The cipher is decided by presence, never by a string's length.
        let (cipher, source) = match (&theirs.aes256, attempt.ours.aes256.is_empty()) {
            (Some(key), false) if !key.is_empty() => (Cipher::Aes256, key.as_str()),
            _ => (Cipher::Aes128, theirs.fingerprint.as_str()),
        };
        let key_len = cipher.key_len();
        let (key, prefix) =
            lowlat_crypto::key_material(source, key_len).map_err(|_| Error::Credentials)?;
        let mut material = [0u8; 36];
        material
            .get_mut(..key_len)
            .ok_or(Error::Credentials)?
            .copy_from_slice(key.get(..key_len).ok_or(Error::Credentials)?);
        material
            .get_mut(key_len..key_len + prefix.len())
            .ok_or(Error::Credentials)?
            .copy_from_slice(&prefix);
        if cipher == Cipher::Aes128 {
            lowlat_common::log_info!(
                "client: attempt={} takes the legacy cipher, the exchange carried no media key",
                id
            );
        }
        let seed = lowlat_crypto::transaction_seed().map_err(|_| Error::Crypto)?;
        let relay_seed = lowlat_crypto::transaction_seed().map_err(|_| Error::Crypto)?;
        // What an earlier attempt's relay left in the status is not this
        // attempt's.
        self.telemetry.relayed.store(0, Ordering::Relaxed);
        self.telemetry.path_relayed.store(false, Ordering::Relaxed);

        let socket = lowlat_net::Socket::open_or_any_port(0)
            .map_err(|e| Error::io(crate::event::IoStage::Socket, &e))?;
        #[cfg(target_os = "linux")]
        socket.linux_qos(attempt.config.linux_qos);
        let bound = socket
            .local_addr()
            .map_err(|e| Error::io(crate::event::IoStage::LocalAddress, &e))?
            .port();
        let wake = match &attempt.config.relay {
            Some(relay) => relay.wake(),
            None => Wake::new(),
        }
        .map_err(|e| Error::io(crate::event::IoStage::RelayCarrier, &e))?;

        let (inject, arrivals) = mpsc::channel::<Arrival>();
        for arrival in attempt.pending.drain(..) {
            let _ = inject.send(arrival);
        }
        let (ask, asked) = mpsc::channel::<Ask>();
        let requests: Arc<Ring<Request, RING_DEPTH>> = Arc::new(Ring::new());

        if attempt.config.ipv4_only {
            lowlat_common::log_info!("client: attempt={} offers and checks IPv4 only", id);
        }
        // Gathered before the session thread starts, which is handed them: a
        // reflexive answer that repeats one is not offered again.
        let hosts: Vec<SocketAddr> = if attempt.config.relay.is_none() {
            lowlat_net::host_addresses(attempt.config.shared_address_space)
                .into_iter()
                .map(|ip| SocketAddr::new(ip, bound))
                .filter(|addr| attempt.config.admits(*addr))
                .collect()
        } else {
            Vec::new()
        };
        let args = crate::shell::Attached {
            #[cfg(feature = "usb-hid-input")]
            hid_mouse: attempt.config.hid_mouse,
            #[cfg(feature = "usb-hid-input")]
            hid_keyboard: attempt.config.hid_keyboard,
            external_consumer: self.external_consumer.clone(),
            socket,
            servers: attempt
                .config
                .servers
                .iter()
                .copied()
                .filter(|server| attempt.config.admits(*server))
                .collect(),
            hosts: hosts.clone(),
            relay: attempt.config.relay.clone(),
            relay_seed,
            ours: (attempt.ours.ufrag.clone(), attempt.ours.pwd.clone()),
            theirs: (theirs.ufrag.clone(), theirs.pwd.clone()),
            material,
            cipher,
            seed,
            init: {
                let init = attempt.config.init(&self.caps);
                let init = {
                    let mut init = init;
                    init.caches_cursor = false;
                    init
                };
                self.telemetry
                    .asked_flags
                    .store(attempt.config.video.asked(), Ordering::Relaxed);
                self.telemetry
                    .declared_flags
                    .store(init.flags, Ordering::Relaxed);
                self.telemetry.fallback_420.store(false, Ordering::Relaxed);
                init
            },
            arrivals,
            asked,
            requests: Arc::clone(&requests),
            emit: self.emit.clone(),
            telemetry: Arc::clone(&self.telemetry),
            units: self.units.clone(),
            packets: self.packets.clone(),
            epoch: Arc::clone(&attempt.epoch),
        };
        let thread = Guest::spawn(wake, move |wake, running| {
            crate::shell::run(args, wake, running);
        })
        .map_err(|e| Error::io(crate::event::IoStage::Thread, &e))?;
        attempt.inject = Some(inject);
        attempt.ask = Some(ask);
        attempt.requests = Some(requests);
        attempt.thread = Some(thread);

        // A relay attempt offers the relayed address and nothing else, once the
        // relay has one; the session thread raises it and the readiness
        // marker after it.
        if attempt.config.relay.is_none() {
            for addr in hosts {
                self.emit.send(Event::Candidate {
                    addr,
                    from_stun: false,
                    lan: true,
                });
            }
            // After the candidates, which is the order a peer expects: "that
            // is all of mine, now yours".
            self.emit.send(Event::Ready);
        }
        Ok(())
    }

    pub fn request_external_keyframe_format(&self, fallback_420: bool) -> bool {
        let Some(thread) = self.attempt.as_ref().and_then(|a| a.thread.as_ref()) else {
            return false;
        };
        if fallback_420 {
            self.telemetry.fallback_420.store(true, Ordering::Release);
        }
        self.telemetry.request.store(true, Ordering::Release);
        let _ = thread.wake_handle().notify();
        true
    }

    pub fn wake_external_consumer(&mut self, thread: std::thread::Thread) {
        self.external_consumer = Some(thread);
    }

    pub fn send_external_control(&mut self, bytes: &[u8]) -> bool {
        if bytes.len() < 13 || bytes.len() > 256 {
            return false;
        }
        if lowlat_core::control::parse(bytes).is_err() {
            return false;
        }
        // Session initialization, disconnect, and reports remain SDK-owned.
        if matches!(bytes.get(12), Some(10 | 11 | 21 | 25)) {
            return false;
        }
        let mut packet = [0; 256];
        let Some(destination) = packet.get_mut(..bytes.len()) else {
            return false;
        };
        destination.copy_from_slice(bytes);
        let Some(attempt) = self.attempt.as_ref() else {
            return false;
        };
        let (Some(requests), Some(thread)) = (attempt.requests.as_ref(), attempt.thread.as_ref())
        else {
            return false;
        };
        if requests
            .push(Request {
                bytes: packet,
                len: bytes.len() as u16,
            })
            .is_err()
        {
            self.telemetry.input_dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        thread.wake_handle().notify().is_ok()
    }

    pub fn send_user_data(&mut self, id: u32, text: &[u8]) -> bool {
        let Some(attempt) = self.attempt.as_ref() else {
            return false;
        };
        match (attempt.ask.as_ref(), attempt.thread.as_ref()) {
            (Some(ask), Some(thread)) => {
                ask.send(Ask::UserData(id, text.to_vec())).is_ok()
                    && thread.wake_handle().notify().is_ok()
            }
            _ => false,
        }
    }

    pub fn telemetry(&self) -> &Telemetry {
        &self.telemetry
    }

    pub fn units(&self) -> Units {
        self.units.clone()
    }

    pub fn poll_event(&mut self) -> Option<events::Received<Event>> {
        self.events.try_recv()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if let Some(id) = self.attempt.as_ref().map(|a| a.id.clone()) {
            self.end_connection(&id);
        }
    }
}

#[cfg(test)]
mod external_media_tests {
    use super::*;

    fn begin(client: &mut Client) {
        let theirs = lowlat_crypto::credentials().unwrap();
        client
            .new_attempt("external-test", Config::default(), Transport::Bud)
            .unwrap();
        client
            .begin_p2p(
                "external-test",
                &Peer {
                    ufrag: theirs.ufrag,
                    pwd: theirs.pwd,
                    fingerprint: theirs.fingerprint,
                    aes256: Some(theirs.aes256),
                },
            )
            .unwrap();
    }

    #[test]
    fn application_owned_units_have_no_competing_decoder() {
        let mut client = Client::new_external_video(
            Caps {
                h264: true,
                ..Caps::default()
            },
            Port::Any,
        )
        .unwrap();
        begin(&mut client);
        let units = client.units();
        units.hand_over(b"original-compressed-access-unit");
        std::thread::sleep(Duration::from_millis(10));
        assert_eq!(units.queued(), 1, "an internal consumer stole the unit");
        {
            let lease = units.take().unwrap();
            assert_eq!(lease.bytes(), b"original-compressed-access-unit");
        }
        assert!(client.request_external_keyframe_format(false));
        assert!(client.request_external_keyframe_format(true));
        assert!(client.telemetry.fallback_420.load(Ordering::Acquire));
        assert!(client.request_external_keyframe_format(false));
        assert!(client.telemetry.fallback_420.load(Ordering::Acquire));
        client.end_connection("external-test");
        assert!(!client.request_external_keyframe_format(false));
    }

    #[test]
    fn external_input_rejects_session_owned_and_oversized_messages() {
        let mut client = Client::new_external_video(
            Caps {
                h264: true,
                ..Caps::default()
            },
            Port::Any,
        )
        .unwrap();
        begin(&mut client);
        let mut frame = [0u8; 13];
        for opcode in [10, 11, 21, 25] {
            frame[12] = opcode;
            assert!(!client.send_external_control(&frame));
        }
        assert!(!client.send_external_control(&[0; 257]));
        assert!(!client.send_external_control(&[0; 12]));
        frame[12] = 28;
        assert!(client.send_external_control(&frame));
        client.end_connection("external-test");
        assert!(!client.send_external_control(&frame));
    }
}
