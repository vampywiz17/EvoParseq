//! Presentation of existing telemetry; no sampling, window APIs or video work.
use crate::stats::{MemoryUsage, Sample, Values};

/// Empty values denote section headings; all units/text are decided here.
pub type Row = (String, String);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Overview,
    Network,
    Video,
    Audio,
    Timing,
}
impl Tab {
    pub const ALL: [Self; 5] = [
        Self::Overview,
        Self::Network,
        Self::Video,
        Self::Audio,
        Self::Timing,
    ];
    pub fn caption(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Network => "Network",
            Self::Video => "Video",
            Self::Audio => "Audio",
            Self::Timing => "Timings",
        }
    }
    fn section(label: &str) -> Self {
        match label {
            crate::app_style::forms::STAT_NETWORK
            | crate::app_style::forms::STAT_BUD_DELIVERY
            | crate::app_style::forms::STAT_SECURITY => Self::Network,
            crate::app_style::forms::STAT_VIDEO => Self::Video,
            crate::app_style::forms::STAT_AUDIO => Self::Audio,
            crate::app_style::forms::STAT_LOCAL_VIDEO_TIMING
            | crate::app_style::forms::STAT_HOST_TIMING_SECTION => Self::Timing,
            _ => Self::Overview,
        }
    }
}

pub struct Presentation<'a> {
    pub status: &'a str,
    pub cards: Vec<(&'static str, String)>,
    pub rows: Vec<Row>,
    pub footer: String,
}
impl<'a> Presentation<'a> {
    /// Shared categorization; changing tabs never changes sampling or backend state.
    pub fn select(mut self, tab: Tab) -> Self {
        let mut section = Tab::Overview;
        self.rows.retain(|(label, value)| {
            if value.is_empty() {
                section = Tab::section(label);
            }
            section == tab
                || (tab == Tab::Overview
                    && match section {
                        Tab::Network => matches!(
                            label.as_str(),
                            crate::app_style::forms::STAT_TRANSPORT
                                | crate::app_style::forms::STAT_ROUTE
                        ),
                        Tab::Video => matches!(
                            label.as_str(),
                            crate::app_style::forms::STAT_CODEC
                                | crate::app_style::forms::STAT_RESOLUTION
                                | crate::app_style::forms::STAT_PRESENT_SUBMISSIONS
                        ),
                        _ => false,
                    })
        });
        if tab != Tab::Overview {
            self.cards.clear();
        }
        self.footer = match tab {
            Tab::Overview => "Use the tabs for connection, video, audio and timing details. An unavailable measurement is unknown, not zero.".into(),
            Tab::Network => self.footer,
            Tab::Video => "Presentation submissions are accepted by the renderer, not confirmed monitor scanout. Reported color depth and chroma describe the received stream and application output.".into(),
            Tab::Audio => "Audio queues and drop counters describe audio frames, not video frames. Recovery counters track local audio-output reopening.".into(),
            Tab::Timing => "Local timings are CPU wall time, excluding GPU completion and monitor scanout. Recent averages cover up to two seconds; an em dash means no fresh samples. Local session means/peaks are retained. Host averages/peaks cover the current diagnostic collection; host reports are not capture-to-display latency.".into(),
        };
        if self.rows.is_empty() {
            self.rows
                .push(("Details".into(), "Waiting for relevant telemetry".into()));
        }
        self
    }

    pub fn new(
        sample: &'a Sample,
        values: &Values,
        usage: (Option<f64>, Option<f64>, Option<f64>),
        memory: Option<MemoryUsage>,
        timing: &crate::frame_timing::Snapshot,
    ) -> Self {
        let stale = sample.at.elapsed().as_secs() > 4 && sample.state == "connected";
        let status = if stale {
            "Waiting for fresh telemetry"
        } else {
            &sample.state
        };
        let cards = [
            ("Connection RTT", number(sample.rtt_ms, "ms")),
            ("Decoded video", number(values.fps, "fps")),
            ("Incoming traffic", number(values.incoming, "Mbps")),
            ("Outgoing traffic", number(values.outgoing, "Mbps")),
        ];
        #[cfg(all(feature = "bud", not(target_os = "linux")))]
        let mut cards: Vec<_> = if sample.bud.is_some() {
            cards
                .into_iter()
                .filter(|(_, value)| value != "—")
                .collect()
        } else {
            cards.into()
        };
        #[cfg(any(not(feature = "bud"), target_os = "linux"))]
        let mut cards: Vec<_> = cards.into();
        // Keep the existing policy: BUD hides unavailable cards, then stale
        // telemetry replaces values in the remaining cards with an em dash.
        if stale {
            for (_, value) in &mut cards {
                value.clear();
                value.push('—');
            }
        }
        let mut rows = rows(sample, values, usage);
        if let Some(memory) = memory {
            for (label, bytes) in [
                (
                    crate::app_style::forms::STAT_MEMORY_RESIDENT_TOTAL,
                    Some(memory.working_set_bytes),
                ),
                (
                    crate::app_style::forms::STAT_MEMORY_RESIDENT_PRIVATE,
                    memory.private_working_set_bytes,
                ),
                (
                    crate::app_style::forms::STAT_MEMORY_COMMITTED_PRIVATE,
                    Some(memory.private_committed_bytes),
                ),
            ] {
                if let Some(bytes) = bytes {
                    rows.push((
                        label.into(),
                        format!("{:.1} MiB", bytes as f64 / 1_048_576.),
                    ));
                }
            }
        }
        rows.extend(host_timing_rows(timing));
        #[cfg(feature = "bud")]
        let footer = if sample.bud.is_some() {
            "Traffic counts UDP payload bytes or TURN stream plaintext accepted for send or received, including BUD/crypto/TURN control and retransmissions. Stream framing padding is included; TLS, IP and TCP/UDP headers and account HTTPS/WSS traffic are excluded. RTT is the native transport estimate. Host encode time keeps the last available report. Late fragments indicate reordering or retransmission, not proven packet loss. NACKs request missing fragments. Compressed backlog is before decoding; no GPU timings are inferred."
        } else {
            WEBRTC_FOOTER
        };
        #[cfg(not(feature = "bud"))]
        let footer = WEBRTC_FOOTER;
        let footer = if timing.requested {
            format!("{footer} Host timing averages and peaks cover the current diagnostic collection. These are host durations, not capture-to-display latency or GPU execution times.")
        } else {
            footer.to_owned()
        };
        Self {
            status,
            cards,
            rows,
            footer,
        }
    }
}
const WEBRTC_FOOTER: &str = "Traffic is measured usage, not available bandwidth. RTT is the active ICE path's round-trip time, not ICMP ping. Local dropped frames are not network packet loss. Missing values remain unknown.";

fn number(value: Option<f64>, unit: &str) -> String {
    value
        .filter(|v| v.is_finite())
        .map(|v| format!("{v:.1} {unit}"))
        .unwrap_or_else(|| "—".into())
}
fn reported(s: &str) -> String {
    if s.is_empty() {
        "Not reported".into()
    } else {
        s.into()
    }
}
fn host_timing_rows(s: &crate::frame_timing::Snapshot) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    if !s.requested && !s.request_failed {
        return rows;
    }
    rows.push((
        crate::app_style::forms::STAT_HOST_TIMING_SECTION.into(),
        String::new(),
    ));
    if s.request_failed {
        rows.push((
            crate::app_style::forms::STAT_HOST_TIMING.into(),
            "Unavailable (request failed)".into(),
        ));
        return rows;
    }
    rows.push((
        crate::app_style::forms::STAT_TIMING_VALUES.into(),
        "Latest / average / peak (ms)".into(),
    ));
    let value = |series: &crate::frame_timing::Series, i: usize| {
        let n = |v: f64| {
            if v < 1_000_000. {
                format!("{v:.2}")
            } else {
                format!("{v:.2e}")
            }
        };
        format!(
            "{} / {} / {}",
            n(series.last[i]),
            n(series.mean[i]),
            n(series.peak[i])
        )
    };
    let age = |series: &crate::frame_timing::Series| {
        let seconds = series.at.elapsed().as_secs_f64();
        if seconds > 3. {
            format!("Stale ({seconds:.1} s old)")
        } else {
            format!("{seconds:.1} s")
        }
    };
    if s.video.iter().all(Option::is_none) {
        rows.push((
            crate::app_style::forms::STAT_VIDEO_TIMING.into(),
            "Not reported by host".into(),
        ));
    }
    for (stream, series) in s.video.iter().enumerate() {
        let Some(series) = series else {
            continue;
        };
        let prefix = if stream == 0 {
            String::new()
        } else {
            format!("Stream {stream} ")
        };
        for (i, label) in [
            "Video processing",
            "Capture segment",
            "Host encode",
            "Video frame interval",
        ]
        .into_iter()
        .enumerate()
        {
            rows.push((format!("{prefix}{label}"), value(series, i)));
        }
        rows.push((
            format!("{prefix}Video timing reports"),
            series.count.to_string(),
        ));
        rows.push((format!("{prefix}Video report age"), age(series)));
    }
    if let Some(series) = &s.audio {
        rows.push((
            crate::app_style::forms::STAT_AUDIO_PROCESSING.into(),
            value(series, 0),
        ));
        rows.push((
            crate::app_style::forms::STAT_AUDIO_BLOCK_INTERVAL.into(),
            value(series, 2),
        ));
        // The intermediate host slot is not a complete per-peer codec timing.
        rows.push((
            crate::app_style::forms::STAT_AUDIO_TIMING_REPORTS.into(),
            series.count.to_string(),
        ));
        rows.push((
            crate::app_style::forms::STAT_AUDIO_REPORT_AGE.into(),
            age(series),
        ));
    } else {
        rows.push((
            crate::app_style::forms::STAT_AUDIO_TIMING.into(),
            "Not reported by host".into(),
        ));
    }
    if s.rejected > 0 {
        rows.push((
            crate::app_style::forms::STAT_INVALID_TIMING_REPORTS.into(),
            s.rejected.to_string(),
        ));
    }
    if s.unknown > 0 {
        rows.push((
            crate::app_style::forms::STAT_UNKNOWN_TIMING_FORMATS.into(),
            s.unknown.to_string(),
        ));
    }
    rows
}
fn webrtc_network_rows(s: &Sample, row: &mut impl FnMut(&str, String)) {
    row(crate::app_style::forms::STAT_NETWORK, String::new());
    row(
        crate::app_style::forms::STAT_ROUTE,
        if s.state == "connected" && s.at.elapsed().as_secs() > 4 {
            "Unverified (telemetry unavailable)".into()
        } else {
            s.route.into()
        },
    );
    row(
        crate::app_style::forms::STAT_LOCAL_CANDIDATE,
        reported(&s.local),
    );
    row(
        crate::app_style::forms::STAT_REMOTE_CANDIDATE,
        reported(&s.remote),
    );
    row(
        crate::app_style::forms::STAT_TRANSPORT,
        reported(&s.protocol),
    );
    row(
        crate::app_style::forms::STAT_TURN_SERVER_IN_USE,
        s.turn_server.clone().unwrap_or_else(|| {
            if s.route == "Direct — no TURN" {
                "None".into()
            } else {
                "Not reported".into()
            }
        }),
    );
    row(
        crate::app_style::forms::STAT_TURN_TRANSPORT,
        s.turn_protocol
            .clone()
            .unwrap_or_else(|| "Not reported".into()),
    );
}
fn webrtc_security_rows(s: &Sample, row: &mut impl FnMut(&str, String)) {
    row(crate::app_style::forms::STAT_SECURITY, String::new());
    row(crate::app_style::forms::STAT_DTLS_STATE, s.dtls.clone());
    row(
        crate::app_style::forms::STAT_ENCRYPTION,
        if s.dtls == "connected" {
            "DTLS protects SCTP data channels".into()
        } else {
            "Not reported".into()
        },
    );
}
fn rows(
    s: &Sample,
    v: &Values,
    cpu: (Option<f64>, Option<f64>, Option<f64>),
) -> Vec<(String, String)> {
    let values = v;
    let recent_fresh = s.at.elapsed() <= crate::stats_recent::WINDOW;
    let mut rows = Vec::new();
    let mut row = |a: &str, b: String| rows.push((a.into(), b));
    #[cfg(feature = "bud")]
    if let Some(bud) = &s.bud {
        row(crate::app_style::forms::STAT_NETWORK, String::new());
        row(crate::app_style::forms::STAT_TRANSPORT, "BUD / UDP".into());
        if bud.connected {
            row(
                crate::app_style::forms::STAT_ROUTE,
                if bud.relay { "TURN relay" } else { "Direct" }.into(),
            );
        }
        if let Some(carrier) = bud.relay_transport.filter(|_| bud.relay) {
            row(
                crate::app_style::forms::STAT_TURN_TRANSPORT,
                carrier.to_uppercase(),
            );
        }
        row(
            crate::app_style::forms::STAT_HOST_ENCODE_TIME,
            bud.encode_ms
                .map(|encode| number(Some(encode), "ms"))
                .unwrap_or_else(|| "Waiting for host".into()),
        );
        row(crate::app_style::forms::STAT_BUD_DELIVERY, String::new());
        for (name, channel) in ["Control", "Video", "Audio"].iter().zip(&bud.channels) {
            row(&format!("{name} late fragments"), channel.late.to_string());
            row(&format!("{name} NACK requests"), channel.nacks.to_string());
        }
        row(
            crate::app_style::forms::STAT_DUPLICATE_FRAGMENTS,
            bud.channels
                .iter()
                .map(|c| c.duplicates)
                .sum::<u64>()
                .to_string(),
        );
        row(
            crate::app_style::forms::STAT_OUTSIDE_RECEIVE_WINDOW,
            bud.channels
                .iter()
                .map(|c| c.window_drops)
                .sum::<u64>()
                .to_string(),
        );
        row(
            crate::app_style::forms::STAT_COMPRESSED_VIDEO_BACKLOG,
            format!(
                "{} messages / {} ms",
                bud.video_backlog, bud.video_backlog_ms
            ),
        );
        row(
            crate::app_style::forms::STAT_SKIPPED_ENCODED_PICTURES,
            bud.skipped_video.to_string(),
        );
        row(
            crate::app_style::forms::STAT_AUDIO_RECEIVE_POOL_DROPS,
            bud.audio_pool_drops.to_string(),
        );
        row(
            crate::app_style::forms::STAT_INPUT_QUEUE_DROPS,
            bud.input_drops.to_string(),
        );
    } else {
        webrtc_network_rows(s, &mut row);
    }
    #[cfg(not(feature = "bud"))]
    webrtc_network_rows(s, &mut row);
    row(crate::app_style::forms::STAT_VIDEO, String::new());
    let video = s.video.as_ref();
    row(
        crate::app_style::forms::STAT_CODEC,
        s.codec
            .or_else(|| video.and_then(|x| x.codec))
            .map(|c| c.name())
            .unwrap_or("Not reported")
            .into(),
    );
    row(
        crate::app_style::forms::STAT_PROFILE,
        s.profile
            .map(|p| {
                if s.codec == Some(crate::video_stream::Codec::Hevc) {
                    match p {
                        1 => "Main".into(),
                        2 => "Main 10".into(),
                        4 => "Range extensions".into(),
                        _ => format!("HEVC profile {p}"),
                    }
                } else {
                    match p {
                        66 => "Baseline".into(),
                        77 => "Main".into(),
                        100 => "High".into(),
                        _ => format!("H.264 profile {p}"),
                    }
                }
            })
            .unwrap_or_else(|| "Not reported".into()),
    );
    row(
        crate::app_style::forms::STAT_HEVC_ADVERTISED,
        if s.hevc_advertised { "Yes" } else { "No" }.into(),
    );
    row(
        crate::app_style::forms::STAT_RESOLUTION,
        video
            .and_then(|x| x.width.zip(x.height))
            .map(|(w, h)| format!("{w} × {h}"))
            .unwrap_or_else(|| "Not reported".into()),
    );
    row(
        crate::app_style::forms::STAT_DECODER,
        video
            .and_then(|x| x.decoder)
            .unwrap_or("Not reported")
            .into(),
    );
    if video.is_some_and(|v| v.decoder_fallback.is_some()) {
        row(
            crate::app_style::forms::STAT_DECODER_FALLBACK,
            "Compatibility decoder in use".into(),
        );
    }
    if let Some(depth) = video.and_then(|v| v.decoded_bit_depth) {
        row(
            crate::app_style::forms::STAT_DECODED_COLOR_DEPTH,
            format!("{depth}-bit"),
        );
    }
    if let Some(chroma) = video.and_then(|v| v.decoded_chroma) {
        row(crate::app_style::forms::STAT_DECODED_CHROMA, chroma.into());
    }
    if let Some(depth) = video.and_then(|v| v.display_rgb_bit_depth) {
        row(
            crate::app_style::forms::STAT_RGB_OUTPUT,
            format!("{depth}-bit SDR (application output)"),
        );
    }
    if video.is_some_and(|v| v.display_output_fallback.is_some()) {
        row(
            crate::app_style::forms::STAT_RGB_OUTPUT_FALLBACK,
            "Using compatible 8-bit output".into(),
        );
    }
    row(
        crate::app_style::forms::STAT_RENDERER,
        video
            .and_then(|x| x.renderer)
            .unwrap_or("Not reported")
            .into(),
    );
    row(
        crate::app_style::forms::STAT_GPU_ADAPTER,
        video
            .and_then(|x| x.adapter.clone())
            .unwrap_or_else(|| "Not reported".into()),
    );
    if let Some(status) = video.and_then(|v| v.zero_copy_status.as_deref()) {
        row(
            crate::app_style::forms::STAT_ZERO_COPY_EXPERIMENTAL,
            if status.starts_with("Original NV12") || status.starts_with("Original P010") {
                "Active"
            } else if status.starts_with("Starting") {
                "Starting"
            } else {
                "Unavailable; using normal GPU rendering"
            }
            .into(),
        );
        if status.starts_with("Original P010") {
            row(
                crate::app_style::forms::STAT_DISPLAY_OUTPUT,
                "Windows-managed; final color depth not verified".into(),
            );
        }
    }
    if video.is_some_and(|v| v.recovering) {
        row(
            crate::app_style::forms::STAT_VIDEO_STATUS,
            "Recovering; waiting for a new keyframe.".into(),
        );
    }
    if let Some(stage) = video.and_then(|x| x.failure_stage) {
        row(
            crate::app_style::forms::STAT_VIDEO_STATUS,
            match stage {
                "video-selected-gpu-unavailable-select-automatic-in-settings" => {
                    "Selected GPU unavailable; reselect in Settings.".into()
                }
                "video-d3d11-device" | "video-decoder-not-d3d11-aware" => {
                    "Video GPU unavailable or unsupported; try another GPU.".into()
                }
                _ => "Playback stopped. Reconnect or select another GPU.".into(),
            },
        );
    }
    row(
        crate::app_style::forms::STAT_HARDWARE_DECODE,
        video
            .map(|x| match x.hardware_decode_status() {
                "Likely (D3D11 decoder surface)" => "Likely (GPU decoder output)",
                status => status,
            })
            .unwrap_or("Not reported")
            .into(),
    );
    row(
        crate::app_style::forms::STAT_DECODER_OUTPUT,
        video
            .and_then(|x| {
                x.native_decoder_surface.map(str::to_owned).or_else(|| {
                    x.d3d11_decoder_surface.map(|surface| {
                        if surface {
                            "D3D11 decoder surface".into()
                        } else {
                            "DXGI surface".into()
                        }
                    })
                })
            })
            .unwrap_or_else(|| "Not reported".into())
            .into(),
    );
    row(
        crate::app_style::forms::STAT_LOCAL_DROPPED_FRAMES,
        video
            .map(|x| x.frames_dropped.to_string())
            .unwrap_or_else(|| "Not reported".into()),
    );
    if let Some(v) = video {
        if let Some(status) = v.vsr_status {
            row(
                crate::app_style::forms::STAT_VIDEO_AI_UPSCALING,
                status.into(),
            );
        }
        if let Some(mode) = v.presentation_mode {
            row(crate::app_style::forms::STAT_DISPLAY_TIMING, mode.into());
        }
        row(
            crate::app_style::forms::STAT_DECODED_VIDEO_RATE,
            number(values.fps.filter(|_| recent_fresh), "fps"),
        );
        row(
            crate::app_style::forms::STAT_PRESENT_SUBMISSIONS,
            number(values.present_fps.filter(|_| recent_fresh), "fps"),
        );
        row(
            crate::app_style::forms::STAT_PRESENTATION_RATE_SCOPE,
            "Accepted submissions; not monitor scanout".into(),
        );
        if v.local_timing.decoder_service.count > 0 {
            row(
                crate::app_style::forms::STAT_LOCAL_VIDEO_TIMING,
                String::new(),
            );
            row(
                crate::app_style::forms::STAT_CPU_TIMING_VALUES,
                "Latest / recent avg / session avg / peak (ms)".into(),
            );
            row(
                crate::app_style::forms::STAT_RECENT_TIMING_WINDOW,
                "Up to 2 s; — means no new samples".into(),
            );
            for (i, (name, value)) in v.local_timing.stages().into_iter().enumerate() {
                row(
                    name,
                    if value.count == 0 {
                        "Not reported".into()
                    } else {
                        format!(
                            "{:.2} / {} / {:.2} / {:.2}",
                            value.latest_ms,
                            values.recent_timing_ms[i]
                                .filter(|_| recent_fresh)
                                .map_or_else(|| "—".into(), |v| format!("{v:.2}")),
                            value.average_ms,
                            value.peak_ms
                        )
                    },
                );
            }
            row(
                crate::app_style::forms::STAT_DISPLAY_CATCH_UP_SKIPS,
                v.display_updates_skipped.to_string(),
            );
            if v.adaptive_display_catch_up {
                row(
                    crate::app_style::forms::STAT_ADAPTIVE_CATCH_UP_ACTIVATIONS,
                    v.adaptive_display_catch_up_activations.to_string(),
                );
            }
            row(
                crate::app_style::forms::STAT_TIMING_SCOPE,
                "CPU wall time; excludes GPU completion and scanout".into(),
            );
        }
    }
    row(crate::app_style::forms::STAT_AUDIO, String::new());
    for (name, value) in [
        (
            crate::app_style::forms::STAT_CODEC,
            s.audio_codec.unwrap_or("Opus"),
        ),
        (crate::app_style::forms::STAT_SAMPLE_RATE, "48,000 Hz"),
    ] {
        row(
            name,
            if s.audio_ready {
                value.into()
            } else {
                "Not reported".into()
            },
        );
    }
    row(
        crate::app_style::forms::STAT_CHANNELS,
        if s.audio_ready {
            s.audio_layout.map_or_else(
                || "2 (stereo)".into(),
                |layout| match layout.channels {
                    6 => "6 (5.1)".into(),
                    8 => "8 (7.1)".into(),
                    _ => "2 (stereo)".into(),
                },
            )
        } else {
            "Not reported".into()
        },
    );
    row(
        crate::app_style::forms::STAT_MEASURED_AUDIO_TRAFFIC,
        number(v.audio, "kbps"),
    );
    #[cfg(windows)]
    if let Some(audio) = &s.audio_playback {
        if let Some(layout) = audio
            .layout
            .filter(|layout| Some(*layout) != s.audio_layout)
        {
            row(
                crate::app_style::forms::STAT_PLAYBACK_CHANNELS,
                layout.channels.to_string(),
            );
        }
        row(
            crate::app_style::forms::STAT_PLAYBACK_BUFFER,
            number(
                Some((audio.queued_frames as f64 + audio.device_padding as f64) / 48.0),
                "ms",
            ),
        );
        row(
            crate::app_style::forms::STAT_BUFFER_LIMITS,
            format!(
                "{} / {} ms{}",
                audio.minimum_ms,
                audio.maximum_ms,
                if audio.custom_buffer {
                    " (custom)"
                } else {
                    " (automatic)"
                }
            ),
        );
        if audio.recovering || audio.recoveries > 0 || audio.failure_hresult.is_some() {
            row(
                crate::app_style::forms::STAT_AUDIO_OUTPUT,
                if audio.recovering {
                    "Reconnecting"
                } else if audio.failure_hresult.is_some() {
                    "Unavailable"
                } else {
                    "Ready"
                }
                .into(),
            );
            row(
                crate::app_style::forms::STAT_AUDIO_RECOVERIES,
                audio.recoveries.to_string(),
            );
        }
        row(
            crate::app_style::forms::STAT_AUDIO_UNDERRUNS,
            audio.underruns.to_string(),
        );
        row(
            crate::app_style::forms::STAT_AUDIO_DEVICE_BUFFER,
            number(Some(audio.device_capacity_frames as f64 / 48.0), "ms"),
        );
    }
    #[cfg(target_os = "linux")]
    if let Some(audio) = &s.audio_native {
        row(
            crate::app_style::forms::STAT_AUDIO_OUTPUT,
            if audio.worker_finished {
                "Stopped"
            } else {
                "PipeWire"
            }
            .into(),
        );
        row(
            crate::app_style::forms::STAT_PLAYBACK_BUFFER,
            format!(
                "{:.1} ms (application queue; excludes device latency)",
                audio.queued_samples as f64 / 96.
            ),
        );
        row(
            crate::app_style::forms::STAT_BUFFER_LIMITS,
            "100 ms maximum (automatic)".into(),
        );
        row(
            crate::app_style::forms::STAT_AUDIO_DROPS,
            format!("{} packets", audio.packets_dropped),
        );
        row("Audio decode errors", audio.decode_errors.to_string());
        row("Audio decoded packets", audio.packets_decoded.to_string());
        row("PCM frames consumed", audio.pcm_frames_consumed.to_string());
    }
    if let Some(ms) = s.audio_decoded_queue_ms {
        row(
            crate::app_style::forms::STAT_DECODED_AUDIO_WAITING,
            number(Some(ms), "ms"),
        );
    }
    #[cfg(feature = "bud")]
    if let Some(bud) = &s.bud {
        if let Some(cipher) = bud.cipher {
            row(crate::app_style::forms::STAT_SECURITY, String::new());
            row(
                crate::app_style::forms::STAT_MEDIA_ENCRYPTION,
                cipher.into(),
            );
        }
    } else {
        webrtc_security_rows(s, &mut row);
    }
    #[cfg(not(feature = "bud"))]
    webrtc_security_rows(s, &mut row);
    row(crate::app_style::forms::STAT_APP_PERFORMANCE, String::new());
    row(
        crate::app_style::forms::STAT_CPU_ALL_CORES,
        number(cpu.0, "%"),
    );
    row(
        crate::app_style::forms::STAT_GPU_BUSIEST_ENGINE,
        number(cpu.1, "%"),
    );
    #[cfg(feature = "bud")]
    if s.bud.is_some() {
        rows.retain(|(_, value)| value != "Not reported" && value != "—");
    }
    rows
}
pub fn compact_metrics(
    sample: Option<&crate::stats::Sample>,
    rates: &crate::stats::Values,
) -> String {
    let metric = |value: Option<f64>, unit: &str| {
        value
            .filter(|v| v.is_finite() && *v >= 0.0)
            .map_or_else(|| "N/A".into(), |v| format!("{v:.1}{unit}"))
    };
    let decode = sample
        .and_then(|s| s.video.as_ref())
        .map(|v| &v.local_timing.decoder_service)
        .filter(|v| v.count > 0)
        .map(|v| v.latest_ms);
    let encode = sample
        .and_then(|s| s.bud.as_ref())
        .and_then(|b| b.encode_ms)
        .or_else(|| sample.and_then(|s| s.host_encode_ms));
    format!(
        "Decode CPU {} | Encode {}\nRTT {} | RX {} | D/P {}/{} fps",
        metric(decode, "ms"),
        metric(encode, "ms"),
        metric(sample.and_then(|s| s.rtt_ms), "ms"),
        metric(rates.incoming, "Mbps"),
        metric(rates.fps, ""),
        metric(rates.present_fps, "")
    )
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_keep_every_detail_and_separate_video_audio_network_and_timings() {
        let mut sample = Sample {
            video: Some(crate::video_output::Snapshot {
                decoder_initialized: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        sample
            .video
            .as_mut()
            .unwrap()
            .local_timing
            .decoder_service
            .record(std::time::Duration::from_millis(1));
        let timing = crate::frame_timing::Shared::default().snapshot(0);
        let values = Values::default();
        let make = || {
            Presentation::new(
                &sample,
                &values,
                (Some(1.0), Some(2.0), None),
                None,
                &timing,
            )
        };
        let all = make().rows;
        let pages: Vec<_> = Tab::ALL.into_iter().map(|t| make().select(t)).collect();
        assert!(!pages[0].cards.is_empty());
        assert!(pages[1..].iter().all(|p| p.cards.is_empty()));
        for row in all {
            assert!(
                pages.iter().any(|p| p.rows.contains(&row)),
                "Missing row: {row:?}"
            );
        }
        assert!(pages[1]
            .rows
            .iter()
            .any(|(k, _)| k == crate::app_style::forms::STAT_SECURITY));
        assert!(!pages[2]
            .rows
            .iter()
            .any(|(k, _)| k == "Decoder API service"));
        assert!(pages[3]
            .rows
            .iter()
            .any(|(k, _)| k == crate::app_style::forms::STAT_SAMPLE_RATE));
        assert!(pages[4]
            .rows
            .iter()
            .any(|(k, _)| k == "Decoder API service"));
        assert!(pages[0]
            .rows
            .iter()
            .any(|(k, _)| k == crate::app_style::forms::STAT_CPU_ALL_CORES));
    }
    #[test]
    fn recent_timing_and_submission_rates_keep_scope_and_lifetime_values() {
        let mut sample = Sample {
            video: Some(crate::video_output::Snapshot {
                decoder_initialized: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        sample
            .video
            .as_mut()
            .unwrap()
            .local_timing
            .decoder_service
            .record(std::time::Duration::from_millis(9));
        let mut values = Values {
            fps: Some(60.0),
            present_fps: Some(30.0),
            ..Default::default()
        };
        values.recent_timing_ms[2] = Some(2.0);
        let view = Presentation::new(
            &sample,
            &values,
            (None, None, None),
            None,
            &Default::default(),
        );
        assert!(view.rows.contains(&(
            crate::app_style::forms::STAT_PRESENT_SUBMISSIONS.into(),
            "30.0 fps".into()
        )));
        assert!(view.rows.contains(&(
            "Decoder API service".into(),
            "9.00 / 2.00 / 9.00 / 9.00".into()
        )));
        assert!(view.rows.iter().any(|(k, v)| k
            == crate::app_style::forms::STAT_PRESENTATION_RATE_SCOPE
            && v.contains("not monitor scanout")));
        sample.at -= std::time::Duration::from_secs(3);
        let stale = Presentation::new(
            &sample,
            &values,
            (None, None, None),
            None,
            &Default::default(),
        );
        assert!(stale.rows.contains(&(
            "Decoder API service".into(),
            "9.00 / — / 9.00 / 9.00".into()
        )));
        assert!(stale.rows.contains(&(
            crate::app_style::forms::STAT_PRESENT_SUBMISSIONS.into(),
            "—".into()
        )));
    }
    #[test]
    fn cards_preserve_units_unknowns_and_stale_connection_policy() {
        let timing = crate::frame_timing::Shared::default().snapshot(0);
        let mut sample = Sample {
            state: "connected".into(),
            rtt_ms: Some(2.5),
            ..Default::default()
        };
        let values = Values {
            fps: Some(60.0),
            incoming: Some(f64::NAN),
            outgoing: Some(f64::INFINITY),
            ..Default::default()
        };
        let view = Presentation::new(&sample, &values, (None, None, None), None, &timing);
        assert_eq!(view.status, "connected");
        assert_eq!(
            view.cards,
            [
                ("Connection RTT", "2.5 ms".into()),
                ("Decoded video", "60.0 fps".into()),
                ("Incoming traffic", "—".into()),
                ("Outgoing traffic", "—".into()),
            ]
        );
        sample.at = std::time::Instant::now() - std::time::Duration::from_secs(6);
        let view = Presentation::new(&sample, &values, (None, None, None), None, &timing);
        assert_eq!(view.status, "Waiting for fresh telemetry");
        assert!(view.cards.iter().all(|(_, value)| value == "—"));
        assert!(view.rows.contains(&(
            crate::app_style::forms::STAT_ROUTE.into(),
            "Unverified (telemetry unavailable)".into()
        )));
        sample.state = "Disconnected".into();
        let view = Presentation::new(&sample, &values, (None, None, None), None, &timing);
        assert_eq!(view.status, "Disconnected");
        assert_eq!(view.cards[0].1, "2.5 ms");
    }
    #[cfg(feature = "bud")]
    #[test]
    fn bud_cards_hide_missing_values_before_blanking_stale_telemetry() {
        let timing = crate::frame_timing::Shared::default().snapshot(0);
        let mut sample = Sample {
            state: "connected".into(),
            rtt_ms: Some(10.0),
            bud: Some(evoparseq_bud::bud::Snapshot::default()),
            ..Default::default()
        };
        let values = Values {
            fps: Some(60.0),
            outgoing: Some(0.0),
            ..Default::default()
        };
        let view = Presentation::new(&sample, &values, (None, None, None), None, &timing);
        assert_eq!(
            view.cards
                .iter()
                .map(|(label, _)| *label)
                .collect::<Vec<_>>(),
            ["Connection RTT", "Decoded video", "Outgoing traffic"]
        );
        assert!(view.footer.contains("Stream framing padding is included"));
        assert!(view.footer.contains("not proven packet loss"));
        assert!(!view
            .rows
            .iter()
            .any(|(label, _)| label == crate::app_style::forms::STAT_DTLS_STATE));
        sample.at = std::time::Instant::now() - std::time::Duration::from_secs(6);
        let view = Presentation::new(&sample, &values, (None, None, None), None, &timing);
        assert_eq!(view.cards.len(), 3);
        assert!(view.cards.iter().all(|(_, value)| value == "—"));
    }
    #[test]
    fn memory_and_host_diagnostics_keep_order_units_and_scope_without_sampling() {
        let timing = crate::frame_timing::Shared::default();
        timing.begin(7);
        timing.set_requested(7, true);
        timing.receive(
            7,
            &crate::frame_timing::tests::packet(0, 0, &[12., 2., 4., 16.]),
        );
        let report = timing.snapshot(7);
        let sample = Sample::default();
        let memory = MemoryUsage {
            working_set_bytes: 3 * 1_048_576,
            private_committed_bytes: 5 * 1_048_576,
            private_working_set_bytes: None,
        };
        let view = Presentation::new(
            &sample,
            &Values::default(),
            (Some(2.0), Some(3.0), None),
            Some(memory),
            &report,
        );
        let position = |label: &str| view.rows.iter().position(|(key, _)| key == label).unwrap();
        assert!(
            position(crate::app_style::forms::STAT_APP_PERFORMANCE)
                < position(crate::app_style::forms::STAT_MEMORY_RESIDENT_TOTAL)
        );
        assert!(
            position(crate::app_style::forms::STAT_MEMORY_COMMITTED_PRIVATE)
                < position(crate::app_style::forms::STAT_HOST_TIMING_SECTION)
        );
        assert!(view.rows.contains(&(
            crate::app_style::forms::STAT_MEMORY_RESIDENT_TOTAL.into(),
            "3.0 MiB".into()
        )));
        assert!(view.rows.contains(&(
            crate::app_style::forms::STAT_CPU_ALL_CORES.into(),
            "2.0 %".into()
        )));
        assert!(!view
            .rows
            .iter()
            .any(|(label, _)| label == crate::app_style::forms::STAT_MEMORY_RESIDENT_PRIVATE));
        assert!(view
            .rows
            .contains(&("Host encode".into(), "4.00 / 4.00 / 4.00".into())));
        assert!(view.footer.contains("round-trip time, not ICMP ping"));
        assert!(view
            .footer
            .contains("not capture-to-display latency or GPU execution times"));
        assert!(!view.footer.contains("Stream framing padding"));
    }

    #[test]
    fn timing_rows_preserve_unknown_and_stale_values_and_do_not_invent_audio_encode_time() {
        let bus = crate::frame_timing::Shared::default();
        bus.begin(1);
        assert!(super::host_timing_rows(&bus.snapshot(1)).is_empty());
        bus.set_requested(1, true);
        let waiting = super::host_timing_rows(&bus.snapshot(1));
        assert!(waiting
            .iter()
            .any(|(k, v)| k == crate::app_style::forms::STAT_VIDEO_TIMING
                && v == "Not reported by host"));
        bus.receive(
            1,
            &crate::frame_timing::tests::packet(0, 0, &[12., 2., 4., 16.]),
        );
        bus.receive(
            1,
            &crate::frame_timing::tests::packet(0, 0, &[20., 3., 6., 32.]),
        );
        bus.receive(1, &crate::frame_timing::tests::packet(1, 0, &[8., 0., 20.]));
        let mut report = bus.snapshot(1);
        report.video[0].as_mut().unwrap().at =
            std::time::Instant::now() - std::time::Duration::from_secs(5);
        let rows = super::host_timing_rows(&report);
        assert!(rows
            .iter()
            .any(|(k, v)| k == "Host encode" && v == "6.00 / 5.00 / 6.00"));
        assert!(rows
            .iter()
            .any(|(k, v)| k == "Video frame interval" && v == "32.00 / 24.00 / 32.00"));
        assert!(rows
            .iter()
            .any(|(k, v)| k == "Video report age" && v.starts_with("Stale")));
        assert!(rows
            .iter()
            .any(|(k, v)| k == crate::app_style::forms::STAT_AUDIO_PROCESSING
                && v == "8.00 / 8.00 / 8.00"));
        assert!(!rows.iter().any(|(k, _)| k == "Audio encode"));
        bus.failed(1);
        assert!(super::host_timing_rows(&bus.snapshot(1))
            .iter()
            .any(|(_, v)| v.contains("request failed")));
    }
    #[test]
    fn rgb_output_precision_is_observed_separately_from_decoded_precision() {
        let mut sample = Sample {
            video: Some(crate::video_output::Snapshot {
                decoded_bit_depth: Some(10),
                ..Default::default()
            }),
            ..Default::default()
        };
        let displayed = rows(&sample, &Values::default(), (None, None, None));
        assert!(!displayed
            .iter()
            .any(|(k, _)| k == crate::app_style::forms::STAT_RGB_OUTPUT));
        let video = sample.video.as_mut().unwrap();
        video.display_rgb_bit_depth = Some(10);
        let displayed = rows(&sample, &Values::default(), (None, None, None));
        assert!(displayed
            .iter()
            .any(|(k, v)| k == crate::app_style::forms::STAT_RGB_OUTPUT
                && v.starts_with("10-bit SDR")));
        let video = sample.video.as_mut().unwrap();
        video.display_rgb_bit_depth = Some(8);
        video.display_output_fallback = Some("Driver rejected RGB10: 0x80070057".into());
        let displayed = rows(&sample, &Values::default(), (None, None, None));
        assert!(displayed
            .iter()
            .any(|(k, v)| k == crate::app_style::forms::STAT_DECODED_COLOR_DEPTH && v == "10-bit"));
        assert!(displayed
            .iter()
            .any(|(k, v)| k == crate::app_style::forms::STAT_RGB_OUTPUT
                && v.starts_with("8-bit SDR")));
        assert!(displayed
            .iter()
            .any(|(k, _)| k == crate::app_style::forms::STAT_RGB_OUTPUT_FALLBACK));
        assert!(!displayed.iter().any(|(_, v)| v.contains("0x80070057")));
    }

    #[test]
    fn playback_status_is_readable_without_exposing_internal_error_details() {
        let mut sample = Sample {
            video: Some(crate::video_output::Snapshot {
                decoder_fallback: Some("Intel GPU renderer fell behind; internal detail".into()),
                zero_copy_status: Some(
                    "Original NV12 surfaces; no app conversion pass. Scanout not verified.".into(),
                ),
                failure_stage: Some("video-vendor-process"),
                failure_hresult: Some("0x80004005".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let displayed = rows(&sample, &Values::default(), (None, None, None));
        assert!(displayed.iter().any(|(k, v)| k
            == crate::app_style::forms::STAT_ZERO_COPY_EXPERIMENTAL
            && v == "Active"));
        assert!(displayed
            .iter()
            .any(|(k, v)| k == crate::app_style::forms::STAT_DECODER_FALLBACK
                && v == "Compatibility decoder in use"));
        assert!(!displayed.iter().any(|(_, v)| v.contains("internal detail")
            || v.contains("Scanout")
            || v.contains("video-vendor-process")
            || v.contains("0x80004005")));
        // Detailed telemetry is retained; only its UI presentation changes.
        assert_eq!(
            sample.video.as_ref().unwrap().failure_hresult.as_deref(),
            Some("0x80004005")
        );
        let video = sample.video.as_mut().unwrap();
        video.zero_copy_status = Some(crate::video_output::original_surface_status(true).into());
        video.decoded_bit_depth = Some(10);
        let displayed = rows(&sample, &Values::default(), (None, None, None));
        assert!(displayed.iter().any(|(k, v)| k
            == crate::app_style::forms::STAT_ZERO_COPY_EXPERIMENTAL
            && v == "Active"));
        assert!(displayed
            .iter()
            .any(|(k, v)| k == crate::app_style::forms::STAT_DECODED_COLOR_DEPTH && v == "10-bit"));
        assert!(displayed
            .iter()
            .any(|(k, v)| k == crate::app_style::forms::STAT_DISPLAY_OUTPUT
                && v.contains("not verified")));
        assert!(!displayed
            .iter()
            .any(|(k, _)| k == crate::app_style::forms::STAT_RGB_OUTPUT));
        sample.video.as_mut().unwrap().zero_copy_status =
            Some("Fallback: unsupported allocation".into());
        let displayed = rows(&sample, &Values::default(), (None, None, None));
        assert!(displayed.iter().any(|(k, v)| k
            == crate::app_style::forms::STAT_ZERO_COPY_EXPERIMENTAL
            && v == "Unavailable; using normal GPU rendering"));
    }

    #[cfg(feature = "bud")]
    #[test]
    fn bud_statistics_use_native_counters_and_hide_ice_dtls_and_unknowns() {
        let mut native = evoparseq_bud::bud::Snapshot {
            connected: true,
            relay: true,
            relay_transport: Some("tls"),
            cipher: Some("AES-256-GCM"),
            ..Default::default()
        };
        native.channels[1].late = 7;
        native.channels[1].nacks = 2;
        let sample = Sample {
            bud: Some(native),
            dtls: "connected".into(),
            ..Default::default()
        };
        let rows = rows(&sample, &Values::default(), (None, None, None));
        assert!(rows
            .iter()
            .any(|(k, v)| k == "Video late fragments" && v == "7"));
        assert!(rows.iter().any(
            |(k, v)| k == crate::app_style::forms::STAT_MEDIA_ENCRYPTION && v == "AES-256-GCM"
        ));
        assert!(rows
            .iter()
            .any(|(k, v)| k == crate::app_style::forms::STAT_TURN_TRANSPORT && v == "TLS"));
        assert!(!rows
            .iter()
            .any(|(k, v)| k.contains("candidate") || k.contains("DTLS") || v == "Not reported"));
        let position = rows
            .iter()
            .position(|(k, _)| k == crate::app_style::forms::STAT_HOST_ENCODE_TIME)
            .unwrap();
        assert_eq!(rows[position].1, "Waiting for host");
        let mut updated = sample.clone();
        updated.bud.as_mut().unwrap().encode_ms = Some(3.4);
        let refreshed = super::rows(&updated, &Values::default(), (None, None, None));
        assert_eq!(refreshed.len(), rows.len());
        assert_eq!(
            refreshed[position],
            (
                crate::app_style::forms::STAT_HOST_ENCODE_TIME.into(),
                "3.4 ms".into()
            )
        );
    }

    #[test]
    fn silence_retains_audio_format_and_unknowns_stay_unknown() {
        let sample = Sample {
            audio_ready: true,
            route: "Direct — no TURN",
            ..Default::default()
        };
        let rows = rows(&sample, &Values::default(), (None, None, None));
        assert!(rows.contains(&(crate::app_style::forms::STAT_CODEC.into(), "Opus".into())));
        assert!(rows.contains(&(
            crate::app_style::forms::STAT_TURN_SERVER_IN_USE.into(),
            "None".into()
        )));
        assert!(rows.iter().all(|(label, _)| ![
            "Configured bitrate",
            "Packets lost",
            "DTLS version / cipher",
            "GPU video decode"
        ]
        .contains(&label.as_str())));
    }
    #[test]
    fn compact_metrics_use_existing_values_and_keep_unknowns_unknown() {
        let mut sample = crate::stats::Sample {
            rtt_ms: Some(2.5),
            video: Some(Default::default()),
            ..Default::default()
        };
        let rates = crate::stats::Values {
            incoming: Some(10.0),
            fps: Some(60.0),
            ..Default::default()
        };
        let empty = compact_metrics(None, &Default::default());
        assert!(empty.contains("Decode CPU N/A"));
        sample
            .video
            .as_mut()
            .unwrap()
            .local_timing
            .decoder_service
            .record(std::time::Duration::from_millis(1));
        let text = compact_metrics(Some(&sample), &rates);
        assert!(text.contains("Decode CPU 1.0ms"));
        assert!(text.contains("Encode N/A"));
        assert!(text.contains("RTT 2.5ms"));
        assert!(text.contains("RX 10.0Mbps"));
    }
}
