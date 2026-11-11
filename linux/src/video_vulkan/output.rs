use lowlat_decode::h264::dpb::{Dpb, Slot};

/// Isolated legacy/reference-chain fixture consumer.
#[cfg(test)]
pub fn latest(dpb: &mut Dpb) -> Option<Slot> {
    let mut latest = None;
    while let Some(output) = dpb.next_output() {
        dpb.taken(output.slot);
        latest = Some(output.slot);
    }
    latest
}

/// The parser already provides a spare slot for one retained output. Keep
/// exactly that selected image leased, independently of reference marking.
#[derive(Default)]
pub struct Selection {
    slot: Option<Slot>,
}
pub trait Buffer {
    fn next_slot(&mut self) -> Option<Slot>;
    fn release(&mut self, slot: Slot);
}
impl Buffer for Dpb {
    fn next_slot(&mut self) -> Option<Slot> {
        self.next_output().map(|output| output.slot)
    }
    fn release(&mut self, slot: Slot) {
        self.taken(slot);
    }
}
impl Buffer for lowlat_decode::hevc::dpb::Dpb {
    fn next_slot(&mut self) -> Option<Slot> {
        self.next_output().map(|output| output.slot)
    }
    fn release(&mut self, slot: Slot) {
        self.taken(slot);
    }
}
impl Selection {
    pub fn latest(&mut self, dpb: &mut impl Buffer) -> Option<Slot> {
        let mut changed = false;
        while let Some(slot) = dpb.next_slot() {
            if let Some(previous) = self.slot.replace(slot) {
                if previous != slot {
                    dpb.release(previous);
                }
            }
            changed = true;
        }
        if changed { self.slot } else { None }
    }
    pub fn clear(&mut self, dpb: &mut impl Buffer) {
        if let Some(slot) = self.slot.take() {
            dpb.release(slot);
        }
        while let Some(slot) = dpb.next_slot() {
            dpb.release(slot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Pending {
        outputs: std::collections::VecDeque<Slot>,
        released: Vec<Slot>,
    }
    impl Buffer for Pending {
        fn next_slot(&mut self) -> Option<Slot> {
            self.outputs.pop_front()
        }
        fn release(&mut self, slot: Slot) {
            self.released.push(slot);
        }
    }
    #[test]
    fn linux_surface_selection_retains_only_latest_and_preserves_no_output_picture() {
        let mut dpb = Pending::default();
        let mut selection = Selection::default();
        dpb.outputs.extend([0, 1, 2]);
        assert_eq!(selection.latest(&mut dpb), Some(2));
        assert_eq!(dpb.released, [0, 1]);
        assert_eq!(selection.latest(&mut dpb), None);
        assert_eq!(selection.slot, Some(2));
        assert_eq!(dpb.released, [0, 1]);
        dpb.outputs.push_back(3);
        assert_eq!(selection.latest(&mut dpb), Some(3));
        assert_eq!(dpb.released, [0, 1, 2]);
        dpb.outputs.push_back(4);
        selection.clear(&mut dpb);
        assert_eq!(selection.slot, None);
        assert_eq!(dpb.released, [0, 1, 2, 3, 4]);
    }
}
