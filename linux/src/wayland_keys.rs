//! Public Wayland keyboard-shortcuts-inhibit protocol on winit's existing
//! display/surface. One protocol queue, dispatched by the existing UI loop;
//! no input hook, polling thread, private window access or second display.
use anyhow::{Context, Result};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle,
    protocol::{wl_registry, wl_seat, wl_surface},
};
use wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::{
    zwp_keyboard_shortcuts_inhibit_manager_v1 as manager,
    zwp_keyboard_shortcuts_inhibitor_v1 as inhibitor,
};
use winit::raw_window_handle::{
    HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
};
#[derive(Default)]
struct State {
    manager: Option<manager::ZwpKeyboardShortcutsInhibitManagerV1>,
    seat: Option<wl_seat::WlSeat>,
    inhibitor: Option<inhibitor::ZwpKeyboardShortcutsInhibitorV1>,
    active: bool,
    requested: bool,
}
impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        q: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            if interface == "zwp_keyboard_shortcuts_inhibit_manager_v1" {
                state.manager = Some(registry.bind(name, version.min(1), q, ()));
            } else if interface == "wl_seat" && state.seat.is_none() {
                state.seat = Some(registry.bind(name, version.min(7), q, ()));
            }
        }
    }
}
wayland_client::delegate_noop!(State:ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(State:ignore manager::ZwpKeyboardShortcutsInhibitManagerV1);
impl Dispatch<inhibitor::ZwpKeyboardShortcutsInhibitorV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &inhibitor::ZwpKeyboardShortcutsInhibitorV1,
        event: inhibitor::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.active = matches!(event, inhibitor::Event::Active);
        crate::event_log::record(
            "INFO",
            "keyboard_shortcuts_inhibition_changed",
            serde_json::json!({"active":state.active}),
        );
    }
}
pub struct Shortcuts {
    queue: EventQueue<State>,
    state: State,
    connection: Connection,
    surface: wl_surface::WlSurface,
    _registry: wl_registry::WlRegistry,
}
impl Shortcuts {
    pub fn new(window: &(impl HasDisplayHandle + HasWindowHandle)) -> Result<Self> {
        let (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(surface)) = (
            window.display_handle()?.as_raw(),
            window.window_handle()?.as_raw(),
        ) else {
            anyhow::bail!("Wayland surface unavailable")
        };
        // The eframe app is dropped before its root native window. The foreign
        // display and surface remain owned exclusively by winit.
        let backend = unsafe {
            wayland_client::backend::Backend::from_foreign_display(display.display.as_ptr().cast())
        };
        let connection = Connection::from_backend(backend);
        let id = unsafe {
            wayland_client::backend::ObjectId::from_ptr(
                wl_surface::WlSurface::interface(),
                surface.surface.as_ptr().cast(),
            )
        }
        .context("Wayland surface ID unavailable")?;
        let surface = wl_surface::WlSurface::from_id(&connection, id)?;
        let mut queue = connection.new_event_queue();
        let registry = connection.display().get_registry(&queue.handle(), ());
        let mut state = State::default();
        queue.roundtrip(&mut state)?; // Once at native window creation only.
        Ok(Self {
            queue,
            state,
            connection,
            surface,
            _registry: registry,
        })
    }
    pub fn available(&self) -> bool {
        self.state.manager.is_some() && self.state.seat.is_some()
    }
    pub fn update(&mut self, requested: bool) {
        if self.queue.dispatch_pending(&mut self.state).is_err() {
            self.state.active = false;
            return;
        }
        if requested == self.state.requested {
            return;
        }
        self.state.requested = requested;
        if requested {
            if let (Some(manager), Some(seat)) = (&self.state.manager, &self.state.seat) {
                self.state.inhibitor =
                    Some(manager.inhibit_shortcuts(&self.surface, seat, &self.queue.handle(), ()));
            }
        } else if let Some(inhibitor) = self.state.inhibitor.take() {
            inhibitor.destroy();
            self.state.active = false;
        }
        let _ = self.connection.flush();
        crate::event_log::record(
            "INFO",
            "keyboard_shortcuts_requested",
            serde_json::json!({"requested":requested,"supported":self.available()}),
        );
    }
}
impl Drop for Shortcuts {
    fn drop(&mut self) {
        if let Some(inhibitor) = self.state.inhibitor.take() {
            if inhibitor.is_alive() {
                inhibitor.destroy();
            }
        }
        if let Some(manager) = self.state.manager.take() {
            if manager.is_alive() {
                manager.destroy();
            }
        }
        if let Some(seat) = self.state.seat.take() {
            if seat.is_alive() && seat.version() >= 5 {
                seat.release();
            }
        }
        let _ = self.connection.flush();
    }
}
