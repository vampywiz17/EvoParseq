use super::{wire, Answer, Sink};
use anyhow::{bail, ensure, Context, Result};
use hmac::{Hmac, Mac};
use purecrypto::{
    ec::{BoxedEcdsaPrivateKey, CurveId},
    tls::{Config, Connection, ProtocolVersion, SigningKey},
    x509::{CertSigner, Certificate, DistinguishedName, Time, Validity},
};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::net::UdpSocket;

pub struct Identity {
    key: BoxedEcdsaPrivateKey,
    der: Vec<u8>,
    pub fingerprint: String,
}
impl Identity {
    pub fn generate() -> Result<Self> {
        let mut rng = purecrypto::rng::OsRng;
        let key = BoxedEcdsaPrivateKey::generate(CurveId::P256, &mut rng);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .context("System clock is before Unix epoch")?
            .as_secs();
        let cert = Certificate::self_signed_general(
            &CertSigner::Ecdsa(&key),
            &DistinguishedName::common_name("EvoParseq"),
            &Validity::new(
                Time::from_unix(now.saturating_sub(300)),
                Time::from_unix(now.saturating_add(86400)),
            ),
            1,
            false,
            &[],
        )
        .map_err(|_| anyhow::anyhow!("Ephemeral certificate creation failed"))?;
        let der = cert.to_der().to_vec();
        ensure!(der.len() <= 512, "Certificate exceeds native Phaze bound");
        let fingerprint = hex(&Sha256::digest(&der));
        Ok(Self {
            key,
            der,
            fingerprint,
        })
    }
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn attr(b: &mut Vec<u8>, kind: u16, v: &[u8]) {
    b.extend(kind.to_be_bytes());
    b.extend((v.len() as u16).to_be_bytes());
    b.extend(v);
    while !b.len().is_multiple_of(4) {
        b.push(0)
    }
}
fn header(kind: u16, id: [u8; 12]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend(kind.to_be_bytes());
    b.extend([0, 0]);
    b.extend(0x2112a442u32.to_be_bytes());
    b.extend(id);
    b
}
fn finish_stun(b: &mut Vec<u8>, key: &str) -> Result<()> {
    let n = b.len();
    b[2..4].copy_from_slice(&((n + 24 - 20) as u16).to_be_bytes());
    let mut mac = Hmac::<Sha1>::new_from_slice(key.as_bytes()).unwrap();
    mac.update(b);
    let tag = mac.finalize().into_bytes();
    attr(b, 8, &tag);
    let n = b.len();
    b[2..4].copy_from_slice(&((n + 8 - 20) as u16).to_be_bytes());
    let crc = crc32fast::hash(b) ^ 0x5354554e;
    attr(b, 0x8028, &crc.to_be_bytes());
    Ok(())
}
fn check_stun<'a>(b: &'a [u8], key: &str) -> Result<Option<&'a [u8]>> {
    ensure!(
        b.len() >= 20 && b[4..8] == 0x2112a442u32.to_be_bytes(),
        "Invalid STUN header"
    );
    ensure!(
        u16::from_be_bytes(b[2..4].try_into().unwrap()) as usize + 20 == b.len(),
        "Invalid STUN length"
    );
    let mut p = 20;
    let mut username = None;
    let mut integrity = false;
    let mut fp = false;
    while p + 4 <= b.len() {
        let kind = u16::from_be_bytes(b[p..p + 2].try_into().unwrap());
        let n = u16::from_be_bytes(b[p + 2..p + 4].try_into().unwrap()) as usize;
        ensure!(p + 4 + n <= b.len(), "Truncated STUN attribute");
        match kind {
            6 => {
                ensure!(!integrity, "Unauthenticated trailing username");
                username = Some(&b[p + 4..p + 4 + n])
            }
            8 => {
                ensure!(n == 20 && !integrity, "Invalid STUN integrity");
                let mut prefix = b[..p].to_vec();
                prefix[2..4].copy_from_slice(&((p + 24 - 20) as u16).to_be_bytes());
                let mut m = Hmac::<Sha1>::new_from_slice(key.as_bytes()).unwrap();
                m.update(&prefix);
                m.verify_slice(&b[p + 4..p + 24])
                    .map_err(|_| anyhow::anyhow!("STUN authentication failed"))?;
                integrity = true
            }
            0x8028 => {
                ensure!(n == 4 && p + 8 == b.len(), "Invalid STUN fingerprint");
                let x = u32::from_be_bytes(b[p + 4..p + 8].try_into().unwrap());
                ensure!(
                    x == crc32fast::hash(&b[..p]) ^ 0x5354554e,
                    "STUN fingerprint mismatch"
                );
                fp = true
            }
            _ => {}
        }
        p += 4 + n.div_ceil(4) * 4;
    }
    ensure!(
        p == b.len() && integrity && fp,
        "Incomplete authenticated STUN packet"
    );
    Ok(username)
}
fn reply_stun(id: [u8; 12], addr: SocketAddr, key: &str) -> Result<Vec<u8>> {
    let mut b = header(0x0101, id);
    let mut x = Vec::new();
    match canonical_ip(addr.ip()) {
        IpAddr::V4(ip) => {
            x.extend([0, 1]);
            x.extend((addr.port() ^ 0x2112).to_be_bytes());
            x.extend((u32::from(ip) ^ 0x2112a442).to_be_bytes());
        }
        IpAddr::V6(ip) => {
            x.extend([0, 2]);
            x.extend((addr.port() ^ 0x2112).to_be_bytes());
            let mut mask = [0; 16];
            mask[..4].copy_from_slice(&0x2112a442u32.to_be_bytes());
            mask[4..].copy_from_slice(&id);
            x.extend(ip.octets().iter().zip(mask).map(|(a, b)| a ^ b));
        }
    }
    attr(&mut b, 0x20, &x);
    finish_stun(&mut b, key)?;
    Ok(b)
}
pub fn local_socket() -> Result<UdpSocket> {
    let socket = socket2::Socket::new(
        socket2::Domain::IPV6,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )
    .context("Create dual-stack UDP socket")?;
    socket
        .set_only_v6(false)
        .context("Enable IPv4/IPv6 dual stack")?;
    socket
        .bind(&SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), 0).into())
        .context("Bind dual-stack UDP socket")?;
    socket.set_nonblocking(true)?;
    UdpSocket::from_std(socket.into()).context("Register dual-stack UDP socket")
}
fn canonical_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v) => v.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        _ => ip,
    }
}
fn local_ips() -> Result<BTreeSet<IpAddr>> {
    let mut ips = BTreeSet::from([
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(Ipv6Addr::LOCALHOST),
    ]);
    for i in if_addrs::get_if_addrs().context("Enumerate local interface addresses")? {
        let ip = canonical_ip(i.ip());
        if !ip.is_unspecified() && !ip.is_multicast() {
            ips.insert(ip);
        }
    }
    Ok(ips)
}
pub fn local_candidates(port: u16) -> Result<Vec<SocketAddr>> {
    // Native peer info has room for sixteen addresses. Prefer loopback and IPv4;
    // scoped link-local IPv6 needs separate interface-index handling and is omitted.
    let ips = local_ips()?;
    let mut out = vec![
        SocketAddr::new("::1".parse().unwrap(), port),
        SocketAddr::new("127.0.0.1".parse().unwrap(), port),
    ];
    for ip in ips {
        if ip.is_loopback() || matches!(ip,IpAddr::V6(v)if v.is_unicast_link_local()) {
            continue;
        }
        if out.len() == 16 {
            break;
        }
        out.push(SocketAddr::new(ip, port));
    }
    Ok(out)
}
pub(crate) fn dual_endpoint(addr: SocketAddr) -> SocketAddr {
    match addr {
        SocketAddr::V4(v) => {
            SocketAddr::V6(SocketAddrV6::new(v.ip().to_ipv6_mapped(), v.port(), 0, 0))
        }
        _ => addr,
    }
}
fn candidate_endpoint(c: &super::Candidate) -> Option<SocketAddr> {
    if c.port == 0 {
        return None;
    }
    let addr = if let Ok(ip) = c.addr.parse::<IpAddr>() {
        SocketAddr::new(ip, c.port)
    } else {
        format!("[{}]:{}", c.addr, c.port).parse().ok()?
    };
    Some(dual_endpoint(addr))
}
fn probe_targets(a: &Answer) -> BTreeSet<SocketAddr> {
    a.candidates
        .iter()
        .filter_map(candidate_endpoint)
        .filter(|a| !a.ip().is_unspecified() && !a.ip().is_multicast())
        .take(32)
        .collect()
}
fn unreachable(e: &std::io::Error) -> bool {
    e.raw_os_error() == Some(10054)
        || matches!(
            e.kind(),
            std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::ConnectionRefused
                | std::io::ErrorKind::NetworkUnreachable
                | std::io::ErrorKind::HostUnreachable
        )
}
async fn select_peer(
    socket: &impl crate::Datagram,
    own: &str,
    pwd: &str,
    a: &Answer,
) -> Result<SocketAddr> {
    let mut targets: Vec<_> = probe_targets(a)
        .into_iter()
        .filter_map(|a| socket.endpoint(a))
        .collect();
    ensure!(
        !targets.is_empty(),
        "Host answer has no numeric candidate for the selected carrier"
    );
    let mut id = [0; 12];
    getrandom::fill(&mut id).map_err(|_| anyhow::anyhow!("OS randomness unavailable"))?;
    let mut req = header(1, id);
    attr(&mut req, 6, format!("{own}:{}", a.ufrag).as_bytes());
    finish_stun(&mut req, pwd)?;
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    let stop = tokio::time::Instant::now() + Duration::from_secs(15);
    let mut buf = [0; 2048];
    let (
        mut sent,
        mut unreachable_sends,
        mut received,
        mut invalid_stun,
        mut authenticated_requests,
    ) = (0u32, 0u32, 0u32, 0u32, 0u32);
    loop {
        tokio::select! {
        _=tokio::time::sleep_until(stop)=>bail!("No authenticated STUN response from advertised host candidates (candidates={}, sent={}, unreachable={}, received={}, invalid_stun={}, authenticated_requests={})", targets.len(), sent, unreachable_sends, received, invalid_stun, authenticated_requests),
        _=tick.tick()=>{for target in &targets{match socket.send_to(&req,*target).await{Ok(_)=>{sent+=1;},Err(err)if unreachable(&err)=>{unreachable_sends+=1;},Err(err)=>return Err(err).context("STUN candidate send failed")}}},
        r=socket.recv_from(&mut buf)=>{
        let(n,src)=match r{Ok(v)=>v,Err(err)if unreachable(&err)=>{continue},Err(err)=>return Err(err).context("STUN receive failed")};received+=1;let Some(src)=socket.endpoint(src) else { continue };
        if n<20{invalid_stun+=1;continue}let b=&buf[..n];if b[4..8]!=0x2112a442u32.to_be_bytes(){invalid_stun+=1;continue}
        let username=match check_stun(b,&a.pwd){Ok(u)=>u,Err(_)=>{invalid_stun+=1;continue}};
        let kind=u16::from_be_bytes(b[..2].try_into().unwrap());
        if kind==1{if username!=Some(format!("{}:{own}",a.ufrag).as_bytes()){invalid_stun+=1;continue}authenticated_requests+=1;let txid=b[8..20].try_into().unwrap();socket.send_to(&reply_stun(txid,src,pwd)?,src).await?;
            // An authenticated peer-reflexive endpoint may have a different NAT
            // port from signaling. Never learn it from an unsigned datagram.
            if !targets.contains(&src)&&targets.len()<64 {targets.push(src);socket.send_to(&req,src).await?;}
        }
        else if kind==0x101&&b[8..20]==id{return Ok(src)}
        }}
    }
}
// The installed host routes non-Hello handshake datagrams only if a CID
// record is present in that same datagram. Gather the library's ready records
// without changing their contents or the cryptographic transcript.
fn datagram_cid(b: &[u8]) -> Option<[u8; 8]> {
    if b.len() > 13 && b[0] == 22 && b[13] == 1 {
        return routing_cid(b);
    }
    let mut p = 0;
    while p + 13 <= b.len() {
        if b[p] == 25 {
            return b.get(p + 11..p + 19)?.try_into().ok();
        }
        let n = u16::from_be_bytes(b[p + 11..p + 13].try_into().ok()?) as usize;
        p = p.checked_add(13 + n)?;
    }
    None
}
fn outbound_batches(c: &mut Connection) -> Result<Vec<Vec<u8>>> {
    let mut batches = Vec::new();
    let mut batch = Vec::new();
    loop {
        let record = c
            .pop()
            .map_err(|_| anyhow::anyhow!("DTLS outbound record error"))?;
        if record.is_empty() {
            break;
        }
        ensure!(record.len() <= 1232, "DTLS record exceeds probe MTU");
        if batch.len() + record.len() > 1232 {
            ensure!(
                datagram_cid(&batch).is_some(),
                "Handshake flight exceeds single routable datagram bound"
            );
            batches.push(std::mem::take(&mut batch));
        }
        batch.extend(record);
    }
    if !batch.is_empty() {
        ensure!(
            datagram_cid(&batch).is_some(),
            "Outgoing handshake datagram has no Phaze routing CID"
        );
        batches.push(batch);
    }
    Ok(batches)
}

async fn flush(c: &mut Connection, socket: &impl crate::Datagram, peer: SocketAddr) -> Result<u64> {
    let mut bytes = 0;
    for packet in outbound_batches(c)? {
        bytes += socket.send_to(&packet, peer).await? as u64;
    }
    Ok(bytes)
}
fn send(c: &mut Connection, b: &[u8]) -> Result<()> {
    let mut app = vec![0xcd, 2];
    app.extend(b);
    c.send(&app)
        .map_err(|_| anyhow::anyhow!("DTLS application write failed"))
}
async fn release_and_close(
    c: &mut Connection,
    reliable: &mut wire::Reliable,
    socket: &impl crate::Datagram,
    peer: SocketAddr,
) {
    if let Ok(packets) = reliable.frame(&crate::Input::Release.encode(0)) {
        for packet in packets {
            let _ = send(c, &packet);
        }
    }
    // Close is outside the reliable window, so a full/lost input queue cannot
    // prevent the host from ending this peer and releasing its pressed state.
    let _ = c.send(&[0xcd, 1]);
    let _ = flush(c, socket, peer).await;
}
async fn handle_stun(
    s: &impl crate::Datagram,
    b: &[u8],
    src: SocketAddr,
    own: &str,
    pwd: &str,
    a: &Answer,
) -> Result<()> {
    if b.len() < 20 {
        return Ok(());
    }
    if let Ok(Some(username)) = check_stun(b, &a.pwd) {
        if b[..2] == [0, 1] && username == format!("{}:{own}", a.ufrag).as_bytes() {
            s.send_to(&reply_stun(b[8..20].try_into().unwrap(), src, pwd)?, src)
                .await?;
        }
    }
    Ok(())
}
fn routing_cid(b: &[u8]) -> Option<[u8; 8]> {
    if b.len() < 60 || b[..3] != [22, 0xfe, 0xfd] || b[13] != 1 {
        return None;
    }
    let end = (13 + u16::from_be_bytes(b[11..13].try_into().ok()?) as usize).min(b.len());
    let mut p = 60 + b[59] as usize;
    let n = *b.get(p)? as usize;
    p = p.checked_add(1 + n)?;
    let n = u16::from_be_bytes(b.get(p..p + 2)?.try_into().ok()?) as usize;
    p = p.checked_add(2 + n)?;
    let n = *b.get(p)? as usize;
    p = p.checked_add(1 + n + 2)?;
    while p + 4 <= end {
        let ty = u16::from_be_bytes(b[p..p + 2].try_into().ok()?);
        let n = u16::from_be_bytes(b[p + 2..p + 4].try_into().ok()?) as usize;
        p += 4;
        if p + n > end {
            return None;
        }
        if ty == 0x36 {
            if n != 9 || b[p] != 8 {
                return None;
            }
            return b.get(p + 1..p + 9)?.try_into().ok();
        }
        p += n;
    }
    None
}
pub async fn run(
    socket: impl crate::Datagram,
    identity: Identity,
    own_cid: u32,
    ufrag: &str,
    pwd: &str,
    a: Answer,
    sink: &mut impl Sink,
) -> Result<()> {
    sink.stage("probing_authenticated_stun");
    let peer = tokio::time::timeout(
        Duration::from_secs(30),
        select_peer(&socket, ufrag, pwd, &a),
    )
    .await
    .context("Phaze candidate probing timed out")??;
    sink.stage("stun_authenticated");
    let cid = ((own_cid as u64) << 32 | a.cid as u64)
        .to_le_bytes()
        .to_vec();
    let config = Config::builder()
        .rng(Arc::new(purecrypto::rng::OsRng))
        .dtls()
        .versions(ProtocolVersion::DTLSv1_2, ProtocolVersion::DTLSv1_2)
        .cipher_suites(&[0xc02c, 0xc02b])
        // Phaze 0.13.2 omits EMS in its observed ServerHello. This experimental adapter allows legacy DTLS 1.2, still offers EMS and pins
        // the peer certificate to authenticated signaling before sending data.
        .require_extended_master_secret(false)
        .identity(vec![identity.der], SigningKey::Ecdsa(identity.key))
        .verify_certificates(false)
        .connection_id(cid)
        .max_record_size(1232)
        .build();
    // Phaze uses an authenticated-signaling DER fingerprint, not a public CA name.
    // Never send hello/request until that fingerprint has been checked below.
    let mut c = Connection::client(&config)
        .map_err(|_| anyhow::anyhow!("Cannot initialize Rust DTLS client"))?;
    let start = Instant::now();
    let mut buf = vec![0; 65536];
    let mut tick = tokio::time::interval(Duration::from_millis(4));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    sink.stage("dtls_handshaking");
    c.handshake()
        .map_err(|_| anyhow::anyhow!("DTLS initial handshake error"))?;
    flush(&mut c, &socket, peer).await?;
    while !c.is_handshake_complete() {
        ensure!(
            start.elapsed() < Duration::from_secs(25),
            "DTLS handshake timeout"
        );
        tokio::select! {
            _=tick.tick()=>{
                if c.next_timeout().is_some_and(|t|start.elapsed()>=t) {
                    c.on_timeout(start.elapsed());
                }
            },
            r=socket.recv_from(&mut buf)=>{
                let(n,src)=match r {
                    Ok(v)=>v,
                    Err(err) if unreachable(&err)=>continue,
                    Err(_)=>bail!("Phaze UDP handshake receive failed"),
                };
                let Some(src)=socket.endpoint(src) else { continue };
                if n>=8 && buf[4..8]==0x2112a442u32.to_be_bytes() {
                    handle_stun(&socket,&buf[..n],src,ufrag,pwd,&a).await?;
                    continue;
                }
                if src!=peer {continue;}
                c.set_now(start.elapsed());
                c.feed(&buf[..n]).map_err(|_|anyhow::anyhow!("DTLS peer record rejected"))?;
                c.handshake().map_err(|_|anyhow::anyhow!("DTLS handshake failed"))?;
            }
        }
        flush(&mut c, &socket, peer).await?;
    }
    let cert = c
        .peer_certificates()
        .first()
        .context("DTLS peer provided no certificate")?;
    ensure!(
        hex(&Sha256::digest(cert)) == a.cert_hash.to_ascii_lowercase(),
        "DTLS peer fingerprint does not match authenticated answer"
    );
    sink.stage("dtls_authenticated");

    let mut reliable = wire::Reliable::default();
    let mut audio = crate::audio::Audio::default();
    let mut cursors = crate::cursor::Cursors::default();
    for p in reliable.frame(&wire::hello(sink.raw_audio()))? {
        send(&mut c, &p)?;
    }
    let transmission_started = Instant::now();
    flush(&mut c, &socket, peer).await?;
    reliable.transmitted(transmission_started);
    sink.stage("waiting_outputs");
    let mut requested = false;
    let app_started = Instant::now();
    let mut video_received = false;
    let mut last_heartbeat = Instant::now();
    let mut heartbeat_received = false;
    let mut heartbeat_timer = tokio::time::interval(Duration::from_millis(200));
    heartbeat_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut selected_output = None;
    let mut input = Vec::new();
    let mut deferred_motion = None;
    let mut outputs_available = Vec::new();
    let (mut received, mut sent) = (0u64, 0u64);
    let mut reported = Instant::now();
    let mut arrival = reported;
    let mut bandwidth_reported = false;
    loop {
        ensure!(
            last_heartbeat.elapsed() < Duration::from_secs(20),
            "Phaze peer heartbeat timed out"
        );
        ensure!(
            video_received || app_started.elapsed() < Duration::from_secs(20),
            "Phaze first video timed out"
        );
        let mut stream_request = None;
        let mut bandwidth_ack_sent = false;
        tokio::select! {
            _=heartbeat_timer.tick()=>{
                c.send(&wire::HEARTBEAT).map_err(|_|anyhow::anyhow!("Phaze heartbeat send failed"))?;
            },
            _=tick.tick()=>{
                if c.next_timeout().is_some_and(|t|start.elapsed()>=t){c.on_timeout(start.elapsed());}
                input.clear();
                let running=sink.poll(selected_output,&mut input);
                if !matches!(running,Ok(true)) {
                    release_and_close(&mut c,&mut reliable,&socket,peer).await;
                    return running.map(|_|());
                }
                if let Some((output,config))=sink.stream_request() {
                    ensure!(outputs_available.iter().any(|o: &wire::Output| o.id==output && o.active), "Phaze display is unavailable");
                    if selected_output != Some(output) {
                        deferred_motion = None;
                        if let Some(previous)=selected_output {
                            let mut stop=vec![4]; stop.extend(u32::to_be_bytes(previous));
                            for p in reliable.frame(&stop)? { send(&mut c,&p)?; }
                        }
                    }
                    for p in reliable.frame(&config.request(output)?)? { send(&mut c,&p)?; }
                    selected_output=Some(output);
                    stream_request=Some((output,config));
                }
                if let Some(output)=selected_output {
                    // A newer position supersedes a deferred position. A release
                    // clears it, so it cannot move the pointer after focus loss.
                    let deferred = deferred_motion.take().filter(|_| {
                        !input.first().is_some_and(|event| {
                            event.is_absolute_motion() || *event == crate::Input::Release
                        })
                    });
                    let mut commands = deferred.into_iter().chain(input.drain(..)).peekable();
                    while let Some(command) = commands.next() {
                        let outstanding = reliable.pending().count();
                        // Preserve room for button and key transitions when the
                        // peer is slow to acknowledge a burst of positions.
                        if outstanding >= 48 && command.is_absolute_motion() && commands.peek().is_none() {
                            deferred_motion = Some(command);
                            break;
                        }
                        if outstanding >= 63 {
                            release_and_close(&mut c,&mut reliable,&socket,peer).await;
                            bail!("Phaze input acknowledgment window exhausted; reconnect to continue");
                        }
                        for p in reliable.frame(&command.encode(output))? { send(&mut c,&p)?; }
                    }
                }
            },
            r=socket.recv_from(&mut buf)=>{
                let(n,src)=match r {
                    Ok(v)=>v,
                    Err(err) if unreachable(&err)=>continue,
                    Err(_)=>bail!("Phaze UDP receive failed")
                };
                let Some(src)=socket.endpoint(src) else { continue };
                if src!=peer { continue }
                if n>=8 && buf[4..8]==0x2112a442u32.to_be_bytes() {
                    handle_stun(&socket,&buf[..n],src,ufrag,pwd,&a).await?;
                    continue
                }
                received=received.saturating_add(n as u64);
                arrival=Instant::now();
                c.set_now(start.elapsed());
                c.feed(&buf[..n]).map_err(|_|anyhow::anyhow!("Phaze DTLS record rejected"))?;
            }
        }
        loop {
            let b = c
                .recv()
                .map_err(|_| anyhow::anyhow!("Phaze DTLS read failed"))?;
            if b.is_empty() {
                break;
            }
            let payload = match wire::envelope(&b) {
                Some(wire::Envelope::Heartbeat) => {
                    last_heartbeat = Instant::now();
                    if !heartbeat_received {
                        heartbeat_received = true;
                        sink.stage("peer_heartbeat_received");
                    }
                    continue;
                }
                Some(wire::Envelope::Close) => bail!("Phaze host closed the connection"),
                Some(wire::Envelope::Reliable(payload)) => payload,
                None => continue,
            };
            let (frames, ack) = reliable.receive_at(payload, arrival)?;
            if let Some(ack) = ack {
                send(&mut c, &ack)?;
                bandwidth_ack_sent |= ack[0] == 4;
            }
            for frame in frames {
                match frame.first() {
                    Some(2) => {
                        let outputs = wire::outputs(&frame[1..])?;
                        sink.outputs(&outputs);
                        if !requested {
                            let output = outputs
                                .iter()
                                .find(|o| o.active && o.primary)
                                .or_else(|| outputs.iter().find(|o| o.active))
                                .context("Phaze host has no active display")?;
                            let config = sink.video_config();
                            for p in reliable.frame(&config.request(output.id)?)? {
                                send(&mut c, &p)?;
                            }
                            selected_output = Some(output.id);
                            requested = true;
                            stream_request = Some((output.id, config));
                            sink.stage("video_requested");
                        }
                        outputs_available = outputs;
                    }
                    Some(5) if requested => {
                        let bytes = wire::encoded(&frame[1..])?;
                        let flags = u32::from_be_bytes(frame[9..13].try_into().unwrap());
                        sink.video(bytes, flags & 1 != 0)?;
                        video_received |= !bytes.is_empty();
                    }
                    Some(6) => {
                        if let Some(visible) = crate::cursor::visibility(&frame[1..]) {
                            sink.cursor_visibility(visible);
                        }
                    }
                    Some(7) => {
                        let (codec, pcm, payload_bytes) = audio.decode(&frame[1..])?;
                        sink.audio(pcm, codec, payload_bytes)?;
                    }
                    Some(12 | 19) => {
                        if let Some(image) = cursors.receive(&frame) {
                            sink.cursor_image(image);
                        }
                    }
                    _ => {}
                }
            }
        }
        reliable.retry_due(Instant::now(), |packet| send(&mut c, packet))?;
        let transmission_started = Instant::now();
        sent = sent.saturating_add(flush(&mut c, &socket, peer).await?);
        reliable.transmitted(transmission_started);
        if bandwidth_ack_sent && !bandwidth_reported {
            bandwidth_reported = true;
            sink.bandwidth_feedback_started();
        }
        if let Some((output, config)) = stream_request {
            sink.stream_request_sent(output, config);
        }
        if reported.elapsed() >= Duration::from_millis(500) {
            sink.rtt(reliable.rtt(Instant::now()));
            sink.traffic(received, sent);
            reported = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stun_requires_integrity_and_fingerprint() -> Result<()> {
        let mut packet = header(1, [3; 12]);
        attr(&mut packet, 6, b"fixture:user");
        finish_stun(&mut packet, "fixture-key")?;
        assert_eq!(
            check_stun(&packet, "fixture-key")?,
            Some(&b"fixture:user"[..])
        );
        assert!(check_stun(&packet, "wrong-key").is_err());
        packet[24] ^= 1;
        assert!(check_stun(&packet, "fixture-key").is_err());
        Ok(())
    }
    #[test]
    fn dtls_loopback_test() -> Result<()> {
        legacy_server_hello_test()?;
        let client = Identity::generate()?;
        let server = Identity::generate()?;
        let expected = server.fingerprint.clone();
        let client_expected = client.fingerprint.clone();
        let cid = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let base = || {
            Config::builder()
                .rng(Arc::new(purecrypto::rng::OsRng))
                .dtls()
                .versions(ProtocolVersion::DTLSv1_2, ProtocolVersion::DTLSv1_2)
                .cipher_suites(&[0xc02c])
                .verify_certificates(false)
                .connection_id(cid.clone())
        };
        let mut client_roots = purecrypto::tls::RootCertStore::new();
        client_roots
            .add_der(client.der.clone())
            .map_err(|e| anyhow::anyhow!("Loopback trust root: {e:?}"))?;
        let cc = base()
            .require_extended_master_secret(false)
            .identity(vec![client.der], SigningKey::Ecdsa(client.key))
            .build();
        let mut cookie = [0u8; 32];
        getrandom::fill(&mut cookie).map_err(|_| anyhow::anyhow!("OS randomness unavailable"))?;
        let sc = base()
            .cookie_secret(cookie)
            .peer_address(b"loopback-fixture".to_vec())
            .identity(vec![server.der], SigningKey::Ecdsa(server.key))
            .client_auth(purecrypto::tls::ClientAuth::new(client_roots, true))
            .build();
        let mut c = Connection::client(&cc)
            .map_err(|e| anyhow::anyhow!("Loopback client configuration: {e:?}"))?;
        let mut s = Connection::server(&sc)
            .map_err(|e| anyhow::anyhow!("Loopback server configuration: {e:?}"))?;
        c.handshake()
            .map_err(|e| anyhow::anyhow!("Loopback start: {e:?}"))?;
        for _ in 0..40 {
            for p in outbound_batches(&mut c)? {
                ensure!(
                    datagram_cid(&p).as_ref().map(|v| v.as_slice()) == Some(cid.as_slice()),
                    "Every client handshake datagram must be routable by the installed host"
                );
                if p.len() > 13 && p[0] == 22 && p[13] == 1 {
                    ensure!(
                        routing_cid(&p).as_ref().map(|v| v.as_slice()) == Some(cid.as_slice()),
                        "ClientHello not routable by installed Phaze host CID parser"
                    );
                }

                s.feed(&p)
                    .map_err(|e| anyhow::anyhow!("Loopback server feed: {e:?}"))?;
                s.handshake()
                    .map_err(|e| anyhow::anyhow!("Loopback server handshake: {e:?}"))?;
            }
            for _ in 0..20 {
                let p = s
                    .pop()
                    .map_err(|e| anyhow::anyhow!("Loopback pop: {e:?}"))?;
                if p.is_empty() {
                    break;
                }

                c.feed(&p)
                    .map_err(|e| anyhow::anyhow!("Loopback client feed: {e:?}"))?;
                c.handshake()
                    .map_err(|e| anyhow::anyhow!("Loopback client handshake: {e:?}"))?;
            }
            if c.is_handshake_complete() && s.is_handshake_complete() {
                break;
            }
        }
        ensure!(
            c.is_handshake_complete() && s.is_handshake_complete(),
            "Loopback did not complete"
        );
        ensure!(
            c.negotiated_cipher_suite() == Some(0xc02c),
            "Wrong loopback suite"
        );
        ensure!(
            c.local_connection_id() == Some(cid.as_slice())
                && c.peer_connection_id() == Some(cid.as_slice()),
            "CID negotiation failed"
        );
        ensure!(
            hex(&Sha256::digest(&c.peer_certificates()[0])) == expected,
            "Server fingerprint test"
        );
        ensure!(
            hex(&Sha256::digest(&s.peer_certificates()[0])) == client_expected,
            "Client fingerprint test"
        );
        c.send(&wire::HEARTBEAT)
            .map_err(|e| anyhow::anyhow!("Loopback send: {e:?}"))?;
        loop {
            let p = c
                .pop()
                .map_err(|e| anyhow::anyhow!("Loopback pop: {e:?}"))?;
            if p.is_empty() {
                break;
            }
            s.feed(&p)
                .map_err(|e| anyhow::anyhow!("Loopback data: {e:?}"))?;
        }
        ensure!(
            s.recv()
                .map_err(|e| anyhow::anyhow!("Loopback recv: {e:?}"))?
                == wire::HEARTBEAT,
            "Encrypted CID data roundtrip test"
        );
        Ok(())
    }

    fn legacy_server_hello_test() -> Result<()> {
        // Synthetic, unauthenticated ServerHello only: acceptance is not a
        // completed or authenticated handshake. No live packet is rewritten.
        let mut body = vec![0xfe, 0xfd];
        body.extend([0u8; 32]);
        body.extend([0, 0xc0, 0x2c, 0, 0, 0]); // empty SID/extensions, c02c, null compression
        let mut handshake = vec![
            2,
            0,
            0,
            body.len() as u8,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            body.len() as u8,
        ];
        handshake.extend(body);
        let mut packet = vec![22, 0xfe, 0xfd, 0, 0, 0, 0, 0, 0, 0, 0];
        packet.extend((handshake.len() as u16).to_be_bytes());
        packet.extend(handshake);
        for require_ems in [true, false] {
            let config = Config::builder()
                .rng(Arc::new(purecrypto::rng::OsRng))
                .dtls()
                .versions(ProtocolVersion::DTLSv1_2, ProtocolVersion::DTLSv1_2)
                .cipher_suites(&[0xc02c])
                .require_extended_master_secret(require_ems)
                .verify_certificates(false)
                .build();
            let mut client = Connection::client(&config)
                .map_err(|_| anyhow::anyhow!("Synthetic EMS client configuration"))?;
            client
                .handshake()
                .map_err(|_| anyhow::anyhow!("Synthetic EMS start"))?;
            client
                .feed(&packet)
                .map_err(|_| anyhow::anyhow!("Synthetic EMS feed"))?;
            ensure!(
                client.negotiated_cipher_suite() == if require_ems { None } else { Some(0xc02c) },
                "Missing EMS must explain strict ServerHello rejection"
            );
            ensure!(
                !client.is_handshake_complete() && client.peer_certificates().is_empty(),
                "Synthetic ServerHello must not authenticate a peer"
            );
        }
        Ok(())
    }
    #[tokio::test]
    async fn authenticated_stun_learns_a_peer_reflexive_nat_port() -> Result<()> {
        let socket = local_socket()?;
        let client = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), socket.local_addr()?.port());
        let unused = UdpSocket::bind("127.0.0.1:0").await?;
        let server = UdpSocket::bind("127.0.0.1:0").await?;
        let actual = server.local_addr()?;
        let answer = Answer {
            ufrag: "host".into(),
            pwd: zeroize::Zeroizing::new("host-fixture-secret".into()),
            cert_hash: "0".repeat(64),
            cid: 1,
            candidates: vec![super::super::Candidate {
                addr: "127.0.0.1".into(),
                port: unused.local_addr()?.port(),
            }],
        };
        let peer = async {
            let mut request = header(1, [7; 12]);
            attr(&mut request, 6, b"host:client");
            finish_stun(&mut request, "host-fixture-secret")?;
            server.send_to(&request, client).await?;
            let mut bytes = [0; 2048];
            loop {
                let (n, from) = server.recv_from(&mut bytes).await?;
                if bytes[..2] == [0, 1] {
                    assert_eq!(
                        check_stun(&bytes[..n], "client-fixture-secret")?,
                        Some(b"client:host".as_slice())
                    );
                    let id = bytes[8..20].try_into().unwrap();
                    server
                        .send_to(&reply_stun(id, from, "host-fixture-secret")?, from)
                        .await?;
                    break;
                }
            }
            Ok::<_, anyhow::Error>(())
        };
        let (selected, ()) = tokio::time::timeout(Duration::from_secs(3), async {
            tokio::try_join!(
                select_peer(&socket, "client", "client-fixture-secret", &answer),
                peer
            )
        })
        .await??;
        assert_eq!(selected, dual_endpoint(actual));
        Ok(())
    }
}
