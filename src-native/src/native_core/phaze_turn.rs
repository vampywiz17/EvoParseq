//! Isolated proof of the production Rust TURN carrier; no user credentials.
use evoparseq_phaze::{
    turn::{Carrier, Relay},
    Datagram,
};
use std::{
    io,
    net::SocketAddr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, UdpSocket},
    task::JoinHandle,
};
use webrtc::{
    turn::{
        auth::{generate_auth_key, AuthHandler},
        relay::relay_static::RelayAddressGeneratorStatic,
        server::{
            config::{ConnConfig, ServerConfig},
            Server,
        },
    },
    util::vnet::net::Net,
};
struct Auth;
impl AuthHandler for Auth {
    fn auth_handle(
        &self,
        username: &str,
        realm: &str,
        _: SocketAddr,
    ) -> std::result::Result<Vec<u8>, webrtc::turn::Error> {
        assert_eq!(username, "phaze-fixture");
        assert_eq!(realm, "isolated-fixture");
        Ok(generate_auth_key(username, realm, "fixture-only-password"))
    }
}
async fn server() -> anyhow::Result<(Server, SocketAddr)> {
    let listener = Arc::new(UdpSocket::bind("127.0.0.1:0").await?);
    let address = listener.local_addr()?;
    let server = Server::new(ServerConfig {
        conn_configs: vec![ConnConfig {
            conn: listener,
            relay_addr_generator: Box::new(RelayAddressGeneratorStatic {
                relay_address: "127.0.0.1".parse()?,
                address: "127.0.0.1".into(),
                net: Arc::new(Net::new(None)),
            }),
        }],
        realm: "isolated-fixture".into(),
        auth_handler: Arc::new(Auth),
        channel_bind_timeout: Duration::ZERO,
        alloc_close_notify: None,
    })
    .await?;
    Ok((server, address))
}
trait Stream: AsyncRead + AsyncWrite + Unpin + Send + Sync {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + Sync> Stream for T {}
fn observe(packet: &[u8], refresh: &AtomicUsize, permission: &AtomicUsize) {
    if packet.starts_with(&[0, 4]) {
        refresh.fetch_add(1, Ordering::Relaxed);
    }
    if packet.starts_with(&[0, 8]) {
        permission.fetch_add(1, Ordering::Relaxed);
    }
}
async fn stream_proxy(
    target: SocketAddr,
    tls: bool,
) -> anyhow::Result<(SocketAddr, Vec<Vec<u8>>, JoinHandle<io::Result<()>>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let certificate =
        rcgen::generate_simple_self_signed(vec!["localhost".into(), "127.0.0.1".into()])?;
    let roots = vec![certificate.cert.der().to_vec()];
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_no_client_auth()
    .with_single_cert(
        vec![certificate.cert.der().clone()],
        rustls::pki_types::PrivatePkcs8KeyDer::from(certificate.key_pair.serialize_der()).into(),
    )?;
    let task = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await?;
        tcp.set_nodelay(true)?;
        let stream: Box<dyn Stream> = if tls {
            Box::new(
                tokio_rustls::TlsAcceptor::from(Arc::new(config))
                    .accept(tcp)
                    .await?,
            )
        } else {
            Box::new(tcp)
        };
        let (mut reader, mut writer) = tokio::io::split(stream);
        let udp = UdpSocket::bind("127.0.0.1:0").await?;
        udp.connect(target).await?;
        let up = async {
            let mut packet = vec![0; 65556];
            loop {
                reader.read_exact(&mut packet[..4]).await?;
                let n = 20 + usize::from(u16::from_be_bytes([packet[2], packet[3]]));
                reader.read_exact(&mut packet[4..n]).await?;
                udp.send(&packet[..n]).await?;
            }
            #[allow(unreachable_code)]
            Ok::<(), io::Error>(())
        };
        let down = async {
            let mut packet = vec![0; 65556];
            loop {
                let n = udp.recv(&mut packet).await?;
                // Split every TURN frame deliberately, exercising TCP and TLS read boundaries.
                writer.write_all(&packet[..3]).await?;
                writer.flush().await?;
                tokio::task::yield_now().await;
                writer.write_all(&packet[3..n]).await?;
                writer.flush().await?;
            }
            #[allow(unreachable_code)]
            Ok::<(), io::Error>(())
        };
        tokio::select! {result=up=>result,result=down=>result}
    });
    Ok((address, roots, task))
}
async fn roundtrip(relay: &Relay) -> anyhow::Result<()> {
    let advertised = relay.address();
    let peer = UdpSocket::bind("127.0.0.1:0").await?;
    let target = peer.local_addr()?;
    let mut received = [0; 2048];
    assert_eq!((&relay).candidates()?, vec![advertised]);
    for size in [20usize, 35, 1232] {
        let payload: Vec<_> = (0..size).map(|i| (i % 251) as u8).collect();
        (&relay).send_to(&payload, target).await?;
        let (n, from) = peer.recv_from(&mut received).await?;
        assert_eq!(from, advertised);
        assert_eq!(&received[..n], payload);
        peer.send_to(&payload, advertised).await?;
        let (n, from) = (&relay).recv_from(&mut received).await?;
        assert_eq!((&relay).endpoint(from), Some(target));
        assert_eq!(&received[..n], payload);
    }
    Ok(())
}
#[tokio::test]
async fn production_phaze_turn_udp_tcp_tls_are_opaque_and_tls_is_verified() -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(30), async {
        let (server, address) = server().await?;
        let udp = Relay::allocate(
            "127.0.0.1",
            address.port(),
            Carrier::Udp,
            "phaze-fixture",
            "fixture-only-password",
        )
        .await?;
        roundtrip(&udp).await?;
        udp.close().await;
        for carrier in [Carrier::Tcp, Carrier::Tls] {
            let (proxy, roots, task) = stream_proxy(address, carrier == Carrier::Tls).await?;
            let relay = Relay::allocate_trusted(
                "127.0.0.1",
                proxy.port(),
                carrier,
                "phaze-fixture",
                "fixture-only-password",
                &roots,
            )
            .await?;
            roundtrip(&relay).await?;
            relay.close().await;
            drop(relay);
            task.abort();
        }
        let (proxy, _, task) = stream_proxy(address, true).await?;
        assert!(Relay::allocate(
            "127.0.0.1",
            proxy.port(),
            Carrier::Tls,
            "phaze-fixture",
            "fixture-only-password"
        )
        .await
        .is_err());
        task.abort();
        server.close().await?;
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    Ok(())
}
#[tokio::test]
async fn production_phaze_turn_renews_allocation_and_peer_permissions() -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(150),async {
        let(server,target)=server().await?;
        let listener=UdpSocket::bind("127.0.0.1:0").await?;let address=listener.local_addr()?;
        let refresh=Arc::new(AtomicUsize::new(0));let permission=Arc::new(AtomicUsize::new(0));
        let observed=(refresh.clone(),permission.clone());
        let proxy=tokio::spawn(async move {
            let upstream=UdpSocket::bind("127.0.0.1:0").await?;upstream.connect(target).await?;
            let(mut up,mut down)=(vec![0;65556],vec![0;65556]);let mut client=None;
            loop {tokio::select! {
                result=listener.recv_from(&mut up)=>{let(n,from)=result?;client=Some(from);observe(&up[..n],&observed.0,&observed.1);upstream.send(&up[..n]).await?;},
                result=upstream.recv(&mut down)=>{let n=result?;if let Some(client)=client{listener.send_to(&down[..n],client).await?;}},
            }} #[allow(unreachable_code)] Ok::<(),io::Error>(())
        });
        let relay=Relay::allocate("127.0.0.1",address.port(),Carrier::Udp,"phaze-fixture","fixture-only-password").await?;
        roundtrip(&relay).await?;assert_eq!(permission.load(Ordering::Relaxed),1);
        tokio::time::timeout(Duration::from_secs(135),async { while refresh.load(Ordering::Relaxed)<1 || permission.load(Ordering::Relaxed)<2 {tokio::time::sleep(Duration::from_millis(250)).await;} }).await?;
        assert!(refresh.load(Ordering::Relaxed)>=1);assert!(permission.load(Ordering::Relaxed)>=2);
        roundtrip(&relay).await?;relay.close().await;drop(relay);proxy.abort();server.close().await?;
        Ok::<_,anyhow::Error>(())
    }).await??;
    Ok(())
}
