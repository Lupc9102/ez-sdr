# Integrated UI review and source lifecycle correction

Agent: native Codex session subagent `/root/integration_review`, inherited GPT-6 model route (exact provider identifier not exposed). Date: 2026-09-23. No Morph or agy used.

## Completed work

- Read `UI_REBUILD_TASK.md`, logged identity there, and created/maintained `INTEGRATION_REVIEW_TASK.md`.
- Reviewed application startup, Radio controls, source/audio lifecycle, 1090/UAT dispatch, offline Meteor controls, and CPU-rendered application artifacts at both desktop sizes.
- Reported concrete bugs to root/component owners: spectrum tuning and Start action did not apply receiver/audio state; idle daemon ADS-B did not connect; 1090 entry left local radio audio enabled; generic UAT Stop did not cancel its receiver; keyboard mute drifted from Radio's private mute state; daemon Host commit did not reconnect; failed audio startup could not recover; daemon volume was not applied; synchronous Meteor dialogs/export blocked live processing; Meteor results inherited horizontal layout; slider rails matched panel color; the UAT render fixture lacked position timestamps.
- Identified the source lifecycle failure underlying stalled live reception: all local producers treated a full bounded queue as a disconnected receiver, terminated, and left the UI reporting Running.
- Implemented the assigned correction in `ez-gui/src/source_manager.rs`. Live overflow now drops only that block and continues. Replay uses cancellable backpressure for samples, errors, and EOF, retaining data ordering. Stop interrupts pacing and blocked sends; unexpected receiver disconnect surfaces an error. Empty, partial-sample, and nonregular replay inputs are rejected, and prior local workers are joined before restart.
- Added seven regressions covering live overflow versus disconnect, actual demo recovery after a stalled consumer, replay byte integrity/EOF through saturation, Stop while a sample or EOF send is blocked, Stop during slow replay pacing, empty looped input, and unexpected worker termination.

## Validation

`cargo test -p ez-gui --no-default-features source_manager::tests --lib`: **20 passed**, including all seven new regressions. `git diff --check -- ez-gui/src/source_manager.rs`: clean.

Replay cancellation tests verify Stop within 250 ms. Real RTL-SDR shutdown timing remains unverified because the worker still depends on its next USB read. The source tests do not claim real hardware/audio/off-air acceptance. Parent and component owners are responsible for integration fixes reported above and their final verification.

The inspected software-rendered artifacts contain actual application widgets; the identified Meteor layout and invisible-slider issues originate in application layout/theme settings. The UAT fixture at review time did not establish UAT map/heading behavior because it inserted aircraft without the UAT position timestamps required to draw them.
