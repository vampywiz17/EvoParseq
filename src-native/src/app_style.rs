//! Platform-independent launcher palette and copy. OS widgets remain adapters.
pub const BACKGROUND: [u8; 3] = [25, 29, 35];
pub const FIELD: [u8; 3] = [44, 50, 58];
pub const TEXT: [u8; 3] = [240, 243, 245];
pub const BLUE: [u8; 3] = [47, 128, 237];
pub const WORDMARK: [&str; 2] = ["EVO", "PARSEQ"];
pub const SUBTITLE: &str = "NATIVE CORE  /  Your computers, your connection";
pub const LOGIN_FIELDS: [&str; 3] = ["Email", "Password", "Two-factor code (optional)"];
pub const LOGIN_NOTICE: &str =
    "Use your Parsec account. Passwords and two-factor codes are never saved.";

pub const SETTINGS_FIELD: [u8; 3] = [33, 33, 33];
pub const SETTINGS_TEXT: [u8; 3] = [238, 238, 238];
pub const SETTINGS_MUTED: [u8; 3] = [188, 188, 188];
pub const SETTINGS_ACCENT: [u8; 3] = [0, 187, 255];
pub const SETTINGS_TABS: [&str; 4] = ["Network", "Video", "Audio", "Input"];
