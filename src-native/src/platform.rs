//! Local desktop services, separate from account transport and remote clipboard.

#[cfg(test)]
use {serde::Serialize, std::sync::Mutex};

use crate::clipboard_sync::remote_text;
pub use crate::clipboard_sync::MAX_TEXT;
#[cfg(test)]
use crate::clipboard_sync::REMOTE_TEXT;

pub trait Desktop: Send + Sync {
    fn read_text(&self) -> Option<String>;
    fn write_text(&self, text: &str) -> bool;
}

#[cfg(test)]
#[derive(Clone, Default, Serialize)]
pub struct Snapshot {
    pub clipboard_reads: u64,
    pub clipboard_writes: u64,
    pub unavailable_or_rejected: u64,
}

#[derive(Default)]
pub struct Services {
    desktop: Option<Box<dyn Desktop>>,
    #[cfg(test)]
    stats: Mutex<Snapshot>,
}

impl Services {
    pub fn new(desktop: Box<dyn Desktop>) -> Self {
        Self {
            desktop: Some(desktop),
            #[cfg(test)]
            stats: Default::default(),
        }
    }

    #[cfg(test)]
    pub fn snapshot(&self) -> Snapshot {
        self.stats.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    #[cfg(test)]
    fn outcome(&self, success: bool, update: impl FnOnce(&mut Snapshot)) {
        let mut stats = self.stats.lock().unwrap_or_else(|e| e.into_inner());
        if success {
            update(&mut stats);
        } else {
            stats.unavailable_or_rejected = stats.unavailable_or_rejected.saturating_add(1);
        }
    }

    #[cfg(test)]
    pub fn read_text(&self) -> String {
        self.try_read_text().unwrap_or_default()
    }

    // The binding is consumed by test-only permission counters.
    #[cfg_attr(not(test), allow(clippy::let_and_return))]
    pub fn try_read_text(&self) -> Option<String> {
        let text = self
            .desktop
            .as_ref()
            .and_then(|d| d.read_text())
            .filter(|s| s.len() <= MAX_TEXT && !s.contains('\0'));
        #[cfg(test)]
        self.outcome(text.is_some(), |s| {
            s.clipboard_reads = s.clipboard_reads.saturating_add(1)
        });
        text
    }

    pub fn read_notification_text(&self) -> Option<String> {
        // Enforce the remote byte limit without splitting a UTF-8 character.
        self.try_read_text().map(remote_text)
    }

    #[cfg_attr(not(test), allow(clippy::let_and_return))]
    pub fn write_text(&self, text: &str) -> bool {
        let ok = text.len() <= MAX_TEXT
            && !text.contains('\0')
            && self.desktop.as_ref().is_some_and(|d| d.write_text(text));
        #[cfg(test)]
        self.outcome(ok, |s| {
            s.clipboard_writes = s.clipboard_writes.saturating_add(1)
        });
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
