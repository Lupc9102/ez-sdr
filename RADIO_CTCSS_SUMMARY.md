# Streaming CTCSS squelch completion

Harness/model: native Codex session subagent `/root/meteor_completion`; inherited GPT-6-family route, exact provider ID unavailable. Used only built-in session collaboration; no Morph or agy. Scope: new `ez-gui/src/radio_squelch.rs` and this scoped tracker/report pair.

## API and integration

- `SquelchMode::{Off, Power, CtcssMute, CtcssDecode}` defaults to Off and serializes as `off`, `power`, `ctcss_mute`, `ctcss_decode`.
- `CTCSS_TONES` contains the exact 51 entries extracted by the reference auditor from the installed module, including 150.0 and 151.4 Hz. Any is represented by `None` rather than a fake tone.
- `CtcssSquelch::new()` / `reset()` create or clear the streaming state and close the gate.
- `process(raw_discriminator, sample_rate_hz, selected_tone_hz: Option<f32>) -> Vec<f32>` returns exactly one gate gain in 0..=1 for each input sample. State persists across arbitrary block boundaries. Sample-rate/selection changes reset state; invalid choices fail closed.
- `status() -> CtcssStatus` exposes independent `selected_tone_hz`, `detected_tone_hz`, `confidence`, and logical `gate_open`. Confidence is normalized coherent candidate energy, not a calibrated probability; a candidate can still fail other acceptance checks.

Root integrates the module export, saved controls and mode applicability. The demod agent supplies `take_nfm_subaudible_audio()` at the exact output audio rate with the same length as the audible block. Feed that pre-volume/pre-user-filter discriminator tap to this module. CTCSS Mute multiplies corresponding audible samples by the returned envelope; Decode Only runs detection and ignores the envelope. Power squelch remains independent. Reset CTCSS on source stream-generation or demodulation-mode changes. Current tested integration scope is local NFM.

## Processing behavior

A 25 Hz DC blocker and sixth-order 300 Hz lowpass precede decimation to approximately 1.2 kHz. A bounded 500 ms history is analyzed every 40 ms using Hann-windowed Goertzel correlations for all 51 tones. Detection requires sufficient coherent subaudible energy, dominance over the next candidate, and at least 0.25% coherent energy relative to the pre-lowpass discriminator signal. The last requirement prevents strong voice-band signals from being misidentified through tiny antialias residuals, while preserving amplitude-scale independence.

Two consecutive qualified analyses acquire a tone in approximately 540 ms. Separate acquisition/hold confidence thresholds prevent chattering. A selected tone or Any can authorize audio; another correctly decoded standard tone remains visible without opening a mismatched selection. Authorization has 160 ms hang and a 5 ms opening/10 ms closing gain ramp. Loss response includes the analysis-window response, so the hang setting alone is not the total release delay. No audio is delayed or replayed to recover the initially muted acquisition interval.

Internal analysis storage is bounded: four buffers total at most 2,800 f64 samples plus 51 coefficients over supported 8–384 kHz audio rates. The returned gain vector scales with the caller's current block. Nonfinite input samples are treated as silence. No absolute receiver power or playback volume threshold participates in tone decisions.

## Verification

`cargo test -p ez-gui --offline --no-default-features radio_squelch::tests --lib`: **14 passed**. `cargo check -p ez-gui --offline --features 'audio rtlsdr'` and `rustfmt --check` passed. A temporary independent offline Cargo harness also passed all tests while parallel app integration was initially incomplete.

The regressions cover:

- Every installed tone at 44.1 and 48 kHz, requiring correct labels, open gates and greater than 98% coherent confidence for clean tones.
- Correct decoding but closed authorization for adjacent wrong tones, including both directions of the 150.0/151.4 pair.
- Any selection; small oscillator errors; rejection of the ambiguous midpoint between the close pair.
- Silence, DC, deterministic broadband noise and moving speech-like fundamentals/harmonics; a selected tone mixed with voice/noise/DC; and a 0.015-amplitude tone beneath 0.20/0.15-amplitude voice-band components.
- Rejection of voice-band sinusoids that would alias exactly onto supported tones after decimation.
- Amplitude-scale invariance across a 10,000-fold change and rejection of two equal competing tones.
- Acquisition confirmation, a 100 ms dropout without gate chatter, release within 200–600 ms after tone cessation, and a monotonic approximately 10 ms closing ramp.
- Bit-for-bit identical gain streams and final status under irregular chunk sizes, including one-sample chunks, at both requested rates.
- Reset/rate/selection changes, unsupported configuration, and nonfinite data.

These are synthetic signal and compilation checks. Live RF, physical audio and equivalence to the reference module's internal algorithm have not been established. A stable audio component indistinguishable from a valid CTCSS tone remains indistinguishable from signaling; the detector does not claim semantic voice recognition.
