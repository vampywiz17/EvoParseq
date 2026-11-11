//! Linux outgoing control/data policy. AF21 follows RFC 8837's high data class.
//! SO_PRIORITY 6 is within Linux's unprivileged 0..=6 range. Neither proves
//! network forwarding treatment. Options are independent and failure is optional.
use std::{io, os::fd::AsRawFd};

pub const DSCP: i32 = 18;
pub const PRIORITY: i32 = 6;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub ipv4_dscp: Option<u32>,
    pub ipv6_dscp: Option<u32>,
    pub priority: Option<u32>,
    pub ipv4_error: Option<u32>,
    pub ipv6_error: Option<u32>,
    pub priority_error: Option<u32>,
}

fn option(fd: i32, level: i32, name: i32, value: Option<i32>) -> io::Result<i32> {
    let mut actual = value.unwrap_or_default();
    let mut length = std::mem::size_of::<i32>() as libc::socklen_t;
    // SAFETY: the borrowed descriptor is not owned here; both calls receive an
    // initialized int and its exact size. No pointer survives either call.
    let result = unsafe {
        if value.is_some() {
            libc::setsockopt(fd, level, name, std::ptr::from_ref(&actual).cast(), length)
        } else {
            libc::getsockopt(
                fd,
                level,
                name,
                std::ptr::from_mut(&mut actual).cast(),
                &mut length,
            )
        }
    };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(actual)
    }
}

fn mark(fd: i32, level: i32, name: i32) -> io::Result<u32> {
    let current = option(fd, level, name, None)?;
    // Preserve ECN; only the six DSCP bits belong to this policy.
    option(fd, level, name, Some((DSCP << 2) | (current & 3)))?;
    let actual = option(fd, level, name, None)? >> 2;
    if actual != DSCP {
        return Err(io::Error::other("DSCP readback mismatch"));
    }
    Ok(actual as u32)
}

fn record(result: io::Result<u32>, value: &mut Option<u32>, error: &mut Option<u32>) {
    match result {
        Ok(actual) => *value = Some(actual),
        Err(e) => *error = Some(e.raw_os_error().unwrap_or(libc::EIO) as u32),
    }
}

pub(super) fn create(socket: &impl AsRawFd, requested: bool) -> super::Status {
    let mut status = super::Status {
        requested,
        ..Default::default()
    };
    if !requested {
        return status;
    }
    let fd = socket.as_raw_fd();
    let domain = match option(fd, libc::SOL_SOCKET, libc::SO_DOMAIN, None) {
        Ok(domain) => domain,
        Err(error) => {
            status.error = Some(error.raw_os_error().unwrap_or(libc::EIO) as u32);
            return status;
        }
    };
    let v6 = domain == libc::AF_INET6;
    let v4 = matches!(domain, libc::AF_INET | libc::AF_INET6);
    let s = &mut status.linux;
    if v4 {
        record(
            mark(fd, libc::IPPROTO_IP, libc::IP_TOS),
            &mut s.ipv4_dscp,
            &mut s.ipv4_error,
        );
    }
    if v6 {
        record(
            mark(fd, libc::IPPROTO_IPV6, libc::IPV6_TCLASS),
            &mut s.ipv6_dscp,
            &mut s.ipv6_error,
        );
    }
    // IP_TOS can itself change sk_priority: apply SO_PRIORITY last, then read it.
    let priority = option(fd, libc::SOL_SOCKET, libc::SO_PRIORITY, Some(PRIORITY))
        .and_then(|_| option(fd, libc::SOL_SOCKET, libc::SO_PRIORITY, None))
        .and_then(|actual| {
            if actual == PRIORITY {
                Ok(actual as u32)
            } else {
                Err(io::Error::other("priority readback mismatch"))
            }
        });
    record(priority, &mut s.priority, &mut s.priority_error);
    status.configured_dscp = s.ipv4_dscp.or(s.ipv6_dscp);
    status.error = s.ipv4_error.or(s.ipv6_error).or(s.priority_error);
    status.applied =
        v4 && s.ipv4_dscp.is_some() && (!v6 || s.ipv6_dscp.is_some()) && s.priority.is_some();
    if !v4 {
        status.error = Some(libc::EAFNOSUPPORT as u32);
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::UdpSocket;

    #[test]
    fn linux_qos_is_optional_preserves_ecn_and_socket_io() {
        for address in ["127.0.0.1:0", "[::1]:0"] {
            let socket = UdpSocket::bind(address).unwrap();
            let fd = socket.as_raw_fd();
            let v6 = socket.local_addr().unwrap().is_ipv6();
            let (level, name) = if v6 {
                (libc::IPPROTO_IPV6, libc::IPV6_TCLASS)
            } else {
                (libc::IPPROTO_IP, libc::IP_TOS)
            };
            option(fd, level, name, Some(3)).unwrap();
            let before = option(fd, libc::SOL_SOCKET, libc::SO_PRIORITY, None).unwrap();
            assert_eq!(create(&socket, false), super::super::Status::default());
            assert_eq!(option(fd, level, name, None).unwrap(), 3);
            assert_eq!(
                option(fd, libc::SOL_SOCKET, libc::SO_PRIORITY, None).unwrap(),
                before
            );
            let status = create(&socket, true);
            assert!(status.applied, "{status:?}");
            assert_eq!(option(fd, level, name, None).unwrap(), (DSCP << 2) | 3);
            assert_eq!(
                option(fd, libc::SOL_SOCKET, libc::SO_PRIORITY, None).unwrap(),
                PRIORITY
            );
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            socket
                .send_to(b"input", socket.local_addr().unwrap())
                .unwrap();
            let mut bytes = [0; 5];
            socket.recv_from(&mut bytes).unwrap();
            assert_eq!(&bytes, b"input");
        }
    }

    #[test]
    fn linux_refused_options_are_independent_and_nonfatal() {
        // AF_UNIX cannot accept IP marking, but remains a valid live connection.
        use std::{
            io::{Read, Write},
            os::unix::net::UnixStream,
        };
        let (mut a, mut b) = UnixStream::pair().unwrap();
        assert!(mark(a.as_raw_fd(), libc::IPPROTO_IP, libc::IP_TOS).is_err());
        let status = create(&a, true);
        assert!(!status.applied);
        assert_eq!(status.linux.priority, Some(PRIORITY as u32));
        assert_eq!(status.error, Some(libc::EAFNOSUPPORT as u32));
        a.write_all(b"q").unwrap();
        let mut byte = [0];
        b.read_exact(&mut byte).unwrap();
        assert_eq!(byte, [b'q']);
        let mut value = None;
        let mut error = None;
        record(
            Err(io::Error::from_raw_os_error(libc::EPERM)),
            &mut value,
            &mut error,
        );
        assert_eq!(value, None);
        assert_eq!(error, Some(libc::EPERM as u32));
    }
}
