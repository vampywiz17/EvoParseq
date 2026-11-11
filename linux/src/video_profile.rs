//! One decode usage policy for capability queries and all session resources.
use ash::vk;

pub fn usage() -> vk::VideoDecodeUsageInfoKHR<'static> {
    // A use-case hint, not a latency guarantee or an extra decode buffer.
    vk::VideoDecodeUsageInfoKHR::default()
        .video_usage_hints(vk::VideoDecodeUsageFlagsKHR::STREAMING)
}

/// Resource layout selected from the profile capabilities, never the vendor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surfaces {
    pub layered: bool,
    pub distinct: bool,
}
impl Surfaces {
    /// Whole-NV12 video views must not inherit SAMPLED: that would require
    /// a YCbCr sampler conversion. Presentation samples separate R8/RG8 views.
    pub fn view_usage(self, output: bool) -> vk::ImageUsageFlags {
        let usage = if output {
            self.output_usage()
        } else {
            self.reference_usage()
        };
        usage & !vk::ImageUsageFlags::SAMPLED
    }
    pub fn output_usage(self) -> vk::ImageUsageFlags {
        let usage = vk::ImageUsageFlags::VIDEO_DECODE_DST_KHR | vk::ImageUsageFlags::SAMPLED;
        if self.distinct {
            usage
        } else {
            usage | vk::ImageUsageFlags::VIDEO_DECODE_DPB_KHR
        }
    }
    pub fn reference_usage(self) -> vk::ImageUsageFlags {
        if self.distinct {
            vk::ImageUsageFlags::VIDEO_DECODE_DPB_KHR
        } else {
            self.output_usage()
        }
    }
}
pub fn original_surfaces(
    flags: vk::VideoCapabilityFlagsKHR,
    dpb: u32,
    references: u32,
    decode: vk::VideoDecodeCapabilityFlagsKHR,
) -> anyhow::Result<Surfaces> {
    anyhow::ensure!(
        decode.intersects(
            vk::VideoDecodeCapabilityFlagsKHR::DPB_AND_OUTPUT_COINCIDE
                | vk::VideoDecodeCapabilityFlagsKHR::DPB_AND_OUTPUT_DISTINCT
        ),
        "No supported Vulkan decode output layout"
    );
    anyhow::ensure!(
        dpb >= 2 && references >= 1,
        "Insufficient Vulkan DPB capacity"
    );
    Ok(Surfaces {
        layered: !flags.contains(vk::VideoCapabilityFlagsKHR::SEPARATE_REFERENCE_IMAGES),
        distinct: !decode.contains(vk::VideoDecodeCapabilityFlagsKHR::DPB_AND_OUTPUT_COINCIDE),
    })
}
pub fn original_nv12(format: &vk::VideoFormatPropertiesKHR<'_>) -> bool {
    nv12(format)
        && format
            .image_create_flags
            .contains(vk::ImageCreateFlags::MUTABLE_FORMAT)
}
fn nv12(format: &vk::VideoFormatPropertiesKHR<'_>) -> bool {
    format.format == vk::Format::G8_B8R8_2PLANE_420_UNORM
        && format.image_type == vk::ImageType::TYPE_2D
        && format.image_tiling == vk::ImageTiling::OPTIMAL
}
pub unsafe fn validate_formats(
    video: &ash::khr::video_queue::Instance,
    gpu: vk::PhysicalDevice,
    profile: &vk::VideoProfileInfoKHR<'_>,
    surfaces: Surfaces,
) -> anyhow::Result<()> {
    let output = unsafe { formats(video, gpu, profile, surfaces.output_usage())? };
    anyhow::ensure!(
        output.iter().any(original_nv12),
        "NV12 output requires sampleable mutable 2D optimal-tiling surfaces"
    );
    if surfaces.distinct {
        let references = unsafe { formats(video, gpu, profile, surfaces.reference_usage())? };
        anyhow::ensure!(
            references.iter().any(nv12),
            "NV12 reference surfaces are unavailable"
        );
    }
    Ok(())
}
pub unsafe fn formats(
    video: &ash::khr::video_queue::Instance,
    gpu: vk::PhysicalDevice,
    profile: &vk::VideoProfileInfoKHR<'_>,
    usage: vk::ImageUsageFlags,
) -> anyhow::Result<Vec<vk::VideoFormatPropertiesKHR<'static>>> {
    let profiles = [*profile];
    let mut list = vk::VideoProfileListInfoKHR::default().profiles(&profiles);
    let info = vk::PhysicalDeviceVideoFormatInfoKHR::default()
        .image_usage(usage)
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
    fn linux_surface_layout_handles_intel_nvidia_and_amd_capabilities() {
        for layered in [false, true] {
            for distinct in [false, true] {
                let caps = if layered {
                    vk::VideoCapabilityFlagsKHR::empty()
                } else {
                    vk::VideoCapabilityFlagsKHR::SEPARATE_REFERENCE_IMAGES
                };
                let decode = if distinct {
                    vk::VideoDecodeCapabilityFlagsKHR::DPB_AND_OUTPUT_DISTINCT
                } else {
                    vk::VideoDecodeCapabilityFlagsKHR::DPB_AND_OUTPUT_COINCIDE
                };
                let surfaces = original_surfaces(caps, 16, 15, decode).unwrap();
                assert_eq!(surfaces, Surfaces { layered, distinct });
                assert!(surfaces.output_usage().contains(
                    vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::VIDEO_DECODE_DST_KHR
                ));
                assert_eq!(
                    surfaces
                        .reference_usage()
                        .contains(vk::ImageUsageFlags::SAMPLED),
                    !distinct
                );
                assert!(
                    surfaces
                        .reference_usage()
                        .contains(vk::ImageUsageFlags::VIDEO_DECODE_DPB_KHR)
                );
                for output in [false, true] {
                    let view = surfaces.view_usage(output);
                    assert!(!view.contains(vk::ImageUsageFlags::SAMPLED));
                    assert!(view.contains(if output {
                        vk::ImageUsageFlags::VIDEO_DECODE_DST_KHR
                    } else {
                        vk::ImageUsageFlags::VIDEO_DECODE_DPB_KHR
                    }));
                }
            }
        }
        let both = vk::VideoDecodeCapabilityFlagsKHR::DPB_AND_OUTPUT_COINCIDE
            | vk::VideoDecodeCapabilityFlagsKHR::DPB_AND_OUTPUT_DISTINCT;
        assert!(
            !original_surfaces(
                vk::VideoCapabilityFlagsKHR::SEPARATE_REFERENCE_IMAGES,
                16,
                15,
                both
            )
            .unwrap()
            .distinct
        );
        assert!(
            original_surfaces(
                vk::VideoCapabilityFlagsKHR::empty(),
                16,
                15,
                vk::VideoDecodeCapabilityFlagsKHR::empty()
            )
            .is_err()
        );
        assert!(original_surfaces(vk::VideoCapabilityFlagsKHR::empty(), 1, 0, both).is_err());
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
