//! Exercise the production native adapter against a real, disposable compositor.
//! Only playback/media is replaced: the tested Wayland protocol code is included
//! directly from the Linux client. This executable is never shipped in Flatpak.
#![allow(dead_code)]
extern crate self as eframe;
use anyhow::{Context, Result};
pub use egui;
use std::{
    io::{BufRead, BufReader, Write},
    os::fd::{AsFd, FromRawFd},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    protocol::{
        wl_buffer, wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer,
        wl_data_source, wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm,
        wl_shm_pool, wl_surface,
    },
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

// Minimal media/session doubles. Pointer vocabulary is the real shared Rust type.
mod control {
    pub fn header(op: u8, a: i32, b: i32, c: i32) -> bytes::Bytes {
        let mut out = Vec::from(a.to_be_bytes());
        out.extend(b.to_be_bytes());
        out.extend(c.to_be_bytes());
        out.push(op);
        out.into()
    }
}
#[path = "../../../src-native/src/desktop_input.rs"]
mod desktop_input;
#[path = "../../../src-native/src/viewport.rs"]
mod viewport;
mod input_handoff {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum DragPhase {
        Begin,
        Motion,
        Drop,
        Cancel,
    }
    #[derive(Clone, Copy, Debug)]
    pub enum Event {
        DisplayDrag {
            stream: usize,
            pointer: crate::desktop_input::Pointer,
            phase: DragPhase,
        },
    }
}
mod event_log {
    pub static LOCAL_POINTER: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    pub fn record(_: &str, event: &str, details: serde_json::Value) {
        if event == "wayland_display_drag_local_pointer" {
            LOCAL_POINTER.store(
                details["visible"].as_bool().unwrap(),
                std::sync::atomic::Ordering::Release,
            );
        }
        println!("{event} {details}");
    }
}
mod display_stream {
    pub const STREAMS: usize = 3;
}
mod display_routes {
    pub fn viewport(s: usize) -> egui::ViewportId {
        egui::ViewportId::from_hash_of(("display-stream", s))
    }
}
mod playback {
    use super::*;
    pub struct Playback {
        pub native_drag: AtomicBool,
        stream: usize,
        peers: Mutex<Vec<std::sync::Weak<Playback>>>,
        pub events: Mutex<Vec<input_handoff::Event>>,
        active: AtomicBool,
    }
    impl Playback {
        pub fn pair() -> [Arc<Self>; 2] {
            let pair = std::array::from_fn(|stream| {
                Arc::new(Self {
                    native_drag: AtomicBool::new(false),
                    stream,
                    peers: Mutex::new(Vec::new()),
                    events: Mutex::new(Vec::new()),
                    active: AtomicBool::new(true),
                })
            });
            for p in &pair {
                *p.peers.lock().unwrap() = pair.iter().map(Arc::downgrade).collect();
            }
            pair
        }
        pub fn stream(&self) -> usize {
            self.stream
        }
        pub fn display(&self, s: usize) -> Option<Arc<Self>> {
            self.peers.lock().unwrap().get(s)?.upgrade()
        }
        pub fn primary(&self) -> Option<Arc<Self>> {
            self.display(0)
        }
        pub fn owns_input(&self) -> bool {
            true
        }
        pub fn input_active(&self) -> bool {
            self.active.load(Ordering::Acquire)
        }
        pub fn stopped(&self) -> bool {
            false
        }
        pub fn desktop_pointer(
            &self,
            pos: egui::Pos2,
            buttons: u32,
        ) -> Option<desktop_input::Pointer> {
            Some(desktop_input::Pointer {
                x: f64::from(pos.x),
                y: f64::from(pos.y),
                relative: false,
                moved: true,
                viewport: (800, 600),
                buttons,
                wheel: (0, 0),
            })
        }
        pub fn push_routed(&self, event: input_handoff::Event) {
            self.events.lock().unwrap().push(event);
        }
    }
}
#[path = "../../../linux/src/wayland_drag.rs"]
mod wayland_drag;

#[derive(Default)]
struct Fixture {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm: Option<xdg_wm_base::XdgWmBase>,
    seat: Option<wl_seat::WlSeat>,
    manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    outputs: Vec<(wl_output::WlOutput, i32, i32)>,
    surfaces: Vec<wl_surface::WlSurface>,
    buffers: Vec<wl_buffer::WlBuffer>,
    current: Option<usize>,
    point: (f64, f64),
    pressed: bool,
    serial: u32,
    clipboard: Option<wl_data_device::WlDataDevice>,
    selections: usize,
    configured: usize,
}
impl Dispatch<wl_registry::WlRegistry, ()> for Fixture {
    fn event(
        s: &mut Self,
        r: &wl_registry::WlRegistry,
        e: wl_registry::Event,
        _: &(),
        _: &Connection,
        q: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = e
        {
            match interface.as_str() {
                "wl_compositor" => s.compositor = Some(r.bind(name, version.min(4), q, ())),
                "wl_shm" => s.shm = Some(r.bind(name, 1, q, ())),
                "xdg_wm_base" => s.wm = Some(r.bind(name, 1, q, ())),
                "wl_seat" => s.seat = Some(r.bind(name, version.min(7), q, ())),
                "wl_data_device_manager" => s.manager = Some(r.bind(name, 3, q, ())),
                "wl_output" => {
                    let output = r.bind(name, version.min(3), q, ());
                    s.outputs.push((output, 0, 0));
                }
                _ => {}
            }
        }
    }
}
impl Dispatch<wl_output::WlOutput, ()> for Fixture {
    fn event(
        s: &mut Self,
        o: &wl_output::WlOutput,
        e: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Geometry { x, y, .. } = e {
            let output = s.outputs.iter_mut().find(|v| v.0 == *o).unwrap();
            output.1 = x;
            output.2 = y;
        }
    }
}
impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Fixture {
    fn event(
        _: &mut Self,
        o: &xdg_wm_base::XdgWmBase,
        e: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = e {
            o.pong(serial);
        }
    }
}
impl Dispatch<xdg_surface::XdgSurface, usize> for Fixture {
    fn event(
        s: &mut Self,
        o: &xdg_surface::XdgSurface,
        e: xdg_surface::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let xdg_surface::Event::Configure { serial } = e else {
            return;
        };
        o.ack_configure(serial);
        s.surfaces[*index].attach(Some(&s.buffers[*index]), 0, 0);
        s.surfaces[*index].damage(0, 0, 800, 600);
        s.surfaces[*index].commit();
        s.configured += 1;
    }
}
impl Dispatch<wl_pointer::WlPointer, ()> for Fixture {
    fn event(
        s: &mut Self,
        _: &wl_pointer::WlPointer,
        e: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match e {
            wl_pointer::Event::Enter {
                surface,
                surface_x,
                surface_y,
                ..
            } => {
                s.current = s.surfaces.iter().position(|p| p.id() == surface.id());
                s.point = (surface_x, surface_y);
            }
            wl_pointer::Event::Leave { .. } => s.current = None,
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => s.point = (surface_x, surface_y),
            wl_pointer::Event::Button {
                serial,
                state: wayland_client::WEnum::Value(state),
                ..
            } => {
                s.serial = serial;
                s.pressed = state == wl_pointer::ButtonState::Pressed;
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_data_device::WlDataDevice, ()> for Fixture {
    fn event(
        s: &mut Self,
        device: &wl_data_device::WlDataDevice,
        e: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Simulate a toolkit which already has its own data-device resource.
        if let wl_data_device::Event::Selection { id: Some(o) } = e {
            if s.clipboard.as_ref() == Some(device) {
                s.selections += 1;
            }
            o.destroy();
        }
    }
    wayland_client::event_created_child!(Fixture,wl_data_device::WlDataDevice,[0=>(wl_data_offer::WlDataOffer,())]);
}
wayland_client::delegate_noop!(Fixture:ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(Fixture:ignore wl_shm::WlShm);
wayland_client::delegate_noop!(Fixture:ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(Fixture:ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(Fixture:ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(Fixture:ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(Fixture:ignore wl_keyboard::WlKeyboard);
wayland_client::delegate_noop!(Fixture:ignore wl_data_device_manager::WlDataDeviceManager);
wayland_client::delegate_noop!(Fixture:ignore wl_data_offer::WlDataOffer);
wayland_client::delegate_noop!(Fixture:ignore wl_data_source::WlDataSource);
wayland_client::delegate_noop!(Fixture:ignore xdg_toplevel::XdgToplevel);

struct SurfaceHandle(wl_surface::WlSurface);
impl winit::raw_window_handle::HasWindowHandle for SurfaceHandle {
    fn window_handle(
        &self,
    ) -> std::result::Result<
        winit::raw_window_handle::WindowHandle<'_>,
        winit::raw_window_handle::HandleError,
    > {
        use winit::raw_window_handle::*;
        let ptr = std::ptr::NonNull::new(self.0.id().as_ptr().cast()).unwrap();
        Ok(unsafe {
            WindowHandle::borrow_raw(RawWindowHandle::Wayland(WaylandWindowHandle::new(ptr)))
        })
    }
}
struct Driver {
    child: std::process::Child,
    out: BufReader<std::process::ChildStdout>,
}
impl Driver {
    fn new() -> Result<Self> {
        let mut child = Command::new("/usr/bin/python3")
            .arg("ci-tests/wayland-input/drive.py")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let mut out = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        out.read_line(&mut line)?;
        anyhow::ensure!(line.trim() == "ready", "driver unavailable: {line}");
        Ok(Self { child, out })
    }
    fn command(&mut self, value: serde_json::Value) -> Result<()> {
        writeln!(self.child.stdin.as_mut().unwrap(), "{value}")?;
        self.child.stdin.as_mut().unwrap().flush()?;
        let mut line = String::new();
        self.out.read_line(&mut line)?;
        anyhow::ensure!(line.trim() == "ok", "driver failed: {line}");
        Ok(())
    }
}
impl Drop for Driver {
    fn drop(&mut self) {
        let _ = writeln!(self.child.stdin.as_mut().unwrap(), "[\"quit\"]");
        let _ = self.child.wait();
    }
}

fn main() -> Result<()> {
    let connection = Connection::connect_to_env()?;
    let mut queue = connection.new_event_queue();
    let q = queue.handle();
    let _registry = connection.display().get_registry(&q, ());
    let mut f = Fixture::default();
    queue.roundtrip(&mut f)?;
    queue.roundtrip(&mut f)?;
    anyhow::ensure!(
        f.outputs.len() == 3,
        "two streams and one neutral output required"
    );
    f.outputs.sort_by_key(|o| (o.1, o.2));
    // The headless seat gains a real virtual keyboard only on first input.
    // Initialize it before binding wl_keyboard or mapping focused windows.
    let mut driver = Driver::new()?;
    driver.command(serde_json::json!(["escape"]))?;
    queue.roundtrip(&mut f)?;
    let _toolkit_device =
        f.manager
            .as_ref()
            .unwrap()
            .get_data_device(f.seat.as_ref().unwrap(), &q, ());
    let _pointer = f.seat.as_ref().unwrap().get_pointer(&q, ());
    let _keyboard = f.seat.as_ref().unwrap().get_keyboard(&q, ());
    f.clipboard = Some(_toolkit_device.clone());
    let mut toplevels = Vec::new();
    for index in 0..3 {
        let surface = f.compositor.as_ref().unwrap().create_surface(&q, ());
        let xdg = f.wm.as_ref().unwrap().get_xdg_surface(&surface, &q, index);
        let top = xdg.get_toplevel(&q, ());
        top.set_title(format!("EvoParseq isolated drag fixture {index}"));
        top.set_app_id(format!("io.github.evoparseq.fixture{index}"));
        top.set_fullscreen(Some(&f.outputs[index].0));
        let fd =
            unsafe { libc::memfd_create(c"evoparseq-drag-fixture".as_ptr(), libc::MFD_CLOEXEC) };
        anyhow::ensure!(fd >= 0, "memfd unavailable");
        let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
        let pixels = vec![if index == 0 { 0x80 } else { 0xc0 }; 800 * 600 * 4];
        file.write_all(&pixels)?;
        let pool = f
            .shm
            .as_ref()
            .unwrap()
            .create_pool(file.as_fd(), pixels.len() as i32, &q, ());
        let buffer = pool.create_buffer(0, 800, 600, 800 * 4, wl_shm::Format::Argb8888, &q, ());
        pool.destroy();
        f.surfaces.push(surface);
        f.buffers.push(buffer);
        f.surfaces[index].commit();
        toplevels.push((xdg, top));
    }
    for _ in 0..10 {
        queue.roundtrip(&mut f)?;
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    anyhow::ensure!(f.configured >= 2, "surfaces were not mapped");
    let mut drag = wayland_drag::Drag::connect(connection.clone(), egui::Context::default())?;
    for i in 0..2 {
        drag.surface(i, &SurfaceHandle(f.surfaces[i].clone()))?;
    }
    let playback = playback::Playback::pair();
    let mut selections = Vec::new();
    // All movement is generated inside the disposable headless compositor.
    let mut pump = |drag: &mut wayland_drag::Drag, f: &mut Fixture| -> Result<()> {
        for _ in 0..4 {
            std::thread::sleep(std::time::Duration::from_millis(15));
            queue.roundtrip(f)?;
            drag.dispatch();
        }
        Ok(())
    };
    for (source, target) in [(0, 1), (1, 0)] {
        // egui-winit creates a clipboard data device for a newly opened
        // viewport on the same display. Exercise that late binding too.
        let _late_toolkit_device =
            f.manager
                .as_ref()
                .unwrap()
                .get_data_device(f.seat.as_ref().unwrap(), &q, ());
        driver.command(serde_json::json!(["move", -10000., -10000.]))?;
        pump(&mut drag, &mut f)?;
        // Edges may be outside mapped surface input regions. First move inside
        // an output, then use its real enter coordinates to reach the source.
        driver.command(serde_json::json!(["move", 300., 250.]))?;
        pump(&mut drag, &mut f)?;
        let (x, y) = (f.outputs[source].1 + 300, f.outputs[source].2 + 250);
        let current = f.current.context("No pointer surface after edge reset")?;
        // Mutter clamps large relative moves to an output edge, not necessarily
        // desktop (0,0). Position from the actual pointer enter coordinates.
        driver.command(serde_json::json!([
            "move",
            x as f64 - (f.outputs[current].1 as f64 + f.point.0),
            y as f64 - (f.outputs[current].2 as f64 + f.point.1)
        ]))?;
        pump(&mut drag, &mut f)?;
        anyhow::ensure!(
            f.current == Some(source),
            "pointer did not reach source {source}: {:?} {:?}",
            f.current,
            f.point
        );
        driver.command(serde_json::json!(["button", true]))?;
        pump(&mut drag, &mut f)?;
        drag.begin(
            &playback[source],
            egui::pos2(f.point.0 as f32, f.point.1 as f32),
        )?;
        pump(&mut drag, &mut f)?;
        // The real pointer can leave while the host-rendered cursor stays at
        // its video's edge. Show native DnD feedback on a non-stream surface.
        driver.command(serde_json::json!([
            "move",
            (f.outputs[2].1 - f.outputs[source].1) as f64,
            (f.outputs[2].2 - f.outputs[source].2) as f64
        ]))?;
        pump(&mut drag, &mut f)?;
        anyhow::ensure!(
            event_log::LOCAL_POINTER.load(Ordering::Acquire),
            "no visible local pointer between stream windows"
        );
        driver.command(serde_json::json!([
            "move",
            (f.outputs[target].1 - f.outputs[2].1) as f64,
            (f.outputs[target].2 - f.outputs[2].2) as f64
        ]))?;
        pump(&mut drag, &mut f)?;
        anyhow::ensure!(
            !event_log::LOCAL_POINTER.load(Ordering::Acquire),
            "local drag pointer duplicated the host cursor inside video"
        );
        driver.command(serde_json::json!(["button", false]))?;
        pump(&mut drag, &mut f)?;
        let events = playback[0].events.lock().unwrap();
        anyhow::ensure!(events.iter().any(|e|matches!(e,input_handoff::Event::DisplayDrag{stream,phase:input_handoff::DragPhase::Motion,..} if *stream==target)),"missing peer motion: {events:?}");
        anyhow::ensure!(events.iter().filter(|e|matches!(e,input_handoff::Event::DisplayDrag{stream,phase:input_handoff::DragPhase::Drop,..} if *stream==target)).count()==1,"drop was not delivered once: {events:?}");
        drop(events);
        anyhow::ensure!(
            f.current == Some(target),
            "native pointer did not remain on target"
        );
        println!(
            "drop {source}->{target}: surface={:?}, point={:?}, outputs={:?}",
            f.current,
            f.point,
            f.outputs.iter().map(|o| (o.1, o.2)).collect::<Vec<_>>()
        );
        let previous = f.point;
        driver.command(serde_json::json!([
            "move",
            if f.point.0 > 400. { -12. } else { 12. },
            8.
        ]))?;
        pump(&mut drag, &mut f)?;
        anyhow::ensure!(
            f.current == Some(target),
            "post-drop pointer left target {target}: surface={:?}, point={:?}",
            f.current,
            f.point
        );
        anyhow::ensure!(f.point != previous, "no ordinary pointer motion after drop");
        driver.command(serde_json::json!([
            "move",
            f.outputs[source].1 as f64 + 300. - (f.outputs[target].1 as f64 + f.point.0),
            f.outputs[source].2 as f64 + 250. - (f.outputs[target].2 as f64 + f.point.1)
        ]))?;
        pump(&mut drag, &mut f)?;
        anyhow::ensure!(
            f.current == Some(source),
            "post-drop pointer could not freely leave destination"
        );
        // Rebinding DnD must leave the ordinary clipboard service subscribed.
        // This source is synthetic; no real clipboard/account data is read.
        let selected = f.selections;
        let selection = f.manager.as_ref().unwrap().create_data_source(&q, ());
        selection.offer("text/plain;charset=utf-8".to_owned());
        f.clipboard
            .as_ref()
            .unwrap()
            .set_selection(Some(&selection), f.serial);
        selections.push(selection);
        pump(&mut drag, &mut f)?;
        anyhow::ensure!(
            f.selections > selected,
            "clipboard lost selection updates after drag"
        );
        playback[0].events.lock().unwrap().clear();
    }
    // Cancellation must end once even if no destination drop is delivered.
    driver.command(serde_json::json!(["button", true]))?;
    pump(&mut drag, &mut f)?;
    let source = f.current.context("No source for cancel test")?;
    drag.begin(
        &playback[source],
        egui::pos2(f.point.0 as f32, f.point.1 as f32),
    )?;
    pump(&mut drag, &mut f)?;
    driver.command(serde_json::json!(["escape"]))?;
    pump(&mut drag, &mut f)?;
    driver.command(serde_json::json!(["button", false]))?;
    pump(&mut drag, &mut f)?;
    anyhow::ensure!(
        playback[0]
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(
                e,
                input_handoff::Event::DisplayDrag {
                    phase: input_handoff::DragPhase::Cancel,
                    ..
                }
            ))
            .count()
            == 1,
        "cancel must release once"
    );
    println!(
        "PASS: real native Wayland drag in both directions, local gap feedback, clipboard continuity, free post-drop pointer and cancellation"
    );
    Ok(())
}
