//! Same-device decode/render dependencies. No image copies or CPU waits.
use super::device::Context;
use anyhow::Result;
use ash::vk;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

#[derive(Default)]
struct State {
    graphics: u64,
    staged: bool,
}
impl State {
    fn stage(&mut self) -> Option<u64> {
        if self.staged {
            return None;
        }
        self.graphics = self
            .graphics
            .checked_add(1)
            .expect("GPU timeline exhausted");
        self.staged = true;
        Some(self.graphics)
    }
}
pub struct Sync {
    context: Arc<Context>,
    pub decode: vk::Semaphore,
    pub graphics: vk::Semaphore,
    decoded: AtomicU64,
    state: Mutex<State>,
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
            state: Mutex::new(State::default()),
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
        self.decoded.store(value, Ordering::Release);
    }
    pub fn graphics_value(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .graphics
    }
    pub fn before_submit(&self, queue: &wgpu::Queue) {
        let Some(value) = self.state.lock().unwrap_or_else(|e| e.into_inner()).stage() else {
            return;
        };
        // Called only from the Linux painter's immediate submit hook. No
        // other submission/mapping can consume the staged dependencies.
        let hal =
            unsafe { queue.as_hal::<wgpu::hal::api::Vulkan>() }.expect("Vulkan graphics queue");
        hal.add_wait_semaphore(
            self.decode,
            Some(self.decoded.load(Ordering::Acquire)),
            vk::PipelineStageFlags::ALL_COMMANDS,
        );
        hal.add_signal_semaphore(self.graphics, Some(value));
    }
    pub fn after_submit(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).staged = false;
    }
}
impl Drop for Sync {
    fn drop(&mut self) {
        // Session teardown drains decode and graphics before the final owner
        // is dropped; callbacks also retain this object through their submit.
        unsafe {
            self.context.raw.destroy_semaphore(self.decode, None);
            self.context.raw.destroy_semaphore(self.graphics, None);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linux_sync_repeated_callbacks_signal_once_per_submission() {
        let mut state = State::default();
        assert_eq!(state.stage(), Some(1));
        assert_eq!(state.stage(), None);
        state.staged = false;
        assert_eq!(state.stage(), Some(2));
    }
}
