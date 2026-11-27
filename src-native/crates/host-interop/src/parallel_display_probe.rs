//! Explicit, one-shot diagnostic for Phaze's independent streams.
use crate::{wire, Output, VideoConfig};
use anyhow::Result;
use std::time::{Duration, Instant};

pub(crate) enum Action {
    Start(Vec<u8>),
    Stop([u8; 5], bool),
    Status(&'static str),
}

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

    pub(crate) fn poll(
        &mut self,
        now: Instant,
        primary: Option<u32>,
        outputs: &[Output],
        may_start: bool,
        updating: bool,
    ) -> Result<Option<Action>> {
        if updating || self.stop_due(now) {
            return Ok(self
                .stop(now, updating)
                .map(|output| Action::Stop(wire::stop_video(output), updating)));
        }
        if self.finish_due(now) {
            return Ok(Some(Action::Status("primary_not_seen_after_stop")));
        }
        if !self.waiting() || !may_start {
            return Ok(None);
        }
        let Some(primary) = primary else {
            return Ok(None);
        };
        let Some(secondary) = outputs.iter().find(|o| o.active && o.id != primary) else {
            return Ok(None);
        };
        let config = VideoConfig {
            mbps: 3,
            fps: 5,
            ..VideoConfig::default()
        };
        let message = config.request(secondary.id)?;
        self.start(primary, secondary.id, now);
        Ok(Some(Action::Start(message)))
    }

    pub(crate) fn video(
        &mut self,
        frame: &[u8],
        selected: Option<u32>,
        has_payload: bool,
        now: Instant,
    ) -> (bool, Option<&'static str>) {
        let output = wire::be(frame, 0);
        let result = if has_payload {
            self.frame(output, now)
        } else {
            None
        };
        (selected == Some(output), result)
    }

    pub(crate) fn active(&self) -> bool {
        matches!(self, Self::Receiving { .. } | Self::Stopped { .. })
    }

    fn waiting(&self) -> bool {
        matches!(self, Self::Waiting)
    }

    fn start(&mut self, primary: u32, secondary: u32, now: Instant) {
        *self = Self::Receiving {
            primary,
            secondary,
            started: now,
            secondary_seen: false,
            interleaved: false,
        };
    }

    fn stop_due(&self, now: Instant) -> bool {
        matches!(self, Self::Receiving { started, interleaved, .. }
            if *interleaved || now.duration_since(*started) >= Self::TIMEOUT)
    }

    fn stop(&mut self, now: Instant, interrupted: bool) -> Option<u32> {
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

    fn frame(&mut self, output: u32, now: Instant) -> Option<&'static str> {
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

    fn finish_due(&mut self, now: Instant) -> bool {
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

    #[test]
    fn probe_queues_only_the_secondary_and_keeps_its_video_out_of_the_primary() -> Result<()> {
        let now = Instant::now();
        let active = |id, active| Output {
            refresh_fields: Default::default(),
            id,
            active,
            name: String::new(),
            width: 1920,
            height: 1080,
            primary: id == 1,
        };
        let outputs = [active(1, true), active(2, false), active(3, true)];
        let mut probe = Probe::default();
        assert!(probe.poll(now, None, &outputs, true, false)?.is_none());
        assert!(probe.poll(now, Some(1), &outputs, false, false)?.is_none());
        assert!(probe
            .poll(now, Some(1), &outputs[..2], true, false)?
            .is_none());
        let Some(Action::Start(message)) = probe.poll(now, Some(1), &outputs, true, false)? else {
            panic!("expected secondary start");
        };
        assert_eq!(message[0], 3);
        assert_eq!(wire::be(&message, 1), 3);
        assert_eq!(wire::be(&message, 9), 3);
        assert_eq!(wire::be(&message, 13), 5);
        assert!(probe.poll(now, Some(1), &outputs, true, false)?.is_none());

        let primary = 1u32.to_be_bytes();
        let secondary = 3u32.to_be_bytes();
        assert_eq!(probe.video(&secondary, Some(1), false, now), (false, None));
        assert_eq!(probe.video(&primary, Some(1), true, now), (true, None));
        assert!(probe.poll(now, Some(1), &outputs, true, false)?.is_none());
        assert_eq!(probe.video(&secondary, Some(1), true, now), (false, None));
        assert_eq!(probe.video(&primary, Some(1), true, now), (true, None));

        let Some(Action::Stop(message, false)) = probe.poll(now, Some(1), &outputs, true, false)?
        else {
            panic!("expected secondary stop");
        };
        assert_eq!(message, [4, 0, 0, 0, 3]);
        assert_eq!(
            probe.video(&primary, Some(1), true, now + Probe::AFTER_STOP),
            (true, Some("confirmed"))
        );
        assert!(probe.poll(now, Some(1), &outputs, true, false)?.is_none());
        Ok(())
    }

    #[test]
    fn probe_stops_the_secondary_when_the_user_switches_the_primary() -> Result<()> {
        let now = Instant::now();
        let mut probe = Probe::default();
        probe.start(1, 2, now);
        assert!(matches!(
            probe.poll(now, Some(1), &[], true, true)?,
            Some(Action::Stop([4, 0, 0, 0, 2], true))
        ));
        assert!(!probe.waiting());
        probe.start(1, 2, now);
        let deadline = now + Probe::TIMEOUT;
        assert!(matches!(
            probe.poll(deadline, Some(1), &[], false, false)?,
            Some(Action::Stop([4, 0, 0, 0, 2], false))
        ));
        assert!(matches!(
            probe.poll(deadline + Probe::TIMEOUT, Some(1), &[], false, false)?,
            Some(Action::Status("primary_not_seen_after_stop"))
        ));
        Ok(())
    }
}
