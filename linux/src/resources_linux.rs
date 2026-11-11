//! Optional process-only resource reads, at most 1Hz while Connection stats is open.
//! No GPU probe, subprocess, periodic thread, smaps walk or render instrumentation.
use std::time::Instant;
#[derive(Default)]
pub struct Sampler {
    previous: Option<(Instant, f64)>,
    cpus: Option<usize>,
    pub cpu: Option<f64>,
    pub resident: Option<u64>,
}
impl Sampler {
    pub fn pause(&mut self) {
        self.previous = None;
        self.cpu = None;
        self.cpus = None;
    }
    pub fn update(&mut self, now: Instant) {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } == 0 {
            let usage = unsafe { usage.assume_init() };
            let time = usage.ru_utime.tv_sec as f64
                + usage.ru_stime.tv_sec as f64
                + (usage.ru_utime.tv_usec + usage.ru_stime.tv_usec) as f64 / 1e6;
            let cpus = *self
                .cpus
                .get_or_insert_with(|| std::thread::available_parallelism().map_or(1, usize::from));
            self.cpu = self.previous.and_then(|(at, old)| {
                let seconds = now.saturating_duration_since(at).as_secs_f64();
                let cpus = cpus as f64;
                (seconds > 0. && time >= old)
                    .then(|| ((time - old) / seconds / cpus * 100.).clamp(0., 100.))
            });
            self.previous = Some((now, time));
        } else {
            self.previous = None;
            self.cpu = None;
        }
        self.resident = std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|text| resident(&text));
    }
}
fn resident(text: &str) -> Option<u64> {
    let line = text.lines().find_map(|l| l.strip_prefix("VmRSS:"))?;
    let mut fields = line.split_whitespace();
    let kib: u64 = fields.next()?.parse().ok()?;
    if fields.next() != Some("kB") {
        return None;
    }
    kib.checked_mul(1024)
}
#[cfg(test)]
mod tests {
    #[test]
    fn linux_rss_parser_does_not_substitute_virtual_or_anonymous_memory() {
        assert_eq!(
            super::resident("VmSize: 900 kB\nRssAnon: 100 kB\nVmRSS: 400 kB"),
            Some(409600)
        );
        assert_eq!(super::resident("VmSize: 900 kB"), None);
        assert_eq!(super::resident("VmRSS: 1 MB"), None);
    }
}
