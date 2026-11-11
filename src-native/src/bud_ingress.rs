//! Shared BUD ingress, extracted from the working Windows media path.
//! Borrows encoded payloads; native decoder/output APIs remain in adapters.
use anyhow::{ensure, Result};
use evoparseq_bud::{
    bud::{MediaSource, Session},
    media::{self, Rotation, Video},
};

pub const PUMP_BUDGET: usize = 8;

pub trait VideoTarget {
    fn encoder_rebuilt(&self);
    fn submit_oriented(
        &self,
        bytes: &[u8],
        info: crate::video_stream::PacketInfo,
        rotation: u8,
    ) -> Result<bool>;
}

/// Both the initial attempt and an automatic relay retry use the same capabilities.
pub fn session(
    hevc: bool,
    ten_bit: bool,
    chroma_444: bool,
    raw: bool,
    layout: crate::audio_layout::Layout,
) -> Result<Session> {
    let mut session = Session::external_video_format(hevc, ten_bit, chroma_444)?;
    session.allow_raw_audio(raw);
    session.set_audio_layout(layout.channels, layout.mask)?;
    crate::event_log::record(
        "INFO",
        "audio_capabilities_declared",
        serde_json::json!({
            "transport":"BUD", "raw_pcm_allowed":raw, "channels":layout.channels, "channel_mask":layout.mask
        }),
    );
    Ok(session)
}

pub struct VideoState {
    inspector: crate::video_stream::Inspector,
    waiting_for_keyframe: bool,
    encoder_generation: Option<u32>,
    metadata_rebuilt: bool,
    hevc_available: bool,
    ten_bit_available: bool,
    chroma_444_available: bool,
}

impl VideoState {
    pub fn new(hevc: bool, ten_bit: bool, chroma_444: bool) -> Self {
        let mut inspector = crate::video_stream::Inspector::default();
        inspector.enable_hevc(hevc);
        Self {
            inspector,
            waiting_for_keyframe: true,
            encoder_generation: None,
            metadata_rebuilt: false,
            hevc_available: hevc,
            ten_bit_available: hevc && ten_bit,
            chroma_444_available: hevc && chroma_444,
        }
    }
    pub fn snapshot(&self) -> crate::video_stream::Snapshot {
        self.inspector.snapshot()
    }
    pub fn wait_for_keyframe(&mut self) {
        self.waiting_for_keyframe = true;
    }
    fn restart_encoder(&mut self, target: &impl VideoTarget) {
        self.waiting_for_keyframe = true;
        self.inspector = Default::default();
        self.inspector.enable_hevc(self.hevc_available);
        target.encoder_rebuilt();
        crate::event_log::record("INFO", "bud_video_encoder_rebuilt", serde_json::json!({}));
    }

    pub fn receive(&mut self, message: &[u8], target: &impl VideoTarget) -> Result<bool> {
        let parsed = media::video(message).inspect_err(|_| {
            crate::event_log::record(
                "ERROR",
                "bud_media_rejected",
                serde_json::json!({"stage":"video_framing_or_format"}),
            );
        })?;
        if matches!(&parsed, Video::Metadata(m) if m.chroma_444) && !self.chroma_444_available {
            anyhow::bail!("BUD host sent unadvertised 4:4:4 video");
        }
        let ten_bit = match &parsed {
            Video::Metadata(metadata) => metadata.ten_bit,
            Video::AccessUnit { header, .. } => header.ten_bit,
        };
        if ten_bit && !self.ten_bit_available {
            crate::event_log::record(
                "ERROR",
                "bud_media_rejected",
                serde_json::json!({"stage":"main10_not_advertised"}),
            );
            anyhow::bail!("BUD host sent Main 10 without an advertised GPU decoder");
        }
        // Validate depth before handing the original payload to the native decoder.
        match parsed {
            Video::Metadata(metadata) => {
                if metadata.rebuilt {
                    self.restart_encoder(target);
                    self.metadata_rebuilt = true;
                }
                Ok(false)
            }
            Video::AccessUnit { header, annex_b } => {
                if self
                    .encoder_generation
                    .is_some_and(|previous| previous != header.frame_id)
                    && !self.metadata_rebuilt
                {
                    self.restart_encoder(target);
                }
                self.encoder_generation = Some(header.frame_id);
                self.metadata_rebuilt = false;
                // Existing inspector handles the actual Annex B codec. Native
                // headers from older hosts may label HEVC as H.264.
                let Some(info) = self.inspector.receive(annex_b) else {
                    return Ok(false);
                };
                ensure!(
                    info.codec != crate::video_stream::Codec::Hevc || self.hevc_available,
                    "BUD host sent HEVC without an available native decoder"
                );
                if self.waiting_for_keyframe && !info.idr && !info.parameters {
                    return Ok(false);
                }

                let accepted = target.submit_oriented(
                    annex_b,
                    info,
                    match header.rotation {
                        Rotation::Deg90 => 1,
                        Rotation::Deg180 => 2,
                        Rotation::Deg270 => 3,
                        _ => 0,
                    },
                )?;

                if accepted && info.idr {
                    self.waiting_for_keyframe = false;
                }
                if !accepted {
                    self.waiting_for_keyframe = true;
                }
                Ok(accepted)
            }
        }
    }
}

#[derive(Default)]
pub struct Pumped {
    pub video_messages: usize,
    pub audio_messages: usize,
    pub video_bytes: usize,
    pub audio_bytes: usize,
    pub accepted_video: usize,
    pub accepted_audio: usize,
}

impl Pumped {
    /// A full batch may leave queued media. Drain it before parking the consumer.
    pub fn more_pending(&self) -> bool {
        self.video_messages >= PUMP_BUDGET || self.audio_messages >= PUMP_BUDGET
    }
}

// Static dispatch keeps the borrowed SDK handoff and makes queue scheduling
// testable without networking, credentials or a GPU.
trait PacketSource {
    fn with_video<T>(&mut self, sink: impl FnOnce(&[u8]) -> Result<T>) -> Result<Option<T>>;
    fn with_audio<T>(&mut self, sink: impl FnOnce(&[u8]) -> Result<T>) -> Result<Option<T>>;
}
impl PacketSource for MediaSource {
    fn with_video<T>(&mut self, sink: impl FnOnce(&[u8]) -> Result<T>) -> Result<Option<T>> {
        self.with_video(sink)
    }
    fn with_audio<T>(&mut self, sink: impl FnOnce(&[u8]) -> Result<T>) -> Result<Option<T>> {
        self.with_audio(sink)
    }
}

/// Bounded work on the consumer, without holding session/global GUI locks.
pub(crate) fn pump(
    source: &mut MediaSource,
    state: &mut VideoState,
    target: &impl VideoTarget,
    audio_sender: &crate::audio_stream::Sender,
) -> Result<Pumped> {
    pump_source(source, state, target, audio_sender)
}

/// Bounded work on the application consumer; network receipt runs in the
/// SDK thread. No session/global GUI lock is held for decoder processing.
fn pump_source(
    source: &mut impl PacketSource,
    state: &mut VideoState,
    target: &impl VideoTarget,
    audio_sender: &crate::audio_stream::Sender,
) -> Result<Pumped> {
    let mut counts = Pumped::default();
    for _ in 0..PUMP_BUDGET {
        let Some((accepted, size)) =
            source.with_video(|message| Ok((state.receive(message, target)?, message.len())))?
        else {
            break;
        };
        counts.video_messages += 1;
        counts.video_bytes += size;
        counts.accepted_video += usize::from(accepted);
    }
    for _ in 0..PUMP_BUDGET {
        let Some((accepted, size)) = source.with_audio(|message| {
            Ok((
                match media::audio_layout(message)? {
                    (media::Audio::Opus(bytes), channels, mask) => audio_sender.submit_layout(
                        bytes,
                        false,
                        crate::audio_layout::Layout::new(channels, mask)?,
                    ),
                    (media::Audio::Pcm(bytes), channels, mask) => audio_sender.submit_layout(
                        bytes,
                        true,
                        crate::audio_layout::Layout::new(channels, mask)?,
                    ),
                },
                message.len(),
            ))
        })?
        else {
            break;
        };
        counts.audio_messages += 1;
        counts.audio_bytes += size;
        counts.accepted_audio += usize::from(accepted);
    }
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;

    #[derive(Default)]
    struct Target {
        resets: Cell<usize>,
        reject: Cell<bool>,
        deliveries: RefCell<Vec<(usize, u8)>>,
    }
    impl VideoTarget for Target {
        fn encoder_rebuilt(&self) {
            self.resets.set(self.resets.get() + 1);
        }
        fn submit_oriented(
            &self,
            bytes: &[u8],
            _: crate::video_stream::PacketInfo,
            rotation: u8,
        ) -> Result<bool> {
            self.deliveries
                .borrow_mut()
                .push((bytes.as_ptr() as usize, rotation));
            Ok(!self.reject.get())
        }
    }
    fn picture(generation: u32, nal: u8, rotation: u8) -> Vec<u8> {
        let mut packet = generation.to_le_bytes().to_vec();
        packet.extend_from_slice(&[0x80, 0x07, 0x38, 0x04, 1, rotation]);
        packet.extend_from_slice(&[0, 0, 1, nal, 0x88]);
        packet
    }
    fn metadata(generation: u32) -> Vec<u8> {
        let mut packet = picture(generation, 0, 1);
        packet.truncate(10);
        packet[9] = 0x61;
        packet.extend_from_slice(&[1, 0, 0, 0, 3, 0, 0, 0, 1, 2, 1]);
        packet
    }
    #[test]
    fn keyframe_gate_preserves_payload_and_rotation_and_recovers_after_rejection() {
        let mut state = VideoState::new(false, false, false);
        let target = Target::default();
        let delta = picture(1, 0x41, 1);
        assert!(!state.receive(&delta, &target).unwrap());
        assert!(target.deliveries.borrow().is_empty());
        let key = picture(1, 0x65, 2);
        assert!(state.receive(&key, &target).unwrap());
        assert_eq!(
            target.deliveries.borrow()[0],
            (key[10..].as_ptr() as usize, 1)
        );
        assert!(state.receive(&delta, &target).unwrap());
        target.reject.set(true);
        assert!(!state.receive(&delta, &target).unwrap());
        target.reject.set(false);
        let count = target.deliveries.borrow().len();
        assert!(!state.receive(&delta, &target).unwrap());
        assert_eq!(target.deliveries.borrow().len(), count);
        assert!(state.receive(&key, &target).unwrap());
        assert!(state.receive(&delta, &target).unwrap());
    }
    #[test]
    fn generation_change_and_metadata_reset_once_then_gate_deltas() {
        let mut state = VideoState::new(false, false, false);
        let target = Target::default();
        assert!(state.receive(&picture(1, 0x65, 1), &target).unwrap());
        for _ in 0..3 {
            assert!(state.receive(&picture(1, 0x41, 1), &target).unwrap());
        }
        assert_eq!(target.resets.get(), 0);
        assert!(!state.receive(&picture(2, 0x41, 1), &target).unwrap());
        assert_eq!(target.resets.get(), 1);
        assert!(state.receive(&picture(2, 0x65, 1), &target).unwrap());
        assert!(!state.receive(&metadata(3), &target).unwrap());
        assert_eq!(target.resets.get(), 2);
        assert!(state.receive(&picture(3, 0x65, 1), &target).unwrap());
        assert_eq!(target.resets.get(), 2);
        state.wait_for_keyframe();
        assert!(!state.receive(&picture(3, 0x41, 1), &target).unwrap());
    }
    #[test]
    fn unadvertised_depth_chroma_and_malformed_headers_never_reach_decoder() {
        let mut state = VideoState::new(false, false, false);
        let target = Target::default();
        let mut packet = picture(1, 0x65, 1);
        packet[9] |= 8;
        assert!(state.receive(&packet, &target).is_err());
        let mut packet = metadata(1);
        packet[19] = 0;
        assert!(state.receive(&packet, &target).is_err());
        assert!(state.receive(&packet[..9], &target).is_err());
        assert!(target.deliveries.borrow().is_empty());
        assert_eq!(target.resets.get(), 0);
    }

    struct Source {
        video: VecDeque<Vec<u8>>,
        audio: VecDeque<Vec<u8>>,
        order: Vec<char>,
    }
    impl PacketSource for Source {
        fn with_video<T>(&mut self, sink: impl FnOnce(&[u8]) -> Result<T>) -> Result<Option<T>> {
            let Some(packet) = self.video.pop_front() else {
                return Ok(None);
            };
            self.order.push('v');
            sink(&packet).map(Some)
        }
        fn with_audio<T>(&mut self, sink: impl FnOnce(&[u8]) -> Result<T>) -> Result<Option<T>> {
            let Some(packet) = self.audio.pop_front() else {
                return Ok(None);
            };
            self.order.push('a');
            sink(&packet).map(Some)
        }
    }
    #[cfg(not(windows))]
    #[test]
    fn pcm_surround_reaches_common_callback_bit_exact_and_keeps_speaker_positions() {
        let layout = crate::audio_layout::Layout::new(6, 0x3f).unwrap();
        let audio = crate::audio_stream::Pipeline::start_with_layout(Some(100), layout);
        let mut reader = audio.native_receiver().callback_reader().unwrap();
        let samples = [i16::MIN, -1, 0, 1, 12345, i16::MAX];
        // Native header: 5.1 back speakers, one frame, 48 kHz, PCM, six channels.
        let mut packet = vec![0x3f, 0, 0, 0, 1, 0, 0, 0, 0x80, 0xbb, 0, 0, 2, 2, 6];
        for value in samples {
            packet.extend_from_slice(&value.to_le_bytes());
        }
        let mut source = Source {
            video: VecDeque::new(),
            audio: VecDeque::from([packet]),
            order: Vec::new(),
        };
        let counts = pump_source(
            &mut source,
            &mut VideoState::new(false, false, false),
            &Target::default(),
            &audio.sender().unwrap(),
        )
        .unwrap();
        assert_eq!(counts.accepted_audio, 1);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while audio.snapshot().packets_decoded == 0 && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(audio.snapshot().codec, Some("PCM (uncompressed)"));
        let mut output = [0; 6];
        assert_eq!(reader.render_layout(&mut output, layout, 0).frames, 1);
        assert_eq!(output, samples);
    }
    #[test]
    fn video_first_bounded_batches_drain_bursts_without_parking_or_starving_audio() {
        let audio = crate::audio_stream::Pipeline::start_with_budget(Some(100));
        let sender = audio.sender().unwrap();
        let target = Target::default();
        let mut state = VideoState::new(false, false, false);
        // Header: mask 3, 960 samples, 48000 Hz, Opus, two channels.
        let opus = vec![
            3, 0, 0, 0, 0xc0, 3, 0, 0, 0x80, 0xbb, 0, 0, 1, 2, 2, 0xf8, 0xff, 0xfe,
        ];
        let mut source = Source {
            video: (0..17).map(|_| picture(1, 0x65, 1)).collect(),
            audio: (0..17).map(|_| opus.clone()).collect(),
            order: Vec::new(),
        };
        let mut total = 0;
        loop {
            let batch = pump_source(&mut source, &mut state, &target, &sender).unwrap();
            assert!(batch.video_messages <= PUMP_BUDGET && batch.audio_messages <= PUMP_BUDGET);
            total += batch.accepted_video;
            if !batch.more_pending() {
                break;
            }
            assert!(!source.video.is_empty() && !source.audio.is_empty());
        }
        assert_eq!(total, 17);
        assert!(source.video.is_empty() && source.audio.is_empty());
        assert_eq!(
            &source.order[..16],
            &['v', 'v', 'v', 'v', 'v', 'v', 'v', 'v', 'a', 'a', 'a', 'a', 'a', 'a', 'a', 'a']
        );
    }
}
