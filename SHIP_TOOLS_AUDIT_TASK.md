# Ship tools audit

## Target

Perform a bounded audit/fix of `ez-gui/src/bookmark_manager.rs`, `secondary_panel.rs`, `scanner.rs`, `keyboard.rs`, and `web_remote.rs`. Move bookmark CSV import and session-notes export dialogs/file I/O off the UI thread and show failures. Check keyboard/remote/scanner start-stop/tune flows for concrete defects. Root owns app/config/UI integration; report fixes needed there without editing `app.rs` or `config.rs`. Preserve all dirty work. No additional network escalation; socket tests remain limited by the documented environment.

## Tasklist

- [x] Record assignment and ownership — native Codex session subagent `/root/integration_review`, inherited GPT-6 route
- [ ] Inspect and fix asynchronous bookmark CSV import and notes export flows
- [ ] Inspect keyboard, scanner, and remote start/stop/tune paths; fix or report concrete defects
- [ ] Run focused regression and formatting checks
- [ ] Write `SHIP_TOOLS_AUDIT_SUMMARY.md` and hand off

## Tips

- Pickup 2026-09-23: native Codex session subagent `/root/integration_review`, inherited GPT-6 route; exact provider/model route unavailable. Built-in agents only; no Morph/agy.
- Preceding work is complete: `SHIP_RUNTIME_AUDIT_SUMMARY.md` and `RADIO_CONTROLS_INTEGRATION_REVIEW.md`. No owned live process remains.
- Root's config import/export is already asynchronous; do not change config.rs. Root concurrently owns app integration, settings persistence, and capture transitions.
- Loopback bind is sandbox-blocked. Automatic approval review failed with a provider model-route 403; parent knows. Do not attempt another network escalation.
