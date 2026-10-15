# Local demodulator parity

## Target
Implement functioning DSB and CW demodulation, including configurable CW beat tone, and independent local RF channel bandwidth separate from the audio low-pass filter. Own only `ez-gui/src/demod.rs`, `ez-gui/src/sdr_panel.rs`, this tracker, and `DSP_PARITY_SUMMARY.md`. Verify synthesized signal recovery, adjacent-channel rejection, and continuous state across chunks. Preserve the existing daemon protocol.

## Tasklist
- [x] Read continuation trackers and inspect DSP/mode integration — native Codex session subagent `/root/integration_review` / inherited GPT-6 route
- [x] Communicate mode/API changes and external exhaustive-match updates — native Codex session subagent `/root/integration_review` / inherited GPT-6 route
- [x] Implement independent RF channel filtering and mode defaults — native Codex session subagent `/root/integration_review` / inherited GPT-6 route
- [x] Implement DSB product detection and CW beat-frequency detection — native Codex session subagent `/root/integration_review` / inherited GPT-6 route
- [x] Add meaningful tone, rejection, and chunk-continuity regressions — native Codex session subagent `/root/integration_review` / inherited GPT-6 route
- [x] Repair optional RF decimation and DC-correction stream state — native Codex session subagent `/root/integration_review` / inherited GPT-6 route; isolated 64-test DSP/mode suite passed
- [x] Run focused checks and write standalone summary — native Codex session subagent `/root/integration_review` / inherited GPT-6 route; final cargo runs passed 42 DSP + 31 mode tests, rustfmt and diff checks clean

## Tips
- Pickup 2026-09-23: native Codex session subagent `/root/integration_review`, inherited GPT-6 route; exact provider/model identifier is not exposed. No Morph or agy. Read `RADIO_PARITY_TASK.md` and `UI_REBUILD_TASK.md`.
- Root owns app/radio UI/config. Source owner handles daemon exhaustive match; DSB/CW are local-only until protocol support is implemented. Existing audio LPF currently doubles as AM/NFM RF filter cutoff; separate the two.
- Planned APIs: `set_rf_bandwidth(f32)`, `set_cw_tone(f32)`, `DemodMode::{Dsb,Cw}`, `DemodMode::default_rf_bandwidth_hz()`. RF bandwidth is a full centered span except USB/LSB, where it is the selected one-sided span.
- Implemented cached sixth-order Butterworth complex filtering, signed DSB detection, centered narrow CW filtering before configurable BFO, corrected SSB sideband selection/pitch, and persistent fractional resampling to the exact output clock. Initial pre-existing 24 DSP tests pass; new signal tests are running.
- Optional RF processing now filters before decimation, preserves sample phase, tracks the reduced RF clock, estimates DC continuously, applies blanking per sample, and restores notch I/Q history correctly. 33 DSP + 31 mode tests passed through a standalone rustc harness importing the actual owned source files. Full-crate rerun currently awaits another agent's audio-device API; no full-build success is implied.
- Final verification supersedes the temporary integration blocker: full-crate focused `cargo test -p ez-gui --no-default-features demod::tests --lib` passed 42 tests (including the subsequent nine stereo tests), and `sdr_panel::tests` passed all 31 tests. `rustfmt --check` and scoped `git diff --check` pass. DSP code frozen and released to root for broader validation.
