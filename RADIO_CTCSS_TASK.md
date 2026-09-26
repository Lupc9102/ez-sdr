# Streaming CTCSS squelch

## Target

Implement a real deterministic streaming CTCSS detector and smooth tone gate for local NFM. Consume the pre-volume/pre-audio-filter discriminator tap at its exact audio rate, keep state independent from power squelch, expose selected and detected tones separately, and preserve output across arbitrary chunk boundaries. Use bounded detector history, the installed reference modes/tone list, and meaningful adjacent-tone/noise/speech/timing/rate tests. Root owns module export/config/UI/app integration; the demod agent owns the raw tap.

## Tasklist

- [x] Read root tracker, coordinate reference controls and exact-rate raw tap, propose API — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route, exact provider ID unavailable
- [x] Implement `radio_squelch.rs` mode enum, standard tones, streaming detector and gate — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route
- [x] Verify signal selectivity, confidence/hang/release, chunk invariance, and 44.1/48 kHz operation — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route; all 14 tests pass in the ez-gui library build and independent offline harness; audio+RTL feature check and formatting check passed
- [x] Report API, measured behavior, limitations and integration handoff — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route; `RADIO_CTCSS_SUMMARY.md`

## Tips

- Pickup 2026-09-23: native Codex session subagent `/root/meteor_completion`; inherited GPT-6-family route, exact provider route unavailable. Built-in session agents only, no Morph/agy. Own only new `ez-gui/src/radio_squelch.rs` and this scoped tracker/report pair.
- Root tracker: `RADIO_CONTROLS_TASK.md`. Installed auditor confirmed Off/off, Power/power, CTCSS (Mute)/ctcss_mute, CTCSS (Decode Only)/ctcss_decode, CTCSS Tone, Received Tone and Any. Exact installed tone table is being checked.
- Demod integration provides `take_nfm_subaudible_audio() -> Vec<f32>` at exact configured audio rate and identical count to the corresponding audible NFM samples. Tap precedes user LPF/HPF/DC/notch/gain/AGC/pitch/volume; phase-difference units demand relative confidence thresholds.
- Proposed detector API: `CtcssSquelch::process(raw, rate, selected_tone: Option<f32>) -> Vec<f32>` returns one smooth gate gain per input sample, with `None` meaning Any. Decode-only should ignore returned gains and display status. Reset on stream generation changes.
- Auditor recovered exactly 51 installed tones, including 150.0 next to 151.4 Hz. Implemented that table with normalized Goertzel correlations over overlapping 500 ms Hann windows, 40 ms decisions, two-window acquisition, 160 ms hang, and 5/10 ms gain ramps. This is an independent implementation, not a claim of matching the binary's algorithm.
- Added a 0.25% coherent-energy requirement relative to the pre-lowpass discriminator signal. Without this relative check, a tiny antialias stopband residual from a strong voice-band sinusoid could receive misleadingly high normalized narrowband confidence. Regression tests verify those aliases stay closed while weak actual CTCSS beneath stronger voice remains detectable.
