//! Shared preferences for requesting Phaze virtual displays.
use serde::{Deserialize, Serialize};

pub const MIRROR_LABEL: &str = "Mirror display";
pub const MIRROR_CHOICES: &[&str] = &["Off", "On"];
pub const MIRROR_HELP: &str =
    "Request virtual displays on the host.\nMay replace host displays. Next connection.";
pub const DISPLAY_COUNT_LABEL: &str = "Virtual display count";
pub const DISPLAY_COUNT_HELP: &str =
    "Number of displays to request at startup.\nThe host decides which displays to create.";
pub const RESOLUTION_LABEL: &str = "Virtual display resolution";
pub const RESOLUTION_HELP: &str =
    "Requested host desktop size.\nThe host selects the actual mode. Next connection.";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayLayout {
    pub enabled: bool,
    pub resolution: Resolution,
    pub count: DisplayCount,
    pub additional_resolutions: [Resolution; 3],
}

impl DisplayLayout {
    /// TV always requests at most its single configured display.
    pub fn single_display(self) -> Self {
        Self {
            count: DisplayCount::One,
            ..self
        }
    }

    pub fn requests(self, index: usize) -> bool {
        self.enabled && index < self.count.total()
    }

    pub fn resolutions(self) -> [Resolution; 4] {
        let [second, third, fourth] = self.additional_resolutions;
        [self.resolution, second, third, fourth]
    }

    pub fn set_resolution(&mut self, index: usize, resolution: Resolution) {
        if let Some(additional) = index.checked_sub(1) {
            if let Some(slot) = self.additional_resolutions.get_mut(additional) {
                *slot = resolution;
            }
        } else {
            self.resolution = resolution;
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DisplayCount {
    #[default]
    One,
    Two,
    Three,
    Four,
}

impl DisplayCount {
    pub const ALL: [Self; 4] = [Self::One, Self::Two, Self::Three, Self::Four];
    pub const LABELS: [&'static str; 4] = ["1", "2", "3", "4"];

    pub fn index(self) -> usize {
        self as usize
    }

    pub(crate) fn total(self) -> usize {
        self.index() + 1
    }
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
