# Integrated UI correctness review

## Target
Review the current integrated Rust GUI for concrete functional bugs in Radio source/audio lifecycle, UAT dispatch and ADS-B presentation, offline Meteor controls, and software-rendered UI evidence. Review only; do not edit component owners' files without coordination.

## Tasklist
- [x] Inspect integrated application and Radio lifecycle — native Codex session subagent `/root/integration_review` / inherited GPT-6 route
- [x] Inspect ADS-B/UAT and offline Meteor integration — native Codex session subagent `/root/integration_review` / inherited GPT-6 route
- [x] Review renderer/test evidence and send actionable findings — native Codex session subagent `/root/integration_review` / inherited GPT-6 route
- [x] Fix local source queue saturation, preserve replay samples/EOF, and keep cancellation responsive — native Codex session subagent `/root/integration_review` / inherited GPT-6 route
- [x] Run focused source regressions and report verified results — native Codex session subagent `/root/integration_review` / inherited GPT-6 route; all 20 source tests passed

## Tips
- Pickup 2026-09-23: native Codex session subagent `/root/integration_review`, inherited GPT-6 model route; exact provider identifier is not exposed. Read `UI_REBUILD_TASK.md`. No Morph or agy.
- Root and component owners are editing concurrently. Verify findings against current contents and report exact paths/locations.
- Root assigned exclusive ownership of `ez-gui/src/source_manager.rs` after review found every local producer terminated on bounded queue Full. Live producers should drop overflow blocks without terminating; file replay must retain every block with cancellable backpressure. Root handles application cadence/audio/tune integration; Radio and UAT owners handle their panels.
- Validation: `cargo test -p ez-gui --no-default-features source_manager::tests --lib` passed 20/20 tests, including seven new regression tests. Replay sample/EOF backpressure and slow pacing stop within the tested 250 ms limit. No real RTL-SDR shutdown timing is claimed: its worker still waits for the next USB read.
