# Radio DSP controls completed

Implemented in `ez-gui/src/demod.rs`; code frozen for integration. Agent identity: native Codex session subagent `/root/integration_review`, inherited GPT-6 route; exact provider/model identifier unavailable. No Morph/agy, no runtime dependencies added, and unrelated dirty changes preserved.

## Functional controls

- **AM carrier AGC:** selects a separate RF-magnitude envelope estimator and complex gain before AM envelope detection. Post-envelope audio AGC is skipped in this mode, preserving modulation-depth information while compensating RF carrier strength. Disabling Carrier AGC retains post-envelope audio AGC. The Advanced `agc_enabled` switch remains the master enable.
- **Attack/Decay:** new rate-independent envelope AGC parameters use reference ranges 1–200 and 1–20 inverse seconds. They update at the actual IQ clock for carrier AGC and actual intermediate audio clock for audio AGC. The reference auditor established defaults 50/5, with CW 100/5. Legacy coefficient setters remain compatible and select the preceding gain-servo behavior; root should apply the new rate setter after legacy setters when restoring a mode profile.
- **FM Low Pass:** a real bypass of the user audio LPF in NFM and WFM mono/stereo. A separate sixth-order post-discriminator antialias filter remains active before integer audio decimation and fractional resampling. Stereo's multiplex/channel-separation filters remain active. The bypass therefore does not route raw high-frequency multiplex components straight into aliased audio.
- **FM de-emphasis:** zero is a true None setting; 22, 50 and 75 microseconds operate in NFM and WFM, including both stereo channels.
- **FM IF Noise Reduction:** implements the installed algorithm recovered by the reference auditor: for each sliding complex IF window, apply the four-term Nuttall window, transform, retain the largest-magnitude FFT bin, and reconstruct the inverse transform's center sample. The inverse center is evaluated analytically from that one bin, avoiding a redundant full inverse FFT. NFM presets use NOAA APT 9 bins, Voice 15 bins, Narrow Band 31 bins; WFM always uses 32 bins and ignores the NFM preset choice.
- **IF clock handling:** antialiased floating-point fractional resampling brings ordinary NFM/WFM channels to the reference 50/250 kHz clocks before sliding FFT processing. Sources below those rates retain their clock. Unusually wide custom RF channels increase the IF clock to preserve their useful bandwidth. Demodulator clock state is scoped and restored so normal per-buffer rate-setting calls do not reset the stream. Discriminator gain is compensated to avoid a loudness jump solely from IF clock changes.
- **RF blanker Level:** parameterizes the RF-stage impulse suppression using the recovered reference ratio law. Level is a linear magnitude-to-mean ratio in 1–10 (default 1); it is not calibrated dB. The mean coefficient is `min(500 / actual RF sample rate, 1)`. Above threshold, the sample is scaled down to the running mean, retaining complex phase. The unrelated audio blanker remains separate.

## Decoder taps and APIs

```rust
set_carrier_agc_enabled(bool)
set_agc_rates(attack_per_second: f32, decay_per_second: f32)
set_fm_lowpass_enabled(bool)
set_fm_if_noise_reduction(enabled: bool, preset: FmIfPreset)
set_rf_noise_blanker_level(f32)
set_deemph_tau(microseconds: f32) // 0 means None

take_nfm_subaudible_audio() -> Vec<f32>
set_rds_tap_enabled(bool)
take_wfm_multiplex() -> (Vec<f32>, f64)
```

`FmIfPreset::{NoaaApt, Voice, NarrowBand}` implements Default, Copy, equality and serde serialization/deserialization, with snake_case saved names and a `label()` helper. Default is Voice; IF NR itself remains disabled by default. The existing byte/float mono/stereo entry points remain available.

The NFM tap preserves subaudible discriminator content before user LPF/HPF/DC/notch/gain/AGC/pitch or root-applied volume. Its independent persistent resampler produces exactly the same per-call count as audible NFM audio at the configured output rate, including when pitch changes audible samples. It uses the legacy discriminator scale; CTCSS confidence should remain relative to signal energy. Each demodulation call replaces the tap, and non-NFM/reset clears it. No CTCSS detector is implemented here.

The optional WFM tap is the raw multiplex before de-emphasis and audio filters, normalized to 75 kHz peak deviation, with its exact f64 rate. It works in mono and stereo and collects only while enabled. With IF NR off it uses the input IQ clock; normal NR-on WFM uses 250 kHz. Non-WFM/reset clears it. No RDS decoder is implemented here.

## Verification

- `cargo test -p ez-gui --no-default-features --offline demod::tests --lib` — **55 passed**, retaining all preceding 45 tests.
- `rustfmt --edition 2021 --check ez-gui/src/demod.rs` — passed.
- `git diff --check -- ez-gui/src/demod.rs RADIO_DSP_CONTROLS_TASK.md` — passed.

The ten added regressions cover:

1. Carrier strength compensation versus post-envelope audio normalization, with modulation-depth preservation.
2. Carrier-AGC physical-time consistency at 96/384 kHz, bitwise chunk continuity and reset equivalence.
3. Real low-pass bypass gain and suppression of a 40 kHz component that would alias to 8 kHz; the test requires more than approximately 34 dB rejection relative to the wanted tone.
4. Distinct None/22/50/75 de-emphasis behavior in NFM and WFM.
5. Exact reference FMIF preset sizes and single-bin inverse-center mathematics.
6. Improved synthetic noisy-NFM tone-to-residual power ratio (more than 25% in the regression) with bounded clean-tone toggle gain.
7. Exact CTCSS tap counts and chunk/reset equality at 44.1 kHz from 192,003 Hz input, with and without IF NR, while audible processing includes HPF, notch, DC block, pitch and zero gain.
8. A preserved 57 kHz RDS component in raw taps from WFM mono/stereo despite audio filtering/gain settings.
9. WFM stereo IF NR clock, audio/tap sample counts, chunk equality and reset equivalence.
10. Actual RF impulse suppression controlled by blanker Level.

## Evidence and limits

The reference auditor recovered algorithm/control evidence from the installed binary, including AM AGC-mode selection, FMIF initialization/run, window coefficients, preset counts, IF clocks and blanker update law. Working disassembly evidence is in `/tmp/sdrpp-radio-annotated.txt`; root/reference reports retain the durable inventory. This implementation does not claim bitwise SDR++ equivalence: the surrounding filters/resampler, bounded streaming AGC gain/limiting, arithmetic and startup details differ.

Strongest-bin FM IF reduction can attenuate rapidly varying modulation, including stereo pilot/RDS content. The tests establish stereo-path continuity with IF NR enabled, not unchanged stereo separation or RDS sensitivity under that setting. CTCSS remains NFM-only in this component. Hardware RF, native audio, real-station RDS and end-to-end detector validation are not claimed.

Root owns per-mode persistent configuration, UI, decoder/gate integration and complete application tests. Updated release throughput should include IF NR enabled, since the previous measurements did not include sliding FFT noise reduction or the new FM antialias stage.
