//! Android request policy in Rust; output capability discovery stays in Android.
#![cfg_attr(not(target_os = "android"), allow(dead_code))]
use crate::audio_layout::Layout;
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub custom: bool,
    pub minimum_ms: u32,
    pub maximum_ms: u32,
    pub pcm: bool,
    pub surround: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            custom: false,
            minimum_ms: 75,
            maximum_ms: 150,
            pcm: false,
            surround: false,
        }
    }
}
impl Settings {
    pub fn validate(self) -> Result<()> {
        if self.custom {
            crate::audio_stream::validate_buffer_limits(self.minimum_ms, self.maximum_ms)?;
        }
        Ok(())
    }
    pub fn minimum_frames(self) -> usize {
        if self.custom {
            self.minimum_ms as usize * 48
        } else {
            0
        }
    }
    pub fn maximum(self) -> u32 {
        if self.custom {
            self.maximum_ms
        } else {
            100
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Device {
    pub id: i32,
    pub masks: Vec<u32>,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inventory {
    pub devices: Vec<Device>,
}
impl Inventory {
    pub fn parse(text: &str) -> Result<Self> {
        ensure!(text.len() <= 32768, "Audio capability report exceeds limit");
        let value: Self = serde_json::from_str(text)?;
        ensure!(
            value.devices.len() <= 64
                && value
                    .devices
                    .iter()
                    .all(|d| d.id > 0 && d.masks.len() <= 64),
            "Invalid audio capability report"
        );
        Ok(value)
    }
    pub fn layout(&self, device_id: i32) -> Layout {
        let masks = self
            .devices
            .iter()
            .find(|d| d.id == device_id)
            .map(|d| d.masks.as_slice())
            .unwrap_or_default();
        // Public AudioFormat Java masks differ from AAudio/native masks.
        for (java, channels, native) in [(0x18fc, 8, 0x63f), (0x183c, 6, 0x60f), (0xfc, 6, 0x3f)] {
            if masks.contains(&java) {
                return Layout::new(channels, native).unwrap();
            }
        }
        Layout::STEREO
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_defaults_and_custom_bounds_match_shared_policy() {
        let defaults: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(defaults.maximum(), 100);
        assert_eq!(defaults.minimum_frames(), 0);
        let mut value = Settings {
            custom: true,
            minimum_ms: 0,
            maximum_ms: 500,
            ..Default::default()
        };
        assert!(value.validate().is_ok());
        value.minimum_ms = 501;
        assert!(value.validate().is_err());
        value.minimum_ms = 0;
        value.maximum_ms = 0;
        assert!(value.validate().is_err());
    }
    #[test]
    fn selected_endpoint_requires_explicit_speaker_positions() {
        let value =
            Inventory::parse(r#"{"devices":[{"id":1,"masks":[6396]},{"id":2,"masks":[252]}]}"#)
                .unwrap();
        assert_eq!(value.layout(1), Layout::new(8, 0x63f).unwrap());
        assert_eq!(value.layout(2), Layout::new(6, 0x3f).unwrap());
        assert_eq!(value.layout(3), Layout::STEREO);
        assert!(Inventory::parse(r#"{"devices":[{"id":0,"masks":[]}]}"#).is_err());
    }
}
