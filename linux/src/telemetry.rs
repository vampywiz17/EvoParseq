//! Demand-driven adapters for the common desktop telemetry/presentation model.
//! No media buffers, GPU queries, rendering passes or additional worker threads.
use crate::{connection::Mode, stats, stats_view};
use anyhow::Result;
use bytes::Bytes;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[derive(Default)]
pub struct Host {
    pub config: crate::host_video::Config,
    pub outputs: Vec<crate::host_displays::Output>,
}
pub struct Session {
    pub bus: Arc<stats::Shared>,
    pub generation: u64,
    pub mode: Mode,
    pub host: Mutex<Host>,
    audio: Mutex<Option<crate::audio_stream::NativeReceiver>>,
    next: Mutex<Option<Instant>>,
    pub output: Arc<Mutex<crate::audio_pipewire::State>>,
}
impl Session {
    pub fn new(mode: Mode) -> Arc<Self> {
        let bus = Arc::new(stats::Shared::default());
        let generation = bus.begin();
        Arc::new(Self {
            bus,
            generation,
            mode,
            host: Mutex::new(Host::default()),
            audio: Mutex::new(None),
            next: Mutex::new(None),
            output: Arc::new(Mutex::new(Default::default())),
        })
    }
    pub fn set_audio(&self, audio: crate::audio_stream::NativeReceiver) {
        *self.audio.lock().unwrap_or_else(|e| e.into_inner()) = Some(audio);
    }
    pub fn sample_due(&self) -> bool {
        if !self.bus.sample_requested() {
            return false;
        }
        let mut next = self.next.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        if next.is_some_and(|at| now < at) {
            return false;
        }
        *next = Some(now + Duration::from_secs(1));
        true
    }
}
#[derive(Default)]
pub struct Control {
    timing: crate::frame_timing::Requests,
    queried: bool,
}
impl Control {
    pub fn receive(&mut self, bytes: &[u8], session: &Session) {
        if session.bus.host_timing.receive(session.generation, bytes) {
            return;
        }
        if let Ok(crate::control::Message::Buffer {
            event,
            payload: Some(range),
        }) = crate::control::decode(bytes)
        {
            if event["type"] != 3 {
                return;
            }
            let mut host = session.host.lock().unwrap_or_else(|e| e.into_inner());
            match event["id"].as_u64() {
                Some(11) => {
                    let _ = host.config.receive(&bytes[range]);
                }
                Some(12) => {
                    if let Ok(outputs) = crate::host_displays::parse(&bytes[range]) {
                        host.outputs = outputs;
                    }
                }
                _ => {}
            }
        }
    }
    pub fn update(
        &mut self,
        ready: bool,
        session: &Session,
        mut send: impl FnMut(Bytes) -> Result<()>,
    ) -> Result<()> {
        if !ready {
            return Ok(());
        }
        let desired = session.bus.sample_requested();
        if desired && !self.queried {
            // Stock-host read-only queries, once per connection, on explicit diagnostics visibility.
            self.queried = true;
            let _ = send(crate::host_video::query()?);
            let _ = send(crate::host_displays::query()?);
        }
        if let Some(payload) = self.timing.update(desired) {
            session
                .bus
                .host_timing
                .set_requested(session.generation, desired);
            if send(Bytes::copy_from_slice(&payload)).is_err() {
                self.timing.fail();
                session.bus.host_timing.failed(session.generation);
            }
        }
        Ok(())
    }
    pub fn stop(&mut self, session: &Session, mut send: impl FnMut(Bytes) -> Result<()>) {
        if let Some(bytes) = self.timing.update(false) {
            let _ = send(Bytes::copy_from_slice(&bytes));
        }
        session.bus.host_timing.end(session.generation);
    }
}
#[derive(Default)]
pub struct Details {
    pub status: String,
    pub cards: Vec<(&'static str, String)>,
    pub rows: Vec<stats_view::Row>,
    pub footer: String,
}
#[derive(Default)]
pub struct View {
    next: Option<Instant>,
    details: Vec<Details>,
    rates: stats::Rates,
    pub sample: Option<stats::Sample>,
    pub values: stats::Values,
    pub timing: crate::frame_timing::Snapshot,
    pub resources: crate::resources_linux::Sampler,
    pub compact: String,
    pub fields: [String; 6],
    pub display: String,
    pub display_hint: String,
    pub status: String,
    pub audio: crate::audio_pipewire::State,
}
impl View {
    pub fn refresh(
        &mut self,
        session: &Session,
        playback: &crate::playback::Playback,
        decoder: Option<&crate::video_vulkan::Decoder>,
        stats_open: bool,
        refresh_hz: Option<f64>,
    ) {
        if !session.bus.sample_requested() {
            self.next = None;
            self.resources.pause();
            return;
        }
        let now = Instant::now();
        if self.next.is_some_and(|at| now < at) {
            return;
        }
        self.next = Some(now + Duration::from_secs(1));
        if stats_open {
            self.resources.update(now);
        } else {
            self.resources.pause();
        }
        let Some(mut sample) = session.bus.read() else {
            return;
        };
        let mut video = playback.video_snapshot();
        if let Some(decoder) = decoder {
            sample.profile = decoder.snapshot(&mut video).or(sample.profile);
        }
        sample.video = Some(video);
        if let Some(audio) = session
            .audio
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            let report = audio.snapshot();
            sample.audio_ready = report.codec.is_some();
            sample.audio_codec = report.codec;
            sample.audio_layout = report.layout;
            sample.audio_decoded_queue_ms = Some(report.queued_samples as f64 / 96.);
            sample.audio_native = Some(report);
        }
        self.timing = session.bus.host_timing.snapshot(session.generation);
        sample.host_encode_ms = self.timing.video[0]
            .as_ref()
            .filter(|s| s.at.elapsed() < Duration::from_secs(3))
            .map(|s| s.last[2]);
        if self
            .sample
            .as_ref()
            .is_none_or(|old| old.at != sample.at || old.generation != sample.generation)
        {
            self.values = self.rates.update(&sample);
        }
        self.compact = stats_view::compact_metrics(Some(&sample), &self.values);
        let host = session.host.lock().unwrap_or_else(|e| e.into_inner());
        let observed = host.config.observed();
        let value = |key: &str| {
            observed[key].as_u64().map_or_else(
                || "Not reported by host".into(),
                |v| {
                    if v == 0 {
                        "Host default".into()
                    } else {
                        v.to_string()
                    }
                },
            )
        };
        let width = observed["width"].as_u64();
        let height = observed["height"].as_u64();
        let resolution = match width.zip(height) {
            Some((w, h)) if w > 0 && h > 0 => format!("{w} x {h}"),
            Some(_) => "Keep host".into(),
            None => "Not reported by host".into(),
        };
        self.fields = [
            resolution,
            value("bandwidth_mbps"),
            value("fps"),
            observed["constant_fps"]
                .as_bool()
                .map_or("Not reported by host", |fixed| {
                    if fixed { "Fixed" } else { "Dynamic" }
                })
                .into(),
            observed["rotated"]
                .as_bool()
                .map_or("Not reported by host", |rotated| {
                    if rotated { "Rotated" } else { "Normal" }
                })
                .into(),
            "Off".into(),
        ];
        self.display = host
            .config
            .output_id()
            .and_then(|id| host.outputs.iter().find(|o| o.id == id))
            .map_or_else(
                || {
                    if host.outputs.is_empty() {
                        "Not reported by host".into()
                    } else {
                        "Current output not reported".into()
                    }
                },
                |o| o.name.clone(),
            );
        self.display_hint = format!(
            "{} | {}",
            session.mode.label(),
            sample
                .bud
                .as_ref()
                .map_or(sample.route, |bud| if !bud.connected {
                    "Connecting"
                } else if bud.relay {
                    "TURN relay"
                } else {
                    "Direct"
                })
        );
        let fps = observed["fps"]
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .filter(|v| *v > 0);
        self.status = crate::display_timing::Display {
            hz: refresh_hz,
            ..Default::default()
        }
        .advice(fps.unwrap_or(60), fps, self.values.fps, false);
        self.audio = session
            .output
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        drop(host);
        self.sample = Some(sample);
        self.details = (0..5).map(|tab| self.build(tab)).collect();
    }
    pub fn presentation(&self, tab: usize) -> Option<&Details> {
        self.details.get(tab)
    }
    fn build(&self, tab: usize) -> Details {
        let sample = self.sample.as_ref().unwrap();
        let mut result = stats_view::Presentation::new(
            sample,
            &self.values,
            (self.resources.cpu, None, None),
            None,
            &self.timing,
        )
        .select(stats_view::Tab::ALL[tab]);
        if tab == 2 {
            for (label, value) in &mut result.rows {
                if label == crate::app_style::forms::STAT_HARDWARE_DECODE {
                    *value =
                        "Vulkan Video configured; hardware execution not independently verified"
                            .into();
                }
                if label == crate::app_style::forms::STAT_PRESENTATION_RATE_SCOPE {
                    *value = "Scheduled Vulkan draws; not monitor scanout".into();
                }
            }
        }
        if tab == 0 {
            let f = crate::app_style::forms::STAT_MEMORY_RESIDENT_TOTAL;
            result.rows.push((
                f.into(),
                self.resources.resident.map_or_else(
                    || "Not reported by Linux".into(),
                    |v| format!("{:.1} MiB (Linux RSS estimate)", v as f64 / 1048576.),
                ),
            ));
            result.rows.push((
                crate::app_style::forms::STAT_MEMORY_RESIDENT_PRIVATE.into(),
                "No equivalent collected (Linux accounting differs)".into(),
            ));
            result.rows.push((
                crate::app_style::forms::STAT_MEMORY_COMMITTED_PRIVATE.into(),
                "Not applicable (Linux commit accounting)".into(),
            ));
            for (label, value) in &mut result.rows {
                if label == crate::app_style::forms::STAT_GPU_BUSIEST_ENGINE {
                    *value = "Not collected (driver sampling disabled)".into();
                }
            }
        }
        Details {
            status: result.status.into(),
            cards: result.cards,
            rows: result.rows,
            footer: result.footer,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linux_diagnostics_queries_once_and_stops_host_reports_when_hidden() {
        let session = Session::new(Mode::Bud);
        let mut control = Control::default();
        let mut sent = Vec::new();
        control
            .update(true, &session, |b| {
                sent.push(b);
                Ok(())
            })
            .unwrap();
        assert!(sent.is_empty());
        assert!(!session.sample_due());
        session
            .bus
            .overlay_visible
            .store(true, std::sync::atomic::Ordering::Relaxed);
        control
            .update(true, &session, |b| {
                sent.push(b);
                Ok(())
            })
            .unwrap();
        assert_eq!(sent.len(), 3);
        control
            .update(true, &session, |b| {
                sent.push(b);
                Ok(())
            })
            .unwrap();
        assert_eq!(sent.len(), 3);
        assert!(session.sample_due());
        assert!(!session.sample_due());
        session
            .bus
            .overlay_visible
            .store(false, std::sync::atomic::Ordering::Relaxed);
        control
            .update(true, &session, |b| {
                sent.push(b);
                Ok(())
            })
            .unwrap();
        assert_eq!(sent.len(), 4);
        assert_eq!(sent[3][3], 0);
    }
}
