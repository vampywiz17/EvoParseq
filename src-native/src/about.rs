//! Canonical desktop information content and layout. Native shells own windows.
pub const REPOSITORY: &str = "https://github.com/vampywiz17/EvoParseq";
pub const TITLE: &str = "About EvoParseq";
pub const HEADING: &str = "EvoParseq";
pub const UPDATES_TITLE: &str = "EvoParseq - Updates";
pub const UPDATES_HEADING: &str = "Updates";
pub const CHECK_AGAIN: &str = "Check again";
pub const UPDATES_SIZE: [f32; 2] = [740., 720.];
pub const PROJECT: &str = "Project page";
pub const CLOSE: &str = "Close";
pub const SIZE: [f32; 2] = [740., 640.];
pub struct Content {
    pub version: String,
    pub status: String,
    pub notes: String,
}
pub fn content(version: &str, platform: &str, credit: &str, migration: &str) -> Content {
    Content {
        version: format!("Version {version} | {platform}\r\nNative Rust core"),
        status: "Independent client for compatible, unmodified Parsec hosts.".into(),
        notes: format!("Networking, GPU video, audio and input are handled by the native Rust client. No Parsec WebAssembly core is bundled, loaded or executed.\r\n\r\nProject: {REPOSITORY}\r\n\r\nEvoParseq is an independent community project, not affiliated with or endorsed by Parsec/Unity or lowlat.\r\n\r\nThanks to nomi-san/lowlat for inspiration and the reused open-source components. Original license and attribution notices are included with the application.\r\nhttps://github.com/nomi-san/lowlat\r\n\r\n{credit}\r\n\r\n{migration}"),
    }
}
/// Icon, heading, version, status, notes, check, download, close, in that order.
pub fn rectangles(width: i32, height: i32) -> [[i32; 4]; 8] {
    let inner = (width - 48).max(1);
    let bottom = height - 62;
    [
        [24, 25, 40, 40],
        [80, 20, inner - 56, 48],
        [24, 76, inner, 42],
        [24, 126, inner, 78],
        [24, 214, inner, (bottom - 234).max(1)],
        [24, bottom, 142, 36],
        [178, bottom, 150, 36],
        [width - 132, bottom, 108, 36],
    ]
}
