# Meteor offline decoder completion

Harness/model: native Codex session subagent, inherited GPT-6 route (`/root/meteor_completion`). No Morph or agy used.

## Changes

- Completed `ez-gui/src/decoding_panel.rs` worker/UI integration: cancel action, byte/percent progress, immutable input controls during decode, short aligned signed CS8/CF32 blocks, background completion, shutdown cancellation, and useful format/rate/alignment errors.
- Added progressive channel previews with MSU-MR labels and final image replacement. Offline rendering hides daemon telemetry and ignores unsupported tracking-satellite notices. Root integrates a separate panel instance for Meteor imports.
- Connected PNG export to collision-safe creation, recording-based names, and save feedback. Previous exports are retained.
- Cleared stale pass output when inputs change, rejected overlapping satellite decode requests, and handled unexpected worker-channel closure.
- Added a documented synthetic MSU-MR transport fixture in `ez-gui/tests/fixtures`.

## Verification

`rustfmt --edition 2021 ez-gui/src/decoding_panel.rs` completed.

`cargo test -p ez-gui --no-default-features decoding_panel::tests --lib` passed **16 tests**. Coverage includes signed CS8 → OQPSK → Viterbi → Reed-Solomon → CCSDS → MSU-MR image → PNG (every expected pixel), complete byte accounting, invalid formats/rates/alignment, cancellation and shutdown signals, background completion, disconnected workers, existing-export preservation, and offscreen egui preview/progress/telemetry-isolation behavior.

This is synthetic signal and offscreen UI verification. Native desktop visual inspection and real off-air recordings are separate validation scopes; no hardware or off-air result is claimed here.

## Integration review fixes

Moved recording/folder dialogs and PNG encoding into a polled background worker so these operations do not stop the app's frame loop. Decoded images are shared through `Arc` without a large UI-thread clone. Inputs remain locked until selection/export completes; cancellation and errors release the lock and preserve previous results. Worker completion wakes egui, and completed decodes release their obsolete progress snapshots.

Wrapped the results scroll area in an explicit top-down layout. Heading, status/export controls, and all channel images now stack vertically instead of inheriting the outer horizontal columns.

The same focused test command now passes **20 tests**, adding blocked-chooser responsiveness, asynchronous non-overwriting export with shared image storage, dialog cancellation/export failure recovery, and measured vertical result geometry at 1000×700. Native rerender remains with the primary agent.
