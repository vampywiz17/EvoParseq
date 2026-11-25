//! One overlapped stream and its application wake share a completion port.
//! TLS owns records above this layer; only encrypted network bytes are copied
//! into the bounded send storage required to outlive an overlapped operation.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::windows::io::{AsRawSocket, FromRawSocket};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HANDLE, WAIT_TIMEOUT};
use windows_sys::Win32::Networking::WinSock as ws;
use windows_sys::Win32::Storage::FileSystem::SetFileCompletionNotificationModes;
use windows_sys::Win32::System::IO::{
    CancelIoEx, CreateIoCompletionPort, GetQueuedCompletionStatusEx, OVERLAPPED, OVERLAPPED_ENTRY,
};
use windows_sys::Win32::System::WindowsProgramming::{
    FILE_SKIP_COMPLETION_PORT_ON_SUCCESS, FILE_SKIP_SET_EVENT_ON_HANDLE,
};

use super::socket::{last_error, len_of, to_storage};
use super::wake::Wake;

const KEY_TCP: usize = 3;
const CAPACITY: usize = 32 * 1024;

#[derive(Default)]
struct State {
    connect: OVERLAPPED,
    recv: OVERLAPPED,
    send: OVERLAPPED,
    connect_posted: bool,
    recv_posted: bool,
    send_posted: bool,
    recv_flags: u32,
    read_ready: bool,
    output: Box<[u8]>,
    sent: usize,
    queued: usize,
}

impl State {
    fn pending(&self) -> bool {
        self.connect_posted || self.recv_posted || self.send_posted
    }
}

pub(crate) struct Tcp {
    socket: TcpStream,
    wake: Wake,
    state: Box<State>,
    connected: bool,
    skip_sync: bool,
    eof: bool,
    read_error: Option<io::Error>,
    write_progress: bool,
}

impl std::fmt::Debug for Tcp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tcp")
            .field("socket", &self.socket)
            .finish_non_exhaustive()
    }
}

// SAFETY: the unique owner moves, but all kernel-referenced storage remains
// boxed at a stable address until every posted operation has completed.
unsafe impl Send for Tcp {}

impl Tcp {
    pub(crate) fn qos(&self, requested: bool) -> evoparseq_socket_qos::Flow {
        evoparseq_socket_qos::Flow::new(&self.socket, requested, "BUD")
    }

    pub(crate) fn start(server: SocketAddr) -> io::Result<Self> {
        Self::start_inner(server, true)
    }

    fn start_inner(server: SocketAddr, skip_success: bool) -> io::Result<Self> {
        super::socket::start()?;
        // SAFETY: documented overlapped socket creation, transferred once to RAII.
        let raw = unsafe {
            ws::WSASocketW(
                i32::from(ws::AF_INET6),
                ws::SOCK_STREAM,
                ws::IPPROTO_TCP,
                std::ptr::null(),
                0,
                ws::WSA_FLAG_OVERLAPPED | ws::WSA_FLAG_NO_HANDLE_INHERIT,
            )
        };
        if raw == ws::INVALID_SOCKET {
            return Err(last_error());
        }
        // SAFETY: fresh owned socket, closed by TcpStream on every error path.
        let socket = unsafe { TcpStream::from_raw_socket(raw as u64) };
        let zero = 0i32;
        // SAFETY: the option takes an integer of exactly this size.
        if unsafe {
            ws::setsockopt(
                raw,
                ws::IPPROTO_IPV6,
                ws::IPV6_V6ONLY,
                (&raw const zero).cast(),
                len_of::<i32>(),
            )
        } != 0
        {
            return Err(last_error());
        }
        let (local, len) = to_storage(SocketAddr::from((std::net::Ipv6Addr::UNSPECIFIED, 0)));
        // SAFETY: ConnectEx requires a previously bound socket.
        if unsafe { ws::bind(raw, (&raw const local).cast(), len) } != 0 {
            return Err(last_error());
        }
        socket.set_nonblocking(true)?;
        let wake = Wake::new()?;
        // SAFETY: this socket joins this attempt's only completion port.
        if unsafe { CreateIoCompletionPort(raw as HANDLE, wake.port(), KEY_TCP, 1) }.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: optional optimization; on unsupported providers all synchronous
        // successes are instead collected through the completion port.
        let skip_sync = skip_success
            && unsafe {
                SetFileCompletionNotificationModes(
                    raw as HANDLE,
                    (FILE_SKIP_COMPLETION_PORT_ON_SUCCESS | FILE_SKIP_SET_EVENT_ON_HANDLE) as u8,
                )
            } != 0;
        let mut connect: ws::LPFN_CONNECTEX = None;
        let connect_id = ws::WSAID_CONNECTEX;
        let mut returned = 0u32;
        // SAFETY: documented extension lookup, with exact GUID/function sizes.
        if unsafe {
            ws::WSAIoctl(
                raw,
                ws::SIO_GET_EXTENSION_FUNCTION_POINTER,
                (&raw const connect_id).cast(),
                size_of::<windows_sys::core::GUID>() as u32,
                (&raw mut connect).cast(),
                size_of::<ws::LPFN_CONNECTEX>() as u32,
                &raw mut returned,
                std::ptr::null_mut(),
                None,
            )
        } != 0
        {
            return Err(last_error());
        }
        let connect = connect.ok_or_else(|| io::Error::other("ConnectEx unavailable"))?;
        let mut tcp = Self {
            socket,
            wake,
            state: Box::new(State {
                output: vec![0; CAPACITY].into_boxed_slice(),
                ..State::default()
            }),
            connected: false,
            skip_sync,
            eof: false,
            read_error: None,
            write_progress: false,
        };
        let (remote, len) = to_storage(server);
        let mut bytes = 0;
        // SAFETY: stable OVERLAPPED, live socket, no initial send buffer. The
        // address is captured by ConnectEx before it returns.
        let ok = unsafe {
            connect(
                raw,
                (&raw const remote).cast(),
                len,
                std::ptr::null(),
                0,
                &raw mut bytes,
                &raw mut tcp.state.connect,
            )
        };
        if ok == 0 {
            let error = last_error();
            if error.raw_os_error() != Some(ws::WSA_IO_PENDING) {
                return Err(error);
            }
            tcp.state.connect_posted = true;
        } else if skip_sync {
            tcp.connected()?;
        } else {
            tcp.state.connect_posted = true;
        }
        Ok(tcp)
    }

    fn raw(&self) -> ws::SOCKET {
        self.socket.as_raw_socket() as ws::SOCKET
    }

    fn connected(&mut self) -> io::Result<()> {
        // SAFETY: required after successful ConnectEx before ordinary socket APIs.
        if unsafe {
            ws::setsockopt(
                self.raw(),
                ws::SOL_SOCKET,
                ws::SO_UPDATE_CONNECT_CONTEXT,
                std::ptr::null(),
                0,
            )
        } != 0
        {
            return Err(last_error());
        }
        self.connected = true;
        self.post_recv()
    }

    fn post_recv(&mut self) -> io::Result<()> {
        if self.state.recv_posted || self.state.read_ready || self.eof || self.read_error.is_some()
        {
            return Ok(());
        }
        self.state.recv = OVERLAPPED::default();
        self.state.recv_flags = 0;
        let buffer = ws::WSABUF {
            len: 0,
            buf: std::ptr::null_mut(),
        };
        let mut bytes = 0;
        // SAFETY: zero-byte readiness receive, with no payload storage lent
        // to the kernel. OVERLAPPED/flags remain boxed until completion.
        let rc = unsafe {
            ws::WSARecv(
                self.raw(),
                &buffer,
                1,
                &raw mut bytes,
                &raw mut self.state.recv_flags,
                &raw mut self.state.recv,
                None,
            )
        };
        if rc == 0 && self.skip_sync {
            self.received(bytes);
        } else if rc == 0 || last_error().raw_os_error() == Some(ws::WSA_IO_PENDING) {
            self.state.recv_posted = true;
        } else {
            self.read_error = Some(last_error());
        }
        Ok(())
    }

    fn received(&mut self, _count: u32) {
        // Zero means readiness here, not EOF. Only actual recv reports EOF.
        self.state.read_ready = true;
    }

    fn sent(&mut self, count: u32) -> io::Result<()> {
        if count == 0 || count as usize > self.state.queued - self.state.sent {
            return Err(io::ErrorKind::WriteZero.into());
        }
        self.state.sent += count as usize;
        self.write_progress = true;
        if self.state.sent == self.state.queued {
            self.state.sent = 0;
            self.state.queued = 0;
        }
        Ok(())
    }

    fn post_send(&mut self) -> io::Result<()> {
        // Bounded synchronous progress, with an immediate continuation in poll
        // for an unposted tail. A partial send never duplicates accepted bytes.
        for _ in 0..64 {
            if self.state.send_posted || self.state.queued == 0 {
                break;
            }
            self.state.send = OVERLAPPED::default();
            let buffer = ws::WSABUF {
                len: (self.state.queued - self.state.sent) as u32,
                buf: self
                    .state
                    .output
                    .get_mut(self.state.sent..)
                    .ok_or(io::ErrorKind::InvalidData)?
                    .as_mut_ptr(),
            };
            let mut bytes = 0;
            // SAFETY: as for post_recv; output cannot change while posted.
            let rc = unsafe {
                ws::WSASend(
                    self.raw(),
                    &buffer,
                    1,
                    &raw mut bytes,
                    0,
                    &raw mut self.state.send,
                    None,
                )
            };
            if rc == 0 && self.skip_sync {
                self.sent(bytes)?;
            } else if rc == 0 || last_error().raw_os_error() == Some(ws::WSA_IO_PENDING) {
                self.state.send_posted = true;
            } else {
                return Err(last_error());
            }
        }
        Ok(())
    }

    pub(crate) fn poll(&mut self, timeout: Duration, write: bool) -> io::Result<crate::tcp::Ready> {
        if self.connected {
            self.post_send()?;
            self.post_recv()?;
        }
        let immediate = self.state.read_ready
            || self.eof
            || self.read_error.is_some()
            || (self.connected && write && !self.pending_write())
            || (self.pending_write() && !self.state.send_posted);
        let timeout = if immediate {
            0
        } else {
            u32::try_from(timeout.as_millis() + u128::from(timeout.subsec_nanos() % 1_000_000 != 0))
                .unwrap_or(u32::MAX - 1)
        };
        self.completions(timeout, false)?;
        if self.connected {
            self.post_send()?;
            self.post_recv()?;
        }
        Ok(crate::tcp::Ready {
            readable: self.state.read_ready || self.eof || self.read_error.is_some(),
            writable: self.connected && !self.pending_write(),
            error: false,
        })
    }

    fn completions(&mut self, timeout: u32, closing: bool) -> io::Result<()> {
        let mut entries = [OVERLAPPED_ENTRY::default(); 8];
        let mut count = 0;
        // SAFETY: live port and correctly sized writable entries.
        if unsafe {
            GetQueuedCompletionStatusEx(
                self.wake.port(),
                entries.as_mut_ptr(),
                entries.len() as u32,
                &raw mut count,
                timeout,
                0,
            )
        } == 0
        {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(WAIT_TIMEOUT as i32) {
                Ok(())
            } else {
                Err(error)
            };
        }
        // Consume every dequeued completion even if one fails, so Drop cannot
        // wait forever for an operation whose entry was already removed.
        let mut failure = None;
        for entry in entries.iter().take(count as usize) {
            if entry.lpCompletionKey == super::KEY_WAKE {
                self.wake.take()?;
                continue;
            }
            if entry.lpCompletionKey != KEY_TCP {
                continue;
            }
            let connect = std::ptr::eq(entry.lpOverlapped, &raw const self.state.connect);
            let recv = std::ptr::eq(entry.lpOverlapped, &raw const self.state.recv);
            let send = std::ptr::eq(entry.lpOverlapped, &raw const self.state.send);
            if connect {
                self.state.connect_posted = false;
            }
            if recv {
                self.state.recv_posted = false;
            }
            if send {
                self.state.send_posted = false;
            }
            if closing {
                continue;
            }
            let result = if entry.Internal != 0 {
                let mut bytes = 0;
                let mut flags = 0;
                // SAFETY: dequeued operation remains allocated; translates the
                // completion's NTSTATUS to the documented Winsock error.
                unsafe {
                    ws::WSAGetOverlappedResult(
                        self.raw(),
                        entry.lpOverlapped,
                        &raw mut bytes,
                        0,
                        &raw mut flags,
                    )
                };
                Err(last_error())
            } else if connect {
                self.connected()
            } else if recv {
                self.received(entry.dwNumberOfBytesTransferred);
                Ok(())
            } else if send {
                self.sent(entry.dwNumberOfBytesTransferred)
            } else {
                Err(io::Error::other("unknown TCP completion"))
            };
            if let Err(error) = result {
                if recv {
                    self.read_error = Some(error);
                } else if failure.is_none() {
                    failure = Some(error);
                }
            }
        }
        failure.map_or(Ok(()), Err)
    }

    pub(crate) fn notify_handle(&self) -> io::Result<super::WakeHandle> {
        self.wake.handle()
    }
    pub(crate) fn take_error(&self) -> io::Result<Option<io::Error>> {
        self.socket.take_error()
    }
    pub(crate) fn peer_addr(&self) -> io::Result<SocketAddr> {
        if !self.connected {
            return Err(io::ErrorKind::NotConnected.into());
        }
        self.socket.peer_addr().map(crate::tcp::normalize)
    }
    pub(crate) fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr().map(crate::tcp::normalize)
    }
    pub(crate) fn set_nodelay(&self, enabled: bool) -> io::Result<()> {
        self.socket.set_nodelay(enabled)
    }
    #[cfg(test)]
    pub(crate) fn nodelay(&self) -> io::Result<bool> {
        self.socket.nodelay()
    }
    pub(crate) fn pending_write(&self) -> bool {
        self.state.queued != 0
    }
    pub(crate) fn take_write_progress(&mut self) -> bool {
        std::mem::take(&mut self.write_progress)
    }

    fn cancel(&mut self) {
        // SAFETY: cancel every request but keep its storage alive until its
        // completion is dequeued; shutdown also releases a pending receive.
        unsafe {
            CancelIoEx(self.raw() as HANDLE, std::ptr::null());
        }
        let _ = self.socket.shutdown(std::net::Shutdown::Both);
        let deadline = Instant::now() + Duration::from_secs(1);
        while self.state.pending() && Instant::now() < deadline {
            let _ = self.completions(10, true);
        }
    }
}

impl Read for Tcp {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        if self.eof {
            return Ok(0);
        }
        if !self.connected {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        // No intermediate receive copy: read into the caller or rustls buffer.
        match self.socket.read(bytes) {
            Ok(0) => {
                self.eof = true;
                self.state.read_ready = false;
                Ok(0)
            }
            Ok(count) => {
                if count < bytes.len() {
                    self.state.read_ready = false;
                    self.post_recv()?;
                }
                Ok(count)
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                self.state.read_ready = false;
                if let Some(error) = self.read_error.take() {
                    return Err(error);
                }
                self.post_recv()?;
                Err(error)
            }
            Err(error) => Err(error),
        }
    }
}

impl Write for Tcp {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        if !self.connected || self.pending_write() {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let count = bytes.len().min(self.state.output.len());
        self.state
            .output
            .get_mut(..count)
            .ok_or(io::ErrorKind::InvalidData)?
            .copy_from_slice(bytes.get(..count).ok_or(io::ErrorKind::InvalidData)?);
        self.state.queued = count;
        self.post_send()?;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.pending_write() {
            Err(io::ErrorKind::WouldBlock.into())
        } else {
            Ok(())
        }
    }
}

impl Drop for Tcp {
    fn drop(&mut self) {
        self.cancel();
        if self.state.pending() {
            // A broken provider must never turn a bounded cancellation into
            // use-after-free. Same fail-safe as the native UDP receive pool.
            let _ = Box::leak(std::mem::take(&mut self.state));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_tcp_completion_modes_deliver_bytes_and_drain_cancelled_receive() {
        for skip in [true, false] {
            for _ in 0..8 {
                let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
                let mut tcp = Tcp::start_inner(listener.local_addr().unwrap(), skip).unwrap();
                let deadline = Instant::now() + Duration::from_secs(3);
                while !tcp.connected {
                    assert!(Instant::now() < deadline);
                    tcp.poll(Duration::from_millis(20), true).unwrap();
                }
                let (mut peer, _) = listener.accept().unwrap();
                peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                assert_eq!(tcp.write(b"hello").unwrap(), 5);
                while tcp.pending_write() {
                    assert!(Instant::now() < deadline);
                    tcp.poll(Duration::from_millis(20), true).unwrap();
                }
                let mut bytes = [0; 5];
                peer.read_exact(&mut bytes).unwrap();
                assert_eq!(&bytes, b"hello");
                assert!(tcp.state.recv_posted);
                tcp.cancel();
                assert!(
                    !tcp.state.pending(),
                    "cancel left kernel-owned storage live"
                );
            }
        }
    }
}
