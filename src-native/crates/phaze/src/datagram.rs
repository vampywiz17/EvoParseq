//! Datagram carrier boundary: TURN transports the same authenticated Phaze
//! packets. The embedding client owns allocation, permissions and renewal.
use std::{future::Future, io, net::SocketAddr};

pub trait Datagram: Sync {
    fn candidates(&self) -> io::Result<Vec<SocketAddr>>;
    /// Ask the native host to send its short-TTL NAT-opening probe to the
    /// advertised address. Local-interface candidates do not need this.
    fn candidate_short_ttl(&self) -> bool {
        false
    }
    /// Canonical socket-family representation; None means unsupported family.
    fn endpoint(&self, address: SocketAddr) -> Option<SocketAddr>;
    fn send_to(
        &self,
        data: &[u8],
        peer: SocketAddr,
    ) -> impl Future<Output = io::Result<usize>> + Send;
    fn recv_from(
        &self,
        data: &mut [u8],
    ) -> impl Future<Output = io::Result<(usize, SocketAddr)>> + Send;

    /// Optional kernel receive timestamp, in nanoseconds on the socket's clock.
    fn recv_from_timed(
        &self,
        data: &mut [u8],
    ) -> impl Future<Output = io::Result<(usize, SocketAddr, Option<i64>)>> + Send {
        async {
            let (len, peer) = self.recv_from(data).await?;
            Ok((len, peer, None))
        }
    }
}

impl Datagram for tokio::net::UdpSocket {
    fn candidates(&self) -> io::Result<Vec<SocketAddr>> {
        super::transport::local_candidates(self.local_addr()?.port()).map_err(io::Error::other)
    }
    fn endpoint(&self, address: SocketAddr) -> Option<SocketAddr> {
        Some(super::transport::dual_endpoint(address))
    }
    async fn send_to(&self, data: &[u8], peer: SocketAddr) -> io::Result<usize> {
        self.send_to(data, peer).await
    }
    async fn recv_from(&self, data: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        self.recv_from(data).await
    }
}
