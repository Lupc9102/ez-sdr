use super::*;
use ez_daemon::hardware::adsb::{encode_uc8, EXAMPLE_DF17_CALLSIGN};
use ez_proto::{CaptureClock, CaptureTimestamp};
use std::time::{Duration, Instant};

pub(crate) fn position_messages(icao: u32) -> [[u8; 14]; 2] {
    [
        [0x58, 0x00, 0x06, 0x43, 0x5D, 0xA9, 0xC2], // odd first
        [0x58, 0x00, 0x02, 0xD6, 0x91, 0xBB, 0xC9], // even latest
    ]
    .map(|me| {
        let mut raw = [0; 14];
        raw[..4].copy_from_slice(&[0x8d, (icao >> 16) as u8, (icao >> 8) as u8, icao as u8]);
        raw[4..11].copy_from_slice(&me);
        let crc = dump1090::crc::crc24(&raw[..11]);
        raw[11..].copy_from_slice(&crc.to_be_bytes()[1..]);
        raw
    })
}

fn merge_at(decoder: &mut AdsBDecoder, raw: [u8; 14], stamp: CaptureTimestamp) {
    decoder.capture_now = capture_cpr_time(stamp);
    decoder.capture_observed_at = Some(Instant::now());
    decoder.process_decoded_at(
        0x3c4b12,
        17,
        raw,
        1.0,
        true,
        0,
        dump1090::demod::ScoreRank::Df17Known,
        Some(stamp),
    );
}

#[test]
fn forward_iq_loss_keeps_fresh_fix_and_independent_cpr_frames() {
    let [odd, even] = position_messages(0x4840d6);
    let pair = encode_uc8(&[odd, even]);
    let callsign = encode_uc8(&[EXAMPLE_DF17_CALLSIGN]);
    let clock = CaptureClock::new(71, 0, 2_400_000, None);
    let mut decoder = AdsBDecoder::new();
    decoder.feed_iq_with_clock(&pair, clock, InputFormat::Uc8);
    let original = decoder.get_aircraft().pop().unwrap();
    assert!(original.position_valid());
    let position_stamp = decoder.aircraft[&0x4840d6].capture_position_seen;

    // Lose one normal input buffer, then receive only identity data. Dropped
    // samples are not grounds for erasing a previously validated coordinate.
    let mut next = clock
        .offset_samples(pair.len() as u64 / 2 + 16_384)
        .unwrap();
    next.discontinuous = true;
    decoder.feed_iq_with_clock(&callsign, next, InputFormat::Uc8);
    let aircraft = decoder.get_aircraft().pop().unwrap();
    assert_eq!(aircraft.callsign, "KLM 63EW");
    assert!(
        aircraft.position_valid(),
        "lost buffer erased a fresh position"
    );
    assert_eq!((aircraft.lat, aircraft.lon), (original.lat, original.lon));
    assert_eq!(
        decoder.aircraft[&0x4840d6].capture_position_seen,
        position_stamp
    );

    // Freshness still follows acquisition time; identity cannot rejuvenate it.
    let mut expired = next.offset_samples(61 * 2_400_000).unwrap();
    expired.discontinuous = true;
    decoder.feed_iq_with_clock(&callsign, expired, InputFormat::Uc8);
    assert!(!decoder.get_aircraft()[0].position_valid());

    for seconds in [1, 11] {
        let mut decoder = AdsBDecoder::new();
        decoder.feed_iq_with_clock(&encode_uc8(&[odd]), clock, InputFormat::Uc8);
        let mut next = clock.offset_samples(seconds * 2_400_000).unwrap();
        next.discontinuous = true;
        decoder.feed_iq_with_clock(&encode_uc8(&[even]), next, InputFormat::Uc8);
        assert_eq!(decoder.get_aircraft()[0].position_valid(), seconds == 1);
    }
}

#[test]
fn crc_valid_weak_position_frames_survive_the_semantic_merge() {
    let [odd, even] = position_messages(0x4840d6);
    let mut decoder = AdsBDecoder::new();
    decoder.feed_iq(&encode_uc8(&[EXAMPLE_DF17_CALLSIGN]), 2_400_000);
    let mut weak = encode_uc8(&[odd, even]);
    for components in weak.chunks_exact_mut(2) {
        if components[0] > 127 {
            components[0] = 132;
        }
    }
    let mut invalid = [odd, even];
    for frame in &mut invalid {
        frame[11] ^= 1; // CRC parity damage must still be rejected.
    }
    let mut invalid_weak = encode_uc8(&invalid);
    for components in invalid_weak.chunks_exact_mut(2) {
        if components[0] > 127 {
            components[0] = 132;
        }
    }
    decoder.feed_iq(&invalid_weak, 2_400_000);
    assert_eq!(decoder.stats().1, 1);
    assert!(!decoder.get_aircraft()[0].position_valid());
    decoder.feed_iq(&weak, 2_400_000);
    assert_eq!(
        decoder.stats().1,
        3,
        "demodulator must validate both weak frames"
    );
    let aircraft = decoder.get_aircraft().pop().unwrap();
    assert_eq!(aircraft.callsign, "KLM 63EW");
    assert!(
        aircraft.position_valid(),
        "accepted position frames were discarded during merging"
    );
}

#[test]
fn capture_pairing_ignores_processing_age_and_rejects_fast_stale_capture() {
    let [odd, even] = position_messages(0x3c4b12);
    for (interval, eligible) in [(9, true), (15, false)] {
        let mut decoder = AdsBDecoder::new();
        let origin = CaptureClock::new(17, 0, 2_400_000, None);
        merge_at(&mut decoder, odd, origin.message_timestamp_2400(0).unwrap());
        // Model a long queue/CPU pause without waiting 90 seconds. Both arrival
        // timestamps are stale, while the actual message clocks remain intact.
        let state = decoder.aircraft.get_mut(&0x3c4b12).unwrap();
        state.seen = Instant::now() - Duration::from_secs(900);
        state.entry.seen = state.seen;
        merge_at(
            &mut decoder,
            even,
            origin
                .message_timestamp_2400(interval * 12_000_000)
                .unwrap(),
        );
        let ac = decoder.get_aircraft().pop().unwrap();
        assert_eq!(ac.has_position, eligible);
        if eligible {
            assert!((ac.lat - 52.2572).abs() < 0.001);
            assert!((ac.lon - 8.6676).abs() < 0.001);
            assert!(ac.seen.elapsed() < Duration::from_secs(1));
        }
    }
}

#[test]
fn real_iq_message_timestamp_retains_overlap_phase_epoch_and_exact_native_rate() {
    let [odd, even] = position_messages(0x3c4b12);
    let raw = encode_uc8(&[odd, even]);
    let mut clock = CaptureClock::new(23, 1_001, 16_800_003, None);
    clock.sample_stride = 7;
    let mut decoder = AdsBDecoder::new();
    decoder.feed_iq_with_clock(&raw[..260], clock, InputFormat::Uc8);
    decoder.feed_iq_with_clock(
        &raw[260..],
        clock.offset_samples(130).unwrap(),
        InputFormat::Uc8,
    );
    let ac = decoder.get_aircraft().pop().unwrap();
    assert!(ac.has_position);
    assert!((ac.lat - 52.2572).abs() < 0.001);
    assert!((ac.lon - 8.6676).abs() < 0.001);
    let stamp = decoder.aircraft[&0x3c4b12].capture_position_seen.unwrap();
    assert_eq!(stamp.epoch, 23);
    assert_eq!(stamp.source_sample_rate_hz, 16_800_003);
    assert_eq!(stamp.unix_time_ns(), None);
    let mut whole = AdsBDecoder::new();
    whole.feed_iq_with_clock(&raw, clock, InputFormat::Uc8);
    assert_eq!(
        stamp,
        whole.aircraft[&0x3c4b12].capture_position_seen.unwrap()
    );
    let mut output_clock = AdsBDecoder::new();
    output_clock.feed_iq_with_clock(
        &raw,
        CaptureClock::new(1, 0, 2_400_000, None),
        InputFormat::Uc8,
    );
    let output = output_clock.aircraft[&0x3c4b12]
        .capture_position_seen
        .unwrap();
    let ticks =
        i128::from(output.source_sample_index) * 5 + i128::from(output.source_sample_phase_fifths);
    assert_eq!(stamp, clock.message_timestamp_2400(ticks).unwrap());
    let age = capture_cpr_time(stamp).unwrap();
    let end = clock
        .message_timestamp_2400(raw.len() as i128 / 2 * 5)
        .and_then(capture_cpr_time)
        .unwrap();
    assert!(
        ac.position_seen
            .unwrap()
            .elapsed()
            .abs_diff(end.checked_duration_since(age).unwrap())
            < Duration::from_millis(20)
    );
}

#[test]
fn real_iq_forward_gap_preserves_messages_but_loop_epoch_and_split_loss_reset() {
    let [odd, even] = position_messages(0x3c4b12);
    let first = encode_uc8(&[odd, even]);
    let single = encode_uc8(&[odd]);
    for new_epoch in [false, true] {
        let mut decoder = AdsBDecoder::new();
        let clock = CaptureClock::new(10, 0, 2_400_000, None);
        decoder.feed_iq_with_clock(&first, clock, InputFormat::Uc8);
        assert!(decoder.get_aircraft()[0].has_position);
        let mut next = clock.offset_samples(first.len() as u64 / 2).unwrap();
        if new_epoch {
            next.epoch = 11;
            next.source_sample_index = 0;
        } else {
            next.source_sample_index += 1;
        }
        decoder.feed_iq_with_clock(&single, next, InputFormat::Uc8);
        assert_eq!(decoder.get_aircraft()[0].has_position, !new_epoch);
        assert_eq!(
            decoder.aircraft[&0x3c4b12].capture_seen.unwrap().epoch,
            next.epoch
        );
    }
    let raw = encode_uc8(&[EXAMPLE_DF17_CALLSIGN]);
    let mut decoder = AdsBDecoder::new();
    decoder.feed_iq_with_clock(
        &raw[..260],
        CaptureClock::new(2, 0, 2_400_000, None),
        InputFormat::Uc8,
    );
    decoder.feed_iq_with_clock(
        &raw[260..],
        CaptureClock::new(2, 131, 2_400_000, None),
        InputFormat::Uc8,
    );
    assert_eq!(
        decoder.total_messages, 0,
        "split frame crossed missing samples"
    );
    decoder.feed_iq_with_clock(
        &raw,
        CaptureClock::new(2, raw.len() as u64 / 2 + 1, 2_400_000, None),
        InputFormat::Uc8,
    );
    assert_eq!(decoder.total_messages, 1);
}

#[test]
fn unknown_or_regressed_clock_and_explicit_reset_discard_old_position() {
    let [odd, even] = position_messages(0x4840d6);
    let pair = encode_uc8(&[odd, even]);
    let identity = encode_uc8(&[EXAMPLE_DF17_CALLSIGN]);
    for mode in ["unknown", "backwards", "reset", "origin", "format"] {
        let mut decoder = AdsBDecoder::new();
        let clock = CaptureClock::new(if mode == "unknown" { 0 } else { 71 }, 0, 2_400_000, None);
        decoder.feed_iq_with_clock(&pair, clock, InputFormat::Uc8);
        assert!(decoder.get_aircraft()[0].position_valid());
        let mut next = clock
            .offset_samples(pair.len() as u64 / 2 + 16_384)
            .unwrap();
        next.discontinuous = true;
        match mode {
            "backwards" => next.source_sample_index = 0,
            "reset" => decoder.mark_discontinuous(),
            "origin" => next.origin_unix_ns = Some(1_000),
            "format" => {
                decoder.feed_iq_with_clock(&[0; 4096], next, InputFormat::Sc16);
                assert!(decoder.get_aircraft().is_empty());
                continue;
            }
            _ => {}
        }
        decoder.feed_iq_with_clock(&identity, next, InputFormat::Uc8);
        assert!(
            !decoder.get_aircraft()[0].position_valid(),
            "retained fix across {mode}"
        );
    }
}

#[test]
fn capture_position_age_and_idle_display_expiry_preserve_delayed_replay_state() {
    let [odd, even] = position_messages(0x3c4b12);
    let raw = encode_uc8(&[odd, even]);
    let clock = CaptureClock::new(5, 0, 2_400_000, None);
    let mut decoder = AdsBDecoder::new();
    decoder.feed_iq_with_clock(&raw, clock, InputFormat::Uc8);
    decoder.capture_observed_at = Some(Instant::now() - Duration::from_secs(61));
    assert!(!decoder.get_aircraft()[0].has_position);
    decoder.capture_observed_at = Some(Instant::now() - Duration::from_secs(601));
    assert!(decoder.get_aircraft().is_empty());
    assert_eq!(decoder.aircraft.len(), 1); // bounded internal captured reference
    let followup = encode_uc8(&[EXAMPLE_DF17_CALLSIGN]);
    decoder.feed_iq_with_clock(
        &followup,
        clock.offset_samples(raw.len() as u64 / 2).unwrap(),
        InputFormat::Uc8,
    );
    assert!(
        decoder
            .get_aircraft()
            .iter()
            .find(|ac| ac.icao == 0x3c4b12)
            .unwrap()
            .has_position
    );
    decoder.capture_now = Some(dump1090::cpr::CprTimestamp::Capture {
        epoch: 5,
        elapsed: Duration::from_secs(61),
    });
    decoder.capture_observed_at = Some(Instant::now());
    assert!(
        !decoder
            .get_aircraft()
            .iter()
            .find(|ac| ac.icao == 0x3c4b12)
            .unwrap()
            .has_position
    );
}

#[test]
fn rejected_input_does_not_hide_the_next_capture_epoch_change() {
    let mut decoder = AdsBDecoder::new();
    let raw = encode_uc8(&[EXAMPLE_DF17_CALLSIGN]);
    decoder.feed_iq_with_clock(
        &raw,
        CaptureClock::new(30, 0, 2_400_000, None),
        InputFormat::Uc8,
    );
    decoder.feed_iq_with_clock(
        &raw,
        CaptureClock::new(31, 0, 2_000_000, None),
        InputFormat::Uc8,
    );
    assert!(decoder.sample_rate_mismatch());
    let [odd, _] = position_messages(0x3c4b12);
    decoder.feed_iq_with_clock(
        &encode_uc8(&[odd]),
        CaptureClock::new(32, 0, 2_400_000, None),
        InputFormat::Uc8,
    );
    assert!(!decoder.sample_rate_mismatch());
    assert_eq!(decoder.aircraft.len(), 1);
    assert!(decoder.aircraft.contains_key(&0x3c4b12));
    assert!(!decoder.aircraft.contains_key(&0x4840d6));
}
