//! Phaze opcode 7 audio, decoded to reusable 48 kHz stereo s16 samples.
//! AAC uses standard AudioSpecificConfig 0x1190, not native Phaze code.
use anyhow::{ensure, Context, Result};
use symphonia_codec_aac::AacDecoder;
use symphonia_core::{
    codecs::audio::{
        well_known::CODEC_ID_AAC, AudioCodecParameters, AudioDecoder, AudioDecoderOptions,
    },
    packet::PacketRef,
    units::{Duration, Timestamp},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioCodec {
    Pcm,
    Aac,
}
impl AudioCodec {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pcm => "PCM s16le",
            Self::Aac => "AAC-LC",
        }
    }
}

#[derive(Default)]
pub(crate) struct Audio {
    aac: Option<AacDecoder>,
    pcm: Vec<i16>,
}
impl Audio {
    pub fn decode(&mut self, bytes: &[u8]) -> Result<(AudioCodec, &[i16])> {
        ensure!(bytes.len() >= 5, "Short Phaze audio header");
        let frames = u32::from_be_bytes(bytes[1..5].try_into().unwrap()) as usize;
        let payload = &bytes[5..];
        let codec = match bytes[0] {
            0 => {
                ensure!(
                    frames > 0 && frames <= 5760 && payload.len() == frames * 4,
                    "Invalid Phaze stereo PCM length"
                );
                self.aac = None;
                self.pcm.clear();
                self.pcm.extend(
                    payload
                        .chunks_exact(2)
                        .map(|s| i16::from_le_bytes([s[0], s[1]])),
                );
                AudioCodec::Pcm
            }
            1 => {
                ensure!(
                    frames == 0 && !payload.is_empty() && payload.len() <= 8192,
                    "Invalid Phaze AAC header or length"
                );
                if let Err(error) = self.decode_aac(payload) {
                    // No stale PCM or overlap state may survive a rejected packet.
                    self.aac = None;
                    self.pcm.clear();
                    return Err(error);
                }
                AudioCodec::Aac
            }
            _ => anyhow::bail!("Unknown Phaze audio encoding"),
        };
        Ok((codec, &self.pcm))
    }
    fn decode_aac(&mut self, payload: &[u8]) -> Result<()> {
        if self.aac.is_none() {
            let mut params = AudioCodecParameters::new();
            params
                .for_codec(CODEC_ID_AAC)
                .with_extra_data(Box::new([0x11, 0x90]));
            self.aac = Some(
                AacDecoder::try_new(&params, &AudioDecoderOptions::default())
                    .context("Initialize Phaze AAC-LC decoder")?,
            );
        }
        // Packet boundaries come from the reliable protocol. No demuxer or copy
        // of the encoded payload is needed; no host timing is inferred here.
        let packet = PacketRef::new(0, Timestamp::ZERO, Duration::new(1024), payload);
        let decoded = self
            .aac
            .as_mut()
            .unwrap()
            .decode_ref(&packet)
            .context("Decode Phaze AAC-LC audio")?;
        ensure!(
            decoded.spec().rate() == 48000
                && decoded.spec().channels().count() == 2
                && decoded.frames() == 1024,
            "Unsupported Phaze AAC output format"
        );
        decoded.copy_to_vec_interleaved(&mut self.pcm);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Original synthetic AAC-LC raw_data_block: CPE, common long window,
    // max_sfb=0 (silence), no side tools, ID_END and byte alignment.
    const SILENCE: [u8; 6] = [0x21, 0x00, 0x04, 0x00, 0x80, 0x1c];
    fn packet(encoded: u8, frames: u32, payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![encoded];
        bytes.extend(frames.to_be_bytes());
        bytes.extend(payload);
        bytes
    }
    #[test]
    fn aac_raw_block_decodes_stereo_and_reuses_state_across_packets() -> Result<()> {
        let mut audio = Audio::default();
        for _ in 0..3 {
            let (codec, pcm) = audio.decode(&packet(1, 0, &SILENCE))?;
            assert_eq!(codec.label(), "AAC-LC");
            assert_eq!(pcm.len(), 2048);
            assert!(pcm.iter().all(|s| *s == 0));
        }
        assert!(audio.aac.is_some());
        let (codec, pcm) = audio.decode(&packet(0, 1, &[1, 0, 255, 255]))?;
        assert_eq!(codec.label(), "PCM s16le");
        assert_eq!(pcm, &[1, -1]);
        assert!(audio.aac.is_none());
        assert_eq!(audio.decode(&packet(1, 0, &SILENCE))?.1.len(), 2048);
        Ok(())
    }
    #[test]
    fn rejected_audio_never_produces_stale_samples_and_decoder_can_restart() -> Result<()> {
        let mut audio = Audio::default();
        audio.decode(&packet(1, 0, &SILENCE))?;
        assert!(audio.decode(&packet(1, 0, &[0])).is_err());
        assert!(audio.aac.is_none() && audio.pcm.is_empty());
        assert_eq!(audio.decode(&packet(1, 0, &SILENCE))?.1.len(), 2048);
        for invalid in [
            vec![],
            vec![0; 4],
            packet(2, 0, &SILENCE),
            packet(1, 1, &SILENCE),
            packet(1, 0, &[]),
            packet(1, 0, &[0; 8193]),
            packet(0, 0, &[]),
            packet(0, 1, &[0; 3]),
            packet(0, u32::MAX, &[]),
        ] {
            assert!(audio.decode(&invalid).is_err());
        }
        assert_eq!(audio.decode(&packet(0, 5760, &[0; 23040]))?.1.len(), 11520);
        Ok(())
    }
}
