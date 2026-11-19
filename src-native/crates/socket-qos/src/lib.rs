//! Optional platform socket marking, once per socket. No shaping or probing.
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub requested: bool,
    pub applied: bool,
    pub configured_dscp: Option<u32>,
    pub configured_l2: Option<u32>,
    pub error: Option<u32>,
    #[cfg(target_os = "linux")]
    pub linux: linux::Status,
}
#[derive(Debug, Default)]
pub struct Flow {
    pub status: Status,
    #[cfg(windows)]
    handle: usize,
}
// The optional observer receives one setup result, never packet/frame events.
static OBSERVER: OnceLock<fn(&str, Status)> = OnceLock::new();
pub fn observe(observer: fn(&str, Status)) {
    let _ = OBSERVER.set(observer);
}
impl Flow {
    #[cfg(windows)]
    pub fn new(
        socket: &impl std::os::windows::io::AsRawSocket,
        requested: bool,
        transport: &str,
    ) -> Self {
        let flow = windows::create(
            socket,
            Status {
                requested,
                ..Status::default()
            },
        );
        flow.report(transport);
        flow
    }
    #[cfg(target_os = "linux")]
    pub fn new(socket: &impl std::os::fd::AsRawFd, requested: bool, transport: &str) -> Self {
        let flow = Self {
            status: linux::create(socket, requested),
        };
        flow.report(transport);
        flow
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    pub fn new<T>(_socket: &T, requested: bool, transport: &str) -> Self {
        // Other platforms keep their existing no-op policy.
        let flow = Self {
            status: Status {
                requested,
                ..Status::default()
            },
        };
        flow.report(transport);
        flow
    }
    fn report(&self, transport: &str) {
        if self.status.requested
            && let Some(observer) = OBSERVER.get()
        {
            observer(transport, self.status);
        }
    }
}
#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(windows)]
mod windows {
    use super::*;
    use std::{ffi::c_void, os::windows::io::AsRawSocket, sync::OnceLock};
    use windows_sys::Win32::{Foundation::GetLastError, System::LibraryLoader::*};
    type Handle = *mut c_void;
    #[repr(C)]
    struct Version {
        major: u16,
        minor: u16,
    }
    #[repr(C)]
    #[derive(Default)]
    struct Priority {
        dscp: u32,
        _nonconformant_dscp: u32,
        l2: u32,
        _nonconformant_l2: u32,
    }
    type Create = unsafe extern "system" fn(*const Version, *mut Handle) -> i32;
    type Add = unsafe extern "system" fn(Handle, usize, *const c_void, i32, u32, *mut u32) -> i32;
    type Query = unsafe extern "system" fn(
        Handle,
        u32,
        i32,
        *mut u32,
        *mut Priority,
        u32,
        *mut c_void,
    ) -> i32;
    type Close = unsafe extern "system" fn(Handle) -> i32;
    struct Api {
        create: Create,
        add: Add,
        query: Query,
        close: Close,
    }
    fn api() -> Option<&'static Api> {
        static API: OnceLock<Option<Api>> = OnceLock::new();
        API.get_or_init(|| unsafe {
            let name: Vec<_> = "qwave.dll\0".encode_utf16().collect();
            // Only the system library; successful resolution lives for the process.
            let library = LoadLibraryExW(
                name.as_ptr(),
                std::ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_SYSTEM32,
            );
            if library.is_null() {
                return None;
            }
            let found = (|| {
                Some(Api {
                    create: std::mem::transmute::<unsafe extern "system" fn() -> isize, Create>(
                        GetProcAddress(library, c"QOSCreateHandle".as_ptr().cast())?,
                    ),
                    add: std::mem::transmute::<unsafe extern "system" fn() -> isize, Add>(
                        GetProcAddress(library, c"QOSAddSocketToFlow".as_ptr().cast())?,
                    ),
                    query: std::mem::transmute::<unsafe extern "system" fn() -> isize, Query>(
                        GetProcAddress(library, c"QOSQueryFlow".as_ptr().cast())?,
                    ),
                    close: std::mem::transmute::<unsafe extern "system" fn() -> isize, Close>(
                        GetProcAddress(library, c"QOSCloseHandle".as_ptr().cast())?,
                    ),
                })
            })();
            if found.is_none() {
                windows_sys::Win32::Foundation::FreeLibrary(library);
            }
            found
        })
        .as_ref()
    }
    pub(super) fn create(socket: &impl AsRawSocket, status: Status) -> Flow {
        let mut flow = Flow { status, handle: 0 };
        if !flow.status.requested {
            return flow;
        }
        let Some(api) = api() else {
            flow.status.error = Some(126);
            return flow;
        };
        unsafe {
            let mut handle = std::ptr::null_mut();
            if (api.create)(&Version { major: 1, minor: 0 }, &mut handle) == 0 {
                flow.status.error = Some(GetLastError());
                return flow;
            }
            flow.handle = handle as usize;
            let mut id = 0;
            // Connected TCP: never reconnect/disconnect it to apply a mark.
            // AudioVideo=3; NON_ADAPTIVE=2: no path probes or bandwidth shaping.
            if (api.add)(
                handle,
                socket.as_raw_socket() as usize,
                std::ptr::null(),
                3,
                2,
                &mut id,
            ) == 0
            {
                flow.status.error = Some(GetLastError());
                return flow;
            }
            flow.status.applied = true;
            let mut priority = Priority::default();
            let mut size = std::mem::size_of::<Priority>() as u32;
            if (api.query)(
                handle,
                id,
                1,
                &mut size,
                &mut priority,
                0,
                std::ptr::null_mut(),
            ) != 0
            {
                flow.status.configured_dscp = Some(priority.dscp);
                flow.status.configured_l2 = Some(priority.l2);
            } else {
                flow.status.error = Some(GetLastError());
            }
        }
        flow
    }
    impl Drop for Flow {
        fn drop(&mut self) {
            if self.handle != 0
                && let Some(api) = api()
            {
                // Closing the handle removes its flow; the socket stays owned by its stream.
                unsafe {
                    (api.close)(self.handle as Handle);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_and_optional_marking_preserve_connected_io() {
        use std::{
            io::{Read, Write},
            net::{TcpListener, TcpStream},
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        for requested in [false, true] {
            let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let (mut server, _) = listener.accept().unwrap();
            client
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            server
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            let flow = Flow::new(&client, requested, "fixture");
            assert_eq!(flow.status.requested, requested);
            if !requested {
                assert_eq!(flow.status, Status::default());
            }
            client.write_all(b"q").unwrap();
            let mut byte = [0];
            server.read_exact(&mut byte).unwrap();
            assert_eq!(&byte, b"q");
            drop(flow); // closes only its own QOS handle, never the TCP socket
            server.write_all(b"s").unwrap();
            client.read_exact(&mut byte).unwrap();
            assert_eq!(&byte, b"s");
        }
    }
}
