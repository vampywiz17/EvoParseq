//! Remote cursor overlay only: no access to decoded video or readback.
use crate::cursor_pixels::Pixels;
use eframe::egui;
use std::sync::Arc;

#[derive(Default)]
pub struct Cursor {
    cached: Option<(Arc<Pixels>, egui::TextureHandle)>,
}
impl Cursor {
    pub fn paint(
        &mut self,
        ctx: &egui::Context,
        image: Option<Arc<Pixels>>,
        position: egui::Pos2,
    ) -> bool {
        let Some(image) = image else {
            self.cached = None;
            return false;
        };
        if self
            .cached
            .as_ref()
            .is_none_or(|(cached, _)| !Arc::ptr_eq(cached, &image))
        {
            let texture = ctx.load_texture(
                "phaze-host-cursor",
                egui::ColorImage::from_rgba_unmultiplied(
                    [image.size.0 as usize, image.size.1 as usize],
                    &image.rgba,
                ),
                egui::TextureOptions::LINEAR,
            );
            self.cached = Some((image, texture));
        }
        let (image, texture) = self.cached.as_ref().unwrap();
        let scale = ctx.pixels_per_point();
        let origin = position - egui::vec2(image.hotspot.0 as f32, image.hotspot.1 as f32) / scale;
        let size = egui::vec2(image.size.0 as f32, image.size.1 as f32) / scale;
        ctx.layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::new("phaze-cursor"),
        ))
        .image(
            texture.id(),
            egui::Rect::from_min_size(origin, size),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
            egui::Color32::WHITE,
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linux_cursor_uploads_only_changed_shapes_and_releases_cache() {
        let ctx = egui::Context::default();
        let mut cursor = Cursor::default();
        let image = Arc::new(Pixels {
            size: (2, 1),
            hotspot: (1, 0),
            rgba: vec![255; 8],
        });
        let raw = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400., 300.),
            )),
            ..Default::default()
        };
        let first = ctx.run(raw(), |ctx| {
            assert!(cursor.paint(ctx, Some(image.clone()), egui::pos2(20., 30.)));
        });
        let first_id = cursor.cached.as_ref().unwrap().1.id();
        assert!(
            first
                .textures_delta
                .set
                .iter()
                .any(|(id, _)| *id == first_id)
        );
        let shape = first
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Mesh(mesh) => Some(mesh),
                _ => None,
            })
            .expect("cursor image mesh");
        assert_eq!(
            shape.calc_bounds(),
            egui::Rect::from_min_size(egui::pos2(19., 30.), egui::vec2(2., 1.))
        );
        let second = ctx.run(raw(), |ctx| {
            assert!(cursor.paint(ctx, Some(image.clone()), egui::pos2(50., 60.)));
        });
        assert!(
            !second
                .textures_delta
                .set
                .iter()
                .any(|(id, _)| *id == first_id)
        );
        let changed = Arc::new(Pixels {
            size: (1, 1),
            hotspot: (0, 0),
            rgba: vec![0; 4],
        });
        let third = ctx.run(raw(), |ctx| {
            assert!(cursor.paint(ctx, Some(changed.clone()), egui::pos2(50., 60.)));
        });
        let third_id = cursor.cached.as_ref().unwrap().1.id();
        assert!(
            third
                .textures_delta
                .set
                .iter()
                .any(|(id, _)| *id == third_id)
        );
        assert!(third.textures_delta.free.contains(&first_id));
        let clear = ctx.run(raw(), |ctx| {
            assert!(!cursor.paint(ctx, None, egui::Pos2::ZERO));
        });
        assert!(clear.textures_delta.free.contains(&third_id));
        assert!(cursor.cached.is_none());
    }
}
