//! Same-device decode/render dependencies. No image copies or CPU waits.
use super::device::Context;
use anyhow::Result;
use ash::vk;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

pub struct Sync {
    context: Arc<Context>,
    pub decode: vk::Semaphore,
    pub graphics: vk::Semaphore,
    decoded: AtomicU64,
    state: Mutex<super::surface_usage::Usage>,
}
impl Sync {
    pub fn new(context: Arc<Context>) -> Result<Arc<Self>> {
        let mut kind =
            vk::SemaphoreTypeCreateInfo::default().semaphore_type(vk::SemaphoreType::TIMELINE);
        let info = vk::SemaphoreCreateInfo::default().push_next(&mut kind);
        let decode = unsafe { context.raw.create_semaphore(&info, None)? };
        let graphics = match unsafe { context.raw.create_semaphore(&info, None) } {
            Ok(value) => value,
            Err(error) => {
                unsafe { context.raw.destroy_semaphore(decode, None) };
                return Err(error.into());
            }
        };
        Ok(Arc::new(Self {
            context,
            decode,
            graphics,
            decoded: AtomicU64::new(0),
            state: Mutex::new(super::surface_usage::Usage::default()),
        }))
    }
    pub fn defer_backlog(self: &Arc<Self>, playback: &crate::playback::Playback) {
        if !playback.has_queued_video() {
            return;
        }
        let value = self.decoded.load(Ordering::Acquire);
        let sync = self.clone();
        let started = std::time::Instant::now();
        playback.defer_gpu(move || {
            // A driver error/timeout also wakes the UI to report it via reap().
            unsafe { sync.context.raw.get_semaphore_counter_value(sync.decode) }
                .map_or(true, |done| done >= value)
                || started.elapsed() >= std::time::Duration::from_millis(100)
        });
    }
    pub fn next_decode(&self) -> u64 {
        self.decoded
            .load(Ordering::Acquire)
            .checked_add(1)
            .expect("GPU timeline exhausted")
    }
    pub fn submitted(&self, value: u64) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .decoded(value);
        self.decoded.store(value, Ordering::Release);
    }
    pub fn graphics_value(&self, used: &[usize]) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .decode_wait(used)
    }
    pub fn select(&self, surface: usize) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .select(surface);
    }
    pub fn selected(&self, surface: usize) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .selected(surface)
    }
    pub fn prepare_submit(&self, surface: usize) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .prepare(surface);
    }
    pub fn before_submit(&self, queue: &wgpu::Queue) {
        let Some((ready, value)) = self.state.lock().unwrap_or_else(|e| e.into_inner()).stage()
        else {
            return;
        };
        // Called only from the Linux painter's immediate submit hook. No
        // other submission/mapping can consume the staged dependencies.
        let hal =
            unsafe { queue.as_hal::<wgpu::hal::api::Vulkan>() }.expect("Vulkan graphics queue");
        hal.add_wait_semaphore(
            self.decode,
            Some(ready),
            vk::PipelineStageFlags::ALL_COMMANDS,
        );
        hal.add_signal_semaphore(self.graphics, Some(value));
    }
    pub fn after_submit(&self) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .finish();
    }
}
impl Drop for Sync {
    fn drop(&mut self) {
        // A callback can retain these semaphores beyond Session teardown.
        // Its last owner must also drain any later render submission. This
        // wait happens only at teardown, never in the per-picture path.
        if self.context.wait_graphics().is_err() {
            struct Retired {
                _context: Arc<Context>,
                _semaphores: [vk::Semaphore; 2],
            }
            static RETIRED: std::sync::OnceLock<Mutex<Vec<Retired>>> = std::sync::OnceLock::new();
            RETIRED
                .get_or_init(Default::default)
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(Retired {
                    _context: self.context.clone(),
                    _semaphores: [self.decode, self.graphics],
                });
            return;
        }
        unsafe {
            self.context.raw.destroy_semaphore(self.decode, None);
            self.context.raw.destroy_semaphore(self.graphics, None);
        }
    }
}
