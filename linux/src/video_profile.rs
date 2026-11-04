//! One decode usage policy for capability queries and all session resources.
use ash::vk;

pub fn usage() -> vk::VideoDecodeUsageInfoKHR<'static> {
    // A use-case hint, not a latency guarantee or an extra decode buffer.
    vk::VideoDecodeUsageInfoKHR::default()
        .video_usage_hints(vk::VideoDecodeUsageFlagsKHR::STREAMING)
}
