//! Statistics helpers
//!
//! Translated from dump1090's `stats.c`. Accumulates counters, power
//! measurements, histograms, and per-second rate tracking.

use std::fmt;
use std::time::Duration;

/// Maximum correctable bit errors.
pub const MAX_BITERRORS: usize = 2;

/// Downlink formats tracked.
pub const DF_COUNT: usize = 32;

/// Range histogram buckets (distance).
pub const RANGE_BUCKET_COUNT: usize = 76;

/// Gain step slots.
pub const GAIN_COUNT: usize = 64;

/// Signal histogram bins (3 dB per bin, from -96 dBFS to 0 dBFS).
pub const SIGNAL_HISTOGRAM_BINS: usize = 32;
/// Lower edge of the first signal histogram bin in dBFS.
pub const SIGNAL_HISTOGRAM_MIN_DB: f64 = -96.0;
/// Width of each signal histogram bin in dB.
pub const SIGNAL_HISTOGRAM_STEP_DB: f64 = 3.0;

/// Seconds kept in the per-second rate ring buffer.
pub const RATE_HISTORY_SECONDS: usize = 60;

/// Accumulated statistics for a receiver interval.
#[derive(Debug, Clone)]
pub struct Stats {
    pub start_ms: u64,
    pub end_ms: u64,

    // Mode S demodulator
    pub demod_preambles: u32,
    pub demod_rejected_bad: u32,
    pub demod_rejected_unknown_icao: u32,
    pub demod_accepted: [u32; MAX_BITERRORS + 1],

    // Mode A/C
    pub demod_modeac: u32,

    pub samples_processed: u64,
    pub samples_dropped: u64,

    pub sdr_gain: i32,

    // CPU timing
    pub demod_cpu: Duration,
    pub reader_cpu: Duration,
    pub background_cpu: Duration,

    // Power measurements (linear)
    pub noise_power_sum: f64,
    pub noise_power_count: u64,
    pub signal_power_sum: f64,
    pub signal_power_count: u64,
    pub peak_signal_power: f64,
    pub strong_signal_count: u32,

    // Remote (network) messages
    pub remote_received_modeac: u32,
    pub remote_received_modes: u32,
    pub remote_rejected_bad: u32,
    pub remote_rejected_unknown_icao: u32,
    pub remote_accepted: [u32; MAX_BITERRORS + 1],

    // Total messages
    pub messages_total: u32,
    pub messages_by_df: [u32; DF_COUNT],

    // CPR decoding results
    pub cpr_surface: u32,
    pub cpr_airborne: u32,
    pub cpr_global_ok: u32,
    pub cpr_global_bad: u32,
    pub cpr_global_skipped: u32,
    pub cpr_global_range_checks: u32,
    pub cpr_global_speed_checks: u32,
    pub cpr_local_ok: u32,
    pub cpr_local_skipped: u32,
    pub cpr_local_range_checks: u32,
    pub cpr_local_speed_checks: u32,
    pub cpr_local_aircraft_relative: u32,
    pub cpr_local_receiver_relative: u32,
    pub cpr_filtered: u32,

    pub suppressed_altitude_messages: u32,

    // Aircraft tracking
    pub unique_aircraft: u32,
    pub single_message_aircraft: u32,
    pub unreliable_aircraft: u32,

    // Histograms
    pub range_histogram: [u32; RANGE_BUCKET_COUNT],
    pub signal_histogram: [u32; SIGNAL_HISTOGRAM_BINS],

    // Per-second rate tracking
    pub rate_history: [u32; RATE_HISTORY_SECONDS],
    pub rate_cursor: usize,
    pub current_second_start: u64,
    pub current_second_messages: u32,

    // Adaptive gain measurements
    pub adaptive_valid: bool,
    pub adaptive_gain_seconds: [u32; GAIN_COUNT],
    pub adaptive_loud_undecoded: u32,
    pub adaptive_loud_decoded: u32,
    pub adaptive_gain_changes: u32,
    pub adaptive_noise_dbfs: f64,
    pub adaptive_range_gain_limit: i32,
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            start_ms: 0,
            end_ms: 0,
            demod_preambles: 0,
            demod_rejected_bad: 0,
            demod_rejected_unknown_icao: 0,
            demod_accepted: [0; MAX_BITERRORS + 1],
            demod_modeac: 0,
            samples_processed: 0,
            samples_dropped: 0,
            sdr_gain: -1,
            demod_cpu: Duration::ZERO,
            reader_cpu: Duration::ZERO,
            background_cpu: Duration::ZERO,
            noise_power_sum: 0.0,
            noise_power_count: 0,
            signal_power_sum: 0.0,
            signal_power_count: 0,
            peak_signal_power: 0.0,
            strong_signal_count: 0,
            remote_received_modeac: 0,
            remote_received_modes: 0,
            remote_rejected_bad: 0,
            remote_rejected_unknown_icao: 0,
            remote_accepted: [0; MAX_BITERRORS + 1],
            messages_total: 0,
            messages_by_df: [0; DF_COUNT],
            cpr_surface: 0,
            cpr_airborne: 0,
            cpr_global_ok: 0,
            cpr_global_bad: 0,
            cpr_global_skipped: 0,
            cpr_global_range_checks: 0,
            cpr_global_speed_checks: 0,
            cpr_local_ok: 0,
            cpr_local_skipped: 0,
            cpr_local_range_checks: 0,
            cpr_local_speed_checks: 0,
            cpr_local_aircraft_relative: 0,
            cpr_local_receiver_relative: 0,
            cpr_filtered: 0,
            suppressed_altitude_messages: 0,
            unique_aircraft: 0,
            single_message_aircraft: 0,
            unreliable_aircraft: 0,
            range_histogram: [0; RANGE_BUCKET_COUNT],
            signal_histogram: [0; SIGNAL_HISTOGRAM_BINS],
            rate_history: [0; RATE_HISTORY_SECONDS],
            rate_cursor: 0,
            current_second_start: 0,
            current_second_messages: 0,
            adaptive_valid: false,
            adaptive_gain_seconds: [0; GAIN_COUNT],
            adaptive_loud_undecoded: 0,
            adaptive_loud_decoded: 0,
            adaptive_gain_changes: 0,
            adaptive_noise_dbfs: 0.0,
            adaptive_range_gain_limit: -1,
        }
    }
}

impl Stats {
    /// Reset all counters to zero.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Record a Mode-S message with the given corrected bit count.
    pub fn record_demod_accepted(&mut self, bit_errors: usize) {
        let idx = bit_errors.min(MAX_BITERRORS);
        self.demod_accepted[idx] += 1;
    }

    /// Record a remote message with the given corrected bit count.
    pub fn record_remote_accepted(&mut self, bit_errors: usize) {
        let idx = bit_errors.min(MAX_BITERRORS);
        self.remote_accepted[idx] += 1;
    }

    /// Record total message arrival with DF type.
    pub fn record_message(&mut self, df: u8) {
        self.messages_total += 1;
        if (df as usize) < DF_COUNT {
            self.messages_by_df[df as usize] += 1;
        }
        self.current_second_messages += 1;
    }

    /// Record signal power (linear fraction of full scale) and bin it.
    pub fn record_signal_power(&mut self, power: f64) {
        self.signal_power_sum += power;
        self.signal_power_count += 1;
        if power > self.peak_signal_power {
            self.peak_signal_power = power;
        }
        if power > 0.5 {
            // above -3 dBFS in linear power
            self.strong_signal_count += 1;
        }

        let dbfs = 10.0 * power.log10();
        let bin = ((dbfs - SIGNAL_HISTOGRAM_MIN_DB) / SIGNAL_HISTOGRAM_STEP_DB)
            .floor()
            .clamp(0.0, (SIGNAL_HISTOGRAM_BINS - 1) as f64) as usize;
        self.signal_histogram[bin] += 1;
    }

    /// Record noise power (linear fraction of full scale).
    pub fn record_noise_power(&mut self, power: f64) {
        self.noise_power_sum += power;
        self.noise_power_count += 1;
    }

    /// Record an aircraft at a given range into the range histogram.
    pub fn record_range(&mut self, range_m: f64, max_range_m: f64) {
        if max_range_m <= 0.0 {
            return;
        }
        let bucket = ((range_m / max_range_m) * RANGE_BUCKET_COUNT as f64).floor() as usize;
        let bucket = bucket.min(RANGE_BUCKET_COUNT - 1);
        self.range_histogram[bucket] += 1;
    }

    /// Advance per-second rate tracking. Call with the current monotonic time in ms.
    pub fn tick_second(&mut self, now_ms: u64) {
        if self.start_ms == 0 {
            self.start_ms = now_ms;
        }
        self.end_ms = now_ms;

        if self.current_second_start == 0 {
            self.current_second_start = now_ms;
        }

        while now_ms >= self.current_second_start + 1000 {
            self.rate_history[self.rate_cursor] = self.current_second_messages;
            self.rate_cursor = (self.rate_cursor + 1) % RATE_HISTORY_SECONDS;
            self.current_second_messages = 0;
            self.current_second_start += 1000;
        }
    }

    /// Average messages per second over the non-zero recorded history.
    pub fn message_rate(&self) -> f64 {
        let sum: u32 = self.rate_history.iter().sum();
        if sum == 0 {
            0.0
        } else {
            let non_zero = self.rate_history.iter().filter(|&&x| x > 0).count().max(1);
            sum as f64 / non_zero as f64
        }
    }

    /// Merge `other` into `self`. Follows the C `add_stats` logic.
    pub fn add(&mut self, other: &Self) {
        self.start_ms = if self.start_ms == 0 {
            other.start_ms
        } else if other.start_ms == 0 {
            self.start_ms
        } else {
            self.start_ms.min(other.start_ms)
        };

        let (newer_end_ms, newer_sdr_gain, newer_rate_history, newer_rate_cursor,
             newer_current_second_start, newer_current_second_messages) =
            if other.end_ms > self.end_ms
                || (other.end_ms == self.end_ms && other.start_ms > self.start_ms)
            {
                (other.end_ms, other.sdr_gain, other.rate_history, other.rate_cursor,
                 other.current_second_start, other.current_second_messages)
            } else {
                (self.end_ms, self.sdr_gain, self.rate_history, self.rate_cursor,
                 self.current_second_start, self.current_second_messages)
            };
        self.end_ms = newer_end_ms;
        self.sdr_gain = newer_sdr_gain;

        self.demod_preambles += other.demod_preambles;
        self.demod_rejected_bad += other.demod_rejected_bad;
        self.demod_rejected_unknown_icao += other.demod_rejected_unknown_icao;
        for i in 0..=MAX_BITERRORS {
            self.demod_accepted[i] += other.demod_accepted[i];
        }
        self.demod_modeac += other.demod_modeac;

        self.samples_processed += other.samples_processed;
        self.samples_dropped += other.samples_dropped;

        self.demod_cpu += other.demod_cpu;
        self.reader_cpu += other.reader_cpu;
        self.background_cpu += other.background_cpu;

        self.noise_power_sum += other.noise_power_sum;
        self.noise_power_count += other.noise_power_count;

        self.signal_power_sum += other.signal_power_sum;
        self.signal_power_count += other.signal_power_count;

        self.peak_signal_power = self.peak_signal_power.max(other.peak_signal_power);

        self.strong_signal_count += other.strong_signal_count;

        self.remote_received_modeac += other.remote_received_modeac;
        self.remote_received_modes += other.remote_received_modes;
        self.remote_rejected_bad += other.remote_rejected_bad;
        self.remote_rejected_unknown_icao += other.remote_rejected_unknown_icao;
        for i in 0..=MAX_BITERRORS {
            self.remote_accepted[i] += other.remote_accepted[i];
        }

        self.messages_total += other.messages_total;
        for i in 0..DF_COUNT {
            self.messages_by_df[i] += other.messages_by_df[i];
        }

        self.cpr_surface += other.cpr_surface;
        self.cpr_airborne += other.cpr_airborne;
        self.cpr_global_ok += other.cpr_global_ok;
        self.cpr_global_bad += other.cpr_global_bad;
        self.cpr_global_skipped += other.cpr_global_skipped;
        self.cpr_global_range_checks += other.cpr_global_range_checks;
        self.cpr_global_speed_checks += other.cpr_global_speed_checks;
        self.cpr_local_ok += other.cpr_local_ok;
        self.cpr_local_skipped += other.cpr_local_skipped;
        self.cpr_local_range_checks += other.cpr_local_range_checks;
        self.cpr_local_speed_checks += other.cpr_local_speed_checks;
        self.cpr_local_aircraft_relative += other.cpr_local_aircraft_relative;
        self.cpr_local_receiver_relative += other.cpr_local_receiver_relative;
        self.cpr_filtered += other.cpr_filtered;

        self.suppressed_altitude_messages += other.suppressed_altitude_messages;

        self.unique_aircraft += other.unique_aircraft;
        self.single_message_aircraft += other.single_message_aircraft;
        self.unreliable_aircraft += other.unreliable_aircraft;

        for i in 0..RANGE_BUCKET_COUNT {
            self.range_histogram[i] += other.range_histogram[i];
        }

        for i in 0..SIGNAL_HISTOGRAM_BINS {
            self.signal_histogram[i] += other.signal_histogram[i];
        }

        // Rate tracking: take the newer instance's volatile state
        self.rate_history = newer_rate_history;
        self.rate_cursor = newer_rate_cursor;
        self.current_second_start = newer_current_second_start;
        self.current_second_messages = newer_current_second_messages;

        // Adaptive: take newest valid set
        let (adaptive_best_valid, adaptive_best_noise, adaptive_best_range) =
            if self.adaptive_valid && other.adaptive_valid {
                if other.end_ms > self.end_ms
                    || (other.end_ms == self.end_ms && other.start_ms > self.start_ms)
                {
                    (other.adaptive_valid, other.adaptive_noise_dbfs, other.adaptive_range_gain_limit)
                } else {
                    (self.adaptive_valid, self.adaptive_noise_dbfs, self.adaptive_range_gain_limit)
                }
            } else if self.adaptive_valid {
                (self.adaptive_valid, self.adaptive_noise_dbfs, self.adaptive_range_gain_limit)
            } else {
                (other.adaptive_valid, other.adaptive_noise_dbfs, other.adaptive_range_gain_limit)
            };

        self.adaptive_valid = adaptive_best_valid;
        for i in 0..GAIN_COUNT {
            self.adaptive_gain_seconds[i] += other.adaptive_gain_seconds[i];
        }
        self.adaptive_loud_undecoded += other.adaptive_loud_undecoded;
        self.adaptive_loud_decoded += other.adaptive_loud_decoded;
        self.adaptive_gain_changes += other.adaptive_gain_changes;
        self.adaptive_noise_dbfs = adaptive_best_noise;
        self.adaptive_range_gain_limit = adaptive_best_range;
    }
}

impl fmt::Display for Stats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Statistics: {} ms - {} ms", self.start_ms, self.end_ms)?;

        writeln!(f, "Local receiver:")?;
        writeln!(f, "  {:>12} samples processed", self.samples_processed)?;
        writeln!(f, "  {:>12} samples dropped", self.samples_dropped)?;
        writeln!(f, "  {:>12} Mode A/C messages received", self.demod_modeac)?;
        writeln!(
            f,
            "  {:>12} Mode-S message preambles received",
            self.demod_preambles
        )?;
        writeln!(
            f,
            "    {:>12} with bad message format or invalid CRC",
            self.demod_rejected_bad
        )?;
        writeln!(
            f,
            "    {:>12} with unrecognized ICAO address",
            self.demod_rejected_unknown_icao
        )?;
        writeln!(
            f,
            "    {:>12} accepted with correct CRC",
            self.demod_accepted[0]
        )?;
        for i in 1..=MAX_BITERRORS {
            writeln!(
                f,
                "    {:>12} accepted with {}-bit error repaired",
                self.demod_accepted[i], i
            )?;
        }

        if self.noise_power_count > 0 {
            let dbfs = 10.0 * (self.noise_power_sum / self.noise_power_count as f64).log10();
            writeln!(f, "  {:>5.1} dBFS noise power", dbfs)?;
        } else {
            writeln!(f, "  ----- dBFS noise power")?;
        }

        if self.signal_power_count > 0 {
            let dbfs = 10.0 * (self.signal_power_sum / self.signal_power_count as f64).log10();
            writeln!(f, "  {:>5.1} dBFS mean signal power", dbfs)?;
        } else {
            writeln!(f, "  ----- dBFS mean signal power")?;
        }

        if self.peak_signal_power > 0.0 {
            let dbfs = 10.0 * self.peak_signal_power.log10();
            writeln!(f, "  {:>5.1} dBFS peak signal power", dbfs)?;
        } else {
            writeln!(f, "  ----- dBFS peak signal power")?;
        }

        writeln!(
            f,
            "  {:>5} messages with signal power above -3dBFS",
            self.strong_signal_count
        )?;

        if self.sdr_gain >= 0 {
            writeln!(f, "  {:>4.1} dB current SDR gain (step {})", self.sdr_gain as f32, self.sdr_gain)?;
        }

        if self.adaptive_valid {
            writeln!(f, "Adaptive gain:")?;
            writeln!(f, "  {:>5} loud undecoded bursts", self.adaptive_loud_undecoded)?;
            writeln!(f, "  {:>5} loud decoded messages", self.adaptive_loud_decoded)?;
            writeln!(f, "  {:>5.1} dBFS latest noise floor", self.adaptive_noise_dbfs)?;
            writeln!(
                f,
                "  {:>5} latest dynamic range gain upper limit (step {})",
                self.adaptive_range_gain_limit, self.adaptive_range_gain_limit
            )?;
            writeln!(
                f,
                "  {:>5} gain changes caused by adaptive gain control",
                self.adaptive_gain_changes
            )?;

            let total_seconds: u32 = self.adaptive_gain_seconds.iter().sum();
            if total_seconds > 0 {
                let mut count = 0u32;
                for (i, &sec) in self.adaptive_gain_seconds.iter().enumerate() {
                    count += sec;
                    if count >= total_seconds / 2 {
                        writeln!(f, "  {:>5} dB median gain (step {})", i as f32, i)?;
                        break;
                    }
                }
                writeln!(f, "  Gain histogram:")?;
                for (i, &sec) in self.adaptive_gain_seconds.iter().enumerate() {
                    if sec > 0 {
                        writeln!(
                            f,
                            "    {:>5.1} dB: {:>5} seconds ({:>5.1}%)",
                            i as f32,
                            sec,
                            100.0 * sec as f64 / total_seconds as f64
                        )?;
                    }
                }
            }
        }

        writeln!(f, "Remote messages:")?;
        writeln!(
            f,
            "  {:>8} Mode A/C messages received",
            self.remote_received_modeac
        )?;
        writeln!(
            f,
            "  {:>8} Mode S messages received",
            self.remote_received_modes
        )?;
        writeln!(
            f,
            "    {:>8} with bad message format or invalid CRC",
            self.remote_rejected_bad
        )?;
        writeln!(
            f,
            "    {:>8} with unrecognized ICAO address",
            self.remote_rejected_unknown_icao
        )?;
        writeln!(
            f,
            "    {:>8} accepted with correct CRC",
            self.remote_accepted[0]
        )?;
        for i in 1..=MAX_BITERRORS {
            writeln!(
                f,
                "    {:>8} accepted with {}-bit error repaired",
                self.remote_accepted[i], i
            )?;
        }

        writeln!(f, "Decoder:")?;
        writeln!(f, "  {:>8} total usable messages", self.messages_total)?;
        for i in 0..DF_COUNT {
            if self.messages_by_df[i] > 0 {
                writeln!(f, "    {:>8} DF{} messages", self.messages_by_df[i], i)?;
            }
        }

        writeln!(
            f,
            "  {:>8} surface position messages received",
            self.cpr_surface
        )?;
        writeln!(
            f,
            "  {:>8} airborne position messages received",
            self.cpr_airborne
        )?;
        writeln!(
            f,
            "  {:>8} global CPR attempts with valid positions",
            self.cpr_global_ok
        )?;
        writeln!(
            f,
            "  {:>8} global CPR attempts with bad data",
            self.cpr_global_bad
        )?;
        writeln!(
            f,
            "    {:>8} global CPR attempts that failed the range check",
            self.cpr_global_range_checks
        )?;
        writeln!(
            f,
            "    {:>8} global CPR attempts that failed the speed check",
            self.cpr_global_speed_checks
        )?;
        writeln!(
            f,
            "  {:>8} global CPR attempts with insufficient data",
            self.cpr_global_skipped
        )?;
        writeln!(
            f,
            "  {:>8} local CPR attempts with valid positions",
            self.cpr_local_ok
        )?;
        writeln!(
            f,
            "    {:>8} aircraft-relative positions",
            self.cpr_local_aircraft_relative
        )?;
        writeln!(
            f,
            "    {:>8} receiver-relative positions",
            self.cpr_local_receiver_relative
        )?;
        writeln!(
            f,
            "  {:>8} local CPR attempts that did not produce useful positions",
            self.cpr_local_skipped
        )?;
        writeln!(
            f,
            "    {:>8} local CPR attempts that failed the range check",
            self.cpr_local_range_checks
        )?;
        writeln!(
            f,
            "    {:>8} local CPR attempts that failed the speed check",
            self.cpr_local_speed_checks
        )?;
        writeln!(
            f,
            "  {:>8} CPR messages that look like transponder failures filtered",
            self.cpr_filtered
        )?;

        writeln!(
            f,
            "  {:>8} non-ES altitude messages from ES-equipped aircraft ignored",
            self.suppressed_altitude_messages
        )?;
        writeln!(f, "  {:>8} unique aircraft tracks", self.unique_aircraft)?;
        writeln!(
            f,
            "  {:>8} aircraft tracks where only one message was seen",
            self.single_message_aircraft
        )?;
        writeln!(
            f,
            "  {:>8} aircraft tracks which were not marked reliable",
            self.unreliable_aircraft
        )?;

        let demod_ms = self.demod_cpu.as_millis() as u64;
        let reader_ms = self.reader_cpu.as_millis() as u64;
        let bg_ms = self.background_cpu.as_millis() as u64;
        let elapsed = self.end_ms.saturating_sub(self.start_ms).max(1);
        let cpu_pct = 100.0 * (demod_ms + reader_ms + bg_ms) as f64 / elapsed as f64;
        writeln!(f, "CPU load: {:>5.1}%", cpu_pct)?;
        writeln!(f, "  {:>5} ms for demodulation", demod_ms)?;
        writeln!(f, "  {:>5} ms for reading from USB", reader_ms)?;
        writeln!(
            f,
            "  {:>5} ms for network input and background tasks",
            bg_ms
        )?;

        writeln!(f, "Signal histogram (3 dB bins, -96..0 dBFS):")?;
        for (i, &count) in self.signal_histogram.iter().enumerate() {
            if count > 0 {
                let low = SIGNAL_HISTOGRAM_MIN_DB + (i as f64) * SIGNAL_HISTOGRAM_STEP_DB;
                writeln!(
                    f,
                    "  {:>6.0}..{:>6.0} dBFS: {}",
                    low,
                    low + SIGNAL_HISTOGRAM_STEP_DB,
                    count
                )?;
            }
        }

        writeln!(
            f,
            "Per-second rate (avg over last {}s): {:.1} msg/s",
            RATE_HISTORY_SECONDS,
            self.message_rate()
        )?;

        Ok(())
    }
}
