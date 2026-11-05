//! Local immersive-input state shared by Win32 capture and session controls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Mode {
    pub mouse: bool,
    pub keyboard: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct State {
    pub mode: Mode,
    pub detached: bool,
    pub warp_disabled: bool,
}

pub const IMMERSIVE_LABELS: [&str; 4] = ["Disabled", "Mouse", "Keyboard", "Mouse + keyboard"];
pub fn immersive_index(mode: u8) -> usize {
    match mode {
        3 => 1,
        2 => 2,
        1 => 3,
        _ => 0,
    }
}
pub fn immersive_value(index: usize) -> u8 {
    match index {
        1 => 3,
        2 => 2,
        3 => 1,
        _ => 0,
    }
}
