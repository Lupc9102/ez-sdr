# Verification and reproducibility

Audit source: `e0f2d3b6cde2f9601e7a18fdd2d89ae75b2836be`. Audit began 13 October and resumed/concluded 14 October 2026. Application source was preserved; added Rust/Python/JavaScript files are audit probes, not product fixes.

## Results

- `cargo check --workspace --locked --all-targets`: **passed**, with the duplicated-test-attribute warnings. [Log](/home/lupc/Documents/ez-sdr/audit/2026-10-13/check-default.log).
- `cargo fmt --check`: **failed** on current `adsb_panel.rs` and `app.rs`. No automatic formatter repair was applied to product files. [Log](/home/lupc/Documents/ez-sdr/audit/2026-10-13/fmt.log).
- `cargo clippy --workspace --locked --all-targets --no-default-features -- -D warnings`: **failed** on duplicated `#[test]` attributes at config.rs:1286 and config.rs:1293. [Log](/home/lupc/Documents/ez-sdr/audit/2026-10-13/clippy-no-default.log).
- `cargo test --workspace --locked`: **incomplete after the first turn's interruption**. Eight completed harnesses reported 784 passing executions, zero failures and two ignored tests before GUI execution was interrupted. This is not a full default-feature pass. [Log](/home/lupc/Documents/ez-sdr/audit/2026-10-13/workspace-tests.log).
- `cargo test --workspace --locked --no-default-features`: **failed**. Completed harnesses reported 1,674 passing executions, one failure and seven ignored tests. The GUI harness alone reported 893 passed, one failed, five ignored. The failure was `demod_frame_versions_identify_latest_controls_under_iq_backlog` at its two-second final-frame deadline; Cargo stopped before running the later crate harnesses. [Log](/home/lupc/Documents/ez-sdr/audit/2026-10-13/workspace-no-default-tests.log).
- Exact isolated rerun of that no-default GUI test: **passed in 0.50 seconds**. A full-run failure remains a failure even when a targeted rerun succeeds. The result suggests load sensitivity, rather than proving persistent settings loss. [Log](/home/lupc/Documents/ez-sdr/audit/2026-10-13/demod-frame-isolated.log).
- `cargo build -p ez-gui --locked --no-default-features --features test-helpers --lib`: **passed**, enabling the real-sidebar PPM probe. [Log](/home/lupc/Documents/ez-sdr/audit/2026-10-13/ui-probe-build.log).
- All six local probes: **reproduced the reported defects**. Core geometry/startup output, rigctl/Discord results, popup PPM output and JavaScript event/input results are retained alongside their source files.
- Package-script shell syntax, audit Python byte-compilation and JavaScript syntax checks: **passed**. This checks syntax, not execution on Windows/macOS or a clean Linux install.
- The new scorecard's scoped TypeScript check: **passed**. An initial whole-canvas-directory check found errors in older unrelated scorecards; they were not edited. [Scoped log](/home/lupc/Documents/ez-sdr/audit/2026-10-13/canvas-typecheck-scoped.log).
- `graphify update .`: **passed**, AST-only, with no LLM/API extraction. Its final refresh rebuilt 8,128 nodes and 18,136 edges. Existing dirty graph metadata and unrelated user deletions were not included in the audit commit. [Log](/home/lupc/Documents/ez-sdr/audit/2026-10-13/graphify-update.log).

Counts are test-harness executions, not unique independent tests: the legacy decoder has both library and binary harnesses, and the duplicated test attributes also inflate totals. Do not add the isolated rerun to a headline unique-test count.

## Test scheduling intervention

The isolated Discord teardown probe found a busy-loop worker after the notifier was dropped. During the whole GUI run, task snapshots identified 51 hot orphan workers from completed app/runtime/Discord tests and a separate still-active `radio_power` test. To let remaining DSP checks finish, those 51 workers were assigned nice priority 19 and then confined to one CPU. The active DSP test and compilers retained their full CPU affinity. No assertion, application code or fixture data was changed. [Exact intervention log](/home/lupc/Documents/ez-sdr/audit/2026-10-13/test-worker-priority.log).

The final-frame deadline failure had already occurred before this scheduling intervention. A clean full-suite rerun after fixing worker teardown is required to establish an unmodified green release gate. Performance figures from this overloaded run should not be used as product benchmarks.

## Probe commands

From the repository root, build the existing libraries with the audit helper feature, then run the local reproductions:

```sh
cargo build -p ez-gui --locked --no-default-features --features test-helpers --lib
python3 audit/2026-10-13/run-probes.py
```

The runner uses already-built matching libraries, temporary executables, Linux `/proc` for the lifecycle/CPU observations, and local loopback sockets. It does not send Discord messages or call AI providers. Cached `ez_proto` and `num_complex` feature variants from the matching compiler are tried to match the daemon library's crate identity; the GUI library must expose `test-helpers`. It is an audit runner for this checkout, not a portable production test framework.

Probe inventory:

- [core-probe.rs](/home/lupc/Documents/ez-sdr/audit/2026-10-13/core-probe.rs): gated hardware-rate race, actual 3.2 MHz ADS-B tap and occupied-port ingest leak. [Output](/home/lupc/Documents/ez-sdr/audit/2026-10-13/core-probe.log).
- [side-services-probe.rs](/home/lupc/Documents/ez-sdr/audit/2026-10-13/side-services-probe.rs): Hamlib command mismatch and Discord post-drop busy worker. [Output](/home/lupc/Documents/ez-sdr/audit/2026-10-13/side-services-probe.log).
- [rigctl-lifecycle-probe.rs](/home/lupc/Documents/ez-sdr/audit/2026-10-13/rigctl-lifecycle-probe.rs): idle-client monopolization and identical-config disconnect. [Result](/home/lupc/Documents/ez-sdr/audit/2026-10-13/rigctl-lifecycle-result.json).
- [popup-ppm-probe.rs](/home/lupc/Documents/ez-sdr/audit/2026-10-13/popup-ppm-probe.rs): renders the actual Radio sidebar and asserts the pending PPM overwrite. [Output](/home/lupc/Documents/ez-sdr/audit/2026-10-13/popup-ppm-probe.log).
- [browser-lifecycle-probe.js](/home/lupc/Documents/ez-sdr/audit/2026-10-13/browser-lifecycle-probe.js): evaluates shipped daemon browser JavaScript and delays an old WebSocket's close callback until after reconnect. [Result](/home/lupc/Documents/ez-sdr/audit/2026-10-13/browser-lifecycle-result.json).
- [desktop-remote-input-probe.js](/home/lupc/Documents/ez-sdr/audit/2026-10-13/desktop-remote-input-probe.js): evaluates shipped desktop remote JavaScript and feeds telemetry while the frequency input is focused. [Result](/home/lupc/Documents/ez-sdr/audit/2026-10-13/desktop-remote-input-result.json).

## Unverified boundaries

USB inventory showed no SDR receiver. There was no new physical antenna/audio/USB recovery validation. The original off-air 80k Meteor gate remains open. HackRF/Soapy native libraries, cargo-audit and cargo-deny were unavailable locally. No clean-machine installer/package execution was performed in this audit. Authentication/TLS remote-use limits and macOS unsigned/unnotarized distribution remain as documented by the project.

The headless preview test renders real egui widgets through the existing CPU renderer. It establishes layout snapshots, not native window interaction, sound playback, antenna reception or GPU behavior. Its results and the separately completed protocol/LRPT/UAT harnesses are recorded in the completion addendum below.

## Completion addendum

See the final command summaries in [completion.json](/home/lupc/Documents/ez-sdr/audit/2026-10-13/completion.json) for the remaining-crate and UI-preview counts and statuses. These results supplement the failing workspace run rather than changing it into a pass.

The separate protocol/LRPT/UAT command passed 156 test executions across six harnesses, with four ignored and zero failures. The preview's original Cargo wrapper returned status 143 despite complete passing test output; the exact preview test was therefore rerun directly from the unchanged compiled no-default test executable and completed successfully (one test, 247.27 seconds). It rendered 36 images across 12 scenarios and three sizes. Four representative 1000×700 images are retained; redundant generated frames were removed after review. [Exact preview log](/home/lupc/Documents/ez-sdr/audit/2026-10-13/ui-preview-exact.log).

Representative snapshots:

- [Radio](/home/lupc/Documents/ez-sdr/audit/2026-10-13/screenshots/radio-1000x700.png).
- [ADS-B fixture](/home/lupc/Documents/ez-sdr/audit/2026-10-13/screenshots/adsb-fixture-1000x700.png).
- [Native UAT fixture](/home/lupc/Documents/ez-sdr/audit/2026-10-13/screenshots/uat-fixture-1000x700.png).
- [Meteor workspace](/home/lupc/Documents/ez-sdr/audit/2026-10-13/screenshots/meteor-1000x700.png).
