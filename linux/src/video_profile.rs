//! One decode usage policy for capability queries and all session resources.
use ash::vk;

pub fn usage() -> vk::VideoDecodeUsageInfoKHR<'static> {
    // A use-case hint, not a latency guarantee or an extra decode buffer.
    vk::VideoDecodeUsageInfoKHR::default()
        .video_usage_hints(vk::VideoDecodeUsageFlagsKHR::STREAMING)
}

/// Required by the existing original-surface path, independent of GPU vendor.
pub fn original_surfaces(
    flags: vk::VideoCapabilityFlagsKHR,
    dpb: u32,
    references: u32,
    decode: vk::VideoDecodeCapabilityFlagsKHR,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        flags.contains(vk::VideoCapabilityFlagsKHR::SEPARATE_REFERENCE_IMAGES),
        "This driver requires a layered DPB; separate reference surfaces are unavailable"
    );
    anyhow::ensure!(
        decode.contains(vk::VideoDecodeCapabilityFlagsKHR::DPB_AND_OUTPUT_COINCIDE),
        "This driver requires distinct DPB/output images; that path is not integrated"
    );
    anyhow::ensure!(
        dpb >= 2 && references >= 1,
        "Insufficient Vulkan DPB capacity"
    );
    Ok(())
}
pub fn surface_usage() -> vk::ImageUsageFlags {
    vk::ImageUsageFlags::VIDEO_DECODE_DST_KHR
        | vk::ImageUsageFlags::VIDEO_DECODE_DPB_KHR
        | vk::ImageUsageFlags::SAMPLED
}
pub fn original_nv12(format: &vk::VideoFormatPropertiesKHR<'_>) -> bool {
    format.format == vk::Format::G8_B8R8_2PLANE_420_UNORM
        && format.image_type == vk::ImageType::TYPE_2D
        && format.image_tiling == vk::ImageTiling::OPTIMAL
        && format
            .image_create_flags
            .contains(vk::ImageCreateFlags::MUTABLE_FORMAT)
}
pub unsafe fn formats(
    video: &ash::khr::video_queue::Instance,
    gpu: vk::PhysicalDevice,
    profile: &vk::VideoProfileInfoKHR<'_>,
) -> anyhow::Result<Vec<vk::VideoFormatPropertiesKHR<'static>>> {
    let profiles = [*profile];
    let mut list = vk::VideoProfileListInfoKHR::default().profiles(&profiles);
    let info = vk::PhysicalDeviceVideoFormatInfoKHR::default()
        .image_usage(surface_usage())
        .push_next(&mut list);
    let mut count = 0;
    unsafe {
        (video.fp().get_physical_device_video_format_properties_khr)(
            gpu,
            &info,
            &mut count,
            std::ptr::null_mut(),
        )
        .result()?;
    }
    let mut formats = vec![vk::VideoFormatPropertiesKHR::default(); count as usize];
    if count > 0 {
        unsafe {
            (video.fp().get_physical_device_video_format_properties_khr)(
                gpu,
                &info,
                &mut count,
                formats.as_mut_ptr(),
            )
            .result()?;
        }
        formats.truncate(count as usize);
    }
    Ok(formats)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linux_surface_eligibility_is_vendor_independent_and_rejects_unsupported_dpb_modes() {
        let mut caps = vk::VideoCapabilitiesKHR::default()
            .flags(vk::VideoCapabilityFlagsKHR::SEPARATE_REFERENCE_IMAGES)
            .max_dpb_slots(16)
            .max_active_reference_pictures(15);
        let mut decode = vk::VideoDecodeCapabilitiesKHR::default()
            .flags(vk::VideoDecodeCapabilityFlagsKHR::DPB_AND_OUTPUT_COINCIDE);
        assert!(
            original_surfaces(
                caps.flags,
                caps.max_dpb_slots,
                caps.max_active_reference_pictures,
                decode.flags
            )
            .is_ok()
        );
        caps.flags = vk::VideoCapabilityFlagsKHR::empty();
        assert!(
            original_surfaces(
                caps.flags,
                caps.max_dpb_slots,
                caps.max_active_reference_pictures,
                decode.flags
            )
            .unwrap_err()
            .to_string()
            .contains("layered")
        );
        caps.flags = vk::VideoCapabilityFlagsKHR::SEPARATE_REFERENCE_IMAGES;
        decode.flags = vk::VideoDecodeCapabilityFlagsKHR::DPB_AND_OUTPUT_DISTINCT;
        assert!(
            original_surfaces(
                caps.flags,
                caps.max_dpb_slots,
                caps.max_active_reference_pictures,
                decode.flags
            )
            .unwrap_err()
            .to_string()
            .contains("distinct")
        );
        let mut format = vk::VideoFormatPropertiesKHR::default()
            .format(vk::Format::G8_B8R8_2PLANE_420_UNORM)
            .image_type(vk::ImageType::TYPE_2D)
            .image_tiling(vk::ImageTiling::OPTIMAL)
            .image_create_flags(vk::ImageCreateFlags::MUTABLE_FORMAT);
        assert!(original_nv12(&format));
        format.image_create_flags = vk::ImageCreateFlags::empty();
        assert!(!original_nv12(&format));
    }
}
