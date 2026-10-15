# ADS-B / UAT shipping audit

## Target
Audit and fix user-visible ADS-B 1090 MHz and UAT 978 MHz defects in `ez-gui/src/adsb_panel.rs`, `ez-gui/src/adsb_decoder.rs`, `ez-gui/src/uat_receiver.rs`, and `dump1090/**`. Focus on region/source changes, stale state and cancellation, map position/heading validity, partial or malformed UAT reports, reconnect behavior, and 1090 decoding. Preserve the shared dirty worktree. Root owns app/config/radio UI; send integration issues there. No broad project mapping, reset, commit, Morph or agy.

## Tasklist
- [x] Read previous UAT/emulation reports and establish file ownership/current dirty state — built-in Codex session agent `/root/radio_rebuild` / inherited GPT-6-family route; exact provider identifier not exposed
- [ ] Audit receiver, decoding, state lifecycle and map boundary behavior; reproduce concrete defects.
- [ ] Fix confirmed owned-file defects and notify root of integration findings.
- [ ] Run meaningful focused regression and flow tests.
- [ ] Finalize standalone audit report and hand off remaining verified limitations.

## Tips
- Pickup 2026-09-23: built-in Codex session agent `/root/radio_rebuild`, inherited GPT-6-family route; exact provider identifier not exposed. Built-in session agents only.
- Read UAT_SESSION_SUMMARY.md and ADSB_EMULATION_SESSION_SUMMARY.md. Prior work implemented dump978-fa JSON TCP, bounded queues and line lengths, cancellation/generation isolation, map plane SVG heading, independent position expiry and map tile resource limits. Existing hardware-free 1090 IQ fixture lives in the daemon and is outside this agent's ownership.
- Owned files are already dirty, and uat_receiver.rs is untracked from earlier authorized work; preserve all existing changes. Root app.rs/config.rs/radio_ui.rs are not owned here.
- A full graph rebuild is inappropriate for this scoped known-file audit; inspect these entry points directly.
