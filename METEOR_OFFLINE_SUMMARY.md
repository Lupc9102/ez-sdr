# Meteor offline-only workflow completion

Harness/model: native Codex session subagent `/root/meteor_completion`; inherited GPT-6-family route, exact provider ID unavailable. Built-in collaboration only; no Morph or agy.

## Changed

- `quick_start.rs`: Meteor is labeled as an offline decoder and advertises only the supported `.cs8`/`.cf32` formats. The welcome screen offers a direct offline path without selecting hardware. Setup/completion instructions cover existing files, recorded sample rate, presets, decoding and PNG export instead of RF tuning, antennas or capture.
- Extracted in-memory workflow configuration from persistence. Meteor returns before applying a previously selected source mode or any source stop/start/tune operation. Ordinary running Radio audio/recording remains untouched. Saved setup completion is still handled by the normal apply path; regression tests never write user configuration.
- `satellite_panel.rs`: Meteor selection updates orbit tracking only, disables the legacy auto-tune indicator, clears the desktop live-satellite request, and preserves source frequency/center/rate/status/generation. Meteor recording controls are replaced with offline-import guidance. The recording start function independently rejects Meteor before creating any directory or writer, and sample feeding stops a stale legacy capture when Meteor is selected. Finalizing an old Meteor-tagged capture does not automatically hand it to a decoder.
- Non-Meteor satellite tuning/recording remains available; the ordinary Radio recorder was not edited. The legacy Decode sidebar now contains offline file instructions instead of satellite capture controls or an antenna checklist.
- `satellite_tab.rs`: legacy Decode renders `ui_offline`, with an offline footer and no live constellation/capture sidebar. Meteor orbit tracking also no longer presents the unrelated live-IQ constellation as its reception quality.

## Verification

- `cargo test -p ez-gui --offline --no-default-features quick_start::tests --lib`: **3 passed**. New regression covers idle/running Demo, pending daemon and replay states, proving the wizard ignores a prior Hardware choice for Meteor and preserves source identity/settings plus ongoing Radio recording/audio.
- `cargo test -p ez-gui --offline --no-default-features satellite_panel::tests --lib`: **15 passed**. New data-path checks prove Meteor selection does not retune/start/restart or request a live satellite; a prohibited recording creates no output; ISS tuning remains available; old Meteor capture finalization cannot enqueue a decoder handoff.
- `cargo test -p ez-gui --offline --no-default-features decoding_panel::tests --lib`: **20 passed**, including signed CS8 image recovery, progressive offline rendering, background file dialogs, cancellation and PNG pixel/export checks.
- Audio+RTL feature compilation and formatting checks passed.

## Root integration findings

The main Meteor tab already uses its separate offline decoder without source-control actions. Root was notified that the desktop daemon event loop still derived LRPT subscriptions from a selected Meteor satellite, and that scheduled Meteor passes could retune the receiver. Read-only followup confirmed root changed daemon synchronization to pass `false` for Meteor and added scheduler filtering before choosing eligible live reception jobs, preserving concurrent ISS jobs and generic custom Radio tasks.

Remaining content outside this agent's ownership was reported to root: `howto_panel.rs` still described Meteor capture settings, automatic Doppler correction and recording-to-decoding, plus an AI example that tuned Meteor RAW; `ai_panel.rs` still accepted Meteor through `select_satellite`. Root owns that final guidance/AI cleanup. Standalone daemon capabilities and generic Radio tune/record operations remain in scope as ordinary radio features.

No native hardware or real user configuration was exercised by these tests.
