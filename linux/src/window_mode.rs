//! Native window state only. Shared Windows client dimensions drive desktop UI.
use eframe::egui;
#[derive(Clone, PartialEq)]
struct Applied {
    mode: (bool, bool),
    maximized: bool,
    fullscreen: bool,
    size: Option<egui::Vec2>,
}
pub fn apply(ctx: &egui::Context, streaming: bool, settings: bool) {
    let observed = ctx.input(|i| Applied {
        mode: (streaming, settings),
        maximized: i.viewport().maximized.unwrap_or(false),
        fullscreen: i.viewport().fullscreen.unwrap_or(false),
        size: i.viewport().inner_rect.map(|r| r.size()),
    });
    let id = egui::Id::new("native-window-mode");
    let previous = ctx.data(|d| d.get_temp::<Applied>(id));
    if previous.as_ref() == Some(&observed) {
        return;
    }
    let changed = previous.as_ref().is_none_or(|p| p.mode != observed.mode);
    ctx.data_mut(|d| d.insert_temp(id, observed.clone()));
    if changed {
        ctx.send_viewport_cmd(egui::ViewportCommand::Resizable(streaming));
        ctx.send_viewport_cmd(egui::ViewportCommand::EnableButtons {
            close: true,
            minimized: true,
            maximize: streaming,
        });
    }
    if streaming {
        if changed {
            ctx.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(egui::vec2(
                f32::INFINITY,
                f32::INFINITY,
            )));
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(160., 90.)));
        }
    } else {
        let size = if settings {
            crate::app_style::desktop::SETTINGS_WINDOW
        } else {
            crate::app_style::desktop::WINDOW
        };
        let size = egui::vec2(size[0] as f32, size[1] as f32 + 24.);
        // Wayland configure is asynchronous. Reassert the canonical size when
        // unmaximizing/fullscreen exit is acknowledged, not just on mode change.
        // Only native geometry changes trigger retries: no repaint/polling loop.
        if changed
            || observed.fullscreen
            || observed.maximized
            || observed.size.is_some_and(|s| (s - size).length_sq() > 1.)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(size));
            ctx.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(size));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linux_fixed_window_recovers_after_async_fullscreen_exit() {
        let ctx = egui::Context::default();
        for (stream, settings) in [(false, false), (false, true), (true, false), (false, false)] {
            let out = ctx.run(Default::default(), |ctx| apply(ctx, stream, settings));
            assert!(
                out.viewport_output[&egui::ViewportId::ROOT]
                    .commands
                    .iter()
                    .any(|c| matches!(c,egui::ViewportCommand::Resizable(v) if *v==stream))
            );
        }
        let mut raw = egui::RawInput::default();
        let root = raw.viewports.get_mut(&egui::ViewportId::ROOT).unwrap();
        root.fullscreen = Some(false);
        root.maximized = Some(false);
        root.inner_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1920., 1080.),
        ));
        let out = ctx.run(raw.clone(), |ctx| apply(ctx, false, false));
        let commands = &out.viewport_output[&egui::ViewportId::ROOT].commands;
        assert!(commands.iter().any(|c| matches!(c,egui::ViewportCommand::InnerSize(s) if s.x==crate::app_style::desktop::WINDOW[0] as f32)));
        let out = ctx.run(raw, |ctx| apply(ctx, false, false));
        assert!(
            out.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .is_empty()
        );
    }
}
