//! Adaptive gain control
//!
//! Translated from dump1090's `adaptive.c`. Processes incoming sample
//! blocks, tracks the noise floor via a radix-sort percentile, detects
//! loud undecoded bursts, and suggests SDR gain changes.

use std::f64;

/// Number of gain step slots tracked for statistics.
pub const GAIN_COUNT: usize = 64;

/// u16 sample threshold corresponding to approximately -3 dBFS amplitude.
const LOUD_SAMPLE_THRESHOLD: u16 = 46395;

/// Decoded message metadata consumed by the adaptive controller.
#[derive(Debug, Clone, Copy)]
pub struct DecodedMessage {
    /// Signal level as a fraction of full-scale power [0.0..1.0].
    pub signal_level: f64,
}

/// Dynamic range scanner state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RangeScanState {
    Idle,
    ScanUp,
    ScanDown,
    RescanUp,
    RescanDown,
}

/// Adaptive gain controller.
///
/// Tracks noise floor and burst rates over ~1 s blocks and suggests
/// gain steps to keep the receiver in a usable dynamic range.
pub struct AdaptiveGain {
    // ====== Configuration ======
    pub sample_rate: u32,
    pub min_gain_db: f32,
    pub max_gain_db: f32,
    pub duty_cycle: f32,
    pub burst_enabled: bool,
    pub burst_alpha: f32,
    pub burst_change_delay: u32,
    pub burst_loud_rate: f32,
    pub burst_loud_runlength: u32,
    pub burst_quiet_rate: f32,
    pub burst_quiet_runlength: u32,
    pub range_enabled: bool,
    pub range_alpha: f32,
    pub range_percentile: u32,
    /// Target dynamic range in dB.
    pub range_target_db: f32,
    pub range_change_delay: u32,
    pub range_scan_delay: u32,
    pub range_rescan_delay: u32,

    // ====== Derived timing ======
    samples_per_window: usize,
    samples_per_subblock: usize,
    subblocks_per_block: usize,
    duty_n: usize,
    duty_d: usize,

    // ====== Block / subblock state ======
    subblocks_remaining: usize,
    subblock_samples_remaining: usize,
    duty_counter: usize,
    subblock_active: bool,

    // ====== Burst state ======
    burst_window_remaining: usize,
    burst_window_counter: usize,
    burst_runlength: usize,
    burst_block_loud_undecoded: u32,
    burst_block_loud_decoded: u32,
    burst_loud_undecoded_smoothed: f64,
    burst_loud_decoded_smoothed: f64,
    burst_change_timer: u32,
    burst_loud_threshold: f64,
    burst_loud_blocks: u32,
    burst_quiet_blocks: u32,

    // ====== Range / noise floor state ======
    range_radix: Vec<u32>,
    range_radix_counter: usize,
    range_smoothed: f64,
    range_state: RangeScanState,
    range_change_timer: u32,
    range_rescan_timer: u32,
    range_gain_limit: i32,

    // ====== Gain bookkeeping ======
    gain_table: Vec<f32>,
    gain_min: i32,
    gain_max: i32,
    gain_up_db: f32,
    gain_down_db: f32,
    current_gain_step: i32,

    // ====== Outputs ======
    /// Latest suggested gain step (consumed by caller).
    pub suggested_gain_step: Option<i32>,
    /// Latest noise-floor estimate in dBFS.
    pub noise_floor_dbfs: f64,
    /// Latest available dynamic range in dB.
    pub dynamic_range_db: f64,
    /// Total loud undecoded bursts observed.
    pub loud_undecoded: u32,
    /// Total loud decoded messages observed.
    pub loud_decoded: u32,
    /// Total gain changes performed.
    pub gain_changes: u32,
    /// Seconds spent at each gain step (indexed by step).
    pub gain_seconds: [u32; GAIN_COUNT],
}

impl AdaptiveGain {
    /// Create a new controller with the given sample rate.
    ///
    /// Default configuration matches typical dump1090 defaults.
    pub fn new(sample_rate: u32) -> Self {
        let samples_per_window = (sample_rate / 25_000).max(1) as usize;
        let samples_per_subblock = samples_per_window * 1250;
        let subblocks_per_block = 20;
        let duty_d = subblocks_per_block;
        let duty_n = duty_d; // 100 % until configured

        Self {
            sample_rate,
            min_gain_db: 0.0,
            max_gain_db: 100.0,
            duty_cycle: 1.0,
            burst_enabled: false,
            burst_alpha: 0.1,
            burst_change_delay: 0,
            burst_loud_rate: 5.0,
            burst_loud_runlength: 3,
            burst_quiet_rate: 1.0,
            burst_quiet_runlength: 3,
            range_enabled: false,
            range_alpha: 0.1,
            range_percentile: 10,
            range_target_db: 50.0,
            range_change_delay: 0,
            range_scan_delay: 60,
            range_rescan_delay: 300,

            samples_per_window,
            samples_per_subblock,
            subblocks_per_block,
            duty_n,
            duty_d,

            subblocks_remaining: subblocks_per_block,
            subblock_samples_remaining: samples_per_subblock,
            duty_counter: 0,
            subblock_active: false,

            burst_window_remaining: samples_per_window,
            burst_window_counter: 0,
            burst_runlength: 0,
            burst_block_loud_undecoded: 0,
            burst_block_loud_decoded: 0,
            burst_loud_undecoded_smoothed: 0.0,
            burst_loud_decoded_smoothed: 0.0,
            burst_change_timer: 0,
            burst_loud_threshold: 0.0,
            burst_loud_blocks: 0,
            burst_quiet_blocks: 0,

            range_radix: vec![0u32; 65536],
            range_radix_counter: 0,
            range_smoothed: 0.0,
            range_state: RangeScanState::RescanUp,
            range_change_timer: 0,
            range_rescan_timer: 0,
            range_gain_limit: 0,

            gain_table: Vec::new(),
            gain_min: 0,
            gain_max: 0,
            gain_up_db: 0.0,
            gain_down_db: 0.0,
            current_gain_step: 0,

            suggested_gain_step: None,
            noise_floor_dbfs: 0.0,
            dynamic_range_db: 0.0,
            loud_undecoded: 0,
            loud_decoded: 0,
            gain_changes: 0,
            gain_seconds: [0; GAIN_COUNT],
        }
    }

    /// Provide the SDR gain lookup table and current step.
    ///
    /// `table[i]` is the dB value for step `i`. The controller clamps
    /// its operating range to `[min_gain_db, max_gain_db]`.
    pub fn configure_gain(&mut self, table: Vec<f32>, current_step: i32) {
        self.gain_table = table;
        self.recompute_gain_limits();
        self.set_gain(current_step);
        self.range_gain_limit = self.current_gain_step;
    }

    /// Set the duty cycle fraction (0.0..1.0].
    pub fn set_duty_cycle(&mut self, duty: f32) {
        let n = (self.duty_d as f32 * duty.clamp(0.0001, 1.0)).round().max(1.0) as usize;
        self.duty_n = n.min(self.duty_d);
    }

    /// Update internal thresholds to reflect a new current gain step.
    fn gain_changed(&mut self) {
        let step = self.current_gain_step.clamp(0, self.gain_table.len().saturating_sub(1) as i32);
        self.current_gain_step = step;

        let current_db = self.gain_table[step as usize];
        let up_db = self.gain_table.get((step + 1) as usize).copied().unwrap_or(current_db);
        let down_db = self.gain_table.get((step - 1) as usize).copied().unwrap_or(current_db);
        self.gain_up_db = up_db - current_db;
        self.gain_down_db = current_db - down_db;

        let loud_threshold_dbfs = 0.0 - self.gain_up_db - 3.0;
        self.burst_loud_threshold = 10f64.powf(loud_threshold_dbfs as f64 / 10.0);

        self.range_change_timer = self.range_change_delay;
        self.burst_change_timer = self.burst_change_delay;
        self.burst_loud_blocks = 0;
        self.burst_quiet_blocks = 0;
    }

    /// Attempt to set the current gain step.
    fn set_gain(&mut self, step: i32) -> bool {
        let step = step.clamp(self.gain_min, self.gain_max);
        if self.current_gain_step == step {
            return false;
        }
        self.current_gain_step = step;
        self.gain_changed();
        self.gain_changes += 1;
        true
    }

    fn recompute_gain_limits(&mut self) {
        let max_step = self.gain_table.len().saturating_sub(1) as i32;
        self.gain_min = max_step;
        for step in 0..=max_step {
            if self.gain_table[step as usize] >= self.min_gain_db {
                self.gain_min = step;
                break;
            }
        }
        self.gain_max = self.gain_min;
        for step in (self.gain_min..=max_step).rev() {
            if self.gain_table[step as usize] <= self.max_gain_db {
                self.gain_max = step;
                break;
            }
        }
    }

    /// Feed samples into the adaptive system.
    ///
    /// `decoded` should be provided when the buffer corresponds to a
    /// successfully decoded message; those samples are then skipped for
    /// burst detection.
    ///
    /// Returns `Some(gain_step)` if a gain change is suggested this block.
    pub fn update(&mut self, buf: &[u16], decoded: Option<&DecodedMessage>) -> Option<i32> {
        if !self.burst_enabled && !self.range_enabled {
            return None;
        }

        let mut buf = buf;
        let mut suggested = None;

        // Complete subblocks
        while buf.len() >= self.subblock_samples_remaining {
            let (chunk, rest) = buf.split_at(self.subblock_samples_remaining);
            if self.subblock_active {
                self.update_subblock(chunk, decoded);
            }
            buf = rest;

            self.subblock_samples_remaining = self.samples_per_subblock;

            self.duty_counter += self.duty_n;
            if self.duty_counter >= self.duty_d {
                self.duty_counter -= self.duty_d;
                self.subblock_active = true;
            } else {
                self.subblock_active = false;
                self.burst_end_of_window(0);
            }

            self.subblocks_remaining -= 1;
            if self.subblocks_remaining == 0 {
                self.subblocks_remaining = self.subblocks_per_block;
                self.end_of_block();
                if let Some(step) = self.suggested_gain_step.take() {
                    suggested = Some(step);
                }
            }
        }

        // Final partial subblock
        if !buf.is_empty() {
            if self.subblock_active {
                self.update_subblock(buf, decoded);
            }
            self.subblock_samples_remaining -= buf.len();
        }

        suggested
    }

    fn update_subblock(&mut self, buf: &[u16], decoded: Option<&DecodedMessage>) {
        if let Some(msg) = decoded {
            if msg.signal_level >= self.burst_loud_threshold {
                self.burst_block_loud_decoded += 1;
            }
            self.burst_skip(buf.len());
        } else {
            self.burst_update(buf);
            self.range_update(buf);
        }
    }

    fn end_of_block(&mut self) {
        self.range_end_of_block();
        self.burst_end_of_block();
        self.control_update();

        let idx = self.current_gain_step.clamp(0, GAIN_COUNT as i32 - 1) as usize;
        self.gain_seconds[idx] += 1;
    }

    // -----------------------------------------------------------------
    // Burst measurement
    // -----------------------------------------------------------------

    fn burst_skip(&mut self, mut length: usize) {
        if !self.burst_enabled {
            return;
        }

        if length < self.burst_window_remaining {
            self.burst_window_remaining -= length;
            return;
        }

        self.burst_end_of_window(self.burst_window_counter);
        length -= self.burst_window_remaining;

        let windows = length / self.samples_per_window;
        let samples = windows * self.samples_per_window;
        for _ in 0..windows {
            self.burst_end_of_window(0);
        }
        length -= samples;

        self.burst_window_counter = 0;
        self.burst_window_remaining = self.samples_per_window - length;
    }

    fn burst_update(&mut self, buf: &[u16]) {
        if !self.burst_enabled {
            return;
        }
        let mut buf = buf;
        let mut length = buf.len();

        if length < self.burst_window_remaining {
            self.burst_window_counter += count_loud_samples(buf);
            self.burst_window_remaining -= length;
            return;
        }

        let n = self.burst_window_remaining;
        let counter = self.burst_window_counter + count_loud_samples(&buf[..n]);
        self.burst_end_of_window(counter);
        buf = &buf[n..];
        length -= n;

        let windows = length / self.samples_per_window;
        let samples = windows * self.samples_per_window;
        self.burst_scan_windows(&buf[..samples]);
        buf = &buf[samples..];
        length -= samples;

        self.burst_window_counter = count_loud_samples(buf);
        self.burst_window_remaining = self.samples_per_window - length;
    }

    fn burst_scan_windows(&mut self, buf: &[u16]) {
        let windows = buf.len() / self.samples_per_window;
        for i in 0..windows {
            let start = i * self.samples_per_window;
            let end = start + self.samples_per_window;
            let counter = count_loud_samples(&buf[start..end]);
            self.burst_end_of_window(counter);
        }
    }

    fn burst_end_of_window(&mut self, counter: usize) {
        let threshold = self.samples_per_window / 4;
        if counter > threshold {
            self.burst_runlength += 1;
        } else {
            if self.burst_runlength >= 2 && self.burst_runlength <= 5 {
                self.burst_block_loud_undecoded += 1;
            }
            self.burst_runlength = 0;
        }
    }

    fn burst_end_of_block(&mut self) {
        if !self.burst_enabled {
            return;
        }

        let scale = self.duty_d as f64 / self.duty_n as f64;

        self.loud_undecoded += self.burst_block_loud_undecoded;
        let a = self.burst_alpha as f64;
        self.burst_loud_undecoded_smoothed = self.burst_loud_undecoded_smoothed * (1.0 - a)
            + scale * self.burst_block_loud_undecoded as f64 * a;
        self.burst_block_loud_undecoded = 0;

        self.loud_decoded += self.burst_block_loud_decoded;
        self.burst_loud_decoded_smoothed = self.burst_loud_decoded_smoothed * (1.0 - a)
            + scale * self.burst_block_loud_decoded as f64 * a;
        self.burst_block_loud_decoded = 0;
    }

    // -----------------------------------------------------------------
    // Range / noise floor measurement
    // -----------------------------------------------------------------

    fn range_update(&mut self, buf: &[u16]) {
        if !self.range_enabled {
            return;
        }
        self.range_radix_counter += buf.len();
        for &sample in buf {
            self.range_radix[sample as usize] += 1;
        }
    }

    fn range_end_of_block(&mut self) {
        if !self.range_enabled {
            return;
        }

        let count_n = (self.range_radix_counter as f64 * self.range_percentile as f64 / 100.0) as usize;
        let mut n = 0usize;
        let mut i = 0usize;
        while i < 65536 && n <= count_n {
            n += self.range_radix[i] as usize;
            i += 1;
        }
        let percentile_n = i.saturating_sub(1) as f64;

        let alpha = self.range_alpha as f64;
        self.range_smoothed = self.range_smoothed * (1.0 - alpha) + percentile_n * alpha;

        self.noise_floor_dbfs = if self.range_smoothed > 0.0 {
            20.0 * (self.range_smoothed / 65536.0).log10()
        } else {
            0.0
        };

        self.dynamic_range_db = if self.range_smoothed > 0.0 {
            -20.0 * (self.range_smoothed / 65536.0).log10()
        } else {
            0.0
        };

        self.range_radix.fill(0);
        self.range_radix_counter = 0;
    }

    // -----------------------------------------------------------------
    // Control logic
    // -----------------------------------------------------------------

    fn control_update(&mut self) {
        let mut gain_up = false;
        let mut gain_up_reason: Option<&str> = None;
        let mut gain_down = false;
        let mut gain_down_reason: Option<&str> = None;
        let mut gain_not_up = false;

        let current_gain = self.current_gain_step;

        if self.burst_change_timer > 0 {
            self.burst_change_timer -= 1;
        }
        if self.range_change_timer > 0 {
            self.range_change_timer -= 1;
        }
        if self.range_rescan_timer > 0 {
            self.range_rescan_timer -= 1;
        }

        // Burst control
        if self.burst_enabled && self.burst_change_timer == 0 {
            if self.burst_loud_undecoded_smoothed > self.burst_loud_rate as f64 {
                self.burst_quiet_blocks = 0;
                self.burst_loud_blocks += 1;
            } else if self.burst_loud_decoded_smoothed < self.burst_quiet_rate as f64 {
                self.burst_loud_blocks = 0;
                self.burst_quiet_blocks += 1;
            } else {
                self.burst_loud_blocks = 0;
                self.burst_quiet_blocks = 0;
            }

            if self.burst_loud_blocks >= self.burst_loud_runlength {
                gain_down = true;
                gain_not_up = true;
                gain_down_reason = Some("high rate of loud undecoded messages");

                if matches!(self.range_state, RangeScanState::ScanDown | RangeScanState::RescanDown)
                {
                    self.range_state = RangeScanState::Idle;
                    self.range_rescan_timer = 0;
                }
            } else if self.burst_quiet_blocks < self.burst_quiet_runlength {
                gain_not_up = true;
            } else if current_gain < self.range_gain_limit {
                gain_up = true;
                gain_up_reason = Some("low loud message rate and gain below dynamic range limit");
            }
        }

        // Range control
        if self.range_enabled && self.range_change_timer == 0 {
            let available_range = self.dynamic_range_db;
            if available_range >= self.range_target_db as f64 && current_gain > self.range_gain_limit
            {
                self.range_gain_limit = current_gain;
            }

            match self.range_state {
                RangeScanState::ScanUp | RangeScanState::RescanUp => {
                    if available_range < self.range_target_db as f64 {
                        gain_down = true;
                        gain_not_up = true;
                        gain_down_reason = Some("probing dynamic range gain lower bound");
                        self.range_state = if self.range_state == RangeScanState::RescanUp {
                            RangeScanState::RescanDown
                        } else {
                            RangeScanState::ScanDown
                        };
                        if self.range_gain_limit >= current_gain {
                            self.range_gain_limit = current_gain - 1;
                        }
                    } else if current_gain >= self.gain_max {
                        self.range_state = RangeScanState::Idle;
                        self.range_rescan_timer = self.range_rescan_delay;
                    } else if !gain_not_up {
                        gain_up = true;
                        gain_up_reason = Some("probing dynamic range gain upper bound");
                    }
                }

                RangeScanState::ScanDown | RangeScanState::RescanDown => {
                    if available_range >= self.range_target_db as f64 {
                        // C original reads state after setting it to IDLE,
                        // causing the ternary to always select rescan_delay.
                        // We replicate that behaviour.
                        self.range_state = RangeScanState::Idle;
                        self.range_rescan_timer = self.range_rescan_delay;
                    } else {
                        if self.range_gain_limit >= current_gain {
                            self.range_gain_limit = current_gain - 1;
                        }
                        if current_gain <= self.gain_min {
                            self.range_state = RangeScanState::Idle;
                            self.range_rescan_timer = self.range_rescan_delay;
                        } else {
                            gain_down = true;
                            gain_not_up = true;
                            gain_down_reason = Some("probing dynamic range gain lower bound");
                        }
                    }
                }

                RangeScanState::Idle => {
                    if available_range + (self.gain_down_db as f64) / 2.0
                        < self.range_target_db as f64
                        && current_gain > self.gain_min
                    {
                        if self.range_gain_limit >= current_gain {
                            self.range_gain_limit = current_gain - 1;
                        }
                        self.range_state = RangeScanState::ScanDown;
                        gain_down = true;
                        gain_not_up = true;
                        gain_down_reason = Some("dynamic range fell below target value");
                    } else if self.range_rescan_timer == 0 && !gain_not_up {
                        if available_range >= self.range_target_db as f64
                            && current_gain < self.gain_max
                        {
                            gain_up = true;
                            gain_up_reason = Some(
                                "periodic re-probing of dynamic range gain upper bound",
                            );
                            self.range_state = RangeScanState::RescanUp;
                        } else {
                            self.range_rescan_timer = self.range_rescan_delay;
                        }
                    }
                }
            }
        }

        // Apply gain change
        if gain_down {
            self.set_gain(current_gain - 1);
            self.suggested_gain_step = Some(self.current_gain_step);
        } else if gain_up && !gain_not_up {
            self.set_gain(current_gain + 1);
            self.suggested_gain_step = Some(self.current_gain_step);
        }
    }
}

/// Count samples above the -3 dBFS threshold.
fn count_loud_samples(buf: &[u16]) -> usize {
    buf.iter().filter(|&&s| s > LOUD_SAMPLE_THRESHOLD).count()
}
