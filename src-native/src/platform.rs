//! Local desktop services, separate from account transport and remote clipboard.
use serde::Serialize;
use std::sync::Mutex;

use crate::clipboard_sync::remote_text;
pub use crate::clipboard_sync::MAX_TEXT;
#[cfg(test)]
use crate::clipboard_sync::REMOTE_TEXT;

pub trait Desktop: Send + Sync {
    fn read_text(&self) -> Option<String>;
    fn write_text(&self, text: &str) -> bool;
}

#[derive(Clone, Default, Serialize)]
pub struct Snapshot {
    pub clipboard_reads: u64,
    pub clipboard_writes: u64,
    pub unavailable_or_rejected: u64,
}

#[derive(Default)]
pub struct Services {
    desktop: Option<Box<dyn Desktop>>,
    stats: Mutex<Snapshot>,
}

impl Services {
    pub fn new(desktop: Box<dyn Desktop>) -> Self {
        Self {
            desktop: Some(desktop),
            stats: Default::default(),
        }
    }

    #[cfg(test)]
    pub fn snapshot(&self) -> Snapshot {
        self.stats.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

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

    pub fn try_read_text(&self) -> Option<String> {
        let text = self
            .desktop
            .as_ref()
            .and_then(|d| d.read_text())
            .filter(|s| s.len() <= MAX_TEXT && !s.contains('\0'));
        self.outcome(text.is_some(), |s| {
            s.clipboard_reads = s.clipboard_reads.saturating_add(1)
        });
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
    struct Fixture;
    impl Desktop for Fixture {
        fn read_text(&self) -> Option<String> {
            Some("Árvíztűrő 🦀 secret-sentinel".into())
        }
        fn write_text(&self, _: &str) -> bool {
            true
        }
    }
    #[test]
    fn clipboard_services_reject_invalid_text_and_keep_only_counters() {
        let s = Services::new(Box::new(Fixture));
        assert!(s.read_text().contains("🦀"));
        s.write_text("secret-sentinel");
        s.write_text("bad\0text");
        let stats = s.snapshot();
        assert_eq!(stats.clipboard_reads, 1);
        assert_eq!(stats.clipboard_writes, 1);
        assert_eq!(stats.unavailable_or_rejected, 1);
        assert!(!serde_json::to_string(&stats)
            .unwrap()
            .contains("secret-sentinel"));
    }
    #[test]
    fn offline_services_have_no_os_side_effects() {
        let s = Services::default();
        assert_eq!(s.read_text(), "");
        s.write_text("fixture");
        assert_eq!(s.snapshot().unavailable_or_rejected, 2);
    }
}
