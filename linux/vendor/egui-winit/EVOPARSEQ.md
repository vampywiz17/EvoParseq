# Linux cursor cache patch

Upstream egui-winit 0.33.3, egui commit
44cdd653e2317d300fb8a6c9c36b03f23991e803 (MIT OR Apache-2.0).
The vendored Cargo.toml and src are the registry source, except for one change:
State::on_window_event invalidates current_cursor_icon on CursorEntered and
CursorLeft. The next ordinary hover update reapplies the window cursor instead
of assuming the cursor used on another Wayland surface is still current.
No private framework API, custom Wayland protocol or extra dependency is used.
Only linux/Cargo.toml selects this patch. Check upstream when updating egui;
remove this patch once equivalent cursor invalidation is available upstream.
