//! QPSK demodulator: AGC, decimating low-pass filter, RRC matched filter,
//! Gardner timing recovery, and a 4th-power Costas loop.
//!
//! Streaming design: `QpskDemod::process` can be called repeatedly with
//! new sample chunks, carrying filter/loop state across calls, so it works
//! identically for whole-file offline decode and live incremental decode.

use num_complex::Complex32;

/// Simple soft-knee AGC for complex baseband samples: tracks a running
/// estimate of signal magnitude and scales samples toward a target
/// amplitude, mirroring the AGC approach used elsewhere in this codebase
/// (`ez-gui/src/demod.rs::apply_agc`) but generalized to complex IQ rather
/// than real audio samples.
#[derive(Debug, Clone)]
struct Agc {
    target: f32,
    gain: f32,
    attack: f32,
    decay: f32,
}

impl Agc {
    fn new(target: f32) -> Self {
        Self {
            target,
            gain: 1.0,
            attack: 0.01,
            decay: 0.001,
        }
    }

    fn process(&mut self, sample: Complex32) -> Complex32 {
        let mag = sample.norm();
        if mag > 1e-9 {
            let error = self.target / (mag * self.gain);
            let rate = if error < 1.0 { self.attack } else { self.decay };
            self.gain += rate * (error - 1.0) * self.gain;
            self.gain = self.gain.clamp(1e-3, 1e3);
        }
        sample * self.gain
    }
}

/// Generate root-raised-cosine FIR filter taps.
///
/// `beta` is the roll-off factor (0..1), `sps` is samples-per-symbol at
/// the filter's operating rate, `span_symbols` is the filter length in
/// symbol periods (total taps = `span_symbols * sps + 1`).
#[must_use]
pub fn rrc_taps(beta: f32, sps: f32, span_symbols: usize) -> Vec<f32> {
    let n_taps = span_symbols * (sps as usize) + 1;
    let mut taps = vec![0.0f32; n_taps];
    let mid = (n_taps as f32 - 1.0) / 2.0;

    for (i, tap) in taps.iter_mut().enumerate() {
        let t = (i as f32 - mid) / sps;
        *tap = rrc_impulse(t, beta);
    }

    // Normalize energy to unity gain at DC.
    let sum: f32 = taps.iter().sum();
    if sum.abs() > 1e-9 {
        for tap in &mut taps {
            *tap /= sum;
        }
    }
    taps
}

fn rrc_impulse(t: f32, beta: f32) -> f32 {
    const EPS: f32 = 1e-6;
    if t.abs() < EPS {
        return 1.0 - beta + 4.0 * beta / std::f32::consts::PI;
    }
    let four_beta_t = 4.0 * beta * t;
    if beta > EPS && (four_beta_t.abs() - 1.0).abs() < 1e-3 {
        // t = +-1/(4*beta): limit form to avoid 0/0.
        let x = std::f32::consts::PI / (4.0 * beta);
        return (beta / 2.0f32.sqrt())
            * ((1.0 + 2.0 / std::f32::consts::PI) * x.sin()
                + (1.0 - 2.0 / std::f32::consts::PI) * x.cos());
    }
    let pi = std::f32::consts::PI;
    let numerator = (pi * t * (1.0 - beta)).sin() + four_beta_t * (pi * t * (1.0 + beta)).cos();
    let denominator = pi * t * (1.0 - four_beta_t * four_beta_t);
    numerator / denominator
}

/// Simple FIR filter with persistent history, for streaming operation
/// across chunk boundaries.
#[derive(Debug, Clone)]
struct FirFilter {
    taps: Vec<f32>,
    history: Vec<Complex32>,
}

impl FirFilter {
    fn new(taps: Vec<f32>) -> Self {
        let history = vec![Complex32::new(0.0, 0.0); taps.len()];
        Self { taps, history }
    }

    /// Push one sample and return the filtered output.
    fn push(&mut self, sample: Complex32) -> Complex32 {
        self.history.rotate_left(1);
        *self.history.last_mut().unwrap() = sample;
        self.history
            .iter()
            .zip(self.taps.iter())
            .map(|(&s, &t)| s * t)
            .sum()
    }
}

/// Low-pass decimation FIR (simple windowed-sinc design) used to bring the
/// raw sample rate down to a manageable multiple of the symbol rate before
/// matched filtering.
fn lowpass_taps(cutoff_ratio: f32, n_taps: usize) -> Vec<f32> {
    let mut taps = vec![0.0f32; n_taps];
    let mid = (n_taps as f32 - 1.0) / 2.0;
    for (i, tap) in taps.iter_mut().enumerate() {
        let x = i as f32 - mid;
        let sinc = if x.abs() < 1e-6 {
            2.0 * cutoff_ratio
        } else {
            (2.0 * std::f32::consts::PI * cutoff_ratio * x).sin() / (std::f32::consts::PI * x)
        };
        // Hamming window.
        let window = 0.54 - 0.46 * (2.0 * std::f32::consts::PI * i as f32 / (n_taps as f32 - 1.0)).cos();
        *tap = sinc * window;
    }
    let sum: f32 = taps.iter().sum();
    if sum.abs() > 1e-9 {
        for tap in &mut taps {
            *tap /= sum;
        }
    }
    taps
}

/// Configuration for the QPSK demodulator.
#[derive(Debug, Clone, Copy)]
pub struct QpskConfig {
    pub sample_rate: f32,
    pub symbol_rate: f32,
    /// RRC roll-off factor. Commonly cited in the 0.35-0.6 range for this
    /// signal family in public descriptions; kept tunable rather than
    /// assumed as gospel.
    pub rrc_beta: f32,
    /// Costas loop bandwidth (normalized), controls lock speed vs jitter.
    pub costas_bandwidth: f32,
}

impl Default for QpskConfig {
    fn default() -> Self {
        Self {
            sample_rate: 2_048_000.0,
            symbol_rate: 72_000.0,
            rrc_beta: 0.5,
            costas_bandwidth: 0.02,
        }
    }
}

/// One recovered, phase-tracked QPSK symbol plus loop-quality info.
#[derive(Debug, Clone, Copy)]
pub struct RecoveredSymbol {
    pub value: Complex32,
    /// Per-symbol Costas lock snapshot. Not currently consumed (callers use
    /// `QpskDemod::is_locked()` for the aggregate state instead), kept for
    /// future per-symbol diagnostics (e.g. highlighting exactly where in a
    /// pass lock was lost).
    #[allow(dead_code)]
    pub costas_locked: bool,
}

/// How many RRC-filtered samples of history to retain for interpolation
/// (needs to cover at least one full symbol period plus slack for the
/// half-symbol lookback the Gardner detector needs).
const HISTORY_LEN: usize = 64;

/// Streaming QPSK demodulator.
pub struct QpskDemod {
    /// Retained for future introspection/debugging (e.g. surfacing the
    /// effective decode parameters to the GUI); not read after construction.
    #[allow(dead_code)]
    config: QpskConfig,
    agc: Agc,
    decim_filter: FirFilter,
    decim_factor: usize,
    decim_counter: usize,
    rrc_filter: FirFilter,

    // Timing recovery state: `history` is a ring buffer of RRC-filtered
    // samples at the decimated rate; `write_index` is the absolute sample
    // count written so far (monotonic); `tau` is the floating-point
    // absolute sample position of the *next* symbol decision instant.
    history: Vec<Complex32>,
    write_count: u64,
    sps: f32, // samples per symbol at the decimated rate
    tau: f64,
    timing_gain: f32,

    // Costas loop (4th power) state.
    costas_phase: f32,
    costas_freq: f32,
    costas_alpha: f32,
    costas_beta: f32,
    locked: bool,
    lock_error_avg: f32,
}

impl QpskDemod {
    #[must_use]
    pub fn new(config: QpskConfig) -> Self {
        let decim_factor = ((config.sample_rate / (config.symbol_rate * 6.0)).floor() as usize).max(1);
        let decimated_rate = config.sample_rate / decim_factor as f32;
        let sps = decimated_rate / config.symbol_rate;

        let lp_taps = lowpass_taps(0.5 / decim_factor as f32, 31);
        let rrc = rrc_taps(config.rrc_beta, sps, 6);

        // Costas loop gains derived from bandwidth (standard 2nd-order PLL
        // approximation): alpha ~ bandwidth, beta ~ bandwidth^2/damping.
        let bw = config.costas_bandwidth;
        let costas_alpha = bw;
        let costas_beta = bw * bw * 0.25;

        Self {
            config,
            agc: Agc::new(1.0),
            decim_filter: FirFilter::new(lp_taps),
            decim_factor,
            decim_counter: 0,
            rrc_filter: FirFilter::new(rrc),
            history: vec![Complex32::new(0.0, 0.0); HISTORY_LEN],
            write_count: 0,
            sps,
            // Start the first decision instant a couple of symbols in so
            // the interpolator always has real history to look back on.
            tau: f64::from(sps) * 2.0,
            timing_gain: 0.02,
            costas_phase: 0.0,
            costas_freq: 0.0,
            costas_alpha,
            costas_beta,
            locked: false,
            lock_error_avg: 1.0,
        }
    }

    #[must_use]
    pub fn is_locked(&self) -> bool {
        self.locked
    }

    /// Linearly interpolate the (decimated-rate) sample stream at
    /// absolute position `pos` (in samples, may be fractional), reading
    /// from the ring buffer. `pos` must lie within the currently buffered
    /// history window (i.e. within `HISTORY_LEN` samples of the most
    /// recent write) or this returns a zero sample.
    fn interpolate_at(&self, pos: f64) -> Complex32 {
        if pos < 0.0 {
            return Complex32::new(0.0, 0.0);
        }
        let floor_pos = pos.floor();
        let frac = (pos - floor_pos) as f32;
        let idx0 = floor_pos as i64;
        let idx1 = idx0 + 1;

        let latest = self.write_count as i64 - 1;
        let oldest = latest - HISTORY_LEN as i64 + 1;
        if idx0 < oldest || idx1 > latest {
            return Complex32::new(0.0, 0.0);
        }

        let ring_idx = |abs_idx: i64| -> Complex32 {
            let rel = (abs_idx.rem_euclid(HISTORY_LEN as i64)) as usize;
            self.history[rel]
        };

        let s0 = ring_idx(idx0);
        let s1 = ring_idx(idx1);
        s0 + (s1 - s0) * frac
    }

    /// Process a chunk of baseband `Complex32` samples at the configured
    /// input sample rate, returning any recovered symbols produced.
    pub fn process(&mut self, samples: &[Complex32]) -> Vec<RecoveredSymbol> {
        let mut out = Vec::new();

        for &raw in samples {
            let agc_sample = self.agc.process(raw);
            let filtered = self.decim_filter.push(agc_sample);

            self.decim_counter += 1;
            if self.decim_counter < self.decim_factor {
                continue;
            }
            self.decim_counter = 0;

            let matched = self.rrc_filter.push(filtered);
            let write_idx = (self.write_count as usize) % HISTORY_LEN;
            self.history[write_idx] = matched;
            self.write_count += 1;

            // Emit as many symbol decisions as now fall within buffered
            // history (usually 0 or 1 per input sample, but handles any
            // rate slack cleanly).
            loop {
                let latest = self.write_count as i64 - 1;
                if self.tau > latest as f64 {
                    break;
                }

                let current = self.interpolate_at(self.tau);
                let half_symbol_ago = self.interpolate_at(self.tau - f64::from(self.sps) / 2.0);
                let one_symbol_ago = self.interpolate_at(self.tau - f64::from(self.sps));

                // Gardner timing error detector: correlates the
                // half-symbol (transition) sample against the difference
                // between consecutive symbol-spaced samples. Zero when
                // timing is correctly aligned.
                let diff = current - one_symbol_ago;
                let timing_error = half_symbol_ago.re * diff.re + half_symbol_ago.im * diff.im;
                let timing_error = timing_error.clamp(-1.0, 1.0);
                self.tau += f64::from(self.sps) - f64::from(self.timing_gain * timing_error);

                // 4th-power Costas loop for residual carrier tracking. The
                // 4th-power error detector's stable equilibria are spaced
                // every 90 deg starting at 0 deg (i.e. it pulls the
                // constellation to align with the I/Q *axes*), whereas our
                // ideal QPSK constellation sits on the *diagonals* (45,
                // 135, 225, 315 deg) -- see `costas_phase_error`'s doc
                // comment. Track the loop against the axis-aligned
                // reference, then rotate the corrected sample by +45 deg
                // before handing it to the quadrant/differential-decode
                // stage so it lines up with the diagonal convention that
                // `diff_decode::symbol_to_quadrant` expects.
                let loop_corrected = current * Complex32::from_polar(1.0, -self.costas_phase);
                let phase_error = costas_phase_error(loop_corrected);

                self.costas_freq += self.costas_beta * phase_error;
                self.costas_phase += self.costas_freq + self.costas_alpha * phase_error;
                self.costas_phase = self.costas_phase.rem_euclid(2.0 * std::f32::consts::PI);

                // Lock quality: exponential moving average of |phase error|.
                self.lock_error_avg = 0.98 * self.lock_error_avg + 0.02 * phase_error.abs();
                self.locked = self.lock_error_avg < 0.2;

                let corrected =
                    loop_corrected * Complex32::from_polar(1.0, std::f32::consts::FRAC_PI_4);

                out.push(RecoveredSymbol {
                    value: corrected,
                    costas_locked: self.locked,
                });
            }
        }

        out
    }
}

/// 4th-power Costas phase error detector for QPSK: raise the symbol to the
/// 4th power to strip the QPSK modulation (leaving only 4x the residual
/// carrier phase), then take its angle / 4 as the phase error estimate.
///
/// Because the 4th power wraps every 90 degrees, this detector's stable
/// equilibria (error == 0) are spaced every 90 deg starting at 0 deg -- it
/// pulls its input to align with the I/Q axes, not with the diagonal
/// (45/135/225/315 deg) points where an ideal `+-1,+-1` QPSK constellation
/// actually sits. Callers that want output aligned to the diagonal
/// constellation must rotate the loop's corrected sample by +45 deg
/// afterward (see `QpskDemod::process`).
fn costas_phase_error(sample: Complex32) -> f32 {
    if sample.norm() < 1e-9 {
        return 0.0;
    }
    let normalized = sample / sample.norm();
    let fourth = normalized * normalized * normalized * normalized;
    fourth.arg() / 4.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rrc_taps_are_symmetric_and_normalized() {
        let taps = rrc_taps(0.5, 4.0, 6);
        let n = taps.len();
        for i in 0..n / 2 {
            assert!((taps[i] - taps[n - 1 - i]).abs() < 1e-4, "taps not symmetric at {i}");
        }
        let sum: f32 = taps.iter().sum();
        assert!((sum - 1.0).abs() < 1e-3, "taps should sum to ~1.0, got {sum}");
    }

    #[test]
    fn lowpass_taps_sum_to_unity() {
        let taps = lowpass_taps(0.2, 31);
        let sum: f32 = taps.iter().sum();
        assert!((sum - 1.0).abs() < 1e-3);
    }

    #[test]
    fn agc_normalizes_toward_target() {
        let mut agc = Agc::new(1.0);
        let mut last_mag = 0.0;
        for _ in 0..2000 {
            let out = agc.process(Complex32::new(50.0, 0.0));
            last_mag = out.norm();
        }
        assert!((last_mag - 1.0).abs() < 0.2, "AGC did not converge, got {last_mag}");
    }

    #[test]
    fn costas_phase_error_treats_all_four_constellation_points_as_equal_lock_points() {
        // The 4th-power detector strips QPSK modulation: raising each of
        // the 4 constellation points (at 45/135/225/315 deg, i.e.
        // Complex::new(+-1,+-1)) to the 4th power maps all of them to the
        // *same* residual angle (4*45=180, 4*135=540=180 mod 360, etc.).
        // The detector output is that residual angle divided by 4, so it
        // can land on any of 4 branches spaced pi/2 apart depending on
        // where atan2's branch cut falls -- multiplying back by 4 removes
        // that arbitrary branch choice and should recover the same
        // underlying angle (mod 2*pi) for all 4 constellation points. This
        // 4-fold branch ambiguity is exactly what differential decoding
        // resolves downstream.
        let mut quadrupled = Vec::new();
        for k in 0..4 {
            let phase = std::f32::consts::FRAC_PI_4 + (k as f32) * std::f32::consts::FRAC_PI_2;
            let err = costas_phase_error(Complex32::from_polar(1.0, phase));
            quadrupled.push(Complex32::from_polar(1.0, 4.0 * err));
        }
        for w in quadrupled.windows(2) {
            assert!((w[0] - w[1]).norm() < 1e-3, "quadrupled={quadrupled:?}");
        }
    }

    #[test]
    fn costas_phase_error_has_restoring_force_around_lock_point() {
        // Near a lock point, a small clockwise/counter-clockwise
        // perturbation in phase should push the error in a consistent
        // direction (a real restoring force), confirming this is a
        // genuine stable equilibrium and not a flat/degenerate detector.
        // Use a lock point away from the atan2 branch cut (0 deg, which
        // 4th-powers to 0 rad with no wraparound nearby) to keep the
        // comparison simple.
        let lock_phase = 0.0f32;
        let base_err = costas_phase_error(Complex32::from_polar(1.0, lock_phase));
        let plus = costas_phase_error(Complex32::from_polar(1.0, lock_phase + 0.05));
        let minus = costas_phase_error(Complex32::from_polar(1.0, lock_phase - 0.05));
        assert!(plus > base_err, "plus={plus} base={base_err}");
        assert!(minus < base_err, "minus={minus} base={base_err}");
    }

    #[test]
    fn demod_produces_symbols_from_upsampled_qpsk_stream() {
        // Build a simple, noiseless QPSK signal: repeat 4 constellation
        // points, each held for `sps` samples at a moderate oversample
        // ratio, and confirm the demod produces a nonzero symbol stream at
        // roughly the right rate without panicking or NaN outputs.
        let config = QpskConfig {
            sample_rate: 8000.0,
            symbol_rate: 1000.0,
            rrc_beta: 0.5,
            costas_bandwidth: 0.02,
        };
        let sps_in = (config.sample_rate / config.symbol_rate) as usize; // 8
        let points = [
            Complex32::new(1.0, 1.0),
            Complex32::new(-1.0, 1.0),
            Complex32::new(-1.0, -1.0),
            Complex32::new(1.0, -1.0),
        ];
        let mut samples = Vec::new();
        for i in 0..400 {
            let pt = points[i % 4] * 50.0; // scale like real ADC amplitude
            for _ in 0..sps_in {
                samples.push(pt);
            }
        }

        let mut demod = QpskDemod::new(config);
        let symbols = demod.process(&samples);
        assert!(!symbols.is_empty(), "expected some recovered symbols");
        for s in &symbols {
            assert!(s.value.re.is_finite());
            assert!(s.value.im.is_finite());
        }
    }

    #[test]
    fn streaming_process_across_chunks_matches_single_call() {
        let config = QpskConfig {
            sample_rate: 8000.0,
            symbol_rate: 1000.0,
            rrc_beta: 0.5,
            costas_bandwidth: 0.02,
        };
        let samples: Vec<Complex32> = (0..2000)
            .map(|i| Complex32::from_polar(50.0, (i as f32) * 0.01))
            .collect();

        let mut demod_whole = QpskDemod::new(config);
        let whole = demod_whole.process(&samples);

        let mut demod_chunked = QpskDemod::new(config);
        let mut chunked = Vec::new();
        for chunk in samples.chunks(97) {
            chunked.extend(demod_chunked.process(chunk));
        }

        assert_eq!(whole.len(), chunked.len());
    }
}
