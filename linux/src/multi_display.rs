//! Linux viewport/decoder adapters. Wire framing, input and resolution policy
//! are shared with Windows. One connection/audio output, one decoder per screen.
use crate::{desktop_linux::DesktopInput, playback::Playback, video_vulkan};
use anyhow::{Result, ensure};
use eframe::egui;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
pub struct Displays {
    slots: Mutex<[Option<Arc<Channel>>; crate::display_stream::STREAMS - 1]>,
    pub owner: AtomicUsize,
    pub hevc: std::sync::atomic::AtomicBool,
    release: std::sync::atomic::AtomicBool,
}
pub struct Channel {
    pub playback: Arc<Playback>,
    pub inspector: Arc<Mutex<crate::video_stream::Inspector>>,
    ingress: Mutex<crate::bud_ingress::VideoState>,
    opened: std::time::Instant,
}
pub fn viewport(stream: usize) -> egui::ViewportId {
    egui::ViewportId::from_hash_of(("display-stream", stream))
}
impl Displays {
    pub fn flags(&self) -> i32 {
        8 | i32::from(self.hevc.load(Ordering::Acquire))
    }
    pub fn get(&self, stream: usize) -> Option<Arc<Channel>> {
        self.slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(stream.checked_sub(1)?)?
            .clone()
    }
    pub fn open(self: &Arc<Self>, stream: usize, ctx: &egui::Context) -> Result<()> {
        ensure!(
            (1..crate::display_stream::STREAMS).contains(&stream),
            "Invalid display stream"
        );
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        ensure!(slots[stream - 1].is_none(), "Display is already open");
        let playback = Playback::new(ctx.clone());
        playback.attach_displays(self, stream);
        playback.set_hevc(self.hevc.load(Ordering::Acquire));
        let mut inspector = crate::video_stream::Inspector::default();
        inspector.configure(crate::video_stream::VideoProtocol {
            version: 1,
            message_size: 11,
            version_offset: 0,
            flag_offset: 4,
        });
        inspector.enable_hevc(playback.hevc());
        slots[stream - 1] = Some(Arc::new(Channel {
            opened: std::time::Instant::now(),
            playback: playback.clone(),
            inspector: Arc::new(Mutex::new(inspector)),
            ingress: Mutex::new(crate::bud_ingress::VideoState::new(
                playback.hevc(),
                false,
                false,
            )),
        }));
        Ok(())
    }
    pub fn close(&self, stream: usize) {
        if let Some(index) = stream.checked_sub(1) {
            let channel = self
                .slots
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_mut(index)
                .and_then(Option::take);
            if let Some(channel) = channel {
                if self
                    .owner
                    .compare_exchange(stream + 1, 0, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    self.release.store(true, Ordering::Release);
                }
                channel.playback.stop();
                channel.playback.repaint();
            }
        }
    }
    pub fn clear(&self) {
        for stream in 1..crate::display_stream::STREAMS {
            self.close(stream);
        }
    }
    pub fn poll(
        &self,
        primary: &Playback,
        mut send: impl FnMut(&[u8]) -> Result<()>,
    ) -> Result<()> {
        if self.release.swap(false, Ordering::AcqRel) {
            send(&crate::control::header(24, 0, 0, 0))?;
        }
        for stream in 1..crate::display_stream::STREAMS {
            let Some(channel) = self.get(stream) else {
                continue;
            };
            channel.playback.bind_input_consumer();
            channel.playback.poll_gpu();
            if primary.stopped()
                || channel.playback.stopped()
                || (channel.opened.elapsed() >= std::time::Duration::from_secs(20)
                    && channel.playback.video_snapshot().frames_decoded == 0)
            {
                send(&crate::display_stream::enable(stream, false)?)?;
                self.close(stream);
            } else {
                match channel.playback.take_keyframe_request() {
                    Ok(Some(_)) => {
                        channel
                            .ingress
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .wait_for_keyframe();
                        send(&crate::control::header(13, stream as i32, self.flags(), 1))?;
                    }
                    Err(_) => {
                        channel.playback.stop();
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
    pub fn bud(&self, source: &mut evoparseq_bud::bud::MediaSource) -> Result<bool> {
        let mut pending = false;
        for stream in 1..crate::display_stream::STREAMS {
            let channel = self.get(stream);
            for index in 0..crate::bud_ingress::PUMP_BUDGET {
                if source
                    .with_secondary_video(stream, |bytes| {
                        if let Some(channel) = &channel {
                            if channel
                                .ingress
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .receive(bytes, channel.playback.as_ref())
                                .is_err()
                            {
                                channel.playback.stop();
                            }
                        }
                        Ok(())
                    })?
                    .is_none()
                {
                    break;
                }
                pending |= index + 1 == crate::bud_ingress::PUMP_BUDGET;
            }
        }
        Ok(pending)
    }
}
impl Drop for Displays {
    fn drop(&mut self) {
        self.clear();
    }
}

pub struct Surface {
    channel: Arc<Channel>,
    decoder: video_vulkan::Decoder,
    input: DesktopInput,
    badge: crate::session_overlay::Badge,
    fitted: Option<(u32, u32)>,
    resolution: crate::session_preferences::Resolution,
}

/// Used for the primary and every additional decoder: no extra decode pass.
pub fn decode(
    playback: &Playback,
    decoder: &mut video_vulkan::Decoder,
    focused: bool,
) -> Result<usize> {
    playback.resume_if_ready(focused)?;
    let available = std::cell::Cell::new(decoder.available()?);
    let consumed = playback.decode_pending_ready(
        || available.get() > 0,
        |bytes, codec, rotation| {
            available.set(decoder.submit_oriented(bytes, codec, rotation)?);
            Ok(())
        },
    )?;
    decoder.defer_backlog(playback);
    Ok(consumed)
}

pub fn fit(ctx: &egui::Context, source: (u32, u32), fitted: &mut Option<(u32, u32)>) {
    if *fitted == Some(source) {
        return;
    }
    let view = ctx.input(|i| i.viewport().clone());
    if view.fullscreen.unwrap_or(false) || view.maximized.unwrap_or(false) {
        return;
    }
    if let Some(monitor) = view.monitor_size {
        if let Some((w, h)) = crate::viewport::stream_window_size(
            source,
            ctx.pixels_per_point(),
            (monitor.x, monitor.y),
        ) {
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(160., 90.)));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w, h)));
            *fitted = Some(source);
        }
    }
}

impl crate::Desktop {
    pub(super) fn additional_displays(&mut self, ctx: &egui::Context) {
        let Some(telemetry) = self.telemetry.clone() else {
            self.screens = Default::default();
            return;
        };
        let Some(context) = self.video_context.clone() else {
            return;
        };
        for stream in 1..crate::display_stream::STREAMS {
            let slot = &mut self.screens[stream - 1];
            let Some(channel) = telemetry.displays.get(stream) else {
                *slot = None;
                continue;
            };
            if channel.playback.stopped() {
                continue;
            }
            if slot
                .as_ref()
                .is_none_or(|surface| !Arc::ptr_eq(&surface.channel, &channel))
            {
                *slot = Some(Surface {
                    channel: channel.clone(),
                    decoder: video_vulkan::Decoder::new(context.clone(), self.output_format),
                    input: DesktopInput::new(self.desktop_input.keyboard()),
                    badge: Default::default(),
                    fitted: None,
                    resolution: crate::session_preferences::Resolution::Host,
                });
            }
            let surface = slot.as_mut().unwrap();
            let parent = self.playback.clone();
            ctx.show_viewport_immediate(
                viewport(stream),
                egui::ViewportBuilder::default()
                    .with_title(format!("EvoParseq - Display {}", stream + 1))
                    .with_decorations(true)
                    .with_inner_size([960., 564.])
                    .with_resizable(true),
                |child, _| {
                    if child.input(|i| i.viewport().close_requested()) {
                        surface.input.release(child, &channel.playback);
                        channel.playback.stop();
                        return;
                    }
                    let mut raw = child.input(|i| i.raw.clone());
                    channel.playback.controller_focus(raw.focused&&!surface.badge.open);
                    surface.input.events(child, &mut raw, &channel.playback);
                    match decode(&channel.playback, &mut surface.decoder, raw.focused) {
                        Err(error) => {
                        crate::event_log::record("ERROR","additional_display_video_failed",serde_json::json!({"stream":stream,"vulkan_result":error.downcast_ref::<ash::vk::Result>().map(|result|result.as_raw())}));
                            channel.playback.stop();
                        }
                        Ok(n) if n > 0 => child.request_repaint(),
                        _ => {}
                    }
                    egui::TopBottomPanel::top("display-menu")
                        .exact_height(24.)
                        .show(child, |ui| {
                            ui.horizontal(|ui| {
                                if ui.button(crate::app_style::forms::MENU_SETTINGS).clicked() {
                                    surface.input.release(child, &channel.playback);
                                    surface.badge.restore();
                                }
                                ui.add_enabled(
                                    false,
                                    egui::Button::new(crate::app_style::forms::MENU_STATS),
                                );
                            });
                        });
                    egui::CentralPanel::default()
                        .frame(egui::Frame::new().fill(egui::Color32::BLACK))
                        .show(child, |ui| {
                            let area = surface.decoder.show(ui);
                            surface.input.video = area.map(|a| a.0);
                            if let Some((_, source)) = area {
                                channel.playback.set_input_geometry(source);
                                fit(child, source, &mut surface.fitted);
                            }
                            surface.input.cursor(child, &channel.playback);
                        });
                    surface
                        .badge
                        .button(child, &self.logo, &mut surface.input, &channel.playback);
                    if surface.badge.open {
                        secondary_controls(child, stream, surface, parent.as_ref(), &telemetry);
                    }
                    channel.playback.finish_clipboard(
                        surface.input.enabled && raw.focused && !surface.badge.open,
                    );
                },
            );
        }
    }
}

fn secondary_controls(
    ctx: &egui::Context,
    stream: usize,
    surface: &mut Surface,
    primary: Option<&Arc<Playback>>,
    telemetry: &crate::telemetry::Session,
) {
    use crate::launcher::{color, label, rect_at};
    let panel = egui::Area::new(egui::Id::new(("session-panel", stream)))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(16., 94.))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_unmultiplied(25, 29, 35, 238))
                .show(ui, |ui| {
                    let (area, _) =
                        ui.allocate_exact_size(egui::vec2(418., 328.), egui::Sense::hover());
                    let r = |rect| rect_at(area.min, rect);
                    let fg = color(crate::app_style::TEXT);
                    label(ui, r([24, 18, 350, 36]), "Session", 25., fg);
                    label(ui, r([24, 76, 145, 24]), "Resolution", 16., fg);
                    ui.scope_builder(
                        egui::UiBuilder::new().max_rect(r([174, 72, 218, 28])),
                        |ui| {
                            let mut value =
                                crate::session_preferences::resolution_text(surface.resolution);
                            egui::ComboBox::from_id_salt(("resolution", stream))
                                .width(208.)
                                .selected_text(&value)
                                .show_ui(ui, |ui| {
                                    for option in crate::session_preferences::RESOLUTION_CHOICES {
                                        ui.selectable_value(&mut value, (*option).into(), *option);
                                    }
                                });
                            if let Ok(resolution) = crate::session_preferences::resolution(&value) {
                                surface.resolution = resolution;
                            }
                        },
                    );
                    for (i, (text, x, y)) in [
                        ("Apply", 24, 120),
                        ("Fullscreen", 220, 120),
                        (
                            if surface.input.enabled {
                                "Release input"
                            } else {
                                "Capture input"
                            },
                            24,
                            172,
                        ),
                        ("Close screen", 220, 172),
                        ("Close", 220, 224),
                    ]
                    .iter()
                    .enumerate()
                    {
                        if ui
                            .put(r([*x, *y, 174, 36]), egui::Button::new(*text))
                            .clicked()
                        {
                            match i {
                                0 => {
                                    if let Some(primary) = primary {
                                        primary.push_session(
                                            crate::session_controls::Command::DisplayResolution {
                                                stream,
                                                resolution: surface.resolution,
                                                size: {
                                                    let size = ctx.content_rect().size()
                                                        * ctx.pixels_per_point();
                                                    (
                                                        size.x.round().max(1.) as u32,
                                                        (size.y - 24. * ctx.pixels_per_point())
                                                            .round()
                                                            .max(1.)
                                                            as u32,
                                                    )
                                                },
                                            },
                                        );
                                    }
                                }
                                1 => ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(
                                    !ctx.input(|i| i.viewport().fullscreen.unwrap_or(false)),
                                )),
                                2 => {
                                    if surface.input.enabled {
                                        surface.input.release(ctx, &surface.channel.playback);
                                    } else {
                                        surface.badge.open = false;
                                        surface.input.enable(&surface.channel.playback);
                                    }
                                }
                                3 => surface.channel.playback.stop(),
                                _ => surface.badge.open = false,
                            }
                        }
                    }
                    label(
                        ui,
                        r([24, 276, 370, 44]),
                        &telemetry
                            .host
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .status,
                        16.,
                        fg,
                    );
                });
        });
    surface.input.controls = Some(panel.response.rect);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linux_bounded_screens_are_independent_and_never_stop_the_primary() {
        let displays = Arc::new(Displays::default());
        let ctx = egui::Context::default();
        let primary = Playback::new(ctx.clone());
        primary.attach_displays(&displays, 0);
        assert!(displays.open(0, &ctx).is_err());
        assert!(displays.open(3, &ctx).is_err());
        displays.open(1, &ctx).unwrap();
        displays.open(2, &ctx).unwrap();
        assert!(displays.open(1, &ctx).is_err());
        let second = displays.get(1).unwrap();
        displays.close(1);
        assert!(second.playback.stopped());
        assert!(!displays.get(2).unwrap().playback.stopped());
        assert!(!primary.stopped());
        displays.clear();
        assert!(displays.get(2).is_none());
    }
}
