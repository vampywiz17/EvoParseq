//! Smithay Clipboard
//!
//! Provides access to the Wayland clipboard for gui applications. The user
//! should have surface around.

#![deny(clippy::all, clippy::if_not_else, clippy::enum_glob_use)]
use std::ffi::c_void;
use std::io::Result;
use std::sync::mpsc::{self, Receiver};

use sctk::reexports::calloop::channel::{self, Sender};
use sctk::reexports::client::Connection;
use sctk::reexports::client::backend::Backend;

mod mime;
mod state;
mod worker;

/// Access to a Wayland clipboard.
pub struct Clipboard {
    request_sender: Sender<worker::Command>,
    drag: DragContext,
    request_receiver: Receiver<Result<String>>,
    clipboard_thread: Option<std::thread::JoinHandle<()>>,
}

impl Clipboard {
    /// Creates new clipboard which will be running on its own thread with its
    /// own event queue to handle clipboard requests.
    ///
    /// # Safety
    ///
    /// `display` must be a valid `*mut wl_display` pointer, and it must remain
    /// valid for as long as `Clipboard` object is alive.
    pub unsafe fn new(display: *mut c_void) -> Self {
        let backend = unsafe { Backend::from_foreign_display(display.cast()) };
        let connection = Connection::from_backend(backend);

        // Create channel to send data to clipboard thread.
        let (request_sender, rx_chan) = channel::channel();
        // Create channel to get data from the clipboard thread.
        let (clipboard_reply_sender, request_receiver) = mpsc::channel();

        let drag = DragContext::default();
        let name = String::from("smithay-clipboard");
        let clipboard_thread = worker::spawn(
            name,
            connection,
            rx_chan,
            clipboard_reply_sender,
            drag.clone(),
        );

        Self {
            request_receiver,
            request_sender,
            clipboard_thread,
            drag,
        }
    }

    /// Share the existing data device with native drag handling. No additional
    /// worker or event queue is created. Callbacks run on the clipboard worker;
    /// they must not block or call synchronous clipboard methods.
    pub fn drag_context(&self) -> DragContext {
        self.drag.clone()
    }

    /// Load clipboard data.
    ///
    /// Loads content from a clipboard on a last observed seat.
    pub fn load(&self) -> Result<String> {
        let _ = self.request_sender.send(worker::Command::Load);

        if let Ok(reply) = self.request_receiver.recv() {
            reply
        } else {
            // The clipboard thread is dead, however we shouldn't crash downstream, so
            // propogating an error.
            Err(std::io::Error::other("clipboard is dead."))
        }
    }

    /// Store to a clipboard.
    ///
    /// Stores to a clipboard on a last observed seat.
    pub fn store<T: Into<String>>(&self, text: T) {
        let request = worker::Command::Store(text.into());
        let _ = self.request_sender.send(request);
    }

    /// Load primary clipboard data.
    ///
    /// Loads content from a  primary clipboard on a last observed seat.
    pub fn load_primary(&self) -> Result<String> {
        let _ = self.request_sender.send(worker::Command::LoadPrimary);

        if let Ok(reply) = self.request_receiver.recv() {
            reply
        } else {
            // The clipboard thread is dead, however we shouldn't crash downstream, so
            // propogating an error.
            Err(std::io::Error::other("clipboard is dead."))
        }
    }

    /// Store to a primary clipboard.
    ///
    /// Stores to a primary clipboard on a last observed seat.
    pub fn store_primary<T: Into<String>>(&self, text: T) {
        let request = worker::Command::StorePrimary(text.into());
        let _ = self.request_sender.send(request);
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        // Shutdown smithay-clipboard.
        let _ = self.request_sender.send(worker::Command::Exit);
        if let Some(clipboard_thread) = self.clipboard_thread.take() {
            let _ = clipboard_thread.join();
        }
    }
}

/// Native drag destination events, independent of clipboard contents.
pub enum DragEvent {
    Enter {
        surface: sctk::reexports::client::protocol::wl_surface::WlSurface,
        x: f64,
        y: f64,
    },
    Motion {
        x: f64,
        y: f64,
    },
    Leave,
    Drop,
}
type DragCallback = std::sync::Arc<dyn Fn(DragEvent) -> bool + Send + Sync>;
#[derive(Default)]
struct DragState {
    device: Option<sctk::reexports::client::protocol::wl_data_device::WlDataDevice>,
    handler: Option<(String, DragCallback)>,
}
/// A handle to the clipboard service's data device and optional drag handler.
#[derive(Clone, Default)]
pub struct DragContext(std::sync::Arc<std::sync::Mutex<DragState>>);
impl DragContext {
    pub fn device(
        &self,
    ) -> Option<sctk::reexports::client::protocol::wl_data_device::WlDataDevice> {
        self.0.lock().unwrap().device.clone()
    }
    pub fn set_handler(&self, mime: String, handler: DragCallback) {
        self.0.lock().unwrap().handler = Some((mime, handler));
    }
}
