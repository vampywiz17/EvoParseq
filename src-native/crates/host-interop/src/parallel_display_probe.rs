//! Explicit, one-shot diagnostic for Phaze's independent streams.
use std::time::{Duration, Instant};

#[derive(Default)]
pub(crate) enum Probe {
    #[default]
    Waiting,
    Receiving {
        primary: u32,
        secondary: u32,
        started: Instant,
        secondary_seen: bool,
        interleaved: bool,
    },
    Stopped {
        primary: u32,
        at: Instant,
        interleaved: bool,
    },
    Finished,
}

impl Probe {
    const TIMEOUT: Duration = Duration::from_secs(8);
    const AFTER_STOP: Duration = Duration::from_millis(250);

    pub(crate) fn waiting(&self) -> bool {
        matches!(self, Self::Waiting)
    }

    pub(crate) fn start(&mut self, primary: u32, secondary: u32, now: Instant) {
        *self = Self::Receiving {
            primary,
            secondary,
            started: now,
            secondary_seen: false,
            interleaved: false,
        };
    }

    pub(crate) fn stop_due(&self, now: Instant) -> bool {
        matches!(self, Self::Receiving { started, interleaved, .. }
            if *interleaved || now.duration_since(*started) >= Self::TIMEOUT)
    }

    pub(crate) fn stop(&mut self, now: Instant, interrupted: bool) -> Option<u32> {
        let Self::Receiving {
            primary,
            secondary,
            interleaved,
            ..
        } = *self
        else {
            if interrupted && matches!(self, Self::Stopped { .. }) {
                *self = Self::Finished;
            }
            return None;
        };
        *self = if interrupted {
            Self::Finished
        } else {
            Self::Stopped {
                primary,
                at: now,
                interleaved,
            }
        };
        Some(secondary)
    }

    pub(crate) fn frame(&mut self, output: u32, now: Instant) -> Option<&'static str> {
        match self {
            Self::Receiving {
                primary,
                secondary,
                secondary_seen,
                interleaved,
                ..
            } => {
                *secondary_seen |= output == *secondary;
                *interleaved |= *secondary_seen && output == *primary;
            }
            Self::Stopped {
                primary,
                at,
                interleaved,
            } if output == *primary && now.duration_since(*at) >= Self::AFTER_STOP => {
                let status = if *interleaved {
                    "confirmed"
                } else {
                    "no_interleaved_video"
                };
                *self = Self::Finished;
                return Some(status);
            }
            _ => {}
        }
        None
    }

    pub(crate) fn finish_due(&mut self, now: Instant) -> bool {
        if matches!(self, Self::Stopped { at, .. } if now.duration_since(*at) >= Self::TIMEOUT) {
            *self = Self::Finished;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirms_interleaved_frames_and_primary_after_stopping_secondary() {
        let now = Instant::now();
        let mut probe = Probe::default();
        assert!(probe.waiting());
        probe.start(1, 2, now);
        assert_eq!(probe.frame(1, now), None);
        assert_eq!(probe.frame(2, now), None);
        assert!(!probe.stop_due(now));
        assert_eq!(probe.frame(1, now), None);
        assert!(probe.stop_due(now));
        assert_eq!(probe.stop(now, false), Some(2));
        assert_eq!(probe.frame(2, now + Probe::AFTER_STOP), None);
        assert_eq!(probe.frame(1, now), None);
        assert_eq!(probe.frame(1, now + Probe::AFTER_STOP), Some("confirmed"));
        assert!(!probe.finish_due(now + Probe::TIMEOUT));
    }

    #[test]
    fn times_out_and_accepts_interruptions_without_retrying() {
        let now = Instant::now();
        let mut probe = Probe::default();
        probe.start(1, 2, now);
        let deadline = now + Probe::TIMEOUT;
        assert!(probe.stop_due(deadline));
        assert_eq!(probe.stop(deadline, false), Some(2));
        assert_eq!(
            probe.frame(1, deadline + Probe::AFTER_STOP),
            Some("no_interleaved_video")
        );
        probe.start(1, 2, now);
        assert_eq!(probe.stop(now, false), Some(2));
        assert!(probe.finish_due(deadline));
        probe.start(1, 2, now);
        assert_eq!(probe.stop(now, false), Some(2));
        assert_eq!(probe.stop(now, true), None);
        assert!(!probe.finish_due(deadline));
        probe.start(1, 2, now);
        assert_eq!(probe.stop(now, true), Some(2));
        assert_eq!(probe.stop(now, true), None);
        assert!(!probe.waiting());
    }
}
