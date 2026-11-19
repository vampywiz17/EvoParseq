//! Shared local shortcut policy; native events stay in adapters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    Menu,
    Statistics,
    Fullscreen,
    ReleaseInput,
    Disconnect,
    SecureAttention,
}

pub fn shortcut_binding(code: &str) -> Option<(i32, Shortcut)> {
    match code {
        "KeyM" | "KeyO" => Some((3, Shortcut::Menu)),
        "KeyS" => Some((3, Shortcut::Statistics)),
        "KeyW" => Some((3, Shortcut::Fullscreen)),
        "KeyR" => Some((3, Shortcut::ReleaseInput)),
        "KeyQ" => Some((3, Shortcut::Disconnect)),
        "End" => Some((6, Shortcut::SecureAttention)),
        _ => None,
    }
}
pub fn shortcut(code: &str, modifiers: i32) -> Option<Shortcut> {
    let (required, action) = shortcut_binding(code)?;
    (modifiers & 15 == required).then_some(action)
}

#[derive(Debug, PartialEq, Eq)]
pub enum ShortcutRoute {
    Forward,
    Consume,
    Activate(Shortcut),
}

#[derive(Default)]
pub struct Shortcuts(std::collections::BTreeSet<&'static str>);
impl Shortcuts {
    /// Consume the trigger's repeats and release even if modifiers change.
    /// A key already held remotely must never become a local shortcut on repeat.
    pub fn route(
        &mut self,
        code: &'static str,
        down: bool,
        repeated: bool,
        action: Option<Shortcut>,
    ) -> ShortcutRoute {
        if !down {
            return if self.0.remove(code) {
                ShortcutRoute::Consume
            } else {
                ShortcutRoute::Forward
            };
        }
        if self.0.contains(code) {
            return ShortcutRoute::Consume;
        }
        if !repeated {
            if let Some(action) = action {
                self.0.insert(code);
                return ShortcutRoute::Activate(action);
            }
        }
        ShortcutRoute::Forward
    }
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcuts_reserve_exact_chords_and_ignore_lock_toggles() {
        for (key, mods, action) in [
            ("KeyM", 3, Shortcut::Menu),
            ("KeyO", 3, Shortcut::Menu),
            ("KeyS", 3, Shortcut::Statistics),
            ("KeyW", 3, Shortcut::Fullscreen),
            ("KeyR", 3, Shortcut::ReleaseInput),
            ("KeyQ", 3, Shortcut::Disconnect),
            ("End", 6, Shortcut::SecureAttention),
        ] {
            assert_eq!(shortcut(key, mods), Some(action));
            assert_eq!(shortcut(key, mods | 48), Some(action));
            for extra in [4, 8] {
                if mods & extra == 0 {
                    assert_eq!(shortcut(key, mods | extra), None);
                }
            }
        }
        for (key, mods) in [
            ("Delete", 6),
            ("Tab", 4),
            ("Enter", 4),
            ("F8", 0),
            ("F11", 0),
            ("KeyE", 3),
            ("KeyR", 8),
        ] {
            assert_eq!(shortcut(key, mods), None);
        }
    }
    #[test]
    fn shortcut_repeats_and_release_stay_local_without_stealing_held_remote_keys() {
        let mut keys = Shortcuts::default();
        let full = shortcut("KeyW", 3);
        assert_eq!(keys.route("KeyW", true, true, full), ShortcutRoute::Forward);
        assert_eq!(
            keys.route("KeyW", false, false, full),
            ShortcutRoute::Forward
        );
        assert_eq!(
            keys.route("KeyW", true, false, full),
            ShortcutRoute::Activate(Shortcut::Fullscreen)
        );
        assert_eq!(keys.route("KeyW", true, true, None), ShortcutRoute::Consume);
        assert_eq!(
            keys.route("KeyW", false, false, None),
            ShortcutRoute::Consume
        );
        assert_eq!(
            keys.route("KeyW", false, false, None),
            ShortcutRoute::Forward
        );
        assert_eq!(
            keys.route("KeyW", true, false, full),
            ShortcutRoute::Activate(Shortcut::Fullscreen)
        );
        keys.clear();
        assert_eq!(
            keys.route("KeyW", true, false, None),
            ShortcutRoute::Forward
        );
    }
}
