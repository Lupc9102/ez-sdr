# RTL source parity task

## Target

Add asynchronous RTL-SDR enumeration/refresh, validated device selection that survives USB index reorder when a unique serial is available, and real librtlsdr offset tuning. Preserve live queue/backpressure, replay integrity, cancellation, and source lifecycle behavior. Root integrates controls/config/app; edits here are limited to `ez-gui/src/source_manager.rs` and these source task/report documents. No Morph or agy.

## Tasklist

- [x] Inspect source worker and librtlsdr binding capabilities; send integration API — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route
- [x] Implement asynchronous enumeration and stable validated device selection — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route
- [x] Apply selected device and true offset tuning in hardware worker — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route
- [x] Add deterministic selection/refresh/lifecycle tests and compile RTL feature offline — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route; 28 source tests passed and RTL feature checked
- [x] Record evidence, limitations, and final API in standalone summary — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route; `SOURCE_PARITY_SUMMARY.md`
- [x] Correct Demo sample pacing to count complex I/Q pairs and include processing time — native Codex session subagent `/root/meteor_completion` / inherited GPT-6-family route; 29 source tests pass

## Tips

- Pickup 2026-09-23: native Codex session subagent `/root/meteor_completion`, inherited GPT-6-family route; exact provider model identifier is not exposed. Read `RADIO_PARITY_TASK.md` and `UI_REBUILD_TASK.md`. User requested built-in session subagents and explicitly forbids agy/Morph.
- Source worker queue and shutdown handling was recently fixed by integration_review. Preserve current live overflow-drop semantics and cancellable replay sends.
- No physical RTL-SDR validation is implied by deterministic tests or feature compilation. Root owns `radio_ui.rs`, `config.rs`, `app.rs`; DSP agent owns `demod.rs`, `sdr_panel.rs`.
- API sent to root: `RtlDeviceInfo`, serial/index `RtlDeviceSelection`, source fields `rtl_device`, `rtl_devices`, `rtl_device_refresh_error`, `offset_tuning`; nonblocking refresh/poll/in-flight methods, validated selection and selected descriptor methods. Worker re-resolves USB identity at open. Root restarts hardware after configuration changes. DSB/CW daemon modes must map to no audio subscription because the wire protocol cannot represent them.
