# Linux cursor cache patch

Upstream egui-winit 0.33.3, egui commit
44cdd653e2317d300fb8a6c9c36b03f23991e803 (MIT OR Apache-2.0).
The vendored Cargo.toml and src are the registry source, with these narrow native integration changes:
State::on_window_event invalidates current_cursor_icon on CursorEntered and
CursorLeft. The next ordinary hover update reapplies the window cursor instead
of assuming the cursor used on another Wayland surface is still current.
No private framework API, custom Wayland protocol or extra dependency is used.
Only linux/Cargo.toml selects this patch. Check upstream when updating egui;
remove this patch once equivalent cursor invalidation is available upstream.

RequestPaste also requests repaint after enqueuing the deferred eframe action.
This delivers its resulting Paste input without depending on a new video frame
or unrelated pointer event. It uses the existing clipboard backend/event queue;
no clipboard monitor, polling loop, new worker or media render pass is added.
