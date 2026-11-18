//! Opt-in comparison of kernel and application receive times for direct Phaze.
use evoparseq_phaze::Datagram;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    io,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6},
    os::fd::AsRawFd,
    sync::Mutex,
    time::{Duration, Instant},
};
use tokio::{io::Interest, net::UdpSocket};

// Linux UAPI SO_TIMESTAMPNS_NEW uses two i64 fields on both Android ABIs.
const SO_TIMESTAMPNS_NEW: libc::c_int = 64;
const BURST_PACKETS: usize = 24;
const MAX_BURSTS: u8 = 8;

pub fn enable(socket: UdpSocket) -> Option<TracedSocket> {
    let enabled: libc::c_int = 1;
    // The descriptor stays live and the option value is one initialized int.
    let result = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            SO_TIMESTAMPNS_NEW,
            std::ptr::from_ref(&enabled).cast(),
            size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    crate::event_log::record(
        "INFO",
        "phaze_android_rx_timestamping",
        json!({"kernel_timestamps_enabled": result == 0}),
    );
    (result == 0).then(|| TracedSocket {
        socket,
        trace: Mutex::new(Trace {
            next: Instant::now() + Duration::from_secs(5),
            remaining: MAX_BURSTS,
            samples: Vec::with_capacity(BURST_PACKETS),
        }),
    })
}

pub struct TracedSocket {
    socket: UdpSocket,
    trace: Mutex<Trace>,
}

impl Datagram for TracedSocket {
    fn candidates(&self) -> io::Result<Vec<SocketAddr>> {
        Datagram::candidates(&self.socket)
    }

    fn endpoint(&self, address: SocketAddr) -> Option<SocketAddr> {
        Datagram::endpoint(&self.socket, address)
    }

    async fn send_to(&self, data: &[u8], peer: SocketAddr) -> io::Result<usize> {
        self.socket.send_to(data, peer).await
    }

    async fn recv_from(&self, data: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        let (len, peer, kernel_ns, read_at) = self
            .socket
            .async_io(Interest::READABLE, || {
                receive(self.socket.as_raw_fd(), data)
            })
            .await?;
        if len >= 1000 {
            if let Some(kernel_ns) = kernel_ns {
                self.trace.lock().unwrap_or_else(|e| e.into_inner()).sample(
                    &data[..len],
                    kernel_ns,
                    read_at,
                );
            }
        }
        Ok((len, peer))
    }
}

struct Trace {
    next: Instant,
    remaining: u8,
    samples: Vec<Sample>,
}

struct Sample {
    kernel_ns: i64,
    read_at: Instant,
    size: usize,
    tag: u64,
}

impl Trace {
    fn sample(&mut self, data: &[u8], kernel_ns: i64, read_at: Instant) {
        if self.remaining == 0 || read_at < self.next {
            return;
        }
        // A short hash of the encrypted datagram prefix matches the host trace
        // without writing any packet bytes to the Android diagnostic export.
        let hash = Sha256::digest(&data[..24]);
        self.samples.push(Sample {
            kernel_ns,
            read_at,
            size: data.len(),
            tag: u64::from_be_bytes(hash[..8].try_into().unwrap()),
        });
        if self.samples.len() == BURST_PACKETS {
            let first = &self.samples[0];
            let samples: Vec<_> = self
                .samples
                .iter()
                .map(|sample| {
                    json!({
                        "tag": format!("{:016x}", sample.tag),
                        "size": sample.size,
                        "kernel_ns": sample.kernel_ns - first.kernel_ns,
                        "read_ns": sample.read_at.duration_since(first.read_at).as_nanos() as u64,
                    })
                })
                .collect();
            crate::event_log::record(
                "INFO",
                "phaze_android_rx_timing",
                json!({"samples":samples}),
            );
            self.samples.clear();
            self.remaining -= 1;
            self.next = read_at + Duration::from_secs(5);
        }
    }
}

fn receive(
    fd: libc::c_int,
    data: &mut [u8],
) -> io::Result<(usize, SocketAddr, Option<i64>, Instant)> {
    // Aligned backing storage is required for cmsghdr and the sender address.
    let mut control = [0usize; 16];
    // All-zero is a valid initial value for these C output structures.
    let mut address: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    let mut buffer = libc::iovec {
        iov_base: data.as_mut_ptr().cast(),
        iov_len: data.len(),
    };
    message.msg_name = std::ptr::from_mut(&mut address).cast();
    message.msg_namelen = size_of::<libc::sockaddr_storage>() as libc::socklen_t;
    message.msg_iov = &mut buffer;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = size_of_val(&control);
    // All writable pointers refer to the initialized storage above for this call.
    let len = unsafe { libc::recvmsg(fd, &mut message, 0) };
    let read_at = Instant::now();
    if len < 0 {
        return Err(io::Error::last_os_error());
    }
    let address = match libc::c_int::from(address.ss_family) {
        libc::AF_INET if message.msg_namelen as usize >= size_of::<libc::sockaddr_in>() => {
            // The kernel reported an initialized IPv4 address in aligned storage.
            let addr = unsafe { *std::ptr::from_ref(&address).cast::<libc::sockaddr_in>() };
            SocketAddr::from((
                Ipv4Addr::from(addr.sin_addr.s_addr.to_ne_bytes()),
                u16::from_be(addr.sin_port),
            ))
        }
        libc::AF_INET6 if message.msg_namelen as usize >= size_of::<libc::sockaddr_in6>() => {
            // The kernel reported an initialized IPv6 address in aligned storage.
            let addr = unsafe { *std::ptr::from_ref(&address).cast::<libc::sockaddr_in6>() };
            SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::from(addr.sin6_addr.s6_addr),
                u16::from_be(addr.sin6_port),
                u32::from_be(addr.sin6_flowinfo),
                addr.sin6_scope_id,
            ))
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid UDP sender address",
            ))
        }
    };
    Ok((len as usize, address, timestamp(&message), read_at))
}

fn timestamp(message: &libc::msghdr) -> Option<i64> {
    if message.msg_flags & libc::MSG_CTRUNC != 0 {
        return None;
    }
    // recvmsg populated the ancillary buffer and its used length. Only read
    // the two time64 fields after validating the header and payload size.
    unsafe {
        let mut header = libc::CMSG_FIRSTHDR(message);
        while !header.is_null() {
            if (*header).cmsg_level == libc::SOL_SOCKET
                && (*header).cmsg_type == SO_TIMESTAMPNS_NEW
                && (*header).cmsg_len >= libc::CMSG_LEN(16) as usize
            {
                let data = libc::CMSG_DATA(header).cast::<[i64; 2]>();
                let [seconds, nanos] = data.read_unaligned();
                return seconds.checked_mul(1_000_000_000)?.checked_add(nanos);
            }
            header = libc::CMSG_NXTHDR(message, header);
        }
    }
    None
}
