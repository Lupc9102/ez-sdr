# Ship runtime audit

## Target

Audit and fix concrete runtime defects in launch/source/audio/session flows, bounded to `ez-gui/src/source_manager.rs`, `audio_output.rs`, `audio_resampler.rs`, `recorder_panel.rs`, `daemon_client.rs`, and `ez-daemon/**`. Trace startup, stop/restart/EOF and device errors, audio queue lifecycle, remote source behavior, and save-path failures. Preserve the existing dirty worktree. Root owns `app.rs`, `config.rs`, `radio_ui.rs`, and `main.rs`; communicate required integration fixes instead of editing those files. Add meaningful regressions for reproduced defects and deliver `SHIP_RUNTIME_AUDIT_SUMMARY.md`.

## Tasklist

- [x] Record scope and ownership — native Codex session subagent `/root/integration_review`, inherited GPT-6 route
- [x] Inspect bounded source lifecycle and EOF/error flows — native Codex session subagent `/root/integration_review`, inherited GPT-6 route
- [x] Inspect audio queue/restart and recorder save-path flows — native Codex session subagent `/root/integration_review`, inherited GPT-6 route
- [x] Inspect daemon client and server source/control lifecycle flows — native Codex session subagent `/root/integration_review`, inherited GPT-6 route
- [x] Fix concrete defects in owned files and report root integration changes; file/source failures reproduced, socket reproduction blocked as documented — native Codex session subagent `/root/integration_review`, inherited GPT-6 route
- [x] Run focused meaningful regression tests and formatting checks; 101 focused tests pass, restricted loopback tests compile but cannot execute — native Codex session subagent `/root/integration_review`, inherited GPT-6 route
- [x] Write audit summary and hand off verified changes — native Codex session subagent `/root/integration_review`, inherited GPT-6 route

## Tips

- Pickup 2026-09-23: native Codex session subagent `/root/integration_review`, inherited GPT-6 route. The exact provider/model route is not exposed; built-in session agents only, no Morph/agy.
- Prior completed scope: `RADIO_CONTROLS_INTEGRATION_REVIEW.md`. Three new CTCSS tests, ten enabled-control benchmark cases, eight baseline cases and the AM selection test pass. No owned live process remains from that assignment.
- Root is concurrently integrating app/config/UI changes. Other agents own decoder/build work; do not overwrite their edits. This is a bounded code/flow audit, not project-wide mapping or rewriting.
- Concrete fixes in progress: cancellable daemon handshake/socket sends; actual WAV rate with stop-on-rate-change; collision-safe local save filenames, JSON metadata, visible save/write failures, finalization on drop; safe extreme/nonfinite CF32 replay conversion; daemon recording flush errors and terminal hardware status.
- Root was asked to wire `RecorderPanel::set_audio_sample_rate(self.audio.sample_rate())` before recording controls and after successful audio startup; root acknowledged.
- Recorder focused run: 25 tests pass, `/tmp/ez-sdr-runtime-recorder-tests.log`. Loopback regression is blocked: sandbox bind returns EPERM, and automatic approval review fails with provider route HTTP403 before executing the escalation. Parent informed; no bypass attempted. A socket-free pending-operation cancellation test is available for unaffected validation.
- Final focused counts: recorder26, source44, daemon recording10, ingest5, cancellation1, audio output12, audio resampler3 = 101 passing tests. All owned runtime files are frozen at handoff; no owned live process remains. Root confirmed actual-rate setter wiring. Exact commands/limits are in `SHIP_RUNTIME_AUDIT_SUMMARY.md`.
