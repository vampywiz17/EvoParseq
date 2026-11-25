//! Android TV request presets. Rust owns validation; the TV view only renders
//! these choices. Decoder support and actual host replies are separate checks.
use crate::session_preferences::{
    bandwidth_choices, bandwidth_menu, PanelFields, FPS_CHOICES, LEGACY_BANDWIDTH_CHOICES,
    PARSEC_BANDWIDTH_CHOICES, PHAZE_BANDWIDTH_CHOICES, RESOLUTION_CHOICES,
};
use anyhow::{ensure, Result};
use evoparseq_host_interop::display_layout::{
    DisplayLayout, Resolution, MIRROR_CHOICES, MIRROR_HELP, MIRROR_LABEL, RESOLUTION_LABEL,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    #[serde(default)]
    pub audio: crate::audio_preferences::Settings,
    #[serde(default)]
    pub logging: bool,
    pub transport: usize,
    pub video: usize,
    pub resolution: usize,
    #[serde(default)]
    pub phaze_display_layout: DisplayLayout,
    pub fps: usize,
    pub bandwidth: usize,
    /// Parsec-only selection; legacy bandwidth is retained for Phaze.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parsec_bandwidth: Option<usize>,
    #[serde(default)]
    pub pacing: usize,
}

const BUFFER_MS: &[u32] = &[
    0, 5, 10, 15, 20, 25, 30, 40, 50, 60, 75, 100, 125, 150, 200, 250, 300, 400, 500,
];
const TRANSPORT: &[&str] = &["WebRTC", "lowlat"];
const PACING: &[&str] = &["Dynamic FPS", "Fixed FPS"];
const VIDEO: &[&str] = &["H.264 · 8-bit SDR", "HEVC · 8-bit SDR", "HEVC · 10-bit SDR"];
type ChoiceField = (&'static str, &'static str, &'static [&'static str], usize);

impl Default for Preferences {
    fn default() -> Self {
        Self {
            audio: Default::default(),
            logging: false,
            transport: 0,
            pacing: 0,
            video: 0,
            resolution: 0,
            phaze_display_layout: DisplayLayout::default(),
            fps: FPS_CHOICES.iter().position(|s| *s == "60").unwrap(),
            bandwidth: LEGACY_BANDWIDTH_CHOICES
                .iter()
                .position(|s| *s == "20")
                .unwrap(),
            parsec_bandwidth: None,
        }
    }
}

impl Preferences {
    fn fields(&self) -> [ChoiceField; 6] {
        [
            ("transport", "Transport", TRANSPORT, self.transport),
            ("video", "Video request", VIDEO, self.video),
            (
                "resolution",
                "Resolution request",
                RESOLUTION_CHOICES,
                self.resolution,
            ),
            ("fps", "Frame rate request (FPS)", FPS_CHOICES, self.fps),
            ("pacing", "Frame pacing", PACING, self.pacing),
            (
                "bandwidth",
                "Bandwidth limit request (Mbps)",
                PARSEC_BANDWIDTH_CHOICES,
                self.parsec_bandwidth.unwrap_or(self.bandwidth),
            ),
        ]
    }

    pub fn parse(text: &str) -> Result<Self> {
        ensure!(text.len() <= 4096, "Settings exceed limit.");
        let value: Self = serde_json::from_str(text)?;
        for (_, _, choices, index) in value.fields() {
            ensure!(index < choices.len(), "Invalid TV setting.");
        }
        ensure!(
            value.bandwidth < LEGACY_BANDWIDTH_CHOICES.len(),
            "Invalid Phaze bandwidth."
        );
        // Reuse the Windows panel's parsing/limits rather than duplicating them.
        value.host_request()?;
        value.audio.validate()?;
        ensure!(
            BUFFER_MS.contains(&value.audio.minimum_ms)
                && BUFFER_MS.contains(&value.audio.maximum_ms),
            "Unsupported audio buffer preset"
        );
        Ok(value)
    }

    #[cfg(test)]
    pub fn select(&mut self, field: &str, index: usize) -> Result<()> {
        self.select_for_service(field, index, true)
    }

    pub fn select_for_service(&mut self, field: &str, index: usize, parsec: bool) -> Result<()> {
        let mut audio = self.audio;
        let boolean = || {
            ensure!(index < 2, "Invalid audio setting");
            Ok(index == 1)
        };
        match field {
            "mirror_display" => {
                ensure!(!parsec, "Mirror display requires Phaze.");
                ensure!(index < MIRROR_CHOICES.len(), "Invalid display setting.");
                self.phaze_display_layout.enabled = index == 1;
                return Ok(());
            }
            "virtual_display_resolution" => {
                ensure!(
                    !parsec && self.phaze_display_layout.enabled,
                    "Enable Mirror display to choose a virtual display resolution."
                );
                self.phaze_display_layout.resolution = Resolution::from_index(index)
                    .ok_or_else(|| anyhow::anyhow!("Invalid resolution."))?;
                return Ok(());
            }
            "audio_custom" => audio.custom = boolean()?,
            "audio_pcm" => audio.pcm = boolean()?,
            "audio_surround" => audio.surround = boolean()?,
            "audio_minimum" => {
                audio.minimum_ms = *BUFFER_MS
                    .get(index)
                    .ok_or_else(|| anyhow::anyhow!("Invalid audio buffer"))?
            }
            "audio_maximum" => {
                audio.maximum_ms = *BUFFER_MS
                    .get(index)
                    .ok_or_else(|| anyhow::anyhow!("Invalid audio buffer"))?
            }
            "logging" => {
                self.logging = boolean()?;
                return Ok(());
            }
            _ => return self.select_video(field, index, parsec),
        }
        audio.validate()?;
        self.audio = audio;
        Ok(())
    }

    fn select_video(&mut self, field: &str, index: usize, parsec: bool) -> Result<()> {
        let choices = if field == "bandwidth" && !parsec {
            PHAZE_BANDWIDTH_CHOICES
        } else {
            self.fields()
                .into_iter()
                .find(|(key, _, _, _)| *key == field)
                .ok_or_else(|| anyhow::anyhow!("Unknown TV setting."))?
                .2
        };
        let keeping_legacy_bandwidth = field == "bandwidth"
            && !parsec
            && !PHAZE_BANDWIDTH_CHOICES.contains(&LEGACY_BANDWIDTH_CHOICES[self.bandwidth]);
        ensure!(
            index < choices.len() + usize::from(keeping_legacy_bandwidth),
            "Invalid TV setting."
        );
        match field {
            "transport" => self.transport = index,
            "video" => self.video = index,
            "resolution" => self.resolution = index,
            "fps" => self.fps = index,
            "bandwidth" if parsec => self.parsec_bandwidth = Some(index),
            "bandwidth" if keeping_legacy_bandwidth && index == choices.len() => {}
            "bandwidth" => {
                let rate = PHAZE_BANDWIDTH_CHOICES[index];
                self.bandwidth = LEGACY_BANDWIDTH_CHOICES
                    .iter()
                    .position(|stored| *stored == rate)
                    .expect("all new Phaze rates exist in the legacy index domain");
            }
            "pacing" => self.pacing = index,
            _ => unreachable!(),
        }
        Ok(())
    }

    pub fn view(&self) -> Value {
        self.view_for(crate::account::Service::Parsec)
    }

    pub fn view_for(&self, service: crate::account::Service) -> Value {
        let phaze = service == crate::account::Service::Phaze;
        let mut rows: Vec<Value> = self.fields().into_iter()
            .filter(|(key, _, _, _)| !phaze || !matches!(*key, "transport" | "resolution" | "pacing"))
            .map(|(key, label, choices, selected)| {
                if phaze && key == "bandwidth" {
                    let (choices, selected) =
                        bandwidth_menu(false, LEGACY_BANDWIDTH_CHOICES[self.bandwidth]);
                    return json!({"key":key,"label":label,"choices":choices,"selected":selected,"group":"Network"});
                }
                let (choices, selected) = if phaze && key == "video" {
                    (&choices[..2], selected.min(1))
                } else {
                    (choices, selected)
                };
                json!({"key":key,"label":label,"choices":choices,"selected":selected,"group":if key=="transport" || key=="bandwidth" {"Network"} else {"Video"}})
            }).collect();
        if phaze {
            let layout = self.phaze_display_layout;
            rows.push(json!({
                "key": "mirror_display",
                "label": MIRROR_LABEL,
                "choices": MIRROR_CHOICES,
                "selected": usize::from(layout.enabled),
                "group": "Video"
            }));
            if layout.enabled {
                rows.push(json!({
                    "key": "virtual_display_resolution",
                    "label": RESOLUTION_LABEL,
                    "choices": Resolution::labels(),
                    "selected": layout.resolution.index(),
                    "group": "Video"
                }));
            }
        }
        for (key, label, selected) in [
            ("audio_custom", "Custom audio buffer", self.audio.custom),
            (
                "audio_pcm",
                if phaze {
                    "Uncompressed PCM (default AAC-LC)"
                } else {
                    "Raw PCM (LOWLAT only)"
                },
                self.audio.pcm,
            ),
            (
                "audio_surround",
                "Match output speaker layout (LOWLAT only)",
                self.audio.surround,
            ),
            ("logging", "Diagnostic logging", self.logging),
        ] {
            if phaze && key == "audio_surround" {
                continue;
            }
            rows.push(json!({"key":key,"label":label,"choices":["Off","On"],"selected":usize::from(selected),"group":if key=="logging" {"Diagnostics"} else {"Audio"}}));
        }
        for (key, label, value) in [
            (
                "audio_minimum",
                "Minimum audio buffer (ms)",
                self.audio.minimum_ms,
            ),
            (
                "audio_maximum",
                "Maximum audio buffer (ms)",
                self.audio.maximum_ms,
            ),
        ] {
            rows.push(json!({"key":key,"label":label,"choices":BUFFER_MS.iter().map(|v|format!("{v} ms")).collect::<Vec<_>>(),"selected":BUFFER_MS.iter().position(|v|*v==value).unwrap_or(0),"group":"Audio"}));
        }
        let request_hint =
            "Requests apply to your next connection. The host chooses the actual video format.";
        json!({"ok":true,"settings":self,"rows":rows,
            "request_hint":request_hint,
            "video_hint":if phaze {MIRROR_HELP} else {request_hint},
            "audio_hint":if phaze {"Applies to your next connection. Phaze delivers 48 kHz stereo AAC-LC or uncompressed PCM."} else {"Applies to the next connection. Default buffering preserves existing latency. A larger minimum can smooth jitter but adds delay. PCM needs more bandwidth. Surround requires LOWLAT and a verified 5.1/7.1 output; otherwise stereo is used."},
            "diagnostic_hint":"Enable logging before reproducing the issue, then save or share the bounded diagnostic archive. It contains operational events and device information, excluding credentials, clipboard and media content."})
    }

    pub fn host_request(&self) -> Result<crate::session_preferences::Video> {
        self.host_request_for_service(true)
    }

    fn host_request_for_service(&self, parsec: bool) -> Result<crate::session_preferences::Video> {
        PanelFields {
            resolution: RESOLUTION_CHOICES
                .get(self.resolution)
                .ok_or_else(|| anyhow::anyhow!("Invalid resolution."))?,
            bandwidth: (if parsec {
                bandwidth_choices(true)
            } else {
                LEGACY_BANDWIDTH_CHOICES
            })
            .get(if parsec {
                self.parsec_bandwidth.unwrap_or(self.bandwidth)
            } else {
                self.bandwidth
            })
            .ok_or_else(|| anyhow::anyhow!("Invalid bandwidth."))?,
            fps: FPS_CHOICES
                .get(self.fps)
                .ok_or_else(|| anyhow::anyhow!("Invalid frame rate."))?,
            constant_fps: self.pacing == 1,
            rotated: false,
            immersive: 0,
            fullscreen: true,
        }
        .parse_preserving(parsec, None)
    }

    #[cfg(any(target_os = "android", test))]
    pub fn phaze_video_config(&self) -> Result<evoparseq_host_interop::VideoConfig> {
        let video = self.host_request_for_service(false)?;
        Ok(evoparseq_host_interop::VideoConfig {
            hevc: self.video > 0,
            chroma_444: false,
            mbps: video.bandwidth_mbps.unwrap_or(20),
            fps: video.fps.unwrap_or(60),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirror_display_is_opt_in_and_retains_the_selected_resolution() -> Result<()> {
        let mut saved = Preferences::default();
        let resolution = "virtual_display_resolution";
        assert!(saved.select_for_service(resolution, 0, false).is_err());
        assert!(saved.select_for_service("mirror_display", 1, true).is_err());
        saved.select_for_service("mirror_display", 1, false)?;
        saved.select_for_service(resolution, Resolution::Hd.index(), false)?;
        let phaze = saved.view_for(crate::account::Service::Phaze);
        let rows = phaze["rows"].as_array().unwrap();
        assert!(rows.iter().any(|row| row["key"] == resolution));
        assert!(saved
            .select_for_service(resolution, usize::MAX, false)
            .is_err());

        saved.select_for_service("mirror_display", 0, false)?;
        let saved = Preferences::parse(&serde_json::to_string(&saved)?)?;
        assert!(!saved.phaze_display_layout.enabled);
        assert_eq!(saved.phaze_display_layout.resolution, Resolution::Hd);
        Ok(())
    }

    #[test]
    fn parsec_bandwidth_presets_do_not_raise_phaze_stream_limit() -> Result<()> {
        let mut saved = Preferences::default();
        let selected = PHAZE_BANDWIDTH_CHOICES
            .iter()
            .position(|v| *v == "50")
            .unwrap();
        let legacy = LEGACY_BANDWIDTH_CHOICES
            .iter()
            .position(|v| *v == "50")
            .unwrap();
        saved.select_for_service("bandwidth", selected, false)?;
        for mbps in [100_u32, 150] {
            let choice = mbps.to_string();
            let index = PARSEC_BANDWIDTH_CHOICES
                .iter()
                .position(|v| *v == choice)
                .unwrap();
            saved.select("bandwidth", index)?;
            let parsed = Preferences::parse(&serde_json::to_string(&saved)?)?;
            assert_eq!(parsed.host_request()?.requested_bandwidth(true), Some(mbps));
            assert_eq!(parsed.phaze_video_config()?.mbps, 50);
            assert_eq!(parsed.bandwidth, legacy);
            assert_eq!(parsed.parsec_bandwidth, Some(index));
            let view = parsed.view_for(crate::account::Service::Phaze);
            let bandwidth = view["rows"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["key"] == "bandwidth")
                .unwrap();
            assert_eq!(
                bandwidth["selected"].as_u64(),
                Some((PHAZE_BANDWIDTH_CHOICES.len() - 1) as u64)
            );
            assert_eq!(
                bandwidth["choices"].as_array().unwrap().len(),
                PHAZE_BANDWIDTH_CHOICES.len()
            );
        }
        // A later Phaze selection must not discard the Parsec-specific 150.
        saved.select_for_service("bandwidth", 0, false)?;
        assert_eq!(
            LEGACY_BANDWIDTH_CHOICES[saved.bandwidth],
            PHAZE_BANDWIDTH_CHOICES[0]
        );
        assert_eq!(saved.host_request()?.requested_bandwidth(true), Some(150));
        Ok(())
    }

    #[test]
    fn saved_legacy_phaze_index_survives_new_six_choice_menu() -> Result<()> {
        let mut prefs = Preferences::parse(
            r#"{"transport":0,"video":0,"resolution":0,"fps":0,"bandwidth":2}"#,
        )?;
        assert_eq!(LEGACY_BANDWIDTH_CHOICES[prefs.bandwidth], "7");
        assert_eq!(prefs.phaze_video_config()?.mbps, 7);
        let view = prefs.view_for(crate::account::Service::Phaze);
        let row = view["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["key"] == "bandwidth")
            .unwrap();
        assert_eq!(row["choices"].as_array().unwrap().len(), 7);
        assert_eq!(row["choices"][6], "Keep current: 7");
        assert_eq!(row["selected"], 6);
        prefs.select_for_service("bandwidth", 6, false)?;
        assert_eq!(prefs.phaze_video_config()?.mbps, 7);
        prefs.select_for_service("bandwidth", 1, false)?;
        assert_eq!(prefs.phaze_video_config()?.mbps, 10);
        assert_eq!(LEGACY_BANDWIDTH_CHOICES[prefs.bandwidth], "10");
        assert!(prefs.select_for_service("bandwidth", 6, false).is_err());

        prefs.select_for_service("bandwidth", 5, false)?;
        prefs.select_for_service("bandwidth", 13, true)?;
        let restored = Preferences::parse(&serde_json::to_string(&prefs)?)?;
        assert_eq!(restored.phaze_video_config()?.mbps, 50);
        assert_eq!(
            restored.host_request()?.requested_bandwidth(true),
            Some(150)
        );
        Ok(())
    }

    #[test]
    fn saved_tv_limits_are_encoded_in_the_phaze_stream_request() -> Result<()> {
        let mut saved = Preferences::default();
        saved.select_for_service(
            "bandwidth",
            PHAZE_BANDWIDTH_CHOICES
                .iter()
                .position(|v| *v == "50")
                .unwrap(),
            false,
        )?;
        saved.select("fps", FPS_CHOICES.iter().position(|v| *v == "120").unwrap())?;
        let restored = Preferences::parse(&serde_json::to_string(&saved)?)?;
        let request = restored.phaze_video_config()?.request(7)?;
        assert_eq!(request[0], 3);
        assert_eq!(&request[9..13], &50u32.to_be_bytes());
        assert_eq!(&request[13..17], &120u32.to_be_bytes());
        Ok(())
    }

    #[test]
    fn phaze_uses_tv_presets_without_exposing_unavailable_parsec_modes() {
        let preferences = Preferences {
            video: 2,
            ..Default::default()
        };
        let view = preferences.view_for(crate::account::Service::Phaze);
        let rows = view["rows"].as_array().unwrap();
        assert!(!rows.iter().any(|row| matches!(
            row["key"].as_str(),
            Some("transport" | "resolution" | "pacing" | "audio_surround")
        )));
        let video = rows.iter().find(|row| row["key"] == "video").unwrap();
        assert_eq!(video["choices"].as_array().unwrap().len(), 2);
        assert_eq!(video["selected"], 1);
        assert_eq!(preferences.video, 2);
    }

    #[test]
    fn audio_settings_are_backward_compatible_and_invalid_edits_are_atomic() {
        let mut value =
            Preferences::parse(r#"{"transport":0,"video":0,"resolution":0,"fps":0,"bandwidth":0}"#)
                .unwrap();
        assert!(!value.audio.custom && !value.audio.pcm && !value.audio.surround && !value.logging);
        value.select("audio_custom", 1).unwrap();
        let before = serde_json::to_string(&value).unwrap();
        assert!(value.select("audio_maximum", 0).is_err());
        assert_eq!(before, serde_json::to_string(&value).unwrap());
        value.select("audio_minimum", 0).unwrap();
        value.select("audio_maximum", 18).unwrap();
        value.select("audio_pcm", 1).unwrap();
        value.select("audio_surround", 1).unwrap();
        value.select("logging", 1).unwrap();
        let restored = Preferences::parse(&serde_json::to_string(&value).unwrap()).unwrap();
        assert_eq!(restored.audio.maximum(), 500);
        assert_eq!(restored.audio.minimum_frames(), 0);
        assert!(restored.audio.pcm && restored.audio.surround && restored.logging);
    }
    #[test]
    fn default_is_webrtc_h264_and_round_trips_without_secrets() {
        let value = Preferences::default();
        let text = serde_json::to_string(&value).unwrap();
        let restored = Preferences::parse(&text).unwrap();
        assert_eq!(
            restored.view()["rows"][0]["choices"][restored.transport],
            "WebRTC"
        );
        assert_eq!(restored.video, 0);
        assert_eq!(restored.host_request().unwrap().fps, Some(60));
        assert_eq!(
            restored.host_request().unwrap().requested_bandwidth(true),
            Some(20)
        );
        assert_eq!(restored.phaze_video_config().unwrap().mbps, 20);
        assert!(!text.contains("token"));
    }

    #[test]
    fn legacy_pacing_defaults_to_dynamic_and_fixed_is_a_host_request() {
        let legacy = r#"{"transport":0,"video":0,"resolution":0,"fps":0,"bandwidth":0}"#;
        let mut prefs = Preferences::parse(legacy).unwrap();
        assert_eq!(prefs.host_request().unwrap().constant_fps, Some(false));
        prefs.select("pacing", 1).unwrap();
        assert_eq!(prefs.host_request().unwrap().constant_fps, Some(true));
        assert!(prefs.select("pacing", 2).is_err());
        assert_eq!(
            Preferences::parse(&serde_json::to_string(&prefs).unwrap())
                .unwrap()
                .host_request()
                .unwrap()
                .constant_fps,
            Some(true)
        );
    }

    #[test]
    fn reject_invalid_settings_and_preserve_selection_on_failure() {
        let mut value = Preferences::default();
        assert!(value.select("video", 3).is_err());
        assert_eq!(value.video, 0);
        assert!(value.select("unknown", 0).is_err());
        value.select("video", 2).unwrap();
        assert_eq!(value.video, 2);
        assert!(Preferences::parse(
            "{\"transport\":99,\"video\":0,\"resolution\":0,\"fps\":0,\"bandwidth\":0}"
        )
        .is_err());
        assert!(Preferences::parse(&"x".repeat(4097)).is_err());
    }
}
