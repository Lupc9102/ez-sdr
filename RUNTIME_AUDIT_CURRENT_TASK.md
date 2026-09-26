# Current EZ-SDR perf/runtime audit

## Target

Audit the current `255123c` performance changes and the surrounding `ez-gui` runtime for regressions. Focus on executable behavior and tests: hidden-tab DSP gating, repaint cadence, waterfall row/texture uploads, FFT size controls (including SDR++'s 65536 option), and build/test health. Preserve the dirty worktree and avoid unrelated edits. Report exact commands, failures, and any isolated fix to `/root`.

## Tasklist

- [x] Inspect current perf diff and relevant runtime paths. — native Codex tools / GPT-6 (`/root/runtime_audit`)
- [x] Run focused GUI tests and workspace checks where feasible. — native Codex tools / GPT-6 (`/root/runtime_audit`); workspace all-feature check passed, Radio controls/performance passed, current GUI suite exposed FFT-limit and async-worker failures.
- [x] Validate hidden-tab DSP/repaint/waterfall behavior and parity controls. — native Codex tools / GPT-6 (`/root/runtime_audit`); local spectrum batching is gated to Radio, but worker results are only polled during sample pushes, so a final frame can remain queued when the source/tab goes idle.
- [x] Record findings and handoff evidence to `/root`. — native Codex tools / GPT-6 (`/root/runtime_audit`); exact test failures, async FFT behavior, release throughput, workspace check, and CW test geometry issue reported.
- [ ] Audit and correct stale user-facing CW, stereo/RDS, and UAT capability claims in `sdr_panel.rs`, `quick_start.rs`, and `howto_panel.rs`.

## Tips

- Pickup 2026-09-26: active harness/model is native Codex tools / GPT-6 (`/root/runtime_audit`). Existing worktree is intentionally dirty; do not reset or clean it.
- The current commit advertises `MAX_FFT_SIZE = 8_192` while the UI still offers 16,384/32,768/65,536 and installed SDR++ uses 65,536. Audit this conflict before making changes.
- 2026-09-26 update: FFT parity work now drains the local async worker once per `CentralApp::logic` tick and retains the newest worker output; the old “only polled during sample pushes” caveat is superseded. Daemon spectrum events remain ungated by active tab.
