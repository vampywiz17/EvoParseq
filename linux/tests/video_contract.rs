#[path = "../src/video_vulkan/parameters.rs"]
mod parameters;

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
