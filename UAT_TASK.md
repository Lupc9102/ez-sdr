# 978 MHz UAT receive integration

## Target
Connect the ADS-B map to actual decoded 978 MHz UAT aircraft reports from FlightAware dump978-fa, preserving existing local 1090 decoding. Provide clear source controls, asynchronous lifecycle/error handling, and trustworthy position parsing without fabricated aircraft. Coordinate application dispatch and source ownership with the root agent.

## Tasklist
- [x] Harden OSM tile fetches, show missing/offline tiles, and reduce dark-theme map glare after root visual review — native Codex session subagent / inherited GPT-6, `/root/uat_implementation`; 21 panel tests passed including tile byte/dimension bounds, retry backoff and receiver lifecycle fixes
- [x] Inspect dump978-fa's documented receive protocol and existing ADS-B integration points — native Codex session subagent / inherited GPT-6, `/root/uat_implementation`
- [x] Implement asynchronous UAT report receiver and meaningful parser/lifecycle tests — native Codex session subagent / inherited GPT-6, `/root/uat_implementation`; 7 parser/stream tests and real TCP reconnect/cancel test passed
- [x] Add 978 source controls and aircraft integration in ADS-B panel; coordinate root dispatch — native Codex session subagent / inherited GPT-6, `/root/uat_implementation`; 4 panel merge/identity/expiry/source ownership tests passed, root wired poll/status/dispatch
- [x] Document decoder setup and verified limits; run relevant checks — native Codex session subagent / inherited GPT-6, `/root/uat_implementation`; see `UAT_RECEIVER.md`, 12 targeted tests passed including actual loopback TCP

## Tips
- Active pickup 2026-09-23: native Codex session subagent `/root/uat_implementation`, inherited GPT-6 model route. Explicit Astra override was rejected by routing; no external Morph/agy used. Read both task documents and FlightAware's direct newline JSON serializer. This work owns `adsb_panel.rs` and new `uat_receiver.rs`; root owns app dispatch/module export.
- Current user instruction supersedes old agy references: use built-in Codex session subagents only.
- Verification 2026-09-23: `cargo test -p ez-gui --lib --no-default-features uat_ -- --skip tcp_reconnect_and_cancellation_lifecycle` passed 11 tests; the separately run exact TCP reconnect/cancellation test also passed (2.03 s). Physical UAT reception is not tested; external dump978-fa must own the SDR, and must release a shared dongle before local1090 can reopen it.
- Map follow-up: HTTP timeout8s, response cap1MiB, PNG dimensions256×256 and allocation cap4MiB; PNG decode offUI, invalid cached bytes re-fetched, per-tile5→60s retry backoff (512 failure-state cap), visible loading/offline status, subtle missing grid and muted OSM tint. 1090 entry now disables radio audio and starts idle daemon; generic Stop cancels UAT. Final panel test run21/21 passed. Root owns rerendered visual evidence.
- Active harness/model: native Codex subagent / GPT-6, `/root/uat_finish`, 2026-09-23.
- Read `UI_REBUILD_TASK.md` and `UI_BUILD_TASK.md` before pickup. Prior work exposed 978 as tune-only; do not report that as working reception.
- Scope is targeted implementation; use `agy` for overall project mapping or massive rewrites per user instruction.
