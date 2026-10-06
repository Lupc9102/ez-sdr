//! Event-driven ADS-B/Mode-S packet decoding: turns a stream of [`SampleBlock`]s into
//! per-aircraft [`AircraftTelemetry`] snapshots via `dump1090`'s demodulator/decoder.
//!
//! Unlike [`crate::pipelines::spectrum::SpectrumPipeline`]/[`crate::pipelines::audio::AudioPipeline`]
//! (continuous per-sample DSP producing output at a fixed cadence), this pipeline is
//! event-driven and bursty: most blocks decode to zero messages. A snapshot of every
//! currently-tracked aircraft is republished periodically, including while input is
//! idle, so position and track expiry never depends on receiving another block.
//!
//! `dump1090::track::Tracker` is kept as the single source of truth for per-ICAO liveness
//! and total message count (it sees every framed message, decodable or not); this
//! pipeline's own `aircraft` map only carries the semantic ADS-B fields (callsign,
//! altitude, position, velocity) that `Tracker` doesn't. The two are synced at publish
//! time so a client never sees a `msg_count` that undercounts CRC-valid-but-not-yet-fully-
//! decoded traffic.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use dump1090::cpr::{CprDecoder, CprFrame, CprTimestamp, CprType};
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
    /// Tail copied from the previous block so Mode S messages spanning a
    /// channelizer block boundary remain decodable.
    overlap_tail: Vec<u16>,
    continuity: crate::bus::SampleContinuity,
    aircraft: HashMap<u32, AircraftTelemetry>,
    capture_seen: HashMap<u32, ez_proto::CaptureTimestamp>,
    capture_position_seen: HashMap<u32, ez_proto::CaptureTimestamp>,
    capture_now: Option<CprTimestamp>,
    capture_observed_at: Option<Instant>,
    last_capture_clock: Option<ez_proto::CaptureClock>,
    blocks_since_publish: u32,
    last_publish: Instant,
    receiver_position: Option<(f64, f64)>,
    rate_window: ez_proto::MessageRateWindow,
    output: Broadcaster<ez_proto::AircraftSnapshot>,
}

impl PacketPipeline {
    #[must_use]
    pub fn new(input: SampleBusHandle, sample_rate_hz: u32) -> Self {
        if sample_rate_hz != 2_400_000 {
            tracing::error!(
                "PacketPipeline expects 2.4 MHz input, got {} Hz",
                sample_rate_hz
            );
        }
        Self {
            input,
            demod: Demod2400::new(),
            stats: DemodStats::default(),
            tracker: Tracker::new(),
            cpr: CprDecoder::new(),
            mag_buf: Vec::new(),
            overlap_tail: Vec::new(),
            continuity: crate::bus::SampleContinuity::default(),
            aircraft: HashMap::new(),
            capture_seen: HashMap::new(),
            capture_position_seen: HashMap::new(),
            capture_now: None,
            capture_observed_at: None,
            last_capture_clock: None,
            blocks_since_publish: 0,
            last_publish: Instant::now(),
            receiver_position: None,
            rate_window: Default::default(),
            output: Broadcaster::new(),
        }
    }

    /// Subscribes to published aircraft snapshots. Each value is a full replacement of
    /// every currently-tracked aircraft, so a slow subscriber should only ever want the
    /// newest one — hence [`OverflowPolicy::DropOldest`].
    #[must_use]
    pub fn subscribe(&self, capacity: usize) -> BroadcasterHandle<ez_proto::AircraftSnapshot> {
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
        let consumed = match self.input.recv_timeout(poll_timeout) {
            Some(block) => {
                self.process_block(&block);
                true
            }
            None => false,
        };
        if self.last_publish.elapsed() >= Duration::from_secs(1) {
            self.prune_and_publish();
        }
        consumed
    }

    /// Actual capture-site coordinates; absent unless explicitly configured.
    /// In particular, a remote viewer's map location is not a receiver reference.
    pub fn set_receiver_position(&mut self, position: Option<(f64, f64)>) {
        self.receiver_position = position.filter(|&(lat, lon)| {
            lat.is_finite()
                && lon.is_finite()
                && (-90.0..=90.0).contains(&lat)
                && (-180.0..=180.0).contains(&lon)
        });
        self.cpr = CprDecoder::new();
    }

    pub fn run(&mut self, running: &AtomicBool) {
        const POLL_INTERVAL: Duration = Duration::from_millis(250);
        while running.load(Ordering::Relaxed) {
            self.tick(POLL_INTERVAL);
        }
    }

    fn receiver_snapshot(&self, now: Instant) -> ez_proto::ReceiverSnapshot {
        let stats = &self.stats;
        let average = |sum: f64, count: u64| if count > 0 { sum / count as f64 } else { 0.0 };
        ez_proto::ReceiverSnapshot {
            timestamp_ms: now_ms(),
            total_messages: stats.demod_accepted.iter().sum(),
            messages_per_second: self.rate_window.rate_at(now),
            decode_stats: (
                stats.demod_preambles,
                stats.demod_accepted.iter().sum(),
                stats.demod_rejected_bad + stats.demod_rejected_unknown_icao,
                stats.demod_modeac,
                stats.demod_accepted[0],
                stats.demod_accepted[1],
                stats.demod_accepted[2],
                average(stats.signal_power_sum, stats.signal_power_count),
                average(stats.noise_power_sum, stats.noise_power_count),
                stats.peak_signal_power,
                stats.strong_signal_count,
            ),
        }
    }

    fn process_block(&mut self, block: &SampleBlock) {
        let mut sum_level = 0.0_f64;
        let mut sum_power = 0.0_f64;
        let mut data = std::mem::take(&mut self.mag_buf);
        data.clear();
        let clock = block.capture_clock();
        let contiguous = self.continuity.observe(block);
        if !contiguous {
            self.overlap_tail.clear();
            self.cpr.clear();
            let changed_epoch = self.last_capture_clock.is_some_and(|previous| {
                previous.epoch != clock.epoch
                    || previous.source_sample_rate_hz != clock.source_sample_rate_hz
                    || previous.sample_stride != clock.sample_stride
                    || previous.origin_unix_ns != clock.origin_unix_ns
            });
            if changed_epoch {
                self.demod.icao_filter = Default::default();
                self.tracker = Tracker::new();
                self.aircraft.clear();
                self.capture_seen.clear();
                self.capture_position_seen.clear();
            } else {
                for aircraft in self.aircraft.values_mut() {
                    aircraft.lat = None;
                    aircraft.lon = None;
                    aircraft.position_seen_ms = None;
                    aircraft.position_source = None;
                    aircraft.position_quality = None;
                }
                self.capture_position_seen.clear();
            }
        }
        self.last_capture_clock = Some(clock);
        self.capture_observed_at = Some(Instant::now());
        self.capture_now = clock
            .message_timestamp_2400(block.samples.len() as i128 * 5)
            .and_then(capture_cpr_time);
        let overlap = self.overlap_tail.len();
        data.reserve(overlap + block.samples.len());
        data.extend_from_slice(&self.overlap_tail);
        for sample in block.samples.iter() {
            // All daemon sources expose normalized full-scale components.
            let norm = f64::from(sample.norm()).min(1.0);
            sum_level += norm;
            sum_power += norm * norm;
            data.push((norm * 65535.0).min(65535.0) as u16);
        }
        let n = (block.samples.len().max(1)) as f64;

        let mag = MagBuf {
            data: data.into(),
            total_length: overlap + block.samples.len(),
            valid_length: overlap + block.samples.len(),
            overlap,
            sample_timestamp: block.start_sample.saturating_sub(overlap as u64) * 5,
            sys_timestamp: now_ms(),
            flags: MagBufFlags(if contiguous { 0 } else { MAGBUF_DISCONTINUOUS }),
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

        const OVERLAP_SAMPLES: usize = 320;
        let keep = mag.data.len().min(OVERLAP_SAMPLES);
        self.overlap_tail.clear();
        self.overlap_tail
            .extend_from_slice(&mag.data[mag.data.len() - keep..]);

        self.mag_buf = mag.data.into_owned();

        self.rate_window
            .observe_total(self.stats.demod_accepted.iter().sum(), Instant::now());

        for mm in &messages {
            let Some(capture) = clock
                .message_timestamp_2400(
                    i128::from(mm.timestamp_msg) - i128::from(block.start_sample.saturating_mul(5)),
                )
                .filter(|capture| capture.relative_time_ns().is_some())
            else {
                continue;
            };
            self.merge_message_at(mm, Some(capture));
        }

        self.blocks_since_publish += 1;
        if self.blocks_since_publish >= PUBLISH_EVERY_N_BLOCKS {
            self.blocks_since_publish = 0;
            self.prune_and_publish();
        }
    }

    #[cfg(test)]
    fn merge_message(&mut self, mm: &ModesMessage) {
        self.capture_now = None;
        self.capture_observed_at = None;
        self.merge_message_at(mm, None);
    }

    fn merge_message_at(&mut self, mm: &ModesMessage, capture: Option<ez_proto::CaptureTimestamp>) {
        let cpr_time = capture
            .and_then(capture_cpr_time)
            .unwrap_or(CprTimestamp::Processing(Instant::now()));
        let decoded = mode_s::decode_mode_s_message(&mm.msg);
        // Public/test callers may construct ModesMessage directly. Keep the
        // tracker keyed by the same validated namespace as the semantic fields.
        let mut tracked = mm.clone();
        if let Some(decoded) = &decoded {
            tracked.addr = decoded.icao;
        }
        // Internal capture state survives an idle display expiry, but is bounded
        // even when a replay is paused or fed a dense burst of distinct addresses.
        if !self.tracker.aircraft.contains_key(&tracked.addr) && self.tracker.len() >= 4096 {
            if let Some(oldest) = self
                .tracker
                .aircraft
                .iter()
                .max_by_key(|(icao, state)| {
                    self.capture_seen
                        .get(icao)
                        .copied()
                        .and_then(capture_cpr_time)
                        .and_then(|seen| cpr_time.checked_duration_since(seen))
                        .unwrap_or_else(|| {
                            Duration::from_millis(
                                mm.sys_timestamp_msg.saturating_sub(state.last_seen_ms),
                            )
                        })
                })
                .map(|(&icao, _)| icao)
            {
                self.tracker.aircraft.remove(&oldest);
                self.aircraft.remove(&oldest);
                self.capture_seen.remove(&oldest);
                self.capture_position_seen.remove(&oldest);
            }
        }
        self.tracker.update_from_message(&tracked);
        if let Some(capture) = capture {
            self.capture_seen.insert(tracked.addr, capture);
        } else {
            self.capture_seen.remove(&tracked.addr);
        }
        let Some(decoded) = decoded else {
            return;
        };

        let prior_position = self
            .aircraft
            .get(&decoded.icao)
            .filter(|entry| {
                entry.position_seen_ms.is_some_and(|seen| {
                    self.capture_position_seen
                        .get(&decoded.icao)
                        .copied()
                        .and_then(capture_cpr_time)
                        .and_then(|position| cpr_time.checked_duration_since(position))
                        .map_or(
                            mm.sys_timestamp_msg.saturating_sub(seen) <= STALE_AIRCRAFT_MS,
                            |age| age <= Duration::from_millis(STALE_AIRCRAFT_MS),
                        )
                })
            })
            .and_then(|entry| entry.lat.zip(entry.lon));

        let cpr_position = if matches!(mm.msgtype, 17 | 18) && check_crc(&mm.msg) {
            let tc = mm.msg[4] >> 3;
            if (9..=18).contains(&tc) || (20..=22).contains(&tc) {
                self.cpr.submit_with_timestamp(
                    decoded.icao,
                    extract_airborne_cpr(&mm.msg),
                    cpr_time,
                )
            } else if (5..=8).contains(&tc) {
                prior_position
                    .or(self.receiver_position)
                    .and_then(|(lat, lon)| {
                        self.cpr
                            .submit_surface_with_timestamp(
                                decoded.icao,
                                extract_surface_cpr(&mm.msg),
                                lat,
                                lon,
                                cpr_time,
                            )
                            .or_else(|| self.cpr.decode_surface_relative(decoded.icao, lat, lon))
                            .filter(|&(decoded_lat, decoded_lon)| {
                                dump1090::cpr::surface_within_reference(
                                    decoded_lat,
                                    decoded_lon,
                                    lat,
                                    lon,
                                )
                            })
                    })
            } else {
                None
            }
        } else {
            None
        };

        let entry = self
            .aircraft
            .entry(decoded.icao)
            .or_insert_with(|| AircraftTelemetry {
                icao: decoded.icao,
                ..Default::default()
            });
        entry.address_type = decoded.address_type.into();
        entry.source = decoded.source.into();

        if let Some(callsign) = decoded.callsign {
            entry.callsign = Some(callsign);
        }
        if let Some(ground) = decoded.on_ground {
            if entry.on_ground.is_some_and(|prior| prior != ground) {
                entry.altitude_ft = None;
                entry.ground_speed_kt = None;
                entry.track_deg = None;
                entry.heading_deg = None;
                entry.airspeed_kt = None;
                entry.surface_direction_deg = None;
            }
            entry.on_ground = Some(ground);
        }
        let tc = if matches!(decoded.df, 17 | 18) {
            mm.msg[4] >> 3
        } else {
            0
        };
        if matches!(tc, 9..=18 | 20..=22) || matches!(decoded.df, 0 | 4 | 16 | 20) {
            entry.altitude_ft = decoded.altitude;
            entry.altitude_geometric = (20..=22).contains(&tc);
        }
        if tc == 19 {
            match mm.msg[4] & 7 {
                1 | 2 => {
                    entry.ground_speed_kt = decoded.velocity.map(|v| v.0);
                    entry.track_deg = decoded.velocity.map(|v| v.1);
                }
                3 | 4 => {
                    entry.heading_deg = decoded.magnetic_heading;
                    entry.heading_true = false;
                    entry.airspeed_kt = decoded.airspeed;
                    entry.airspeed_true = decoded.airspeed_true;
                }
                _ => {}
            }
            entry.vertical_rate_fpm = decoded.vertical_rate;
        }
        if (5..=8).contains(&tc) {
            entry.altitude_ft = None;
            entry.track_deg = None;
            entry.heading_deg = None;
            entry.airspeed_kt = None;
            entry.vertical_rate_fpm = None;
            entry.ground_speed_kt = decoded.surface_movement;
            entry.surface_direction_deg = decoded.surface_direction;
        }
        if let Some(emergency) = decoded.emergency {
            entry.emergency = Some(emergency);
        }
        if let Some(squawk) = decoded.squawk {
            entry.squawk = Some(squawk);
        }

        // decode_mode_s_message already CRC-gates its own field extraction, but CPR
        // extraction is a separate code path this pipeline owns independently — feeding a
        // corrupted frame into the decoder risks poisoning a legitimate even/odd pairing
        // for this ICAO, so it gets its own explicit CRC guard.
        if let Some((lat, lon)) = cpr_position {
            entry.lat = Some(lat);
            entry.lon = Some(lon);
            entry.position_seen_ms = Some(mm.sys_timestamp_msg);
            if let Some(capture) = capture {
                self.capture_position_seen.insert(decoded.icao, capture);
            } else {
                self.capture_position_seen.remove(&decoded.icao);
            }
            entry.position_source = Some(decoded.source.into());
            entry.position_quality = Some(ez_proto::AircraftQuality {
                reliable: mm.reliable,
                corrected_bits: mm.correctedbits.min(2) as u8,
                score: mm.score.quality(),
            });
        }
    }

    fn prune_and_publish(&mut self) {
        self.prune_and_publish_at(now_ms());
    }

    fn prune_and_publish_at(&mut self, timestamp_ms: u64) {
        self.last_publish = Instant::now();
        self.blocks_since_publish = 0;
        let cutoff_ms = timestamp_ms.saturating_sub(STALE_AIRCRAFT_MS);
        let capture_age = |capture: Option<ez_proto::CaptureTimestamp>| {
            capture
                .and_then(capture_cpr_time)
                .zip(self.capture_now)
                .and_then(|(seen, current)| current.checked_duration_since(seen))
        };
        self.tracker.aircraft.retain(|icao, state| {
            capture_age(self.capture_seen.get(icao).copied())
                .map_or(state.last_seen_ms >= cutoff_ms, |age| {
                    age <= Duration::from_millis(STALE_AIRCRAFT_MS)
                })
        });

        let tracker = &self.tracker;
        self.aircraft
            .retain(|icao, entry| match tracker.aircraft.get(icao) {
                Some(state) => {
                    entry.msg_count = state.msg_count;
                    entry.last_seen_ms = state.last_seen_ms;
                    let position_age = capture_age(self.capture_position_seen.get(icao).copied());
                    if position_age.map_or(
                        entry.position_seen_ms.is_none_or(|seen| seen < cutoff_ms),
                        |age| age > Duration::from_millis(STALE_AIRCRAFT_MS),
                    ) {
                        entry.lat = None;
                        entry.lon = None;
                    }
                    true
                }
                None => false,
            });
        self.capture_seen
            .retain(|icao, _| tracker.aircraft.contains_key(icao));
        self.capture_position_seen
            .retain(|icao, _| tracker.aircraft.contains_key(icao));

        // Display liveness ages while no input arrives. The retained capture
        // state uses sample time alone, so a delayed replay block can still pair
        // with its recorded predecessor. Never invent acquisition UTC.
        let idle_age = self
            .capture_observed_at
            .map_or(Duration::ZERO, |seen| seen.elapsed());
        let snapshot: Vec<AircraftTelemetry> = self
            .aircraft
            .iter()
            .filter_map(|(icao, entry)| {
                let mut published = entry.clone();
                if let Some(age) = capture_age(self.capture_seen.get(icao).copied()) {
                    let age = age.saturating_add(idle_age);
                    if age > Duration::from_millis(STALE_AIRCRAFT_MS) {
                        return None;
                    }
                    published.last_seen_ms = timestamp_ms.saturating_sub(age.as_millis() as u64);
                }
                if let Some(age) = capture_age(self.capture_position_seen.get(icao).copied()) {
                    let age = age.saturating_add(idle_age);
                    published.position_seen_ms =
                        Some(timestamp_ms.saturating_sub(age.as_millis() as u64));
                    if age > Duration::from_millis(STALE_AIRCRAFT_MS) {
                        published.lat = None;
                        published.lon = None;
                    }
                }
                Some(published)
            })
            .collect();
        self.output.publish(ez_proto::AircraftSnapshot {
            channel_id: 0,
            aircraft: snapshot,
            receiver: self.receiver_snapshot(Instant::now()),
        });
    }
}

fn capture_cpr_time(capture: ez_proto::CaptureTimestamp) -> Option<CprTimestamp> {
    Some(CprTimestamp::Capture {
        epoch: capture.epoch,
        elapsed: Duration::from_nanos(capture.relative_time_ns()?),
    })
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

fn extract_surface_cpr(msg: &[u8; 14]) -> CprFrame {
    let odd = (msg[6] & 0x04) != 0;
    let lat =
        (u32::from(msg[6] & 0x03) << 15) | (u32::from(msg[7]) << 7) | (u32::from(msg[8]) >> 1);
    let lon = (u32::from(msg[8] & 0x01) << 16) | (u32::from(msg[9]) << 8) | u32::from(msg[10]);
    CprFrame {
        cpr_type: CprType::Surface,
        odd,
        lat,
        lon,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SampleBus;
    use crate::channelizer::Channelizer;
    use crate::hardware::adsb::AdsbEmulatorSource;
    use crate::hardware::IqSource;
    use num_complex::Complex32;

    fn test_pipeline() -> (SampleBus, PacketPipeline) {
        let bus = SampleBus::new();
        let handle = bus.subscribe(8, OverflowPolicy::DropIncoming);
        (bus, PacketPipeline::new(handle, 2_400_000))
    }

    #[test]
    fn receiver_snapshot_counts_real_messages_and_expires_on_idle() {
        use crate::hardware::adsb::{encode_uc8, EXAMPLE_DF17_CALLSIGN};
        let (_, mut pipeline) = test_pipeline();
        let output = pipeline.subscribe(8);
        let raw = encode_uc8(&[EXAMPLE_DF17_CALLSIGN; 100]);
        pipeline.process_block(&SampleBlock {
            capture: None,
            start_sample: 0,
            sample_rate_hz: 2_400_000,
            center_freq_hz: 1_090_000_000,
            samples: std::sync::Arc::from(lrpt_decode::uc8_bytes_to_normalized(&raw)),
        });
        pipeline.prune_and_publish();
        let snapshot = output.try_recv().unwrap();
        assert_eq!(snapshot.aircraft.len(), 1);
        assert_eq!(snapshot.receiver.total_messages, 100);
        assert_eq!(snapshot.receiver.decode_stats.1, 100);
        assert_eq!(snapshot.receiver.messages_per_second, 20.0);
        let idle = pipeline.receiver_snapshot(Instant::now() + Duration::from_secs(6));
        assert_eq!(idle.total_messages, 100);
        assert_eq!(idle.messages_per_second, 0.0);
    }

    #[test]
    fn signed_altitude_and_velocity_semantics_survive_daemon_merge() {
        let (_, mut pipeline) = test_pipeline();
        let icao = 1;
        for altitude in [-1000, -500, 0, 35000] {
            let n = ((altitude + 1000) / 25) as u32;
            let ac12 = ((n & 0x7f0) << 1) | 0x10 | (n & 15);
            pipeline.merge_message(&synthetic_df17(
                icao,
                [11 << 3, (ac12 >> 4) as u8, (ac12 << 4) as u8, 0, 0, 0, 0],
            ));
            assert_eq!(pipeline.aircraft[&icao].altitude_ft, Some(altitude));
        }
        for subtype in 1..=4 {
            let payload = if subtype <= 2 {
                [(19 << 3) | subtype, 0, 101, 0, 0x20, 0, 0]
            } else {
                [
                    (19 << 3) | subtype,
                    5,
                    0,
                    12 | if subtype == 4 { 0x80 } else { 0 },
                    0xa0,
                    0,
                    0,
                ]
            };
            pipeline.merge_message(&synthetic_df17(icao, payload));
            let ac = &pipeline.aircraft[&icao];
            assert_eq!(ac.on_ground, Some(false));
            assert_eq!(
                ac.ground_speed_kt,
                Some(if subtype == 1 { 100.0 } else { 400.0 })
            );
            assert_eq!(ac.track_deg, Some(90.0));
            if subtype <= 2 {
                assert_eq!(ac.heading_deg, None);
                assert_eq!(ac.airspeed_kt, None);
            } else {
                assert_eq!(ac.heading_deg, Some(90.0));
                assert!(!ac.heading_true);
                assert_eq!(
                    ac.airspeed_kt,
                    Some(if subtype == 3 { 100.0 } else { 400.0 })
                );
                assert_eq!(ac.airspeed_true, subtype == 4);
            }
        }
        pipeline.merge_message(&synthetic_df17(icao, [5 << 3, 0x1c, 0, 0, 0, 0, 0]));
        let ac = &pipeline.aircraft[&icao];
        assert_eq!(ac.on_ground, Some(true));
        assert_eq!(ac.ground_speed_kt, Some(0.0));
        assert_eq!(ac.surface_direction_deg, Some(180.0));
        assert_eq!(ac.heading_deg, None);
        assert_eq!(ac.track_deg, None);
        assert_eq!(ac.airspeed_kt, None);
        assert_eq!(ac.altitude_ft, None);
        pipeline.merge_message(&synthetic_df17(icao, [(5 << 3) | 7, 0xd0, 0, 0, 0, 0, 0]));
        assert_eq!(pipeline.aircraft[&icao].ground_speed_kt, None);
        assert_eq!(pipeline.aircraft[&icao].surface_direction_deg, None);
        pipeline.merge_message(&synthetic_df17(icao, [(19 << 3) | 3, 0, 0, 0, 0, 0, 0]));
        let ac = &pipeline.aircraft[&icao];
        assert_eq!(ac.on_ground, Some(false));
        assert_eq!(ac.surface_direction_deg, None);
        assert_eq!(ac.heading_deg, None);
        assert_eq!(ac.airspeed_kt, None);
    }

    #[test]
    fn stale_position_expires_during_other_traffic_and_idle_tracks_publish_expiry() {
        let (bus, mut pipeline) = test_pipeline();
        let output = pipeline.subscribe(8);
        let now = now_ms();
        for payload in [
            [0x58, 0, 0x02, 0xd6, 0x91, 0xbb, 0xc9],
            [0x58, 0, 0x06, 0x43, 0x5d, 0xa9, 0xc2],
        ] {
            let mut frame = synthetic_df17(1, payload);
            frame.sys_timestamp_msg = now - 61_000;
            pipeline.merge_message(&frame);
        }
        assert!(pipeline.aircraft[&1].lat.is_some());
        let mut other = synthetic_df17(1, [0x20, 0x2c, 0xc3, 0x7c, 0xdb, 0x31, 0x57]);
        other.sys_timestamp_msg = now;
        pipeline.merge_message(&other);
        assert!(pipeline.aircraft[&1].lat.is_some());
        let mut surface = synthetic_df17(1, [5 << 3, 0, 0, 0, 0, 0, 0]);
        surface.sys_timestamp_msg = now;
        pipeline.merge_message(&surface);
        assert_eq!(pipeline.aircraft[&1].position_seen_ms, Some(now - 61_000)); // stale reference cannot refresh
        pipeline.prune_and_publish_at(now);
        let snapshot = output
            .recv_timeout(Duration::from_millis(10))
            .unwrap()
            .aircraft;
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].lat, None);
        assert_eq!(snapshot[0].lon, None);
        assert_eq!(snapshot[0].last_seen_ms, now);
        pipeline.tracker.aircraft.get_mut(&1).unwrap().last_seen_ms = now - 61_000;
        pipeline.last_publish = Instant::now() - Duration::from_secs(2);
        assert!(!pipeline.tick(Duration::from_millis(1))); // no incoming samples
        assert!(output
            .recv_timeout(Duration::from_millis(10))
            .unwrap()
            .aircraft
            .is_empty());
        assert_eq!(pipeline.tracked_count(), 0);
        drop(bus);
    }

    #[test]
    fn geometric_cpr_and_initial_surface_reference_match_desktop_policy() {
        for tc in 20..=22 {
            let (_, mut pipeline) = test_pipeline();
            pipeline.merge_message(&synthetic_df17(
                1,
                [tc << 3, 0, 0x02, 0xd6, 0x91, 0xbb, 0xc9],
            ));
            pipeline.merge_message(&synthetic_df17(
                1,
                [tc << 3, 0, 0x06, 0x43, 0x5d, 0xa9, 0xc2],
            ));
            assert!((pipeline.aircraft[&1].lat.unwrap() - 52.2658).abs() < 0.001);
            assert!((pipeline.aircraft[&1].lon.unwrap() - 8.5522).abs() < 0.001);
        }
        let (_, mut pipeline) = test_pipeline();
        let lat: f64 = 51.51;
        let lon: f64 = -0.11;
        let raw_lat = (lat.rem_euclid(1.5) / 1.5 * 131072.0).round() as u32;
        let dlon = 90.0 / 37.0;
        let raw_lon = (lon.rem_euclid(dlon) / dlon * 131072.0).round() as u32;
        let surface = synthetic_df17(
            1,
            [
                5 << 3,
                0,
                (raw_lat >> 15) as u8,
                (raw_lat >> 7) as u8,
                (raw_lat << 1) as u8 | (raw_lon >> 16) as u8,
                (raw_lon >> 8) as u8,
                raw_lon as u8,
            ],
        );
        pipeline.merge_message(&surface);
        assert!(pipeline.aircraft[&1].lat.is_none());
        pipeline.set_receiver_position(Some((51.5, -0.1)));
        pipeline.merge_message(&surface);
        assert!((pipeline.aircraft[&1].lat.unwrap() - lat).abs() < 0.001);
        assert!((pipeline.aircraft[&1].lon.unwrap() - lon).abs() < 0.001);
        pipeline.set_receiver_position(Some((f64::NAN, 0.0)));
        assert!(pipeline.receiver_position.is_none());
        assert!(!dump1090::cpr::surface_within_reference(
            53.0, 2.0, 51.5, -0.1
        ));
    }

    fn silence_block(len: usize) -> SampleBlock {
        SampleBlock {
            capture: None,
            start_sample: 0,
            sample_rate_hz: 2_400_000,
            center_freq_hz: 1_090_000_000,
            samples: std::sync::Arc::from(vec![Complex32::new(0.0, 0.0); len]),
        }
    }

    #[test]
    fn rtl_sdr_style_adsb_iq_reaches_packet_pipeline() {
        let (bus, mut pipeline) = test_pipeline();
        let mut source = AdsbEmulatorSource::example(false);
        source.start().unwrap();
        let mut samples = vec![Complex32::new(0.0, 0.0); 600];
        let n = source.read_iq(&mut samples).unwrap();
        assert!(n > 300);
        samples.truncate(n);
        bus.publish(SampleBlock {
            capture: None,
            start_sample: 0,
            sample_rate_hz: 2_400_000,
            center_freq_hz: 1_090_000_000,
            samples: std::sync::Arc::from(samples),
        });
        assert!(pipeline.tick(Duration::from_millis(10)));
        assert_eq!(pipeline.tracked_count(), 1);
    }

    #[test]
    fn upstream_missing_input_cannot_form_a_packet_across_the_channelizer() {
        use crate::hardware::adsb::{encode_uc8, EXAMPLE_DF17_CALLSIGN};
        for gap in [0, 1, 1000] {
            let wideband = SampleBus::new();
            let mut channelizer = Channelizer::new(
                wideband.subscribe(8, OverflowPolicy::DropIncoming),
                1_090_000_000,
                2_400_000,
            );
            let (_, input) = channelizer.add_channel(1_090_000_000, 2_400_000).unwrap();
            let mut decoder = PacketPipeline::new(input, 2_400_000);
            let samples =
                lrpt_decode::uc8_bytes_to_normalized(&encode_uc8(&[EXAMPLE_DF17_CALLSIGN]));
            let cut = samples.len() / 2;
            for (start, samples) in [(0, &samples[..cut]), (cut as u64 + gap, &samples[cut..])] {
                wideband.publish(SampleBlock {
                    capture: None,
                    start_sample: start,
                    sample_rate_hz: 2_400_000,
                    center_freq_hz: 1_090_000_000,
                    samples: std::sync::Arc::from(samples),
                });
                assert!(channelizer.tick(Duration::from_millis(10)));
                assert!(decoder.tick(Duration::from_millis(10)));
            }
            assert_eq!(decoder.tracked_count(), usize::from(gap == 0), "gap={gap}");
        }
    }

    #[test]
    fn rtl_sdr_style_adsb_iq_survives_the_wideband_channelizer() {
        let wideband = SampleBus::new();
        let input = wideband.subscribe(8, OverflowPolicy::DropIncoming);
        let mut channelizer = Channelizer::new(input, 1_090_000_000, 2_400_000);
        let (_, packet_input) = channelizer
            .add_channel(1_090_000_000, 2_400_000)
            .expect("ADS-B channel should fit the 2.4 MSPS capture");
        let mut pipeline = PacketPipeline::new(packet_input, 2_400_000);

        let mut source = AdsbEmulatorSource::example(false);
        source.start().unwrap();
        let mut samples = vec![Complex32::new(0.0, 0.0); 600];
        let n = source.read_iq(&mut samples).unwrap();
        samples.truncate(n);
        wideband.publish(SampleBlock {
            capture: None,
            start_sample: 0,
            sample_rate_hz: 2_400_000,
            center_freq_hz: 1_090_000_000,
            samples: std::sync::Arc::from(samples),
        });

        assert!(channelizer.tick(Duration::from_millis(10)));
        assert!(pipeline.tick(Duration::from_millis(10)));
        assert_eq!(pipeline.tracked_count(), 1);
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
    fn df18_namespaces_survive_tracking_and_snapshot_publication() {
        let (_bus, mut pipeline) = test_pipeline();
        for cf in [None, Some(1), Some(5)] {
            let mut mm = synthetic_df17(0xABCDEF, [1 << 3, 0, 0, 0, 0, 0, 0]);
            if let Some(cf) = cf {
                mm.msg[0] = (18 << 3) | cf;
                mm.msgtype = 18;
                let parity = dump1090::crc::crc24(&mm.msg[..11]);
                mm.msg[11] = (parity >> 16) as u8;
                mm.msg[12] = (parity >> 8) as u8;
                mm.msg[13] = parity as u8;
            }
            mm.sys_timestamp_msg = now_ms();
            pipeline.merge_message(&mm);
        }
        let output = pipeline.subscribe(1);
        pipeline.prune_and_publish();
        let snapshot = output.try_recv().unwrap().aircraft;
        assert_eq!(snapshot.len(), 3);
        for key in [0xABCDEF, 0x10ABCDEF, 0x12ABCDEF] {
            assert!(snapshot
                .iter()
                .any(|entry| entry.icao == key && entry.msg_count == 1));
        }
        assert_eq!(pipeline.aircraft[&0x12ABCDEF].source, "tisb");
    }

    #[test]
    fn aircraft_status_preserves_vertical_rate_in_daemon() {
        let (_bus, mut pipeline) = test_pipeline();
        let velocity = synthetic_df17(1, [19 << 3 | 1, 0, 1, 0, 0x20, 0x08, 0x04]);
        pipeline.merge_message(&velocity);
        let rate = pipeline.aircraft[&1].vertical_rate_fpm;
        assert!(rate.is_some());
        for emergency in 0..=7 {
            pipeline.merge_message(&synthetic_df17(
                1,
                [28 << 3 | 1, emergency << 5 | 0x0a, 0xaa, 0, 0, 0, 0],
            ));
            let entry = &pipeline.aircraft[&1];
            assert_eq!(entry.vertical_rate_fpm, rate);
            assert_eq!(entry.emergency, Some(emergency));
            assert_eq!(entry.squawk, Some(7700));
        }
    }

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
        let mm = synthetic_df17(icao, [11 << 3, 0x07, 0xA0, 0, 0, 0, 0]);
        pipeline.merge_message(&mm);
        let entry = pipeline.aircraft.get(&icao).expect("entry created");
        assert_eq!(entry.altitude_ft, Some(450));
    }

    #[test]
    fn merge_message_extracts_velocity() {
        let (_bus, mut pipeline) = test_pipeline();
        let icao = 0x444444;
        // TC19 subtype 1 with the reference bit layout: E/W +400 kt
        // (raw 401), N/S +200 kt (raw 201) → speed ≈ 447.2 kt.
        let mm = synthetic_df17(icao, [(19 << 3) | 1, 0x01, 0x91, 0x19, 0x20, 0x00, 0x00]);
        pipeline.merge_message(&mm);
        let entry = pipeline.aircraft.get(&icao).expect("entry created");
        let speed = entry.ground_speed_kt.expect("velocity set");
        assert!((speed - 447.2).abs() < 0.5, "speed wrong: {speed}");
        let track = entry.track_deg.expect("heading set");
        assert!((track - 63.4).abs() < 0.5, "heading wrong: {track}");
        assert!((0.0..360.0).contains(&track));
    }

    #[test]
    fn merge_message_extracts_vertical_rate() {
        let (_bus, mut pipeline) = test_pipeline();
        let icao = 0x454545;
        // TC19, vertical-rate raw value 11 => 640 ft/min descent (sign bit set).
        let mm = synthetic_df17(icao, [(19 << 3) | 1, 0x01, 0x91, 0x19, 0x28, 11 << 2, 0x00]);
        pipeline.merge_message(&mm);
        assert_eq!(pipeline.aircraft[&icao].vertical_rate_fpm, Some(-640));
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
        let snapshot = out
            .try_recv()
            .expect("expected a published snapshot")
            .aircraft;
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

#[cfg(test)]
#[path = "packet_capture_tests.rs"]
mod capture_tests;
