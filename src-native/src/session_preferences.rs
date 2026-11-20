//! Shared video requests, presets and validation. No window, codec, storage or OS APIs.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub const MAX_BANDWIDTH_MBPS: u32 = 100;
/// Newer Parsec hosts accept 100 and 150 Mbps; Phaze remains limited to 100.
pub const MAX_PARSEC_BANDWIDTH_MBPS: u32 = 150;
pub const MAX_FPS: u32 = 240;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resolution {
    #[default]
    Host,
    Client,
    Fixed(u32, u32),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Video {
    pub resolution: Resolution,
    /// None preserves the current host setting; Some is a request, not telemetry.
    pub bandwidth_mbps: Option<u32>,
    /// Parsec-only high bitrate request; never forwarded to Phaze.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parsec_bandwidth_mbps: Option<u32>,
    pub fps: Option<u32>,
    pub constant_fps: Option<bool>,
    pub rotated: Option<bool>,
    pub immersive: u8,
    pub fullscreen: bool,
}
impl Default for Video {
    fn default() -> Self {
        Self {
            resolution: Resolution::Host,
            bandwidth_mbps: None,
            parsec_bandwidth_mbps: None,
            fps: None,
            constant_fps: None,
            rotated: None,
            immersive: 0,
            fullscreen: false,
        }
    }
}
impl Video {
    /// Parsec requests may use a higher limit without changing Phaze behavior.
    pub fn requested_bandwidth(&self, parsec: bool) -> Option<u32> {
        if parsec {
            self.parsec_bandwidth_mbps.or(self.bandwidth_mbps)
        } else {
            self.bandwidth_mbps
        }
    }

    pub fn set_bandwidth(&mut self, parsec: bool, value: u32) -> Result<()> {
        let max = if parsec {
            MAX_PARSEC_BANDWIDTH_MBPS
        } else {
            MAX_BANDWIDTH_MBPS
        };
        ensure!((1..=max).contains(&value), "Value must be 1–{max}.");
        if parsec {
            // Do not overwrite a previously saved Phaze/legacy bandwidth.
            self.parsec_bandwidth_mbps = Some(value);
        } else {
            // Likewise, adjusting Phaze does not erase a Parsec preference.
            self.bandwidth_mbps = Some(value);
        }
        Ok(())
    }

    pub fn has_host_settings(&self) -> bool {
        self.resolution != Resolution::Host
            || self.bandwidth_mbps.is_some()
            || self.parsec_bandwidth_mbps.is_some()
            || self.fps.is_some()
            || self.constant_fps.is_some()
            || self.rotated.is_some()
    }

    pub fn validate(&self) -> Result<()> {
        if let Resolution::Fixed(w, h) = self.resolution {
            ensure!(
                (320..=8192).contains(&w) && (200..=8192).contains(&h),
                "Resolution is outside supported limits"
            );
        }
        ensure!(
            self.bandwidth_mbps
                .is_none_or(|n| (1..=MAX_BANDWIDTH_MBPS).contains(&n)),
            "Bandwidth must be 1–{MAX_BANDWIDTH_MBPS} Mbps"
        );
        ensure!(
            self.parsec_bandwidth_mbps
                .is_none_or(|n| (1..=MAX_PARSEC_BANDWIDTH_MBPS).contains(&n)),
            "Parsec bandwidth must be 1–{MAX_PARSEC_BANDWIDTH_MBPS} Mbps"
        );
        ensure!(
            self.fps.is_none_or(|n| (1..=MAX_FPS).contains(&n)),
            "FPS must be 1–{MAX_FPS}"
        );
        ensure!(self.immersive <= 3, "Invalid immersive mode");
        Ok(())
    }
}

impl Video {
    pub fn request(&self, client_size: (u32, u32)) -> Result<bytes::Bytes> {
        self.validate()?;
        let mut values = serde_json::Map::new();
        let (w, h) = match self.resolution {
            Resolution::Host => (0, 0),
            Resolution::Client => client_size,
            Resolution::Fixed(w, h) => (w, h),
        };
        ensure!(
            w <= 16384 && h <= 16384,
            "Client display dimensions exceed limit"
        );
        values.insert("resolutionX".into(), w.into());
        values.insert("resolutionY".into(), h.into());
        if let Some(n) = self.requested_bandwidth(true) {
            values.insert("encoderMaxBitrate".into(), n.into());
        }
        if let Some(n) = self.fps {
            values.insert("encoderFPS".into(), n.into());
        }
        if let Some(n) = self.constant_fps {
            values.insert("fullFPS".into(), n.into());
        }
        if let Some(n) = self.rotated {
            values.insert("rotated".into(), n.into());
        }
        // Same owner-video contract used by the existing core and lowlat daemon.
        // No codec flags here: the BUD adapter preserves its negotiated capability.
        crate::control::text(17, 11, &serde_json::json!({"video":[values]}).to_string())
    }
}

pub const HOST_RESOLUTION: &str = "Keep host";
pub const CLIENT_RESOLUTION: &str = "Match client display";
// Stock Windows UI presets, not a host hardware capability declaration.
// Current stock output/settings replies do not contain supported-mode lists.
pub const RESOLUTION_CHOICES: &[&str] = &[
    HOST_RESOLUTION,
    CLIENT_RESOLUTION,
    "3840 x 2160",
    "3840 x 1600",
    "3440 x 1440",
    "2560 x 1600",
    "2560 x 1440",
    "2560 x 1080",
    "1920 x 1200",
    "1920 x 1080",
    "1680 x 1050",
    "1600 x 1200",
    "1366 x 768",
    "1280 x 1024",
    "1280 x 800",
    "1280 x 720",
    "1024 x 768",
];
pub const BANDWIDTH_CHOICES: &[&str] = &[
    "3", "5", "7", "10", "15", "20", "25", "30", "35", "40", "45", "50",
];
pub const PARSEC_BANDWIDTH_CHOICES: &[&str] = &[
    "3", "5", "7", "10", "15", "20", "25", "30", "35", "40", "45", "50", "100", "150",
];
#[cfg(any(target_os = "linux", target_os = "android", test))]
pub fn bandwidth_choices(parsec: bool) -> &'static [&'static str] {
    if parsec {
        PARSEC_BANDWIDTH_CHOICES
    } else {
        BANDWIDTH_CHOICES
    }
}
pub const FPS_CHOICES: &[&str] = &["30", "60", "100", "120", "144", "240"];
pub fn current_choice(value: &str) -> String {
    format!("Keep current: {value}")
}
pub fn resolution_text(resolution: Resolution) -> String {
    match resolution {
        Resolution::Host => HOST_RESOLUTION.into(),
        Resolution::Client => CLIENT_RESOLUTION.into(),
        Resolution::Fixed(w, h) => format!("{w} x {h}"),
    }
}
/// Native views read their widgets; every view uses the same parsing and
/// validation before producing an owned video-settings command.
pub struct PanelFields<'a> {
    pub resolution: &'a str,
    pub bandwidth: &'a str,
    pub fps: &'a str,
    pub constant_fps: bool,
    pub rotated: bool,
    pub immersive: u8,
    pub fullscreen: bool,
}
impl PanelFields<'_> {
    #[cfg(test)]
    pub fn parse(&self) -> Result<Video> {
        self.parse_for_service(false)
    }

    #[cfg(any(target_os = "android", test))]
    pub fn parse_for_service(&self, parsec: bool) -> Result<Video> {
        self.parse_preserving(parsec, None)
    }

    /// GUI adapters supply the saved profile so changing one service's
    /// bandwidth never clears the other service's preference.
    pub fn parse_preserving(&self, parsec: bool, saved: Option<&Video>) -> Result<Video> {
        let mut video = Video {
            resolution: resolution(self.resolution)?,
            bandwidth_mbps: saved.and_then(|v| v.bandwidth_mbps),
            parsec_bandwidth_mbps: saved.and_then(|v| v.parsec_bandwidth_mbps),
            fps: number(self.fps, MAX_FPS)?,
            constant_fps: Some(self.constant_fps),
            rotated: Some(self.rotated),
            immersive: self.immersive,
            fullscreen: self.fullscreen,
        };
        let max = if parsec {
            MAX_PARSEC_BANDWIDTH_MBPS
        } else {
            MAX_BANDWIDTH_MBPS
        };
        video.set_bandwidth(
            parsec,
            number(self.bandwidth, max)?.expect("panel bandwidth required"),
        )?;
        video.validate()?;
        Ok(video)
    }
}
pub(crate) fn number(text: &str, max: u32) -> Result<Option<u32>> {
    let n: u32 = text
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("Enter a number."))?;
    ensure!((1..=max).contains(&n), "Value must be 1–{max}.");
    Ok(Some(n))
}
pub(crate) fn resolution(text: &str) -> Result<Resolution> {
    match text.trim() {
        HOST_RESOLUTION => Ok(Resolution::Host),
        CLIENT_RESOLUTION => Ok(Resolution::Client),
        s => {
            let (w, h) = s
                .split_once(['x', '×'])
                .ok_or_else(|| anyhow::anyhow!("Enter resolution as width x height."))?;
            Ok(Resolution::Fixed(w.trim().parse()?, h.trim().parse()?))
        }
    }
}

/// Host preferences, not the actual decoded stream or capture confirmation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostVideo {
    pub bandwidth_mbps: Option<u32>,
    pub fps: Option<u32>,
    pub constant_fps: Option<bool>,
    pub rotated: Option<bool>,
}
impl HostVideo {
    pub fn from_report(report: &serde_json::Value) -> Self {
        let number = |key: &str, max| {
            report[key]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| (1..=max).contains(n))
        };
        Self {
            bandwidth_mbps: number("bandwidth_mbps", MAX_BANDWIDTH_MBPS),
            fps: number("fps", MAX_FPS),
            constant_fps: report["constant_fps"].as_bool(),
            rotated: report["rotated"].as_bool(),
        }
    }

    pub fn panel_video(self, mut saved: Video) -> Video {
        saved.bandwidth_mbps = Some(saved.bandwidth_mbps.or(self.bandwidth_mbps).unwrap_or(10));
        saved.fps = Some(saved.fps.or(self.fps).unwrap_or(60));
        saved.constant_fps = Some(saved.constant_fps.or(self.constant_fps).unwrap_or(false));
        saved.rotated = Some(saved.rotated.or(self.rotated).unwrap_or(false));
        saved
    }
}

#[cfg(any(target_os = "android", test))]
/// A configured host rate is a Surface hint, not measured FPS. Dynamic/unknown
/// input must not be advertised as a fixed-cadence source to the compositor.
pub fn frame_rate_hint(host: &serde_json::Value) -> f64 {
    if host["constant_fps"] != true {
        return 0.;
    }
    host["fps"]
        .as_u64()
        .filter(|n| (1..=1000).contains(n))
        .map(|n| n as f64)
        .unwrap_or(0.)
}

#[cfg(test)]
mod parsec_bandwidth_tests {
    use super::*;

    #[test]
    fn parsec_high_bitrate_request_is_not_forwarded_to_phaze() -> Result<()> {
        assert_eq!(
            &PARSEC_BANDWIDTH_CHOICES[BANDWIDTH_CHOICES.len()..],
            &["100", "150"]
        );
        for mbps in [100, 150] {
            let selection = mbps.to_string();
            let fields = PanelFields {
                resolution: HOST_RESOLUTION,
                bandwidth: &selection,
                fps: "60",
                constant_fps: false,
                rotated: false,
                immersive: 0,
                fullscreen: false,
            };
            let video = fields.parse_for_service(true)?;
            assert_eq!(video.requested_bandwidth(true), Some(mbps));
            if mbps == 150 {
                assert!(fields.parse().is_err());
                assert_eq!(video.requested_bandwidth(false), None);
                assert!(video.has_host_settings());
            }
            let encoded = video.request((1920, 1080))?;
            let payload: serde_json::Value =
                serde_json::from_slice(&encoded[13..encoded.len() - 1])?;
            assert_eq!(payload["video"][0]["encoderMaxBitrate"], mbps);
            let restored: Video = serde_json::from_str(&serde_json::to_string(&video)?)?;
            assert_eq!(restored.requested_bandwidth(true), Some(mbps));
        }
        assert!(!bandwidth_choices(false).contains(&"150"));
        assert!(bandwidth_choices(true).contains(&"150"));
        let mut video = Video::default();
        video.set_bandwidth(false, 50)?;
        assert_eq!(video.requested_bandwidth(true), Some(50));
        assert_eq!(video.requested_bandwidth(false), Some(50));
        assert!(video.set_bandwidth(false, 150).is_err());

        // This is the persisted state transition that must remain harmless
        // when a user connects to Parsec after a Phaze session.
        let saved = video.clone();
        let fields = PanelFields {
            resolution: HOST_RESOLUTION,
            bandwidth: "150",
            fps: "60",
            constant_fps: false,
            rotated: false,
            immersive: 0,
            fullscreen: false,
        };
        let parsec = fields.parse_preserving(true, Some(&saved))?;
        assert_eq!(parsec.requested_bandwidth(false), Some(50));
        assert_eq!(parsec.requested_bandwidth(true), Some(150));
        let restored: Video = serde_json::from_str(&serde_json::to_string(&parsec)?)?;
        assert_eq!(restored.requested_bandwidth(false), Some(50));
        assert_eq!(restored.requested_bandwidth(true), Some(150));
        let phaze_fields = PanelFields {
            bandwidth: "40",
            ..fields
        };
        let changed_phaze = phaze_fields.parse_preserving(false, Some(&restored))?;
        assert_eq!(changed_phaze.requested_bandwidth(false), Some(40));
        assert_eq!(changed_phaze.requested_bandwidth(true), Some(150));
        Ok(())
    }
}

#[cfg(test)]
mod frame_rate_tests {
    #[test]
    fn surface_rate_uses_host_configuration_not_requested_or_measured_fps() {
        assert_eq!(
            super::frame_rate_hint(&serde_json::json!({"fps":60,"constant_fps":true})),
            60.
        );
        assert_eq!(
            super::frame_rate_hint(&serde_json::json!({"fps":120,"constant_fps":true})),
            120.
        );
        for host in [
            serde_json::json!({"fps":60,"constant_fps":false}),
            serde_json::json!({"fps":60}),
            serde_json::json!({"constant_fps":true}),
            serde_json::json!({"fps":0,"constant_fps":true}),
            serde_json::json!({"fps":-1,"constant_fps":true}),
            serde_json::json!({"fps":1001,"constant_fps":true}),
            serde_json::Value::Null,
        ] {
            assert_eq!(super::frame_rate_hint(&host), 0.);
        }
    }
}
