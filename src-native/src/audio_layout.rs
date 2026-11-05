//! Validated Windows speaker layouts used by the native BUD audio bridge.
use anyhow::{bail, Result};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Layout {
    pub channels: u8,
    pub mask: u32,
}
impl Layout {
    pub const STEREO: Self = Self {
        channels: 2,
        mask: 3,
    };

    pub fn new(channels: u8, mask: u32) -> Result<Self> {
        match (channels, mask) {
            (2, 0 | 3) => Ok(Self::STEREO),
            (6, 0x3f | 0x60f) | (8, 0x63f) => Ok(Self { channels, mask }),
            _ => bail!("unsupported native speaker layout"),
        }
    }

    pub fn samples(self, frames: usize) -> usize {
        frames * usize::from(self.channels)
    }

    /// Explicit stereo fallback for a disconnected multichannel endpoint.
    /// Equal-power center/surround fold-down, LFE omitted; saturates to i16.
    /// Normal matching-layout output never runs this conversion.
    #[cfg(any(not(windows), test))]
    pub fn stereo(self, frame: &[i16]) -> [i16; 2] {
        let front = |side: usize| {
            let center = i64::from(frame[2]);
            let mut surround = i64::from(frame[4 + side]);
            if self.channels == 8 {
                surround += i64::from(frame[6 + side]);
            }
            let sample = i64::from(frame[side]) + ((center + surround) * 23170 / 32768);
            sample.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16
        };
        if self == Self::STEREO {
            [frame[0], frame[1]]
        } else {
            [front(0), front(1)]
        }
    }

    /// Parsec's multistream contract, in Windows speaker order. These are not
    /// Vorbis output channel positions; the mapping describes the wire streams.
    pub fn opus(self) -> (i32, i32, [u8; 8]) {
        match self.channels {
            6 => (4, 2, [0, 1, 4, 5, 2, 3, 0, 0]),
            8 => (5, 3, [0, 1, 6, 7, 2, 3, 4, 5]),
            _ => (1, 1, [0, 1, 0, 0, 0, 0, 0, 0]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fallback_fold_down_uses_correct_speakers_omits_lfe_and_saturates() {
        let layout = Layout::new(8, 0x63f).unwrap();
        assert_eq!(layout.stereo(&[0, 0, 0, 30000, 0, 0, 0, 0]), [0, 0]);
        assert_eq!(layout.stereo(&[0, 0, 0, 0, 10000, 0, 0, 0]), [7070, 0]);
        assert_eq!(layout.stereo(&[0, 0, 0, 0, 0, 0, 0, 10000]), [0, 7070]);
        assert_eq!(
            layout.stereo(&[30000, 30000, 30000, 0, 30000, 30000, 30000, 30000]),
            [i16::MAX, i16::MAX]
        );
        assert_eq!(
            layout.stereo(&[-30000, -30000, -30000, 0, -30000, -30000, -30000, -30000]),
            [i16::MIN, i16::MIN]
        );
        assert_eq!(Layout::STEREO.stereo(&[-123, 456]), [-123, 456]);
    }
    #[test]
    fn speaker_masks_and_wire_mapping_are_validated() {
        for (channels, mask) in [(6, 0x3f), (6, 0x60f), (8, 0x63f)] {
            let layout = Layout::new(channels, mask).unwrap();
            let (streams, coupled, mapping) = layout.opus();
            assert_eq!(streams + coupled, i32::from(channels));
            let mut indices = mapping[..usize::from(channels)].to_vec();
            indices.sort();
            assert_eq!(indices, (0..channels).collect::<Vec<_>>());
        }
        assert!(Layout::new(6, 3).is_err());
        assert!(Layout::new(8, 0x3f).is_err());
        assert_eq!(Layout::new(2, 0).unwrap(), Layout::STEREO);
    }
}
