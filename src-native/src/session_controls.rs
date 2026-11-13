//! Shared desktop session commands and status text; no OS or UI dependencies.
use crate::session_preferences::Video;
#[derive(Clone, Debug)]
pub enum Command {
    Apply(Video),
    SelectDisplay(String),
    RefreshDisplays,
    #[cfg(windows)]
    AddDisplay(usize),
    Disconnect,
    HideButton,
    SecureAttention,
    #[cfg(target_os = "linux")]
    Clipboard {
        text: String,
        paste: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Applied,
    Failed,
    Confirmed,
    Waiting,
    Unconfirmed,
    AttentionSent,
    AttentionFailed,
}

impl Status {
    pub fn text(self) -> &'static str {
        match self {
            Self::Applied | Self::Confirmed => "",
            Self::Failed => "Could not apply or save settings. See the event log.",
            Self::Waiting => "Applying settings...",
            Self::Unconfirmed => "Host did not confirm the requested settings. See the event log.",
            Self::AttentionSent => "Ctrl+Alt+Del requested from the host.",
            Self::AttentionFailed => "Could not send Ctrl+Alt+Del. See the event log.",
        }
    }
}
