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
        lines.push(format!(
            "{name} [vendor {:04x}, device {:04x}, API {}.{}.{}]",
            properties.vendor_id,
            properties.device_id,
            vk::api_version_major(properties.api_version),
            vk::api_version_minor(properties.api_version),
            vk::api_version_patch(properties.api_version)
        ));
        let mut driver = vk::PhysicalDeviceDriverProperties::default();
        let mut properties2 = vk::PhysicalDeviceProperties2::default().push_next(&mut driver);
        unsafe {
            instance.get_physical_device_properties2(gpu, &mut properties2);
        }
        let driver_name = unsafe { CStr::from_ptr(driver.driver_name.as_ptr()) }.to_string_lossy();
        let driver_info = unsafe { CStr::from_ptr(driver.driver_info.as_ptr()) }.to_string_lossy();
        lines.push(format!("Driver: {driver_name} · {driver_info}"));
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
            let mut usage = crate::video_profile::usage();
            let profile = profile(h264, eight)
                .push_next(&mut codec)
                .push_next(&mut usage);
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
                let mut usage = crate::video_profile::usage();
                let profile = profile(h265, depth)
                    .push_next(&mut codec)
                    .push_next(&mut usage);
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
    let flags = caps.flags;
    let (width, height, dpb, references) = (
        caps.max_coded_extent.width,
        caps.max_coded_extent.height,
        caps.max_dpb_slots,
        caps.max_active_reference_pictures,
    );
    let decode_flags = decode.flags;
    lines.push(format!("{label}: up to {width} × {height} · DPB {dpb} / references {references} · {flags:?} · {decode_flags:?}"));
    if profile.luma_bit_depth != vk::VideoComponentBitDepthFlagsKHR::TYPE_8 {
        lines.push("10-bit playback is not integrated on Linux".into());
        return;
    }
    let result = (|| -> anyhow::Result<()> {
        crate::video_profile::original_surfaces(flags, dpb, references, decode_flags)?;
        let formats = unsafe { crate::video_profile::formats(video, gpu, profile)? };
        anyhow::ensure!(
            formats.iter().any(crate::video_profile::original_nv12),
            "Original NV12 surfaces require sampleable mutable 2D optimal-tiling output"
        );
        Ok(())
    })();
    lines.push(match result {
        Ok(()) => {
            "Original-surface path: driver-reported eligible; playback validation still required"
                .into()
        }
        Err(error) => format!("Original-surface path unavailable: {error:#}"),
    });
}
