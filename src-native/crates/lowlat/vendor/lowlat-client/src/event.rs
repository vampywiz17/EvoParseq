//! What the client tells the application, and what it refuses: the events a
//! session raises, why an attempt ended, what a call can refuse, and what
//! reaches the session loop from outside it.
//!
//! Shared by the seam, the session loop, the decode thread and the driver.
//! Nothing here opens a socket or a device, so it builds wherever the driver
//! does.

use std::net::SocketAddr;

use lowlat_common::events::Queued;
use lowlat_core::conn::Kind;

/// Which pipe an attempt asks for. A client of this library uses the native
/// one; the browser's exists so a page can be a guest, and a native client
/// has no reason to be one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Bud,
    Web,
}

/// What the answer told us about the host.
#[derive(Debug, Clone)]
pub struct Peer {
    pub ufrag: String,
    pub pwd: String,
    /// The host's certificate digest: the legacy cipher's key material.
    pub fingerprint: String,
    /// The host's media key, absent from a legacy generation's answer.
    pub aes256: Option<String>,
}

/// What the application must forward to the host, or act on.
///
/// Exhaustive on purpose: the C boundary translates every variant, and one
/// added here must fail to compile there rather than fall through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Sanitized local IO failure, preceding Ended. Never carries error text.
    IoFailure {
        stage: IoStage,
        kind: std::io::ErrorKind,
        os_error: Option<i32>,
    },
    /// Original native control message for an application-owned UI.
    ExternalControl { bytes: Vec<u8> },
    /// A local candidate, to be sent to the host as it is found.
    Candidate {
        addr: SocketAddr,
        from_stun: bool,
        lan: bool,
    },
    /// Send the host a candidate marked `sync`, once, after every real one.
    Ready,
    /// Connectivity completed; the initialization has gone out.
    Established { addr: SocketAddr },
    /// The attempt is over, with the reason typed.
    Ended { outcome: Outcome },
}

impl Queued for Event {
    fn body(&self) -> &[u8] {
        match self {
            Event::ExternalControl { bytes } => bytes,
            _ => &[],
        }
    }
    fn held(&self) -> usize {
        0
    }
}

/// Why an attempt finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Negotiated, and no path was found.
    ConnectivityFailed,
    /// The host stopped answering.
    PeerGone,
    /// Nothing sent has been acknowledged for the delivery deadline.
    Undeliverable,
    /// The socket or the loop failed.
    TransportFailed,
    /// The host ended the session and said why, with its own status.
    Disconnected(i32),
    /// A message arrived that this client cannot take: larger than any
    /// buffer, or unreadable. The channel cannot advance past it.
    Unreadable,
    /// The relay did not answer in time to allocate and permit.
    RelayUnreachable,
    /// The relay refused the credentials or the allocation, or is full. The
    /// same relay refuses again.
    RelayRefused,
    /// A renewal was refused, or went unanswered until what it renewed
    /// lapsed; the relay has let the allocation go.
    RelayLost,
}

/// What a seam call can refuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// There is already an attempt; a client makes one at a time.
    Busy,
    /// No attempt by that name.
    UnknownAttempt,
    /// The attempt has already begun.
    AlreadyBegun,
    /// The browser pipe was asked for; a native client does not use it.
    Transport,
    /// The answer's credentials cannot key a session: no media key on an
    /// offer that carried one, or material that does not decode.
    Credentials,
    /// The platform would not supply entropy.
    Crypto,
    /// A socket or thread could not be had.
    Io {
        stage: IoStage,
        kind: std::io::ErrorKind,
        os_error: Option<i32>,
    },
    /// No active session.
    NoSession,
    /// Unsupported application media capability.
    Unsupported,
}

/// Safe local setup phases. No peer/server strings are retained in errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoStage {
    Socket,
    LocalAddress,
    RelayCarrier,
    Thread,
    CarrierPrepare,
    TransportIo,
}
impl IoStage {
    pub fn name(self) -> &'static str {
        match self {
            Self::Socket => "udp_socket",
            Self::LocalAddress => "local_address",
            Self::RelayCarrier => "relay_carrier_create",
            Self::Thread => "session_thread",
            Self::CarrierPrepare => "relay_connect_or_tls_handshake",
            Self::TransportIo => "transport_io",
        }
    }
}
impl Error {
    pub(crate) fn io(stage: IoStage, error: &std::io::Error) -> Self {
        Self::Io {
            stage,
            kind: error.kind(),
            os_error: error.raw_os_error(),
        }
    }
}

/// What reaches the loop from outside it.
pub(crate) enum Arrival {
    Candidate(SocketAddr, Kind),
    PeerReady,
}

/// What the application asks the loop to say.
pub(crate) enum Ask {
    UserData(u32, Vec<u8>),
    /// Say goodbye and stop.
    Leave,
}

/// How long a departure is given to reach the host before the loop stops.
/// The message is on a reliable channel and needs time to get there.
pub(crate) const LEAVE_GRACE_MS: f64 = 250.0;
