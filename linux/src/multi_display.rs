//! Wayland display viewports and Vulkan decoder ownership only.
use crate::{
    desktop_linux::DesktopInput,
    display_routes::{Channel, viewport},
    playback::Playback,
    video_vulkan,
};
use anyhow::Result;
use eframe::egui;
use std::sync::Arc;
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
                                ui.menu_button(crate::app_style::forms::MENU_HELP,|ui| {
                                    for (label,mode) in [(crate::app_style::forms::MENU_ABOUT,crate::information_linux::Mode::About),(crate::app_style::forms::MENU_UPDATES,crate::information_linux::Mode::Updates)] {
                                        if ui.button(label).clicked(){surface.input.release(child,&channel.playback);self.ui_state.information.open(ctx,mode);ui.close();}
                                    }
                                });
                            });
                        });
                    egui::CentralPanel::default()
                        .frame(egui::Frame::new().fill(egui::Color32::BLACK))
                        .show(child, |ui| {
                            let area = surface.decoder.show(ui);
                            surface.input.video = area.map(|a| a.0);
                            channel.playback.set_video_rect(surface.input.video);
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
                    let (area, _) = ui.allocate_exact_size(
                        egui::vec2(crate::app_style::session::PANEL_WIDTH as f32, 328.),
                        egui::Sense::hover(),
                    );
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
                    for (i, (text, rect)) in
                        crate::app_style::forms::secondary_actions(surface.input.enabled)
                            .iter()
                            .enumerate()
                    {
                        if ui.put(r(*rect), egui::Button::new(*text)).clicked() {
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
