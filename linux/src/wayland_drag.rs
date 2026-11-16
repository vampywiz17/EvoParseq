//! Native Wayland DnD for a held desktop mouse button across our own surfaces.
//! Uses the existing display/UI loop and shared bounded input inbox. No file
//! transfers, clipboard contents, pointer warps, worker or recurring timer.
use crate::{
    input_handoff::{DragPhase, Event},
    playback::Playback,
};
use anyhow::{Context, Result};
use eframe::egui;
use std::sync::{Arc, Mutex, atomic::Ordering};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle,
    protocol::{
        wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer, wl_data_source,
        wl_pointer, wl_registry, wl_seat, wl_shm, wl_surface,
    },
};
use winit::raw_window_handle::{
    HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
};

const MIME: &str = "application/x-evoparseq-pointer-drag";
const KEY: &str = "native-display-drag";
pub type Observer =
    Arc<dyn Fn(egui::ViewportId, &winit::window::Window, &winit::event::WindowEvent) + Send + Sync>;
struct Gesture {
    origin: Arc<Playback>,
    root: Arc<Playback>,
    source: wl_data_source::WlDataSource,
    target: Option<usize>,
    inside_video: bool,
    last: (usize, crate::desktop_input::Pointer),
    ended: bool,
    icon: Option<wl_surface::WlSurface>,
    icon_visible: bool,
    icon_positioned: bool,
}
struct DragIcon {
    _theme: wayland_cursor::CursorTheme,
    image: wayland_cursor::CursorImageBuffer,
}
struct State {
    ctx: egui::Context,
    manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    device: Option<wl_data_device::WlDataDevice>,
    seat: Option<wl_seat::WlSeat>,
    pointer: Option<wl_pointer::WlPointer>,
    focus: Option<wayland_client::backend::ObjectId>,
    enter_serial: Option<u32>,
    press: Option<(u32, wayland_client::backend::ObjectId)>,
    surfaces: [Option<wl_surface::WlSurface>; crate::display_stream::STREAMS],
    gesture: Option<Gesture>,
    offer: Option<wl_data_offer::WlDataOffer>,
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    icon: Option<DragIcon>,
}
impl State {
    fn show_icon(&mut self, visible: bool) {
        let Some(g) = &mut self.gesture else { return };
        let (Some(surface), Some(icon)) = (&g.icon, &self.icon) else {
            return;
        };
        if g.icon_visible == visible || (g.ended && visible) {
            return;
        }
        if visible {
            let (x, y) = if g.icon_positioned {
                (0, 0)
            } else {
                let (x, y) = icon.image.hotspot();
                (-(x as i32), -(y as i32))
            };
            surface.attach(Some(&icon.image), x, y);
            let (w, h) = icon.image.dimensions();
            surface.damage(0, 0, w as i32, h as i32);
            g.icon_positioned = true;
        } else {
            surface.attach(None, 0, 0);
        }
        surface.commit();
        g.icon_visible = visible;
        crate::event_log::record(
            "INFO",
            "wayland_display_drag_local_pointer",
            serde_json::json!({"visible":visible}),
        );
    }
    fn position(&mut self, x: f64, y: f64) {
        let Some(g) = &mut self.gesture else { return };
        if g.ended {
            return;
        }
        let Some(stream) = g.target else {
            self.show_icon(true);
            return;
        };
        let Some(target) = g.root.display(stream).filter(|p| !p.stopped()) else {
            self.end(true);
            return;
        };
        let zoom = f64::from(self.ctx.zoom_factor());
        if let Some(pointer) =
            target.desktop_pointer(egui::pos2((x / zoom) as f32, (y / zoom) as f32), 1)
        {
            if !g.inside_video {
                crate::event_log::record(
                    "INFO",
                    "wayland_display_drag_video_boundary",
                    serde_json::json!({"stream":stream,"inside_video":true}),
                );
            }
            g.inside_video = true;
            g.last = (stream, pointer);
            g.root.push_routed(Event::DisplayDrag {
                stream,
                pointer,
                phase: DragPhase::Motion,
            });
        } else {
            if g.inside_video {
                crate::event_log::record(
                    "INFO",
                    "wayland_display_drag_video_boundary",
                    serde_json::json!({"stream":stream,"inside_video":false}),
                );
            }
            g.inside_video = false;
        }
        let show = !g.inside_video;
        self.show_icon(show);
    }
    fn end(&mut self, cancelled: bool) {
        self.show_icon(false);
        let Some(g) = &mut self.gesture else { return };
        if g.ended {
            return;
        }
        g.ended = true;
        let cancelled = cancelled || g.target.is_none() || !g.inside_video;
        let (stream, mut pointer) = g.last;
        pointer.buttons = 0;
        pointer.moved = !cancelled;
        g.root.push_routed(Event::DisplayDrag {
            stream,
            pointer,
            phase: if cancelled {
                DragPhase::Cancel
            } else {
                DragPhase::Drop
            },
        });
        let id = if g.origin.stream() == 0 {
            egui::ViewportId::ROOT
        } else {
            crate::display_routes::viewport(g.origin.stream())
        };
        self.ctx
            .input_mut_for(id, |input| input.pointer = Default::default());
        crate::event_log::record(
            "INFO",
            "wayland_display_drag_ended",
            serde_json::json!({"cancelled":cancelled,"stream":stream}),
        );
    }
    fn clear(&mut self) {
        if let Some(offer) = self.offer.take() {
            offer.destroy();
        }
        if let Some(g) = self.gesture.take() {
            g.source.destroy();
            if let Some(icon) = g.icon {
                icon.destroy();
            }
            if let Some(icon) = g.icon {
                icon.destroy();
            }
        }
        if let Some(device) = self.device.take() {
            device.release();
        }
    }
}
impl Dispatch<wl_registry::WlRegistry, ()> for State {
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
            if interface == "wl_data_device_manager" && version >= 3 {
                s.manager = Some(r.bind(name, 3, q, ()));
            }
            if interface == "wl_seat" && s.seat.is_none() {
                s.seat = Some(r.bind(name, version.min(7), q, ()));
            }
            if interface == "wl_compositor" {
                s.compositor = Some(r.bind(name, version.min(4), q, ()));
            }
            if interface == "wl_shm" {
                s.shm = Some(r.bind(name, 1, q, ()));
            }
        }
    }
}
impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        s: &mut Self,
        seat: &wl_seat::WlSeat,
        e: wl_seat::Event,
        _: &(),
        _: &Connection,
        q: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: wayland_client::WEnum::Value(c),
        } = e
        {
            if c.contains(wl_seat::Capability::Pointer) && s.pointer.is_none() {
                s.pointer = Some(seat.get_pointer(q, ()));
            } else if !c.contains(wl_seat::Capability::Pointer) {
                s.end(true);
                s.clear();
                s.press = None;
                s.focus = None;
                if let Some(p) = s.pointer.take() {
                    if p.version() >= 3 {
                        p.release();
                    }
                }
            }
        }
    }
}
impl Dispatch<wl_pointer::WlPointer, ()> for State {
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
                serial, surface, ..
            } => {
                s.focus = Some(surface.id());
                s.enter_serial = Some(serial);
            }
            wl_pointer::Event::Leave { .. } => s.focus = None,
            wl_pointer::Event::Button {
                serial,
                button: 272,
                state: wayland_client::WEnum::Value(wl_pointer::ButtonState::Pressed),
                ..
            } => {
                s.press = s.focus.clone().map(|id| (serial, id));
            }
            wl_pointer::Event::Button {
                button: 272,
                state: wayland_client::WEnum::Value(wl_pointer::ButtonState::Released),
                ..
            } => {
                s.press = None;
                // A rejected start_drag still delivers ordinary pointer release.
                if s.gesture.is_some() {
                    s.end(true);
                    s.clear();
                }
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_data_device::WlDataDevice, ()> for State {
    fn event(
        s: &mut Self,
        _: &wl_data_device::WlDataDevice,
        e: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match e {
            wl_data_device::Event::Enter {
                serial,
                surface,
                x,
                y,
                id,
            } => {
                if s.gesture.is_none() {
                    if let Some(offer) = id {
                        offer.destroy();
                    }
                    return;
                }
                let target = s
                    .surfaces
                    .iter()
                    .position(|p| p.as_ref().is_some_and(|p| p.id() == surface.id()));
                s.gesture.as_mut().unwrap().target = target;
                crate::event_log::record(
                    "INFO",
                    "wayland_display_drag_target",
                    serde_json::json!({"stream":target}),
                );
                if let Some(offer) = id {
                    offer.accept(serial, target.map(|_| MIME.to_owned()));
                    offer.set_actions(
                        wl_data_device_manager::DndAction::Move,
                        wl_data_device_manager::DndAction::Move,
                    );
                    s.offer = Some(offer);
                }
                s.position(x, y);
            }
            wl_data_device::Event::Motion { x, y, .. } => s.position(x, y),
            wl_data_device::Event::Leave => {
                if let Some(g) = &mut s.gesture {
                    crate::event_log::record(
                        "INFO",
                        "wayland_display_drag_left",
                        serde_json::json!({"stream":g.target,"ended":g.ended}),
                    );
                    g.target = None;
                    g.inside_video = false;
                }
                s.show_icon(true);
                if let Some(offer) = s.offer.take() {
                    offer.destroy();
                }
            }
            wl_data_device::Event::Drop => {
                s.end(false);
                if let Some(offer) = s.offer.take() {
                    offer.finish();
                    offer.destroy();
                }
            }
            wl_data_device::Event::Selection { id: Some(offer) } => offer.destroy(),
            _ => {}
        }
    }
    wayland_client::event_created_child!(State, wl_data_device::WlDataDevice, [
        0 => (wl_data_offer::WlDataOffer, ())
    ]);
}
impl Dispatch<wl_data_source::WlDataSource, ()> for State {
    fn event(
        s: &mut Self,
        source: &wl_data_source::WlDataSource,
        e: wl_data_source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if !s.gesture.as_ref().is_some_and(|g| g.source == *source) {
            return;
        }
        match e {
            wl_data_source::Event::DndDropPerformed => s.end(false),
            wl_data_source::Event::Cancelled => {
                s.end(true);
                s.clear();
            }
            wl_data_source::Event::DndFinished => {
                s.end(false);
                s.clear();
            }
            // No file, clipboard or host data is offered by this private gesture.
            // Dropping the Send event closes its fd without disclosing any data.
            _ => {}
        }
    }
}
wayland_client::delegate_noop!(State:ignore wl_data_device_manager::WlDataDeviceManager);
wayland_client::delegate_noop!(State:ignore wl_data_offer::WlDataOffer);
wayland_client::delegate_noop!(State:ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(State:ignore wl_shm::WlShm);
wayland_client::delegate_noop!(State:ignore wl_surface::WlSurface);

pub struct Drag {
    connection: Connection,
    queue: EventQueue<State>,
    state: State,
    _registry: wl_registry::WlRegistry,
}
impl Drag {
    pub fn install(
        ctx: &egui::Context,
        window: &(impl HasDisplayHandle + HasWindowHandle),
    ) -> Result<Arc<Mutex<Self>>> {
        let RawDisplayHandle::Wayland(display) = window.display_handle()?.as_raw() else {
            anyhow::bail!("Not Wayland")
        };
        // Lifetime is owned by Desktop and ends before eframe destroys its windows.
        let backend = unsafe {
            wayland_client::backend::Backend::from_foreign_display(display.display.as_ptr().cast())
        };
        let mut result = Self::connect(Connection::from_backend(backend), ctx.clone())?;
        result.surface(0, window)?;
        let result = Arc::new(Mutex::new(result));
        let weak = Arc::downgrade(&result);
        let observer: Observer = Arc::new(move |id, window, _event| {
            if let Some(drag) = weak.upgrade() {
                let mut d = drag.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(stream) = (0..crate::display_stream::STREAMS).find(|s| {
                    id == if *s == 0 {
                        egui::ViewportId::ROOT
                    } else {
                        crate::display_routes::viewport(*s)
                    }
                }) {
                    let _ = d.surface(stream, window);
                }
                d.dispatch();
            }
        });
        ctx.data_mut(|d| {
            d.insert_temp(egui::Id::new(KEY), Arc::downgrade(&result));
            d.insert_temp(egui::Id::new("native-window-observer"), observer);
        });
        Ok(result)
    }
    pub(crate) fn connect(connection: Connection, ctx: egui::Context) -> Result<Self> {
        let mut queue = connection.new_event_queue();
        let registry = connection.display().get_registry(&queue.handle(), ());
        let mut state = State {
            ctx,
            manager: None,
            device: None,
            seat: None,
            pointer: None,
            focus: None,
            enter_serial: None,
            press: None,
            surfaces: Default::default(),
            gesture: None,
            offer: None,
            compositor: None,
            shm: None,
            icon: None,
        };
        queue.roundtrip(&mut state)?;
        // Static native cursor buffer only: the compositor moves this DnD icon
        // between windows. It is never drawn into a decoded video surface.
        let icon = (|| -> Result<DragIcon> {
            let mut theme = wayland_cursor::CursorTheme::load_or(
                &connection,
                state.shm.clone().context("No shared memory")?,
                "Adwaita",
                24,
            )?;
            let image = theme.get_cursor("default").context("No default cursor")?[0].clone();
            Ok(DragIcon {
                _theme: theme,
                image,
            })
        })();
        state.icon = icon
            .map_err(|_| {
                crate::event_log::record(
                    "WARN",
                    "wayland_drag_pointer_unavailable",
                    serde_json::json!({}),
                )
            })
            .ok();
        state.manager.as_ref().context("DnD v3 unavailable")?;
        state.seat.as_ref().context("Seat unavailable")?;
        queue.roundtrip(&mut state)?;
        Ok(Self {
            connection,
            queue,
            state,
            _registry: registry,
        })
    }
    pub(crate) fn surface(&mut self, stream: usize, window: &impl HasWindowHandle) -> Result<()> {
        let RawWindowHandle::Wayland(surface) = window.window_handle()?.as_raw() else {
            anyhow::bail!("Not Wayland")
        };
        let id = unsafe {
            wayland_client::backend::ObjectId::from_ptr(
                wl_surface::WlSurface::interface(),
                surface.surface.as_ptr().cast(),
            )
        }
        .context("Surface unavailable")?;
        if !self.state.surfaces[stream]
            .as_ref()
            .is_some_and(|s| s.id() == id)
        {
            self.state.surfaces[stream] =
                Some(wl_surface::WlSurface::from_id(&self.connection, id)?);
        }
        Ok(())
    }
    pub fn dispatch(&mut self) {
        if self.queue.dispatch_pending(&mut self.state).is_err() {
            self.state.end(true);
            self.state.clear();
        }
        if self
            .state
            .gesture
            .as_ref()
            .is_some_and(|g| !g.ended && (!g.origin.input_active() || g.origin.stopped()))
        {
            self.state.end(true);
            self.state.clear();
        }
        let _ = self.connection.flush();
    }
    pub(crate) fn begin(&mut self, origin: &Arc<Playback>, point: egui::Pos2) -> Result<()> {
        self.dispatch();
        anyhow::ensure!(
            self.state.gesture.is_none() && origin.owns_input() && origin.input_active(),
            "Not a desktop drag"
        );
        let root = origin.primary().context("No primary display")?;
        anyhow::ensure!(
            (1..crate::display_stream::STREAMS).any(|s| root.display(s).is_some()),
            "No peer display"
        );
        let pointer = origin.desktop_pointer(point, 1).context("Outside video")?;
        let surface = self.state.surfaces[origin.stream()]
            .as_ref()
            .context("Native surface unavailable")?;
        let (serial, id) = self.state.press.clone().context("No held button")?;
        anyhow::ensure!(id == surface.id(), "Button belongs to another surface");
        // Hide the local duplicate while pointer focus still belongs to us.
        // set_cursor requires the enter serial, not the button's drag serial.
        self.state
            .pointer
            .as_ref()
            .context("No pointer")?
            .set_cursor(
                self.state.enter_serial.context("No pointer enter")?,
                None,
                0,
                0,
            );
        let source = self
            .state
            .manager
            .as_ref()
            .unwrap()
            .create_data_source(&self.queue.handle(), ());
        // Bind only for this gesture, after click-to-focus. Mutter selects the
        // latest data device; a later clipboard binding can otherwise swallow
        // the first drag. Leave the shared clipboard's focused device intact.
        self.state.device = Some(self.state.manager.as_ref().unwrap().get_data_device(
            self.state.seat.as_ref().unwrap(),
            &self.queue.handle(),
            (),
        ));
        source.offer(MIME.to_owned());
        source.set_actions(wl_data_device_manager::DndAction::Move);
        origin.native_drag.store(true, Ordering::Release);
        let icon = self
            .state
            .icon
            .as_ref()
            .and_then(|_| self.state.compositor.as_ref())
            .map(|c| c.create_surface(&self.queue.handle(), ()));
        root.push_routed(Event::DisplayDrag {
            stream: origin.stream(),
            pointer,
            phase: DragPhase::Begin,
        });
        self.state.device.as_ref().unwrap().start_drag(
            Some(&source),
            surface,
            icon.as_ref(),
            serial,
        );
        self.state.gesture = Some(Gesture {
            origin: origin.clone(),
            root,
            source,
            target: Some(origin.stream()),
            inside_video: true,
            last: (origin.stream(), pointer),
            ended: false,
            icon,
            icon_visible: false,
            icon_positioned: false,
        });
        if let Err(error) = self.connection.flush() {
            self.state.end(true);
            self.state.clear();
            return Err(error.into());
        }
        crate::event_log::record(
            "INFO",
            "wayland_display_drag_started",
            serde_json::json!({"stream":origin.stream()}),
        );
        Ok(())
    }
}
pub fn begin(ctx: &egui::Context, origin: &Arc<Playback>, point: egui::Pos2) -> bool {
    ctx.data(|d| d.get_temp::<std::sync::Weak<Mutex<Drag>>>(egui::Id::new(KEY)))
        .and_then(|d| d.upgrade())
        .is_some_and(|d| {
            d.lock()
                .unwrap_or_else(|e| e.into_inner())
                .begin(origin, point)
                .is_ok()
        })
}
impl Drop for Drag {
    fn drop(&mut self) {
        self.state.end(true);
        self.state.clear();
        if let Some(p) = self.state.pointer.take() {
            if p.version() >= 3 {
                p.release();
            }
        }
        if let Some(d) = self.state.device.take() {
            d.release();
        }
        if let Some(s) = self.state.seat.take() {
            if s.version() >= 5 {
                s.release();
            }
        }
        let _ = self.connection.flush();
    }
}
