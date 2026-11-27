//! Experimental display-refresh observations and a startup-only test override.
//! Phaze 0.14.3 encodes two independent Hz values scaled by 10,000.
//! Raw values and host-reported rates are not proof of the applied OS mode.
use crate::{display_layout::DisplayLayout, Output};
use anyhow::{bail, ensure, Result};
use serde::Serialize;
use std::ffi::OsStr;

pub const PROBE_ENV: &str = "EVOPARSEQ_PHAZE_DISPLAY_HZ";

pub(crate) const HZ_SCALE: u32 = 10_000;
// Preserve the existing synthetic physical-size metadata. The rate probe must
// never modify it or claim to have measured a client's physical monitor.
pub(crate) const LEGACY_PHYSICAL_SIZE: [u32; 2] = [60, 1];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Fields {
    pub refresh_scaled: u32,
    pub native_refresh_scaled: u32,
}
impl Fields {
    fn hz(raw: u32) -> Option<f64> {
        (raw != 0).then_some(f64::from(raw) / f64::from(HZ_SCALE))
    }
    pub fn refresh_hz(self) -> Option<f64> {
        Self::hz(self.refresh_scaled)
    }
    pub fn native_refresh_hz(self) -> Option<f64> {
        Self::hz(self.native_refresh_scaled)
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Diagnostic<'a> {
    Requested {
        slot: usize,
        descriptor_id: u32,
        size: [u32; 2],
        physical_size: [u32; 2],
        fields: Fields,
        intended_hz: u32,
        probe_override: bool,
    },
    Reported {
        output: &'a Output,
        reported_hz: Option<f64>,
        reported_native_hz: Option<f64>,
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
            refresh_scaled: self.rates[slot] * HZ_SCALE,
            native_refresh_scaled: self.rates[slot] * HZ_SCALE,
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
                physical_size: LEGACY_PHYSICAL_SIZE,
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
        reported_hz: output.refresh_fields.refresh_hz(),
        reported_native_hz: output.refresh_fields.native_refresh_hz(),
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
    fn output(id: u32, fields: Fields) -> Output {
        Output {
            id,
            name: format!("Synthetic {id}"),
            width: 1920,
            height: 1200,
            active: true,
            primary: id == 17,
            physical_size: [600, 340],
            refresh_fields: fields,
        }
    }
    #[test]
    fn refresh_probe_defaults_and_off_layout_preserve_control_boundaries() -> Result<()> {
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
                    refresh_scaled: 600_000,
                    native_refresh_scaled: 600_000
                }
            );
        }
        Ok(())
    }
    #[test]
    fn refresh_probe_encodes_hz_times_10000_without_changing_physical_size() -> Result<()> {
        for count in DisplayCount::ALL {
            let saved = layout(count);
            let probe = Probe::parse(saved, Some(OsStr::new("120")))?;
            let events = probe
                .requests(saved)
                .map(serde_json::to_value)
                .collect::<std::result::Result<Vec<_>, _>>()?;
            assert_eq!(events.len(), count.total());
            for (slot, event) in events.iter().enumerate() {
                assert_eq!(event["fields"]["refresh_scaled"], 1_200_000);
                assert_eq!(event["fields"]["native_refresh_scaled"], 1_200_000);
                assert_eq!(
                    event["physical_size"],
                    serde_json::json!(LEGACY_PHYSICAL_SIZE)
                );
                assert_eq!(event["descriptor_id"], descriptor_id(slot));
                assert_eq!(event["slot"], slot);
                assert_eq!(event["probe_override"], true);
            }
            assert_eq!(saved.resolutions(), [Resolution::Wuxga; 4]);
        }
        let saved = layout(DisplayCount::Two);
        let probe = Probe::parse(saved, Some(OsStr::new(" 60, 120 ")))?;
        assert_eq!(probe.fields(0).refresh_scaled, 600_000);
        assert_eq!(probe.fields(1).refresh_scaled, 1_200_000);
        assert_eq!(probe.fields(2).refresh_scaled, 600_000);
        let event = serde_json::to_value(Probe::default().requests(saved).next().unwrap())?;
        assert_eq!(event["probe_override"], false);
        assert_eq!(event["intended_hz"], 60);
        assert_eq!(event["size"], serde_json::json!([1920, 1200]));
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
    fn refresh_and_native_refresh_decode_independently_not_as_a_ratio() -> Result<()> {
        for (raw, native, hz, native_hz) in [
            (600_000, 1_200_000, Some(60.), Some(120.)),
            (240_000, 600_000, Some(24.), Some(60.)),
            (599_400, 600_000, Some(59.94), Some(60.)),
            (10_000, 10_000, Some(1.), Some(1.)),
            (0, 600_000, None, Some(60.)),
            (600_000, 0, Some(60.), None),
            (0, 0, None, None),
            (
                u32::MAX,
                1,
                Some(f64::from(u32::MAX) / 10_000.),
                Some(0.0001),
            ),
        ] {
            let output = output(
                91,
                Fields {
                    refresh_scaled: raw,
                    native_refresh_scaled: native,
                },
            );
            assert_eq!(output.refresh_fields.refresh_hz(), hz);
            assert_eq!(output.refresh_fields.native_refresh_hz(), native_hz);
            let event = serde_json::to_value(reported(&output))?;
            assert_eq!(event["kind"], "reported");
            assert_eq!(event["output"]["id"], 91);
            assert_eq!(
                event["output"]["physical_size"],
                serde_json::json!([600, 340])
            );
            assert_eq!(event["output"]["refresh_fields"]["refresh_scaled"], raw);
            assert_eq!(
                event["output"]["refresh_fields"]["native_refresh_scaled"],
                native
            );
            assert_eq!(event["reported_hz"], serde_json::json!(hz));
            assert_eq!(event["reported_native_hz"], serde_json::json!(native_hz));
            assert!(event.get("candidate_hz").is_none());
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
                panic!("unexpected outputs");
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
        let saved = layout(DisplayCount::Two);
        Probe::parse(saved, Some(OsStr::new("60,120")))?.hello_sent(saved, &mut sink);
        let outputs = [
            output(
                91,
                Fields {
                    refresh_scaled: 240_000,
                    native_refresh_scaled: 600_000,
                },
            ),
            output(17, Fields::default()),
        ];
        outputs_received(&outputs, &mut sink);
        assert_eq!(sink.0.len(), 4);
        assert_eq!(sink.0[0]["descriptor_id"], descriptor_id(0));
        assert_eq!(sink.0[1]["intended_hz"], 120);
        assert_eq!(sink.0[2]["output"]["id"], 91);
        assert_eq!(sink.0[2]["reported_hz"], 24.);
        assert_eq!(sink.0[3]["output"]["id"], 17);
        assert!(sink.0[3]["reported_hz"].is_null());
        Ok(())
    }
}
