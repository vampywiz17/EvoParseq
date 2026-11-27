//! Retransmission staleness tuning used by the connecting client.
//!
//! The host's encoder bitrate controller and its experimental harness are not
//! part of this transport-only copy. Keep the original send-window floor and
//! all three wire-compatible retransmission levels unchanged.

/// Window below which congestion is never declared, and the cap on outstanding
/// fragments used by the send ring.
pub const WINDOW_FLOOR: u32 = 100;

/// Per-level tuning.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Level {
    /// Multiplier on the smoothed round trip when classifying staleness.
    pub rtt_mult: f64,
    /// Constant added to the staleness threshold, in milliseconds.
    pub base_ms: f64,
    /// Stale-to-window ratio above which the channel is congested.
    pub stale_ratio: f64,
}

/// Level 0 is **not** "disabled".
///
/// Its threshold of zero means any stale fragment declares congestion once the
/// window exceeds the floor, which makes it the most aggressive setting rather
/// than the least. It exists for compatibility with an older scheme and must
/// never be used as a fallback for an out-of-range value.
pub const LEVELS: [Level; 3] = [
    Level {
        rtt_mult: 0.0,
        base_ms: 0.0,
        stale_ratio: 0.0,
    },
    Level {
        rtt_mult: 1.1,
        base_ms: 20.0,
        stale_ratio: 0.15,
    },
    Level {
        rtt_mult: 1.5,
        base_ms: 50.0,
        stale_ratio: 0.35,
    },
];

/// The default. Not level 0; see [`LEVELS`].
pub const DEFAULT_LEVEL: usize = 1;

/// Resolve a level index, falling back to the default rather than to zero.
/// The former host-only adaptive index also resolves to the same default.
pub fn level(index: usize) -> Level {
    *LEVELS.get(index).unwrap_or(&LEVELS[DEFAULT_LEVEL])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_zero_is_the_most_aggressive_not_disabled() {
        assert_eq!(level(0).stale_ratio, 0.0);
        assert!(level(0).rtt_mult < level(DEFAULT_LEVEL).rtt_mult);
        assert!(level(0).base_ms < level(DEFAULT_LEVEL).base_ms);
    }

    #[test]
    fn an_out_of_range_level_falls_back_to_the_default_not_to_zero() {
        for index in [3, 99, usize::MAX] {
            assert_eq!(level(index), LEVELS[DEFAULT_LEVEL]);
            assert_ne!(level(index), LEVELS[0]);
        }
    }
}
