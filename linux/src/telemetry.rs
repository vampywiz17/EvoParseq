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
    pub pending: bool,
    pub status: String,
    pub revision: u64,
    pub outputs_revision: u64,
    pub windows: bool,
    pub hid_mouse: bool,
    pub primary_ready: bool,
}
pub struct Session {
    pub displays: Arc<crate::display_routes::Displays>,
    pub bus: Arc<stats::Shared>,
    pub generation: u64,
    pub mode: Mode,
    pub host: Mutex<Host>,
    audio: Mutex<Option<crate::audio_stream::NativeReceiver>>,
    next: Mutex<Option<Instant>>,
    pub output: Arc<Mutex<crate::audio_pipewire::State>>,
    pub client_size: Mutex<(u32, u32)>,
    pub settings: Mutex<Option<Arc<crate::connection_settings::Manager>>>,
}
impl Session {
    pub fn new(mode: Mode) -> Arc<Self> {
        let bus = Arc::new(stats::Shared::default());
        let generation = bus.begin();
        Arc::new(Self {
            displays: Arc::new(crate::display_routes::Displays::default()),
            bus,
            generation,
            mode,
            host: Mutex::new(Host::default()),
            audio: Mutex::new(None),
            next: Mutex::new(None),
            output: Arc::new(Mutex::new(Default::default())),
            settings: Mutex::new(None),
            client_size: Mutex::new((1920, 1080)),
        })
    }
    pub fn set_audio(&self, audio: crate::audio_stream::NativeReceiver) {
        *self.audio.lock().unwrap_or_else(|e| e.into_inner()) = Some(audio);
    }
    pub fn sync_input(&self, playback: &crate::playback::Playback) {
        if let Some(settings) = self
            .settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            playback.immersive.store(
                settings.native_video().immersive,
                std::sync::atomic::Ordering::Release,
            );
        }
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
    pub(crate) hid_mouse: bool,
    primary_attempted: Option<String>,
    primary_revision: Option<(u64, u64)>,
    confirmation: Option<crate::host_video::Confirmation>,
    selection: Option<(String, u64, u64, Instant)>,
}
impl Control {
    pub fn receive(&mut self, bytes: &[u8], session: &Session) {
        if bytes.len() == 13 && bytes[12] == 27 {
            if let Ok(stream) = usize::try_from(i32::from_be_bytes(bytes[..4].try_into().unwrap()))
            {
                session.displays.close(stream);
            }
            return;
        }
        if session.bus.host_timing.receive(session.generation, bytes) {
            return;
        }
        // Cursor/control metadata still takes its existing decoding path once.
        // Only host-settings replies belong to this adapter; do not allocate a
        // second cursor JSON event on every control frame, even when UI is hidden.
        if bytes.get(12) != Some(&17)
            || !bytes
                .get(4..8)
                .is_some_and(|id| id == 11i32.to_be_bytes() || id == 12i32.to_be_bytes())
        {
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
            // Match the Windows session adapter: stock text replies include a
            // single trailing NUL in the validated USER_DATA payload length.
            let payload = &bytes[range];
            let payload = payload.strip_suffix(&[0]).unwrap_or(payload);
            let mut host = session.host.lock().unwrap_or_else(|e| e.into_inner());
            match event["id"].as_u64() {
                Some(11) => {
                    if host.config.receive(payload).is_ok() {
                        host.revision = host.revision.wrapping_add(1);
                        host.windows = host.config.windows_host();
                    }
                }
                Some(12) => {
                    if let Ok(outputs) = crate::host_displays::parse(payload) {
                        host.outputs = outputs;
                        host.outputs_revision = host.outputs_revision.wrapping_add(1);
                    }
                }
                _ => {}
            }
        }
    }
    pub fn command(
        &mut self,
        command: crate::session_controls::Command,
        session: &Session,
        _size: (u32, u32),
        mut send: impl FnMut(Bytes) -> Result<()>,
    ) -> Result<()> {
        use crate::session_controls::{Command, Status};
        if matches!(
            &command,
            Command::Apply(_) | Command::SelectDisplay(_) | Command::DisplayResolution { .. }
        ) && (self.confirmation.is_some() || self.selection.is_some())
        {
            return Ok(());
        }
        let result = (|| -> Result<()> {
            match command {
                Command::AddDisplay(stream) => {
                    let host = session.host.lock().unwrap_or_else(|e| e.into_inner());
                    anyhow::ensure!(
                        !host.hid_mouse,
                        "Additional displays require normal mouse input"
                    );
                    anyhow::ensure!(
                        session.mode != Mode::Bud
                            || !session
                                .settings
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .as_ref()
                                .is_some_and(|m| m.view().0.hid_keyboard_enabled()),
                        "Additional displays require normal keyboard input"
                    );
                    anyhow::ensure!(
                        session.displays.get(stream).is_some(),
                        "Display window is unavailable"
                    );
                    drop(host);
                    let result = (|| {
                        send(crate::control::header(
                            13,
                            stream as i32,
                            session.displays.flags(),
                            0,
                        ))?;
                        send(crate::display_stream::enable(stream, true)?)
                    })();
                    if result.is_err() {
                        session.displays.close(stream);
                    }
                    result?;
                }
                Command::DisplayResolution {
                    stream,
                    resolution,
                    size,
                } => {
                    anyhow::ensure!(session.displays.get(stream).is_some(), "Display is closed");
                    let frame = session
                        .host
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .config
                        .resolution(stream, resolution, size)?;
                    let confirmation = crate::host_video::Confirmation::resolution(
                        &frame,
                        stream,
                        Instant::now(),
                    )?;
                    send(frame)?;
                    self.confirmation = Some(confirmation);
                    let mut host = session.host.lock().unwrap_or_else(|e| e.into_inner());
                    host.pending = true;
                    host.status = Status::Waiting.text().into();
                }
                Command::RefreshDisplays => {
                    self.primary_attempted = None;
                    send(crate::host_displays::query()?)?;
                    send(crate::host_video::query()?)?;
                }
                Command::SecureAttention => {
                    anyhow::ensure!(
                        session
                            .host
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .windows,
                        "Host does not report Windows secure attention support"
                    );
                    send(crate::control::header(24, 0, 0, 0))?;
                    send(crate::control::text(17, 14, "")?)?;
                    session
                        .host
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .status = Status::AttentionSent.text().into();
                }
                Command::Apply(video) => {
                    video.validate()?;
                    anyhow::ensure!(
                        self.confirmation.is_none() && self.selection.is_none(),
                        "Waiting for the previous host request"
                    );
                    let frame = session
                        .host
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .config
                        .request(
                            &video,
                            *session
                                .client_size
                                .lock()
                                .unwrap_or_else(|e| e.into_inner()),
                        )?;
                    let confirmation =
                        crate::host_video::Confirmation::new(&frame, Instant::now())?;
                    send(frame)?;
                    self.confirmation = Some(confirmation);
                    if let Some(manager) = session
                        .settings
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .as_ref()
                    {
                        manager.update_native(|profile| profile.video = video)?;
                    }
                    let mut host = session.host.lock().unwrap_or_else(|e| e.into_inner());
                    host.pending = true;
                    host.status = Status::Waiting.text().into();
                }
                Command::SelectDisplay(id) => {
                    anyhow::ensure!(
                        self.confirmation.is_none() && self.selection.is_none(),
                        "Waiting for the previous host request"
                    );
                    let mut host = session.host.lock().unwrap_or_else(|e| e.into_inner());
                    anyhow::ensure!(
                        !host.hid_mouse,
                        "Display switching is unavailable with HID mouse input"
                    );
                    anyhow::ensure!(
                        host.outputs.iter().any(|output| output.id == id),
                        "Host display is unavailable; refresh the list"
                    );
                    self.selection = Some((
                        id,
                        host.revision,
                        host.outputs_revision,
                        Instant::now() + Duration::from_secs(5),
                    ));
                    host.pending = true;
                    host.status = Status::Waiting.text().into();
                    drop(host);
                    send(crate::host_displays::query()?)?;
                    send(crate::host_video::query()?)?;
                }
                _ => {}
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.selection = None;
            self.confirmation = None;
            let mut host = session.host.lock().unwrap_or_else(|e| e.into_inner());
            host.pending = false;
            host.status = error.to_string();
        }
        Ok(()) // UI-command failure never tears down an otherwise working stream.
    }
    fn confirm(&mut self, session: &Session, mut send: impl FnMut(Bytes) -> Result<()>) {
        if self.confirmation.is_none() && self.selection.is_none() {
            return;
        }
        use crate::session_controls::Status;
        let mut host = session.host.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        if let Some((id, revision, outputs, deadline)) = &self.selection {
            if now >= *deadline {
                self.selection = None;
                host.pending = false;
                host.status = Status::Unconfirmed.text().into();
            } else if host.revision != *revision && host.outputs_revision != *outputs {
                let result = (|| -> Result<()> {
                    anyhow::ensure!(
                        host.outputs.iter().any(|output| output.id == *id),
                        "Host display disappeared"
                    );
                    send(host.config.select_output(id)?)?;
                    self.confirmation = Some(crate::host_video::Confirmation::output(id, now));
                    if let Some(manager) = session
                        .settings
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .as_ref()
                    {
                        manager.update_native(|profile| {
                            profile.video.resolution = crate::session_preferences::Resolution::Host
                        })?;
                    }
                    Ok(())
                })();
                self.selection = None;
                if let Err(error) = result {
                    host.pending = false;
                    host.status = error.to_string();
                }
            }
        }
        if let Some(confirmation) = &mut self.confirmation {
            match confirmation.poll(&host.config, now) {
                crate::host_video::Verification::Query => {
                    if send(crate::host_video::query().unwrap()).is_err() {
                        self.confirmation = None;
                        host.pending = false;
                        host.status = Status::Failed.text().into();
                    }
                }
                crate::host_video::Verification::Confirmed => {
                    self.confirmation = None;
                    host.pending = false;
                    host.status = Status::Confirmed.text().into();
                }
                crate::host_video::Verification::TimedOut => {
                    self.confirmation = None;
                    host.pending = false;
                    host.status = Status::Unconfirmed.text().into();
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
        self.confirm(session, &mut send);
        if self.hid_mouse {
            let mut host = session.host.lock().unwrap_or_else(|e| e.into_inner());
            if host.hid_mouse
                && self.primary_revision != Some((host.revision, host.outputs_revision))
            {
                self.primary_revision = Some((host.revision, host.outputs_revision));
                let displays = crate::host_displays::Displays {
                    outputs: host.outputs.clone(),
                    ..Default::default()
                };
                host.primary_ready = matches!(
                    displays.primary(host.windows, host.config.output_id()),
                    crate::host_displays::Primary::Ready
                );
                if let crate::host_displays::Primary::Select(id) =
                    displays.primary(host.windows, host.config.output_id())
                {
                    if self.primary_attempted.as_deref() != Some(id)
                        && self.confirmation.is_none()
                        && self.selection.is_none()
                    {
                        self.primary_attempted = Some(id.to_owned());
                        let frame = host.config.select_output(id)?;
                        send(frame)?;
                        self.confirmation =
                            Some(crate::host_video::Confirmation::output(id, Instant::now()));
                        host.pending = true;
                    }
                }
            }
        }
        let desired = session.bus.sample_requested();
        if (desired || self.hid_mouse) && !self.queried {
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
    pub display: String,
    pub display_hint: String,
    pub status: String,
    pub audio: crate::audio_pipewire::State,
    pub draft: crate::session_preferences::Video,
    pub pending: bool,
    pub result: String,
    pub displays: Vec<crate::host_displays::Output>,
    pub current_output: Option<String>,
    pub windows: bool,
    pub hid_mouse: bool,
    pub host_revision: u64,
    pub config_ready: bool,
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
            sample.audio_decoded_queue_ms = Some(report.queued_frames as f64 / 48.);
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
        let saved = session
            .settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|settings| settings.native_video())
            .unwrap_or_default();
        self.draft =
            crate::session_preferences::HostVideo::from_report(&observed).panel_video(saved);
        self.pending = host.pending;
        self.result = host.status.clone();
        self.displays = host.outputs.clone();
        self.current_output = host.config.output_id().map(str::to_owned);
        self.windows = host.windows;
        self.hid_mouse = host.hid_mouse;
        self.host_revision = host.revision;
        self.config_ready = host.config.ready();
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
        let sample = self.sample.as_ref().unwrap();
        let base = stats_view::Presentation::with_missing_values(
            sample,
            &self.values,
            (self.resources.cpu, None, None),
            None,
            &self.timing,
            true,
        );
        self.details = (0..5)
            .map(|tab| self.build_from(tab, base.clone()))
            .collect();
    }
    pub fn presentation(&self, tab: usize) -> Option<&Details> {
        self.details.get(tab)
    }
    #[cfg(test)]
    fn build(&self, tab: usize) -> Details {
        let sample = self.sample.as_ref().unwrap();
        self.build_from(
            tab,
            stats_view::Presentation::with_missing_values(
                sample,
                &self.values,
                (self.resources.cpu, None, None),
                None,
                &self.timing,
                true,
            ),
        )
    }
    fn build_from(&self, tab: usize, base: stats_view::Presentation<'_>) -> Details {
        let sample = self.sample.as_ref().unwrap();
        let mut result = base.select(stats_view::Tab::ALL[tab]);
        if tab == 2 {
            result.footer="Presentation rate counts scheduled Vulkan draw callbacks, not verified GPU submission or monitor scanout. Color depth and chroma describe the decoded stream and application output.".into();
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
        if tab == 3 {
            for (label, value) in &mut result.rows {
                if label == crate::app_style::forms::STAT_AUDIO_OUTPUT {
                    *value = if self.audio.ready {
                        "PipeWire / streaming"
                    } else if self.audio.recovering {
                        "PipeWire / recovering"
                    } else {
                        "PipeWire / not streaming"
                    }
                    .into();
                }
            }
            for (label, value) in &mut result.rows {
                if label == crate::app_style::forms::STAT_BUFFER_LIMITS {
                    if let Some((min, max, custom)) = self.audio.limits {
                        *value = format!(
                            "{min} / {max} ms{} (decoded PCM; device latency additional)",
                            if custom { " (custom)" } else { " (automatic)" }
                        );
                    }
                }
            }
            if let Some(layout) = self
                .audio
                .layout
                .filter(|layout| Some(*layout) != sample.audio_layout)
            {
                result.rows.push((
                    crate::app_style::forms::STAT_PLAYBACK_CHANNELS.into(),
                    layout.channels.to_string(),
                ));
            }
            result.rows.push((
                crate::app_style::forms::STAT_AUDIO_UNDERRUNS.into(),
                "Not collected by this backend".into(),
            ));
            result.rows.push((
                crate::app_style::forms::STAT_AUDIO_DEVICE_BUFFER.into(),
                "Not collected (device/graph latency is not sampled)".into(),
            ));
            result.rows.push((
                crate::app_style::forms::STAT_AUDIO_RECOVERIES.into(),
                format!(
                    "{}{}",
                    self.audio.recoveries,
                    if self.audio.recovering {
                        " (recovering)"
                    } else {
                        ""
                    }
                ),
            ));
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
        if tab == 4 {
            for (label, value) in &mut result.rows {
                if label == crate::app_style::forms::STAT_TIMING_SCOPE {
                    *value="CPU wall time including blocking driver waits; not GPU execution or scanout".into();
                }
            }
            if let Some(video) = sample.video.as_ref() {
                for (label, timing) in [
                    (
                        "Graphics completion wait",
                        video.native_graphics_wait.as_ref(),
                    ),
                    ("Decode completion wait", video.native_decode_wait.as_ref()),
                ] {
                    if let Some(timing) = timing.filter(|t| t.count > 0) {
                        result.rows.push((
                            label.into(),
                            format!(
                                "{:.2} / {:.2} / {:.2} ms (CPU wait: latest / mean / peak)",
                                timing.latest_ms, timing.average_ms, timing.peak_ms
                            ),
                        ));
                    }
                }
                result.rows.push((
                    "Compressed decoder queue".into(),
                    format!(
                        "{} packets / {} bytes; peak {} packets",
                        video.queue_depth, video.queue_bytes, video.queue_peak_depth
                    ),
                ));
            }
            result.footer="Local timings are CPU wall-clock durations and may include blocking driver/completion waits. Completion waits are not GPU execution measurements or monitor scanout. Host reports are not capture-to-display latency.".into();
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
    fn linux_overlay_receives_framed_host_settings_and_displays() {
        for mode in [Mode::Bud, Mode::WebRtc] {
            for terminated in [true, false] {
                let session = Session::new(mode);
                let mut control = Control::default();
                let config = r#"{"video":[{"hostOS":1,"output":"one","encoderFPS":60,"encoderMaxBitrate":10}]}"#;
                let outputs = r#"[{"id":"one","name":"Display 1","adapterName":"GPU"}]"#;
                for (id, json) in [(11, config), (12, outputs)] {
                    let frame = if terminated {
                        crate::control::text(17, id, json).unwrap()
                    } else {
                        let mut frame =
                            crate::control::header(17, json.len() as i32, id, 0).to_vec();
                        frame.extend_from_slice(json.as_bytes());
                        Bytes::from(frame)
                    };
                    control.receive(&frame, &session);
                }
                let host = session.host.lock().unwrap();
                assert!(host.config.ready());
                assert!(host.windows);
                assert_eq!(host.revision, 1);
                assert_eq!(host.outputs_revision, 1);
                assert_eq!(host.outputs[0].id, "one");
                drop(host);
                // Invalid JSON and truncated frames must preserve the last good state.
                control.receive(&crate::control::text(17, 11, "{}").unwrap(), &session);
                control.receive(&crate::control::header(17, 1000, 12, 0), &session);
                let host = session.host.lock().unwrap();
                assert_eq!(host.revision, 1);
                assert_eq!(host.outputs_revision, 1);
            }
        }
    }
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

#[cfg(test)]
mod command_tests {
    use super::*;
    #[test]
    fn linux_overlay_apply_preserves_other_streams_and_invalid_requests_do_not_disconnect() {
        let session = Session::new(Mode::Bud);
        let mut controls = Control::default();
        let initial = serde_json::json!({"video":[{"hostOS":1,"output":"one","resolutionX":1920,"resolutionY":1080,"encoderFPS":60,"encoderMaxBitrate":20,"fullFPS":true,"rotated":false},{"output":"two","encoderFPS":30}]});
        session
            .host
            .lock()
            .unwrap()
            .config
            .receive(initial.to_string().as_bytes())
            .unwrap();
        let mut sent = Vec::new();
        let video = crate::session_preferences::Video {
            fps: Some(30),
            bandwidth_mbps: Some(10),
            constant_fps: Some(false),
            rotated: Some(false),
            ..Default::default()
        };
        controls
            .command(
                crate::session_controls::Command::Apply(video),
                &session,
                (1920, 1080),
                |b| {
                    sent.push(b);
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(sent.len(), 1);
        let written: serde_json::Value =
            serde_json::from_slice(&sent[0][13..sent[0].len() - 1]).unwrap();
        assert_eq!(written["video"][1], initial["video"][1]);
        assert_eq!(written["video"][0]["encoderFPS"], 30);
        assert!(session.host.lock().unwrap().pending);
        session
            .host
            .lock()
            .unwrap()
            .config
            .receive(written.to_string().as_bytes())
            .unwrap();
        controls.confirm(&session, |_| Ok(()));
        assert!(!session.host.lock().unwrap().pending);
        let invalid = crate::session_preferences::Video {
            fps: Some(1000),
            ..Default::default()
        };
        controls
            .command(
                crate::session_controls::Command::Apply(invalid),
                &session,
                (1, 1),
                |_| panic!("invalid request reached transport"),
            )
            .unwrap();
        assert!(!session.host.lock().unwrap().status.is_empty());
        assert!(!session.host.lock().unwrap().pending);
    }
    #[test]
    fn linux_overview_retains_all_four_cards_for_bud_and_records_unknown_as_unknown() {
        let mut view = View::default();
        let mut sample = stats::Sample {
            state: "connected".into(),
            ..Default::default()
        };
        sample.bud = Some(Default::default());
        view.sample = Some(sample);
        let details = view.build(0);
        assert_eq!(details.cards.len(), 4);
        assert!(details.rows.iter().any(|(label, value)| label
            == crate::app_style::forms::STAT_GPU_BUSIEST_ENGINE
            && value.contains("Not collected")));
    }
}
