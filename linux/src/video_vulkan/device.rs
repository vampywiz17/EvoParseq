//! One Vulkan device for decode and the existing egui renderer.
use anyhow::{Context as _, Result, bail, ensure};
use ash::vk;
use eframe::egui_wgpu::WgpuSetupExisting;
use std::{ffi::CStr, sync::Arc};

pub struct Context {
    pub adapter_name: String,
    pub instance: ash::Instance,
    pub raw: ash::Device,
    pub physical: vk::PhysicalDevice,
    pub decode_family: u32,
    pub decode_status: bool,
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
    let mut reasons = Vec::new();
    for adapter in instance.enumerate_adapters(wgpu::Backends::VULKAN) {
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
    bail!("H.264 Vulkan Video unavailable. {}", reasons.join("; "))
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
    let decode_family = families
        .iter()
        .zip(&operations)
        .position(|(q, op)| {
            q.queue_flags.contains(vk::QueueFlags::VIDEO_DECODE_KHR)
                && op
                    .video_codec_operations
                    .contains(vk::VideoCodecOperationFlagsKHR::DECODE_H264)
        })
        .context("H.264 decode queue unavailable")? as u32;
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
        instance: raw_instance.clone(),
        physical,
        decode_family,
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
