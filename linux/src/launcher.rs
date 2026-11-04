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
    style.spacing.item_spacing = egui::vec2(12., 14.);
    style.spacing.button_padding = egui::vec2(16., 9.);
    style.spacing.interact_size.y = 34.;
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(17.));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(17.));
    style
        .text_styles
        .insert(egui::TextStyle::Heading, egui::FontId::proportional(30.));
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
            self.desktop_input = Default::default();
            self.playback = Some(playback.clone());
            self.send(Command::Stream(peer, self.transport, playback));
        }
    }
    pub(super) fn launcher(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Frame::new().inner_margin(28.).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.image((self.logo.id(), egui::vec2(44.,44.)));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.;
                    ui.label(RichText::new(app_style::WORDMARK[0]).size(30.).strong());
                    ui.label(RichText::new(app_style::WORDMARK[1]).size(30.).strong().color(color(app_style::BLUE)));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(if self.show_video { "Back" } else { "Settings" }).clicked() { self.show_video = !self.show_video; }
                    if ui.small_button("Full screen").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!ctx.input(|i| i.viewport().fullscreen.unwrap_or(false))));
                    }
                });
            });
            ui.label(RichText::new(app_style::SUBTITLE).size(14.).color(Color32::GRAY));
            ui.add_space(14.);
            if let Some(error) = &self.error { ui.colored_label(Color32::LIGHT_RED, error); }
            if self.show_video {
                ui.heading("Settings");
                ui.label("Connection");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.transport, connection::Mode::WebRtc, "WebRTC");
                    ui.selectable_value(&mut self.transport, connection::Mode::Bud, "BUD");
                });
                ui.separator();
                ui.heading("Device capabilities");
                ui.label(format!("UI adapter: {}",self.renderer));
                ui.small("Driver-reported capabilities; playback support is validated separately.");
                egui::ScrollArea::vertical().show(ui, |ui| { for line in &self.video { ui.label(line); } });
                return;
            }
            ui.add_enabled_ui(!self.busy, |ui| {
                if self.signed_in {
                    ui.heading("Your computers");
                    let list_height = (ui.available_height() - 190.).clamp(110.,260.);
                    egui::Frame::new().fill(color(app_style::FIELD)).inner_margin(8.).corner_radius(4).show(ui, |ui| {
                        ui.set_min_width((ui.available_width()-16.).max(1.));
                        egui::ScrollArea::vertical().max_height(list_height).min_scrolled_height(list_height).show(ui, |ui| {
                            for host in self.picker.hosts() {
                                let response = ui.add_sized([ui.available_width(),42.], egui::Button::new(format!("{}   /   {}",host.name,host.peer)).selected(self.peer == host.peer));
                                if response.clicked() { self.peer = host.peer.clone(); }
                            }
                            if self.picker.hosts().is_empty() { ui.label("No computers listed. Refresh, or enter the host peer ID."); }
                        });
                    });
                    ui.horizontal(|ui| {
                        ui.label("Host peer ID");
                        ui.add(egui::TextEdit::singleline(&mut self.peer).desired_width(ui.available_width()));
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Refresh").clicked() { self.send(Command::Refresh); }
                        if ui.button("Sign out").clicked() { self.send(Command::SignOut); }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add_enabled(self.video_context.is_some(), egui::Button::new("Connect").fill(color(app_style::BLUE))).clicked() { self.start_stream(ctx); }
                            ui.small(self.transport.label());
                        });
                    });
                    if let Some(result) = &self.session_result { ui.label(result); }
                    if let Some(error) = &self.video_error { ui.colored_label(Color32::LIGHT_RED,error); }
                } else {
                    ui.heading("Sign in to Parsec");
                    ui.add_space(12.);
                    let field_width = (ui.available_width() - 230.).max(140.);
                    let mut submit = false;
                    egui::Grid::new("account-fields").spacing([24.,20.]).show(ui, |ui| {
                        for (i,label) in app_style::LOGIN_FIELDS.iter().enumerate() {
                            ui.add_sized([200.,34.],egui::Label::new(*label));
                            let text: &mut String = match i { 0 => &mut self.email, 1 => &mut self.password, _ => &mut self.tfa };
                            let response = ui.add_sized([field_width,34.],egui::TextEdit::singleline(text).password(i != 0));
                            submit |= response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            ui.end_row();
                        }
                    });
                    ui.add_space(22.);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        submit |= ui.add_sized([180.,40.],egui::Button::new("Sign in").fill(color(app_style::BLUE))).clicked();
                    });
                    if submit {
                        let login = account::Login { email:self.email.clone(), password:std::mem::take(&mut self.password), tfa:std::mem::take(&mut self.tfa) };
                        self.send(Command::SignIn(login));
                    }
                    ui.add_space(20.);
                    ui.small(app_style::LOGIN_NOTICE);
                }
            });
            if self.busy { ui.horizontal(|ui| { ui.spinner(); ui.label("Please wait..."); }); }
            ui.add_space(12.);
            ui.small(concat!("EvoParseq Linux preview ",env!("CARGO_PKG_VERSION")));
        });
        });
    }
}
