# Source and tuning parity continuation

## Target
Continue the active native Rust desktop goal by implementing actual independent VFO/center tuning and the missing source IQ controls without losing precision or breaking ADS-B/Meteor. Add direct-sampling I/Q branch selection, close display inventory gaps, and validate real signal behavior, performance and rendered controls. SDR++ exact reference/native comparison remains required and is not assumed from test success.

## Tasklist
- [x] Revalidate current worktree and classify the preceding turn as progress (new DSP/source/audio/display implementation, release and measured verification) — Codex harness / GPT-6 root
- [x] Implement floating-point radio IQ correction/inversion/filtered decimation and digital VFO translation — native Codex session subagent integration_review / inherited GPT-6-family model, exact route not exposed
- [x] Implement tuner-center capture API and direct-sampling branch selection — native Codex session subagent meteor_completion / inherited GPT-6-family model, exact route not exposed
- [x] Implement independent VFO display/interaction, complex FFT input, and remaining display controls — native Codex session subagent radio_rebuild / inherited GPT-6-family model, exact route not exposed
- [x] Integrate normal/center tuning, source settings, recording metadata and stream separation; fix scoped review findings (replay center/EOF, settings restart, stream generation) — Codex harness / GPT-6 root, exact provider route not exposed
- [x] Verify signal behavior, UI, release and throughput; inspect remaining goal gaps — Codex harness / GPT-6 root, exact provider route not exposed; 590 GUI tests, full-path AM integration, eight release throughput cases, software-rendered widgets, feature checks and 19,905,576-byte release
- [x] Update reference inventory and standalone session summary — Codex harness / GPT-6 root, exact provider route not exposed; SDRPP_UI_REFERENCE.md and TUNING_PARITY_SESSION_SUMMARY.md

## Tips
- Stage complete: full original goal stays active. Exact layout/native/hardware comparison and remaining reference controls are explicitly outstanding in TUNING_PARITY_SESSION_SUMMARY.md. Do not mark the original goal achieved from this stage's tests.
- Current pickup: Codex harness / GPT-6 root. Explicit built-in gpt-6-astra spawn was accepted but execution failed403 (provider accepts codex/gpt-6-astra); attempting that exact prefixed name was rejected as an unknown model by spawn_agent. Working built-in inherited agents continue. No Morph/agy was called.
- Pickup 2026-09-23: Codex harness / GPT-6 root; exact provider identifier not exposed. Use only built-in inherited session subagents (explicit Astra override routing was previously inconsistent); no Morph/agy. All three previous subagents are completed and available for new scoped assignments.
- Root owns app.rs/radio_ui.rs/config.rs/advanced_panel.rs and module integration. DSP owner owns demod.rs and a new radio_iq.rs. Source owner owns source_manager.rs. Display owner owns spectrum.rs. Separate component task/report files avoid tracker write conflicts.
- Preserve the extensive existing dirty worktree. Current release is target/release/ez-gui with audio+RTL, 19,908,688 bytes. Previous authoritative report is RADIO_PARITY_SESSION_SUMMARY.md (553 GUI tests pass with eight socket tests excluded).
- SourceManager.frequency_hz currently represents the tuned/displayed frequency everywhere. New physical center must be separate so existing bookmarks/scanner/remote/audio-channel semantics remain logical RF frequency. Root will coordinate exact APIs before integration.
- ADS-B consumes original source IQ/rate, and Meteor remains independent offline decoding. Radio-only DSP controls must not silently change ADS-B input rate or raw recording metadata.
- Native socket/display access and upstream clone escalation were blocked in earlier work. Do not bypass automatic approval review or claim native/hardware evidence from software-rendered widgets.
