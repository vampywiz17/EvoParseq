//! Linux widgets for the shared launcher identity and account/picker models.
use crate::{Command, Desktop, account, app_style, connection};
use eframe::egui::{self, Color32, RichText};

pub fn color(rgb: [u8; 3]) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}
pub fn style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = color(app_style::BACKGROUND);
    style.visuals.window_fill = color(app_style::BACKGROUND);
    style.visuals.extreme_bg_color = color(app_style::FIELD);
    style.visuals.override_text_color = Some(color(app_style::TEXT));
    style.visuals.selection.bg_fill = color(app_style::BLUE);
    style.visuals.widgets.inactive.bg_fill = color(app_style::FIELD);
    style.visuals.widgets.hovered.bg_fill = color(app_style::BLUE).gamma_multiply(0.7);
    style.visuals.widgets.active.bg_fill = color(app_style::BLUE);
    // egui Button uses weak_bg_fill, not bg_fill.
    style.visuals.widgets.inactive.weak_bg_fill = color(app_style::FIELD);
    style.visuals.widgets.hovered.weak_bg_fill = color(app_style::FIELD);
    style.visuals.widgets.active.weak_bg_fill = color(app_style::FIELD);
    style.spacing.item_spacing = egui::vec2(12., 14.);
    style.spacing.button_padding = egui::vec2(8., 4.);
    style.spacing.interact_size.y = 24.;
    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::proportional(app_style::desktop::BODY as f32),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::proportional(app_style::desktop::BODY as f32),
    );
    style.text_styles.insert(
        egui::TextStyle::Heading,
        egui::FontId::proportional(app_style::desktop::HEADING as f32),
    );
    style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::ZERO;
    style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::ZERO;
    style.visuals.widgets.active.corner_radius = egui::CornerRadius::ZERO;
    style.text_styles.insert(
        egui::TextStyle::Small,
        egui::FontId::proportional(app_style::settings::SMALL as f32),
    );
    ctx.set_style(style);
}
impl Desktop {
    pub(super) fn start_stream(&mut self, ctx: &egui::Context) {
        let peer = match crate::picker::connection_peer(&self.peer) {
            Ok(peer) => peer,
            Err(error) => {
                self.error = Some(error.to_string());
                return;
            }
        };
        if let Some(context) = self.video_context.clone() {
            self.decoder = Some(crate::video_vulkan::Decoder::new(
                context,
                self.output_format,
            ));
            let playback = crate::playback::Playback::new(ctx.clone());
            self.desktop_input.video = None;
            self.badge = Default::default();
            self.launcher_size =
                ctx.input(|input| input.viewport().inner_rect.map(|rect| rect.size()));
            self.fitted_stream = None;
            self.playback = Some(playback.clone());
            self.send(Command::Stream(peer, self.transport, playback));
        }
    }

    pub(super) fn launcher(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        // A fixed logical canvas, as on Windows. Small viewports scroll instead
        // of shrinking label columns or changing control/text dimensions.
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.show_video {
                    self.settings(ui);
                    return;
                }
                use app_style::desktop as d;
                let origin = ui.cursor().min;
                let r = |rect| rect_at(origin, rect);
                ui.allocate_space(egui::vec2(d::CANVAS[0] as f32, d::CANVAS[1] as f32));
                // The Win32 owner-drawn logo is vertically centered in its slot.
                let slot = r(d::LOGO);
                ui.put(
                    egui::Rect::from_center_size(slot.center(), egui::vec2(40., 40.)),
                    egui::Image::new((self.logo.id(), egui::vec2(40., 40.))),
                );
                for i in 0..2 {
                    label(
                        ui,
                        r(d::WORDMARK[i]),
                        app_style::WORDMARK[i],
                        d::HEADING as f32,
                        if i == 1 {
                            color(app_style::BLUE)
                        } else {
                            color(app_style::TEXT)
                        },
                    );
                }
                if ui
                    .put(r(d::SETTINGS), egui::Button::new(d::SETTINGS_TEXT))
                    .clicked()
                    && !self.busy
                {
                    self.show_video = true;
                }
                label(
                    ui,
                    r(d::SUBTITLE),
                    app_style::SUBTITLE,
                    d::BODY as f32,
                    color(app_style::TEXT),
                );
                ui.add_enabled_ui(!self.busy, |ui| {
                    if self.signed_in {
                        let list = r(d::HOSTS);
                        ui.painter().rect_filled(list, 0., color(app_style::FIELD));
                        ui.scope_builder(
                            egui::UiBuilder::new()
                                .max_rect(list.shrink(2.))
                                .layout(egui::Layout::top_down(egui::Align::Min)),
                            |ui| {
                                ui.set_clip_rect(list);
                                ui.spacing_mut().item_spacing.y = 0.;
                                egui::ScrollArea::vertical()
                                    .id_salt("hosts")
                                    .show(ui, |ui| {
                                        for host in self.picker.hosts() {
                                            let (row, response) = ui.allocate_exact_size(
                                                egui::vec2(list.width() - 4., 22.),
                                                egui::Sense::click(),
                                            );
                                            if self.peer == host.peer {
                                                ui.painter().rect_filled(
                                                    row,
                                                    0.,
                                                    color(app_style::BLUE),
                                                );
                                            }
                                            ui.painter().with_clip_rect(row).text(
                                                row.left_center(),
                                                egui::Align2::LEFT_CENTER,
                                                d::host_text(&host.name, &host.peer),
                                                egui::FontId::proportional(d::BODY as f32),
                                                color(app_style::TEXT),
                                            );
                                            if response.clicked() {
                                                self.peer = host.peer.clone();
                                            }
                                        }
                                    });
                            },
                        );
                        label(
                            ui,
                            r(d::PEER_LABEL),
                            d::PEER_TEXT,
                            d::BODY as f32,
                            color(app_style::TEXT),
                        );
                        ui.put(
                            r(d::PEER),
                            egui::TextEdit::singleline(&mut self.peer).char_limit(256),
                        );
                        for i in 0..3 {
                            let response = ui
                                .scope(|ui| {
                                    if i == 2 {
                                        if self.video_context.is_none() {
                                            ui.disable();
                                        }
                                    }
                                    ui.put(r(d::ACTIONS[i]), egui::Button::new(d::ACTION_LABELS[i]))
                                })
                                .inner;
                            if response.clicked() {
                                match i {
                                    0 => self.send(Command::Refresh),
                                    1 => self.send(Command::SignOut),
                                    _ => self.start_stream(ctx),
                                }
                            }
                        }
                    } else {
                        let mut submit = false;
                        for (i, text) in app_style::LOGIN_FIELDS.iter().enumerate() {
                            label(
                                ui,
                                r(d::login_label(i)),
                                text,
                                d::BODY as f32,
                                color(app_style::TEXT),
                            );
                            let value: &mut String = match i {
                                0 => &mut self.email,
                                1 => &mut self.password,
                                _ => &mut self.tfa,
                            };
                            let response = ui.put(
                                r(d::login_edit(i)),
                                egui::TextEdit::singleline(value)
                                    .password(i == 1)
                                    .char_limit(4096),
                            );
                            submit |= response.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        }
                        submit |= ui.put(r(d::LOGIN), egui::Button::new(d::SIGN_IN)).clicked();
                        if submit {
                            let login = account::Login {
                                email: self.email.clone(),
                                password: std::mem::take(&mut self.password),
                                tfa: std::mem::take(&mut self.tfa),
                            };
                            self.send(Command::SignIn(login));
                        }
                    }
                });
                let status = self
                    .error
                    .as_deref()
                    .or(self.video_error.as_deref())
                    .unwrap_or(self.status_text);
                label(
                    ui,
                    r(d::STATUS),
                    status,
                    d::BODY as f32,
                    if self.error.is_some() {
                        Color32::LIGHT_RED
                    } else {
                        color(app_style::TEXT)
                    },
                );
            });
    }
}

pub fn rect_at(origin: egui::Pos2, [x, y, w, h]: [i32; 4]) -> egui::Rect {
    egui::Rect::from_min_size(
        origin + egui::vec2(x as f32, y as f32),
        egui::vec2(w as f32, h as f32),
    )
}
pub(crate) fn label(ui: &mut egui::Ui, rect: egui::Rect, text: &str, size: f32, color: Color32) {
    ui.put(
        rect,
        egui::Label::new(RichText::new(text).size(size).color(color)).halign(egui::Align::Min),
    );
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linux_launcher_login_and_picker_fit_the_native_window_width() {
        let ctx = egui::Context::default();
        style(&ctx);
        let logo = ctx.load_texture(
            "fixture",
            egui::ColorImage::new([1, 1], vec![Color32::WHITE]),
            egui::TextureOptions::LINEAR,
        );
        let (commands, _incoming) = std::sync::mpsc::channel();
        let (_outgoing, replies) = std::sync::mpsc::channel();
        let mut app = Desktop {
            commands,
            replies,
            email: String::new(),
            password: Default::default(),
            tfa: Default::default(),
            picker: Default::default(),
            peer: String::new(),
            transport: connection::Mode::WebRtc,
            logo,
            desktop_input: Default::default(),
            signed_in: false,
            busy: false,
            status_text: app_style::LOGIN_NOTICE,
            error: None,
            show_video: false,
            ui_state: Default::default(),
            badge: Default::default(),
            fitted_stream: None,
            launcher_size: None,
            video_context: None,
            video_error: None,
            output_format: wgpu::TextureFormat::Bgra8Unorm,
            decoder: None,
            playback: None,
        };
        for (signed_in, settings) in [
            (false, false),
            (true, false),
            (true, true),
            (true, true),
            (true, true),
            (true, true),
        ] {
            app.signed_in = signed_in;
            app.show_video = settings;
            if settings {
                app.ui_state.tab = (app.ui_state.tab + 1) % 4;
            }
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(780., 600.),
                )),
                ..Default::default()
            };
            let output = ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    app.launcher(ui, ctx);
                    assert!(
                        ui.min_rect().right() <= 780.,
                        "scroll viewport must fit the native window"
                    );
                });
            });
            fn texts(shape: &egui::epaint::Shape, out: &mut Vec<(String, f32)>) {
                match shape {
                    egui::epaint::Shape::Text(t) => {
                        for section in &t.galley.job.sections {
                            assert!(section.format.font_id.size >= 13., "unreadable UI text");
                        }
                        out.push((t.galley.job.text.clone(), t.pos.x));
                    }
                    egui::epaint::Shape::Vec(shapes) => {
                        for shape in shapes {
                            texts(shape, out)
                        }
                    }
                    _ => {}
                }
            }
            let mut rendered = Vec::new();
            for shape in &output.shapes {
                texts(&shape.shape, &mut rendered);
            }
            assert!(
                !rendered
                    .iter()
                    .any(|(text, _)| text.contains("Linux preview")
                        || text == "Your computers"
                        || text.contains("UI adapter:"))
            );
            if !settings {
                assert!(rendered.iter().any(|(text, _)| text == app_style::SUBTITLE));
            }
        }
    }
}
