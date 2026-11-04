#[path = "../src/video_vulkan/output.rs"]
mod output;
#[path = "../src/video_vulkan/parameters.rs"]
mod parameters;
#[path = "../src/playback.rs"]
mod playback;

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
