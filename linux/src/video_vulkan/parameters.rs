//! Translate the existing shared H.264 parser output into Vulkan standard structs.
use anyhow::{Result, ensure};
use ash::vk::native as n;
// Vulkan standard C structs contain integers, bitfields and nullable pointers;
// zero initializes reserved fields and absent optional parameter pointers.
use lowlat_decode::h264::{
    pps::Pps,
    sps::{ScalingLists, Sps},
};

/// Address the three-byte Annex-B start_code_prefix_one_3bytes, excluding the
/// optional preceding zero_byte. The shared parser addresses the NAL header.
pub fn slice_offset(bytes: &[u8], header: usize) -> Result<u32> {
    let start = header
        .checked_sub(3)
        .filter(|start| header < bytes.len() && bytes.get(*start..header) == Some(&[0, 0, 1]));
    Ok(u32::try_from(start.ok_or_else(|| {
        anyhow::anyhow!("H.264 slice is missing its Annex-B prefix")
    })?)?)
}

pub fn level(id: u8) -> Result<n::StdVideoH264LevelIdc> {
    [
        10u8, 11, 12, 13, 20, 21, 22, 30, 31, 32, 40, 41, 42, 50, 51, 52, 60, 61, 62,
    ]
    .iter()
    .position(|value| *value == id)
    .map(|index| index as u32)
    .ok_or_else(|| anyhow::anyhow!("Unsupported H.264 level {id}"))
}

pub fn scaling(source: &ScalingLists) -> n::StdVideoH264ScalingLists {
    let mut result = n::StdVideoH264ScalingLists {
        scaling_list_present_mask: 0xff,
        ScalingList4x4: source.list_4x4,
        ..unsafe { std::mem::zeroed() }
    };
    // Vulkan standard parameters use the bitstream's scan order.
    for list in &mut result.ScalingList4x4 {
        let raster = *list;
        for (index, position) in lowlat_decode::h264::sps::ZIGZAG_4X4.iter().enumerate() {
            list[index] = raster[*position];
        }
    }
    for (out, raster) in result.ScalingList8x8.iter_mut().zip(&source.list_8x8) {
        for (index, position) in lowlat_decode::h264::sps::ZIGZAG_8X8.iter().enumerate() {
            out[index] = raster[*position];
        }
    }
    result
}

pub fn sps(
    source: &Sps,
    lists: &n::StdVideoH264ScalingLists,
) -> Result<n::StdVideoH264SequenceParameterSet> {
    ensure!(
        source.frame_mbs_only
            && source.chroma_format_idc == 1
            && source.bit_depth_luma_minus8 == 0
            && source.bit_depth_chroma_minus8 == 0,
        "The Linux preview supports progressive H.264 8-bit 4:2:0"
    );
    let crop = source.crop.unwrap_or_default();
    let mut result = n::StdVideoH264SequenceParameterSet {
        profile_idc: source.profile_idc.into(),
        level_idc: level(source.level_idc)?,
        chroma_format_idc: 1,
        seq_parameter_set_id: source.id,
        log2_max_frame_num_minus4: source.log2_max_frame_num_minus4,
        pic_order_cnt_type: source.pic_order_cnt_type.into(),
        offset_for_non_ref_pic: source.offset_for_non_ref_pic,
        offset_for_top_to_bottom_field: source.offset_for_top_to_bottom_field,
        log2_max_pic_order_cnt_lsb_minus4: source.log2_max_pic_order_cnt_lsb_minus4,
        num_ref_frames_in_pic_order_cnt_cycle: source.num_ref_frames_in_pic_order_cnt_cycle,
        max_num_ref_frames: source.max_num_ref_frames,
        pic_width_in_mbs_minus1: source.pic_width_in_mbs_minus1.into(),
        pic_height_in_map_units_minus1: source.pic_height_in_map_units_minus1.into(),
        frame_crop_left_offset: crop[0],
        frame_crop_right_offset: crop[1],
        frame_crop_top_offset: crop[2],
        frame_crop_bottom_offset: crop[3],
        pOffsetForRefFrame: source.offset_for_ref_frame.as_ptr(),
        pScalingLists: lists,
        ..unsafe { std::mem::zeroed() }
    };
    result
        .flags
        .set_constraint_set0_flag(u32::from(source.constraint_flags & 128 != 0));
    result
        .flags
        .set_constraint_set1_flag(u32::from(source.constraint_flags & 64 != 0));
    result
        .flags
        .set_constraint_set2_flag(u32::from(source.constraint_flags & 32 != 0));
    result
        .flags
        .set_constraint_set3_flag(u32::from(source.constraint_flags & 16 != 0));
    result
        .flags
        .set_constraint_set4_flag(u32::from(source.constraint_flags & 8 != 0));
    result
        .flags
        .set_constraint_set5_flag(u32::from(source.constraint_flags & 4 != 0));
    result
        .flags
        .set_direct_8x8_inference_flag(source.direct_8x8_inference.into());
    result.flags.set_frame_mbs_only_flag(1);
    result
        .flags
        .set_delta_pic_order_always_zero_flag(source.delta_pic_order_always_zero.into());
    result
        .flags
        .set_gaps_in_frame_num_value_allowed_flag(source.gaps_in_frame_num_allowed.into());
    result
        .flags
        .set_qpprime_y_zero_transform_bypass_flag(source.qpprime_y_zero_transform_bypass.into());
    result
        .flags
        .set_frame_cropping_flag(source.crop.is_some().into());
    result
        .flags
        .set_seq_scaling_matrix_present_flag(source.scaling_matrix_present.into());
    Ok(result)
}

pub fn pps(
    source: &Pps,
    lists: &n::StdVideoH264ScalingLists,
) -> n::StdVideoH264PictureParameterSet {
    let mut result = n::StdVideoH264PictureParameterSet {
        seq_parameter_set_id: source.sps_id,
        pic_parameter_set_id: source.id,
        num_ref_idx_l0_default_active_minus1: source.num_ref_idx_l0_default_active_minus1,
        num_ref_idx_l1_default_active_minus1: source.num_ref_idx_l1_default_active_minus1,
        weighted_bipred_idc: source.weighted_bipred_idc.into(),
        pic_init_qp_minus26: source.pic_init_qp_minus26,
        pic_init_qs_minus26: source.pic_init_qs_minus26,
        chroma_qp_index_offset: source.chroma_qp_index_offset,
        second_chroma_qp_index_offset: source.second_chroma_qp_index_offset,
        pScalingLists: lists,
        ..unsafe { std::mem::zeroed() }
    };
    result
        .flags
        .set_transform_8x8_mode_flag(source.transform_8x8_mode.into());
    result
        .flags
        .set_redundant_pic_cnt_present_flag(source.redundant_pic_cnt_present.into());
    result
        .flags
        .set_constrained_intra_pred_flag(source.constrained_intra_pred.into());
    result.flags.set_deblocking_filter_control_present_flag(
        source.deblocking_filter_control_present.into(),
    );
    result
        .flags
        .set_weighted_pred_flag(source.weighted_pred.into());
    result
        .flags
        .set_bottom_field_pic_order_in_frame_present_flag(
            source.bottom_field_pic_order_in_frame_present.into(),
        );
    result
        .flags
        .set_entropy_coding_mode_flag(source.entropy_coding_mode.into());
    result
        .flags
        .set_pic_scaling_matrix_present_flag(source.scaling_matrix_present.into());
    result
}
