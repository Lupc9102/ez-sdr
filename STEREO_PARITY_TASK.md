# WFM stereo multiplex decoding

## Target
Add real local WFM stereo decoding while preserving the existing mono demodulator API. Recover the 19 kHz pilot with a PLL, demodulate the 38 kHz difference channel, filter and de-emphasize each channel, and produce exact-rate stereo frames with smooth mono fallback. Preserve functional audio controls. Root owns UI, persistence, recorder/downmix, and interleaved audio transport. No daemon protocol change.

## Tasklist
- [x] Agree API and integration contract with root — native Codex session subagent `/root/integration_review` / inherited GPT-6 route
- [x] Implement multiplex filtering, pilot PLL, stereo separation, and mono fallback — native Codex session subagent `/root/integration_review` / inherited GPT-6 route
- [x] Apply independent audio filtering/de-emphasis/resampling and existing post-processing — native Codex session subagent `/root/integration_review` / inherited GPT-6 route
- [x] Add real multiplex waveform tests for separation, fallback, offset/phase and chunk behavior — native Codex session subagent `/root/integration_review` / inherited GPT-6 route; four initial waveform tests pass
- [x] Complete focused validation and standalone summary — native Codex session subagent `/root/integration_review` / inherited GPT-6 route; all nine stereo waveform regressions pass within 42 DSP tests, plus 31 mode tests

## Tips
- Pickup 2026-09-23: native Codex session subagent `/root/integration_review`, inherited GPT-6 route; exact provider identifier unavailable. No Morph or agy. Preserve earlier `DSP_PARITY_TASK.md` and `DSP_PARITY_SUMMARY.md`.
- API: `demodulate_stereo(&[u8], DemodMode) -> Vec<[f32; 2]>`, plus `last_stereo_locked: bool`. Root calls this only for local WFM when enabled; existing `demodulate` stays mono. Toggle defaults off. Reset on changing channel mode before switching audio queue format.
- Full-crate compilation currently waits for separate audio-device APIs; standalone rustc harness can compile and test actual DSP/mode source files without that dependency.
- Final cargo verification succeeds after audio APIs landed: 42/42 DSP and 31/31 mode tests. Nine stereo regressions cover channel separation/phase/frequency error, absent/weak/lost pilot, existing audio controls, linked AGC, chunk invariance, exact 48/44.1 kHz output, low effective IQ rate fallback, and mono-toggle loudness. Code frozen; root owns broader GUI/release/throughput verification.
