//! Wait-free ownership handoff replacing the mutex PCM queue for pull outputs.
//! Only the native callback reads Ready blocks; only the Opus worker publishes.
use super::{Layout, Snapshot, RECYCLED_BUFFERS};
use rtrb::{Consumer, Producer, PushError, RingBuffer};
use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    Arc,
};

struct Ready {
    pcm: Vec<i16>,
    layout: Layout,
    epoch: u64,
}
#[derive(Default)]
struct Shared {
    epoch: AtomicU64,
    queued: AtomicUsize,
    frames: AtomicUsize,
    consumed: AtomicU64,
    discarded: AtomicU64,
}
pub(super) struct Publisher {
    ready: Producer<Ready>,
    returned: Consumer<Vec<i16>>,
    shared: Arc<Shared>,
}
pub struct CallbackReader {
    ready: Consumer<Ready>,
    returned: Producer<Vec<i16>>,
    shared: Arc<Shared>,
    pending: Option<(Ready, usize)>,
    buffering: bool,
}
#[derive(Default)]
pub struct Rendered {
    pub frames: usize,
    pub invalid_layout: bool,
}
pub(super) fn new() -> (Publisher, CallbackReader) {
    let (tx, rx) = RingBuffer::new(RECYCLED_BUFFERS);
    let (recycle, recycled) = RingBuffer::new(RECYCLED_BUFFERS);
    let shared = Arc::new(Shared::default());
    (
        Publisher {
            ready: tx,
            returned: recycled,
            shared: shared.clone(),
        },
        CallbackReader {
            ready: rx,
            returned: recycle,
            shared,
            pending: None,
            buffering: true,
        },
    )
}
impl Publisher {
    pub fn push(&mut self, pcm: Vec<i16>, layout: Layout, budget: usize) -> Result<(), Vec<i16>> {
        let samples = pcm.len();
        if samples > budget
            || self
                .shared
                .queued
                .load(Ordering::Acquire)
                .saturating_add(samples)
                > budget
        {
            // Retire old PCM on the next device callback, without touching the
            // Opus reference state or blocking the real-time thread.
            self.discard();
            return Err(pcm);
        }
        let epoch = self.shared.epoch.load(Ordering::Acquire);
        self.shared.queued.fetch_add(samples, Ordering::Relaxed);
        self.shared
            .frames
            .fetch_add(samples / usize::from(layout.channels), Ordering::Relaxed);
        if let Err(PushError::Full(block)) = self.ready.push(Ready { pcm, layout, epoch }) {
            self.shared.queued.fetch_sub(samples, Ordering::Relaxed);
            self.shared
                .frames
                .fetch_sub(samples / usize::from(layout.channels), Ordering::Relaxed);
            self.discard();
            return Err(block.pcm);
        }
        Ok(())
    }
    pub fn recycled(&mut self) -> Option<Vec<i16>> {
        self.returned.pop().ok()
    }
    pub fn discard(&self) {
        self.shared.epoch.fetch_add(1, Ordering::Release);
    }
    pub fn snapshot(&self, report: &mut Snapshot) {
        report.queued_samples = self.shared.queued.load(Ordering::Relaxed);
        report.queued_frames = self.shared.frames.load(Ordering::Relaxed);
        report.pcm_frames_consumed += self.shared.consumed.load(Ordering::Relaxed);
        report.packets_dropped += self.shared.discarded.load(Ordering::Relaxed);
    }
}
impl CallbackReader {
    /// Called only with the old native callbacks stopped, before reopening output.
    pub fn restart(&mut self) {
        self.buffering = true;
    }

    /// No mutex, allocation, deallocation, native call or decoding here.
    /// PCM is copied once, directly into the device-owned output buffer.
    pub fn render(&mut self, output: &mut [i16]) -> Rendered {
        self.render_layout(output, Layout::STEREO, 0)
    }
    /// Explicit channel positions belong to the native output adapter. A stereo
    /// host fallback fills only FL/FR, never fabricated surround channels.
    pub fn render_layout(
        &mut self,
        output: &mut [i16],
        layout: Layout,
        minimum_frames: usize,
    ) -> Rendered {
        let channels = usize::from(layout.channels);
        let mut result = Rendered {
            invalid_layout: !output.len().is_multiple_of(channels),
            ..Default::default()
        };
        let mut filled = 0;
        // Bound stale-block retirement as well as useful audio production.
        for _ in 0..=(RECYCLED_BUFFERS * 2 + 1) {
            if result.invalid_layout || filled == output.len() {
                break;
            }
            if let Some((block, offset)) = self.pending.as_mut() {
                if block.epoch != self.shared.epoch.load(Ordering::Acquire)
                    && *offset != block.pcm.len()
                {
                    self.shared
                        .queued
                        .fetch_sub(block.pcm.len() - *offset, Ordering::Relaxed);
                    self.shared.frames.fetch_sub(
                        (block.pcm.len() - *offset) / usize::from(block.layout.channels),
                        Ordering::Relaxed,
                    );
                    self.shared.discarded.fetch_add(1, Ordering::Relaxed);
                    *offset = block.pcm.len();
                }
                if *offset == block.pcm.len() {
                    let (block, _) = self.pending.take().unwrap();
                    if let Err(PushError::Full(pcm)) = self.returned.push(block.pcm) {
                        // Retain ownership until the decoder makes recycling
                        // room; never drop/deallocate a Vec in the callback.
                        let offset = pcm.len();
                        self.pending = Some((
                            Ready {
                                pcm,
                                layout: block.layout,
                                epoch: block.epoch,
                            },
                            offset,
                        ));
                        break;
                    }
                }
            }
            if self.pending.is_none() {
                let Ok(block) = self.ready.pop() else {
                    break;
                };
                self.pending = Some((block, 0));
                // Validate a newly received block's epoch before copying it.
                continue;
            }
            let (block, offset) = self.pending.as_mut().unwrap();
            if block.layout != layout && block.layout != Layout::STEREO && layout != Layout::STEREO
            {
                result.invalid_layout = true;
                break;
            }
            // Retire stale epochs before testing the startup/refill threshold.
            if self.buffering && self.shared.frames.load(Ordering::Acquire) < minimum_frames {
                break;
            }
            self.buffering = false;
            let source_channels = usize::from(block.layout.channels);
            let frames = ((output.len() - filled) / channels)
                .min((block.pcm.len() - *offset) / source_channels);
            let count = frames * source_channels;
            if block.layout == layout {
                output[filled..filled + count]
                    .copy_from_slice(&block.pcm[*offset..*offset + count]);
            } else if layout == Layout::STEREO {
                for (dst, src) in output[filled..filled + frames * channels]
                    .chunks_exact_mut(channels)
                    .zip(block.pcm[*offset..*offset + count].chunks_exact(source_channels))
                {
                    dst.copy_from_slice(&block.layout.stereo(src));
                }
            } else {
                for (dst, src) in output[filled..filled + frames * channels]
                    .chunks_exact_mut(channels)
                    .zip(block.pcm[*offset..*offset + count].chunks_exact(source_channels))
                {
                    dst.fill(0);
                    dst[..2].copy_from_slice(src);
                }
            }
            filled += frames * channels;
            *offset += count;
            self.shared.queued.fetch_sub(count, Ordering::Relaxed);
            self.shared.frames.fetch_sub(frames, Ordering::Relaxed);
        }
        self.buffering = filled < output.len();
        result.frames = filled / channels;
        self.shared
            .consumed
            .fetch_add(result.frames as u64, Ordering::Relaxed);
        output[filled..].fill(0);
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn push(p: &mut Publisher, values: &[i16]) {
        assert!(p.push(values.to_vec(), Layout::STEREO, 9600).is_ok());
    }
    #[test]
    fn partial_blocks_keep_order_and_recycle_original_allocations() {
        let (mut publisher, mut reader) = new();
        let first = vec![1, -1, 2, -2];
        let pointer = first.as_ptr();
        publisher.push(first, Layout::STEREO, 9600).unwrap();
        push(&mut publisher, &[i16::MIN, i16::MAX]);
        let mut output = [0; 2];
        assert_eq!(reader.render(&mut output).frames, 1);
        assert_eq!(output, [1, -1]);
        let mut output = [99; 6];
        let report = reader.render(&mut output);
        assert_eq!(output, [2, -2, i16::MIN, i16::MAX, 0, 0]);
        assert_eq!(report.frames, 2);
        let recycled = publisher.recycled().unwrap();
        assert_eq!(recycled.as_ptr(), pointer);
        let mut snapshot = Snapshot::default();
        publisher.snapshot(&mut snapshot);
        assert_eq!(snapshot.queued_samples, 0);
        assert_eq!(snapshot.pcm_frames_consumed, 3);
    }
    #[test]
    fn a_busy_producer_state_cannot_block_callback_and_recovery_discards_partial_pcm() {
        let state = super::super::NativeReceiver {
            state: Arc::new(std::sync::Mutex::new(super::super::State::default())),
        };
        let mut reader = state.callback_reader().unwrap();
        let mut held = state.state.lock().unwrap();
        super::super::stage_pcm(&mut held, vec![1, -1, 2, -2], 9600);
        let mut output = [0; 2];
        reader.render(&mut output); // state mutex deliberately remains locked
        assert_eq!(output, [1, -1]);
        held.discard_pending();
        super::super::stage_pcm(&mut held, vec![3, -3], 9600);
        reader.render(&mut output);
        assert_eq!(output, [3, -3]);
    }
    #[test]
    fn a_full_return_fifo_retains_ownership_until_producer_recycles() {
        let (mut publisher, mut reader) = new();
        let mut output = [0; 2];
        for _ in 0..RECYCLED_BUFFERS {
            push(&mut publisher, &[1, -1]);
            reader.render(&mut output);
        }
        push(&mut publisher, &[2, -2]);
        reader.render(&mut output);
        push(&mut publisher, &[3, -3]);
        assert_eq!(reader.render(&mut output).frames, 0);
        assert!(reader.pending.is_some());
        publisher.recycled();
        assert_eq!(reader.render(&mut output).frames, 1);
        assert_eq!(output, [3, -3]);
    }
    #[test]
    fn multichannel_keeps_frames_and_positions_and_stereo_fallback_has_no_fake_surround() {
        let (mut publisher, mut reader) = new();
        let layout = Layout::new(6, 0x3f).unwrap();
        publisher
            .push(vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12], layout, 9600)
            .unwrap();
        let mut output = [99; 6];
        assert_eq!(
            reader.render_layout(&mut output, layout, 2).frames,
            2.min(output.len() / 6)
        );
        assert_eq!(output, [1, 2, 3, 4, 5, 6]);
        assert_eq!(reader.render_layout(&mut output, layout, 2).frames, 1);
        assert_eq!(output, [7, 8, 9, 10, 11, 12]);
        reader.render_layout(&mut output, layout, 2);
        publisher.push(vec![20, 21], Layout::STEREO, 9600).unwrap();
        assert_eq!(reader.render_layout(&mut output, layout, 2).frames, 0);
        assert_eq!(output, [0; 6]);
        publisher.push(vec![22, 23], Layout::STEREO, 9600).unwrap();
        assert_eq!(reader.render_layout(&mut output, layout, 2).frames, 1);
        assert_eq!(output, [20, 21, 0, 0, 0, 0]);
    }
    #[test]
    fn recovery_retires_old_partial_blocks_before_refilling_minimum() {
        let (mut publisher, mut reader) = new();
        push(&mut publisher, &[1, -1, 2, -2]);
        let mut output = [0; 2];
        assert_eq!(
            reader.render_layout(&mut output, Layout::STEREO, 2).frames,
            1
        );
        publisher.discard();
        reader.restart();
        push(&mut publisher, &[3, -3]);
        assert_eq!(
            reader.render_layout(&mut output, Layout::STEREO, 2).frames,
            0
        );
        assert_eq!(output, [0, 0]);
        push(&mut publisher, &[4, -4]);
        assert_eq!(
            reader.render_layout(&mut output, Layout::STEREO, 2).frames,
            1
        );
        assert_eq!(output, [3, -3]);
        assert_eq!(
            reader.render_layout(&mut output, Layout::STEREO, 2).frames,
            1
        );
        assert_eq!(output, [4, -4]);
    }
    #[test]
    fn route_recovery_can_fold_surround_down_without_reordering_following_blocks() {
        let (mut publisher, mut reader) = new();
        let layout = Layout::new(8, 0x63f).unwrap();
        publisher
            .push(vec![100, 200, 0, 30000, 0, 0, 0, 0], layout, 9600)
            .unwrap();
        push(&mut publisher, &[300, 400]);
        let mut output = [0; 4];
        assert_eq!(
            reader.render_layout(&mut output, Layout::STEREO, 0).frames,
            2
        );
        assert_eq!(output, [100, 200, 300, 400]);
        let mut report = Snapshot::default();
        publisher.snapshot(&mut report);
        assert_eq!(report.queued_frames, 0);
        assert_eq!(report.queued_samples, 0);
    }
    #[test]
    fn overflow_retires_stale_audio_and_never_exceeds_sample_budget() {
        let (mut publisher, mut reader) = new();
        publisher.push(vec![1; 8], Layout::STEREO, 8).unwrap();
        assert!(publisher.push(vec![2; 2], Layout::STEREO, 8).is_err());
        let mut output = [9; 8];
        assert_eq!(reader.render(&mut output).frames, 0);
        assert_eq!(output, [0; 8]);
        publisher.push(vec![3; 2], Layout::STEREO, 8).unwrap();
        assert_eq!(reader.render(&mut output).frames, 1);
        assert_eq!(output, [3, 3, 0, 0, 0, 0, 0, 0]);
    }
}
