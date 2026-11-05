//! Vulkan H.265 parameter adaptation. Picture parsing/DPB remain lowlat-decode.
//! Only VPS/PTL/ordering and PPS presence bits missing from its public model are
//! read here, on parameter-set arrival, using the same bounded RBSP bit reader.
use anyhow::{Context, Result, ensure};
use ash::vk::native as n;
use lowlat_decode::{
    bits::BitReader,
    hevc::{
        pps::Pps,
        sps::{ScalingLists, Sps, StRps},
    },
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Ptl {
    pub profile: u8,
    pub level: u8,
    pub tier: bool,
    pub progressive: bool,
    pub interlaced: bool,
    pub packed: bool,
    pub frame_only: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Ordering {
    pub buffering: [u8; 7],
    pub reorder: [u8; 7],
    pub latency: [u32; 7],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Header {
    pub id: u8,
    pub vps: u8,
    pub layers: u8,
    pub nesting: bool,
    pub ordering_present: bool,
    pub scaling_present: bool,
    pub ptl: Ptl,
    pub ordering: Ordering,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Vps {
    pub header: Header,
    pub timing: bool,
    pub units: u32,
    pub scale: u32,
    pub proportional: bool,
    pub ticks: u32,
}
pub struct Headers {
    pub vps: [Option<Vps>; 16],
    pub sps: [Option<Header>; 16],
    pub pps: [Option<bool>; 64],
    pub revision: u64,
}
impl Default for Headers {
    fn default() -> Self {
        Self {
            vps: [None; 16],
            sps: [None; 16],
            pps: [None; 64],
            revision: 0,
        }
    }
}
fn read_error(e: lowlat_decode::ParseError) -> anyhow::Error {
    anyhow::anyhow!("HEVC parameter metadata: {e:?}")
}
macro_rules! read {
    ($value:expr) => {
        $value.map_err(read_error)?
    };
}
fn ptl(r: &mut BitReader<'_>, layers: u8) -> Result<Ptl> {
    ensure!(layers <= 6, "Invalid HEVC temporal layer count");
    let space = read!(r.u8(2));
    let tier = read!(r.flag());
    let profile = read!(r.u8(5));
    let _compat = read!(r.bits(32));
    let progressive = read!(r.flag());
    let interlaced = read!(r.flag());
    let packed = read!(r.flag());
    let frame_only = read!(r.flag());
    read!(r.skip(44));
    let level = read!(r.u8(8));
    ensure!(
        space == 0 && profile == 1 && !interlaced,
        "Linux HEVC requires progressive Main profile"
    );
    let mut present = [(false, false); 7];
    for flags in present.iter_mut().take(layers as usize) {
        *flags = (read!(r.flag()), read!(r.flag()));
    }
    if layers > 0 {
        for _ in layers..8 {
            ensure!(read!(r.bits(2)) == 0, "Invalid HEVC reserved bits");
        }
    }
    for (profile, level) in present.iter().take(layers as usize) {
        if *profile {
            read!(r.skip(88));
        }
        if *level {
            read!(r.skip(8));
        }
    }
    level_id(level)?;
    Ok(Ptl {
        profile,
        level,
        tier,
        progressive,
        interlaced,
        packed,
        frame_only,
    })
}
fn ordering(r: &mut BitReader<'_>, layers: u8) -> Result<(bool, Ordering)> {
    let present = read!(r.flag());
    let mut result = Ordering::default();
    for i in if present { 0 } else { layers as usize }..=layers as usize {
        result.buffering[i] = read!(r.ue_max(15)) as u8;
        result.reorder[i] = read!(r.ue_max(result.buffering[i] as u32)) as u8;
        result.latency[i] = read!(r.ue());
        ensure!(
            result.reorder[i] == 0,
            "Low-latency HEVC requires no frame reordering"
        );
    }
    if !present {
        for i in 0..layers as usize {
            result.buffering[i] = result.buffering[layers as usize];
            result.latency[i] = result.latency[layers as usize];
        }
    }
    Ok((present, result))
}
fn vps(bytes: &[u8]) -> Result<Vps> {
    let mut r = BitReader::new(bytes);
    let id = read!(r.u8(4));
    ensure!(
        read!(r.flag()) && read!(r.flag()),
        "External HEVC base layers are unsupported"
    );
    ensure!(read!(r.bits(6)) == 0, "Multilayer HEVC is unsupported");
    let layers = read!(r.u8(3));
    let nesting = read!(r.flag());
    ensure!(read!(r.bits(16)) == 65535, "Invalid HEVC VPS reserved bits");
    let ptl = ptl(&mut r, layers)?;
    let (ordering_present, ordering) = ordering(&mut r, layers)?;
    ensure!(
        read!(r.bits(6)) == 0 && read!(r.ue_max(1023)) == 0,
        "Multilayer HEVC is unsupported"
    );
    let timing = read!(r.flag());
    let mut result = Vps {
        header: Header {
            id,
            vps: id,
            layers,
            nesting,
            ordering_present,
            ptl,
            ordering,
            ..Default::default()
        },
        timing,
        ..Default::default()
    };
    if timing {
        result.units = read!(r.bits(32));
        result.scale = read!(r.bits(32));
        result.proportional = read!(r.flag());
        if result.proportional {
            result.ticks = read!(r.ue());
        }
        ensure!(
            read!(r.ue_max(1024)) == 0,
            "HEVC VPS HRD parameters are unsupported"
        );
    }
    ensure!(!read!(r.flag()), "HEVC VPS extensions are unsupported");
    Ok(result)
}
fn sps_header(bytes: &[u8]) -> Result<Header> {
    let mut r = BitReader::new(bytes);
    let vps = read!(r.u8(4));
    let layers = read!(r.u8(3));
    let nesting = read!(r.flag());
    let ptl = ptl(&mut r, layers)?;
    let id = read!(r.ue_max(15)) as u8;
    ensure!(read!(r.ue_max(3)) == 1, "Linux HEVC requires 4:2:0 chroma");
    read!(r.ue_max(16888));
    read!(r.ue_max(16888));
    if read!(r.flag()) {
        for _ in 0..4 {
            read!(r.ue());
        }
    }
    ensure!(
        read!(r.ue_max(8)) == 0 && read!(r.ue_max(8)) == 0,
        "Linux HEVC requires 8-bit samples"
    );
    read!(r.ue_max(12));
    let (ordering_present, ordering) = ordering(&mut r, layers)?;
    for _ in 0..6 {
        read!(r.ue());
    }
    let scaling_present = read!(r.flag()) && read!(r.flag());
    Ok(Header {
        id,
        vps,
        layers,
        nesting,
        ptl,
        ordering_present,
        ordering,
        scaling_present,
    })
}
impl Headers {
    pub fn observe(&mut self, bytes: &[u8]) -> Result<()> {
        for nal in lowlat_decode::nal::Units::new(bytes) {
            let Some(&head) = nal.bytes.first() else {
                continue;
            };
            let kind = (head >> 1) & 63;
            if (32..=34).contains(&kind) {
                let second = *nal.bytes.get(1).context("Truncated HEVC NAL header")?;
                ensure!(
                    head & 0x81 == 0 && second >> 3 == 0 && second & 7 != 0,
                    "Invalid or multilayer HEVC parameter NAL"
                );
            }
            match kind {
                32 => {
                    let v = vps(nal.bytes.get(2..).context("Truncated VPS")?)?;
                    let slot = &mut self.vps[v.header.id as usize];
                    if *slot != Some(v) {
                        *slot = Some(v);
                        self.revision = self.revision.wrapping_add(1);
                    }
                }
                33 => {
                    let v = sps_header(nal.bytes.get(2..).context("Truncated SPS")?)?;
                    let slot = &mut self.sps[v.id as usize];
                    if *slot != Some(v) {
                        *slot = Some(v);
                        self.revision = self.revision.wrapping_add(1);
                    }
                }
                34 => {
                    let (id, present) = pps_presence(nal.bytes.get(2..).context("Truncated PPS")?)?;
                    let slot = &mut self.pps[id as usize];
                    if *slot != Some(present) {
                        *slot = Some(present);
                        self.revision = self.revision.wrapping_add(1);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    pub fn active(&self, sps: &Sps) -> Result<(&Vps, &Header)> {
        let vps = self.vps[sps.vps_id as usize]
            .as_ref()
            .context("HEVC VPS unavailable")?;
        let header = self.sps[sps.id as usize]
            .as_ref()
            .context("HEVC SPS metadata unavailable")?;
        ensure!(header.vps == sps.vps_id, "HEVC parameter IDs differ");
        Ok((vps, header))
    }
}
pub fn level_id(id: u8) -> Result<n::StdVideoH265LevelIdc> {
    [30, 60, 63, 90, 93, 120, 123, 150, 153, 156, 180, 183, 186]
        .iter()
        .position(|v| *v == id)
        .map(|i| i as u32)
        .context("Unsupported HEVC level")
}
pub fn profile(source: &Ptl) -> Result<n::StdVideoH265ProfileTierLevel> {
    let mut v = n::StdVideoH265ProfileTierLevel {
        general_profile_idc: 1,
        general_level_idc: level_id(source.level)?,
        ..unsafe { std::mem::zeroed() }
    };
    v.flags.set_general_tier_flag(source.tier.into());
    v.flags
        .set_general_progressive_source_flag(source.progressive.into());
    v.flags
        .set_general_interlaced_source_flag(source.interlaced.into());
    v.flags
        .set_general_non_packed_constraint_flag(source.packed.into());
    v.flags
        .set_general_frame_only_constraint_flag(source.frame_only.into());
    Ok(v)
}
pub fn buffers(source: &Ordering) -> n::StdVideoH265DecPicBufMgr {
    n::StdVideoH265DecPicBufMgr {
        max_latency_increase_plus1: source.latency,
        max_dec_pic_buffering_minus1: source.buffering,
        max_num_reorder_pics: source.reorder,
    }
}
pub fn video(
    source: &Vps,
    ptl: &n::StdVideoH265ProfileTierLevel,
    buffers: &n::StdVideoH265DecPicBufMgr,
) -> n::StdVideoH265VideoParameterSet {
    let mut v = n::StdVideoH265VideoParameterSet {
        vps_video_parameter_set_id: source.header.id,
        vps_max_sub_layers_minus1: source.header.layers,
        vps_num_units_in_tick: source.units,
        vps_time_scale: source.scale,
        vps_num_ticks_poc_diff_one_minus1: source.ticks,
        pProfileTierLevel: ptl,
        pDecPicBufMgr: buffers,
        ..unsafe { std::mem::zeroed() }
    };
    v.flags
        .set_vps_temporal_id_nesting_flag(source.header.nesting.into());
    v.flags
        .set_vps_sub_layer_ordering_info_present_flag(source.header.ordering_present.into());
    v.flags
        .set_vps_timing_info_present_flag(source.timing.into());
    v.flags
        .set_vps_poc_proportional_to_timing_flag(source.proportional.into());
    v
}
pub fn validate(s: &Sps) -> Result<()> {
    ensure!(
        s.profile_idc == 1
            && s.chroma_format_idc == 1
            && !s.separate_colour_plane
            && s.bit_depth_luma_minus8 == 0
            && s.bit_depth_chroma_minus8 == 0,
        "Linux HEVC supports Main 8-bit 4:2:0 only"
    );
    ensure!(
        s.range == Default::default(),
        "HEVC range extensions are unsupported"
    );
    ensure!(
        s.max_num_reorder_pics == 0,
        "Low-latency HEVC requires no frame reordering"
    );
    Ok(())
}
pub fn scaling(s: &ScalingLists) -> n::StdVideoH265ScalingLists {
    let mut v: n::StdVideoH265ScalingLists = unsafe { std::mem::zeroed() };
    for (out, input) in v.ScalingList4x4.iter_mut().zip(s.list_4x4) {
        for (i, pos) in lowlat_decode::hevc::sps::DIAG_4X4.iter().enumerate() {
            out[i] = input[*pos];
        }
    }
    for (out, input) in v.ScalingList8x8.iter_mut().zip(s.list_8x8) {
        for (i, pos) in lowlat_decode::hevc::sps::DIAG_8X8.iter().enumerate() {
            out[i] = input[*pos];
        }
    }
    for (out, input) in v.ScalingList16x16.iter_mut().zip(s.list_16x16) {
        for (i, pos) in lowlat_decode::hevc::sps::DIAG_8X8.iter().enumerate() {
            out[i] = input[*pos];
        }
    }
    for (out, input) in v.ScalingList32x32.iter_mut().zip(s.list_32x32) {
        for (i, pos) in lowlat_decode::hevc::sps::DIAG_8X8.iter().enumerate() {
            out[i] = input[*pos];
        }
    }
    v.ScalingListDCCoef16x16 = s.dc_16x16;
    v.ScalingListDCCoef32x32 = s.dc_32x32;
    v
}
pub fn rps(s: &StRps) -> Result<n::StdVideoH265ShortTermRefPicSet> {
    // Canonical explicit representation of the shared parser's derived set.
    let mut v = n::StdVideoH265ShortTermRefPicSet {
        num_negative_pics: s.num_negative,
        num_positive_pics: s.num_positive,
        ..unsafe { std::mem::zeroed() }
    };
    let mut previous = 0i32;
    for i in 0..s.num_negative as usize {
        v.delta_poc_s0_minus1[i] = u16::try_from(previous - s.delta_s0[i] - 1)?;
        previous = s.delta_s0[i];
        v.used_by_curr_pic_s0_flag |= u16::from(s.used_s0[i]) << i;
    }
    previous = 0;
    for i in 0..s.num_positive as usize {
        v.delta_poc_s1_minus1[i] = u16::try_from(s.delta_s1[i] - previous - 1)?;
        previous = s.delta_s1[i];
        v.used_by_curr_pic_s1_flag |= u16::from(s.used_s1[i]) << i;
    }
    Ok(v)
}

pub fn sequence(
    source: &Sps,
    header: &Header,
    ptl: &n::StdVideoH265ProfileTierLevel,
    buffers: &n::StdVideoH265DecPicBufMgr,
    scaling: &n::StdVideoH265ScalingLists,
    sets: &[n::StdVideoH265ShortTermRefPicSet],
    long: &n::StdVideoH265LongTermRefPicsSps,
) -> Result<n::StdVideoH265SequenceParameterSet> {
    validate(source)?;
    let crop = source.conformance_window.unwrap_or_default();
    let mut v = n::StdVideoH265SequenceParameterSet {
        chroma_format_idc: 1,
        pic_width_in_luma_samples: source.width,
        pic_height_in_luma_samples: source.height,
        sps_video_parameter_set_id: source.vps_id,
        sps_max_sub_layers_minus1: source.max_sub_layers_minus1,
        sps_seq_parameter_set_id: source.id,
        conf_win_left_offset: crop[0],
        conf_win_right_offset: crop[1],
        conf_win_top_offset: crop[2],
        conf_win_bottom_offset: crop[3],
        pProfileTierLevel: ptl,
        pDecPicBufMgr: buffers,
        pScalingLists: scaling,
        pShortTermRefPicSet: if sets.is_empty() {
            std::ptr::null()
        } else {
            sets.as_ptr()
        },
        pLongTermRefPicsSps: long,
        bit_depth_luma_minus8: source.bit_depth_luma_minus8,
        bit_depth_chroma_minus8: source.bit_depth_chroma_minus8,
        log2_max_pic_order_cnt_lsb_minus4: source.log2_max_pic_order_cnt_lsb_minus4,
        log2_min_luma_coding_block_size_minus3: source.log2_min_luma_coding_block_size_minus3,
        log2_diff_max_min_luma_coding_block_size: source.log2_diff_max_min_luma_coding_block_size,
        log2_min_luma_transform_block_size_minus2: source.log2_min_luma_transform_block_size_minus2,
        log2_diff_max_min_luma_transform_block_size: source
            .log2_diff_max_min_luma_transform_block_size,
        max_transform_hierarchy_depth_inter: source.max_transform_hierarchy_depth_inter,
        max_transform_hierarchy_depth_intra: source.max_transform_hierarchy_depth_intra,
        num_short_term_ref_pic_sets: source.num_short_term_ref_pic_sets,
        num_long_term_ref_pics_sps: source.num_long_term_ref_pics_sps,
        pcm_sample_bit_depth_luma_minus1: source.pcm_sample_bit_depth_luma_minus1,
        pcm_sample_bit_depth_chroma_minus1: source.pcm_sample_bit_depth_chroma_minus1,
        log2_min_pcm_luma_coding_block_size_minus3: source
            .log2_min_pcm_luma_coding_block_size_minus3,
        log2_diff_max_min_pcm_luma_coding_block_size: source
            .log2_diff_max_min_pcm_luma_coding_block_size,
        ..unsafe { std::mem::zeroed() }
    };
    v.flags
        .set_sps_temporal_id_nesting_flag((header.nesting).into());
    v.flags
        .set_sps_sub_layer_ordering_info_present_flag((header.ordering_present).into());
    v.flags
        .set_conformance_window_flag((source.conformance_window.is_some()).into());
    v.flags
        .set_scaling_list_enabled_flag((source.scaling_list_enabled).into());
    v.flags
        .set_sps_scaling_list_data_present_flag((header.scaling_present).into());
    v.flags.set_amp_enabled_flag((source.amp_enabled).into());
    v.flags
        .set_sample_adaptive_offset_enabled_flag((source.sample_adaptive_offset_enabled).into());
    v.flags.set_pcm_enabled_flag((source.pcm_enabled).into());
    v.flags
        .set_pcm_loop_filter_disabled_flag((source.pcm_loop_filter_disabled).into());
    v.flags
        .set_long_term_ref_pics_present_flag((source.long_term_ref_pics_present).into());
    v.flags
        .set_sps_temporal_mvp_enabled_flag((source.temporal_mvp_enabled).into());
    v.flags
        .set_strong_intra_smoothing_enabled_flag((source.strong_intra_smoothing_enabled).into());
    Ok(v)
}
pub fn picture(
    source: &Pps,
    sps: &Sps,
    deblocking: bool,
    scaling: &n::StdVideoH265ScalingLists,
) -> n::StdVideoH265PictureParameterSet {
    let mut v = n::StdVideoH265PictureParameterSet {
        pps_pic_parameter_set_id: source.id,
        pps_seq_parameter_set_id: source.sps_id,
        sps_video_parameter_set_id: sps.vps_id,
        pScalingLists: scaling,
        num_extra_slice_header_bits: source.num_extra_slice_header_bits,
        num_ref_idx_l0_default_active_minus1: source.num_ref_idx_l0_default_active_minus1,
        num_ref_idx_l1_default_active_minus1: source.num_ref_idx_l1_default_active_minus1,
        init_qp_minus26: source.init_qp_minus26,
        diff_cu_qp_delta_depth: source.diff_cu_qp_delta_depth,
        log2_parallel_merge_level_minus2: source.log2_parallel_merge_level_minus2,
        num_tile_columns_minus1: source.num_tile_columns_minus1,
        num_tile_rows_minus1: source.num_tile_rows_minus1,
        pps_cb_qp_offset: source.cb_qp_offset,
        pps_cr_qp_offset: source.cr_qp_offset,
        pps_beta_offset_div2: source.beta_offset_div2,
        pps_tc_offset_div2: source.tc_offset_div2,
        ..unsafe { std::mem::zeroed() }
    };
    v.column_width_minus1
        .copy_from_slice(&source.column_width_minus1[..19]);
    v.row_height_minus1
        .copy_from_slice(&source.row_height_minus1[..21]);
    v.flags
        .set_dependent_slice_segments_enabled_flag(source.dependent_slice_segments_enabled.into());
    v.flags
        .set_output_flag_present_flag(source.output_flag_present.into());
    v.flags
        .set_sign_data_hiding_enabled_flag(source.sign_data_hiding_enabled.into());
    v.flags
        .set_cabac_init_present_flag(source.cabac_init_present.into());
    v.flags
        .set_constrained_intra_pred_flag(source.constrained_intra_pred.into());
    v.flags
        .set_transform_skip_enabled_flag(source.transform_skip_enabled.into());
    v.flags
        .set_cu_qp_delta_enabled_flag(source.cu_qp_delta_enabled.into());
    v.flags.set_pps_slice_chroma_qp_offsets_present_flag(
        source.slice_chroma_qp_offsets_present.into(),
    );
    v.flags.set_weighted_pred_flag(source.weighted_pred.into());
    v.flags
        .set_weighted_bipred_flag(source.weighted_bipred.into());
    v.flags
        .set_transquant_bypass_enabled_flag(source.transquant_bypass_enabled.into());
    v.flags.set_tiles_enabled_flag(source.tiles_enabled.into());
    v.flags
        .set_entropy_coding_sync_enabled_flag(source.entropy_coding_sync_enabled.into());
    v.flags
        .set_uniform_spacing_flag(source.uniform_spacing.into());
    v.flags
        .set_loop_filter_across_tiles_enabled_flag(source.loop_filter_across_tiles_enabled.into());
    v.flags.set_pps_loop_filter_across_slices_enabled_flag(
        source.loop_filter_across_slices_enabled.into(),
    );
    v.flags.set_deblocking_filter_override_enabled_flag(
        source.deblocking_filter_override_enabled.into(),
    );
    v.flags
        .set_pps_deblocking_filter_disabled_flag(source.disable_deblocking_filter.into());
    v.flags
        .set_pps_scaling_list_data_present_flag(source.scaling_list_data_present.into());
    v.flags
        .set_lists_modification_present_flag(source.lists_modification_present.into());
    v.flags.set_slice_segment_header_extension_present_flag(
        source.slice_segment_header_extension_present.into(),
    );
    v.flags
        .set_deblocking_filter_control_present_flag(deblocking.into());
    v
}

fn pps_presence(bytes: &[u8]) -> Result<(u8, bool)> {
    let mut r = BitReader::new(bytes);
    let id = read!(r.ue_max(63)) as u8;
    read!(r.ue_max(15));
    read!(r.skip(7));
    read!(r.ue_max(14));
    read!(r.ue_max(14));
    read!(r.se());
    read!(r.skip(2));
    if read!(r.flag()) {
        read!(r.ue_max(3));
    }
    read!(r.se());
    read!(r.se());
    read!(r.skip(4));
    let tiles = read!(r.flag());
    read!(r.flag());
    if tiles {
        let cols = read!(r.ue_max(19));
        let rows = read!(r.ue_max(21));
        if !read!(r.flag()) {
            for _ in 0..cols + rows {
                read!(r.ue_max(65535));
            }
        }
        read!(r.flag());
    }
    read!(r.flag());
    Ok((id, read!(r.flag())))
}

pub fn decode_picture(
    job: &lowlat_decode::hevc::Job,
    slots: &[Option<u32>],
) -> Result<n::StdVideoDecodeH265PictureInfo> {
    let first = job
        .slices
        .as_slice()
        .first()
        .copied()
        .flatten()
        .context("Missing HEVC slice")?;
    let mut v = n::StdVideoDecodeH265PictureInfo {
        sps_video_parameter_set_id: job.sps.vps_id,
        pps_seq_parameter_set_id: job.sps.id,
        pps_pic_parameter_set_id: job.pps.id,
        NumDeltaPocsOfRefRpsIdx: first.header.st_rps.predicted_from_deltas,
        PicOrderCntVal: job.current.poc,
        NumBitsForSTRefPicSetInSlice: u16::try_from(first.header.st_rps_bits)?,
        RefPicSetStCurrBefore: [255; 8],
        RefPicSetStCurrAfter: [255; 8],
        RefPicSetLtCurr: [255; 8],
        ..unsafe { std::mem::zeroed() }
    };
    v.flags.set_IrapPicFlag(job.current.irap.into());
    v.flags.set_IdrPicFlag(first.header.is_idr().into());
    v.flags
        .set_IsReference((!first.header.is_sub_layer_non_reference()).into());
    v.flags
        .set_short_term_ref_pic_set_sps_flag(first.header.st_rps_from_sps.into());
    use lowlat_decode::hevc::dpb::Set;
    for (set, out) in [
        (Set::StCurrBefore, &mut v.RefPicSetStCurrBefore),
        (Set::StCurrAfter, &mut v.RefPicSetStCurrAfter),
        (Set::LtCurr, &mut v.RefPicSetLtCurr),
    ] {
        for (index, r) in job
            .references
            .as_slice()
            .iter()
            .flatten()
            .filter(|r| r.set == set)
            .enumerate()
        {
            ensure!(index < 8, "Too many active HEVC reference pictures");
            out[index] = u8::try_from(
                slots
                    .get(r.slot)
                    .copied()
                    .flatten()
                    .context("HEVC reference slot unavailable")?,
            )?;
        }
    }
    Ok(v)
}
