//! Desktop widgets consume the shared Windows form descriptors. Unwired values
//! are N/A and controls disabled; no placeholder can mutate session settings.
use crate::{
    Desktop,
    app_style::{self, forms as f, settings as s},
    connection,
    launcher::{color, label, rect_at},
};
use eframe::egui::{self, Color32, RichText};
#[derive(Default)]
pub struct State {
    pub tab: usize,
    pub provider: usize,
    pub stats: bool,
    pub stats_tab: usize,
}
impl Desktop {
    pub(super) fn settings(&mut self, ui: &mut egui::Ui) {
        ui.style_mut().override_font_id = Some(egui::FontId::proportional(s::BODY as f32));
        ui.visuals_mut().override_text_color = Some(color(app_style::SETTINGS_TEXT));
        ui.visuals_mut().extreme_bg_color = color(app_style::SETTINGS_FIELD);
        let origin = ui.cursor().min
            + egui::vec2(((ui.available_width() - s::WIDTH as f32) / 2.).max(24.), 0.);
        let r = |rect| rect_at(origin, s::positioned(rect));
        let footer = [1150, 1034, 750, 570][self.ui_state.tab];
        ui.allocate_space(egui::vec2(868., (footer + 138 + s::HEADER_EXTRA) as f32));
        label(
            ui,
            r(s::TITLE_RECT),
            s::TITLE,
            46.,
            color(app_style::SETTINGS_TEXT),
        );
        label(
            ui,
            r(s::SUBTITLE_RECT),
            s::SUBTITLE,
            16.,
            color(app_style::SETTINGS_TEXT),
        );
        for i in 0..4 {
            let mut text = RichText::new(app_style::SETTINGS_TABS[i]);
            if i == self.ui_state.tab {
                text = text.color(color(app_style::SETTINGS_FIELD));
            }
            let mut button = egui::Button::new(text);
            if i == self.ui_state.tab {
                button = button.fill(color(app_style::SETTINGS_ACCENT));
            }
            if ui.put(r(s::TABS[i]), button).clicked() {
                self.ui_state.tab = i;
            }
        }
        if ui
            .put(r(s::BACK), egui::Button::new(s::BACK_TEXT))
            .clicked()
        {
            self.show_video = false;
            self.fitted_stream = None;
        }
        if ui
            .put(r(s::SAVE), egui::Button::new("Save settings"))
            .clicked()
        {
            let bud = self.transport == connection::Mode::Bud;
            match self.settings.update_local(|settings| {
                settings.bud = bud;
                Ok(())
            }) {
                Ok(()) => {
                    self.error = None;
                    self.show_video = false;
                    self.fitted_stream = None;
                }
                Err(error) => self.error = Some(error.to_string()),
            }
        }
        ui.painter().line_segment(
            [
                origin + egui::vec2(0., 147.),
                origin + egui::vec2(820., 147.),
            ],
            egui::Stroke::new(1., color(app_style::FIELD)),
        );
        label(
            ui,
            r(s::NETWORK_RECT),
            ["NETWORK", "VIDEO", "AUDIO", "INPUT"][self.ui_state.tab],
            16.,
            color(app_style::SETTINGS_ACCENT),
        );
        let tab = self.ui_state.tab;
        let group = self.ui_state.provider + 1;
        for row in f::SETTINGS
            .iter()
            .filter(|row| row.tab == tab && (row.group == 0 || usize::from(row.group) == group))
        {
            label(
                ui,
                r(s::label(row.y)),
                row.label,
                16.,
                color(app_style::SETTINGS_TEXT),
            );
            if !row.help.is_empty() {
                label(
                    ui,
                    r(s::help(row.y, row.help_height)),
                    row.help,
                    13.,
                    color(app_style::SETTINGS_MUTED),
                );
            }
            let rect = r(s::field(row.y, row.field_height));
            if row.label == f::TRANSPORT.label {
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    egui::ComboBox::from_id_salt("transport")
                        .width(400.)
                        .selected_text(self.transport.label())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.transport,
                                connection::Mode::WebRtc,
                                s::TRANSPORT_CHOICES[0],
                            );
                            ui.selectable_value(
                                &mut self.transport,
                                connection::Mode::Bud,
                                s::TRANSPORT_CHOICES[1],
                            );
                        });
                });
            } else if row.label == f::TURN_PROVIDER.label {
                // Navigation between the two existing Windows form groups, not a saved
                // provider value. Saving persists only the integrated transport choice.
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    egui::ComboBox::from_id_salt("provider-page")
                        .width(400.)
                        .selected_text(
                            ["Custom servers", "Cloudflare TURN"][self.ui_state.provider],
                        )
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.ui_state.provider, 0, "Custom servers");
                            ui.selectable_value(&mut self.ui_state.provider, 1, "Cloudflare TURN");
                        });
                });
            } else {
                ui.scope(|ui| {
                    ui.disable();
                    match row.kind {
                        f::Kind::Edit => {
                            let mut value = "N/A".to_owned();
                            ui.put(rect, egui::TextEdit::multiline(&mut value));
                        }
                        f::Kind::Choice => {
                            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                                egui::ComboBox::from_id_salt(row.label)
                                    .width(400.)
                                    .selected_text("N/A")
                                    .show_ui(ui, |_| {});
                            });
                        }
                        f::Kind::Check => {
                            ui.put(rect, egui::Checkbox::new(&mut false, row.control_text));
                        }
                        f::Kind::Info => {}
                    }
                });
            }
        }
        if self.ui_state.tab == 0 {
            let y = if self.ui_state.provider == 0 {
                734
            } else {
                628
            };
            ui.scope(|ui| {
                ui.disable();
                ui.put(
                    r([420, y, 400, 28]),
                    egui::Checkbox::new(
                        &mut false,
                        if self.ui_state.provider == 0 {
                            "Forget saved password"
                        } else {
                            "Forget saved API token"
                        },
                    ),
                );
            });
        }
        label(
            ui,
            r([0, footer, 820, 64]),
            "N/A",
            13.,
            color(app_style::SETTINGS_MUTED),
        );
        label(
            ui,
            r([0, footer + 78, 820, 40]),
            "N/A",
            16.,
            color(app_style::SETTINGS_TEXT),
        );
    }
    pub(super) fn menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("native-menu")
            .frame(egui::Frame::new().fill(color(app_style::BACKGROUND)))
            .exact_height(24.)
            .show(ctx, |ui| {
                ui.style_mut().override_font_id = Some(egui::FontId::proportional(16.));
                ui.spacing_mut().button_padding = egui::vec2(6., 2.);
                egui::MenuBar::new().ui(ui, |ui| {
                    // Windows launcher has Help; the stream window also has Settings/Stats.
                    if self.playback.is_some() {
                        if ui.button(f::MENU_SETTINGS).clicked() {
                            if let Some(p) = &self.playback {
                                self.desktop_input.release(ctx, p);
                            }
                            self.show_video = true;
                        }
                        if ui.button(f::MENU_STATS).clicked() {
                            if let Some(p) = &self.playback {
                                self.desktop_input.release(ctx, p);
                            }
                            self.ui_state.stats = true;
                        }
                    }
                    ui.menu_button(f::MENU_HELP, |ui| {
                        ui.add_enabled(false, egui::Button::new(f::MENU_ABOUT));
                        ui.add_enabled(false, egui::Button::new(f::MENU_UPDATES));
                    });
                });
            });
    }
    pub(super) fn statistics(&mut self, ctx: &egui::Context) {
        if !self.ui_state.stats {
            return;
        }
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("connection-stats-native"),
            egui::ViewportBuilder::default()
                .with_title(f::MENU_STATS)
                .with_decorations(true)
                .with_resizable(true)
                .with_inner_size([744., 781.])
                .with_min_inner_size([480., 360.]),
            |child, class| {
                if child.input(|i| i.viewport().close_requested()) {
                    self.ui_state.stats = false;
                    ctx.request_repaint();
                    return;
                }
                if class == egui::ViewportClass::Embedded {
                    // Only non-native/headless integrations embed viewports.
                    egui::Window::new(f::MENU_STATS).show(child, |ui| self.statistics_contents(ui));
                } else {
                    egui::CentralPanel::default()
                        .frame(
                            egui::Frame::new()
                                .fill(color(app_style::BACKGROUND))
                                .inner_margin(24.),
                        )
                        .show(child, |ui| self.statistics_contents(ui));
                }
            },
        );
    }
    fn statistics_contents(&mut self, ui: &mut egui::Ui) {
        ui.style_mut().override_font_id = Some(egui::FontId::proportional(16.));
        ui.label(RichText::new(f::MENU_STATS).size(32.));
        ui.label(RichText::new("N/A").color(color(app_style::SETTINGS_ACCENT)));
        ui.horizontal(|ui| {
            for (i, tab) in f::STATS_TABS.iter().enumerate() {
                if ui
                    .add_sized(
                        [124., 34.],
                        egui::Button::new(*tab).selected(i == self.ui_state.stats_tab),
                    )
                    .clicked()
                {
                    self.ui_state.stats_tab = i;
                }
            }
        });
        egui::ScrollArea::vertical().show(ui, |ui| {
            if self.ui_state.stats_tab == 0 {
                ui.horizontal_wrapped(|ui| {
                    for text in [
                        "Connection RTT",
                        "Decoded video",
                        "Incoming traffic",
                        "Outgoing traffic",
                    ] {
                        egui::Frame::new()
                            .fill(color(app_style::SETTINGS_FIELD))
                            .inner_margin(16.)
                            .show(ui, |ui| {
                                ui.set_min_width(290.);
                                ui.label(text);
                                ui.label(RichText::new("N/A").size(32.));
                            });
                    }
                });
            }
            for row in f::STATS
                .iter()
                .filter(|row| row.tab == self.ui_state.stats_tab)
            {
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), if row.section { 48. } else { 44. }),
                    egui::Sense::hover(),
                );
                label(
                    ui,
                    egui::Rect::from_min_size(
                        rect.min + egui::vec2(0., 8.),
                        egui::vec2(if row.section { rect.width() } else { 210. }, 36.),
                    ),
                    row.label,
                    16.,
                    color(if row.section {
                        app_style::SETTINGS_ACCENT
                    } else {
                        app_style::SETTINGS_MUTED
                    }),
                );
                if !row.section {
                    label(
                        ui,
                        egui::Rect::from_min_size(
                            rect.min + egui::vec2(220., 8.),
                            egui::vec2(rect.width() - 220., 36.),
                        ),
                        "N/A",
                        16.,
                        Color32::WHITE,
                    );
                }
            }
            for text in f::additional_stats(self.ui_state.stats_tab) {
                ui.horizontal(|ui| {
                    ui.add_sized([210., 44.], egui::Label::new(text));
                    ui.label("N/A");
                });
            }
        });
    }
}
