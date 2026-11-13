extern crate alloc;

#[path = "../../src-native/src/audio_layout.rs"]
mod audio_layout;
#[path = "../../src-native/src/audio_stream.rs"]
mod audio_stream;
#[path = "../../src-native/src/bud_ingress.rs"]
mod bud_ingress;
#[path = "../src/display_routes.rs"]
mod display_routes;
#[path = "../../src-native/src/display_stream.rs"]
mod display_stream;

#[path = "../../src-native/src/desktop_input.rs"]
mod desktop_input;
#[path = "../../src-native/src/gamepad.rs"]
mod gamepad;
#[path = "../src/video_vulkan/hevc_parameters.rs"]
mod hevc_parameters;
#[path = "../../src-native/src/input_handoff.rs"]
mod input_handoff;
#[path = "../src/video_vulkan/output.rs"]
mod output;
#[path = "../src/video_vulkan/parameters.rs"]
mod parameters;
#[path = "../src/playback.rs"]
mod playback;
#[path = "../../src-native/src/session_controls.rs"]
mod session_controls;
#[path = "../../src-native/src/session_preferences.rs"]
mod session_preferences;
#[path = "../src/video_vulkan/surface_usage.rs"]
mod surface_usage;
#[path = "../../src-native/src/viewport.rs"]
mod viewport;

#[test]
fn native_transport_sink_feeds_linux_playback_and_obeys_cancellation() {
    let playback = playback::Playback::new(eframe::egui::Context::default());
    let sink: std::sync::Arc<dyn video_output::NativeSink> = playback.clone();
    let bytes = [0, 0, 1, 0x65, 0x88];
    let info = video_stream::Inspector::default().receive(&bytes).unwrap();
    assert!(sink.submit(&bytes, info));
    assert_eq!(
        playback
            .decode_pending(|packet| {
                assert_eq!(packet, bytes);
                Ok(())
            })
            .unwrap(),
        1
    );
    sink.stop();
    assert!(playback.stopped());
    assert!(!sink.submit(&bytes, info));
}

#[test]
fn background_backlog_defers_recovery_until_focus_and_restarts_from_cached_parameters() {
    let playback = playback::Playback::new(eframe::egui::Context::default());
    let fixture = include_bytes!("data/h264-40-frames.bin");
    let size = u32::from_le_bytes(fixture[..4].try_into().unwrap()) as usize;
    let first = &fixture[4..4 + size];
    let mut inspector = video_stream::Inspector::default();
    let info = inspector.receive(first).unwrap();
    assert!(info.idr && info.parameters);
    playback.submit_packet(first, info).unwrap();
    let mut parser = lowlat_decode::h264::Stream::new();
    let mut decode = |bytes: &[u8]| -> anyhow::Result<()> {
        assert_eq!(
            parser.read(bytes).unwrap(),
            lowlat_decode::h264::Read::Picture
        );
        parser.finish().unwrap();
        assert!(output::latest(&mut parser.dpb).is_some());
        Ok(())
    };
    assert_eq!(playback.decode_pending(&mut decode).unwrap(), 1);
    for _ in 0..MAX_FRAMES + 1 {
        playback.submit(&[0]).unwrap();
    }
    assert!(playback.status().unwrap().contains("paused"));
    // Time while hidden must not consume the shared recovery deadline.
    assert!(
        playback
            .take_keyframe_request_at(
                std::time::Instant::now() + std::time::Duration::from_secs(3600)
            )
            .unwrap()
            .is_none()
    );
    playback.resume_if_ready(false).unwrap();
    assert_eq!(
        playback
            .decode_pending(|_| panic!("Hidden backlog must be retired"))
            .unwrap(),
        0
    );
    playback.resume_if_ready(true).unwrap();
    assert!(playback.take_keyframe_request().unwrap().is_some());
    assert!(playback.take_keyframe_request().unwrap().is_none());

    let mut key = Vec::new();
    for nal in lowlat_decode::nal::Units::new(first) {
        if nal.bytes[0] & 31 == 5 {
            key.extend_from_slice(&[0, 0, 1]);
            key.extend_from_slice(nal.bytes);
        }
    }
    let key_info = inspector.receive(&key).unwrap();
    assert!(key_info.idr && !key_info.parameters);
    let delta = video_stream::PacketInfo {
        idr: false,
        parameters: false,
        ..key_info
    };
    assert!(!playback.submit_packet(&[0], delta).unwrap());
    assert!(playback.submit_packet(&key, key_info).unwrap());
    assert_eq!(playback.decode_pending(&mut decode).unwrap(), 1);
    assert!(playback.status().is_none());
    // Repeated focus switches must not exhaust decoder-failure attempts.
    for _ in 0..5 {
        for _ in 0..MAX_FRAMES + 1 {
            playback.submit(&[0]).unwrap();
        }
        playback.resume_if_ready(true).unwrap();
        assert!(playback.submit_packet(&key, key_info).unwrap());
        assert_eq!(playback.decode_pending(&mut decode).unwrap(), 1);
        assert!(playback.status().is_none());
    }
}

#[test]
fn resumed_stream_uses_shared_keyframe_deadline_and_send_failure_policy() {
    let playback = playback::Playback::new(eframe::egui::Context::default());
    for _ in 0..MAX_FRAMES + 1 {
        playback.submit(&[0]).unwrap();
    }
    playback.resume_if_ready(true).unwrap();
    let error = playback
        .take_keyframe_request_at(std::time::Instant::now() + std::time::Duration::from_secs(6))
        .unwrap_err();
    assert!(error.to_string().contains("video-recovery-timeout"));
    let playback = playback::Playback::new(eframe::egui::Context::default());
    for _ in 0..MAX_FRAMES + 1 {
        playback.submit(&[0]).unwrap();
    }
    playback.resume_if_ready(true).unwrap();
    let generation = playback.take_keyframe_request().unwrap().unwrap();
    playback.keyframe_request_failed(generation);
    assert!(
        playback
            .take_keyframe_request()
            .unwrap_err()
            .to_string()
            .contains("video-recovery-request-failed")
    );
}

#[test]
fn resumed_packet_keeps_its_parameter_snapshot_when_later_updates_arrive() {
    let playback = playback::Playback::new(eframe::egui::Context::default());
    let fixture = include_bytes!("data/h264-40-frames.bin");
    let size = u32::from_le_bytes(fixture[..4].try_into().unwrap()) as usize;
    let first = &fixture[4..4 + size];
    let mut inspector = video_stream::Inspector::default();
    let info = inspector.receive(first).unwrap();
    let mut cache = video_stream::ParameterCache::default();
    cache.observe(first, info.codec);
    let expected = cache.bytes().unwrap();
    playback.submit_packet(first, info).unwrap();
    playback.decode_pending(|_| Ok(())).unwrap();
    for _ in 0..MAX_FRAMES + 1 {
        playback.submit(&[0]).unwrap();
    }
    playback.resume_if_ready(true).unwrap();
    let mut key = Vec::new();
    let mut later = first.to_vec();
    for nal in lowlat_decode::nal::Units::new(first) {
        if nal.bytes[0] & 31 == 5 {
            key.extend_from_slice(&[0, 0, 1]);
            key.extend_from_slice(nal.bytes);
        }
        if nal.bytes[0] & 31 == 7 {
            later[nal.offset + 3] ^= 1;
        }
    }
    playback
        .submit_packet(&key, inspector.receive(&key).unwrap())
        .unwrap();
    playback
        .submit_packet(&later, inspector.receive(&later).unwrap())
        .unwrap();
    let mut consumed = 0;
    assert_eq!(
        playback
            .decode_pending(|bytes| {
                if consumed == 0 {
                    assert_eq!(&bytes[..expected.len()], expected.as_slice());
                    assert_eq!(&bytes[expected.len()..], key.as_slice());
                } else {
                    assert_eq!(bytes, later.as_slice());
                }
                consumed += 1;
                Ok(())
            })
            .unwrap(),
        2
    );
}

#[test]
fn decode_handoff_handles_60_fps_on_a_59_94_hz_display_without_losing_references() {
    let healthy = playback::Playback::new(eframe::egui::Context::default());
    let mut legacy = std::collections::VecDeque::new();
    let mut next_input = 0u64;
    let mut sent = 0u32;
    let mut decoded = 0u32;
    let mut legacy_failed = false;
    // One frame every 999 ticks vs one repaint every 1000 ticks: exact 60/59.94.
    // Exercise more than five minutes without depending on wall-clock timing.
    for repaint in 0..20_000u64 {
        while next_input <= repaint * 1000 {
            let bytes = sent.to_le_bytes();
            healthy.submit(&bytes).unwrap();
            if !legacy_failed {
                if legacy.len() == 8 {
                    legacy_failed = true;
                } else {
                    legacy.push_back(sent);
                }
            }
            sent += 1;
            next_input += 999;
        }
        // Reproduce the old single-packet-per-repaint consumer.
        legacy.pop_front();
        healthy
            .decode_pending(|bytes| {
                assert_eq!(u32::from_le_bytes(bytes.try_into().unwrap()), decoded);
                decoded += 1;
                Ok(())
            })
            .unwrap();
    }
    assert_eq!(decoded, sent);
    healthy.stop();
    assert!(healthy.stopped());
    assert!(legacy_failed);
}

#[test]
fn decode_batch_preserves_bursts_and_is_bounded_despite_new_arrivals() {
    let playback = playback::Playback::new(eframe::egui::Context::default());
    let limit = MAX_FRAMES as u8;
    for id in 0..limit {
        playback.submit(&[id]).unwrap();
    }
    let mut ids = Vec::new();
    assert_eq!(
        playback
            .decode_pending(|bytes| {
                ids.push(bytes[0]);
                // The producer can use the queue while the native callback runs.
                // These arrivals must not turn this UI update into an unbounded loop.
                playback.submit(&[bytes[0] + limit])?;
                Ok(())
            })
            .unwrap(),
        MAX_FRAMES
    );
    assert_eq!(ids, (0..limit).collect::<Vec<_>>());
    assert_eq!(
        playback
            .decode_pending(|bytes| {
                ids.push(bytes[0]);
                Ok(())
            })
            .unwrap(),
        MAX_FRAMES
    );
    assert_eq!(ids, (0..limit * 2).collect::<Vec<_>>());
    assert_eq!(
        playback
            .decode_pending(|_| panic!("Queue must be empty"))
            .unwrap(),
        0
    );
    playback.submit(&[16]).unwrap();
    playback.stop();
    assert_eq!(
        playback
            .decode_pending(|_| panic!("Stopped playback must not decode"))
            .unwrap(),
        0
    );
}

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
#[path = "../../src-native/src/control.rs"]
mod control;
#[path = "../../src-native/src/event_log.rs"]
mod event_log;
#[path = "../../src-native/src/video_output.rs"]
mod video_output;
#[path = "../../src-native/src/video_stream.rs"]
mod video_stream;

use video_output::MAX_FRAMES;

#[test]
fn observed_nine_packet_burst_is_preserved_without_waiting_for_a_repaint() {
    let playback = playback::Playback::new(eframe::egui::Context::default());
    // Live report: 8 queued packets / 69 KiB arrived within 7.6 ms, with a
    // ninth packet rejected. No elapsed-time simulation proves GPU throughput;
    // this reproduces the complete burst before the UI next consumes input.
    for _ in 0..100 {
        for id in 0..9u8 {
            playback.submit(&vec![id; 8 * 1024]).unwrap();
        }
        let mut expected = 0u8;
        assert_eq!(
            playback
                .decode_pending(|bytes| {
                    assert_eq!(bytes, vec![expected; 8 * 1024]);
                    expected += 1;
                    Ok(())
                })
                .unwrap(),
            9
        );
        assert_eq!(expected, 9);
    }
}

#[test]
fn shared_byte_limit_still_bounds_memory_and_consumption_releases_it() {
    let playback = playback::Playback::new(eframe::egui::Context::default());
    let count = video_output::MAX_BYTES / control::MAX_CHANNEL_MESSAGE;
    for _ in 0..2 {
        for _ in 0..count {
            playback
                .submit(&vec![0; control::MAX_CHANNEL_MESSAGE])
                .unwrap();
        }
        assert_eq!(playback.decode_pending(|_| Ok(())).unwrap(), count);
    }
    assert!(
        playback
            .submit(&vec![0; control::MAX_CHANNEL_MESSAGE + 1])
            .is_err()
    );
}

#[test]
fn bud_encoder_rebuild_retires_old_references_and_resumes_with_parameters() {
    let playback = playback::Playback::new(eframe::egui::Context::default());
    let fixture = include_bytes!("data/h264-40-frames.bin");
    let size = u32::from_le_bytes(fixture[..4].try_into().unwrap()) as usize;
    let first = &fixture[4..4 + size];
    let info = video_stream::Inspector::default().receive(first).unwrap();
    assert!(info.idr && info.parameters);
    assert!(playback.submit_packet(first, info).unwrap());
    playback.encoder_rebuilt();
    // The shared Windows queue retires the codec as well as references.
    // A codec-specific refresh cannot be sent before new parameters arrive.
    assert!(playback.take_keyframe_request().unwrap().is_none());
    let mut count = 0;
    playback
        .decode_pending(|_| {
            count += 1;
            Ok(())
        })
        .unwrap();
    assert_eq!(count, 0, "old generation must not reach the decoder");
    assert!(playback.submit_packet(first, info).unwrap());
    playback
        .decode_pending(|bytes| {
            assert!(bytes.ends_with(first));
            count += 1;
            Ok(())
        })
        .unwrap();
    assert_eq!(count, 1);
    assert!(playback.status().is_none());
}

#[test]
fn hevc_main_metadata_and_reference_slots_match_shared_parser() {
    for fixture in [
        include_bytes!("data/hevc-40-frames.bin").as_slice(),
        include_bytes!("data/hevc-nvenc-ll-8-frames.bin").as_slice(),
    ] {
        let mut stream = lowlat_decode::hevc::Stream::new();
        let mut headers = hevc_parameters::Headers::default();
        let mut pos = 0;
        let mut pictures = 0;
        while pos < fixture.len() {
            let size = u32::from_le_bytes(fixture[pos..pos + 4].try_into().unwrap()) as usize;
            pos += 4;
            let bytes = &fixture[pos..pos + size];
            pos += size;
            headers.observe(bytes).unwrap();
            assert_eq!(
                stream.read(bytes).unwrap(),
                lowlat_decode::hevc::Read::Picture
            );
            let job = stream.job().unwrap();
            hevc_parameters::validate(&job.sps).unwrap();
            let (v, h) = headers.active(&job.sps).unwrap();
            let vp = hevc_parameters::profile(&v.header.ptl).unwrap();
            let sp = hevc_parameters::profile(&h.ptl).unwrap();
            let vb = hevc_parameters::buffers(&v.header.ordering);
            let sb = hevc_parameters::buffers(&h.ordering);
            let lists = hevc_parameters::scaling(&job.sps.scaling);
            let sets = job.sps.st_rps[..job.sps.num_short_term_ref_pic_sets as usize]
                .iter()
                .map(hevc_parameters::rps)
                .collect::<anyhow::Result<Vec<_>>>()
                .unwrap();
            let long = ash::vk::native::StdVideoH265LongTermRefPicsSps {
                used_by_curr_pic_lt_sps_flag: 0,
                lt_ref_pic_poc_lsb_sps: job.sps.lt_ref_pic_poc_lsb_sps,
            };
            let sps =
                hevc_parameters::sequence(&job.sps, h, &sp, &sb, &lists, &sets, &long).unwrap();
            let _vps = hevc_parameters::video(v, &vp, &vb);
            let pps = hevc_parameters::picture(
                &job.pps,
                &job.sps,
                headers.pps[job.pps.id as usize].unwrap(),
                &lists,
            );
            assert_eq!(sps.bit_depth_luma_minus8, 0);
            assert_eq!(pps.pps_seq_parameter_set_id, sps.sps_seq_parameter_set_id);
            let slots: Vec<_> = (0..18).map(|i| Some((17 - i) as u32)).collect();
            let picture = hevc_parameters::decode_picture(job, &slots).unwrap();
            assert_eq!(picture.PicOrderCntVal, job.current.poc);
            use lowlat_decode::hevc::dpb::Set;
            for (set, actual) in [
                (Set::StCurrBefore, picture.RefPicSetStCurrBefore),
                (Set::StCurrAfter, picture.RefPicSetStCurrAfter),
                (Set::LtCurr, picture.RefPicSetLtCurr),
            ] {
                let expected: Vec<_> = job
                    .references
                    .as_slice()
                    .iter()
                    .flatten()
                    .filter(|r| r.set == set)
                    .map(|r| slots[r.slot].unwrap() as u8)
                    .collect();
                assert_eq!(&actual[..expected.len()], expected.as_slice());
                assert!(actual[expected.len()..].iter().all(|v| *v == 255));
            }
            for slice in job.slices.as_slice().iter().flatten() {
                let offset = parameters::slice_offset(bytes, slice.offset).unwrap() as usize;
                assert_eq!(&bytes[offset..offset + 3], &[0, 0, 1]);
            }
            stream.finish().unwrap();
            while let Some(out) = stream.next_output() {
                stream.dpb.taken(out.slot);
            }
            pictures += 1;
        }
        assert!(pictures >= 8);
    }
}
#[test]
fn hevc_rejects_main10_reordering_and_truncated_parameter_sets() {
    for fixture in [
        include_bytes!("data/hevc-main10-first.bin").as_slice(),
        include_bytes!("data/hevc-reordered-first.bin").as_slice(),
    ] {
        let size = u32::from_le_bytes(fixture[..4].try_into().unwrap()) as usize;
        let bytes = &fixture[4..4 + size];
        assert!(hevc_parameters::Headers::default().observe(bytes).is_err());
    }
    for bytes in [
        &[0, 0, 1, 64, 1][..],
        &[0, 0, 1, 66, 1, 0][..],
        &[0, 0, 1, 68, 1][..],
    ] {
        assert!(hevc_parameters::Headers::default().observe(bytes).is_err());
    }
}
#[test]
fn hevc_packets_keep_their_codec_through_the_existing_queue() {
    let playback = playback::Playback::new(eframe::egui::Context::default());
    let fixture = include_bytes!("data/hevc-40-frames.bin");
    let size = u32::from_le_bytes(fixture[..4].try_into().unwrap()) as usize;
    let bytes = &fixture[4..4 + size];
    let mut inspector = video_stream::Inspector::default();
    inspector.enable_hevc(true);
    let info = inspector.receive(bytes).unwrap();
    assert_eq!(info.codec, video_stream::Codec::Hevc);
    assert!(info.idr && info.parameters);
    playback.submit_packet(bytes, info).unwrap();
    assert_eq!(
        playback
            .decode_pending_codec(|packet, codec| {
                assert_eq!(packet, bytes);
                assert_eq!(codec, video_stream::Codec::Hevc);
                Ok(())
            })
            .unwrap(),
        1
    );
}

#[path = "../vendor/wgpu-hal/src/vulkan/semaphore_list.rs"]
mod semaphore_submit_contract;

#[test]
fn linux_gpu_backpressure_keeps_compressed_references_in_order() {
    let playback = playback::Playback::new(eframe::egui::Context::default());
    let fixture = include_bytes!("data/h264-40-frames.bin");
    let mut remaining = fixture.as_slice();
    let mut packets = Vec::new();
    let mut inspector = video_stream::Inspector::default();
    for _ in 0..3 {
        let size = u32::from_le_bytes(remaining[..4].try_into().unwrap()) as usize;
        let bytes = &remaining[4..4 + size];
        playback
            .submit_packet(bytes, inspector.receive(bytes).unwrap())
            .unwrap();
        packets.push(bytes);
        remaining = &remaining[4 + size..];
    }
    // All submission slots busy: do not pop, drop, or restart the decoder.
    assert_eq!(
        playback
            .decode_pending_limited(0, |_, _, _| panic!("GPU busy"))
            .unwrap(),
        0
    );
    let mut parser = lowlat_decode::h264::Stream::new();
    let mut index = 0;
    for budget in [1, 0, 2] {
        let count = playback
            .decode_pending_limited(budget, |bytes, _, _| {
                assert_eq!(bytes, packets[index]);
                assert_eq!(
                    parser.read(bytes).unwrap(),
                    lowlat_decode::h264::Read::Picture
                );
                parser.finish().unwrap();
                assert!(output::latest(&mut parser.dpb).is_some());
                index += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(count, budget);
    }
    assert_eq!(index, 3);
    // Readiness may be refreshed after each decode. A fast GPU can recycle
    // the same bounded slot within one UI drain; capacity is not an FPS cap.
    let retry = playback::Playback::new(eframe::egui::Context::default());
    let mut inspector = video_stream::Inspector::default();
    for bytes in &packets {
        retry
            .submit_packet(bytes, inspector.receive(bytes).unwrap())
            .unwrap();
    }
    let free = std::cell::Cell::new(1);
    let mut parser = lowlat_decode::h264::Stream::new();
    assert_eq!(
        retry
            .decode_pending_ready(
                || free.get() > 0,
                |bytes, _, _| {
                    assert_eq!(
                        parser.read(bytes).unwrap(),
                        lowlat_decode::h264::Read::Picture
                    );
                    parser.finish().unwrap();
                    assert!(output::latest(&mut parser.dpb).is_some());
                    free.set(1);
                    Ok(())
                }
            )
            .unwrap(),
        3
    );
    assert_eq!(
        playback
            .decode_pending_limited(2, |_, _, _| panic!("Already consumed"))
            .unwrap(),
        0
    );
}

#[test]
fn linux_gpu_wakeup_is_one_shot_and_only_polled_when_deferred() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    let playback = playback::Playback::new(eframe::egui::Context::default());
    let ready = Arc::new(AtomicBool::new(false));
    let polls = Arc::new(AtomicUsize::new(0));
    let (r, p) = (ready.clone(), polls.clone());
    playback.defer_gpu(move || {
        p.fetch_add(1, Ordering::Relaxed);
        r.load(Ordering::Relaxed)
    });
    playback.poll_gpu();
    assert_eq!(polls.load(Ordering::Relaxed), 1);
    ready.store(true, Ordering::Relaxed);
    playback.poll_gpu();
    playback.poll_gpu();
    assert_eq!(polls.load(Ordering::Relaxed), 2);
}

/// Exercises the actual patched HAL on a headless Vulkan adapter (including
/// software Vulkan). No video decoding, image readback or production profile.
#[test]
fn linux_vulkan_native_and_wgpu_timeline_bridge() -> anyhow::Result<()> {
    use ash::vk;
    if unsafe { ash::Entry::load() }.is_err() {
        eprintln!("Vulkan sync fixture not exercised: Vulkan loader unavailable");
        return Ok(());
    }
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..Default::default()
    });
    let Some(adapter) = instance
        .enumerate_adapters(wgpu::Backends::VULKAN)
        .into_iter()
        .next()
    else {
        eprintln!("Vulkan sync fixture not exercised: no Vulkan adapter");
        return Ok(());
    };
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    let (device, queue) =
        runtime.block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?;
    let hal = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }.unwrap();
    let mut support = vk::PhysicalDeviceTimelineSemaphoreFeatures::default();
    let mut features = vk::PhysicalDeviceFeatures2::default().push_next(&mut support);
    unsafe {
        hal.shared_instance()
            .raw_instance()
            .get_physical_device_features2(hal.raw_physical_device(), &mut features);
    }
    if support.timeline_semaphore == 0
        || unsafe {
            hal.shared_instance()
                .raw_instance()
                .get_physical_device_properties(hal.raw_physical_device())
        }
        .api_version
            < vk::API_VERSION_1_2
    {
        eprintln!("Vulkan sync fixture not exercised: timeline semaphores unavailable");
        return Ok(());
    }
    let raw = hal.raw_device().clone();
    let raw_queue = hal.raw_queue();
    drop(hal);
    let mut kind =
        vk::SemaphoreTypeCreateInfo::default().semaphore_type(vk::SemaphoreType::TIMELINE);
    let info = vk::SemaphoreCreateInfo::default().push_next(&mut kind);
    let decoded = unsafe { raw.create_semaphore(&info, None)? };
    let graphics = unsafe { raw.create_semaphore(&info, None)? };
    let mut rendered = 0;
    let mut usage = surface_usage::Usage::default();
    for picture in 1..=8u64 {
        let target = picture as usize % 3;
        let reference = (target + 1) % 3;
        let wait_graphics = usage.decode_wait(&[reference, target]);
        let waits = [graphics];
        let signals = [decoded];
        let wait_values = [wait_graphics];
        let signal_values = [picture];
        let stages = [vk::PipelineStageFlags::ALL_COMMANDS];
        let mut timeline = vk::TimelineSemaphoreSubmitInfo::default()
            .wait_semaphore_values(&wait_values)
            .signal_semaphore_values(&signal_values);
        let native = vk::SubmitInfo::default()
            .wait_semaphores(&waits)
            .wait_dst_stage_mask(&stages)
            .signal_semaphores(&signals)
            .push_next(&mut timeline);
        unsafe {
            raw.queue_submit(raw_queue, &[native], vk::Fence::null())?;
        }
        usage.decoded(picture);
        usage.select(target);
        // Some pictures have no render; some are redrawn twice. Repeated
        // waits on an already-signalled timeline value must remain valid.
        for _ in 0..if picture % 3 == 0 { 0 } else { 2 } {
            usage.prepare(target);
            usage.prepare(target); // Clipped/repeated callback, still one signal.
            let (ready, value) = usage.stage().unwrap();
            assert!(usage.stage().is_none());
            rendered = value;
            let command = device.create_command_encoder(&Default::default()).finish();
            {
                let hal = unsafe { queue.as_hal::<wgpu::hal::api::Vulkan>() }.unwrap();
                hal.add_wait_semaphore(decoded, Some(ready), vk::PipelineStageFlags::ALL_COMMANDS);
                hal.add_signal_semaphore(graphics, Some(rendered));
            }
            queue.submit([command]);
            usage.finish();
        }
    }
    device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(2)),
    })?;
    assert_eq!(unsafe { raw.get_semaphore_counter_value(decoded)? }, 8);
    assert_eq!(
        unsafe { raw.get_semaphore_counter_value(graphics)? },
        rendered
    );
    unsafe {
        raw.destroy_semaphore(decoded, None);
        raw.destroy_semaphore(graphics, None);
    }
    eprintln!(
        "Vulkan sync fixture exercised on {}",
        adapter.get_info().name
    );
    Ok(())
}

#[test]
fn linux_selected_h264_surface_stays_leased_without_exhausting_the_dpb() {
    let mut stream = lowlat_decode::h264::Stream::new();
    let mut selection = output::Selection::default();
    let mut selected = None;
    let mut seen = 0u32;
    let mut reused = false;
    for _ in 0..3 {
        // Includes IDR restarts with a retained output.
        let mut bytes = include_bytes!("data/h264-40-frames.bin").as_slice();
        while !bytes.is_empty() {
            let size = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
            let unit = &bytes[4..4 + size];
            bytes = &bytes[4 + size..];
            assert_eq!(
                stream.read(unit).unwrap(),
                lowlat_decode::h264::Read::Picture
            );
            let target = stream.job().unwrap().current.slot;
            assert_ne!(
                Some(target),
                selected,
                "Selected output must not be overwritten"
            );
            reused |= seen & (1 << target) != 0;
            seen |= 1 << target;
            stream.finish().unwrap();
            if let Some(slot) = selection.latest(&mut stream.dpb) {
                selected = Some(slot);
            }
            assert!(selected.is_some());
            assert!(selection.latest(&mut stream.dpb).is_none());
        }
    }
    assert!(reused);
    selection.clear(&mut stream.dpb);
}

#[test]
fn linux_selected_hevc_surface_stays_leased_without_exhausting_the_dpb() {
    for fixture in [
        include_bytes!("data/hevc-40-frames.bin").as_slice(),
        include_bytes!("data/hevc-nvenc-ll-8-frames.bin").as_slice(),
    ] {
        let mut stream = lowlat_decode::hevc::Stream::new();
        let mut selection = output::Selection::default();
        let mut selected = None;
        for _ in 0..3 {
            let mut bytes = fixture;
            while !bytes.is_empty() {
                let size = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
                let unit = &bytes[4..4 + size];
                bytes = &bytes[4 + size..];
                assert_eq!(
                    stream.read(unit).unwrap(),
                    lowlat_decode::hevc::Read::Picture
                );
                assert_ne!(Some(stream.job().unwrap().current.slot), selected);
                stream.finish().unwrap();
                if let Some(slot) = selection.latest(&mut stream.dpb) {
                    selected = Some(slot);
                }
                assert!(selected.is_some());
                assert!(selection.latest(&mut stream.dpb).is_none());
            }
        }
        selection.clear(&mut stream.dpb);
    }
}
