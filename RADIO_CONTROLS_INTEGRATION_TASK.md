# Radio controls integration verification

## Target
Add production-API integration tests for local IQ → demodulation → raw subaudible tap → CTCSS detection/gating, including tone/neighbor/voice behavior and sample alignment at 44.1/48 kHz. Extend the existing ignored release throughput test with enabled FMIF and RDS processing over one actual synthetic source second. Review current app wiring and promptly report concrete regressions. Root owns production fixes; this agent owns only `ez-gui/tests/radio_controls.rs`, `ez-gui/tests/radio_performance.rs`, and this task/report.

## Tasklist
- [x] Record scope, file ownership and active identity — native Codex session subagent `/root/integration_review`, inherited GPT-6 route
- [x] Read current composition APIs and integration wiring; no concrete new defect found — native Codex session subagent `/root/integration_review`, inherited GPT-6 route
- [x] Add end-to-end CTCSS signal/gating regressions at both audio rates; all 3 tests pass — native Codex session subagent `/root/integration_review`, inherited GPT-6 route
- [x] Extend release throughput coverage with enabled FMIF and real RDS waveform processing — native Codex session subagent `/root/integration_review`, inherited GPT-6 route
- [x] Run focused tests and exact release benchmarks, retaining output logs; 3 CTCSS tests, 10 enabled cases, existing AM integration and 8 baseline cases pass — native Codex session subagent `/root/integration_review`, inherited GPT-6 route
- [x] Write `RADIO_CONTROLS_INTEGRATION_REVIEW.md` and hand off results — native Codex session subagent `/root/integration_review`, inherited GPT-6 route

## Tips
- Pickup 2026-09-23: native Codex session subagent `/root/integration_review`, inherited GPT-6 route; exact provider/model route unavailable. Built-in agents only, no Morph/agy.
- Root owns app/profile/UI integration and fixes. RDS agent `/root/radio_rebuild` is still improving the decoder. Coordinate fixture/API reuse and preserve concurrent production edits.
- Prior DSP verification:55 demod tests pass; see `RADIO_DSP_CONTROLS_SUMMARY.md` for raw NFM/WFM tap contracts and known FMIF stereo/RDS sensitivity limits.
- Benchmark complete software composition over one synthetic source second; do not claim USB, native-window or physical audio validation.
- Resumed after the rescinded pause; native Codex session subagent `/root/integration_review`, inherited GPT-6 route. No owned live process was restarted.
- CTCSS command: `cargo test -p ez-gui --no-default-features --offline --test radio_controls -- --nocapture`; 3 passed. Log: `/tmp/ez-sdr-radio-controls-tests.log`. Initial fixture frame-count assertion was corrected to whole emitted frames plus the one streaming interpolation interval; no production code changed.
- Completed release command: `cargo test -p ez-gui --release --features rtlsdr --offline --test radio_performance radio_enabled_controls_throughput -- --ignored --nocapture`; log `/tmp/ez-sdr-radio-enabled-throughput.log`; 10 cases pass. Process has finished.
- Existing cases also pass: `cargo test -p ez-gui --release --features rtlsdr --offline --test radio_performance -- --include-ignored --skip radio_enabled_controls_throughput --test-threads=1 --nocapture`; log `/tmp/ez-sdr-radio-existing-throughput.log`; 2 tests pass, including 8 baseline cases. No remaining owned process.
- Integration test files are frozen at handoff; next assigned scope is the runtime audit documented in `SHIP_RUNTIME_AUDIT_TASK.md`.
