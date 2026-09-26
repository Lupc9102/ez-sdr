# Local DSP parity implementation

Agent: native Codex session subagent `/root/integration_review`, inherited GPT-6 route; exact provider identifier unavailable. No Morph or agy. Date: 2026-09-23.

## Implemented

- Added DSB and CW modes with persisted labels and mode-specific RF channel defaults. WFM defaults to the installed reference's 150 kHz RF width.
- Added `Demodulator::set_rf_bandwidth(f32)` independently of `set_lpf_cutoff`, and `set_cw_tone(f32)` with a 100–2000 Hz range and 700 Hz default. RF width zero selects the mode default.
- Replaced the weak RF filter with a cached sixth-order Butterworth complex filter. Double-precision state supports narrow CW filtering at MHz source clocks.
- DSB uses signed coherent product detection; CW filters around the RF carrier before mixing it to the audible beat frequency.
- Corrected USB/LSB selection to filter the selected sideband and restore its translation, preserving received voice pitch.
- Added continuous fractional audio resampling after integer decimation, correcting output duration and pitch at source rates such as 2.048 MHz.
- Repaired optional RF processing: anti-aliasing and persistent phase before decimation, correct reduced RF clock, continuous DC estimation, per-sample noise blanking, and correct notch history across chunks.

## Evidence

Final full-crate focused verification passed: `cargo test -p ez-gui --no-default-features demod::tests --lib` (**42 passed**, including nine subsequent WFM stereo tests) and `cargo test -p ez-gui --no-default-features sdr_panel::tests --lib` (**31 passed**). Scoped `rustfmt --check` and `git diff --check` are clean. The earlier temporary audio-API integration blocker has been resolved.

New waveform regressions verify DSB fundamental recovery without frequency doubling; independent RF versus audio rejection; configurable CW beat frequency and signed carrier offsets; adjacent CW carrier rejection below 1% of wanted amplitude; USB/LSB opposite-sideband rejection and pitch; exact audio duration; sample-exact whole-buffer versus fragmented-stream output; RF alias rejection; optional processing state across chunks; and RF-decimated CW pitch/duration.

An existing weak WFM de-emphasis transient check was replaced with a real 10 kHz FM tone attenuation regression; its incorrect explanatory coefficient values were corrected. The subsequent stereo extension is documented separately in `STEREO_PARITY_SUMMARY.md`.

## Integration and limits

Root owns the controls and persistence; source owner suppresses unsupported DSB/CW daemon subscriptions. The local DSP API does not extend the daemon protocol. The DC-removal option intentionally removes a carrier exactly at DC, including centered CW; it remains optional. These are synthesized signal tests, not measured reception or physical audio-device verification.
