# Floating-point radio IQ and VFO task

## Target
Implement a streaming, floating-point local Radio input path in `ez-gui/src/radio_iq.rs`, plus precision-preserving complex mono/stereo entry points in `ez-gui/src/demod.rs`. Provide documented DC removal, spectrum inversion, antialiased power-of-two decimation, and an independent continuous digital VFO mixer. Preserve existing byte APIs and all prior DSP/stereo behavior. Root owns module exports and application integration; raw recording and ADS-B retain original bytes and rate. Add no runtime dependencies.

## Tasklist
- [x] Read the continuation tracker and session report; record ownership — native Codex session subagent `/root/integration_review`, inherited GPT-6 route
- [x] Agree the public radio IQ and VFO API with root — native Codex session subagent `/root/integration_review`, inherited GPT-6 route
- [x] Add floating-point mono/stereo demodulation entry points without requantization — native Codex session subagent `/root/integration_review`, inherited GPT-6 route
- [x] Implement streaming radio IQ conversion/correction/inversion/filtered decimation and independent mixer — native Codex session subagent `/root/integration_review`, inherited GPT-6 route
- [x] Verify signal behavior, counts, precision, chunk continuity, and previous DSP/stereo regressions — native Codex session subagent `/root/integration_review`, inherited GPT-6 route
- [x] Complete standalone summary and hand off integration notes — native Codex session subagent `/root/integration_review`, inherited GPT-6 route

## Tips
- Pickup 2026-09-23: native Codex session subagent `/root/integration_review`, inherited GPT-6 route; exact provider/model identifier is not exposed. User requires built-in Codex/Astra session delegation and forbids Morph/agy.
- Own only `demod.rs`, new `radio_iq.rs`, this tracker, and `IQ_TUNING_SUMMARY.md`; preserve the dirty worktree and other agents' edits.
- The installed SDR++ `iqCorrection` label does not establish an algorithm. Document this implementation as streaming DC removal; do not claim calibrated I/Q gain or phase correction.
- Root will disable legacy demodulator RF DC removal and RF decimation on the integrated source-processed path to avoid applying them twice.
- Final verification: 45 demodulation tests and 9 radio IQ tests pass; touched Rust files pass rustfmt and whitespace checks. Code frozen for root's integrated review/release validation.
- `RadioIqProcessor::configure` returns whether settings changed and clears stream state only on change. `reset()` is for source restart/retune. Ordinary buffers preserve half-byte pairing, DC/FIR memory and decimation phase.
- Use `output_rate()` directly with `set_sample_rates_exact`; rounding an odd source rate divided by a power of two would introduce clock drift. `VfoMixer` offset is target minus center and its oscillator persists through reconfiguration.
- See `IQ_TUNING_SUMMARY.md` for final public APIs, alias-transition limits and validation boundaries.
