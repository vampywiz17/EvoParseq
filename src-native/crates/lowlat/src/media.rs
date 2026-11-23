//! Remove native LOWLAT media headers, leaving borrowed original codec payloads.
//! No decoder, resampler, pixel conversion, render resource or media copy.
use anyhow::{Result, ensure};
pub use lowlat_core::video::Rotation;
use lowlat_core::{audio, video};

pub enum Video<'a> {
    Metadata(video::KeyframeMetadata),
    AccessUnit {
        header: video::VideoHeader,
        annex_b: &'a [u8],
    },
}

pub fn video(message: &[u8]) -> Result<Video<'_>> {
    let header =
        video::parse(message).map_err(|e| anyhow::anyhow!("native video header: {e:?}"))?;
    if header.metadata {
        let metadata = video::parse_metadata(message)
            .map_err(|e| anyhow::anyhow!("native video metadata: {e:?}"))?;
        #[cfg(not(feature = "native-core"))]
        ensure!(
            !metadata.chroma_444,
            "LOWLAT 4:4:4 video is not supported by this client"
        );
        return Ok(Video::Metadata(metadata));
    }
    ensure!(
        header.width != 0 && header.height != 0,
        "empty LOWLAT video dimensions"
    );
    let annex_b = message.get(video::VIDEO_HEADER_LEN..).unwrap_or_default();
    ensure!(!annex_b.is_empty(), "empty LOWLAT access unit");
    Ok(Video::AccessUnit { header, annex_b })
}

pub fn opus(message: &[u8]) -> Result<&[u8]> {
    let header =
        audio::parse(message).map_err(|e| anyhow::anyhow!("native audio header: {e:?}"))?;
    ensure!(
        header.codec == audio::Codec::Opus,
        "uncompressed LOWLAT audio is not enabled"
    );
    ensure!(
        header.rate == 48_000 && header.channels == 2 && header.mask == 3,
        "unsupported LOWLAT audio layout"
    );
    let opus = message.get(audio::AUDIO_HEADER_LEN..).unwrap_or_default();
    ensure!(!opus.is_empty(), "empty LOWLAT Opus packet");
    Ok(opus)
}

pub enum Audio<'a> {
    Opus(&'a [u8]),
    Pcm(&'a [u8]),
}

/// The wire's codec tag is authoritative; a RAW permission can still yield Opus.
pub fn audio(message: &[u8]) -> Result<Audio<'_>> {
    let (audio, channels, mask) = audio_layout(message)?;
    ensure!(channels == 2 && mask == 3, "unsupported LOWLAT audio layout");
    Ok(audio)
}

/// Preserve the wire layout for the application's native decoder/output.
pub fn audio_layout(message: &[u8]) -> Result<(Audio<'_>, u8, u32)> {
    let header =
        audio::parse(message).map_err(|e| anyhow::anyhow!("native audio header: {e:?}"))?;
    ensure!(
        header.rate == 48_000
            && matches!(
                (header.channels, header.mask),
                (2, 0 | 3) | (6, 0x3f | 0x60f) | (8, 0x63f)
            ),
        "unsupported LOWLAT audio layout"
    );
    let payload = message.get(audio::AUDIO_HEADER_LEN..).unwrap_or_default();
    ensure!(!payload.is_empty(), "empty LOWLAT audio packet");
    if header.codec == audio::Codec::Opus {
        Ok((Audio::Opus(payload), header.channels, header.mask))
    } else {
        ensure!(
            payload.len() <= 32_000
                && payload
                    .len()
                    .is_multiple_of(usize::from(header.channels) * 2),
            "invalid interleaved PCM packet"
        );
        Ok((Audio::Pcm(payload), header.channels, header.mask))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pcm_and_opus_are_distinguished_by_header_and_pcm_is_bounded() {
        let mut packet = vec![0; audio::AUDIO_HEADER_LEN];
        audio::encode(
            &mut packet,
            &audio::AudioHeader::stereo(1, audio::Codec::Pcm),
        )
        .unwrap();
        packet.extend_from_slice(&[0, 128, 255, 127]);
        let Audio::Pcm(pcm) = audio(&packet).unwrap() else {
            panic!("PCM header ignored")
        };
        assert_eq!(pcm, &[0, 128, 255, 127]);
        assert_eq!(pcm.as_ptr(), packet[audio::AUDIO_HEADER_LEN..].as_ptr());
        packet.push(0);
        assert!(audio(&packet).is_err());
        packet.truncate(audio::AUDIO_HEADER_LEN);
        audio::encode(
            &mut packet,
            &audio::AudioHeader::stereo(960, audio::Codec::Opus),
        )
        .unwrap();
        packet.extend_from_slice(&[0xf8, 0xff, 0xfe]);
        assert!(matches!(audio(&packet), Ok(Audio::Opus(_))));
    }
    #[test]
    fn borrowed_video_is_the_original_bitstream() {
        let mut packet = vec![0; video::VIDEO_HEADER_LEN];
        video::encode(
            &mut packet,
            &video::VideoHeader {
                frame_id: 42,
                width: 1920,
                height: 1080,
                codec: video::Codec::H264,
                rotation: video::Rotation::None,
                ten_bit: false,
                locked: false,
                announced: false,
                metadata: false,
            },
        )
        .unwrap();
        packet.extend_from_slice(&[0, 0, 0, 1, 0x65, 0x88]);
        let Video::AccessUnit { header, annex_b } = video(&packet).unwrap() else {
            panic!("expected picture")
        };
        assert_eq!(header.frame_id, 42);
        assert_eq!(annex_b.as_ptr(), packet[video::VIDEO_HEADER_LEN..].as_ptr());
        assert_eq!(annex_b, &[0, 0, 0, 1, 0x65, 0x88]);
        packet[9] |= 8;
        let Video::AccessUnit { header, annex_b } = video(&packet).unwrap() else {
            panic!("Main 10 flag must not reject an access unit")
        };
        assert!(header.ten_bit);
        assert_eq!(annex_b.as_ptr(), packet[video::VIDEO_HEADER_LEN..].as_ptr());
        assert!(video(&packet[..9]).is_err());
    }

    #[test]
    fn main10_metadata_and_picture_are_borrowed_but_444_stays_rejected() {
        let header = video::VideoHeader {
            frame_id: 42,
            width: 1920,
            height: 1080,
            codec: video::Codec::H265,
            rotation: Rotation::None,
            ten_bit: true,
            locked: false,
            announced: true,
            metadata: false,
        };
        let metadata = video::KeyframeMetadata {
            rebuilt: true,
            keyframe: true,
            ten_bit: true,
            chroma_444: false,
            rotation: Rotation::None,
        };
        let mut packet = vec![0; video::METADATA_LEN];
        video::encode_metadata(&mut packet, &header, &metadata).unwrap();
        let Video::Metadata(parsed) = video(&packet).unwrap() else {
            panic!("Main 10 metadata rejected")
        };
        assert!(parsed.ten_bit && parsed.rebuilt && parsed.keyframe);
        assert!(!parsed.chroma_444);
        packet.truncate(video::VIDEO_HEADER_LEN);
        video::encode(&mut packet, &header).unwrap();
        packet.extend_from_slice(&[0, 0, 0, 1, 0x26, 0x01, 0x80]);
        let Video::AccessUnit { header, annex_b } = video(&packet).unwrap() else {
            panic!("Main 10 picture rejected")
        };
        assert!(header.ten_bit);
        assert_eq!(annex_b, &[0, 0, 0, 1, 0x26, 0x01, 0x80]);
        assert_eq!(annex_b.as_ptr(), packet[video::VIDEO_HEADER_LEN..].as_ptr());
        packet.resize(video::METADATA_LEN, 0);
        let metadata = video::KeyframeMetadata {
            chroma_444: true,
            ..metadata
        };
        video::encode_metadata(&mut packet, &header, &metadata).unwrap();
        #[cfg(not(feature = "native-core"))]
        assert!(
            video(&packet).is_err(),
            "legacy 4:4:4 must remain unsupported"
        );
        #[cfg(feature = "native-core")]
        assert!(matches!(video(&packet), Ok(Video::Metadata(m)) if m.chroma_444));
        assert!(video(&packet[..video::METADATA_LEN - 1]).is_err());
    }

    #[test]
    fn audio_is_borrowed_opus_and_never_redecoded_here() {
        let mut packet = vec![0; audio::AUDIO_HEADER_LEN];
        audio::encode(
            &mut packet,
            &audio::AudioHeader::stereo(960, audio::Codec::Opus),
        )
        .unwrap();
        packet.extend_from_slice(&[0xf8, 0xff, 0xfe]);
        let payload = opus(&packet).unwrap();
        assert_eq!(payload.as_ptr(), packet[audio::AUDIO_HEADER_LEN..].as_ptr());
        assert_eq!(payload, &[0xf8, 0xff, 0xfe]);
        audio::encode(
            &mut packet,
            &audio::AudioHeader::stereo(960, audio::Codec::Pcm),
        )
        .unwrap();
        assert!(opus(&packet).is_err());
    }
}
