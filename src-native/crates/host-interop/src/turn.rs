//! Explicit RFC 8656 datagram carrier. Reuses the existing RustCrypto TURN codec;
//! no native Phaze relay, ring, OpenSSL or native crypto binding is used here.
use anyhow::{bail, ensure, Context, Result};
use lowlat_core::{
    stun::TransactionId,
    turn::{self as wire, Auth, Inbound, Key, Method},
};
use std::{
    collections::BTreeMap,
    io,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf},
    net::{TcpStream, UdpSocket},
    sync::{mpsc, oneshot, watch, Mutex as AsyncMutex},
    task::JoinHandle,
};
use zeroize::Zeroizing;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Carrier {
    Udp,
    Tcp,
    Tls,
}
trait Stream: AsyncRead + AsyncWrite + Unpin + Send + Sync {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + Sync> Stream for T {}
type StreamIo = Box<dyn Stream>;
enum Io {
    Udp(UdpSocket),
    Stream {
        reader: AsyncMutex<ReadHalf<StreamIo>>,
        writer: AsyncMutex<WriteHalf<StreamIo>>,
    },
}
impl Io {
    async fn send(&self, bytes: &[u8]) -> io::Result<()> {
        match self {
            Self::Udp(socket) => {
                let n = socket.send(bytes).await?;
                if n != bytes.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "Partial TURN datagram",
                    ));
                }
            }
            Self::Stream { writer, .. } => {
                let mut writer = writer.lock().await;
                writer.write_all(bytes).await?;
                writer.flush().await?;
            }
        }
        Ok(())
    }
    async fn receive(&self, bytes: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Udp(socket) => socket.recv(bytes).await,
            Self::Stream { reader, .. } => {
                // One dedicated reader owns complete frames. Cancellation only
                // occurs when the carrier is destroyed, never between timer ticks.
                let mut reader = reader.lock().await;
                reader.read_exact(&mut bytes[..4]).await?;
                let body = usize::from(u16::from_be_bytes([bytes[2], bytes[3]]));
                let n = match bytes[0] >> 6 {
                    0 => 20 + body,
                    1 => 4 + body,
                    _ => return Err(io::Error::other("Invalid TURN stream frame")),
                };
                if n > bytes.len() {
                    return Err(io::Error::other("TURN frame exceeds receive limit"));
                }
                reader.read_exact(&mut bytes[4..n]).await?;
                if bytes[0] >> 6 == 1 {
                    let mut pad = [0; 3];
                    reader.read_exact(&mut pad[..(4 - n % 4) % 4]).await?;
                }
                Ok(n)
            }
        }
    }
}
struct Credentials {
    username: Zeroizing<String>,
    password: Zeroizing<String>,
    realm: Vec<u8>,
    nonce: Vec<u8>,
    key: Option<Key>,
}
impl Credentials {
    fn auth(&self) -> Option<Auth<'_>> {
        Some(Auth {
            username: &self.username,
            realm: &self.realm,
            nonce: &self.nonce,
            key: self.key.as_ref()?,
        })
    }
}
#[derive(Clone, Copy)]
enum Request {
    Allocate,
    Refresh(u32),
    Permission(IpAddr),
}
impl Request {
    fn method(self) -> Method {
        match self {
            Self::Allocate => Method::Allocate,
            Self::Refresh(_) => Method::Refresh,
            Self::Permission(_) => Method::CreatePermission,
        }
    }
}
#[derive(Debug)]
struct Refused(u16);
impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TURN request refused (code {})", self.0)
    }
}
impl std::error::Error for Refused {}
type Packet = (Vec<u8>, SocketAddr);
struct Inner {
    io: Io,
    reliable: bool,
    credentials: Mutex<Credentials>,
    control: AsyncMutex<()>,
    pending: Mutex<BTreeMap<[u8; 12], oneshot::Sender<Vec<u8>>>>,
    permissions: Mutex<BTreeMap<IpAddr, Instant>>,
    denied: Mutex<BTreeMap<IpAddr, Instant>>,
    failed: watch::Sender<bool>,
    expires: Mutex<Option<Instant>>,
}
struct Pending<'a> {
    inner: &'a Inner,
    id: [u8; 12],
}
impl Drop for Pending<'_> {
    fn drop(&mut self) {
        self.inner
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
    }
}
fn transaction_id() -> Result<TransactionId> {
    let mut id = [0; 12];
    getrandom::fill(&mut id).map_err(|_| anyhow::anyhow!("OS randomness unavailable"))?;
    Ok(TransactionId(id))
}
impl Inner {
    async fn request(&self, request: Request) -> Result<Vec<u8>> {
        let _control = self.control.lock().await;
        for _ in 0..3 {
            ensure!(!*self.failed.borrow(), "TURN carrier closed");
            let id = transaction_id()?;
            let mut packet = vec![0; 2048];
            let len = {
                let credentials = self.credentials.lock().unwrap_or_else(|e| e.into_inner());
                let auth = credentials.auth();
                match request {
                    Request::Allocate => wire::encode_allocate(&mut packet, id, 600, auth.as_ref()),
                    Request::Refresh(seconds) => wire::encode_refresh(
                        &mut packet,
                        id,
                        seconds,
                        auth.as_ref().context("TURN allocation not authenticated")?,
                    ),
                    Request::Permission(ip) => wire::encode_create_permission(
                        &mut packet,
                        id,
                        ip,
                        auth.as_ref().context("TURN allocation not authenticated")?,
                    ),
                }
                .map_err(|_| anyhow::anyhow!("TURN request exceeds codec bounds"))?
            };
            packet.truncate(len);
            let (tx, mut response) = oneshot::channel();
            self.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(id.0, tx);
            let _pending = Pending {
                inner: self,
                id: id.0,
            };
            self.io
                .send(&packet)
                .await
                .context("TURN request send failed")?;
            let deadline = tokio::time::Instant::now() + Duration::from_millis(39500);
            let mut delay = Duration::from_millis(500);
            let mut resend = tokio::time::Instant::now() + delay;
            let bytes = loop {
                tokio::select! {
                    result=&mut response=>break result.context("TURN response reader closed")?,
                    _=tokio::time::sleep_until(deadline)=>bail!("TURN transaction timed out"),
                    _=tokio::time::sleep_until(resend),if !self.reliable=>{
                        self.io.send(&packet).await.context("TURN request retransmission failed")?;
                        delay=(delay*2).min(Duration::from_secs(16));resend=tokio::time::Instant::now()+delay;
                    }
                }
            };
            let Inbound::Response(reply) =
                wire::parse(&bytes).map_err(|_| anyhow::anyhow!("Invalid TURN response"))?
            else {
                bail!("Not a TURN response")
            };
            ensure!(
                reply.method() == request.method() && reply.transaction_id() == id,
                "TURN transaction mismatch"
            );
            let mut credentials = self.credentials.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(key) = credentials.key.as_ref() {
                ensure!(
                    !reply.is_authenticated() || reply.verify(key),
                    "TURN response integrity rejected"
                );
            }
            if let Some(challenge) = reply.challenge() {
                ensure!(
                    reply.error_code() != Some(wire::UNAUTHORIZED) || credentials.key.is_none(),
                    "TURN credentials rejected"
                );
                // RFC 8489 extended algorithm negotiation is not implemented by
                // this legacy MD5/SHA-1 codec. Refuse it rather than downgrade.
                ensure!(
                    !challenge.nonce.starts_with(b"obMatJos2"),
                    "TURN extended authentication negotiation is unsupported"
                );
                credentials.realm = challenge.realm.to_vec();
                credentials.nonce = challenge.nonce.to_vec();
                credentials.key = Some(Key::long_term(
                    &credentials.username,
                    &credentials.realm,
                    &credentials.password,
                ));
                continue;
            }
            ensure!(
                credentials
                    .key
                    .as_ref()
                    .is_some_and(|key| reply.verify(key)),
                "TURN response is not authenticated"
            );
            if let Some(code) = reply.error_code() {
                return Err(Refused(code).into());
            }
            return Ok(bytes);
        }
        bail!("TURN nonce retry limit reached")
    }
    async fn permission(&self, peer: IpAddr, renew: bool) -> Result<()> {
        if !renew
            && self
                .permissions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&peer)
                .is_some_and(|at| at.elapsed() < Duration::from_secs(240))
        {
            return Ok(());
        }
        if self
            .denied
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&peer)
            .is_some_and(|at| at.elapsed() < Duration::from_secs(30))
        {
            return Err(Refused(403).into());
        }
        match self.request(Request::Permission(peer)).await {
            Ok(_) => {
                let mut permissions = self.permissions.lock().unwrap_or_else(|e| e.into_inner());
                ensure!(
                    permissions.len() < 32 || permissions.contains_key(&peer),
                    "TURN permission limit exceeded"
                );
                permissions.insert(peer, Instant::now());
                Ok(())
            }
            Err(error) => {
                if error.downcast_ref::<Refused>().is_some_and(|e| e.0 == 403) {
                    self.denied
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(peer, Instant::now());
                }
                Err(error)
            }
        }
    }
}
/// Caller-owned allocation. Dropping it aborts its two IO/timer tasks and closes
/// the socket. Explicit close additionally releases the server allocation.
pub struct Relay {
    inner: Arc<Inner>,
    address: SocketAddr,
    packets: AsyncMutex<mpsc::Receiver<Packet>>,
    tasks: Vec<JoinHandle<()>>,
}
impl Drop for Relay {
    fn drop(&mut self) {
        self.inner.failed.send_replace(true);
        for task in &self.tasks {
            task.abort();
        }
    }
}
impl Relay {
    pub async fn allocate(
        host: &str,
        port: u16,
        carrier: Carrier,
        username: &str,
        password: &str,
    ) -> Result<Self> {
        Self::allocate_trusted(host, port, carrier, username, password, &[]).await
    }
    /// Additional certificates must come from the embedding platform's trusted
    /// root store (or an explicitly trusted isolated fixture), never the peer.
    pub async fn allocate_trusted(
        host: &str,
        port: u16,
        carrier: Carrier,
        username: &str,
        password: &str,
        extra_roots: &[Vec<u8>],
    ) -> Result<Self> {
        ensure!(
            !host.is_empty() && port != 0 && username.len() <= 512 && password.len() <= 4096,
            "Invalid TURN configuration"
        );
        let addresses: Vec<_> = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::lookup_host((host, port)),
        )
        .await
        .context("TURN DNS timed out")??
        .filter(SocketAddr::is_ipv4)
        .collect();
        ensure!(!addresses.is_empty(), "TURN server has no IPv4 endpoint");
        let io = if carrier == Carrier::Udp {
            let socket = UdpSocket::bind("0.0.0.0:0").await?;
            socket.connect(addresses[0]).await?;
            Io::Udp(socket)
        } else {
            let socket = tokio::time::timeout(
                Duration::from_secs(10),
                TcpStream::connect(addresses.as_slice()),
            )
            .await
            .context("TURN TCP connect timed out")??;
            socket.set_nodelay(true)?;
            let stream: StreamIo = if carrier == Carrier::Tls {
                let name = rustls::pki_types::ServerName::try_from(host.to_owned())
                    .map_err(|_| anyhow::anyhow!("Invalid TURN TLS name"))?;
                Box::new(
                    tokio::time::timeout(
                        Duration::from_secs(10),
                        tokio_rustls::TlsConnector::from(super::tls_with_roots(extra_roots)?)
                            .connect(name, socket),
                    )
                    .await
                    .context("TURN TLS handshake timed out")??,
                )
            } else {
                Box::new(socket)
            };
            let (reader, writer) = tokio::io::split(stream);
            Io::Stream {
                reader: AsyncMutex::new(reader),
                writer: AsyncMutex::new(writer),
            }
        };
        let inner = Arc::new(Inner {
            io,
            reliable: carrier != Carrier::Udp,
            credentials: Mutex::new(Credentials {
                username: Zeroizing::new(username.into()),
                password: Zeroizing::new(password.into()),
                realm: Vec::new(),
                nonce: Vec::new(),
                key: None,
            }),
            control: AsyncMutex::new(()),
            pending: Mutex::new(BTreeMap::new()),
            permissions: Mutex::new(BTreeMap::new()),
            denied: Mutex::new(BTreeMap::new()),
            failed: watch::channel(false).0,
            expires: Mutex::new(None),
        });
        let (tx, rx) = mpsc::channel(64);
        let receiver = inner.clone();
        let read_task = tokio::spawn(async move {
            let mut buffer = vec![0; 65556];
            let mut closed = receiver.failed.subscribe();
            loop {
                let count = tokio::select! {_=closed.changed()=>break,result=receiver.io.receive(&mut buffer)=>match result{Ok(n)=>n,Err(_)=>{receiver.failed.send_replace(true);break;}}};
                let Ok(message) = wire::parse(&buffer[..count]) else {
                    continue;
                };
                match message {
                    Inbound::Response(response) => {
                        if let Some(sender) = receiver
                            .pending
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .remove(&response.transaction_id().0)
                        {
                            let _ = sender.send(buffer[..count].to_vec());
                        }
                    }
                    Inbound::Indication { peer, data } => {
                        let _ = tx.try_send((data.to_vec(), peer));
                    }
                    _ => {} // No ChannelBind is used: unregistered channels are not admitted.
                }
            }
            receiver
                .pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
        });
        let mut relay = Self {
            inner,
            address: "0.0.0.0:0".parse().unwrap(),
            packets: AsyncMutex::new(rx),
            tasks: vec![read_task],
        };
        let allocated = relay.inner.request(Request::Allocate).await?;
        let Inbound::Response(response) =
            wire::parse(&allocated).map_err(|_| anyhow::anyhow!("Invalid allocation response"))?
        else {
            bail!("Invalid allocation response")
        };
        relay.address = response
            .relayed_address()
            .filter(|a| a.is_ipv4() && !a.ip().is_unspecified() && a.port() != 0)
            .context("TURN response has no usable relayed address")?;
        let mut lifetime = response
            .lifetime_s()
            .filter(|n| *n >= 2)
            .context("TURN response has no valid allocation lifetime")?;
        *relay
            .inner
            .expires
            .lock()
            .unwrap_or_else(|e| e.into_inner()) =
            Some(Instant::now() + Duration::from_secs(u64::from(lifetime)));
        let maintenance = relay.inner.clone();
        relay.tasks.push(tokio::spawn(async move {
            let mut closed=maintenance.failed.subscribe();
            loop {
                tokio::select!{_=closed.changed()=>break,_=tokio::time::sleep(Duration::from_secs(u64::from((lifetime/2).clamp(1,120))))=>{}}
                let renewal=maintenance.request(Request::Refresh(600)).await;
                let Ok(bytes)=renewal else{maintenance.failed.send_replace(true);break};
                let Ok(Inbound::Response(reply))=wire::parse(&bytes) else{maintenance.failed.send_replace(true);break};
                let Some(seconds)=reply.lifetime_s().filter(|n|*n>=2) else{maintenance.failed.send_replace(true);break};lifetime=seconds;*maintenance.expires.lock().unwrap_or_else(|e|e.into_inner())=Some(Instant::now()+Duration::from_secs(u64::from(seconds)));
                let peers:Vec<_>=maintenance.permissions.lock().unwrap_or_else(|e|e.into_inner()).keys().copied().collect();
                for peer in peers {if maintenance.permission(peer,true).await.is_err(){maintenance.failed.send_replace(true);return;}}
            }
        }));
        Ok(relay)
    }
    pub async fn close(&self) {
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            self.inner.request(Request::Refresh(0)),
        )
        .await;
        self.inner.failed.send_replace(true);
    }
    pub fn address(&self) -> SocketAddr {
        self.address
    }
}
impl crate::Datagram for &Relay {
    fn candidates(&self) -> io::Result<Vec<SocketAddr>> {
        Ok(vec![self.address])
    }
    fn candidate_short_ttl(&self) -> bool {
        // A public allocation needs an outbound host probe to open its NAT;
        // advertising it like a local interface leaves the host passive.
        true
    }
    fn endpoint(&self, address: SocketAddr) -> Option<SocketAddr> {
        let address = match address {
            SocketAddr::V6(a) => SocketAddr::new(a.ip().to_ipv4_mapped()?.into(), a.port()),
            a => a,
        };
        (!address.ip().is_loopback() || self.address.ip().is_loopback()).then_some(address)
    }
    async fn send_to(&self, data: &[u8], peer: SocketAddr) -> io::Result<usize> {
        if *self.inner.failed.borrow()
            || self
                .inner
                .expires
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(io::Error::other("TURN allocation lost"));
        }
        self.inner.permission(peer.ip(), false).await.map_err(|e| {
            if e.downcast_ref::<Refused>().is_some_and(|e| e.0 == 403) {
                io::Error::new(
                    io::ErrorKind::HostUnreachable,
                    "TURN peer permission refused",
                )
            } else {
                io::Error::other("TURN permission failed")
            }
        })?;
        let offset = wire::indication_header_len(peer);
        let mut packet = vec![0; offset + data.len() + 3];
        packet[offset..offset + data.len()].copy_from_slice(data);
        let id = transaction_id().map_err(io::Error::other)?;
        let size = wire::wrap_indication(&mut packet, id, peer, data.len())
            .map_err(|_| io::Error::other("TURN datagram exceeds codec bounds"))?;
        self.inner.io.send(&packet[..size]).await?;
        Ok(data.len())
    }
    async fn recv_from(&self, data: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        let mut closed = self.inner.failed.subscribe();
        if *closed.borrow() {
            return Err(io::Error::other("TURN allocation lost"));
        }
        let mut packets = self.packets.lock().await;
        let (packet, peer) = loop {
            let expires = self
                .inner
                .expires
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .context("TURN lease unavailable")
                .map_err(io::Error::other)?;
            if Instant::now() >= expires {
                return Err(io::Error::other("TURN allocation expired"));
            }
            tokio::select! {
                _=closed.changed()=>return Err(io::Error::other("TURN allocation lost")),
                _=tokio::time::sleep_until(tokio::time::Instant::from_std(expires))=>continue,
                packet=packets.recv()=>break packet.ok_or_else(||io::Error::other("TURN receive closed"))?,
            }
        };
        if packet.len() > data.len() {
            return Err(io::Error::other("TURN receive buffer too small"));
        }
        data[..packet.len()].copy_from_slice(&packet);
        Ok((packet.len(), peer))
    }
}
