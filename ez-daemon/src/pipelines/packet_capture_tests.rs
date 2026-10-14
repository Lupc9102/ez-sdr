use super::*;
use crate::bus::SampleBus;
use crate::hardware::adsb::{encode_uc8, EXAMPLE_DF17_CALLSIGN};
use ez_proto::{CaptureClock, CaptureTimestamp};
use num_complex::Complex32;
use std::sync::Arc;

fn pipeline() -> PacketPipeline {
    PacketPipeline::new(
        SampleBus::new().subscribe(8, OverflowPolicy::DropIncoming),
        2_400_000,
    )
    .unwrap()
}

fn position_messages() -> [ModesMessage; 2] {
    [
        [0x58, 0x00, 0x06, 0x43, 0x5D, 0xA9, 0xC2],
        [0x58, 0x00, 0x02, 0xD6, 0x91, 0xBB, 0xC9],
    ]
    .map(|me| {
        let mut raw = [0; 14];
        raw[..4].copy_from_slice(&[0x8d, 0x3c, 0x4b, 0x12]);
        raw[4..11].copy_from_slice(&me);
        let crc = dump1090::crc::crc24(&raw[..11]);
        raw[11..].copy_from_slice(&crc.to_be_bytes()[1..]);
        ModesMessage {
            msg: raw,
            verbatim: raw,
            msgbits: 112,
            msgtype: 17,
            addr: 0x3c4b12,
            ..Default::default()
        }
    })
}

fn block(raw: &[u8], start_sample: u64, clock: CaptureClock) -> SampleBlock {
    SampleBlock {
        product_session: 0,
        observed_at: None,
        capture: Some(clock),
        start_sample,
        sample_rate_hz: 2_400_000,
        center_freq_hz: 1_090_000_000,
        samples: Arc::from(lrpt_decode::uc8_bytes_to_normalized(raw)),
    }
}

fn capture_clock(source_sample_rate_hz: u32, sample_stride: u32) -> CaptureClock {
    let mut clock = CaptureClock::new(1, 0, source_sample_rate_hz, None);
    clock.sample_stride = sample_stride;
    clock
}

fn merge_at(pipeline: &mut PacketPipeline, mm: &ModesMessage, capture: CaptureTimestamp) {
    pipeline.capture_now = capture_cpr_time(capture);
    pipeline.capture_observed_at = Some(Instant::now());
    pipeline.merge_message_at(mm, Some(capture));
}

#[test]
fn daemon_forward_iq_loss_keeps_fresh_published_fix_and_cpr_pair() {
    let frames = position_messages();
    let pair = encode_uc8(&[frames[0].msg, frames[1].msg]);
    let mut callsign = EXAMPLE_DF17_CALLSIGN;
    callsign[1..4].copy_from_slice(&[0x3c, 0x4b, 0x12]);
    let crc = dump1090::crc::crc24(&callsign[..11]);
    callsign[11..].copy_from_slice(&crc.to_be_bytes()[1..]);
    let identity = encode_uc8(&[callsign]);
    let clock = CaptureClock::new(71, 0, 2_400_000, None);
    let mut pipeline = pipeline();
    let output = pipeline.subscribe(8);
    pipeline.process_block(&block(&pair, 0, clock));
    let original = pipeline.aircraft[&0x3c4b12].clone();
    assert!(original.lat.is_some());
    let stamp = pipeline.capture_position_seen[&0x3c4b12];
    let offset = pair.len() as u64 / 2 + 16_384;
    let mut next = clock.offset_samples(offset).unwrap();
    next.discontinuous = true;
    pipeline.process_block(&block(&identity, offset, next));
    pipeline.prune_and_publish();
    let aircraft = output.try_recv().unwrap().aircraft.pop().unwrap();
    assert_eq!(aircraft.callsign.as_deref(), Some("KLM 63EW"));
    assert_eq!(
        aircraft.lat.zip(aircraft.lon),
        original.lat.zip(original.lon)
    );
    assert_eq!(pipeline.capture_position_seen[&0x3c4b12], stamp);

    let expired_offset = offset + 61 * 2_400_000;
    let mut expired = clock.offset_samples(expired_offset).unwrap();
    expired.discontinuous = true;
    pipeline.process_block(&block(&identity, expired_offset, expired));
    pipeline.prune_and_publish();
    assert!(output.try_recv().unwrap().aircraft[0].lat.is_none());

    for seconds in [1, 11] {
        let mut pipeline = self::pipeline();
        pipeline.process_block(&block(&encode_uc8(&[frames[0].msg]), 0, clock));
        let offset = seconds * 2_400_000;
        let mut next = clock.offset_samples(offset).unwrap();
        next.discontinuous = true;
        pipeline.process_block(&block(&encode_uc8(&[frames[1].msg]), offset, next));
        assert_eq!(pipeline.aircraft[&0x3c4b12].lat.is_some(), seconds == 1);
    }
}

#[test]
fn daemon_unknown_regressed_or_changed_geometry_cannot_retain_a_fix() {
    let frames = position_messages();
    let pair = encode_uc8(&[frames[0].msg, frames[1].msg]);
    let single = encode_uc8(&[frames[0].msg]);
    for mode in [
        "unknown",
        "backwards",
        "center",
        "origin",
        "stride",
        "incompatible",
    ] {
        let mut pipeline = pipeline();
        let clock = CaptureClock::new(if mode == "unknown" { 0 } else { 71 }, 0, 2_400_000, None);
        pipeline.process_block(&block(&pair, 0, clock));
        assert!(pipeline.aircraft[&0x3c4b12].lat.is_some());
        let mut offset = pair.len() as u64 / 2 + 16_384;
        let mut next = clock.offset_samples(offset).unwrap();
        next.discontinuous = true;
        match mode {
            "backwards" => {
                offset = 0;
                next.source_sample_index = 0;
            }
            "origin" => next.origin_unix_ns = Some(1_000),
            "stride" => {
                next.sample_stride = 2;
                next.source_sample_rate_hz = 4_800_000;
            }
            "incompatible" => next.source_sample_rate_hz = 3_200_000,
            _ => {}
        }
        let mut next_block = block(&single, offset, next);
        if mode == "center" {
            next_block.center_freq_hz += 1_000_000;
        }
        pipeline.process_block(&next_block);
        assert!(
            pipeline.aircraft[&0x3c4b12].lat.is_none(),
            "retained fix across {mode}"
        );
    }
}

#[test]
fn daemon_capture_pairing_and_position_age_ignore_arrival_timestamps() {
    let [mut odd, mut even] = position_messages();
    let clock = CaptureClock::new(4, 0, 2_400_000, None);
    for (seconds, eligible) in [(9, true), (15, false)] {
        let mut pipeline = pipeline();
        let output = pipeline.subscribe(8);
        odd.sys_timestamp_msg = 100;
        even.sys_timestamp_msg = 900_100; // a long processing/backpressure delay
        merge_at(
            &mut pipeline,
            &odd,
            clock.message_timestamp_2400(0).unwrap(),
        );
        merge_at(
            &mut pipeline,
            &even,
            clock.message_timestamp_2400(seconds * 12_000_000).unwrap(),
        );
        assert_eq!(pipeline.aircraft[&0x3c4b12].lat.is_some(), eligible);
        pipeline.prune_and_publish_at(1_000_000);
        let snapshot = output.try_recv().unwrap();
        assert_eq!(snapshot.aircraft.len(), 1);
        assert_eq!(snapshot.aircraft[0].lat.is_some(), eligible);
        assert!(snapshot.aircraft[0].last_seen_ms > 999_900);
        if eligible {
            assert!((snapshot.aircraft[0].lat.unwrap() - 52.2572).abs() < 0.001);
            assert!((snapshot.aircraft[0].lon.unwrap() - 8.6676).abs() < 0.001);
            // Advancing the capture, rather than the processing clock, expires
            // the old position despite recent nonposition traffic.
            let callsign = ModesMessage {
                msg: EXAMPLE_DF17_CALLSIGN,
                msgbits: 112,
                msgtype: 17,
                addr: 0x4840d6,
                sys_timestamp_msg: 1_000_001,
                ..Default::default()
            };
            merge_at(
                &mut pipeline,
                &callsign,
                clock.message_timestamp_2400(71 * 12_000_000).unwrap(),
            );
            pipeline.prune_and_publish_at(1_000_002);
            assert!(output
                .try_recv()
                .unwrap()
                .aircraft
                .iter()
                .all(|ac| ac.icao != 0x3c4b12));
        }
    }
    // Equal processing instants cannot make a 15-second captured pair eligible.
    odd.sys_timestamp_msg = 123;
    even.sys_timestamp_msg = 123;
    let mut fast = pipeline();
    merge_at(&mut fast, &odd, clock.message_timestamp_2400(0).unwrap());
    merge_at(
        &mut fast,
        &even,
        clock.message_timestamp_2400(15 * 12_000_000).unwrap(),
    );
    assert!(fast.aircraft[&0x3c4b12].lat.is_none());
}

#[test]
fn daemon_real_iq_overlap_phase_and_exact_capture_clock_survive_merging() {
    let frames = position_messages();
    let raw = encode_uc8(&[frames[0].msg, frames[1].msg]);
    let mut clock = CaptureClock::new(8, 1_001, 16_800_000, Some(1_500_000_000));
    clock.sample_stride = 7;
    let mut split = pipeline();
    split.process_block(&block(&raw[..260], 143, clock));
    split.process_block(&block(&raw[260..], 273, clock.offset_samples(130).unwrap()));
    let stamp = split.capture_position_seen[&0x3c4b12];
    assert_eq!(stamp.epoch, 8);
    assert_eq!(stamp.source_sample_rate_hz, 16_800_000);
    assert!(stamp.unix_time_ns().unwrap() > 1_500_000_000);
    let mut whole = pipeline();
    whole.process_block(&block(&raw, 143, clock));
    assert_eq!(stamp, whole.capture_position_seen[&0x3c4b12]);
    let mut output_clock = pipeline();
    output_clock.process_block(&block(&raw, 0, CaptureClock::new(1, 0, 2_400_000, None)));
    let output = output_clock.capture_position_seen[&0x3c4b12];
    assert_eq!(
        stamp,
        clock
            .message_timestamp_2400(
                i128::from(output.source_sample_index) * 5
                    + i128::from(output.source_sample_phase_fifths)
            )
            .unwrap()
    );
    assert!((split.aircraft[&0x3c4b12].lat.unwrap() - 52.2572).abs() < 0.001);
}

#[test]
fn packet_decoder_rejects_incompatible_capture_clocks_and_resets_overlap() {
    let raw = encode_uc8(&[EXAMPLE_DF17_CALLSIGN]);
    let cut = raw.len() / 2;
    let mut pipeline = pipeline();

    pipeline.process_block(&block(&raw[..cut], 0, capture_clock(2_400_000, 1)));
    assert!(!pipeline.overlap_tail.is_empty());

    pipeline.process_block(&block(&raw[cut..], cut as u64, capture_clock(4_800_001, 2)));
    assert_eq!(pipeline.stats.demod_accepted.iter().sum::<u64>(), 0);
    assert!(pipeline.overlap_tail.is_empty());

    let zero_stride = capture_clock(4_800_000, 0);
    pipeline.process_block(&block(&raw, 0, zero_stride));
    let direct_3_2_mhz = SampleBlock {
        product_session: 0,
        observed_at: None,
        capture: None,
        start_sample: 0,
        sample_rate_hz: 3_200_000,
        center_freq_hz: 1_090_000_000,
        samples: Arc::from(lrpt_decode::uc8_bytes_to_normalized(&raw)),
    };
    pipeline.process_block(&direct_3_2_mhz);
    assert_eq!(pipeline.stats.demod_accepted.iter().sum::<u64>(), 0);
    assert!(pipeline.overlap_tail.is_empty());

    let mut exact_2_4_mhz = capture_clock(4_800_000, 2);
    exact_2_4_mhz.epoch = 2;
    pipeline.process_block(&block(&raw, 0, exact_2_4_mhz));
    assert_eq!(pipeline.stats.demod_accepted.iter().sum::<u64>(), 1);
}

#[test]
fn daemon_forward_gap_preserves_messages_but_epoch_and_split_loss_reset() {
    let frames = position_messages();
    let pair = encode_uc8(&[frames[0].msg, frames[1].msg]);
    let odd = encode_uc8(&[frames[0].msg]);
    let clock = CaptureClock::new(19, 0, 2_400_000, None);
    for rewind in [false, true] {
        let mut pipeline = pipeline();
        pipeline.process_block(&block(&pair, 0, clock));
        assert!(pipeline.aircraft[&0x3c4b12].lat.is_some());
        let mut next = clock.offset_samples(pair.len() as u64 / 2).unwrap();
        let mut start = pair.len() as u64 / 2;
        if rewind {
            next.epoch += 1;
            next.source_sample_index = 0;
            start = 0;
        } else {
            next.source_sample_index += 1;
            start += 1;
        }
        pipeline.process_block(&block(&odd, start, next));
        assert_eq!(pipeline.aircraft[&0x3c4b12].lat.is_some(), !rewind);
    }
    let raw = encode_uc8(&[EXAMPLE_DF17_CALLSIGN]);
    let mut pipeline = pipeline();
    pipeline.process_block(&block(&raw[..260], 0, clock));
    pipeline.process_block(&block(&raw[260..], 131, clock.offset_samples(131).unwrap()));
    assert_eq!(pipeline.stats.demod_accepted.iter().sum::<u64>(), 0);
    pipeline.process_block(&block(
        &raw,
        raw.len() as u64 / 2 + 1,
        clock.offset_samples(raw.len() as u64 / 2 + 1).unwrap(),
    ));
    assert_eq!(pipeline.stats.demod_accepted.iter().sum::<u64>(), 1);
}

#[test]
fn daemon_idle_snapshot_expires_but_retains_capture_state_for_backpressured_replay() {
    let frames = position_messages();
    let raw = encode_uc8(&[frames[0].msg, frames[1].msg]);
    let clock = CaptureClock::new(7, 0, 2_400_000, None);
    let mut pipeline = pipeline();
    let output = pipeline.subscribe(8);
    pipeline.process_block(&block(&raw, 0, clock));
    pipeline.capture_observed_at = Some(Instant::now() - Duration::from_secs(61));
    pipeline.prune_and_publish();
    assert!(output.try_recv().unwrap().aircraft.is_empty());
    assert_eq!(pipeline.tracker.len(), 1);
    assert_eq!(pipeline.cpr.len(), 1);
    let next = encode_uc8(&[frames[1].msg]);
    pipeline.process_block(&block(
        &next,
        raw.len() as u64 / 2,
        clock.offset_samples(raw.len() as u64 / 2).unwrap(),
    ));
    pipeline.prune_and_publish();
    assert_eq!(output.try_recv().unwrap().aircraft.len(), 1);
    assert!(pipeline.aircraft[&0x3c4b12].lat.is_some());
}

#[test]
fn daemon_replay_positions_and_rewind_boundaries_match_at_all_speeds() {
    use crate::hardware::{
        replay::{FileReplaySource, ReplayFormat},
        IqSource,
    };
    let frames = position_messages();
    let odd = encode_uc8(&[frames[0].msg]);
    let even = encode_uc8(&[frames[1].msg]);
    let mut raw = vec![127; 2 * 65_536];
    raw[..odd.len()].copy_from_slice(&odd);
    let end = raw.len();
    raw[end - even.len()..].copy_from_slice(&even);
    let path = std::env::temp_dir().join(format!("ez-a08-daemon-{}.cu8", std::process::id()));
    std::fs::write(&path, raw).unwrap();
    let mut positions = Vec::new();
    for speed in [0.1, 1.0, 10.0] {
        let mut replay = FileReplaySource::open(
            path.to_str().unwrap(),
            ReplayFormat::RawU8,
            2_400_000,
            true,
            speed,
        )
        .unwrap();
        replay.start().unwrap();
        let mut pipeline = pipeline();
        let mut buf = vec![Complex32::default(); 32_768];
        let mut prior_epoch = None;
        for _ in 0..3 {
            let mut epoch = 0;
            for index in 0..2 {
                assert_eq!(replay.read_iq(&mut buf).unwrap(), 32_768);
                let clock = replay.capture_clock().unwrap();
                assert_eq!(clock.source_sample_index, index * 32_768);
                assert_eq!(clock.discontinuous, index == 0);
                assert_eq!(clock.origin_unix_ns, None);
                if index == 0 {
                    epoch = clock.epoch;
                    assert_ne!(Some(epoch), prior_epoch);
                } else {
                    assert_eq!(clock.epoch, epoch);
                }
                pipeline.process_block(&SampleBlock {
                    product_session: 0,
                    observed_at: None,
                    capture: Some(clock),
                    start_sample: clock.source_sample_index,
                    sample_rate_hz: 2_400_000,
                    center_freq_hz: 1_090_000_000,
                    samples: Arc::from(buf.clone()),
                });
                let ac = &pipeline.aircraft[&0x3c4b12];
                assert_eq!(
                    ac.lat.is_some(),
                    index == 1,
                    "cross-loop position at {speed}x"
                );
                if index == 1 {
                    positions.push(ac.lat.zip(ac.lon).unwrap());
                }
            }
            prior_epoch = Some(epoch);
        }
        replay.stop();
    }
    std::fs::remove_file(path).unwrap();
    assert!(positions.windows(2).all(|pair| pair[0] == pair[1]));
    assert!((positions[0].0 - 52.2572).abs() < 0.001);
    assert!((positions[0].1 - 8.6676).abs() < 0.001);
}

#[test]
fn retained_capture_state_remains_bounded_during_dense_bursts() {
    let mut pipeline = pipeline();
    let clock = CaptureClock::new(2, 0, 2_400_000, None);
    for address in 1..=4097 {
        let mut mm = position_messages()[0].clone();
        mm.addr = address;
        mm.msg[1..4].copy_from_slice(&address.to_be_bytes()[1..]);
        let crc = dump1090::crc::crc24(&mm.msg[..11]);
        mm.msg[11..].copy_from_slice(&crc.to_be_bytes()[1..]);
        mm.sys_timestamp_msg = 100;
        merge_at(
            &mut pipeline,
            &mm,
            clock
                .message_timestamp_2400(i128::from(address) * 5)
                .unwrap(),
        );
    }
    assert_eq!(pipeline.tracker.len(), 4096);
    assert_eq!(pipeline.capture_seen.len(), 4096);
    assert_eq!(pipeline.aircraft.len(), 4096);
    assert!(!pipeline.tracker.aircraft.contains_key(&1));
    assert!(pipeline.tracker.aircraft.contains_key(&4097));
}
