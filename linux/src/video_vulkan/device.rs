//! One Vulkan device for decode and the existing egui renderer.
use anyhow::{Context as _, Result, bail, ensure};
use ash::vk;
use eframe::egui_wgpu::WgpuSetupExisting;
use std::{ffi::CStr, sync::Arc};

pub struct Context {
    pub adapter_name: String,
    pub frame_latency: Arc<std::sync::atomic::AtomicU32>,
    pub instance: ash::Instance,
    pub raw: ash::Device,
    pub physical: vk::PhysicalDevice,
    pub decode_family: u32,
    pub hevc: bool,
    pub decode_status: bool,
    pub timeline_sync: bool,
    pub decode_queue: vk::Queue,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub video: super::commands::Commands,
    pub properties: vk::PhysicalDeviceMemoryProperties,
    // Keep the instance/adapter alive while native session resources exist.
    _instance: wgpu::Instance,
    _adapter: wgpu::Adapter,
}

pub fn create() -> Result<(WgpuSetupExisting, Arc<Context>)> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..Default::default()
    });
    let requested = requested_adapter(std::env::args().skip(1))?;
    let mut reasons = Vec::new();
    for adapter in instance.enumerate_adapters(wgpu::Backends::VULKAN) {
        if !matches_adapter(&adapter.get_info().name, requested.as_deref()) {
            continue;
        }
        match unsafe { create_on(&instance, &adapter) } {
            Ok(context) => {
                let setup = WgpuSetupExisting {
                    instance: instance.clone(),
                    adapter,
                    device: context.device.clone(),
                    queue: context.queue.clone(),
                };
                return Ok((setup, Arc::new(context)));
            }
            Err(error) => reasons.push(format!("{}: {error:#}", adapter.get_info().name)),
        }
    }
    if reasons.is_empty() && requested.is_some() {
        bail!("No Vulkan adapter matches --video-adapter");
    }
    bail!("H.264 Vulkan Video unavailable. {}", reasons.join("; "))
}

fn requested_adapter(mut args: impl Iterator<Item = String>) -> Result<Option<String>> {
    let mut requested = None;
    while let Some(arg) = args.next() {
        if arg == "--video-adapter" {
            let name = args
                .next()
                .context("--video-adapter requires an adapter name")?;
            ensure!(
                !name.trim().is_empty() && requested.is_none(),
                "Specify --video-adapter once, with a nonempty name"
            );
            requested = Some(name.to_ascii_lowercase());
        }
    }
    Ok(requested)
}
fn matches_adapter(name: &str, requested: Option<&str>) -> bool {
    requested.is_none_or(|needle| name.to_ascii_lowercase().contains(needle))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linux_adapter_selection_never_falls_back_to_a_different_vendor() {
        let selected =
            requested_adapter(["--video-adapter".into(), "NVIDIA".into()].into_iter()).unwrap();
        assert!(matches_adapter("NVIDIA GeForce", selected.as_deref()));
        assert!(!matches_adapter("Intel HD", selected.as_deref()));
        assert!(matches_adapter("AMD Radeon", Some("amd")));
        assert!(requested_adapter(["--video-adapter".into()].into_iter()).is_err());
    }
}

unsafe fn create_on(instance: &wgpu::Instance, adapter: &wgpu::Adapter) -> Result<Context> {
    let hal_instance = unsafe { instance.as_hal::<wgpu::hal::api::Vulkan>() }
        .context("Vulkan instance unavailable")?;
    let hal_adapter = unsafe { adapter.as_hal::<wgpu::hal::api::Vulkan>() }
        .context("Vulkan adapter unavailable")?;
    let raw_instance = hal_instance.shared_instance().raw_instance();
    let physical = hal_adapter.raw_physical_device();
    let features = wgpu::Features::TEXTURE_FORMAT_NV12;
    ensure!(
        adapter.features().contains(features),
        "NV12 sampling unavailable"
    );
    let extensions = unsafe { raw_instance.enumerate_device_extension_properties(physical)? };
    let available = |name: &CStr| {
        extensions
            .iter()
            .any(|e| unsafe { CStr::from_ptr(e.extension_name.as_ptr()) == name })
    };
    let mut required = hal_adapter.required_device_extensions(features);
    for extension in [
        ash::khr::video_queue::NAME,
        ash::khr::video_decode_queue::NAME,
        ash::khr::video_decode_h264::NAME,
    ] {
        ensure!(
            available(extension),
            "Missing {}",
            extension.to_string_lossy()
        );
        if !required.contains(&extension) {
            required.push(extension);
        }
    }
    let count = unsafe { raw_instance.get_physical_device_queue_family_properties2_len(physical) };
    let mut operations = vec![vk::QueueFamilyVideoPropertiesKHR::default(); count];
    let mut statuses = vec![vk::QueueFamilyQueryResultStatusPropertiesKHR::default(); count];
    let mut queries: Vec<_> = operations
        .iter_mut()
        .zip(&mut statuses)
        .map(|(op, status)| {
            vk::QueueFamilyProperties2::default()
                .push_next(op)
                .push_next(status)
        })
        .collect();
    unsafe { raw_instance.get_physical_device_queue_family_properties2(physical, &mut queries) };
    let families: Vec<_> = queries.iter().map(|q| q.queue_family_properties).collect();
    drop(queries);
    let graphics_family = families
        .iter()
        .position(|q| {
            q.queue_flags
                .contains(vk::QueueFlags::GRAPHICS | vk::QueueFlags::COMPUTE)
        })
        .context("Graphics/compute queue unavailable")? as u32;
    let h264_queue = |index: usize| {
        families[index]
            .queue_flags
            .contains(vk::QueueFlags::VIDEO_DECODE_KHR)
            && operations[index]
                .video_codec_operations
                .contains(vk::VideoCodecOperationFlagsKHR::DECODE_H264)
    };
    let decode_family = (0..families.len())
        .find(|&i| {
            h264_queue(i)
                && available(ash::khr::video_decode_h265::NAME)
                && operations[i]
                    .video_codec_operations
                    .contains(vk::VideoCodecOperationFlagsKHR::DECODE_H265)
        })
        .or_else(|| (0..families.len()).find(|&i| h264_queue(i)))
        .context("H.264 decode queue unavailable")? as u32;
    codec_available(raw_instance, physical, false)
        .context("H.264 original-surface path unavailable")?;
    let hevc = available(ash::khr::video_decode_h265::NAME)
        && codec_available(raw_instance, physical, true).is_ok()
        && operations[decode_family as usize]
            .video_codec_operations
            .contains(vk::VideoCodecOperationFlagsKHR::DECODE_H265);
    if hevc && !required.contains(&ash::khr::video_decode_h265::NAME) {
        required.push(ash::khr::video_decode_h265::NAME);
    }
    let priorities = [1.0];
    let mut queues = vec![
        vk::DeviceQueueCreateInfo::default()
            .queue_family_index(graphics_family)
            .queue_priorities(&priorities),
    ];
    if decode_family != graphics_family {
        queues.push(
            vk::DeviceQueueCreateInfo::default()
                .queue_family_index(decode_family)
                .queue_priorities(&priorities),
        );
    }
    let names: Vec<_> = required.iter().map(|name| name.as_ptr()).collect();
    // HAL enables this feature when supported (also used for its own fences).
    // Preserve the synchronous path on devices without timeline semaphores.
    let mut timeline = vk::PhysicalDeviceTimelineSemaphoreFeatures::default();
    let mut queried = vk::PhysicalDeviceFeatures2::default().push_next(&mut timeline);
    unsafe { raw_instance.get_physical_device_features2(physical, &mut queried) };
    let timeline_sync = unsafe { raw_instance.get_physical_device_properties(physical) }
        .api_version
        >= vk::API_VERSION_1_2
        && timeline.timeline_semaphore != 0
        && std::env::var_os("EVOPARSEQ_VULKAN_CPU_SYNC").as_deref()
            != Some(std::ffi::OsStr::new("1"));
    let mut physical_features = hal_adapter.physical_device_features(&required, features);
    let info = physical_features.add_to_device_create(
        vk::DeviceCreateInfo::default()
            .queue_create_infos(&queues)
            .enabled_extension_names(&names),
    );
    let raw = unsafe { raw_instance.create_device(physical, &info, None)? };
    let memory = wgpu::MemoryHints::MemoryUsage;
    let opened = match unsafe {
        hal_adapter.device_from_raw(
            raw.clone(),
            None,
            &required,
            features,
            &memory,
            graphics_family,
            0,
        )
    } {
        Ok(opened) => opened,
        Err(error) => {
            unsafe { raw.destroy_device(None) };
            return Err(error.into());
        }
    };
    let (device, queue) = unsafe {
        adapter.create_device_from_hal(
            opened,
            &wgpu::DeviceDescriptor {
                label: Some("EvoParseq Vulkan video"),
                required_features: features,
                required_limits: adapter.limits(),
                memory_hints: memory,
                ..Default::default()
            },
        )
    }?;
    Ok(Context {
        adapter_name: adapter.get_info().name,
        frame_latency: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        instance: raw_instance.clone(),
        physical,
        decode_family,
        hevc,
        timeline_sync,
        decode_status: statuses[decode_family as usize].query_result_status_support != 0,
        decode_queue: unsafe { raw.get_device_queue(decode_family, 0) },
        properties: unsafe { raw_instance.get_physical_device_memory_properties(physical) },
        video: super::commands::Commands::new(raw_instance, &raw),
        raw,
        device,
        queue,
        _instance: instance.clone(),
        _adapter: adapter.clone(),
    })
}

impl Context {
    pub fn memory_type(&self, bits: u32, flags: vk::MemoryPropertyFlags) -> Result<u32> {
        self.properties
            .memory_types_as_slice()
            .iter()
            .enumerate()
            .find(|(i, memory)| bits & (1 << i) != 0 && memory.property_flags.contains(flags))
            .map(|(i, _)| i as u32)
            .context("No compatible Vulkan memory type")
    }

    pub fn wait_graphics(&self) -> Result<()> {
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_millis(100)),
            })
            .context("Graphics completion timeout")?;
        Ok(())
    }
}

fn codec_available(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    hevc: bool,
) -> Result<()> {
    let entry = unsafe { ash::Entry::load()? };
    let video = ash::khr::video_queue::Instance::new(&entry, instance);
    let mut h264 = vk::VideoDecodeH264ProfileInfoKHR::default()
        .std_profile_idc(vk::native::StdVideoH264ProfileIdc_STD_VIDEO_H264_PROFILE_IDC_HIGH)
        .picture_layout(vk::VideoDecodeH264PictureLayoutFlagsKHR::PROGRESSIVE);
    let mut h265 = vk::VideoDecodeH265ProfileInfoKHR::default().std_profile_idc(1);
    let mut usage = crate::video_profile::usage();
    let profile = vk::VideoProfileInfoKHR::default()
        .video_codec_operation(if hevc {
            vk::VideoCodecOperationFlagsKHR::DECODE_H265
        } else {
            vk::VideoCodecOperationFlagsKHR::DECODE_H264
        })
        .chroma_subsampling(vk::VideoChromaSubsamplingFlagsKHR::TYPE_420)
        .luma_bit_depth(vk::VideoComponentBitDepthFlagsKHR::TYPE_8)
        .chroma_bit_depth(vk::VideoComponentBitDepthFlagsKHR::TYPE_8)
        .push_next(&mut usage);
    let profile = if hevc {
        profile.push_next(&mut h265)
    } else {
        profile.push_next(&mut h264)
    };
    let mut h264_caps = vk::VideoDecodeH264CapabilitiesKHR::default();
    let mut h265_caps = vk::VideoDecodeH265CapabilitiesKHR::default();
    let mut decode = vk::VideoDecodeCapabilitiesKHR::default();
    let caps = vk::VideoCapabilitiesKHR::default().push_next(&mut decode);
    let mut caps = if hevc {
        caps.push_next(&mut h265_caps)
    } else {
        caps.push_next(&mut h264_caps)
    };
    unsafe {
        (video.fp().get_physical_device_video_capabilities_khr)(physical, &profile, &mut caps)
            .result()?;
    }
    let (flags, dpb, references) = (
        caps.flags,
        caps.max_dpb_slots,
        caps.max_active_reference_pictures,
    );
    crate::video_profile::original_surfaces(flags, dpb, references, decode.flags)?;
    let formats = unsafe { crate::video_profile::formats(&video, physical, &profile)? };
    ensure!(
        formats.iter().any(crate::video_profile::original_nv12),
        "Original NV12 surfaces require sampleable mutable 2D optimal-tiling output"
    );
    Ok(())
}
