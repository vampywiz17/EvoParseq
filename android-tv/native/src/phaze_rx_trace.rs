//! Opt-in comparison of kernel and application receive times for direct Phaze.
use evoparseq_phaze::Datagram;
use serde_json::json;
use std::{
    io,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6},
    os::fd::AsRawFd,
};
use tokio::{io::Interest, net::UdpSocket};

// Linux UAPI SO_TIMESTAMPNS_NEW uses two i64 fields on both Android ABIs.
const SO_TIMESTAMPNS_NEW: libc::c_int = 64;
const SO_TIMESTAMPNS_OLD: libc::c_int = 35;

pub fn enable(socket: UdpSocket) -> Option<TracedSocket> {
    let mode = if set_timestamp_option(&socket, SO_TIMESTAMPNS_NEW) {
        Some("time64")
    } else if set_timestamp_option(&socket, SO_TIMESTAMPNS_OLD) {
        Some("legacy")
    } else {
        None
    };
    crate::event_log::record(
        "INFO",
        "phaze_android_rx_timestamping",
        json!({"kernel_timestamps_enabled": mode.is_some(), "mode": mode}),
    );
    mode.map(|_| TracedSocket { socket })
}

fn set_timestamp_option(socket: &UdpSocket, option: libc::c_int) -> bool {
    let enabled: libc::c_int = 1;
    // The descriptor stays live and the option value is one initialized int.
    let result = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            option,
            std::ptr::from_ref(&enabled).cast(),
            size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    result == 0
}

pub struct TracedSocket {
    socket: UdpSocket,
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
        self.socket.recv_from(data).await
    }

    async fn recv_from_timed(
        &self,
        data: &mut [u8],
    ) -> io::Result<(usize, SocketAddr, Option<i64>)> {
        self.socket
            .async_io(Interest::READABLE, || {
                receive(self.socket.as_raw_fd(), data)
            })
            .await
    }
}

fn receive(fd: libc::c_int, data: &mut [u8]) -> io::Result<(usize, SocketAddr, Option<i64>)> {
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
    Ok((len as usize, address, timestamp(&message)))
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
            if (*header).cmsg_level == libc::SOL_SOCKET {
                let data = libc::CMSG_DATA(header);
                let length = (*header).cmsg_len;
                let (seconds, nanos) = match (*header).cmsg_type {
                    SO_TIMESTAMPNS_NEW if length >= libc::CMSG_LEN(16) as usize => {
                        let [seconds, nanos] = data.cast::<[i64; 2]>().read_unaligned();
                        (i128::from(seconds), i128::from(nanos))
                    }
                    SO_TIMESTAMPNS_OLD
                        if length
                            >= libc::CMSG_LEN(size_of::<libc::timespec>() as u32) as usize =>
                    {
                        let time = data.cast::<libc::timespec>().read_unaligned();
                        (i128::from(time.tv_sec), i128::from(time.tv_nsec))
                    }
                    _ => {
                        header = libc::CMSG_NXTHDR(message, header);
                        continue;
                    }
                };
                if seconds >= 0 && (0..1_000_000_000).contains(&nanos) {
                    return i64::try_from(seconds * 1_000_000_000 + nanos).ok();
                }
                return None;
            }
            header = libc::CMSG_NXTHDR(message, header);
        }
    }
    None
}
