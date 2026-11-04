#[path = "../src/video_vulkan/output.rs"]
mod output;
#[path = "../src/video_vulkan/parameters.rs"]
mod parameters;

#[test]
fn presentation_releases_output_leases_beyond_the_dpb_capacity() {
    use lowlat_decode::h264::{Read, Stream, dpb::MAX_FRAMES};
    let mut healthy = Stream::new();
    let mut unacknowledged = Stream::new();
    let mut reproduced_exhaustion = false;
    let mut count = 0;
    let mut bytes = include_bytes!("data/h264-40-frames.bin").as_slice();
    while !bytes.is_empty() {
        let size = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
        let unit = &bytes[4..4 + size];
        bytes = &bytes[4 + size..];
        assert_eq!(healthy.read(unit).unwrap(), Read::Picture);
        healthy.finish().unwrap();
        assert!(output::latest(&mut healthy.dpb).is_some());
        assert!(output::latest(&mut healthy.dpb).is_none());
        count += 1;

        // The old consumer popped outputs without acknowledging ownership.
        // Verify this exact clip actually exposes the reported failure.
        if !reproduced_exhaustion {
            match unacknowledged.read(unit) {
                Err(lowlat_decode::ParseError::TooMany) => reproduced_exhaustion = true,
                Ok(Read::Picture) => {
                    unacknowledged.finish().unwrap();
                    while unacknowledged.next_output().is_some() {}
                }
                other => panic!("Unexpected parser result: {other:?}"),
            }
        }
    }
    assert!(count > MAX_FRAMES * 2);
    assert!(reproduced_exhaustion);
}

#[test]
fn annex_b_offsets_exclude_the_optional_zero_byte() {
    let bytes = [0, 0, 0, 1, 0x65, 0xaa, 0, 0, 1, 0x41, 0xbb];
    let units: Vec<_> = lowlat_decode::nal::Units::new(&bytes).collect();
    assert_eq!(
        parameters::slice_offset(&bytes, units[0].offset).unwrap(),
        1
    );
    assert_eq!(
        parameters::slice_offset(&bytes, units[1].offset).unwrap(),
        6
    );
    assert!(parameters::slice_offset(&bytes, 0).is_err());
    assert!(parameters::slice_offset(&bytes, 6).is_err());
    assert!(parameters::slice_offset(&bytes, bytes.len()).is_err());
}

#[test]
fn output_shader_is_valid_without_a_gpu() {
    let source = include_str!("../src/video_vulkan/present.wgsl");
    let module = naga::front::wgsl::parse_str(source).expect("Video shader must parse");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .expect("Video shader must validate");
}

#[test]
fn h264_levels_use_vulkan_enum_values() {
    use ash::vk::native::*;
    assert_eq!(
        parameters::level(41).unwrap(),
        StdVideoH264LevelIdc_STD_VIDEO_H264_LEVEL_IDC_4_1
    );
    assert_eq!(
        parameters::level(52).unwrap(),
        StdVideoH264LevelIdc_STD_VIDEO_H264_LEVEL_IDC_5_2
    );
    assert!(parameters::level(0).is_err());
}

#[test]
fn scaling_lists_convert_raster_to_bitstream_scan_order() {
    let mut source = lowlat_decode::h264::sps::ScalingLists::FLAT;
    for (i, value) in source.list_4x4[0].iter_mut().enumerate() {
        *value = i as u8;
    }
    for (i, value) in source.list_8x8[0].iter_mut().enumerate() {
        *value = i as u8;
    }
    let output = parameters::scaling(&source);
    assert_eq!(output.ScalingList4x4[0][2], 4);
    assert_eq!(output.ScalingList8x8[0][2], 8);
    assert_eq!(output.ScalingList8x8[1], [16; 64]);
}
