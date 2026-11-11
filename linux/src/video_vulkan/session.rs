use super::{device::Context, parameters};
use anyhow::{Context as _, Result, ensure};
use ash::vk::{self, native as n};
use lowlat_decode::h264::{Job, dpb::MAX_FRAMES, pps::Pps, sps::Sps};
use std::sync::{Arc, Mutex, OnceLock};

const WAIT_NS: u64 = 100_000_000;
const MAX_INPUT: u64 = 8 * 1024 * 1024;

pub struct Session {
    resources: Option<Box<Resources>>,
    pub textures: Vec<wgpu::Texture>,
    pub sync: Option<Arc<super::sync::Sync>>,
    pub size: (u32, u32),
    alignment: u64,
    initialized: bool,
    slot_ids: [Option<u32>; MAX_FRAMES],
    slot_count: u32,
    last_sets: Option<(Sps, Pps)>,
    last_hevc: Option<
        Box<(
            lowlat_decode::hevc::sps::Sps,
            lowlat_decode::hevc::pps::Pps,
            u64,
        )>,
    >,
    pub graphics_wait: crate::video_output::Timing,
    pub decode_wait: crate::video_output::Timing,
}

struct Resources {
    context: Arc<Context>,
    sync: Option<Arc<super::sync::Sync>>,
    session: vk::VideoSessionKHR,
    parameters: vk::VideoSessionParametersKHR,
    session_memory: Vec<vk::DeviceMemory>,
    images: Vec<Arc<Image>>,
    outputs: Vec<Arc<Image>>,
    surfaces: crate::video_profile::Surfaces,
    reference_initialized: [bool; MAX_FRAMES],
    jobs: Vec<Submission>,
    active: usize,
    completed: u64,
    pool: vk::CommandPool,
}

struct Submission {
    buffer: vk::Buffer,
    buffer_memory: vk::DeviceMemory,
    command: vk::CommandBuffer,
    fence: vk::Fence,
    query: vk::QueryPool,
    pending: bool,
    started: std::time::Instant,
}

struct Image {
    context: Arc<Context>,
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
}

impl Drop for Image {
    fn drop(&mut self) {
        // Imported textures retain their allocation until the final plane view
        // and every submitted render command have released it.
        unsafe {
            self.context.raw.destroy_image_view(self.view, None);
            self.context.raw.destroy_image(self.image, None);
            self.context.raw.free_memory(self.memory, None);
        }
    }
}
impl Resources {
    fn new(context: Arc<Context>) -> Result<Self> {
        let sync = if context.timeline_sync {
            Some(super::sync::Sync::new(context.clone())?)
        } else {
            None
        };
        Ok(Self {
            context,
            sync,
            session: vk::VideoSessionKHR::null(),
            parameters: vk::VideoSessionParametersKHR::null(),
            session_memory: Vec::new(),
            images: Vec::new(),
            outputs: Vec::new(),
            surfaces: crate::video_profile::Surfaces {
                layered: false,
                distinct: false,
            },
            reference_initialized: [false; MAX_FRAMES],
            jobs: Vec::new(),
            active: 0,
            completed: 0,
            pool: vk::CommandPool::null(),
        })
    }

    fn reap(&mut self, block: bool) -> Result<()> {
        for job in &mut self.jobs {
            if !job.pending {
                continue;
            }
            let done = unsafe {
                if block {
                    self.context
                        .raw
                        .wait_for_fences(&[job.fence], true, WAIT_NS)?;
                    true
                } else {
                    self.context.raw.get_fence_status(job.fence)?
                }
            };
            if !done {
                ensure!(
                    job.started.elapsed() < std::time::Duration::from_nanos(WAIT_NS),
                    "Vulkan decode completion timeout"
                );
                continue;
            }
            if job.query != vk::QueryPool::null() {
                let mut status = vk::QueryResultStatusKHR::NOT_READY;
                unsafe {
                    self.context.raw.get_query_pool_results(
                        job.query,
                        0,
                        std::slice::from_mut(&mut status),
                        vk::QueryResultFlags::WITH_STATUS_KHR,
                    )?;
                }
                ensure!(
                    status.as_raw() > 0,
                    "Vulkan decoder rejected the picture: {status:?}"
                );
            }
            job.pending = false;
            self.completed += 1;
        }
        Ok(())
    }
    fn wait(&mut self) -> Result<()> {
        self.reap(true)
    }
    fn select(&mut self) -> Result<()> {
        self.reap(false)?;
        self.active = self
            .jobs
            .iter()
            .position(|job| !job.pending)
            .context("Vulkan submission ring is busy")?;
        Ok(())
    }
}

impl Drop for Resources {
    fn drop(&mut self) {
        // Only destroyed after all decode and graphics submissions completed.
        unsafe {
            let raw = &self.context.raw;
            for job in &self.jobs {
                raw.destroy_fence(job.fence, None);
                raw.destroy_query_pool(job.query, None);
                raw.destroy_buffer(job.buffer, None);
                raw.free_memory(job.buffer_memory, None);
            }
            raw.destroy_command_pool(self.pool, None);
            self.context
                .video
                .destroy_video_session_parameters(self.parameters, None);
            self.context.video.destroy_video_session(self.session, None);
            for memory in &self.session_memory {
                raw.free_memory(*memory, None);
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(mut resources) = self.resources.take() {
            if resources.wait().is_err() || resources.context.wait_graphics().is_err() {
                // A timed-out driver may still own these objects. Retain their
                // device as well, rather than freeing GPU-visible memory early.
                static RETIRED: OnceLock<Mutex<Vec<Box<Resources>>>> = OnceLock::new();
                RETIRED
                    .get_or_init(Default::default)
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(resources);
                return;
            }
            self.textures.clear();
        }
    }
}

impl Session {
    pub fn output_layer(&self, slot: usize) -> u32 {
        self.resources.as_ref().map_or(0, |resources| {
            if resources.surfaces.layered && !resources.surfaces.distinct {
                slot as u32
            } else {
                0
            }
        })
    }
    pub fn available(&mut self) -> Result<usize> {
        let resources = self.resources.as_mut().context("Video session closed")?;
        resources.reap(false)?;
        Ok(resources.jobs.iter().filter(|job| !job.pending).count())
    }
    pub fn finish(&mut self) -> Result<()> {
        self.resources
            .as_mut()
            .context("Video session closed")?
            .wait()
    }
    pub fn completed(&self) -> u64 {
        self.resources.as_ref().map_or(0, |r| r.completed)
    }
    pub fn new(context: Arc<Context>, sps: &Sps) -> Result<Self> {
        parameters::sps(sps, &parameters::scaling(&sps.scaling))?;
        let resources = Box::new(Resources::new(context)?);
        let mut result = Self {
            sync: resources.sync.clone(),
            resources: Some(resources),
            textures: Vec::new(),
            size: (sps.coded_width(), sps.coded_height()),
            alignment: 1,
            initialized: false,
            slot_ids: [None; MAX_FRAMES],
            slot_count: 0,
            last_sets: None,
            last_hevc: None,
            graphics_wait: Default::default(),
            decode_wait: Default::default(),
        };
        unsafe { result.initialize(Sequence::H264(sps))? };
        Ok(result)
    }

    pub fn new_hevc(
        context: Arc<Context>,
        sps: &lowlat_decode::hevc::sps::Sps,
        header: &super::hevc_parameters::Header,
    ) -> Result<Self> {
        super::hevc_parameters::validate(sps)?;
        let resources = Box::new(Resources::new(context)?);
        let mut result = Self {
            sync: resources.sync.clone(),
            resources: Some(resources),
            textures: Vec::new(),
            size: (sps.width, sps.height),
            alignment: 1,
            initialized: false,
            slot_ids: [None; MAX_FRAMES],
            slot_count: 0,
            last_sets: None,
            last_hevc: None,
            graphics_wait: Default::default(),
            decode_wait: Default::default(),
        };
        unsafe {
            result.initialize(Sequence::Hevc(sps, header))?;
        }
        Ok(result)
    }
    unsafe fn initialize(&mut self, sequence: Sequence<'_>) -> Result<()> {
        let resources = self.resources.as_mut().context("Video session closed")?;
        let context = &resources.context;
        let raw = &context.raw;
        let mut h264 = vk::VideoDecodeH264ProfileInfoKHR::default()
            .picture_layout(vk::VideoDecodeH264PictureLayoutFlagsKHR::PROGRESSIVE);
        let mut h265 = vk::VideoDecodeH265ProfileInfoKHR::default().std_profile_idc(1);
        let mut usage = crate::video_profile::usage();
        let mut profile = vk::VideoProfileInfoKHR::default()
            .chroma_subsampling(vk::VideoChromaSubsamplingFlagsKHR::TYPE_420)
            .luma_bit_depth(vk::VideoComponentBitDepthFlagsKHR::TYPE_8)
            .chroma_bit_depth(vk::VideoComponentBitDepthFlagsKHR::TYPE_8)
            .push_next(&mut usage);
        let (references, level, hevc) = match sequence {
            Sequence::H264(sps) => {
                h264.std_profile_idc = sps.profile_idc.into();
                profile.video_codec_operation = vk::VideoCodecOperationFlagsKHR::DECODE_H264;
                profile = profile.push_next(&mut h264);
                (
                    u32::from(sps.max_num_ref_frames),
                    parameters::level(sps.level_idc)?,
                    false,
                )
            }
            Sequence::Hevc(sps, header) => {
                ensure!(
                    context.hevc,
                    "HEVC Vulkan Video is unavailable on the selected device/queue"
                );
                profile.video_codec_operation = vk::VideoCodecOperationFlagsKHR::DECODE_H265;
                profile = profile.push_next(&mut h265);
                (
                    u32::from(sps.max_dec_pic_buffering_minus1),
                    super::hevc_parameters::level_id(header.ptl.level)?,
                    true,
                )
            }
        };
        // Entry is needed only to load instance-level Vulkan Video dispatch.
        let entry = unsafe { ash::Entry::load()? };
        let video = ash::khr::video_queue::Instance::new(&entry, &context.instance);
        let mut codec_caps = vk::VideoDecodeH264CapabilitiesKHR::default();
        let mut hevc_caps = vk::VideoDecodeH265CapabilitiesKHR::default();
        let mut decode_caps = vk::VideoDecodeCapabilitiesKHR::default();
        let mut caps = vk::VideoCapabilitiesKHR::default().push_next(&mut decode_caps);
        caps = if hevc {
            caps.push_next(&mut hevc_caps)
        } else {
            caps.push_next(&mut codec_caps)
        };
        unsafe {
            (video.fp().get_physical_device_video_capabilities_khr)(
                context.physical,
                &profile,
                &mut caps,
            )
            .result()?
        };
        if hevc {
            let granularity = caps.picture_access_granularity;
            self.size = (
                self.size.0.div_ceil(granularity.width.max(1)) * granularity.width.max(1),
                self.size.1.div_ceil(granularity.height.max(1)) * granularity.height.max(1),
            );
        }
        let max_coded_extent = caps.max_coded_extent;
        let max_dpb_slots = caps.max_dpb_slots;
        let max_active_reference_pictures = caps.max_active_reference_pictures;
        let min_bitstream_buffer_size_alignment = caps.min_bitstream_buffer_size_alignment;
        let std_header_version = caps.std_header_version;
        let flags = caps.flags;
        resources.surfaces = crate::video_profile::original_surfaces(
            flags,
            max_dpb_slots,
            max_active_reference_pictures,
            decode_caps.flags,
        )?;
        ensure!(
            max_coded_extent.width >= self.size.0 && max_coded_extent.height >= self.size.1,
            "Stream exceeds Vulkan decoder dimensions"
        );
        ensure!(
            max_dpb_slots >= references + 1 && max_active_reference_pictures >= references,
            "Insufficient Vulkan DPB capacity"
        );
        ensure!(
            (if hevc {
                hevc_caps.max_level_idc
            } else {
                codec_caps.max_level_idc
            }) >= level,
            "Stream level exceeds the Vulkan decoder limit"
        );
        self.slot_count = max_dpb_slots.min(MAX_FRAMES as u32);
        self.alignment = min_bitstream_buffer_size_alignment.max(1);
        let extent = vk::Extent2D {
            width: self.size.0,
            height: self.size.1,
        };
        let standard = vk::ExtensionProperties {
            extension_name: std_header_version.extension_name,
            spec_version: std_header_version.spec_version,
        };
        resources.session = unsafe {
            context.video.create_video_session(
                &vk::VideoSessionCreateInfoKHR::default()
                    .queue_family_index(context.decode_family)
                    .video_profile(&profile)
                    .picture_format(vk::Format::G8_B8R8_2PLANE_420_UNORM)
                    .reference_picture_format(vk::Format::G8_B8R8_2PLANE_420_UNORM)
                    .max_coded_extent(extent)
                    .max_dpb_slots(self.slot_count)
                    .max_active_reference_pictures(references.max(1))
                    .std_header_version(&standard),
                None,
            )?
        };
        let requirements = unsafe {
            context
                .video
                .get_video_session_memory_requirements(resources.session)?
        };
        let mut bindings = Vec::new();
        for requirement in requirements {
            let memory = unsafe {
                raw.allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(requirement.memory_requirements.size)
                        .memory_type_index(context.memory_type(
                            requirement.memory_requirements.memory_type_bits,
                            vk::MemoryPropertyFlags::empty(),
                        )?),
                    None,
                )?
            };
            resources.session_memory.push(memory);
            bindings.push(
                vk::BindVideoSessionMemoryInfoKHR::default()
                    .memory_bind_index(requirement.memory_bind_index)
                    .memory(memory)
                    .memory_size(requirement.memory_requirements.size),
            );
        }
        unsafe {
            context
                .video
                .bind_video_session_memory(resources.session, &bindings)?
        };
        let profiles = [profile];
        unsafe {
            crate::video_profile::validate_formats(
                &video,
                context.physical,
                &profile,
                resources.surfaces,
            )?
        };
        let graphics_family = unsafe { context.device.as_hal::<wgpu::hal::api::Vulkan>() }
            .context("Vulkan device unavailable")?
            .queue_family_index();
        let families = [context.decode_family, graphics_family];
        // Keep the parser's existing fixed slot bound and output lease. Distinct
        // output is written by the decode command itself, never copied by us.
        for output in [false, true] {
            if output && !resources.surfaces.distinct {
                continue;
            }
            let sampled = output || !resources.surfaces.distinct;
            let layers = if !output && resources.surfaces.layered {
                MAX_FRAMES as u32
            } else {
                1
            };
            let count = if layers > 1 { 1 } else { MAX_FRAMES };
            let usage = if output {
                resources.surfaces.output_usage()
            } else {
                resources.surfaces.reference_usage()
            };
            for _ in 0..count {
                let mut list = vk::VideoProfileListInfoKHR::default().profiles(&profiles);
                let mut info = vk::ImageCreateInfo::default()
                    .push_next(&mut list)
                    .flags(if sampled {
                        vk::ImageCreateFlags::MUTABLE_FORMAT
                    } else {
                        vk::ImageCreateFlags::empty()
                    })
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(vk::Format::G8_B8R8_2PLANE_420_UNORM)
                    .extent(vk::Extent3D {
                        width: extent.width,
                        height: extent.height,
                        depth: 1,
                    })
                    .mip_levels(1)
                    .array_layers(layers)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(usage);
                if graphics_family != context.decode_family {
                    info = info
                        .sharing_mode(vk::SharingMode::CONCURRENT)
                        .queue_family_indices(&families);
                }
                let image = unsafe { raw.create_image(&info, None)? };
                let mut allocation = Image {
                    context: context.clone(),
                    image,
                    memory: vk::DeviceMemory::null(),
                    view: vk::ImageView::null(),
                };
                let requirement = unsafe { raw.get_image_memory_requirements(image) };
                let memory = unsafe {
                    raw.allocate_memory(
                        &vk::MemoryAllocateInfo::default()
                            .allocation_size(requirement.size)
                            .memory_type_index(context.memory_type(
                                requirement.memory_type_bits,
                                vk::MemoryPropertyFlags::DEVICE_LOCAL,
                            )?),
                        None,
                    )?
                };
                allocation.memory = memory;
                unsafe { raw.bind_image_memory(image, memory, 0)? };
                let mut view_usage = vk::ImageViewUsageCreateInfo::default()
                    .usage(resources.surfaces.view_usage(output));
                let view = unsafe {
                    raw.create_image_view(
                        &vk::ImageViewCreateInfo::default()
                            .push_next(&mut view_usage)
                            .image(image)
                            .view_type(if layers > 1 {
                                vk::ImageViewType::TYPE_2D_ARRAY
                            } else {
                                vk::ImageViewType::TYPE_2D
                            })
                            .format(vk::Format::G8_B8R8_2PLANE_420_UNORM)
                            .subresource_range(
                                vk::ImageSubresourceRange::default()
                                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                                    .level_count(1)
                                    .layer_count(layers),
                            ),
                        None,
                    )?
                };
                allocation.view = view;
                let allocation = Arc::new(allocation);
                let images = if output {
                    &mut resources.outputs
                } else {
                    &mut resources.images
                };
                images.extend((0..layers).map(|_| allocation.clone()));
                if !sampled {
                    continue;
                }
                let hal = unsafe { context.device.as_hal::<wgpu::hal::api::Vulkan>() }
                    .context("Vulkan texture interop unavailable")?;
                let size = wgpu::Extent3d {
                    width: extent.width,
                    height: extent.height,
                    depth_or_array_layers: layers,
                };
                let texture = unsafe {
                    hal.texture_from_raw(
                        image,
                        &wgpu::hal::TextureDescriptor {
                            label: Some("Original NV12 decode surface"),
                            size,
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu::TextureFormat::NV12,
                            usage: wgpu::TextureUses::RESOURCE,
                            memory_flags: wgpu::hal::MemoryFlags::empty(),
                            view_formats: Vec::new(),
                        },
                        Some(Box::new(move || drop(allocation))),
                    )
                };
                drop(hal);
                let texture = unsafe {
                    context
                        .device
                        .create_texture_from_hal::<wgpu::hal::api::Vulkan>(
                            texture,
                            &wgpu::TextureDescriptor {
                                label: Some("Original NV12 decode surface"),
                                size,
                                mip_level_count: 1,
                                sample_count: 1,
                                dimension: wgpu::TextureDimension::D2,
                                format: wgpu::TextureFormat::NV12,
                                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                                view_formats: &[],
                            },
                        )
                };
                self.textures.extend((0..layers).map(|_| texture.clone()));
            }
        }
        // Establish wgpu's RESOURCE state before the first native decode.
        // Subsequent native operations restore SHADER_READ_ONLY_OPTIMAL.
        let mut encoder = context.device.create_command_encoder(&Default::default());
        encoder.transition_resources(
            std::iter::empty(),
            self.textures
                .iter()
                .take(
                    if resources.surfaces.layered && !resources.surfaces.distinct {
                        1
                    } else {
                        MAX_FRAMES
                    },
                )
                .map(|texture| wgpu::TextureTransition {
                    texture,
                    selector: None,
                    state: wgpu::TextureUses::RESOURCE,
                }),
        );
        context.queue.submit([encoder.finish()]);
        context.wait_graphics()?;
        resources.pool = unsafe {
            raw.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(context.decode_family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )?
        };
        for _ in 0..if self.sync.is_some() { 2 } else { 1 } {
            resources.jobs.push(Submission {
                buffer: vk::Buffer::null(),
                buffer_memory: vk::DeviceMemory::null(),
                command: vk::CommandBuffer::null(),
                fence: vk::Fence::null(),
                query: vk::QueryPool::null(),
                pending: false,
                started: std::time::Instant::now(),
            });
            resources.active = resources.jobs.len() - 1;
            if context.decode_status {
                let mut query_profile = profile;
                resources.jobs[resources.active].query = unsafe {
                    raw.create_query_pool(
                        &vk::QueryPoolCreateInfo::default()
                            .query_type(vk::QueryType::RESULT_STATUS_ONLY_KHR)
                            .query_count(1)
                            .push_next(&mut query_profile),
                        None,
                    )?
                };
            }
            let mut list = vk::VideoProfileListInfoKHR::default().profiles(&profiles);
            resources.jobs[resources.active].buffer = unsafe {
                raw.create_buffer(
                    &vk::BufferCreateInfo::default()
                        .size(MAX_INPUT)
                        .usage(vk::BufferUsageFlags::VIDEO_DECODE_SRC_KHR)
                        .push_next(&mut list),
                    None,
                )?
            };
            let requirement = unsafe {
                raw.get_buffer_memory_requirements(resources.jobs[resources.active].buffer)
            };
            resources.jobs[resources.active].buffer_memory = unsafe {
                raw.allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(requirement.size)
                        .memory_type_index(context.memory_type(
                            requirement.memory_type_bits,
                            vk::MemoryPropertyFlags::HOST_VISIBLE
                                | vk::MemoryPropertyFlags::HOST_COHERENT,
                        )?),
                    None,
                )?
            };
            unsafe {
                raw.bind_buffer_memory(
                    resources.jobs[resources.active].buffer,
                    resources.jobs[resources.active].buffer_memory,
                    0,
                )?
            };
            resources.jobs[resources.active].command = unsafe {
                raw.allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(resources.pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1),
                )?
            }[0];
            resources.jobs[resources.active].fence =
                unsafe { raw.create_fence(&Default::default(), None)? };
        }
        crate::event_log::record(
            "INFO",
            "vulkan_video_initialized",
            serde_json::json!({
                "codec": if hevc { "HEVC" } else { "H.264" },
                "gpu_timeline_sync": self.sync.is_some(),
                "surface_tracking": self.sync.is_some(),
                "selected_output_lease": true,
                "submission_slots": resources.jobs.len(),
                "layered_dpb": resources.surfaces.layered,
                "distinct_output": resources.surfaces.distinct,
            }),
        );
        Ok(())
    }

    fn update_parameters(&mut self, job: &Job) -> Result<()> {
        if self.last_sets.as_ref() == Some(&(job.sps, job.pps)) {
            return Ok(());
        }
        let resources = self.resources.as_mut().context("Video session closed")?;
        resources.wait()?;
        let sps_lists = parameters::scaling(&job.sps.scaling);
        let pps_lists = parameters::scaling(&job.pps.scaling);
        let sps = [parameters::sps(&job.sps, &sps_lists)?];
        let pps = [parameters::pps(&job.pps, &pps_lists)];
        let mut add = vk::VideoDecodeH264SessionParametersAddInfoKHR::default()
            .std_sp_ss(&sps)
            .std_pp_ss(&pps);
        let mut codec = vk::VideoDecodeH264SessionParametersCreateInfoKHR::default()
            .max_std_sps_count(1)
            .max_std_pps_count(1)
            .parameters_add_info(&add);
        let parameters = unsafe {
            resources.context.video.create_video_session_parameters(
                &vk::VideoSessionParametersCreateInfoKHR::default()
                    .video_session(resources.session)
                    .push_next(&mut codec),
                None,
            )?
        };
        unsafe {
            resources
                .context
                .video
                .destroy_video_session_parameters(resources.parameters, None)
        };
        resources.parameters = parameters;
        self.last_sets = Some((job.sps, job.pps));
        // The structures, arrays and offset tables remain valid through creation.
        let _ = &mut add;
        Ok(())
    }

    pub fn decode(&mut self, job: &Job, bytes: &[u8]) -> Result<()> {
        self.update_parameters(job)?;
        for (slot, id) in self.slot_ids.iter_mut().enumerate() {
            if !job
                .references
                .as_slice()
                .iter()
                .flatten()
                .any(|reference| reference.slot == slot)
            {
                *id = None;
            }
        }
        ensure!(job.current.slot < MAX_FRAMES, "Invalid target surface slot");
        let free = (0..self.slot_count)
            .find(|id| !self.slot_ids.contains(&Some(*id)))
            .context("No free Vulkan DPB slot")?;
        self.slot_ids[job.current.slot] = Some(free);
        let resources = self.resources.as_mut().context("Video session closed")?;
        if self.sync.is_none() {
            resources.wait()?;
            let started = std::time::Instant::now();
            let result = resources.context.wait_graphics();
            self.graphics_wait.record(started.elapsed());
            result?;
        }
        resources.select()?;
        let size = (bytes.len() as u64).div_ceil(self.alignment) * self.alignment;
        ensure!(
            size > 0 && size <= MAX_INPUT,
            "H.264 access unit exceeds the bounded input buffer"
        );
        let first = job
            .slices
            .as_slice()
            .first()
            .copied()
            .flatten()
            .context("Missing H.264 slice")?;
        let offsets: Vec<u32> = job
            .slices
            .as_slice()
            .iter()
            .flatten()
            .map(|slice| parameters::slice_offset(bytes, slice.offset))
            .collect::<Result<_>>()?;
        let picture = |slot: usize| -> Result<vk::VideoPictureResourceInfoKHR<'static>> {
            let view = resources
                .images
                .get(slot)
                .context("Invalid H.264 surface slot")?
                .view;
            Ok(vk::VideoPictureResourceInfoKHR::default()
                .image_view_binding(view)
                .base_array_layer(if resources.surfaces.layered {
                    slot as u32
                } else {
                    0
                })
                .coded_extent(vk::Extent2D {
                    width: self.size.0,
                    height: self.size.1,
                }))
        };
        let references: Vec<_> = job.references.as_slice().iter().flatten().collect();
        let mut standard_refs = Vec::new();
        for reference in &references {
            ensure!(
                reference.parity.is_none(),
                "Interlaced references are unavailable"
            );
            let mut info = n::StdVideoDecodeH264ReferenceInfo {
                FrameNum: u16::try_from(reference.frame_idx)?,
                PicOrderCnt: [reference.top_poc, reference.bottom_poc],
                ..unsafe { std::mem::zeroed() }
            };
            info.flags
                .set_used_for_long_term_reference(reference.long_term.into());
            standard_refs.push(info);
        }
        let mut codec_refs: Vec<_> = standard_refs
            .iter()
            .map(|info| vk::VideoDecodeH264DpbSlotInfoKHR::default().std_reference_info(info))
            .collect();
        let pictures: Vec<_> = references
            .iter()
            .map(|reference| picture(reference.slot))
            .collect::<Result<_>>()?;
        let slots: Vec<_> = references
            .iter()
            .zip(&pictures)
            .zip(&mut codec_refs)
            .map(|((reference, picture), info)| {
                vk::VideoReferenceSlotInfoKHR::default()
                    .slot_index(self.slot_ids[reference.slot].unwrap() as i32)
                    .picture_resource(picture)
                    .push_next(info)
            })
            .collect();
        let target = picture(job.current.slot)?;
        use lowlat_decode::h264::slice::{Marking, Mmco};
        let long_term = match first.header.marking {
            Marking::Idr {
                long_term: true, ..
            } => Some(0),
            Marking::Adaptive(operations) => {
                operations
                    .as_slice()
                    .iter()
                    .flatten()
                    .find_map(|operation| match operation {
                        Mmco::CurrentLong(index) => Some(*index),
                        _ => None,
                    })
            }
            _ => None,
        };
        let mut target_standard = n::StdVideoDecodeH264ReferenceInfo {
            FrameNum: u16::try_from(long_term.unwrap_or(job.current.frame_num))?,
            PicOrderCnt: [job.current.top_poc, job.current.bottom_poc],
            ..unsafe { std::mem::zeroed() }
        };
        target_standard
            .flags
            .set_used_for_long_term_reference(long_term.is_some().into());
        let mut target_codec =
            vk::VideoDecodeH264DpbSlotInfoKHR::default().std_reference_info(&target_standard);
        let target_slot = vk::VideoReferenceSlotInfoKHR::default()
            .slot_index(free as i32)
            .picture_resource(&target)
            .push_next(&mut target_codec);
        let mut picture_info = n::StdVideoDecodeH264PictureInfo {
            seq_parameter_set_id: job.sps.id,
            pic_parameter_set_id: job.pps.id,
            frame_num: u16::try_from(job.current.frame_num)?,
            idr_pic_id: u16::try_from(first.header.idr_pic_id)?,
            PicOrderCnt: [job.current.top_poc, job.current.bottom_poc],
            ..unsafe { std::mem::zeroed() }
        };
        picture_info.flags.set_IdrPicFlag(job.current.idr.into());
        picture_info
            .flags
            .set_is_intra(first.header.slice_type.is_intra().into());
        picture_info
            .flags
            .set_is_reference(job.current.reference.into());
        let mut codec_info = vk::VideoDecodeH264PictureInfoKHR::default()
            .std_picture_info(&picture_info)
            .slice_offsets(&offsets);
        let output = if resources.surfaces.distinct {
            vk::VideoPictureResourceInfoKHR::default()
                .image_view_binding(resources.outputs[job.current.slot].view)
                .coded_extent(target.coded_extent)
        } else {
            target
        };
        let decode = vk::VideoDecodeInfoKHR::default()
            .src_buffer(resources.jobs[resources.active].buffer)
            .src_buffer_offset(0)
            .src_buffer_range(size)
            .dst_picture_resource(output)
            .setup_reference_slot(&target_slot)
            .reference_slots(&slots)
            .push_next(&mut codec_info);
        let used: Vec<_> = references
            .iter()
            .map(|r| r.slot)
            .chain(std::iter::once(job.current.slot))
            .collect();
        self.execute(bytes, size, target, &slots, &decode, &used)?;
        let _ = &mut target_standard;
        Ok(())
    }

    fn update_hevc_parameters(
        &mut self,
        job: &lowlat_decode::hevc::Job,
        headers: &super::hevc_parameters::Headers,
    ) -> Result<()> {
        use super::hevc_parameters as h;
        ensure!(
            job.pps.range == Default::default(),
            "HEVC PPS range extensions are unsupported"
        );
        if self.last_hevc.as_deref() == Some(&(job.sps, job.pps, headers.revision)) {
            return Ok(());
        }
        let (vps, header) = headers.active(&job.sps)?;
        let vp = h::profile(&vps.header.ptl)?;
        let sp = h::profile(&header.ptl)?;
        let vb = h::buffers(&vps.header.ordering);
        let sb = h::buffers(&header.ordering);
        let sl = h::scaling(&job.sps.scaling);
        let pl = h::scaling(&job.pps.scaling);
        let sets = job.sps.st_rps[..job.sps.num_short_term_ref_pic_sets as usize]
            .iter()
            .map(h::rps)
            .collect::<Result<Vec<_>>>()?;
        let long = n::StdVideoH265LongTermRefPicsSps {
            lt_ref_pic_poc_lsb_sps: job.sps.lt_ref_pic_poc_lsb_sps,
            used_by_curr_pic_lt_sps_flag: job
                .sps
                .used_by_curr_pic_lt_sps
                .iter()
                .enumerate()
                .fold(0, |a, (i, v)| a | (u32::from(*v) << i)),
        };
        let vps = [h::video(vps, &vp, &vb)];
        let sps = [h::sequence(&job.sps, header, &sp, &sb, &sl, &sets, &long)?];
        let pps = [h::picture(
            &job.pps,
            &job.sps,
            headers.pps[job.pps.id as usize].context("HEVC PPS metadata unavailable")?,
            &pl,
        )];
        let add = vk::VideoDecodeH265SessionParametersAddInfoKHR::default()
            .std_vp_ss(&vps)
            .std_sp_ss(&sps)
            .std_pp_ss(&pps);
        let mut codec = vk::VideoDecodeH265SessionParametersCreateInfoKHR::default()
            .max_std_vps_count(1)
            .max_std_sps_count(1)
            .max_std_pps_count(1)
            .parameters_add_info(&add);
        let resources = self.resources.as_mut().context("Video session closed")?;
        resources.wait()?;
        let parameters = unsafe {
            resources.context.video.create_video_session_parameters(
                &vk::VideoSessionParametersCreateInfoKHR::default()
                    .video_session(resources.session)
                    .push_next(&mut codec),
                None,
            )?
        };
        unsafe {
            resources
                .context
                .video
                .destroy_video_session_parameters(resources.parameters, None);
        }
        resources.parameters = parameters;
        self.last_hevc = Some(Box::new((job.sps, job.pps, headers.revision)));
        Ok(())
    }
    pub fn decode_hevc(
        &mut self,
        job: &lowlat_decode::hevc::Job,
        headers: &super::hevc_parameters::Headers,
        bytes: &[u8],
    ) -> Result<()> {
        self.update_hevc_parameters(job, headers)?;
        for (slot, id) in self.slot_ids.iter_mut().enumerate() {
            if !job
                .references
                .as_slice()
                .iter()
                .flatten()
                .any(|r| r.slot == slot)
            {
                *id = None;
            }
        }
        ensure!(
            job.current.slot < lowlat_decode::hevc::dpb::MAX_PICTURES,
            "Invalid HEVC target slot"
        );
        let free = (0..self.slot_count)
            .find(|id| !self.slot_ids.contains(&Some(*id)))
            .context("No free HEVC DPB slot")?;
        self.slot_ids[job.current.slot] = Some(free);
        let resources = self.resources.as_mut().context("Video session closed")?;
        if self.sync.is_none() {
            resources.wait()?;
            let started = std::time::Instant::now();
            let result = resources.context.wait_graphics();
            self.graphics_wait.record(started.elapsed());
            result?;
        }
        resources.select()?;
        let size = (bytes.len() as u64).div_ceil(self.alignment) * self.alignment;
        ensure!(
            size > 0 && size <= MAX_INPUT,
            "HEVC access unit exceeds the bounded input buffer"
        );
        let offsets: Vec<_> = job
            .slices
            .as_slice()
            .iter()
            .flatten()
            .map(|slice| parameters::slice_offset(bytes, slice.offset))
            .collect::<Result<_>>()?;
        let picture = |slot: usize| -> Result<vk::VideoPictureResourceInfoKHR<'static>> {
            Ok(vk::VideoPictureResourceInfoKHR::default()
                .image_view_binding(
                    resources
                        .images
                        .get(slot)
                        .context("Invalid HEVC surface slot")?
                        .view,
                )
                .base_array_layer(if resources.surfaces.layered {
                    slot as u32
                } else {
                    0
                })
                .coded_extent(vk::Extent2D {
                    // Preserve the SPS picture extent; self.size includes image padding.
                    width: job.sps.width,
                    height: job.sps.height,
                }))
        };
        let references: Vec<_> = job.references.as_slice().iter().flatten().collect();
        let standard: Vec<_> = references
            .iter()
            .map(|r| {
                let mut v = n::StdVideoDecodeH265ReferenceInfo {
                    PicOrderCntVal: r.poc,
                    ..unsafe { std::mem::zeroed() }
                };
                v.flags.set_used_for_long_term_reference(r.long_term.into());
                v
            })
            .collect();
        let mut codecs: Vec<_> = standard
            .iter()
            .map(|v| vk::VideoDecodeH265DpbSlotInfoKHR::default().std_reference_info(v))
            .collect();
        let pictures: Vec<_> = references
            .iter()
            .map(|r| picture(r.slot))
            .collect::<Result<_>>()?;
        let slots: Vec<_> = references
            .iter()
            .zip(&pictures)
            .zip(&mut codecs)
            .map(|((r, p), c)| {
                vk::VideoReferenceSlotInfoKHR::default()
                    .slot_index(self.slot_ids[r.slot].unwrap() as i32)
                    .picture_resource(p)
                    .push_next(c)
            })
            .collect();
        let target = picture(job.current.slot)?;
        let target_standard = n::StdVideoDecodeH265ReferenceInfo {
            PicOrderCntVal: job.current.poc,
            ..unsafe { std::mem::zeroed() }
        };
        let mut target_codec =
            vk::VideoDecodeH265DpbSlotInfoKHR::default().std_reference_info(&target_standard);
        let target_slot = vk::VideoReferenceSlotInfoKHR::default()
            .slot_index(free as i32)
            .picture_resource(&target)
            .push_next(&mut target_codec);
        let picture_info = super::hevc_parameters::decode_picture(job, &self.slot_ids)?;
        let mut codec = vk::VideoDecodeH265PictureInfoKHR::default()
            .std_picture_info(&picture_info)
            .slice_segment_offsets(&offsets);
        let output = if resources.surfaces.distinct {
            vk::VideoPictureResourceInfoKHR::default()
                .image_view_binding(resources.outputs[job.current.slot].view)
                .coded_extent(target.coded_extent)
        } else {
            target
        };
        let decode = vk::VideoDecodeInfoKHR::default()
            .src_buffer(resources.jobs[resources.active].buffer)
            .src_buffer_range(size)
            .dst_picture_resource(output)
            .setup_reference_slot(&target_slot)
            .reference_slots(&slots)
            .push_next(&mut codec);
        let used: Vec<_> = references
            .iter()
            .map(|r| r.slot)
            .chain(std::iter::once(job.current.slot))
            .collect();
        self.execute(bytes, size, target, &slots, &decode, &used)
    }
    fn execute(
        &mut self,
        bytes: &[u8],
        size: u64,
        target: vk::VideoPictureResourceInfoKHR<'_>,
        slots: &[vk::VideoReferenceSlotInfoKHR<'_>],
        decode: &vk::VideoDecodeInfoKHR<'_>,
        used: &[usize],
    ) -> Result<()> {
        let resources = self.resources.as_mut().context("Video session closed")?;
        let context = resources.context.clone();
        let raw = &context.raw;
        let mut begin_slots = slots.to_vec();
        // Bind the destination as an inactive slot before its first decode.
        begin_slots.push(
            vk::VideoReferenceSlotInfoKHR::default()
                .slot_index(-1)
                .picture_resource(&target),
        );
        unsafe {
            let mapped = raw.map_memory(
                resources.jobs[resources.active].buffer_memory,
                0,
                size,
                vk::MemoryMapFlags::empty(),
            )?;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.cast(), bytes.len());
            std::ptr::write_bytes(
                mapped.cast::<u8>().add(bytes.len()),
                0,
                size as usize - bytes.len(),
            );
            raw.unmap_memory(resources.jobs[resources.active].buffer_memory);
            raw.reset_command_buffer(
                resources.jobs[resources.active].command,
                vk::CommandBufferResetFlags::empty(),
            )?;
            raw.begin_command_buffer(
                resources.jobs[resources.active].command,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
            let range = vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .level_count(1)
                .layer_count(1);
            let mut barriers: Vec<_> = used
                .iter()
                .map(|slot| {
                    vk::ImageMemoryBarrier::default()
                        .image(resources.images[*slot].image)
                        .subresource_range(range.base_array_layer(if resources.surfaces.layered {
                            *slot as u32
                        } else {
                            0
                        }))
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .old_layout(if resources.surfaces.distinct {
                            if resources.reference_initialized[*slot] {
                                vk::ImageLayout::VIDEO_DECODE_DPB_KHR
                            } else {
                                vk::ImageLayout::UNDEFINED
                            }
                        } else {
                            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
                        })
                        .new_layout(vk::ImageLayout::VIDEO_DECODE_DPB_KHR)
                        .src_access_mask(
                            if resources.surfaces.distinct
                                && !resources.reference_initialized[*slot]
                            {
                                vk::AccessFlags::empty()
                            } else {
                                vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE
                            },
                        )
                        .dst_access_mask(
                            vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE,
                        )
                })
                .collect();
            if resources.surfaces.distinct {
                let slot = *used.last().context("Missing output slot")?;
                barriers.push(
                    vk::ImageMemoryBarrier::default()
                        .image(resources.outputs[slot].image)
                        .subresource_range(range)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                        .new_layout(vk::ImageLayout::VIDEO_DECODE_DST_KHR)
                        .src_access_mask(vk::AccessFlags::MEMORY_READ)
                        .dst_access_mask(vk::AccessFlags::MEMORY_WRITE),
                );
            }
            raw.cmd_pipeline_barrier(
                resources.jobs[resources.active].command,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &barriers,
            );
            if resources.jobs[resources.active].query != vk::QueryPool::null() {
                raw.cmd_reset_query_pool(
                    resources.jobs[resources.active].command,
                    resources.jobs[resources.active].query,
                    0,
                    1,
                );
            }
            context.video.cmd_begin_video_coding(
                resources.jobs[resources.active].command,
                &vk::VideoBeginCodingInfoKHR::default()
                    .video_session(resources.session)
                    .video_session_parameters(resources.parameters)
                    .reference_slots(&begin_slots),
            );
            if !self.initialized {
                context.video.cmd_control_video_coding(
                    resources.jobs[resources.active].command,
                    &vk::VideoCodingControlInfoKHR::default()
                        .flags(vk::VideoCodingControlFlagsKHR::RESET),
                );
            }
            if resources.jobs[resources.active].query != vk::QueryPool::null() {
                raw.cmd_begin_query(
                    resources.jobs[resources.active].command,
                    resources.jobs[resources.active].query,
                    0,
                    vk::QueryControlFlags::empty(),
                );
            }
            context
                .video
                .cmd_decode_video(resources.jobs[resources.active].command, &decode);
            if resources.jobs[resources.active].query != vk::QueryPool::null() {
                raw.cmd_end_query(
                    resources.jobs[resources.active].command,
                    resources.jobs[resources.active].query,
                    0,
                );
            }
            context.video.cmd_end_video_coding(
                resources.jobs[resources.active].command,
                &Default::default(),
            );
            let barriers: Vec<_> = barriers
                .iter()
                // Distinct DPB layers stay in decode layout; only the output
                // is sampled by graphics. Coinciding images retain Intel's path.
                .filter(|barrier| {
                    !resources.surfaces.distinct
                        || barrier.new_layout == vk::ImageLayout::VIDEO_DECODE_DST_KHR
                })
                .map(|barrier| vk::ImageMemoryBarrier {
                    old_layout: barrier.new_layout,
                    new_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                    src_access_mask: vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE,
                    dst_access_mask: vk::AccessFlags::MEMORY_READ,
                    ..*barrier
                })
                .collect();
            raw.cmd_pipeline_barrier(
                resources.jobs[resources.active].command,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &barriers,
            );
            raw.end_command_buffer(resources.jobs[resources.active].command)?;
            raw.reset_fences(&[resources.jobs[resources.active].fence])?;
            let commands = [resources.jobs[resources.active].command];
            let waits = self.sync.as_ref().map(|sync| [sync.graphics]);
            let signals = self.sync.as_ref().map(|sync| [sync.decode]);
            let output_used = if resources.surfaces.distinct {
                &used[used.len() - 1..]
            } else {
                used
            };
            let wait_values = self
                .sync
                .as_ref()
                .map(|sync| [sync.graphics_value(output_used)]);
            let signal_values = self.sync.as_ref().map(|sync| [sync.next_decode()]);
            let stages = [vk::PipelineStageFlags::ALL_COMMANDS];
            let mut timeline = vk::TimelineSemaphoreSubmitInfo::default();
            let mut submit = vk::SubmitInfo::default().command_buffers(&commands);
            if let (Some(waits), Some(signals), Some(wv), Some(sv)) =
                (&waits, &signals, &wait_values, &signal_values)
            {
                timeline = timeline
                    .wait_semaphore_values(wv)
                    .signal_semaphore_values(sv);
                submit = submit
                    .wait_semaphores(waits)
                    .wait_dst_stage_mask(&stages)
                    .signal_semaphores(signals)
                    .push_next(&mut timeline);
            }
            raw.queue_submit(
                context.decode_queue,
                &[submit],
                resources.jobs[resources.active].fence,
            )?;
            for &slot in used {
                resources.reference_initialized[slot] = true;
            }
            if let (Some(sync), Some(values)) = (&self.sync, signal_values) {
                sync.submitted(values[0]);
            }
            resources.jobs[resources.active].started = std::time::Instant::now();
            resources.jobs[resources.active].pending = true;
        }
        if self.sync.is_none() {
            let started = std::time::Instant::now();
            let result = resources.wait();
            self.decode_wait.record(started.elapsed());
            result.context("Vulkan decode completion timeout")?;
        }
        self.initialized = true;
        Ok(())
    }
}

enum Sequence<'a> {
    H264(&'a Sps),
    Hevc(
        &'a lowlat_decode::hevc::sps::Sps,
        &'a super::hevc_parameters::Header,
    ),
}
