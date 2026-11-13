//! Desktop widgets consume the shared Windows form descriptors. Unwired values
//! are N/A and controls disabled; no placeholder can mutate session settings.
use crate::{
    Desktop,
    app_style::{self, forms as f, settings as s},
    connection,
    launcher::{color, label, rect_at},
};
use eframe::egui::{self, RichText};
#[derive(Default)]
pub struct State {
    pub information: crate::information_linux::State,
    pub tab: usize,
    pub provider: usize,
    pub stats: bool,
    pub stats_tab: usize,
    pub network: Option<crate::network_settings::Draft>,
    pub saving:
        Option<std::sync::mpsc::Receiver<(crate::network_settings::Draft, Result<(), String>)>>,
    pub feedback: String,
    pub(super) gpu_changed: bool,
}
impl Desktop {
    pub(super) fn settings(&mut self, ui: &mut egui::Ui) {
        if let Some(receiver) = &self.ui_state.saving {
            match receiver.try_recv() {
                Ok((draft, result)) => {
                    self.ui_state.network = Some(draft);
                    self.ui_state.saving = None;
                    self.ui_state.feedback = match result {
                        Ok(()) if self.ui_state.gpu_changed => {
                            "Saved. Restart EvoParseq to change the Video GPU.".into()
                        }
                        Ok(()) => crate::network_settings::SAVED.into(),
                        Err(error) => error,
                    };
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.ui_state.saving = None;
                    self.ui_state.feedback = "Settings worker is unavailable.".into();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // Keep the canonical form visible while the platform keyring
                    // is waiting for approval; do not accept a second transaction.
                    ui.disable();
                }
            }
        }
        let draft = self
            .ui_state
            .network
            .get_or_insert_with(|| crate::network_settings::Draft::new(self.settings.view().0));
        self.ui_state.provider = usize::from(draft.values.provider == "cloudflare");
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
            self.ui_state.network = None;
            let saved = self.settings.view().0;
            self.transport = if saved.bud {
                connection::Mode::Bud
            } else {
                connection::Mode::WebRtc
            };
            return;
        }
        let save = ui
            .put(r(s::SAVE), egui::Button::new("Save settings"))
            .clicked();
        ui.painter().line_segment(
            [
                origin + egui::vec2(0., 147.),
                origin + egui::vec2(820., 147.),
            ],
            egui::Stroke::new(1_f32, color(app_style::FIELD)),
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
                    if row.label == f::VIDEO_GPU.label {
                        "GPU for stream decoding and rendering.\nRestart EvoParseq after changing the GPU."
                    } else {
                        row.help
                    },
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
            } else if row.label == f::VIDEO_FORMAT.label {
                use crate::connection_settings::VideoFormat;
                let mut value = VideoFormat::current(&draft.values);
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    egui::ComboBox::from_id_salt("video-format")
                        .width(400.)
                        .selected_text(VideoFormat::LABELS[value as usize])
                        .show_ui(ui, |ui| {
                            for (format, text) in
                                VideoFormat::ALL.into_iter().zip(VideoFormat::LABELS)
                            {
                                let supported = format == VideoFormat::H264
                                    || self.video_context.as_ref().is_some_and(|c| {
                                        if format == VideoFormat::Hevc10 {
                                            c.hevc10
                                        } else {
                                            c.hevc
                                        }
                                    });
                                ui.add_enabled_ui(supported, |ui| {
                                    ui.selectable_value(&mut value, format, text);
                                });
                            }
                        });
                });
                value.apply(&mut draft.values);
            } else if row.label == f::DISPLAY_TIMING.label {
                use crate::connection_settings::PresentationMode;
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    egui::ComboBox::from_id_salt(row.label)
                        .width(400.)
                        .selected_text(draft.values.presentation_mode.label())
                        .show_ui(ui, |ui| {
                            for mode in PresentationMode::ALL {
                                ui.scope(|ui| {
                                    ui.selectable_value(
                                        &mut draft.values.presentation_mode,
                                        mode,
                                        mode.label(),
                                    );
                                });
                            }
                        });
                });
            } else if row.label == f::VIDEO_GPU.label {
                if let Some(index) = draft
                    .values
                    .video_gpu
                    .as_ref()
                    .and_then(|p| p.match_index(&self.video_gpus))
                {
                    draft.values.video_gpu = Some(self.video_gpus[index].clone());
                }
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    egui::ComboBox::from_id_salt(row.label)
                        .width(400.)
                        .selected_text(
                            draft
                                .values
                                .video_gpu
                                .as_ref()
                                .map_or("Automatic", |gpu| gpu.name.as_str()),
                        )
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut draft.values.video_gpu, None, "Automatic");
                            if let Some(saved) = draft.values.video_gpu.as_ref() {
                                if saved.match_index(&self.video_gpus).is_none() {
                                    ui.label(format!("Unavailable: {}", saved.name));
                                }
                            }
                            for gpu in &self.video_gpus {
                                ui.selectable_value(
                                    &mut draft.values.video_gpu,
                                    Some(gpu.clone()),
                                    &gpu.name,
                                );
                            }
                        });
                });
            } else if row.label == f::VIDEO_PRESENTATION.label || row.label == f::CHROMA.label {
                let value = if row.label == f::CHROMA.label {
                    crate::connection_settings::BinaryChoice::Chroma.labels()[0]
                } else {
                    crate::connection_settings::BinaryChoice::Rendering.labels()[0]
                };
                ui.add_enabled_ui(false, |ui| {
                    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                        egui::ComboBox::from_id_salt(row.label)
                            .width(400.)
                            .selected_text(value)
                            .show_ui(ui, |_| {});
                    });
                });
            } else if row.label == f::INPUT_MODE.label {
                use crate::connection_settings::InputMode;
                let mut mode = InputMode::current(&draft.values);
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    egui::ComboBox::from_id_salt(row.label)
                        .width(400.)
                        .selected_text(InputMode::LABELS[mode as usize])
                        .show_ui(ui, |ui| {
                            for (item, text) in InputMode::ALL.into_iter().zip(InputMode::LABELS) {
                                ui.add_enabled_ui(
                                    item == InputMode::Standard
                                        || self.transport == connection::Mode::Bud,
                                    |ui| {
                                        ui.selectable_value(&mut mode, item, text);
                                    },
                                );
                            }
                        });
                });
                mode.apply(&mut draft.values);
            } else if tab == 2 {
                use crate::connection_settings::BinaryChoice;
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    if row.label == f::MINIMUM_BUFFER_MS.label
                        || row.label == f::MAXIMUM_BUFFER_MS.label
                    {
                        ui.add_enabled_ui(draft.values.custom_audio_buffer, |ui| {
                            let value = if row.label == f::MINIMUM_BUFFER_MS.label {
                                &mut draft.audio_min
                            } else {
                                &mut draft.audio_max
                            };
                            ui.put(rect, egui::TextEdit::singleline(value).char_limit(6));
                        });
                    } else {
                        let (choice, value, enabled) = if row.label == f::PLAYBACK_BUFFER.label {
                            (
                                BinaryChoice::AudioBuffer,
                                &mut draft.values.custom_audio_buffer,
                                true,
                            )
                        } else if row.label == f::AUDIO_FORMAT.label {
                            (
                                BinaryChoice::AudioCodec,
                                &mut draft.values.allow_raw_audio,
                                self.transport == connection::Mode::Bud,
                            )
                        } else {
                            (
                                BinaryChoice::Speakers,
                                &mut draft.values.experimental_surround_audio,
                                self.transport == connection::Mode::Bud,
                            )
                        };
                        let labels = choice.labels();
                        ui.add_enabled_ui(enabled, |ui| {
                            egui::ComboBox::from_id_salt(row.label)
                                .width(400.)
                                .selected_text(labels[usize::from(*value)])
                                .show_ui(ui, |ui| {
                                    for (index, text) in labels.into_iter().enumerate() {
                                        ui.selectable_value(value, index == 1, text);
                                    }
                                });
                        });
                    }
                });
            } else if row.label == f::SESSION_MENU.label {
                ui.put(
                    rect,
                    egui::Checkbox::new(
                        &mut draft.values.native_core.hide_session_button,
                        row.control_text,
                    ),
                );
            } else if row.label == f::TCP_TLS_QOS.label {
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    let labels = crate::connection_settings::BinaryChoice::TcpQos.labels();
                    egui::ComboBox::from_id_salt(row.label)
                        .width(400.)
                        .selected_text(labels[usize::from(draft.values.experimental_tcp_qos)])
                        .show_ui(ui, |ui| {
                            for (index, text) in labels.into_iter().enumerate() {
                                ui.selectable_value(
                                    &mut draft.values.experimental_tcp_qos,
                                    index == 1,
                                    text,
                                );
                            }
                        });
                });
            } else if tab == 0 {
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    use crate::connection_settings::RouteMode;
                    if row.label == f::STUN_SERVERS.label {
                        ui.put(
                            rect,
                            egui::TextEdit::multiline(&mut draft.stun_urls).char_limit(32768),
                        );
                    } else if row.label == f::CONNECTION_ROUTE.label {
                        let mut route = RouteMode::current(&draft.values);
                        egui::ComboBox::from_id_salt(row.label)
                            .width(400.)
                            .selected_text(RouteMode::LABELS[route as usize])
                            .show_ui(ui, |ui| {
                                for (mode, text) in
                                    RouteMode::ALL.into_iter().zip(RouteMode::LABELS)
                                {
                                    ui.selectable_value(&mut route, mode, text);
                                }
                            });
                        route.apply(&mut draft.values);
                    } else if row.label == f::EVENT_LOG.label {
                        ui.put(
                            rect,
                            egui::Checkbox::new(&mut draft.values.event_logging, row.control_text),
                        );
                    } else {
                        ui.add_enabled_ui(!draft.values.stun_only, |ui| {
                            if row.label == f::TURN_PROVIDER.label {
                                egui::ComboBox::from_id_salt(row.label)
                                    .width(400.)
                                    .selected_text(if draft.values.provider == "cloudflare" {
                                        "Cloudflare TURN"
                                    } else {
                                        "Custom servers"
                                    })
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut draft.values.provider,
                                            "custom".into(),
                                            "Custom servers",
                                        );
                                        ui.selectable_value(
                                            &mut draft.values.provider,
                                            "cloudflare".into(),
                                            "Cloudflare TURN",
                                        );
                                    });
                            } else if row.label == f::CREDENTIAL_CACHE.label {
                                ui.put(
                                    rect,
                                    egui::Checkbox::new(
                                        &mut draft.values.cache_credentials,
                                        row.control_text,
                                    ),
                                );
                            } else {
                                let (value, secret, multiline): (&mut String, bool, bool) =
                                    if row.label == f::TURN_SERVERS.label {
                                        (&mut draft.turn_urls, false, true)
                                    } else if row.label == f::USERNAME.label {
                                        (&mut draft.values.custom_username, false, false)
                                    } else if row.label == f::PASSWORD.label {
                                        (&mut draft.password, true, false)
                                    } else if row.label == f::TURN_KEY_ID.label {
                                        (&mut draft.values.turn_key_id, false, false)
                                    } else if row.label == f::API_TOKEN.label {
                                        (&mut draft.token, true, false)
                                    } else {
                                        (&mut draft.ttl, false, false)
                                    };
                                let edit = if multiline {
                                    egui::TextEdit::multiline(value)
                                } else {
                                    egui::TextEdit::singleline(value).password(secret)
                                };
                                ui.put(rect, edit.char_limit(32768));
                            }
                        });
                    }
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
            ui.add_enabled_ui(!draft.values.stun_only, |ui| {
                let (value, text) = if self.ui_state.provider == 0 {
                    (&mut draft.forget_password, "Forget saved password")
                } else {
                    (&mut draft.forget_token, "Forget saved API token")
                };
                ui.put(r([420, y, 400, 28]), egui::Checkbox::new(value, text));
            });
        }
        label(
            ui,
            r([0, footer, 820, 64]),
            if self.ui_state.tab == 0 {
                crate::network_settings::NOTICE
            } else {
                ""
            },
            13.,
            color(app_style::SETTINGS_MUTED),
        );
        label(
            ui,
            r([0, footer + 78, 820, 40]),
            &self.ui_state.feedback,
            16.,
            color(app_style::SETTINGS_TEXT),
        );
        if save {
            let mut draft = self
                .ui_state
                .network
                .take()
                .expect("network form initialized");
            draft.values.bud = self.transport == connection::Mode::Bud;
            self.ui_state.gpu_changed = draft.values.video_gpu != self.settings.view().0.video_gpu;
            // Retain public/encrypted form values for disabled painting only.
            // Plaintext replacement secrets move exclusively to the save worker.
            let mut pending = crate::network_settings::Draft::new(draft.values.clone());
            pending.stun_urls = draft.stun_urls.clone();
            pending.turn_urls = draft.turn_urls.clone();
            pending.ttl = draft.ttl.clone();
            pending.audio_min = draft.audio_min.clone();
            pending.audio_max = draft.audio_max.clone();
            self.ui_state.network = Some(pending);
            self.ui_state.feedback = crate::network_settings::SAVING.into();
            let settings = self.settings.clone();
            let context = ui.ctx().clone();
            let (sender, receiver) = std::sync::mpsc::channel();
            // One short-lived storage operation, never a media/render worker.
            // Secret portal requests can block for desktop approval.
            match std::thread::Builder::new()
                .name("settings-save".into())
                .spawn(move || {
                    let result = draft.save(&settings).map_err(|error| error.to_string());
                    let _ = sender.send((draft, result));
                    context.request_repaint();
                }) {
                Ok(_) => self.ui_state.saving = Some(receiver),
                Err(_) => self.ui_state.feedback = "Could not start settings worker.".into(),
            }
        }
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
                            self.badge.restore();
                            if let Err(error) = self
                                .settings
                                .update_native(|p| p.hide_session_button = false)
                            {
                                self.error = Some(error.to_string());
                            }
                        }
                        if ui.button(f::MENU_STATS).clicked() {
                            if let Some(p) = &self.playback {
                                self.desktop_input.release(ctx, p);
                            }
                            self.ui_state.stats = true;
                        }
                    }
                    ui.menu_button(f::MENU_HELP, |ui| {
                        for (label, mode) in [
                            (f::MENU_ABOUT, crate::information_linux::Mode::About),
                            (f::MENU_UPDATES, crate::information_linux::Mode::Updates),
                        ] {
                            if ui.button(label).clicked() {
                                if let Some(playback) = &self.playback {
                                    self.desktop_input.release(ctx, playback);
                                }
                                self.ui_state.information.open(ctx, mode);
                                ui.close();
                            }
                        }
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
                .with_resizable(false)
                .with_maximize_button(false)
                .with_inner_size([744., 781.])
                .with_min_inner_size([744., 781.])
                .with_max_inner_size([744., 781.]),
            |child, class| {
                if child.input(|i| i.viewport().maximized.unwrap_or(false)) {
                    child.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
                }
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
        ui.label(
            RichText::new(
                self.metrics
                    .presentation(self.ui_state.stats_tab)
                    .map_or("Waiting for telemetry", |p| p.status.as_str()),
            )
            .color(color(app_style::SETTINGS_ACCENT)),
        );
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
        let Some(presentation) = self.metrics.presentation(self.ui_state.stats_tab) else {
            return;
        };
        egui::ScrollArea::vertical().show(ui, |ui| {
            // Canonical desktop Overview: exactly two cards per row at every width.
            for pair in presentation.cards.chunks(2) {
                ui.columns(2, |columns| {
                    for (column, (label, value)) in columns.iter_mut().zip(pair) {
                        egui::Frame::new()
                            .fill(color(app_style::SETTINGS_FIELD))
                            .inner_margin(16.)
                            .show(column, |ui| {
                                ui.set_min_width((ui.available_width() - 1.).max(0.));
                                ui.label(*label);
                                ui.label(RichText::new(value).size(32.));
                            });
                    }
                });
            }
            for (text, value) in &presentation.rows {
                if value.is_empty() {
                    ui.add_space(16.);
                    ui.label(RichText::new(text).color(color(app_style::SETTINGS_ACCENT)));
                } else {
                    ui.horizontal_top(|ui| {
                        ui.add_sized(
                            [210., 36.],
                            egui::Label::new(
                                RichText::new(text).color(color(app_style::SETTINGS_MUTED)),
                            )
                            .wrap(),
                        );
                        ui.add(egui::Label::new(value).wrap());
                    });
                }
            }
            ui.add_space(20.);
            ui.label(
                RichText::new(&presentation.footer)
                    .size(13.)
                    .color(color(app_style::SETTINGS_MUTED)),
            );
        });
    }
}
