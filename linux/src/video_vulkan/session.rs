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
    session: vk::VideoSessionKHR,
    parameters: vk::VideoSessionParametersKHR,
    session_memory: Vec<vk::DeviceMemory>,
    images: Vec<Arc<Image>>,
    buffer: vk::Buffer,
    buffer_memory: vk::DeviceMemory,
    pool: vk::CommandPool,
    command: vk::CommandBuffer,
    fence: vk::Fence,
    query: vk::QueryPool,
    pending: bool,
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
    fn new(context: Arc<Context>) -> Self {
        Self {
            context,
            session: vk::VideoSessionKHR::null(),
            parameters: vk::VideoSessionParametersKHR::null(),
            session_memory: Vec::new(),
            images: Vec::new(),
            buffer: vk::Buffer::null(),
            buffer_memory: vk::DeviceMemory::null(),
            pool: vk::CommandPool::null(),
            command: vk::CommandBuffer::null(),
            fence: vk::Fence::null(),
            query: vk::QueryPool::null(),
            pending: false,
        }
    }

    fn wait(&mut self) -> Result<()> {
        if self.pending {
            unsafe {
                self.context
                    .raw
                    .wait_for_fences(&[self.fence], true, WAIT_NS)?
            };
            self.pending = false;
        }
        Ok(())
    }
}

impl Drop for Resources {
    fn drop(&mut self) {
        // Only destroyed after all decode and graphics submissions completed.
        unsafe {
            let raw = &self.context.raw;
            raw.destroy_fence(self.fence, None);
            raw.destroy_query_pool(self.query, None);
            raw.destroy_command_pool(self.pool, None);
            self.context
                .video
                .destroy_video_session_parameters(self.parameters, None);
            self.context.video.destroy_video_session(self.session, None);
            for memory in &self.session_memory {
                raw.free_memory(*memory, None);
            }
            raw.destroy_buffer(self.buffer, None);
            raw.free_memory(self.buffer_memory, None);
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
    pub fn new(context: Arc<Context>, sps: &Sps) -> Result<Self> {
        parameters::sps(sps, &parameters::scaling(&sps.scaling))?;
        let mut result = Self {
            resources: Some(Box::new(Resources::new(context))),
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
        let mut result = Self {
            resources: Some(Box::new(Resources::new(context))),
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
        crate::video_profile::original_surfaces(
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
        if context.decode_status {
            let mut query_profile = profile;
            resources.query = unsafe {
                raw.create_query_pool(
                    &vk::QueryPoolCreateInfo::default()
                        .query_type(vk::QueryType::RESULT_STATUS_ONLY_KHR)
                        .query_count(1)
                        .push_next(&mut query_profile),
                    None,
                )?
            };
        }
        let profiles = [profile];
        let usage = crate::video_profile::surface_usage();
        let formats = unsafe { crate::video_profile::formats(&video, context.physical, &profile)? };
        ensure!(
            formats.iter().any(crate::video_profile::original_nv12),
            "The original NV12 decoder surfaces cannot be sampled on this driver"
        );
        let graphics_family = unsafe { context.device.as_hal::<wgpu::hal::api::Vulkan>() }
            .context("Vulkan device unavailable")?
            .queue_family_index();
        let families = [context.decode_family, graphics_family];
        for _ in 0..MAX_FRAMES {
            let mut list = vk::VideoProfileListInfoKHR::default().profiles(&profiles);
            let mut info = vk::ImageCreateInfo::default()
                .push_next(&mut list)
                .flags(vk::ImageCreateFlags::MUTABLE_FORMAT)
                .image_type(vk::ImageType::TYPE_2D)
                .format(vk::Format::G8_B8R8_2PLANE_420_UNORM)
                .extent(vk::Extent3D {
                    width: extent.width,
                    height: extent.height,
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
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
            let view = unsafe {
                raw.create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(vk::Format::G8_B8R8_2PLANE_420_UNORM)
                        .subresource_range(
                            vk::ImageSubresourceRange::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .level_count(1)
                                .layer_count(1),
                        ),
                    None,
                )?
            };
            allocation.view = view;
            let allocation = Arc::new(allocation);
            resources.images.push(allocation.clone());
            let hal = unsafe { context.device.as_hal::<wgpu::hal::api::Vulkan>() }
                .context("Vulkan texture interop unavailable")?;
            let size = wgpu::Extent3d {
                width: extent.width,
                height: extent.height,
                depth_or_array_layers: 1,
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
            self.textures.push(texture);
        }
        // Establish wgpu's RESOURCE state before the first native decode.
        // Subsequent native operations restore SHADER_READ_ONLY_OPTIMAL.
        let mut encoder = context.device.create_command_encoder(&Default::default());
        encoder.transition_resources(
            std::iter::empty(),
            self.textures.iter().map(|texture| wgpu::TextureTransition {
                texture,
                selector: None,
                state: wgpu::TextureUses::RESOURCE,
            }),
        );
        context.queue.submit([encoder.finish()]);
        context.wait_graphics()?;
        let mut list = vk::VideoProfileListInfoKHR::default().profiles(&profiles);
        resources.buffer = unsafe {
            raw.create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(MAX_INPUT)
                    .usage(vk::BufferUsageFlags::VIDEO_DECODE_SRC_KHR)
                    .push_next(&mut list),
                None,
            )?
        };
        let requirement = unsafe { raw.get_buffer_memory_requirements(resources.buffer) };
        resources.buffer_memory = unsafe {
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
        unsafe { raw.bind_buffer_memory(resources.buffer, resources.buffer_memory, 0)? };
        resources.pool = unsafe {
            raw.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(context.decode_family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )?
        };
        resources.command = unsafe {
            raw.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(resources.pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )?
        }[0];
        resources.fence = unsafe { raw.create_fence(&Default::default(), None)? };
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
        resources.wait()?;
        let context = resources.context.clone();
        let graphics_started = std::time::Instant::now();
        let graphics_result = context.wait_graphics();
        self.graphics_wait.record(graphics_started.elapsed());
        graphics_result?;
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
        let decode = vk::VideoDecodeInfoKHR::default()
            .src_buffer(resources.buffer)
            .src_buffer_offset(0)
            .src_buffer_range(size)
            .dst_picture_resource(target)
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
        resources.wait()?;
        let started = std::time::Instant::now();
        let wait = resources.context.wait_graphics();
        self.graphics_wait.record(started.elapsed());
        wait?;
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
        let decode = vk::VideoDecodeInfoKHR::default()
            .src_buffer(resources.buffer)
            .src_buffer_range(size)
            .dst_picture_resource(target)
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
                resources.buffer_memory,
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
            raw.unmap_memory(resources.buffer_memory);
            raw.reset_command_buffer(resources.command, vk::CommandBufferResetFlags::empty())?;
            raw.begin_command_buffer(
                resources.command,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
            let range = vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .level_count(1)
                .layer_count(1);
            let barriers: Vec<_> = used
                .iter()
                .map(|slot| {
                    vk::ImageMemoryBarrier::default()
                        .image(resources.images[*slot].image)
                        .subresource_range(range)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                        .new_layout(vk::ImageLayout::VIDEO_DECODE_DPB_KHR)
                        .src_access_mask(vk::AccessFlags::MEMORY_READ)
                        .dst_access_mask(
                            vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE,
                        )
                })
                .collect();
            raw.cmd_pipeline_barrier(
                resources.command,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &barriers,
            );
            if resources.query != vk::QueryPool::null() {
                raw.cmd_reset_query_pool(resources.command, resources.query, 0, 1);
            }
            context.video.cmd_begin_video_coding(
                resources.command,
                &vk::VideoBeginCodingInfoKHR::default()
                    .video_session(resources.session)
                    .video_session_parameters(resources.parameters)
                    .reference_slots(&begin_slots),
            );
            if !self.initialized {
                context.video.cmd_control_video_coding(
                    resources.command,
                    &vk::VideoCodingControlInfoKHR::default()
                        .flags(vk::VideoCodingControlFlagsKHR::RESET),
                );
            }
            if resources.query != vk::QueryPool::null() {
                raw.cmd_begin_query(
                    resources.command,
                    resources.query,
                    0,
                    vk::QueryControlFlags::empty(),
                );
            }
            context.video.cmd_decode_video(resources.command, &decode);
            if resources.query != vk::QueryPool::null() {
                raw.cmd_end_query(resources.command, resources.query, 0);
            }
            context
                .video
                .cmd_end_video_coding(resources.command, &Default::default());
            let barriers: Vec<_> = barriers
                .iter()
                .map(|barrier| vk::ImageMemoryBarrier {
                    old_layout: vk::ImageLayout::VIDEO_DECODE_DPB_KHR,
                    new_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                    src_access_mask: vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE,
                    dst_access_mask: vk::AccessFlags::MEMORY_READ,
                    ..*barrier
                })
                .collect();
            raw.cmd_pipeline_barrier(
                resources.command,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &barriers,
            );
            raw.end_command_buffer(resources.command)?;
            raw.reset_fences(&[resources.fence])?;
            let commands = [resources.command];
            raw.queue_submit(
                context.decode_queue,
                &[vk::SubmitInfo::default().command_buffers(&commands)],
                resources.fence,
            )?;
            resources.pending = true;
        }
        let decode_started = std::time::Instant::now();
        let decode_result = resources.wait();
        self.decode_wait.record(decode_started.elapsed());
        decode_result.context("Vulkan decode completion timeout")?;
        if resources.query != vk::QueryPool::null() {
            let mut status = vk::QueryResultStatusKHR::NOT_READY;
            unsafe {
                raw.get_query_pool_results(
                    resources.query,
                    0,
                    std::slice::from_mut(&mut status),
                    vk::QueryResultFlags::WITH_STATUS_KHR,
                )?
            };
            ensure!(
                status.as_raw() > 0,
                "Vulkan decoder rejected the picture: {status:?}"
            );
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
