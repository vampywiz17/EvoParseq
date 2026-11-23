//! Optional stock-host telemetry, consumed before application control queues.
//! Host durations are not end-to-end latency or GPU execution measurements.
use std::{sync::Mutex, time::Instant};

pub fn request(enabled: bool) -> [u8; 13] {
    let mut bytes = [0; 13];
    bytes[3] = if enabled { 3 } else { 0 };
    bytes[12] = 35;
    bytes
}

pub fn is_report(bytes: &[u8]) -> bool {
    bytes.get(12) == Some(&34)
}

#[derive(Clone)]
pub struct Series {
    pub count: u64,
    pub last: [f64; 4],
    pub mean: [f64; 4],
    pub peak: [f64; 4],
    pub at: Instant,
}
impl Series {
    fn push(&mut self, values: [f64; 4]) {
        self.count = self.count.saturating_add(1);
        self.at = Instant::now();
        self.last = values;
        for (i, value) in values.into_iter().enumerate() {
            self.mean[i] += (value - self.mean[i]) / self.count as f64;
            self.peak[i] = self.peak[i].max(value);
        }
    }
    fn new(values: [f64; 4]) -> Self {
        Self {
            count: 1,
            last: values,
            mean: values,
            peak: values,
            at: Instant::now(),
        }
    }
}

#[derive(Clone, Default)]
pub struct Snapshot {
    pub requested: bool,
    pub request_failed: bool,
    pub video: [Option<Series>; 5],
    pub audio: Option<Series>,
    pub rejected: u64,
    pub unknown: u64,
}
#[derive(Default)]
struct State {
    generation: u64,
    report: Snapshot,
}
#[derive(Default)]
pub struct Shared(Mutex<State>);
impl Shared {
    pub fn begin(&self, generation: u64) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        // A retiring LOWLAT worker may still reach its route-retry reset after a
        // replacement connection has begun. It must not rewind the collector.
        if generation < state.generation {
            return;
        }
        *state = State {
            generation,
            ..Default::default()
        };
    }
    pub fn current(&self, generation: u64) -> bool {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).generation == generation
    }
    pub fn set_requested(&self, generation: u64, enabled: bool) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.generation == generation {
            if enabled && !state.report.requested {
                state.report = Snapshot::default();
            }
            state.report.requested = enabled;
        }
    }
    pub fn failed(&self, generation: u64) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.generation == generation {
            state.report.requested = false;
            state.report.request_failed = true;
        }
    }
    pub fn end(&self, generation: u64) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.generation == generation {
            state.report = Snapshot::default();
        }
    }
    pub fn snapshot(&self, generation: u64) -> Snapshot {
        let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.generation == generation {
            state.report.clone()
        } else {
            Snapshot::default()
        }
    }
    /// All opcode-34 frames are consumed, including unsupported future variants.
    /// Malformed diagnostics never fail the media connection or fill its queue.
    pub fn receive(&self, generation: u64, bytes: &[u8]) -> bool {
        if !is_report(bytes) {
            return false;
        }
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.generation != generation || !state.report.requested {
            return true;
        }
        let word = |offset| u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap());
        let (kind, stream, reserved) = (word(0), word(4), word(8));
        let fields = match (kind, stream, reserved) {
            (0, 0..=4, 0) => 4,
            (1, 0, 0) => 3,
            _ => {
                state.report.unknown = state.report.unknown.saturating_add(1);
                return true;
            }
        };
        if bytes.len() != 13 + fields * 4 {
            state.report.rejected = state.report.rejected.saturating_add(1);
            return true;
        }
        let mut values = [0.; 4];
        for (i, value) in values[..fields].iter_mut().enumerate() {
            *value = f64::from(f32::from_bits(word(13 + i * 4)));
            if !value.is_finite() || *value < 0. {
                state.report.rejected = state.report.rejected.saturating_add(1);
                return true;
            }
        }
        let slot = if kind == 0 {
            &mut state.report.video[stream as usize]
        } else {
            &mut state.report.audio
        };
        if let Some(series) = slot {
            series.push(values);
        } else {
            *slot = Some(Series::new(values));
        }
        true
    }
}

/// One transition per connection; a diagnostic send failure is nonfatal and
/// disables further attempts until the connection is replaced.
#[derive(Default)]
pub struct Requests {
    enabled: bool,
    failed: bool,
}
impl Requests {
    pub fn update(&mut self, desired: bool) -> Option<[u8; 13]> {
        if self.failed || desired == self.enabled {
            return None;
        }
        self.enabled = desired;
        Some(request(desired))
    }
    pub fn fail(&mut self) {
        self.failed = true;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub fn packet(kind: u32, stream: u32, values: &[f32]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for value in [kind, stream, 0] {
            bytes.extend(value.to_be_bytes());
        }
        bytes.push(34);
        for value in values {
            bytes.extend(value.to_be_bytes());
        }
        bytes
    }
    #[test]
    fn wire_variants_aggregate_without_confusing_audio_and_video() {
        let shared = Shared::default();
        shared.begin(1);
        shared.set_requested(1, true);
        for _ in 0..10_000 {
            assert!(shared.receive(1, &packet(0, 0, &[10., 2., 16., 3.])));
        }
        shared.receive(1, &packet(0, 0, &[20., 4., 32., 6.]));
        shared.receive(1, &packet(0, 4, &[1., 2., 3., 4.]));
        shared.receive(1, &packet(1, 0, &[5., 0., 20.]));
        let s = shared.snapshot(1);
        let video = s.video[0].as_ref().unwrap();
        assert_eq!(video.count, 10_001);
        assert_eq!(video.peak, [20., 4., 32., 6.]);
        assert!((video.mean[0] - (100_020. / 10_001.)).abs() < 1e-8);
        assert_eq!(s.video[4].as_ref().unwrap().last, [1., 2., 3., 4.]);
        assert_eq!(s.audio.unwrap().last, [5., 0., 20., 0.]);
        assert!(s.video[1..4].iter().all(Option::is_none));
    }
    #[test]
    fn invalid_reports_are_isolated_and_old_sessions_cannot_pollute_new_data() {
        let shared = Shared::default();
        shared.begin(7);
        shared.set_requested(7, true);
        for value in [f32::NAN, f32::INFINITY, -1.] {
            shared.receive(7, &packet(0, 0, &[value, 0., 0., 0.]));
        }
        shared.receive(7, &packet(0, 0, &[1., 2., 3.]));
        shared.receive(7, &packet(0, 5, &[1., 2., 3., 4.]));
        assert_eq!(shared.snapshot(7).rejected, 4);
        assert_eq!(shared.snapshot(7).unknown, 1);
        assert!(shared.snapshot(7).video.iter().all(Option::is_none));
        shared.begin(8);
        shared.set_requested(8, true);
        shared.begin(7);
        shared.receive(7, &packet(1, 0, &[1., 2., 3.]));
        shared.end(7);
        shared.failed(7);
        assert!(shared.snapshot(8).requested);
        assert!(shared.snapshot(8).audio.is_none());
        shared.set_requested(8, false);
        shared.receive(8, &packet(1, 0, &[1., 2., 3.]));
        assert!(shared.snapshot(8).audio.is_none());
        assert!(!shared.receive(8, &[0; 12]));
    }
    #[test]
    fn requests_only_follow_collection_transitions_and_fail_nonfatally() {
        let mut requests = Requests::default();
        assert!(requests.update(false).is_none());
        assert_eq!(
            requests.update(true).unwrap(),
            [0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 35]
        );
        assert!(requests.update(true).is_none());
        assert_eq!(requests.update(false).unwrap(), request(false));
        requests.fail();
        assert!(requests.update(true).is_none());
    }
}
