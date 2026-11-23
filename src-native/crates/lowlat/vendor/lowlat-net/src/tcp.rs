//! Shared native stream readiness; TLS remains above the platform socket.

#[derive(Debug, Default)]
pub(crate) struct Ready {
    pub readable: bool,
    pub writable: bool,
    pub error: bool,
}

pub(crate) fn normalize(addr: std::net::SocketAddr) -> std::net::SocketAddr {
    match addr {
        std::net::SocketAddr::V6(addr) => addr
            .ip()
            .to_ipv4_mapped()
            .map_or(std::net::SocketAddr::V6(addr), |ip| {
                std::net::SocketAddr::from((ip, addr.port()))
            }),
        addr => addr,
    }
}
