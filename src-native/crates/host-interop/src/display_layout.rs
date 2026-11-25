//! Shared preferences for requesting one Phaze virtual display.
use serde::{Deserialize, Serialize};

pub const MIRROR_LABEL: &str = "Mirror display";
pub const MIRROR_CHOICES: &[&str] = &["Off", "On"];
pub const MIRROR_HELP: &str =
    "Request one virtual display on the host.\nMay replace host displays. Next connection.";
pub const RESOLUTION_LABEL: &str = "Virtual display resolution";
pub const RESOLUTION_HELP: &str =
    "Requested host desktop size.\nThe host selects the actual mode. Next connection.";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayLayout {
    pub enabled: bool,
    pub resolution: Resolution,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resolution {
    Hd,
    HdPlus,
    FullHd,
    #[default]
    Wuxga,
    Qhd,
    Uhd,
}

impl Resolution {
    const PRESETS: [(Self, &str, [u32; 2]); 6] = [
        (Self::Hd, "1280 × 720", [1280, 720]),
        (Self::HdPlus, "1600 × 900", [1600, 900]),
        (Self::FullHd, "1920 × 1080", [1920, 1080]),
        (Self::Wuxga, "1920 × 1200", [1920, 1200]),
        (Self::Qhd, "2560 × 1440", [2560, 1440]),
        (Self::Uhd, "3840 × 2160", [3840, 2160]),
    ];

    pub fn labels() -> [&'static str; 6] {
        Self::PRESETS.map(|(_, label, _)| label)
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::PRESETS.get(index).map(|(mode, _, _)| *mode)
    }

    pub fn index(self) -> usize {
        Self::PRESETS
            .iter()
            .position(|(mode, _, _)| *mode == self)
            .expect("all resolutions have a preset")
    }

    pub(crate) fn dimensions(self) -> [u32; 2] {
        Self::PRESETS[self.index()].2
    }
}
