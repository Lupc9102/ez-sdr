# Root integration ship review

## Target

Bounded read-only review of the current changes in `ez-gui/src/app.rs`, `config.rs`, `radio_ui.rs` and `app_ui_tests.rs`. Focus on config defaults/migration and asynchronous import/export, source preference restoration, profile synchronization, ADS-B resets and recorder actual-rate wiring. Report concrete integration defects with line references and evidence; do not edit root-owned files or map unrelated code.

## Tasklist

- [ ] Inspect the scoped diffs and relevant call sites.
- [ ] Check concrete regression hypotheses against implementation and available tests.
- [ ] Report substantiated findings and verification limits to root.

## Tips

- Pickup 2026-09-23: native Codex session subagent `/root/meteor_completion`; inherited GPT-6-family route, exact provider ID unavailable. User requests native session `codex/gpt-6-astra` delegation; no Morph or agy.
- Root owns and may concurrently edit the reviewed sources. This review writes only this document. Root is running app smoke checks and finishing the release.
- Prior Meteor ship audit is complete in `SHIP_METEOR_AUDIT_SUMMARY.md`: LRPT 71/71 and decoder panel 22/22 passed, plus audio/RTL build.

## Findings

Review in progress.
