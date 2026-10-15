# Ship audit: offline Meteor decoding

## Target

Audit and fix reproducible failures in the real offline Meteor path: existing `.cs8`/`.cf32` import, input validation, background decode/progress, cancel/retry, results, and image export, with bounded LRPT decoder state. Preserve the prior offline-only workflow and all unrelated dirty work. Owned sources: `decoding_panel.rs`, `satellite_tab.rs`, `quick_start.rs`, `satellite_panel.rs`, and `lrpt-decode/**`. Root owns app/config/Radio integration.

## Tasklist

- [x] Read prior Meteor offline task/report and establish current import/decoder boundaries — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route
- [x] Reproduce user-visible runtime or decoder defects with focused cases — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route; confirmed lost CADUs on large blocks, blocked progress queue, oversized GPU texture panic, and misleading no-sync on nonfinite CF32
- [x] Fix confirmed defects within owned files and add meaningful regressions — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route; process every valid CADU, scan before noise retention trimming, nonblocking/bounded previews, GPU-limited display copies with full exports, precise bad-CF32 errors
- [x] Run affected Meteor/LRPT checks and inspect relevant UI output — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route; LRPT 71/71, decoder panel 22/22, audio+RTL compilation, formatting and scoped whitespace checks passed
- [x] Report completed fixes, integration findings, and remaining practical limits — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route; `SHIP_METEOR_AUDIT_SUMMARY.md` records reproductions, changes, exact verification results and validation limits

## Tips

- Pickup 2026-09-23: native Codex session subagent `/root/meteor_completion`; inherited GPT-6-family route, exact provider ID unavailable. Native session collaboration only; no Morph or agy.
- Read `METEOR_OFFLINE_TASK.md` and `METEOR_OFFLINE_SUMMARY.md`. Prior validation: quick-start 3, satellite panel 15 and decoder panel 20 passing tests; CS8 full image recovery/export fixture exists. Preserve all those changes.
- Prior main integration now disables desktop live LRPT subscriptions and filters scheduled Meteor auto-tuning. Decoder remains offline and independent of ordinary Radio recording.
- File dialogs and export run on worker threads. Decode worker uses 16,384-complex-sample chunks and atomic cancellation/byte progress. Existing saved-config setup tests use the in-memory helper and do not write user configuration.
- Baseline failures reproduced: six continuous CADUs yield 24 RS codewords with small chunks but only 4 from a large block; a full progress queue stalls a worker; a 1568×4096 result panics against a 1024-side GPU limit; NaN CF32 samples misleadingly consume the file and report no sync.
- Progress snapshots now use at-most-1024-side grayscale previews, are sent nonblocking, and retain no more than two queued snapshots even with a caller-provided unbounded channel. Full-resolution final image/export buffers remain intact.
- Continuation pickup 2026-09-23: native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route; exact provider ID unavailable. Read this target, prior agent entries and Meteor summary; completed the final queue-cap regression and checks without repeating the initial audit. User requests `codex/gpt-6-astra` through native session collaboration for delegation; no external model tools were used.
