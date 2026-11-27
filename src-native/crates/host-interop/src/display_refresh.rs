//! Experimental display-refresh observations and a startup-only test override.
//! Raw words are retained: their ratio is a candidate, not verified OS mode.
use crate::{display_layout::DisplayLayout, Output};
use anyhow::{bail, ensure, Result};
use serde::Serialize;
use std::ffi::OsStr;

pub const PROBE_ENV: &str = "EVOPARSEQ_PHAZE_DISPLAY_HZ";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Fields {
    pub word_0: u32,
    pub word_1: u32,
}
impl Fields {
    pub fn candidate_hz(self) -> Option<f64> {
        (self.word_0 != 0 && self.word_1 != 0)
            .then(|| f64::from(self.word_0) / f64::from(self.word_1))
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Diagnostic<'a> {
    Requested {
        slot: usize,
        descriptor_id: u32,
        size: [u32; 2],
        fields: Fields,
        intended_hz: u32,
        probe_override: bool,
    },
    Reported {
        output: &'a Output,
        candidate_hz: Option<f64>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Probe {
    rates: [u32; 4],
    overridden: bool,
}
impl Default for Probe {
    fn default() -> Self {
        Self {
            rates: [60; 4],
            overridden: false,
        }
    }
}

pub(crate) fn descriptor_id(slot: usize) -> u32 {
    0x4556_5031 + slot as u32
}

impl Probe {
    pub(crate) fn from_environment(layout: DisplayLayout) -> Result<Self> {
        let value = layout
            .enabled
            .then(|| std::env::var_os(PROBE_ENV))
            .flatten();
        Self::parse(layout, value.as_deref())
    }
    pub(crate) fn parse(layout: DisplayLayout, value: Option<&OsStr>) -> Result<Self> {
        let mut probe = Self::default();
        if !layout.enabled {
            return Ok(probe);
        }
        let Some(value) = value else {
            return Ok(probe);
        };
        let value = value
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("{PROBE_ENV} must contain UTF-8 60/120 Hz values"))?;
        ensure!(value.len() <= 31, "{PROBE_ENV} is too long");
        let rates = value
            .split(',')
            .map(|part| match part.trim() {
                "60" => Ok(60),
                "120" => Ok(120),
                _ => bail!("{PROBE_ENV} only accepts 60 or 120 Hz"),
            })
            .collect::<Result<Vec<_>>>()?;
        let count = layout.count.total();
        ensure!(
            rates.len() == 1 || rates.len() == count,
            "{PROBE_ENV} needs one value or one value per requested display"
        );
        for (slot, rate) in probe.rates.iter_mut().take(count).enumerate() {
            *rate = rates[if rates.len() == 1 { 0 } else { slot }];
        }
        probe.overridden = true;
        Ok(probe)
    }
    /// Called only after the carrier has accepted the startup hello.
    pub(crate) fn hello_sent(self, layout: DisplayLayout, sink: &mut impl crate::Sink) {
        for event in self.requests(layout) {
            sink.display_refresh_diagnostic(&event);
        }
    }
    pub(crate) fn fields(self, slot: usize) -> Fields {
        Fields {
            word_0: self.rates[slot],
            word_1: 1,
        }
    }
    pub(crate) fn requests(
        self,
        layout: DisplayLayout,
    ) -> impl Iterator<Item = Diagnostic<'static>> {
        layout
            .resolutions()
            .into_iter()
            .enumerate()
            .take(if layout.enabled {
                layout.count.total()
            } else {
                0
            })
            .map(move |(slot, resolution)| Diagnostic::Requested {
                slot,
                descriptor_id: descriptor_id(slot),
                size: resolution.dimensions(),
                fields: self.fields(slot),
                intended_hz: self.rates[slot],
                probe_override: self.overridden,
            })
    }
}

pub(crate) fn outputs_received(outputs: &[Output], sink: &mut impl crate::Sink) {
    for output in outputs {
        sink.display_refresh_diagnostic(&reported(output));
    }
}

pub(crate) fn reported(output: &Output) -> Diagnostic<'_> {
    Diagnostic::Reported {
        output,
        candidate_hz: output.refresh_fields.candidate_hz(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display_layout::{DisplayCount, Resolution};
    fn layout(count: DisplayCount) -> DisplayLayout {
        DisplayLayout {
            enabled: true,
            count,
            ..Default::default()
        }
    }
    #[test]
    fn refresh_probe_defaults_and_off_layout_preserve_existing_behavior() -> Result<()> {
        assert_eq!(
            Probe::parse(layout(DisplayCount::Four), None)?,
            Probe::default()
        );
        let off = DisplayLayout::default();
        assert_eq!(
            Probe::parse(off, Some(OsStr::new("invalid")))?,
            Probe::default()
        );
        assert_eq!(Probe::default().requests(off).count(), 0);
        assert_eq!(Probe::from_environment(off)?, Probe::default());
        for slot in 0..4 {
            assert_eq!(
                Probe::default().fields(slot),
                Fields {
                    word_0: 60,
                    word_1: 1
                }
            );
        }
        Ok(())
    }
    #[test]
    fn refresh_probe_accepts_uniform_and_per_display_values_without_changing_preferences(
    ) -> Result<()> {
        for count in DisplayCount::ALL {
            let saved = layout(count);
            let probe = Probe::parse(saved, Some(OsStr::new("120")))?;
            let events = probe
                .requests(saved)
                .map(serde_json::to_value)
                .collect::<std::result::Result<Vec<_>, _>>()?;
            assert_eq!(events.len(), count.total());
            for (slot, event) in events.iter().enumerate() {
                assert_eq!(event["fields"]["word_0"], 120);
                assert_eq!(event["fields"]["word_1"], 1);
                assert_eq!(event["descriptor_id"], descriptor_id(slot));
                assert_eq!(event["slot"], slot);
                assert_eq!(event["probe_override"], true);
            }
            assert_eq!(saved.resolutions(), [Resolution::Wuxga; 4]);
        }
        let saved = layout(DisplayCount::Two);
        let probe = Probe::parse(saved, Some(OsStr::new(" 60, 120 ")))?;
        assert_eq!(probe.fields(0).word_0, 60);
        assert_eq!(probe.fields(1).word_0, 120);
        assert_eq!(probe.fields(2).word_0, 60);
        let default_event = serde_json::to_value(Probe::default().requests(saved).next().unwrap())?;
        assert_eq!(default_event["probe_override"], false);
        assert_eq!(default_event["intended_hz"], 60);
        assert_eq!(default_event["size"], serde_json::json!([1920, 1200]));
        Ok(())
    }
    #[test]
    fn refresh_probe_rejects_invalid_values_and_counts_before_sending() {
        for value in [
            "",
            "24",
            "0",
            "240",
            "+60",
            "60Hz",
            "60,",
            "60,120,60",
            "60,60,60,60,60",
            "120120120120120120120120120120120120",
        ] {
            assert!(
                Probe::parse(layout(DisplayCount::Two), Some(OsStr::new(value))).is_err(),
                "{value}"
            );
        }
    }
    #[test]
    fn invalid_os_strings_are_rejected_only_when_mirroring() {
        #[cfg(windows)]
        let invalid = {
            use std::os::windows::ffi::OsStringExt;
            std::ffi::OsString::from_wide(&[0xd800])
        };
        #[cfg(unix)]
        let invalid = {
            use std::os::unix::ffi::OsStringExt;
            std::ffi::OsString::from_vec(vec![0xff])
        };
        assert!(Probe::parse(layout(DisplayCount::One), Some(&invalid)).is_err());
        assert_eq!(
            Probe::parse(DisplayLayout::default(), Some(&invalid)).unwrap(),
            Probe::default()
        );
    }
    #[test]
    fn reported_refresh_keeps_raw_words_and_unknown_values() -> Result<()> {
        for (num, den, hz) in [
            (60, 1, Some(60.)),
            (24, 1, Some(24.)),
            (60000, 1001, Some(60000. / 1001.)),
            (0, 1, None),
            (120, 0, None),
            (0, 0, None),
            (u32::MAX, 1, Some(f64::from(u32::MAX))),
        ] {
            let output = Output {
                id: 17,
                name: "Synthetic second display".into(),
                width: 1920,
                height: 1200,
                active: true,
                primary: false,
                refresh_fields: Fields {
                    word_0: num,
                    word_1: den,
                },
            };
            assert_eq!(output.refresh_fields.candidate_hz(), hz);
            let event = serde_json::to_value(reported(&output))?;
            assert_eq!(event["kind"], "reported");
            assert_eq!(event["output"]["id"], 17);
            assert_eq!(event["output"]["refresh_fields"]["word_0"], num);
            assert_eq!(event["output"]["refresh_fields"]["word_1"], den);
            if let Some(hz) = hz {
                assert_eq!(event["candidate_hz"].as_f64(), Some(hz));
            } else {
                assert!(event["candidate_hz"].is_null());
            }
        }
        Ok(())
    }
    #[test]
    fn refresh_notifications_preserve_host_identity_and_never_drive_media_or_input() -> Result<()> {
        #[derive(Default)]
        struct Recorder(Vec<serde_json::Value>);
        impl crate::Sink for Recorder {
            fn display_refresh_diagnostic(&mut self, event: &Diagnostic<'_>) {
                self.0.push(serde_json::to_value(event).unwrap());
            }
            fn stage(&mut self, _: &'static str) {
                panic!("unexpected stage");
            }
            fn outputs(&mut self, _: &[Output]) {
                panic!("diagnostics must not replace outputs");
            }
            fn video(&mut self, _: &[u8], _: bool) -> Result<()> {
                panic!("unexpected video");
            }
            fn audio(&mut self, _: &[i16], _: crate::AudioCodec, _: usize) -> Result<()> {
                panic!("unexpected audio");
            }
            fn poll(
                &mut self,
                _: Option<u32>,
                _: &mut crate::InputBatch,
                _: &mut Vec<crate::ClientControl>,
            ) -> Result<bool> {
                panic!("unexpected input");
            }
        }
        let mut sink = Recorder::default();
        Probe::default().hello_sent(DisplayLayout::default(), &mut sink);
        outputs_received(&[], &mut sink);
        assert!(sink.0.is_empty());
        let layout = layout(DisplayCount::Two);
        Probe::parse(layout, Some(OsStr::new("60,120")))?.hello_sent(layout, &mut sink);
        let outputs = [
            Output {
                id: 91,
                name: "Synthetic host output".into(),
                width: 1920,
                height: 1200,
                active: true,
                primary: false,
                refresh_fields: Fields {
                    word_0: 24,
                    word_1: 1,
                },
            },
            Output {
                id: 17,
                name: "Synthetic primary".into(),
                width: 1920,
                height: 1200,
                active: true,
                primary: true,
                refresh_fields: Fields {
                    word_0: 0,
                    word_1: 0,
                },
            },
        ];
        outputs_received(&outputs, &mut sink);
        assert_eq!(sink.0.len(), 4);
        assert_eq!(sink.0[0]["descriptor_id"], descriptor_id(0));
        assert_eq!(sink.0[1]["intended_hz"], 120);
        assert_eq!(sink.0[2]["output"]["id"], 91);
        assert_eq!(sink.0[2]["candidate_hz"], 24.);
        assert_eq!(sink.0[3]["output"]["id"], 17);
        assert!(sink.0[3]["candidate_hz"].is_null());
        Ok(())
    }
}
