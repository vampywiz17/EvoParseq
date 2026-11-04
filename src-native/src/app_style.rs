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

/// Canonical Windows desktop geometry, in logical pixels: [x, y, width, height].
/// Platform adapters convert these rectangles; they must not invent new layouts.
pub mod desktop {
    pub const WINDOW: [i32; 2] = [744, 551];
    pub const SETTINGS_WINDOW: [i32; 2] = [984, 721];
    pub const BODY: i32 = 17;
    pub const HEADING: i32 = 32;
    pub const CANVAS: [i32; 2] = [728, 551];
    pub const LOGO: [i32; 4] = [32, 24, 40, 46];
    pub const WORDMARK: [[i32; 4]; 2] = [[84, 24, 68, 46], [152, 24, 180, 46]];
    pub const SETTINGS: [i32; 4] = [580, 28, 116, 36];
    pub const SUBTITLE: [i32; 4] = [32, 80, 650, 26];
    pub const HOSTS: [i32; 4] = [32, 130, 664, 220];
    pub const PEER_LABEL: [i32; 4] = [32, 370, 155, 26];
    pub const PEER: [i32; 4] = [200, 366, 496, 32];
    pub const LOGIN: [i32; 4] = [490, 370, 206, 40];
    pub const ACTIONS: [[i32; 4]; 3] =
        [[32, 420, 140, 40], [184, 420, 140, 40], [490, 420, 206, 40]];
    pub const ACTION_LABELS: [&str; 3] = ["Refresh", "Sign out", "Connect"];
    pub const PEER_TEXT: &str = "Host peer ID";
    pub const SIGN_IN: &str = "Sign in";
    pub const SIGNING_IN: &str = "Signing in securely...";
    pub const SIGNING_OUT: &str = "Signing out...";
    pub const SIGNED_OUT: &str = "Signed out. Enter your account details to sign in.";
    pub const REFRESHING: &str = "Refreshing computers...";
    pub const SETTINGS_TEXT: &str = "Settings";
    pub const STATUS: [i32; 4] = [32, 484, 664, 66];
    pub const EMPTY: &str = "No computers listed. Refresh, or enter the host peer ID.";
    pub const CHOOSE: &str =
        "Choose a computer, or enter its peer ID. Owner connections are supported.";
    pub const fn login_label(i: usize) -> [i32; 4] {
        [32, 136 + i as i32 * 60, 215, 26]
    }
    pub const fn login_edit(i: usize) -> [i32; 4] {
        [250, 134 + i as i32 * 60, 446, 32]
    }
    pub fn host_text(name: &str, peer: &str) -> String {
        format!("{name}   /   {peer}")
    }
}

pub mod settings {
    pub const WIDTH: i32 = 820;
    pub const BODY: i32 = 16;
    pub const SMALL: i32 = 13;
    pub const HEADING: i32 = 46;
    pub const HEADER_EXTRA: i32 = 20;
    pub const TITLE: &str = "Settings";
    pub const SUBTITLE: &str = "Customize your connection, video and audio playback.";
    pub const TITLE_RECT: [i32; 4] = [0, 24, 820, 64];
    pub const SUBTITLE_RECT: [i32; 4] = [4, 88, 816, 24];
    pub const TABS: [[i32; 4]; 4] = [
        [0, 142, 120, 40],
        [132, 142, 120, 40],
        [264, 142, 120, 40],
        [396, 142, 120, 40],
    ];
    pub const BACK: [i32; 4] = [550, 142, 110, 40];
    pub const SAVE: [i32; 4] = [676, 142, 144, 40];
    pub const BACK_TEXT: &str = "Back";
    pub const NETWORK: &str = "NETWORK";
    pub const NETWORK_RECT: [i32; 4] = [0, 204, 820, 25];
    pub const TRANSPORT_Y: i32 = 844;
    pub const TRANSPORT: &str = "Transport";
    pub const TRANSPORT_CHOICES: [&str; 2] = ["WebRTC", "BUD"];
    pub const TRANSPORT_HELP: &str = "WebRTC or BUD: direct UDP or TURN over UDP/TLS/TCP.\nUses the TURN URLs above. Reconnect after changing.";
    pub const fn label(y: i32) -> [i32; 4] {
        [0, y + 2, 392, 24]
    }
    pub const fn help(y: i32, height: i32) -> [i32; 4] {
        [0, y + 32, 392, height]
    }
    pub const fn field(y: i32, height: i32) -> [i32; 4] {
        [420, y, 400, height]
    }
    pub const fn positioned(mut rect: [i32; 4]) -> [i32; 4] {
        if rect[1] >= 88 {
            rect[1] += HEADER_EXTRA;
        }
        rect
    }
}

pub mod session {
    pub const BADGE_SIZE: i32 = 48;
    pub const BADGE_ICON: i32 = 32;
    pub const BADGE_ALPHA: u8 = 200;
    pub const BADGE_START: [i32; 2] = [16, 16];
    pub const DISCONNECT: &str = "Disconnect";
    pub const DISCONNECT_RECT: [i32; 4] = [24, 888, 178, 36];
    pub const PANEL_WIDTH: i32 = 420;
    pub fn bounded(position: [i32; 2], size: [i32; 2]) -> [i32; 2] {
        [
            position[0].clamp(0, (size[0] - BADGE_SIZE).max(0)),
            position[1].clamp(0, (size[1] - BADGE_SIZE).max(0)),
        ]
    }
}

#[path = "desktop_forms.rs"]
pub mod forms;
