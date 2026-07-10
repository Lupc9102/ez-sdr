//! Event-driven ADS-B/Mode-S packet decoding: turns a stream of [`SampleBlock`]s into
//! per-aircraft [`AircraftTelemetry`] snapshots via `dump1090`'s demodulator/decoder.
//!
//! Unlike [`crate::pipelines::spectrum::SpectrumPipeline`]/[`crate::pipelines::audio::AudioPipeline`]
//! (continuous per-sample DSP producing output at a fixed cadence), this pipeline is
//! event-driven and bursty: most blocks decode to zero messages. A snapshot of every
//! currently-tracked aircraft is republished every [`PUBLISH_EVERY_N_BLOCKS`] processed
//! blocks, which also drives stale-aircraft pruning.
//!
//! `dump1090::track::Tracker` is kept as the single source of truth for per-ICAO liveness
//! and total message count (it sees every framed message, decodable or not); this
//! pipeline's own `aircraft` map only carries the semantic ADS-B fields (callsign,
//! altitude, position, velocity) that `Tracker` doesn't. The two are synced at publish
//! time so a client never sees a `msg_count` that undercounts CRC-valid-but-not-yet-fully-
//! decoded traffic.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dump1090::cpr::{CprDecoder, CprFrame, CprType};
use dump1090::crc::check_crc;
use dump1090::demod::{
    Demod2400, DemodStats, MagBuf, MagBufFlags, ModesMessage, MAGBUF_DISCONTINUOUS,
};
use dump1090::mode_s;
use dump1090::track::Tracker;

use ez_proto::AircraftTelemetry;

use crate::broadcast::{Broadcaster, BroadcasterHandle, OverflowPolicy};
use crate::bus::{SampleBlock, SampleBusHandle};

/// How many processed blocks between published snapshots. Packet traffic is bursty
/// (most blocks yield zero messages), so publishing is decoupled from real time and tied
/// instead to a fixed count of processed blocks — simple, predictable, and independent of
/// whatever sample rate/block size the channelizer feeds this pipeline.
const PUBLISH_EVERY_N_BLOCKS: u32 = 25;

/// An aircraft with no message in this long is dropped from both `Tracker` and the
/// published snapshot.
const STALE_AIRCRAFT_MS: u64 = 60_000;

pub struct PacketPipeline {
    input: SampleBusHandle,
    demod: Demod2400,
    stats: DemodStats,
    tracker: Tracker,
    cpr: CprDecoder,
    mag_buf: Vec<u16>,
    aircraft: HashMap<u32, AircraftTelemetry>,
    blocks_since_publish: u32,
    output: Broadcaster<Vec<AircraftTelemetry>>,
}

impl PacketPipeline {
    #[must_use]
    pub fn new(input: SampleBusHandle) -> Self {
        Self {
            input,
            demod: Demod2400::new(),
            stats: DemodStats::default(),
            tracker: Tracker::new(),
            cpr: CprDecoder::new(),
            mag_buf: Vec::new(),
            aircraft: HashMap::new(),
            blocks_since_publish: 0,
            output: Broadcaster::new(),
        }
    }

    /// Subscribes to published aircraft snapshots. Each value is a full replacement of
    /// every currently-tracked aircraft, so a slow subscriber should only ever want the
    /// newest one — hence [`OverflowPolicy::DropOldest`].
    #[must_use]
    pub fn subscribe(&self, capacity: usize) -> BroadcasterHandle<Vec<AircraftTelemetry>> {
        self.output.subscribe(capacity, OverflowPolicy::DropOldest)
    }

    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.output.subscriber_count()
    }

    #[must_use]
    pub fn tracked_count(&self) -> usize {
        self.tracker.len()
    }

    /// Polls the input bus once. Returns `true` if a block was consumed and processed,
    /// `false` on timeout — callers should check their own shutdown signal rather than
    /// treat a timeout as "the source is gone" (see
    /// [`crate::broadcast::BroadcasterHandle::recv_timeout`]).
    pub fn tick(&mut self, poll_timeout: Duration) -> bool {
        match self.input.recv_timeout(poll_timeout) {
            Some(block) => {
                self.process_block(&block);
                true
            }
            None => false,
        }
    }

    pub fn run(&mut self, running: &AtomicBool) {
        const POLL_INTERVAL: Duration = Duration::from_millis(250);
        while running.load(Ordering::Relaxed) {
            self.tick(POLL_INTERVAL);
        }
    }

    fn process_block(&mut self, block: &SampleBlock) {
        let mut sum_level = 0.0_f64;
        let mut sum_power = 0.0_f64;
        let mut data = std::mem::take(&mut self.mag_buf);
        data.clear();
        data.reserve(block.samples.len());
        for sample in block.samples.iter() {
            let norm = f64::from(sample.norm());
            sum_level += norm;
            sum_power += norm * norm;
            data.push((norm * 65535.0).min(65535.0) as u16);
        }
        let n = (block.samples.len().max(1)) as f64;

        let mut mag = MagBuf {
            data,
            total_length: block.samples.len(),
            valid_length: block.samples.len(),
            overlap: 0,
            sample_timestamp: block.start_sample,
            sys_timestamp: now_ms(),
            flags: MagBufFlags(MAGBUF_DISCONTINUOUS),
            mean_level: sum_level / n,
            mean_power: sum_power / n,
            dropped: 0,
        };

        // The demodulate callback can't safely borrow back into `self` while `self.demod`
        // is the receiver of this call, so messages are collected first and merged in a
        // second pass once the borrow ends.
        let mut messages: Vec<ModesMessage> = Vec::new();
        self.demod
            .demodulate(&mag, &mut self.stats, &mut |mm| messages.push(mm.clone()));

        self.mag_buf = std::mem::take(&mut mag.data);

        for mm in &messages {
            self.merge_message(mm);
        }

        self.blocks_since_publish += 1;
        if self.blocks_since_publish >= PUBLISH_EVERY_N_BLOCKS {
            self.blocks_since_publish = 0;
            self.prune_and_publish();
        }
    }

    fn merge_message(&mut self, mm: &ModesMessage) {
        // Tracker sees every framed message regardless of whether it semantically
        // decodes, since liveness/count tracking is coarser-grained than field decoding.
        self.tracker.update_from_message(mm);

        let Some(decoded) = mode_s::decode_mode_s_message(&mm.msg) else {
            return;
        };

        let entry = self
            .aircraft
            .entry(decoded.icao)
            .or_insert_with(|| AircraftTelemetry {
                icao: decoded.icao,
                ..Default::default()
            });

        if let Some(callsign) = decoded.callsign {
            entry.callsign = Some(callsign);
        }
        if let Some(altitude) = decoded.altitude {
            entry.altitude_ft = Some(altitude as i32);
        }
        if let Some((speed_kt, heading)) = decoded.velocity {
            entry.ground_speed_kt = Some(speed_kt);
            entry.track_deg = Some(heading);
        }

        // decode_mode_s_message already CRC-gates its own field extraction, but CPR
        // extraction is a separate code path this pipeline owns independently — feeding a
        // corrupted frame into the decoder risks poisoning a legitimate even/odd pairing
        // for this ICAO, so it gets its own explicit CRC guard.
        if matches!(mm.msgtype, 17 | 18) && check_crc(&mm.msg) {
            let tc = mm.msg[4] >> 3;
            if (9..=18).contains(&tc) {
                if let Some((lat, lon)) =
                    self.cpr.submit(decoded.icao, extract_airborne_cpr(&mm.msg))
                {
                    entry.lat = Some(lat);
                    entry.lon = Some(lon);
                }
            }
        }

        // Real ADS-B vertical rate lives in a TC19 subfield this dump1090 port doesn't
        // decode; left `None` rather than fabricated.
    }

    fn prune_and_publish(&mut self) {
        let cutoff_ms = now_ms().saturating_sub(STALE_AIRCRAFT_MS);
        self.tracker.prune_older_than(cutoff_ms);

        let tracker = &self.tracker;
        self.aircraft
            .retain(|icao, entry| match tracker.aircraft.get(icao) {
                Some(state) => {
                    entry.msg_count = state.msg_count;
                    entry.last_seen_ms = state.last_seen_ms;
                    true
                }
                None => false,
            });

        let snapshot: Vec<AircraftTelemetry> = self.aircraft.values().cloned().collect();
        self.output.publish(snapshot);
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Extracts a raw airborne CPR frame from a DF17/18 ME field (type codes 9-18).
///
/// The 56-bit ME field (`msg[4]..=msg[10]`) packs, with no overlap: TC/SS/SAF in
/// `msg[4]`; a 12-bit altitude spanning `msg[5]` and the top nibble of `msg[6]`; a T bit
/// and the odd/even F bit in `msg[6]`; a 17-bit CPR latitude spanning the low 2 bits of
/// `msg[6]`, all of `msg[7]`, and the top 7 bits of `msg[8]`; and a 17-bit CPR longitude
/// spanning the low bit of `msg[8]` and all of `msg[9]`/`msg[10]`.
fn extract_airborne_cpr(msg: &[u8; 14]) -> CprFrame {
    let odd = (msg[6] & 0x04) != 0;
    let lat =
        (u32::from(msg[6] & 0x03) << 15) | (u32::from(msg[7]) << 7) | (u32::from(msg[8]) >> 1);
    let lon = (u32::from(msg[8] & 0x01) << 16) | (u32::from(msg[9]) << 8) | u32::from(msg[10]);
    CprFrame {
        cpr_type: CprType::Airborne,
        odd,
        lat,
        lon,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SampleBus;
    use num_complex::Complex32;

    fn test_pipeline() -> (SampleBus, PacketPipeline) {
        let bus = SampleBus::new();
        let handle = bus.subscribe(8, OverflowPolicy::DropIncoming);
        (bus, PacketPipeline::new(handle))
    }

    fn silence_block(len: usize) -> SampleBlock {
        SampleBlock {
            start_sample: 0,
            sample_rate_hz: 2_400_000,
            center_freq_hz: 1_090_000_000,
            samples: std::sync::Arc::from(vec![Complex32::new(0.0, 0.0); len]),
        }
    }

    /// Builds a synthetic, CRC-valid DF17 message from a 7-byte ME field
    /// (`msg[4]..=msg[10]`).
    fn synthetic_df17(icao: u32, me_field: [u8; 7]) -> ModesMessage {
        let mut raw = [0u8; 14];
        raw[0] = 0x8D;
        raw[1] = (icao >> 16) as u8;
        raw[2] = (icao >> 8) as u8;
        raw[3] = icao as u8;
        raw[4..11].copy_from_slice(&me_field);
        let crc = dump1090::crc::crc24(&raw[..11]);
        raw[11] = (crc >> 16) as u8;
        raw[12] = (crc >> 8) as u8;
        raw[13] = crc as u8;
        ModesMessage {
            msg: raw,
            verbatim: raw,
            msgbits: 112,
            msgtype: 17,
            addr: icao,
            ..ModesMessage::default()
        }
    }

    // Reused directly from dump1090::mode_s's own test module: a real DF17 identification
    // message (icao=0x4840D6) decoding to callsign "KLM 63EW".
    const VALID_DF17_CALLSIGN: [u8; 14] = [
        0x8D, 0x48, 0x40, 0xD6, 0x20, 0x2C, 0xC3, 0x7C, 0xDB, 0x31, 0x57, 0x9F, 0xE8, 0x02,
    ];

    #[test]
    fn extract_airborne_cpr_matches_known_bit_layout_odd() {
        let msg = [0u8, 0, 0, 0, 0, 0, 0x06, 0xCC, 0x6F, 0x55, 0xDA, 0, 0, 0];
        let frame = extract_airborne_cpr(&msg);
        assert!(frame.odd);
        assert_eq!(frame.lat, 91703);
        assert_eq!(frame.lon, 87514);
    }

    #[test]
    fn extract_airborne_cpr_matches_known_bit_layout_even() {
        // Round-trip of dump1090::cpr's own vetted even-frame example (93000, 113609).
        let msg = [0u8, 0, 0, 0, 0, 0, 0x02, 0xD6, 0x91, 0xBB, 0xC9, 0, 0, 0];
        let frame = extract_airborne_cpr(&msg);
        assert!(!frame.odd);
        assert_eq!(frame.lat, 93000);
        assert_eq!(frame.lon, 113609);
    }

    #[test]
    fn merge_message_extracts_callsign() {
        let (_bus, mut pipeline) = test_pipeline();
        let mm = ModesMessage {
            msg: VALID_DF17_CALLSIGN,
            verbatim: VALID_DF17_CALLSIGN,
            msgbits: 112,
            msgtype: 17,
            addr: 0x4840D6,
            sys_timestamp_msg: 500,
            ..ModesMessage::default()
        };
        pipeline.merge_message(&mm);
        let entry = pipeline.aircraft.get(&0x4840D6).expect("entry created");
        assert_eq!(entry.callsign.as_deref(), Some("KLM 63EW"));
    }

    #[test]
    fn merge_message_extracts_altitude() {
        let (_bus, mut pipeline) = test_pipeline();
        let icao = 0x555555;
        // Q-bit set, matching dump1090::mode_s's own decode_altitude test expectation of 450 ft.
        let mm = synthetic_df17(icao, [11 << 3, 0x10, 0, 0, 0, 0, 0]);
        pipeline.merge_message(&mm);
        let entry = pipeline.aircraft.get(&icao).expect("entry created");
        assert_eq!(entry.altitude_ft, Some(450));
    }

    #[test]
    fn merge_message_extracts_velocity() {
        let (_bus, mut pipeline) = test_pipeline();
        let icao = 0x444444;
        let mm = synthetic_df17(icao, [19 << 3, 0x01, 0, 0, 0, 0, 0]);
        pipeline.merge_message(&mm);
        let entry = pipeline.aircraft.get(&icao).expect("entry created");
        assert!(entry.ground_speed_kt.is_some());
        let track = entry.track_deg.expect("heading set");
        assert!((0.0..360.0).contains(&track));
    }

    #[test]
    fn merge_message_pairs_even_odd_cpr_frames_into_a_known_position() {
        // Raw lat/lon values are dump1090::cpr's own vetted airborne example
        // (even 93000/113609, odd 74158/108994 -> lat~=52.2572, lon~=8.6676), re-encoded
        // into ME-field bytes via extract_airborne_cpr's inverse. CprDecoder::submit
        // treats whichever frame arrives second as "latest" and the vetted example pins
        // the even frame as latest, so the odd frame must be submitted first here.
        let icao = 0x3C4B12;
        let even = synthetic_df17(icao, [0x58, 0x00, 0x02, 0xD6, 0x91, 0xBB, 0xC9]);
        let odd = synthetic_df17(icao, [0x58, 0x00, 0x06, 0x43, 0x5D, 0xA9, 0xC2]);
        let (_bus, mut pipeline) = test_pipeline();

        pipeline.merge_message(&odd);
        let entry = pipeline.aircraft.get(&icao).expect("entry created");
        assert!(entry.lat.is_none(), "a single frame must not produce a fix");

        pipeline.merge_message(&even);
        let entry = pipeline.aircraft.get(&icao).expect("entry still present");
        let lat = entry.lat.expect("even+odd pair should decode a position");
        let lon = entry.lon.expect("even+odd pair should decode a position");
        assert!((lat - 52.2572).abs() < 0.001);
        assert!((lon - 8.6676).abs() < 0.001);
    }

    #[test]
    fn merge_message_rejects_bad_crc_without_creating_an_entry() {
        let (_bus, mut pipeline) = test_pipeline();
        let mut bad = VALID_DF17_CALLSIGN;
        bad[2] ^= 0x01;
        let mm = ModesMessage {
            msg: bad,
            verbatim: bad,
            msgbits: 112,
            msgtype: 17,
            addr: 0x4840D6,
            sys_timestamp_msg: 500,
            ..ModesMessage::default()
        };
        pipeline.merge_message(&mm);
        assert!(
            pipeline.aircraft.is_empty(),
            "bad-CRC message must not produce a semantic entry"
        );
        assert_eq!(
            pipeline.tracked_count(),
            1,
            "tracker still counts every framed message"
        );
    }

    #[test]
    fn msg_count_reflects_total_messages_seen_for_an_aircraft() {
        let (_bus, mut pipeline) = test_pipeline();
        let icao = 0x666666;
        for _ in 0..3 {
            let mut mm = synthetic_df17(icao, [19 << 3, 0x01, 0, 0, 0, 0, 0]);
            mm.sys_timestamp_msg = now_ms();
            pipeline.merge_message(&mm);
        }
        pipeline.prune_and_publish();
        assert_eq!(pipeline.aircraft[&icao].msg_count, 3);
    }

    #[test]
    fn prune_and_publish_drops_stale_aircraft_and_syncs_state() {
        let (_bus, mut pipeline) = test_pipeline();
        let stale_icao = 0x111111;
        let fresh_icao = 0x222222;

        let mut stale = synthetic_df17(stale_icao, [19 << 3, 0x01, 0, 0, 0, 0, 0]);
        stale.sys_timestamp_msg = 1_000; // ancient relative to real now_ms()
        pipeline.merge_message(&stale);

        let mut fresh = synthetic_df17(fresh_icao, [19 << 3, 0x01, 0, 0, 0, 0, 0]);
        fresh.sys_timestamp_msg = now_ms();
        pipeline.merge_message(&fresh);

        assert_eq!(pipeline.aircraft.len(), 2);

        pipeline.prune_and_publish();

        assert!(
            !pipeline.aircraft.contains_key(&stale_icao),
            "stale aircraft should be pruned"
        );
        assert!(
            pipeline.aircraft.contains_key(&fresh_icao),
            "fresh aircraft should remain"
        );
        assert_eq!(pipeline.aircraft[&fresh_icao].msg_count, 1);
    }

    #[test]
    fn prune_and_publish_publishes_a_snapshot() {
        let (_bus, mut pipeline) = test_pipeline();
        let out = pipeline.subscribe(4);
        let mut mm = synthetic_df17(0x333333, [19 << 3, 0x01, 0, 0, 0, 0, 0]);
        mm.sys_timestamp_msg = now_ms();
        pipeline.merge_message(&mm);
        pipeline.prune_and_publish();
        let snapshot = out.try_recv().expect("expected a published snapshot");
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].icao, 0x333333);
    }

    #[test]
    fn process_block_on_silence_produces_no_aircraft() {
        let (bus, mut pipeline) = test_pipeline();
        bus.publish(silence_block(4096));
        assert!(pipeline.tick(Duration::from_millis(200)));
        assert!(pipeline.aircraft.is_empty());
        assert_eq!(pipeline.tracked_count(), 0);
    }

    #[test]
    fn publishes_a_snapshot_every_n_blocks() {
        let (bus, mut pipeline) = test_pipeline();
        let out = pipeline.subscribe(4);
        for _ in 0..PUBLISH_EVERY_N_BLOCKS {
            bus.publish(silence_block(256));
            pipeline.tick(Duration::from_millis(200));
        }
        assert!(
            out.try_recv().is_some(),
            "expected a snapshot after N blocks"
        );
    }

    #[test]
    fn tick_times_out_cleanly_when_idle() {
        let (_bus, mut pipeline) = test_pipeline();
        assert!(!pipeline.tick(Duration::from_millis(20)));
    }

    #[test]
    fn run_exits_promptly_when_running_flag_clears() {
        let (_bus, mut pipeline) = test_pipeline();
        let running = AtomicBool::new(false);
        pipeline.run(&running);
    }

    #[test]
    fn subscriber_count_reflects_subscriptions() {
        let (_bus, pipeline) = test_pipeline();
        assert_eq!(pipeline.subscriber_count(), 0);
        let handle = pipeline.subscribe(4);
        assert_eq!(pipeline.subscriber_count(), 1);
        drop(handle);
        assert_eq!(pipeline.subscriber_count(), 0);
    }
}
