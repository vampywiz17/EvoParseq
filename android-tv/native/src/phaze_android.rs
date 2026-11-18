//! MediaCodec/AAudio/D-pad platform binding for the shared Phaze session.
use crate::{account, audio_android, phaze_client, stream::Request};
use anyhow::Result;
use evoparseq_phaze::{ClientControl, Input};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

pub fn run(request: Request, stop: &AtomicBool, status: &Mutex<Value>) -> Result<()> {
    let Request {
        session,
        peer,
        prefs,
        window,
        choices,
        audio_inventory,
        controllers,
        ..
    } = request;
    let video = crate::video_android::Pipeline::start(window, choices)?;
    let mut audio = audio_android::Output::prepare(prefs.audio, &audio_inventory, false);
    let pcm = crate::audio_stream::DecodedPcm::stereo(prefs.audio.maximum());
    let receiver = pcm.receiver();
    audio.attach(receiver.clone())?;
    let config = prefs.phaze_video_config()?;
    let mut client = phaze_client::Client::new(
        Adapter {
            stop,
            status,
            video: &video,
            audio,
            receiver,
            inbox: &controllers,
            desktop: Default::default(),
            events: VecDeque::new(),
            cursor: Default::default(),
            last_pointer: 0,
            cursor_visible: true,
            started: Instant::now(),
            sampled: Instant::now(),
            received: 0,
            video_payload_bytes: 0,
            frames: 0,
            bandwidth_log: prefs.logging.then(BandwidthLog::default),
        },
        config,
        pcm,
        prefs.audio.pcm,
    );
    let network = crate::network::current();
    let result = account::phaze_runtime(async {
        let servers = if network.force_relay {
            Some(network.servers().await?)
        } else {
            None
        };
        let stopped = async {
            while !stop.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Ok(())
        };
        tokio::select! {
            biased;
            result = client.connect(session.token(), &peer, servers, Vec::new) => result,
            result = stopped => result,
        }
    });
    client.platform.cursor.pointer(0);
    video.stop();
    result
}

struct Adapter<'a> {
    stop: &'a AtomicBool,
    status: &'a Mutex<Value>,
    video: &'a crate::video_android::Pipeline,
    audio: audio_android::Output,
    receiver: crate::audio_stream::NativeReceiver,
    inbox: &'a crate::input_handoff::Inbox,
    desktop: phaze_client::DesktopInput,
    events: VecDeque<crate::input_handoff::Event>,
    cursor: crate::gamepad_android::Output,
    last_pointer: u64,
    cursor_visible: bool,
    started: Instant,
    sampled: Instant,
    received: u64,
    video_payload_bytes: u64,
    frames: u64,
    bandwidth_log: Option<BandwidthLog>,
}

#[derive(Default)]
struct BandwidthLog {
    baseline: Option<(Instant, u64)>,
    batches: u64,
    latest: Option<(f64, f64)>,
    peak_upper_mbps: Option<f64>,
}

impl BandwidthLog {
    fn feedback(&mut self, lower_rate: f64, upper_rate: f64) {
        let (lower, upper) = (
            lower_rate * 8.0 / 1_000_000.0,
            upper_rate * 8.0 / 1_000_000.0,
        );
        self.batches += 1;
        self.latest = Some((lower, upper));
        self.peak_upper_mbps = Some(self.peak_upper_mbps.unwrap_or(upper).max(upper));
    }

    fn sample(&mut self, now: Instant, payload_bytes: u64) {
        let (start, previous_bytes) = *self.baseline.get_or_insert((now, payload_bytes));
        let elapsed = now.duration_since(start);
        if elapsed < Duration::from_secs(5) {
            return;
        }
        let video_mbps = payload_bytes.saturating_sub(previous_bytes) as f64 * 8.0
            / elapsed.as_secs_f64()
            / 1_000_000.0;
        crate::event_log::record(
            "INFO",
            "phaze_android_bandwidth",
            json!({
                "window_ms": elapsed.as_millis() as u64,
                "video_payload_mbps": video_mbps,
                "feedback_batches": self.batches,
                "latest_feedback_lower_mbps": self.latest.map(|(lower, _)| lower),
                "latest_feedback_upper_mbps": self.latest.map(|(_, upper)| upper),
                "peak_feedback_upper_mbps": self.peak_upper_mbps,
            }),
        );
        *self = Self {
            baseline: Some((now, payload_bytes)),
            ..Self::default()
        };
    }
}

impl phaze_client::Platform for Adapter<'_> {
    fn bandwidth_feedback_sent(&mut self, lower_rate: f64, upper_rate: f64) {
        if let Some(log) = &mut self.bandwidth_log {
            log.feedback(lower_rate, upper_rate);
        }
    }

    fn cursor_visibility(&mut self, visible: bool) {
        self.cursor_visible = visible;
    }

    fn video(&mut self, bytes: &[u8], info: crate::video_stream::PacketInfo) -> Result<()> {
        self.video.submit(bytes, info);
        Ok(())
    }
    fn reset_video(&mut self) {
        self.video.encoder_rebuilt();
    }
    fn take_recovery(&mut self) -> bool {
        self.video.take_keyframe_request()
    }

    fn poll(
        &mut self,
        state: &mut phaze_client::State,
        _: Option<u32>,
        input: &mut Vec<Input>,
        _: &mut Vec<ClientControl>,
    ) -> Result<bool> {
        if self.stop.load(Ordering::Acquire) {
            return Ok(false);
        }
        let now = Instant::now();
        let video = self.video.snapshot();
        if let Some(error) = &video.error {
            anyhow::bail!("{error}");
        }
        anyhow::ensure!(
            video.submitted > 0 || now.duration_since(self.started) < Duration::from_secs(30),
            "No Phaze video output received within 30 seconds"
        );
        let size = (video.width, video.height);
        self.desktop.engine.geometry(size);
        self.inbox.take(&mut self.events)?;
        while let Some(event) = self.events.pop_front() {
            if let crate::input_handoff::Event::Desktop(event) = event {
                self.desktop.event(event, size, input)?;
            }
        }
        self.desktop.tick(size, input)?;
        let pointer = if self.cursor_visible {
            self.desktop.engine.pointer()
        } else {
            0
        };
        if pointer != self.last_pointer {
            self.cursor.pointer(pointer);
            self.last_pointer = pointer;
        }
        self.audio.tick(now);
        let elapsed = now.duration_since(self.sampled);
        if elapsed >= Duration::from_millis(500) {
            let rate = state.received.saturating_sub(self.received) as f64 * 8.
                / elapsed.as_secs_f64()
                / 1_000_000.;
            let fps = video.submitted.saturating_sub(self.frames) as f64 / elapsed.as_secs_f64();
            let rtt = state.rtt.map(|rtt| rtt.as_secs_f64() * 1000.);
            let mut overlay = crate::overlay::metrics(
                Some(fps),
                rtt,
                rate,
                video.codec,
                video.width,
                video.height,
                video.depth,
            );
            let payload_bytes = state.stream.payload_bytes();
            if let (Some(log), Some(bytes)) = (&mut self.bandwidth_log, payload_bytes) {
                log.sample(now, bytes);
            }
            let payload_mbps = payload_bytes.map(|bytes| {
                bytes.saturating_sub(self.video_payload_bytes) as f64 * 8.0
                    / elapsed.as_secs_f64()
                    / 1_000_000.0
            });
            if let Some(sent) = state.stream.sent() {
                overlay["video_request"] =
                    format!("Requested max {} Mbit/s · {} FPS", sent.mbps, sent.fps).into();
            }
            if let Some(mbps) = payload_mbps {
                overlay["video_payload_rate"] = format!("{mbps:.1} Mbit/s video").into();
            }
            *self.status.lock().unwrap_or_else(|e| e.into_inner()) = json!({
                "state": if video.submitted > 0 { "Streaming" } else { "Connecting…" },
                "ended": false, "video": video, "audio": self.receiver.snapshot(),
                "audio_output": self.audio.snapshot(), "overlay": overlay,
                "network": {"transport":"Phaze","carrier":state.carrier,"route":state.route,"rtt_ms":rtt,"received_mbps":rate,"received_bytes":[state.received],"sent_bytes":[state.sent]},
            });
            self.sampled = now;
            self.received = state.received;
            self.video_payload_bytes = payload_bytes.unwrap_or(0);
            self.frames = video.submitted;
        }
        Ok(true)
    }
}
