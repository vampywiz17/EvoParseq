//! Local desktop services, separate from account transport and remote clipboard.

use crate::clipboard_sync::remote_text;
pub use crate::clipboard_sync::MAX_TEXT;
#[cfg(test)]
use crate::clipboard_sync::REMOTE_TEXT;

pub trait Desktop: Send + Sync {
    fn read_text(&self) -> Option<String>;
    fn write_text(&self, text: &str) -> bool;
}

#[derive(Default)]
pub struct Services {
    desktop: Option<Box<dyn Desktop>>,
}

impl Services {
    pub fn new(desktop: Box<dyn Desktop>) -> Self {
        Self {
            desktop: Some(desktop),
        }
    }

    #[cfg(test)]
    pub fn read_text(&self) -> String {
        self.try_read_text().unwrap_or_default()
    }

    pub fn try_read_text(&self) -> Option<String> {
        let text = self
            .desktop
            .as_ref()
            .and_then(|d| d.read_text())
            .filter(|s| s.len() <= MAX_TEXT && !s.contains('\0'));
        text
    }

    pub fn read_notification_text(&self) -> Option<String> {
        // Enforce the remote byte limit without splitting a UTF-8 character.
        self.try_read_text().map(remote_text)
    }

    pub fn write_text(&self, text: &str) -> bool {
        let ok = text.len() <= MAX_TEXT
            && !text.contains('\0')
            && self.desktop.as_ref().is_some_and(|d| d.write_text(text));
        ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_clipboard_limit_preserves_complete_utf8_characters() {
        assert_eq!(remote_text("á".repeat(60000)).len(), 102398);
        assert_eq!(remote_text("x".repeat(110000)).len(), REMOTE_TEXT);
        assert_eq!(remote_text(String::new()), "");
        assert_eq!(remote_text("Árvíztűrő 🦀".into()), "Árvíztűrő 🦀");
    }
    struct Fixture(std::sync::Arc<std::sync::Mutex<Vec<String>>>);
    impl Desktop for Fixture {
        fn read_text(&self) -> Option<String> {
            Some("Árvíztűrő 🦀 secret-sentinel".into())
        }
        fn write_text(&self, text: &str) -> bool {
            self.0.lock().unwrap().push(text.into());
            true
        }
    }
    #[test]
    fn clipboard_services_reject_invalid_text_before_the_os_adapter() {
        let writes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let s = Services::new(Box::new(Fixture(writes.clone())));
        assert!(s.read_text().contains("🦀"));
        assert!(s.write_text("secret-sentinel"));
        assert!(!s.write_text("bad\0text"));
        assert!(!s.write_text(&"x".repeat(MAX_TEXT + 1)));
        assert_eq!(*writes.lock().unwrap(), ["secret-sentinel"]);
    }
    #[test]
    fn offline_services_have_no_os_side_effects() {
        let s = Services::default();
        assert_eq!(s.read_text(), "");
        assert!(!s.write_text("fixture"));
    }
}
