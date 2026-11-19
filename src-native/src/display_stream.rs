//! Bounded Parsec display-stream wire model; no window or media APIs.
use anyhow::{ensure, Result};
pub const STREAMS: usize = 3;
pub fn motion(stream: usize, bytes: &[u8]) -> Result<bytes::Bytes> {
    ensure!(
        (1..STREAMS).contains(&stream) && bytes.len() == 13 && bytes[12] == 3,
        "Invalid additional display motion"
    );
    let mode = i32::from_be_bytes(bytes[..4].try_into()?);
    ensure!(
        (0..=1).contains(&mode),
        "Unsupported additional display motion mode"
    );
    let x = i32::from_be_bytes(bytes[4..8].try_into()?);
    let y = i32::from_be_bytes(bytes[8..12].try_into()?);
    ensure!(
        (if mode == 0 {
            u16::try_from(x).is_ok()
        } else {
            i16::try_from(x).is_ok()
        }) && i16::try_from(y).is_ok(),
        "Additional display coordinates exceed protocol limits"
    );
    // Stock 150.105c RVA 4aa39: (stream << 1) | relative,
    // unsigned horizontal low half, signed vertical high half, zero.
    let packed = (u32::from(x as u16) | ((y as u16 as u32) << 16)) as i32;
    Ok(crate::control::header(
        26,
        ((stream as i32) << 1) | mode,
        packed,
        0,
    ))
}

pub fn enable(stream: usize, active: bool) -> Result<bytes::Bytes> {
    ensure!(
        (1..STREAMS).contains(&stream),
        "Invalid additional display stream"
    );
    // Stock 150.105c client RVA a1920: stream, enabled, zero, opcode 27.
    // In the opposite direction opcode 27 is a stream-ended notification.
    Ok(crate::control::header(
        27,
        stream as i32,
        i32::from(active),
        0,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn activation_never_targets_primary_or_an_unbounded_slot() {
        for stream in 1..STREAMS {
            for active in [false, true] {
                assert_eq!(
                    enable(stream, active).unwrap(),
                    crate::control::header(27, stream as i32, i32::from(active), 0)
                );
            }
        }
        for stream in [0, STREAMS, usize::MAX] {
            assert!(enable(stream, true).is_err());
        }
    }
    #[test]
    fn motion_preserves_stream_and_signed_packed_coordinates() {
        for stream in 1..STREAMS {
            for (mode, x, y) in [
                (0, 65535, 32767),
                (0, 0, -32768),
                (1, -12, -35),
                (1, 32767, 17),
            ] {
                let packet = motion(stream, &crate::control::header(3, mode, x, y)).unwrap();
                assert_eq!(packet[12], 26);
                assert_eq!(
                    i32::from_be_bytes(packet[..4].try_into().unwrap()),
                    (stream as i32) * 2 + mode
                );
                assert_eq!(
                    u16::from_be_bytes(packet[6..8].try_into().unwrap()),
                    x as u16
                );
                assert_eq!(
                    i16::from_be_bytes(packet[4..6].try_into().unwrap()),
                    y as i16
                );
                assert_eq!(&packet[8..12], &[0; 4]);
            }
        }
        for (mode, x, y) in [
            (0, -1, 0),
            (0, 65536, 0),
            (1, -32769, 0),
            (1, 32768, 0),
            (0, 0, 32768),
            (3, 0, 0),
        ] {
            assert!(motion(1, &crate::control::header(3, mode, x, y)).is_err());
        }
        assert!(motion(0, &crate::control::header(3, 0, 0, 0)).is_err());
        assert!(motion(3, &crate::control::header(3, 0, 0, 0)).is_err());
        assert!(motion(1, &[0; 12]).is_err());
        assert!(motion(1, &crate::control::header(1, 0, 0, 0)).is_err());
    }
}
