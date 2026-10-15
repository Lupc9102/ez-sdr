# App shipping audit and repair

## Target
Finish the native Rust Radio, ADS-B 1090/978 map, and offline Meteor application while auditing and fixing reproducible defects across its user flows. Preserve all pre-existing changes. Keep the radio control work moving concurrently. Produce a tested release artifact and a candid standalone report of verified behavior and remaining environmental or reference limitations.

## Tasklist
- [x] Assign parallel ADS-B, Meteor, and runtime audit/fix owners — Codex built-in session harness / GPT-6 root (exact provider route unavailable)
- [ ] Finish radio controls, integration, and rendered UI review — root
- [ ] Audit/fix ADS-B map, region switching, 1090 decoding, and 978 ingestion — radio_rebuild
- [ ] Audit/fix Meteor import, decode, cancellation, results, and export — meteor_completion
- [ ] Audit/fix source/audio/daemon lifecycle and recording — integration_review
- [ ] Audit/fix root application startup, settings, tab transitions, and auxiliary control integration — root
- [ ] Run current workspace tests, feature checks, release build, and bounded smoke tests — root
- [ ] Write SHIP_READINESS_SESSION_SUMMARY.md with artifact, fixes, evidence, and residual blockers — root

## Tips
- Pickup 2026-09-23: Codex built-in session harness / GPT-6 root; exact provider model route is not exposed. Existing agents inherit session routing. User requested built-in Astra agents and explicitly rejected Morph/agy earlier; no external agent runners are used for these routine bounded audits and fixes.
- Four concurrent slots include root. Reuse the three existing built-in workers. Each owns its scoped tracker and report. Root owns app.rs, config.rs, radio_ui.rs, app_ui_tests.rs, main.rs and top-level integration/documents.
- Read RADIO_CONTROLS_TASK.md and the scoped summaries before touching existing work. Old production audits describe older revisions: reproduce findings against current files before treating them as live defects.
- Preserve the dirty worktree; do not reset, revert, or commit. Hardware, sockets and native display access have previously been restricted. Record skipped evidence honestly.
