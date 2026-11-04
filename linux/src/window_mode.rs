//! Native window state only. Shared Windows client dimensions drive desktop UI.
use eframe::egui;
pub fn apply(ctx: &egui::Context, streaming: bool, settings: bool) {
    let desired = (streaming, settings);
    let id = egui::Id::new("native-window-mode");
    if ctx.data(|d| d.get_temp::<(bool, bool)>(id)) == Some(desired) {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(id, desired));
    ctx.send_viewport_cmd(egui::ViewportCommand::Resizable(streaming));
    if streaming {
        ctx.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(egui::vec2(
            f32::INFINITY,
            f32::INFINITY,
        )));
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(160., 90.)));
    } else {
        let size = if settings {
            crate::app_style::desktop::SETTINGS_WINDOW
        } else {
            crate::app_style::desktop::WINDOW
        };
        let size = egui::vec2(size[0] as f32, size[1] as f32 + 24.);
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(size));
        ctx.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(size));
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linux_desktop_windows_are_fixed_and_stream_restores_resizing() {
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
    }
}
