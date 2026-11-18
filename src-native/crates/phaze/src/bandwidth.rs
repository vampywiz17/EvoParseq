use std::time::Instant;

const HISTORY: usize = 256;
const MAX_PROBES: usize = 19;
const MIN_INTERVALS: u32 = 10;
const GROUP_GAP_NS: u64 = 50_000;
pub(crate) const MAX_PACKET: usize = 1230;

#[derive(Clone, Copy)]
struct Arrival {
    at: Instant,
    len: usize,
    kernel_ns: Option<i64>,
}

/// Comparison for one complete, host-marked probe train. Timing sums cover
/// the same consecutive equal-sized packet pairs as the application estimator.
#[derive(serde::Serialize)]
pub struct ProbeTiming {
    ordinary_packets: usize,
    padding_packets: usize,
    packet_bytes: usize,
    intervals: u32,
    app_interval_sum_ns: u64,
    kernel_interval_sum_ns: Option<u64>,
    group_boundaries: u32,
    largest_boundary_ns: u64,
    reversed_kernel_intervals: u32,
}

struct ProbeTrain {
    end: u64,
    ordinary_count: usize,
    probe_count: usize,
    arrivals: [Option<Arrival>; MAX_PROBES],
}

pub(crate) struct Bandwidth {
    recent: [Option<(u64, Arrival)>; HISTORY],
    probe: Option<ProbeTrain>,
    last_measured: Option<u64>,
    gaps: Option<(f64, f64)>,
    feedback: Option<(f64, f64)>,
    probe_timing: Option<ProbeTiming>,
}

impl Default for Bandwidth {
    fn default() -> Self {
        Self {
            recent: [None; HISTORY],
            probe: None,
            last_measured: None,
            gaps: None,
            feedback: None,
            probe_timing: None,
        }
    }
}

impl Bandwidth {
    pub fn ordinary(&mut self, seq: u64, len: usize, at: Instant, kernel_ns: Option<i64>) {
        if len <= MAX_PACKET {
            self.recent[seq as usize % HISTORY] = Some((seq, Arrival { at, len, kernel_ns }));
        }
    }

    pub fn complete_message(&mut self, start: u64, count: u64) {
        if !(20..=HISTORY as u64).contains(&count) {
            return;
        }
        let end = start + count - 1;
        let arrivals = ordinary_arrivals(&self.recent, start, end);
        if let Some((len, gap)) = train_gap(arrivals) {
            self.observe(end, len, gap);
        }
    }

    pub fn probe(&mut self, packet: &[u8], highest: u64, at: Instant, kernel_ns: Option<i64>) {
        if !(13..=MAX_PACKET).contains(&packet.len()) {
            return;
        }
        let index = usize::from(packet[1]);
        let probe_count = usize::from(packet[2]);
        let ordinary_count = usize::from(u16::from_le_bytes([packet[3], packet[4]]));
        let end = u64::from_le_bytes(packet[5..13].try_into().unwrap());
        if !(1..=MAX_PROBES).contains(&probe_count)
            || !(1..=probe_count).contains(&index)
            || !(1..=HISTORY).contains(&ordinary_count)
            || end < ordinary_count as u64 - 1
            || end.abs_diff(highest) >= HISTORY as u64
            || self.last_measured.is_some_and(|last| end <= last)
            || self.probe.as_ref().is_some_and(|train| end < train.end)
        {
            return;
        }
        if self.probe.as_ref().is_none_or(|train| end > train.end) {
            self.probe = Some(ProbeTrain {
                end,
                ordinary_count,
                probe_count,
                arrivals: [None; MAX_PROBES],
            });
        }
        let train = self.probe.as_mut().unwrap();
        if train.ordinary_count != ordinary_count || train.probe_count != probe_count {
            return;
        }
        let entry = &mut train.arrivals[index - 1];
        if entry.is_some() {
            return;
        }
        *entry = Some(Arrival {
            at,
            len: packet.len(),
            kernel_ns,
        });
        if train.arrivals[probe_count - 1].is_some() {
            let start = end - (ordinary_count as u64 - 1);
            let arrivals = ordinary_arrivals(&self.recent, start, end)
                .chain(train.arrivals[..probe_count].iter().copied());
            let gap = train_gap(arrivals.clone());
            if let Some((len, gap)) = gap {
                self.probe_timing = kernel_ns
                    .and_then(|_| compare_probe(arrivals, len, ordinary_count, probe_count));
                self.observe(end, len, gap);
            }
        }
    }

    fn observe(&mut self, end: u64, len: usize, gap: f64) {
        if self.last_measured.is_some_and(|last| end <= last) {
            return;
        }
        self.last_measured = Some(end);
        // Smooth the mean packet spacing and keep a slower-recovering,
        // conservative spacing for the lower bandwidth estimate.
        let (typical, conservative) = self.gaps.unwrap_or((gap, gap));
        let typical = typical * 0.8 + gap * 0.2;
        let conservative = gap.max(conservative * 0.8 + gap * 0.2).max(typical);
        self.gaps = Some((typical, conservative));
        self.feedback = Some((len as f64 / conservative, len as f64 / typical));
    }

    pub fn take_feedback(&mut self) -> Option<(f64, f64)> {
        self.feedback.take()
    }

    pub fn take_probe_timing(&mut self) -> Option<ProbeTiming> {
        self.probe_timing.take()
    }
}

fn ordinary_arrivals(
    recent: &[Option<(u64, Arrival)>; HISTORY],
    start: u64,
    end: u64,
) -> impl Iterator<Item = Option<Arrival>> + Clone + '_ {
    (start..=end).map(|seq| {
        recent[seq as usize % HISTORY]
            .filter(|(seen, _)| *seen == seq)
            .map(|(_, arrival)| arrival)
    })
}

fn train_gap(arrivals: impl Iterator<Item = Option<Arrival>> + Clone) -> Option<(usize, f64)> {
    let len = arrivals.clone().flatten().map(|sample| sample.len).max()?;
    let mut previous: Option<Arrival> = None;
    let mut intervals = 0;
    let mut seconds = 0.0;
    for arrival in arrivals {
        if let (Some(a), Some(b)) = (previous, arrival) {
            if a.len == len && b.len == len && b.at > a.at {
                intervals += 1;
                seconds += b.at.duration_since(a.at).as_secs_f64();
            }
        }
        previous = arrival;
    }
    (intervals >= MIN_INTERVALS).then(|| (len, seconds / f64::from(intervals)))
}

fn compare_probe(
    arrivals: impl Iterator<Item = Option<Arrival>>,
    len: usize,
    ordinary_packets: usize,
    padding_packets: usize,
) -> Option<ProbeTiming> {
    let mut sample = ProbeTiming {
        ordinary_packets,
        padding_packets,
        packet_bytes: len,
        intervals: 0,
        app_interval_sum_ns: 0,
        kernel_interval_sum_ns: Some(0),
        group_boundaries: 0,
        largest_boundary_ns: 0,
        reversed_kernel_intervals: 0,
    };
    let mut previous: Option<(Arrival, i64)> = None;
    for arrival in arrivals {
        let arrival = arrival?;
        let kernel_ns = arrival.kernel_ns?;
        if let Some((prior, prior_ns)) = previous {
            let gap = kernel_ns
                .checked_sub(prior_ns)
                .and_then(|ns| u64::try_from(ns).ok());
            if gap.is_none() {
                sample.reversed_kernel_intervals += 1;
            }
            if let Some(gap) = gap.filter(|gap| *gap > GROUP_GAP_NS) {
                sample.group_boundaries += 1;
                sample.largest_boundary_ns = sample.largest_boundary_ns.max(gap);
            }
            if prior.len == len && arrival.len == len && arrival.at > prior.at {
                sample.intervals += 1;
                sample.app_interval_sum_ns += arrival.at.duration_since(prior.at).as_nanos() as u64;
                sample.kernel_interval_sum_ns = sample
                    .kernel_interval_sum_ns
                    .zip(gap)
                    .and_then(|(sum, gap)| sum.checked_add(gap));
            }
        }
        previous = Some((arrival, kernel_ns));
    }
    (sample.intervals >= MIN_INTERVALS).then_some(sample)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn probe_feedback_uses_measured_equal_size_packet_intervals() {
        let mut bandwidth = Bandwidth::default();
        let start = Instant::now();
        let mut packet = [0; 1200];
        packet[..5].copy_from_slice(&[6, 1, 7, 5, 0]);
        packet[5..13].copy_from_slice(&4u64.to_le_bytes());
        for i in 0..5 {
            bandwidth.ordinary(
                i,
                1200,
                start + Duration::from_micros(i * 200),
                Some(i as i64 * 1_000),
            );
        }
        for i in 1..=7 {
            packet[1] = i;
            bandwidth.probe(
                &packet,
                4,
                start + Duration::from_micros((4 + u64::from(i)) * 200),
                Some((4 + i64::from(i)) * 1_000),
            );
            if i < 7 {
                assert!(bandwidth.take_feedback().is_none());
            }
        }
        let (low, high) = bandwidth.take_feedback().unwrap();
        assert!((low - 6_000_000.0).abs() < 0.01);
        assert!((low - high).abs() < 0.01);
        let timing = bandwidth.take_probe_timing().unwrap();
        assert_eq!(timing.intervals, 11);
        assert_eq!(timing.app_interval_sum_ns, 2_200_000);
        assert_eq!(timing.kernel_interval_sum_ns, Some(11_000));
        assert_eq!(timing.group_boundaries, 0);
        bandwidth.probe(&packet, 4, start + Duration::from_millis(5), None);
        assert!(bandwidth.take_feedback().is_none());
        assert!(bandwidth.take_probe_timing().is_none());
    }
}
