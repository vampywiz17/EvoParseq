//! Resolve a usable Wayland cursor theme once, before any threads start.
//! wp_cursor_shape remains compositor-owned when supported; older compositors
//! need actual XCursor files inside the sandbox to restore a hidden cursor.
pub fn prepare() {
    if !std::path::Path::new("/app/share/icons/Adwaita/cursors/default").is_file() {
        return;
    }
    let requested = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "default".into());
    let theme = xcursor::CursorTheme::load(&requested);
    if ["default", "text", "ew-resize"]
        .iter()
        .all(|icon| theme.load_icon(icon).is_some())
    {
        return;
    }
    // Called first in main, before eframe, Vulkan, networking or worker creation.
    unsafe {
        std::env::set_var("XCURSOR_THEME", "Adwaita");
    }
    eprintln!("Cursor theme unavailable in sandbox; using bundled GNOME Adwaita cursors");
}

#[cfg(test)]
mod tests {
    #[test]
    fn linux_flatpak_bundles_valid_restore_text_and_resize_cursors() {
        let root = std::path::Path::new("/app/share/icons/Adwaita/cursors");
        if std::env::var_os("FLATPAK_ID").is_none() && !root.exists() {
            return;
        }
        for name in [
            "default",
            "text",
            "ew-resize",
            "ns-resize",
            "pointer",
            "move",
            "wait",
        ] {
            let bytes = std::fs::read(root.join(name))
                .unwrap_or_else(|e| panic!("missing packaged cursor {name}: {e}"));
            assert!(
                xcursor::parser::parse_xcursor(&bytes).is_some(),
                "invalid packaged cursor {name}"
            );
        }
    }
}
