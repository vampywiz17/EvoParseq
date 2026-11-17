//! Isolated TURN feasibility fixture, not a production Phaze relay backend.
//! Real host candidate acceptance and end-to-end authentication still need a live test.
use evoparseq_phaze::Datagram;
use std::{io, net::SocketAddr, sync::Arc, time::Duration};
use webrtc::{
    turn::{
        auth::{generate_auth_key, AuthHandler},
        client::{Client, ClientConfig},
        relay::relay_static::RelayAddressGeneratorStatic,
        server::{
            config::{ConnConfig, ServerConfig},
            Server,
        },
    },
    util::{conn::Conn, vnet::net::Net},
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
struct Carrier(Arc<dyn Conn + Send + Sync>);
impl Datagram for Carrier {
    fn candidates(&self) -> io::Result<Vec<SocketAddr>> {
        Ok(vec![self.0.local_addr().map_err(io::Error::other)?])
    }
    fn endpoint(&self, address: SocketAddr) -> Option<SocketAddr> {
        match address {
            SocketAddr::V4(_) => Some(address),
            SocketAddr::V6(a) => a
                .ip()
                .to_ipv4_mapped()
                .map(|ip| SocketAddr::new(ip.into(), a.port())),
        }
    }
    async fn send_to(&self, data: &[u8], peer: SocketAddr) -> io::Result<usize> {
        self.0.send_to(data, peer).await.map_err(io::Error::other)
    }
    async fn recv_from(&self, data: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        self.0.recv_from(data).await.map_err(io::Error::other)
    }
}
#[tokio::test]
async fn opaque_phaze_datagrams_survive_authenticated_turn_in_both_directions() -> anyhow::Result<()>
{
    tokio::time::timeout(Duration::from_secs(10), async {
        let listener = Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await?);
        let server_address = listener.local_addr()?;
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
        let socket = Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await?);
        let client = Client::new(ClientConfig {
            stun_serv_addr: String::new(),
            turn_serv_addr: server_address.to_string(),
            username: "phaze-fixture".into(),
            password: "fixture-only-password".into(),
            realm: String::new(),
            software: String::new(),
            rto_in_ms: 100,
            conn: socket.clone(),
            vnet: None,
        })
        .await?;
        client.listen().await?;
        let relay = Carrier(Arc::new(client.allocate().await?));
        let advertised = relay.candidates()?[0];
        assert_ne!(advertised, socket.local_addr()?);
        let peer = tokio::net::UdpSocket::bind("127.0.0.1:0").await?;
        let target = peer.local_addr()?;
        let mut received = [0; 2048];
        // Opaque wire-shaped fixtures: STUN, CID record and maximum current
        // Phaze datagram. This proves forwarding, not a native host handshake.
        for size in [20usize, 35, 1232] {
            let payload: Vec<_> = (0..size).map(|i| (i % 251) as u8).collect();
            relay.send_to(&payload, target).await?;
            let (n, from) = peer.recv_from(&mut received).await?;
            assert_eq!(from, advertised);
            assert_eq!(&received[..n], payload);
            peer.send_to(&payload, advertised).await?;
            let (n, from) = relay.recv_from(&mut received).await?;
            assert_eq!(relay.endpoint(from), Some(target));
            assert_eq!(&received[..n], payload);
        }
        relay.0.close().await?;
        client.close().await?;
        socket.close().await?;
        server.close().await?;
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    Ok(())
}
