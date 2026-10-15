# Independent VFO and complex spectrum input report

Native Codex session subagent `/root/radio_rebuild` / inherited GPT-6-family route; exact provider identifier is not exposed. Used only the built-in session harness, with no Morph or agy. Scoped files: `ez-gui/src/spectrum.rs`, `ez-gui/tests/radio_performance.rs` (subsequent test-only delegation), `VFO_DISPLAY_TASK.md`, and this report. Root owns app/config/sidebar integration and final release/render checks.

## Implemented

- Independent public `vfo_freq_hz: Option<u64>` (default `None`) with `vfo_frequency_hz() -> u64`. `None` follows the capture center; `Some` survives capture-center/rate changes. `update_params` continues to describe the source capture, not the VFO.
- VFO passband shading and selected-channel detection now include the target's offset from capture center. USB/LSB keep their one-sided RF passbands. A VFO outside the captured FFT returns the display floor instead of accidentally detecting the nearest edge bin. Spectrum and waterfall show an offset VFO marker; the existing CTR marker stays on capture center. The band identification follows the tuned VFO.
- Spectrum/waterfall clicks, bookmarks, markers, axes, and exported frequencies remain capture-based. A shared plot-position mapping covers click/context-menu actions. Ctrl-scroll changes VFO width without changing either frequency. Corrected visible-peak tuning/info so zooming selects an actual bin from the visible range rather than remapping the strongest full-band bin into the zoomed span.
- `update_params_exact(center_hz: u64, sample_rate: f64)` preserves fractional rates after decimation. The old u32 API forwards to it. FFT cadence, elapsed frame time, bin/plot frequencies and exports use the exact rate. Nonfinite/nonpositive rates fall back to 1 Hz.
- `push_complex_samples(&[num_complex::Complex32])` consumes normalized float IQ without clipping or requantizing. A bin-centered tone with magnitude 1 is 0 dBFS. Complex and raw-byte inputs share ring accumulation, calibrated FFT/windowing, cadence, reusable buffers, the four-transform-per-call budget and newest-window behavior. Nonfinite float samples become zero without losing sample-clock positions; switching to complex input discards an incomplete raw byte pair.
- Public `full_waterfall_update: bool` (default false), matching `DisplaySettings.full_waterfall_update`, and a Full Waterfall Update menu checkbox. Enabled uploads the entire existing bounded texture when real rows change; disabled uploads changed rows. Neither mode uploads/repeats stale rows merely because the UI repaints.

## Integration contract

```rust
spectrum.update_params_exact(capture_center_hz, effective_sample_rate);
spectrum.vfo_freq_hz = Some(tuned_frequency_hz);
spectrum.push_complex_samples(corrected_decimated_capture_iq);
spectrum.full_waterfall_update = configured_full_update;
// spectrum.display_settings().full_waterfall_update reflects menu changes.
```

Input to the spectrum remains centered on the capture RF center, before the demodulator's VFO translation. Existing `update_params(u64, u32)` and `push_iq_samples(&[u8])` callers remain supported. Root must persist the new full-update field and supply VFO/capture metadata before display/detection.

## Verification

`cargo test -p ez-gui --no-default-features --lib spectrum::tests -- --nocapture`: **57 passed, 0 failed**, with 528 unrelated library tests filtered out. This includes all 50 previous tests and seven new regressions:

1. A real 65,536-sample complex FFT retains a −80 dBFS tone below ADC-byte resolution, exact fractional rate, frequency and sample-clock timing.
2. Arbitrary complex chunks match the raw-byte path's calibration, cadence and live ring contents.
3. Oversized complex bursts retain the newest tone while respecting the four-transform budget and fixed buffers.
4. Nonfinite samples do not poison FFT output or combine with a pending raw byte.
5. An offset VFO detects its weaker in-band tone, rejects stronger unrelated tones/out-of-capture channels, and preserves sidedness/frequency under capture retuning.
6. Actual emitted egui passband/CTR coordinates and pointer click events use the proper capture/VFO positions; width changes and zoomed peak mapping retain the expected frequencies.
7. Actual egui texture deltas switch between changed-row and full-texture uploads, with no repeated stale-row upload on redraw.

`rustfmt` passes. These are software signal/headless UI checks; native desktop interaction, physical RF, audio output, final screenshot inspection and broad application integration remain root verification tasks.

## Complete Radio pipeline verification

The updated `radio_performance.rs` exercises RadioIqProcessor (DC correction and filtered source decimation) → capture-centered float spectrum → nonzero-offset VfoMixer → exact-rate complex Demodulator. It never requantizes the intermediate IQ.

The nonignored two-carrier AM integration test passed via `cargo test -p ez-gui --test radio_performance --no-default-features --offline -- --nocapture`: **1 passed, 0 failed, 1 ignored throughput test**. A wanted carrier at capture +18 kHz has 1 kHz modulation; a stronger neighbor at capture −22 kHz has 2.3 kHz modulation. Two independent mixers select each carrier from the same preprocessed float stream. Source rate 768,003 Hz yields exact effective rates 768,003 Hz and 96,000.375 Hz for decimation 1/8. Both yield 12,000 audio frames ±1 for the quarter-second fixture.

Measured settled tone amplitudes:

| Decimation | Selected 1 kHz | Unwanted 2.3 kHz | Neighbor 2.3 kHz | Unwanted 1 kHz |
| --- | ---: | ---: | ---: | ---: |
| 1 | 0.10768 | 0.00001 | 0.14703 | 0.00003 |
| 8 | 0.10766 | 0.00002 | 0.14704 | 0.00002 |

The same test verifies that the spectrum's strongest carrier remains at the neighbor's original RF frequency and the selected VFO detector still sees its wanted signal. Source chunks have odd byte counts to exercise pairing continuity.

The ignored throughput test now covers AM, WFM mono, CW, and WFM stereo at decimation 1 and 8, with actual synthetic signals including an FM stereo pilot, +18 kHz VFO offset, and DC correction. Each case processes exactly one source second (2,400,003 IQ pairs); effective rates are 2,400,003 Hz and 300,000.375 Hz. It requires exactly 48,000 output audio frames ±1 and the correct decimated pair count. Timing includes source IQ processing, 65,536-bin FFT at 20 Hz, mixer and demodulator; fixture generation is excluded.

`cargo test -p ez-gui --test radio_performance --release --features rtlsdr --offline -- --ignored --nocapture` passed all eight cases in one throughput test. Each produced **47,999 audio frames**, satisfying the ±1 frame clock requirement. Both WFM stereo cases acquired pilot lock.

| Mode | Decimation | Effective rate (Hz) | Processing time | Realtime multiple |
| --- | ---: | ---: | ---: | ---: |
| AM | 1 | 2,400,003 | 84 ms | 11.9× |
| AM | 8 | 300,000.375 | 73 ms | 13.8× |
| WFM mono | 1 | 2,400,003 | 122 ms | 8.2× |
| WFM mono | 8 | 300,000.375 | 79 ms | 12.7× |
| CW | 1 | 2,400,003 | 84 ms | 11.9× |
| CW | 8 | 300,000.375 | 71 ms | 14.1× |
| WFM stereo | 1 | 2,400,003 | 182 ms | 5.5× |
| WFM stereo | 8 | 300,000.375 | 109 ms | 9.2× |

These measurements include the complete new float pipeline and supersede the older raw-byte demodulation/FFT-only benchmark for this task. They do not measure native rendering, USB, speaker callbacks, audible reception or end-to-end application latency. `rustfmt` and the scoped diff check pass for the updated test.
