# Radio DSP control implementation

## Target
Implement real AM carrier AGC, WFM audio-lowpass bypass with retained antialias safety, and FM-specific IF noise reduction in `ez-gui/src/demod.rs`. Coordinate installed SDR++ semantics with the reference auditor before claiming equivalence. Preserve float/byte/stereo APIs and the previous 45 demodulation tests. Provide a raw NFM subaudible audio tap for the separate CTCSS detector before user audio processing. Root owns settings/UI/application wiring; another agent owns the detector.

## Tasklist
- [x] Read the current control tracker and preceding DSP/IQ reports; establish ownership — native Codex session subagent `/root/integration_review`, inherited GPT-6 route
- [x] Establish reference evidence and agree precise public integration APIs — native Codex session subagent `/root/integration_review`, inherited GPT-6 route
- [x] Implement separate AM carrier AGC and WFM audio-lowpass bypass with antialias filtering — native Codex session subagent `/root/integration_review`, inherited GPT-6 route
- [x] Implement justified FM IF noise reduction using verified reference semantics where available — native Codex session subagent `/root/integration_review`, inherited GPT-6 route
- [x] Add a streaming NFM discriminator tap at the configured audio output rate — native Codex session subagent `/root/integration_review`, inherited GPT-6 route
- [x] Add reference-style RF blanker level and optional raw WFM multiplex tap for RDS — native Codex session subagent `/root/integration_review`, inherited GPT-6 route
- [x] Verify signal behavior, state/reset/chunk continuity, and all preceding DSP/stereo tests — native Codex session subagent `/root/integration_review`, inherited GPT-6 route
- [x] Complete standalone report and hand off frozen code — native Codex session subagent `/root/integration_review`, inherited GPT-6 route

## Tips
- Pickup 2026-09-23: native Codex session subagent `/root/integration_review`, inherited GPT-6 route; exact provider/model route unavailable. Built-in session agents only; no Morph or agy.
- Own `demod.rs` only plus this tracker/report; coordinate any helper-file ownership first. Preserve other agents' dirty changes.
- Reference auditor `/root/radio_rebuild` is extracting installed SDR++ control semantics, labels and ranges. Do not infer identical algorithms from symbols alone.
- Detector agent `/root/meteor_completion` owns `radio_squelch`; provide a pre-audio-processing NFM tap without duplicating CTCSS detection in demodulation.
- Existing mono/stereo APIs and exact f64 rate handling are documented in `IQ_TUNING_SUMMARY.md`. The source-level IQ processor already handles DC/inversion/decimation in the app path.

- Installed evidence recovered by reference auditor: carrier AGC selects pre-envelope versus post-envelope gain; attack/decay rates50/5 (CW100/5), ranges1..200/1..20. FM Low Pass applies NFM and WFM. FMIF uses Nuttall sliding FFT peak-bin inverse reconstruction, NFM9/15/31 bins and WFM32 bins at50k/250k IF. RF blanker ratio range1..10, default1, mean coefficient500/IF rate. Numerical bitwise reference equivalence is not claimed.
- Final verification: all 55 demodulation tests pass (45 preceding plus 10 new regressions), rustfmt check and scoped whitespace check pass. Code frozen for root's integration/release/performance validation; see `RADIO_DSP_CONTROLS_SUMMARY.md`.
- Added source-compatible APIs for the independent CTCSS and RDS agents. CTCSS raw tap is honestly NFM-only and matches final audio counts; RDS tap is WFM mono/stereo before audio filtering and only collects when enabled.
