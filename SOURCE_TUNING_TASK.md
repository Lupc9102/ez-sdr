# Capture center and source tuning task

## Target

Keep `SourceManager.frequency_hz` as the logical/displayed VFO while introducing an optional independent logical capture center. Local RTL hardware applies a checked signed frequency offset to that capture center, including transverter RF frequencies above 4 GHz. Daemon hardware/spectrum stays on capture center and audio channels retain the VFO through relative retuning. Add selectable direct-sampling I/Q branches without removing the saved-compatible enable boolean. Preserve default center-following behavior, stable USB identity, source queues, replay, and cancellation.

## Tasklist

- [x] Read tuning/reference handoff and send exact root integration API — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route
- [x] Implement center/checked local tuning offset and direct-sampling branch API — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route
- [x] Preserve VFO-vs-center semantics in daemon commands and hardware state updates — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route
- [x] Verify deterministic arithmetic/branch/protocol behavior, existing source tests, and RTL feature compilation — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route; all 39 source tests and audio+RTL feature check passed
- [x] Write standalone source tuning summary with remaining physical validation — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route; `SOURCE_TUNING_SUMMARY.md`
- [x] Make explicit acquisition retuning replace any independent capture center, then verify the regression and report — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route; all 40 source tests and audio+RTL feature check passed
- [x] Add read-only stream generation for local/daemon restarts, preserving idempotence and VFO-only changes — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route; all 43 source tests and audio+RTL feature check passed

## Tips

- Pickup 2026-09-23: native Codex session subagent `/root/meteor_completion`; inherited GPT-6-family route, exact provider ID not exposed. Read `TUNING_PARITY_TASK.md` and `RADIO_PARITY_SESSION_SUMMARY.md`. No Morph or agy.
- Own only `ez-gui/src/source_manager.rs` and this task/summary pair. Root integrates UI/config/app; other subagents own DSP/FFT. Preserve all unrelated dirty work.
- API sent before edits: optional `center_frequency_hz`; `capture_center_frequency_hz()`; signed `frequency_offset_hz`; checked `rtl_center_frequency_hz()`; `DirectSamplingBranch::{I,Q}` default Q and `direct_sampling_branch`; `direct_sampling_mode()` gives 0/1/2. Offset convention sent: physical tuner = logical capture center + signed offset (negative offset subtracts transverter LO).
- No native USB/socket validation is implied by deterministic protocol tests and feature compilation.
- Follow-up pickup 2026-09-23: native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route. Root clarified that `tune_and_restart` explicitly retunes acquisition (including ADS-B), so it must set both VFO and capture center; app reconciliation handles ordinary VFO-only changes.
- Lifecycle pickup 2026-09-23: native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route. Add `stream_generation()` for root's capture signature; increment only when a new local worker or daemon connection attempt starts, after validation, never on stop or idempotent start.
