# Floating-point Radio IQ and digital VFO completion

Implemented and verified in `ez-gui/src/radio_iq.rs` and `ez-gui/src/demod.rs`. Code is frozen for root's integration review. Harness: native Codex session subagent `/root/integration_review`, inherited GPT-6 route; exact model/provider route is not exposed. No Morph or agy was used. Existing unrelated work was preserved.

## Completed behavior

- Byte input is normalized once into `num_complex::Complex32`, preserving a trailing I byte across arbitrary buffer boundaries. Processed IQ is never requantized to bytes.
- Source-side "IQ correction" is explicitly a continuous 5 Hz I/Q DC estimator/subtractor. It removes a carrier exactly at DC and does not claim gain/phase imbalance calibration or literal SDR++ algorithm equivalence.
- IQ inversion conjugates the complex signal, reversing the spectrum sign.
- Power-of-two decimation uses cascaded 63-tap linear-phase Blackman half-band FIRs. Symmetry, zero taps and evaluation only at retained samples limit CPU work. Factors range from 1 to 1024; malformed factors round down to a power of two and cannot reduce the nominal clock below 1 sample/second.
- For N complete input pairs, decimation produces exactly floor(N/factor) complex samples, including startup transients. FIR memory, decimation phase and DC state persist across buffers.
- The separate VFO multiplies by exp(-j 2π offset t), with `offset = target RF - capture center RF`. It retains oscillator phase across chunks and VFO changes and periodically normalizes its double-precision oscillator. Positive and negative target offsets both move the target carrier to DC.
- Mono and stereo demodulation accept complex floats directly. The old public byte APIs remain wrappers. Legacy optional RF DC, notch, blanker and decimation processing now operates entirely in floats; root disables legacy DC/decimation on the source-processed app path.
- Source, filter, stereo and resampler clocks retain f64 precision. The old integer sample-rate setter delegates to the exact-rate setter, preventing fractional-clock loss when source sample rates are not divisible by decimation factors.

## Public integration APIs

```rust
RadioIqConfig { input_rate: u32, dc_remove: bool, invert: bool, decimation: u32 }
RadioIqConfig::normalized(self) -> Self
RadioIqProcessor::new(config: RadioIqConfig) -> Self
RadioIqProcessor::configure(&mut self, config: RadioIqConfig) -> bool
RadioIqProcessor::config(&self) -> RadioIqConfig
RadioIqProcessor::output_rate(&self) -> f64
RadioIqProcessor::reset(&mut self)
RadioIqProcessor::process(&mut self, bytes: &[u8]) -> Vec<Complex32>

VfoMixer::new(sample_rate: f64) -> Self
VfoMixer::configure(&mut self, sample_rate: f64, offset_hz: f64)
VfoMixer::reset(&mut self)
VfoMixer::process(&mut self, samples: &[Complex32]) -> Vec<Complex32>

Demodulator::set_sample_rates_exact(&mut self, input_rate: f64, audio_rate: u32)
Demodulator::demodulate_complex(&mut self, iq: &[Complex32], mode: DemodMode) -> Vec<f32>
Demodulator::demodulate_stereo_complex(&mut self, iq: &[Complex32], mode: DemodMode) -> Vec<[f32; 2]>
```

The spectrum consumes processor output before the VFO; demodulation consumes mixer output. Pass the exact processor rate to both mixer and demodulation. ADS-B and raw recording keep original bytes/rate. Root owns module export, app/config/UI integration, tune limits and restart/drain handling.

## Verification

- `cargo test -p ez-gui --no-default-features --offline demod::tests --lib` — **45 passed**, including all preceding 42 DSP/stereo regressions.
- `cargo test -p ez-gui --no-default-features --offline radio_iq::tests --lib` — **9 passed**.
- `rustfmt --edition 2021 --check ez-gui/src/demod.rs ez-gui/src/radio_iq.rs` — passed.
- `git diff --check -- ez-gui/src/demod.rs ez-gui/src/radio_iq.rs IQ_TUNING_TASK.md` — passed.

The new tests verify exact byte/complex API equivalence for all modes and stereo; retention of a 1e-5-amplitude tone through optional RF decimation; exactly 176,400 audio samples over four seconds at 44.1 kHz from 48,000.75 Hz IQ; exact chunk continuity; DC removal with a preserved adjacent tone; inversion and VFO signs; phase-preserving VFO changes; odd byte boundaries; exact sample counts for every power-of-two factor through 1024; and at least 49 dB rejection of channels that would alias into the passband at factors 2 through 64, including early-stage aliases.

## Limits

Each half-band stage has an antialias transition near the edge: its useful flat passband spans approximately 80% of output Nyquist bandwidth. Do not interpret the outer transition as a flat full-width passband. No finite FIR eliminates all aliases at the exact Nyquist edge. State resets produce ordinary FIR/DC startup transients; output is not padded or flushed.

This assignment verifies synthetic signal behavior and compilation of the GUI library. Root owns the integrated broad suite, release build, updated throughput measurements, UI checks and hardware/off-air/audio validation. Earlier release/throughput measurements predate this new source IQ path and are not claimed as measurements of it.
