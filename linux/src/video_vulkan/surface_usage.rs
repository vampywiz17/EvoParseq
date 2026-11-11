//! Fixed-size ownership/dependency bookkeeping for original decoder images.
//! CPU selection leases are held separately in output::Selection. GPU values
//! record submitted accesses, not estimates of driver execution or scanout.
use lowlat_decode::h264::dpb::MAX_FRAMES;

#[derive(Clone, Copy, Default)]
struct Surface {
    decode: u64,
    graphics: u64,
    reference: bool,
    render: bool,
}
#[derive(Default)]
pub struct Usage {
    surfaces: [Surface; MAX_FRAMES],
    target: Option<usize>,
    selected: Option<usize>,
    graphics: u64,
    staged: bool,
}
impl Usage {
    /// Last element is the destination; preceding elements are references.
    /// All are layout-transitioned by this native decode submission.
    pub fn decode_wait(&mut self, used: &[usize]) -> u64 {
        let (&target, references) = used.split_last().expect("Decode destination");
        assert!(target < MAX_FRAMES);
        self.target = Some(target);
        for surface in &mut self.surfaces {
            surface.reference = false;
        }
        for &reference in references {
            self.surfaces[reference].reference = true;
        }
        self.surfaces
            .iter()
            .enumerate()
            .filter(|(id, surface)| surface.reference || Some(*id) == self.target)
            .map(|(_, surface)| surface.graphics)
            .max()
            .unwrap_or(0)
    }
    pub fn decoded(&mut self, value: u64) {
        for (id, surface) in self.surfaces.iter_mut().enumerate() {
            if surface.reference || Some(id) == self.target {
                surface.decode = value;
            }
        }
    }
    pub fn select(&mut self, surface: usize) {
        assert!(surface < MAX_FRAMES);
        self.selected = Some(surface);
    }
    pub fn selected(&self, surface: usize) -> bool {
        self.selected == Some(surface)
    }
    pub fn prepare(&mut self, surface: usize) {
        if self.selected(surface) {
            self.surfaces[surface].render = true;
        }
    }
    pub fn stage(&mut self) -> Option<(u64, u64)> {
        if self.staged || !self.surfaces.iter().any(|surface| surface.render) {
            return None;
        }
        self.graphics = self
            .graphics
            .checked_add(1)
            .expect("GPU timeline exhausted");
        self.staged = true;
        let mut ready = 0;
        for surface in &mut self.surfaces {
            if surface.render {
                ready = ready.max(surface.decode);
                surface.graphics = self.graphics;
            }
        }
        Some((ready, self.graphics))
    }
    pub fn finish(&mut self) {
        self.staged = false;
        for surface in &mut self.surfaces {
            surface.render = false;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linux_surface_waits_exclude_unrelated_renders_but_include_reference_transitions() {
        let mut usage = Usage::default();
        assert_eq!(usage.decode_wait(&[0]), 0);
        usage.decoded(1);
        usage.select(0);
        usage.prepare(0);
        assert_eq!(usage.stage(), Some((1, 1)));
        usage.finish();
        // Decoding another image does not wait for image 0's GPU reader.
        assert_eq!(usage.decode_wait(&[1]), 0);
        usage.decoded(2);
        usage.select(1);
        usage.prepare(1);
        assert_eq!(usage.stage(), Some((2, 2)));
        usage.finish();
        // Image 0 is only a reference, but its layout still changes. Wait on
        // its reader (1), not the unrelated later render of image 1 (2).
        assert_eq!(usage.decode_wait(&[0, 2]), 1);
        usage.decoded(3);
        usage.select(0);
        usage.prepare(0);
        assert_eq!(usage.stage(), Some((3, 3)));
        usage.finish();
        assert_eq!(usage.decode_wait(&[0]), 3);
    }
    #[test]
    fn linux_surface_selection_and_repeated_callbacks_do_not_duplicate_signals() {
        let mut usage = Usage::default();
        usage.decode_wait(&[0]);
        usage.decoded(1);
        usage.select(0);
        usage.prepare(1); // stale callback is not an active reader
        assert_eq!(usage.stage(), None);
        usage.prepare(0);
        usage.prepare(0);
        assert_eq!(usage.stage(), Some((1, 1)));
        assert_eq!(usage.stage(), None);
        usage.finish();
        usage.prepare(0);
        assert_eq!(usage.stage(), Some((1, 2)));
        usage.finish();
        assert_eq!(usage.decode_wait(&[0]), 2);
    }
    #[test]
    fn linux_surface_multi_callback_submit_waits_for_all_recorded_readers() {
        let mut usage = Usage::default();
        usage.decode_wait(&[0]);
        usage.decoded(1);
        usage.select(0);
        usage.prepare(0);
        usage.decode_wait(&[1]);
        usage.decoded(2);
        usage.select(1);
        usage.prepare(1);
        assert_eq!(usage.stage(), Some((2, 1)));
        assert_eq!(usage.stage(), None);
        usage.finish();
        assert_eq!(usage.decode_wait(&[0]), 1);
        assert_eq!(usage.decode_wait(&[1]), 1);
    }
}
