//! Opt-in BUD media handoff to the existing native decoders.
//! No alternate decoder, renderer, decoded-frame export or GPU interop path.
use anyhow::Result;
use evoparseq_bud::bud::{MediaSource, Session};
use std::sync::Arc;

pub(crate) use crate::bud_ingress::session;
pub struct Media {
    displays: std::sync::Weak<crate::native_core::multi_display::Displays>,
    video: crate::video_windows::Pipeline,
    audio: Arc<crate::audio_stream::Pipeline>,
    audio_sender: crate::audio_stream::Sender,
    ingress: crate::bud_ingress::VideoState,
}

impl Media {
    /// Call only after an explicit BUD selection. Account/UI wiring is separate;
    /// this does not start any connection or change a WebRTC session.
    pub fn start_verified(
        window: Arc<crate::window::Window>,
        hevc_available: bool,
        ten_bit: bool,
        chroma_444: bool,
        raw_audio: bool,
        surround: bool,
    ) -> Result<(Self, Session)> {
        let layout = if surround {
            crate::audio_windows::device_layout().unwrap_or(crate::audio_layout::Layout::STEREO)
        } else {
            crate::audio_layout::Layout::STEREO
        };
        let session = session(hevc_available, ten_bit, chroma_444, raw_audio, layout)?;
        let audio = Arc::new(crate::audio_stream::Pipeline::start_with_layout(
            window.audio_buffer().map(|(_, max)| max),
            layout,
        ));
        let audio_sender = audio
            .sender()
            .ok_or_else(|| anyhow::anyhow!("native audio sender unavailable"))?;
        let displays = Arc::downgrade(&window.displays);
        window
            .multidisplay_ready
            .store(true, std::sync::atomic::Ordering::Release);
        let video = crate::video_windows::Pipeline::start(
            window,
            crate::video_output::DisplayCatchUpPolicy::Fixed,
        );
        Ok((
            Self {
                displays,
                video,
                audio,
                audio_sender,
                ingress: crate::bud_ingress::VideoState::new(hevc_available, ten_bit, chroma_444),
            },
            session,
        ))
    }

    pub fn pump(&mut self, source: &mut MediaSource) -> Result<crate::bud_ingress::Pumped> {
        let counts =
            crate::bud_ingress::pump(source, &mut self.ingress, &self.video, &self.audio_sender)?;
        if let Some(displays) = self.displays.upgrade() {
            displays.pump_bud(source)?;
        }
        Ok(counts)
    }

    pub fn recover(&mut self, session: &mut Session) {
        if let Some((generation, fallback_420)) = self.video.take_keyframe_request() {
            self.ingress.wait_for_keyframe();
            if !session.request_keyframe_format(fallback_420) {
                self.video.recovery_request_failed(generation);
            }
        }
    }

    pub fn audio_pipeline(&self) -> Arc<crate::audio_stream::Pipeline> {
        self.audio.clone()
    }
    pub fn audio_layout(&self) -> crate::audio_layout::Layout {
        self.audio.allowed_layout()
    }

    pub fn ingress(&self) -> crate::media_ingress::Ingress {
        crate::media_ingress::Ingress {
            video_stream: self.ingress.snapshot(),
            video_output: Some(self.video.snapshot()),
            audio_output: Some(self.audio.snapshot()),
            video_decoder_available: true,
            audio_decoder_available: true,
            ..Default::default()
        }
    }
}

impl crate::bud_ingress::VideoTarget for crate::video_windows::Pipeline {
    fn encoder_rebuilt(&self) {
        self.encoder_rebuilt();
    }
    fn submit_oriented(
        &self,
        bytes: &[u8],
        info: crate::video_stream::PacketInfo,
        rotation: u8,
    ) -> Result<bool> {
        Ok(self.submit_oriented(bytes, info, rotation))
    }
}
