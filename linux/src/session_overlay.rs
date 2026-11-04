//! Native desktop session presentation; no media surfaces are copied here.
use crate::{
    Desktop,
    app_style::{forms as f, session as s},
    launcher::{color, label, rect_at},
    playback::Playback,
};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Vec2};
use std::sync::Arc;

pub struct Badge {
    position: Pos2,
    pub open: bool,
    hidden: bool,
    draft: Option<crate::session_preferences::Video>,
}
impl Default for Badge {
    fn default() -> Self {
        Self {
            position: egui::pos2(s::BADGE_START[0] as f32, s::BADGE_START[1] as f32),
            open: false,
            hidden: false,
            draft: None,
        }
    }
}
impl Badge {
    pub fn set_hidden(&mut self, hidden: bool) {
        self.hidden = hidden;
    }
    pub fn restore(&mut self) {
        self.hidden = false;
        self.open = true;
        self.draft = None;
    }
}
impl Desktop {
    pub(super) fn session_overlay(&mut self, ctx: &egui::Context, playback: &Arc<Playback>) {
        if self.badge.hidden {
            self.desktop_input.badge = None;
            self.desktop_input.controls = None;
            return;
        }
        // Windows' native menu is outside its client area. The Linux menu is
        // drawn in the client, so keep the badge/panel below that strip too.
        let origin = ctx.content_rect().min + egui::vec2(0., 24.);
        let size = ctx.content_rect().size() - egui::vec2(0., 24.);
        let bounded = s::bounded(
            [
                self.badge.position.x.round() as i32,
                self.badge.position.y.round() as i32,
            ],
            [size.x as i32, size.y as i32],
        );
        self.badge.position = egui::pos2(bounded[0] as f32, bounded[1] as f32);
        let area = egui::Area::new(egui::Id::new("session-badge"))
            .order(egui::Order::Foreground)
            .fixed_pos(origin + self.badge.position.to_vec2())
            .movable(false)
            .show(ctx, |ui| {
                let (rect, response) = ui.allocate_exact_size(
                    Vec2::splat(s::BADGE_SIZE as f32),
                    Sense::click_and_drag(),
                );
                let alpha = s::BADGE_ALPHA;
                let field = crate::app_style::FIELD;
                ui.painter().rect_filled(
                    rect,
                    0.,
                    Color32::from_rgba_unmultiplied(field[0], field[1], field[2], alpha),
                );
                ui.painter().rect_stroke(
                    rect,
                    0.,
                    egui::Stroke::new(1., Color32::from_rgba_unmultiplied(99, 189, 231, alpha)),
                    egui::StrokeKind::Inside,
                );
                ui.painter().image(
                    self.logo.id(),
                    Rect::from_center_size(rect.center(), Vec2::splat(s::BADGE_ICON as f32)),
                    Rect::from_min_max(Pos2::ZERO, egui::pos2(1., 1.)),
                    Color32::from_white_alpha(alpha),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Session")
                });
                response
            });
        let response = area.inner;
        if response.drag_started() {
            self.desktop_input.release(ctx, playback);
        }
        if response.dragged() {
            self.badge.position += response.drag_delta();
        }
        if response.clicked() {
            self.desktop_input.release(ctx, playback);
            self.badge.open = !self.badge.open;
            self.badge.draft = None;
        }
        self.desktop_input.badge = Some(response.rect);
        self.desktop_input.controls = None;
        if self.badge.open {
            let initial = self.metrics.draft.clone();
            self.badge.draft.get_or_insert(initial);
            // Keep the canonical Windows panel geometry and control positions.
            // Unavailable controls are hidden; opening scrolls to Disconnect.
            let width = s::PANEL_WIDTH as f32;
            let panel_pos =
                origin + egui::vec2(16., s::BADGE_START[1] as f32 + s::BADGE_SIZE as f32 + 10.);
            let panel = egui::Area::new(egui::Id::new("session-panel"))
                .order(egui::Order::Foreground)
                .fixed_pos(panel_pos)
                .movable(false)
                .show(ctx, |ui| {
                    let height = (size.y - s::BADGE_SIZE as f32 - 10. - 32.).clamp(120., 1000.);
                    egui::Frame::new()
                        .fill(Color32::from_rgba_unmultiplied(25, 29, 35, 238))
                        .show(ui, |ui| {
                            ui.set_min_size(egui::vec2(width, height));
                            egui::ScrollArea::vertical()
                                .max_height(height)
                                .min_scrolled_height(height)
                                .show(ui, |ui| {
                                    let (origin, _) = ui.allocate_exact_size(
                                        egui::vec2(width, 1000.),
                                        Sense::hover(),
                                    );
                                    ui.style_mut().override_font_id =
                                        Some(egui::FontId::proportional(16.));
                                    let r = |rect| rect_at(origin.min, rect);
                                    let fg = color(crate::app_style::TEXT);
                                    label(ui, r([24, 18, 350, 36]), "Session", 25., fg);
                                    label(
                                        ui,
                                        r([24, 66, 370, 82]),
                                        if self.metrics.compact.is_empty() {
                                            f::SESSION_METRICS
                                        } else {
                                            &self.metrics.compact
                                        },
                                        12.,
                                        fg,
                                    );
                                    let mut selected=self.metrics.current_output.clone().unwrap_or_default();
                                    let available=!self.metrics.pending&&!self.metrics.displays.is_empty();
                                    ui.add_enabled_ui(available,|ui|{
                                        ui.scope_builder(egui::UiBuilder::new().max_rect(r([24,164,280,28])),|ui|{
                                            egui::ComboBox::from_id_salt("host-display").width(260.).selected_text(if self.metrics.display.is_empty(){"Waiting for host"}else{&self.metrics.display}).show_ui(ui,|ui|{
                                                for output in &self.metrics.displays{ui.selectable_value(&mut selected,output.id.clone(),&output.name);}
                                            });
                                        });
                                    });
                                    if available&&self.metrics.current_output.as_deref()!=Some(selected.as_str())&&!selected.is_empty(){playback.push_session(crate::session_controls::Command::SelectDisplay(selected));}
                                    if ui.put(r([314,164,80,28]),egui::Button::new("Refresh")).clicked(){playback.push_session(crate::session_controls::Command::RefreshDisplays);}
                                    label(
                                        ui,
                                        r([24, 199, 370, 44]),
                                        &self.metrics.display_hint,
                                        16.,
                                        fg,
                                    );
                                    for (i,text) in f::SESSION_LABELS.iter().enumerate(){
                                        label(ui,r([24,254+i as i32*44,150,24]),text,16.,fg);
                                        let draft=self.badge.draft.as_mut().unwrap();
                                        let mut value=match i {0=>crate::session_preferences::resolution_text(draft.resolution),1=>draft.bandwidth_mbps.unwrap_or(10).to_string(),2=>draft.fps.unwrap_or(60).to_string(),3=>if draft.constant_fps.unwrap_or(false){"Fixed"}else{"Dynamic"}.into(),4=>if draft.rotated.unwrap_or(false){"Rotated"}else{"Normal"}.into(),_=>"Off".into()};
                                        let options:Vec<&str>=match i {0=>crate::session_preferences::RESOLUTION_CHOICES.to_vec(),1=>crate::session_preferences::BANDWIDTH_CHOICES.to_vec(),2=>crate::session_preferences::FPS_CHOICES.to_vec(),3=>vec!["Dynamic","Fixed"],4=>vec!["Normal","Rotated"],_=>vec!["Off"]};
                                        ui.add_enabled_ui(!self.metrics.pending&&i!=5&&i!=4,|ui|{
                                            ui.scope_builder(egui::UiBuilder::new().max_rect(r([174,250+i as i32*44,218,28])),|ui|{
                                                egui::ComboBox::from_id_salt(("session-field",i)).width(208.).selected_text(&value).show_ui(ui,|ui|{for option in options{ui.selectable_value(&mut value,option.to_owned(),option);}});
                                            });
                                        });
                                        match i {0=>if let Ok(parsed)=crate::session_preferences::resolution(&value){draft.resolution=parsed;},1=>draft.bandwidth_mbps=value.parse().ok(),2=>draft.fps=value.parse().ok(),3=>draft.constant_fps=Some(value=="Fixed"),4=>draft.rotated=Some(value=="Rotated"),_=>{}}
                                    }
                                    let mut fullscreen =
                                        ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
                                    if ui
                                        .put(
                                            r([24, 516, 350, 30]),
                                            egui::Checkbox::new(&mut fullscreen, "Fullscreen"),
                                        )
                                        .changed()
                                    {
                                        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(
                                            fullscreen,
                                        ));
                                    }
                                    let mut muted=self.metrics.audio.muted.unwrap_or(false);
                                    let audio_commands=self.metrics.audio.commands.clone();
                                    let response=ui.add_enabled_ui(audio_commands.is_some()&&self.metrics.audio.muted.is_some(),|ui|ui.put(r([24,554,160,30]),egui::Checkbox::new(&mut muted,"Mute audio"))).inner;
                                    if response.changed(){if let Some(commands)=&audio_commands{let _=commands.send(crate::audio_pipewire::Command::Mute(muted));}}
                                    let volume=self.metrics.audio.volume.map_or_else(||"Volume: not reported by PipeWire".into(),|v|format!("Volume: {:.0}%",v*100.));
                                    label(ui,r([194,558,198,26]),&volume,16.,fg);
                                    let mut level=self.metrics.audio.volume.unwrap_or(0.)*100.;
                                    let slider=ui.add_enabled_ui(audio_commands.is_some()&&self.metrics.audio.volume.is_some(),|ui|ui.put(r([24,590,368,32]),egui::Slider::new(&mut level,0. ..=100.).show_value(false))).inner;
                                    if slider.changed(){if let Some(commands)=&audio_commands{let _=commands.send(crate::audio_pipewire::Command::Volume(level.round()as u8));}}
                                    label(
                                        ui,
                                        r([24, 634, 370, 146]),
                                        &self.metrics.status,
                                        16.,
                                        fg,
                                    );
                                    let state=if let Some(error)=self.metrics.audio.error.as_deref(){error}else if !self.metrics.result.is_empty(){self.metrics.result.as_str()}else{self.metrics.sample.as_ref().map_or("Waiting for telemetry",|s|s.state.as_str())};
                                    label(ui, r([24, 790, 370, 40]), state, 16., fg);
                                    for (i, (text, rect)) in f::SESSION_ACTIONS.iter().enumerate() {
                                        let response = ui
                                            .scope(|ui| {
                                                if (i==1&&self.metrics.pending)||(i==5&&!self.metrics.windows) {
                                                    ui.disable();
                                                }
                                                ui.put(r(*rect), egui::Button::new(*text))
                                            })
                                            .inner;
                                        if response.clicked() {
                                            match i {
                                                0 => {
                                                    self.desktop_input.release(ctx, playback);
                                                    self.show_video = true;
                                                    self.badge.open = false;
                                                }
                                                1 => {
                                                    if let Some(video)=self.badge.draft.clone(){
                                                        if let Some(telemetry)=&self.telemetry {let size=ctx.input(|i|i.viewport().inner_rect.map(|r|r.size())).unwrap_or_default();let scale=ctx.pixels_per_point();*telemetry.client_size.lock().unwrap_or_else(|e|e.into_inner())=((size.x*scale).round().max(1.)as u32,((size.y-24.).max(1.)*scale).round()as u32);}
                                                        playback.push_session(crate::session_controls::Command::Apply(video));self.metrics.pending=true;
                                                    }
                                                }
                                                4 => {
                                                    self.badge.hidden=true;self.badge.open=false;
                                                    if let Err(error)=self.settings.update_native(|p|p.hide_session_button=true){self.error=Some(error.to_string());}
                                                }
                                                5 =>playback.push_session(crate::session_controls::Command::SecureAttention),
                                                2 => {
                                                    self.desktop_input.release(ctx, playback);
                                                    playback.stop();
                                                    self.badge.open = false;
                                                }
                                                3 => self.badge.open = false,
                                                _ => {}
                                            }
                                        }
                                    }
                                });
                        });
                });
            self.desktop_input.controls = Some(panel.response.rect);
        }
    }
}
