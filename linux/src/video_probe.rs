use anyhow::{Context, Result};
use ash::{Entry, vk};
use std::ffi::CStr;

pub fn report() -> Result<Vec<String>> {
    // No device or decode session is created by the capability probe.
    unsafe {
        let entry = Entry::load().context("Vulkan loader unavailable")?;
        let application = vk::ApplicationInfo::default().api_version(vk::API_VERSION_1_3);
        let instance = entry.create_instance(
            &vk::InstanceCreateInfo::default().application_info(&application),
            None,
        )?;
        let result = adapters(&entry, &instance);
        instance.destroy_instance(None);
        result
    }
}

unsafe fn adapters(entry: &Entry, instance: &ash::Instance) -> Result<Vec<String>> {
    let video = ash::khr::video_queue::Instance::new(entry, instance);
    let mut lines = Vec::new();
    for gpu in unsafe { instance.enumerate_physical_devices()? } {
        let properties = unsafe { instance.get_physical_device_properties(gpu) };
        let name = unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }.to_string_lossy();
        let extensions = unsafe { instance.enumerate_device_extension_properties(gpu)? };
        let has = |name: &CStr| {
            extensions
                .iter()
                .any(|ext| (unsafe { CStr::from_ptr(ext.extension_name.as_ptr()) }) == name)
        };
        lines.push(name.into_owned());
        if properties.api_version < vk::API_VERSION_1_3
            || !has(ash::khr::video_queue::NAME)
            || !has(ash::khr::video_decode_queue::NAME)
        {
            lines.push("Vulkan 1.3 video decode extensions unavailable".into());
            continue;
        }

        let count = unsafe { instance.get_physical_device_queue_family_properties2_len(gpu) };
        let mut operations = vec![vk::QueueFamilyVideoPropertiesKHR::default(); count];
        let mut queues: Vec<_> = operations
            .iter_mut()
            .map(|op| vk::QueueFamilyProperties2::default().push_next(op))
            .collect();
        unsafe { instance.get_physical_device_queue_family_properties2(gpu, &mut queues) };
        let queue_flags: Vec<_> = queues
            .into_iter()
            .map(|queue| queue.queue_family_properties.queue_flags)
            .collect();
        let supported = queue_flags.into_iter().zip(&operations).fold(
            vk::VideoCodecOperationFlagsKHR::empty(),
            |flags, (queue, op)| {
                if queue.contains(vk::QueueFlags::VIDEO_DECODE_KHR) {
                    flags | op.video_codec_operations
                } else {
                    flags
                }
            },
        );
        let profile = |operation, depth| {
            vk::VideoProfileInfoKHR::default()
                .video_codec_operation(operation)
                .chroma_subsampling(vk::VideoChromaSubsamplingFlagsKHR::TYPE_420)
                .luma_bit_depth(depth)
                .chroma_bit_depth(depth)
        };
        let h264 = vk::VideoCodecOperationFlagsKHR::DECODE_H264;
        let h265 = vk::VideoCodecOperationFlagsKHR::DECODE_H265;
        let eight = vk::VideoComponentBitDepthFlagsKHR::TYPE_8;
        let ten = vk::VideoComponentBitDepthFlagsKHR::TYPE_10;
        if has(ash::khr::video_decode_h264::NAME) && supported.contains(h264) {
            let mut codec = vk::VideoDecodeH264ProfileInfoKHR::default()
                .std_profile_idc(vk::native::StdVideoH264ProfileIdc_STD_VIDEO_H264_PROFILE_IDC_HIGH)
                .picture_layout(vk::VideoDecodeH264PictureLayoutFlagsKHR::PROGRESSIVE);
            let profile = profile(h264, eight).push_next(&mut codec);
            let mut codec_caps = vk::VideoDecodeH264CapabilitiesKHR::default();
            unsafe {
                query(
                    &video,
                    gpu,
                    "H.264 High",
                    &profile,
                    &mut codec_caps,
                    &mut lines,
                )
            };
        } else {
            lines.push("H.264 High: decode queue unavailable".into());
        }
        if has(ash::khr::video_decode_h265::NAME) && supported.contains(h265) {
            for (label, depth, id) in [
                (
                    "HEVC Main",
                    eight,
                    vk::native::StdVideoH265ProfileIdc_STD_VIDEO_H265_PROFILE_IDC_MAIN,
                ),
                (
                    "HEVC Main 10",
                    ten,
                    vk::native::StdVideoH265ProfileIdc_STD_VIDEO_H265_PROFILE_IDC_MAIN_10,
                ),
            ] {
                let mut codec = vk::VideoDecodeH265ProfileInfoKHR::default().std_profile_idc(id);
                let profile = profile(h265, depth).push_next(&mut codec);
                let mut codec_caps = vk::VideoDecodeH265CapabilitiesKHR::default();
                unsafe { query(&video, gpu, label, &profile, &mut codec_caps, &mut lines) };
            }
        } else {
            lines.push("HEVC: decode queue unavailable".into());
        }
    }
    Ok(lines)
}

unsafe fn query(
    video: &ash::khr::video_queue::Instance,
    gpu: vk::PhysicalDevice,
    label: &str,
    profile: &vk::VideoProfileInfoKHR<'_>,
    codec: &mut impl vk::ExtendsVideoCapabilitiesKHR,
    lines: &mut Vec<String>,
) {
    let mut decode = vk::VideoDecodeCapabilitiesKHR::default();
    let mut caps = vk::VideoCapabilitiesKHR::default()
        .push_next(codec)
        .push_next(&mut decode);
    let status =
        unsafe { (video.fp().get_physical_device_video_capabilities_khr)(gpu, profile, &mut caps) };
    if status != vk::Result::SUCCESS {
        lines.push(format!("{label}: {status:?}"));
        return;
    }
    let profiles = [*profile];
    let mut list = vk::VideoProfileListInfoKHR::default().profiles(&profiles);
    let info = vk::PhysicalDeviceVideoFormatInfoKHR::default()
        .image_usage(vk::ImageUsageFlags::VIDEO_DECODE_DST_KHR | vk::ImageUsageFlags::SAMPLED)
        .push_next(&mut list);
    let mut count = 0;
    let status = unsafe {
        (video.fp().get_physical_device_video_format_properties_khr)(
            gpu,
            &info,
            &mut count,
            std::ptr::null_mut(),
        )
    };
    let sampling = match (status, count) {
        (vk::Result::SUCCESS, 0) => "no sampleable output format".into(),
        (vk::Result::SUCCESS, count) => format!("{count} sampleable output formats"),
        (status, _) => format!("output format query: {status:?}"),
    };
    lines.push(format!(
        "{label}: up to {} × {} · {sampling}",
        caps.max_coded_extent.width, caps.max_coded_extent.height
    ));
}
