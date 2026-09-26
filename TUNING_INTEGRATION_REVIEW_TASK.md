# Tuning integration review task

## Target
Read-only review of the new local Radio IQ and independent VFO integration in `ez-gui/src/app.rs`, `radio_ui.rs`, `advanced_panel.rs`, and `config.rs`. Identify concrete new defects in tuning/lifecycle, sample-rate handling, raw ADS-B/recording preservation, and daemon behavior. Root owns source changes and is separately fixing recording metadata and running the GUI suite.

## Tasklist
- [x] Record review scope, ownership and active identity — native Codex session subagent `/root/integration_review`, inherited GPT-6 route
- [x] Inspect tuning and processing changes with surrounding lifecycle context — native Codex session subagent `/root/integration_review`, inherited GPT-6 route
- [x] Validate candidate defects and send concrete findings to root promptly — native Codex session subagent `/root/integration_review`, inherited GPT-6 route
- [x] Write `TUNING_INTEGRATION_REVIEW.md` with findings and limits — native Codex session subagent `/root/integration_review`, inherited GPT-6 route

- [x] Reread final fixes and run focused Replay/generation regressions (3 passed) — native Codex session subagent `/root/integration_review`, inherited GPT-6 route

## Tips
- Pickup 2026-09-23: native Codex session subagent `/root/integration_review`, inherited GPT-6 route. Exact provider/model identifier is unavailable. Explicit Astra dispatch was attempted by root and failed with routing/validation errors; this review uses the existing built-in session agent. No Morph or agy.
- Do not edit source files. Root may concurrently update recording metadata. Review actual current files and distinguish new tuning regressions from pre-existing limitations.
- Prior DSP/IQ component work is complete; `IQ_TUNING_SUMMARY.md` contains the tested API contract, exact-rate/phase behavior, and FIR transition limits.
- Review complete: all reported EOF, Replay-center, Settings rate/gain, same-settings restart and remote-gain issues are verified fixed in final source. Replay regression and two local generation tests pass. Remote gain was explicitly reclassified as pre-existing rather than an introduced regression. No source edits or hardware/network execution in this review.
