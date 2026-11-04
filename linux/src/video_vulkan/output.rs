use lowlat_decode::h264::dpb::{Dpb, Slot};

/// Consume pending outputs without retaining their parser-side surface leases.
/// Native decode waits for presentation completion before it reuses a surface;
/// reference pictures remain protected independently by the DPB.
pub fn latest(dpb: &mut Dpb) -> Option<Slot> {
    let mut latest = None;
    while let Some(output) = dpb.next_output() {
        dpb.taken(output.slot);
        latest = Some(output.slot);
    }
    latest
}
