//! Optional public Wayland pointer-warp adapter. Reuses winit's display and
//! existing UI dispatch; no second connection, input injection or worker.
use anyhow::{Context, Result};
use std::sync::{Arc, Mutex};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle,
    protocol::{wl_pointer, wl_registry, wl_seat, wl_surface},
};
use wayland_protocols::wp::pointer_warp::v1::client::wp_pointer_warp_v1 as warp;
use winit::raw_window_handle::{
    HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
};

#[derive(Default)]
struct State {
    manager: Option<warp::WpPointerWarpV1>,
    seat: Option<wl_seat::WlSeat>,
    pointer: Option<wl_pointer::WlPointer>,
    serial: Option<u32>,
    pending: Option<(wayland_client::backend::ObjectId, f64, f64)>,
}
impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        s: &mut Self,
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
            if interface == "wp_pointer_warp_v1" {
                s.manager = Some(registry.bind(name, 1, q, ()));
            } else if interface == "wl_seat" && s.seat.is_none() {
                s.seat = Some(registry.bind(name, version.min(7), q, ()));
            }
        }
    }
}
impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        s: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        q: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: wayland_client::WEnum::Value(caps),
        } = event
        {
            if caps.contains(wl_seat::Capability::Pointer) && s.pointer.is_none() {
                s.pointer = Some(seat.get_pointer(q, ()));
            } else if !caps.contains(wl_seat::Capability::Pointer) {
                if let Some(p) = s.pointer.take() {
                    if p.version() >= 3 {
                        p.release();
                    }
                }
                s.serial = None;
            }
        }
    }
}
impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(
        s: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_pointer::Event::Enter {
            serial,
            surface,
            surface_x,
            surface_y,
        } = event
        {
            s.serial = Some(serial);
            if s.pending
                .as_ref()
                .is_some_and(|(id, _, _)| *id == surface.id())
            {
                let (_, x, y) = s.pending.take().unwrap();
                crate::event_log::record(
                    "INFO",
                    "cursor_peer_surface_entered",
                    serde_json::json!({
                        "after_warp_request":true,
                        "position_matches_request":(surface_x-x).abs()<=1./256. && (surface_y-y).abs()<=1./256.
                    }),
                );
            }
        }
    }
}
wayland_client::delegate_noop!(State:ignore warp::WpPointerWarpV1);

pub struct Warp {
    queue: EventQueue<State>,
    state: State,
    connection: Connection,
    _registry: wl_registry::WlRegistry,
}
impl Warp {
    pub fn install(
        ctx: &eframe::egui::Context,
        window: &impl HasDisplayHandle,
    ) -> Result<Arc<Mutex<Self>>> {
        let RawDisplayHandle::Wayland(display) = window.display_handle()?.as_raw() else {
            anyhow::bail!("Wayland display unavailable")
        };
        // The app owns this adapter and drops it before winit's native windows.
        let backend = unsafe {
            wayland_client::backend::Backend::from_foreign_display(display.display.as_ptr().cast())
        };
        let connection = Connection::from_backend(backend);
        let mut queue = connection.new_event_queue();
        let registry = connection.display().get_registry(&queue.handle(), ());
        let mut state = State::default();
        queue.roundtrip(&mut state)?;
        queue.roundtrip(&mut state)?;
        let supported = state.manager.is_some();
        let result = Arc::new(Mutex::new(Self {
            queue,
            state,
            connection,
            _registry: registry,
        }));
        anyhow::ensure!(supported, "Compositor does not advertise pointer warp");
        let weak = Arc::downgrade(&result);
        let handler: Arc<
            dyn Fn(&winit::window::Window, winit::dpi::PhysicalPosition<f64>) -> bool + Send + Sync,
        > = Arc::new(move |window, position| {
            weak.upgrade().is_some_and(|warp| {
                warp.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .request(window, position)
            })
        });
        ctx.data_mut(|d| d.insert_temp(eframe::egui::Id::new("cursor-position-handler"), handler));
        Ok(result)
    }
    pub fn dispatch(&mut self) {
        // Drain pointer enter/motion notifications in the existing UI loop;
        // never retain mouse events until the next rare cursor transition.
        if self.queue.dispatch_pending(&mut self.state).is_err() {
            self.state.serial = None;
        }
    }
    fn request(
        &mut self,
        window: &winit::window::Window,
        position: winit::dpi::PhysicalPosition<f64>,
    ) -> bool {
        let result = (|| -> Result<()> {
            self.queue.dispatch_pending(&mut self.state)?;
            let RawWindowHandle::Wayland(handle) = window.window_handle()?.as_raw() else {
                anyhow::bail!("Not Wayland")
            };
            // A transition can target another live surface of this same client.
            let id = unsafe {
                wayland_client::backend::ObjectId::from_ptr(
                    wl_surface::WlSurface::interface(),
                    handle.surface.as_ptr().cast(),
                )
            }
            .context("Surface ID unavailable")?;
            let surface = wl_surface::WlSurface::from_id(&self.connection, id)?;
            let position = position.to_logical::<f64>(window.scale_factor());
            self.state
                .manager
                .as_ref()
                .context("Warp manager unavailable")?
                .warp_pointer(
                    &surface,
                    self.state.pointer.as_ref().context("Pointer unavailable")?,
                    position.x,
                    position.y,
                    self.state
                        .serial
                        .context("Pointer enter serial unavailable")?,
                );
            self.connection.flush()?;
            self.state.pending = Some((surface.id(), position.x, position.y));
            Ok(())
        })();
        crate::event_log::record(
            "INFO",
            "cursor_endpoint_warp_requested",
            serde_json::json!({"requested":result.is_ok(),"compositor_acknowledgement":false}),
        );
        result.is_ok()
    }
}
impl Drop for Warp {
    fn drop(&mut self) {
        if let Some(p) = self.state.pointer.take() {
            if p.is_alive() && p.version() >= 3 {
                p.release();
            }
        }
        if let Some(s) = self.state.seat.take() {
            if s.is_alive() && s.version() >= 5 {
                s.release();
            }
        }
        if let Some(m) = self.state.manager.take() {
            if m.is_alive() {
                m.destroy();
            }
        }
        let _ = self.connection.flush();
    }
}
