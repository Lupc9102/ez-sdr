# Meteor offline-only workflow

## Target

Make Meteor onboarding and legacy satellite entry points consistent with the user's requirement: EZ-SDR decodes existing Meteor `.cs8`/`.cf32` recordings offline and does not expose Meteor recording, source retuning/starting, or live daemon LRPT subscription as part of that workflow. Preserve ordinary Radio IQ recording and unrelated satellite tracking/reception. Root owns the main app and decoder integration; this agent owns `quick_start.rs`, `satellite_panel.rs`, `satellite_tab.rs`, and this task/report pair.

## Tasklist

- [x] Inspect onboarding, satellite controls, and main Meteor/daemon routes; identify remaining live paths — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route
- [x] Correct Meteor onboarding labels and isolate setup from source/config persistence side effects for testing — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route
- [x] Prevent Meteor capture/retuning in legacy satellite entry points while preserving unrelated workflows — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route
- [x] Verify no-start/no-retune/no-live-selection regressions and relevant existing decoder tests — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route; quick-start 3, satellite-panel 15, decoder 20 tests passed, including signed CS8 image recovery/export
- [x] Send root any remaining live app routes and write standalone completion report — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route; `METEOR_OFFLINE_SUMMARY.md`; audio+RTL feature compilation and formatting passed

## Tips

- Pickup 2026-09-23: native Codex session subagent `/root/meteor_completion`; inherited GPT-6-family route, exact provider ID unavailable. Built-in session collaboration only, no Morph or agy.
- Root already changed `Workflow::MeteorLrpt::apply` to return without restarting/retuning. The surrounding wizard still changes source mode before that call and still advertises a 137.9 MHz capture setup; both need correction.
- Current offline decoder supports `.cs8` and `.cf32`; CU8 is explicitly rejected. Root owns `app.rs` and `decoding_panel.rs`; inspect those read-only and send integration findings.
- `drain_daemon_events` still derives live LRPT subscription from `SharedState.selected_satellite`; scheduler auto-tuning and satellite selection/record controls require targeted auditing. Avoid writing real user configuration from tests.
- Root expanded ownership to `satellite_tab.rs` for legacy Decode rendering. It now uses `ui_offline`, hides live constellation there, and the sidebar no longer reuses capture controls. Root is handling desktop daemon subscription and Meteor scheduled-job filtering.
- In-memory `Workflow::configure_state` is shared by the persisted wizard apply path and regressions; Meteor returns before changing source mode or any capture setting. Tests call only this helper, so no saved user config is written.
- Root-owned followups reported: desktop daemon subscription now observed passing `false` for Meteor; scheduler now has `active_radio_job` excluding Meteor before choosing a concurrent eligible pass. Stale How To capture/Doppler/AI examples and AI `select_satellite` Meteor state were separately reported for root cleanup.
