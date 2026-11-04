//! Native desktop session presentation; no media surfaces are copied here.
use crate::{Desktop, app_style::session as s, playback::Playback};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Vec2};
use std::sync::Arc;

pub struct Badge {
    position: Pos2,
    pub open: bool,
}
impl Default for Badge {
    fn default() -> Self {
        Self {
            position: egui::pos2(s::BADGE_START[0] as f32, s::BADGE_START[1] as f32),
            open: false,
        }
    }
}
impl Desktop {
    pub(super) fn session_overlay(&mut self, ctx: &egui::Context, playback: &Arc<Playback>) {
        let size = ctx.content_rect().size();
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
            .fixed_pos(self.badge.position)
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
        }
        self.desktop_input.badge = Some(response.rect);
        self.desktop_input.controls = None;
        if self.badge.open {
            // Keep the canonical Windows panel geometry and control positions.
            // Unavailable controls are hidden; opening scrolls to Disconnect.
            let width = s::PANEL_WIDTH as f32;
            let panel_pos = egui::pos2(16., s::BADGE_START[1] as f32 + s::BADGE_SIZE as f32 + 10.);
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
                                .vertical_scroll_offset((s::DISCONNECT_RECT[1] - 24) as f32)
                                .show(ui, |ui| {
                                    let (origin, _) = ui.allocate_exact_size(
                                        egui::vec2(width, 1000.),
                                        Sense::hover(),
                                    );
                                    let rect =
                                        crate::launcher::rect_at(origin.min, s::DISCONNECT_RECT);
                                    if ui.put(rect, egui::Button::new(s::DISCONNECT)).clicked() {
                                        self.desktop_input.release(ctx, playback);
                                        playback.stop();
                                        self.badge.open = false;
                                    }
                                });
                        });
                });
            self.desktop_input.controls = Some(panel.response.rect);
        }
    }
}
