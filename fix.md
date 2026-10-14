# EZ-SDR repair handoff for Luna

## Start here

Implement the software repairs below in order. Work in small verified batches, commit each completed batch, and maintain the checklist at the end of this file. This document is the implementation specification; the audit provides supporting evidence. Do not repeat the entire audit or expand the product scope.

Repository: `/home/lupc/Documents/ez-sdr`. Audited application revision: `e0f2d3b`. Evidence commit: `0495517`. This handoff was written on 14 October 2026. Recheck the current source before editing because line numbers can move. Use the function names below as anchors.

Read [AGENTS.md](/home/lupc/Documents/ez-sdr/AGENTS.md). If `graphify-out/graph.json` exists, first query the relevant feature. After code changes run `graphify update .` (AST only). Commit only your changes; the checkout already contains unrelated deleted screenshots/cache tiles and edited graph metadata. Do not restore, stage or discard those changes.

Supporting evidence:

- [Full audit, F01–F23](/home/lupc/Documents/ez-sdr/audit/2026-10-13/REPORT.md).
- [Verification and environment limits](/home/lupc/Documents/ez-sdr/audit/2026-10-13/verification.md).
- [Six local reproductions](/home/lupc/Documents/ez-sdr/audit/2026-10-13/run-probes.py).
- [CI contract](/home/lupc/Documents/ez-sdr/.github/workflows/ci.yml).

For each batch: read only the named files and directly related tests; reproduce the relevant failure; implement the smallest coherent fix; add a meaningful regression test; run the focused checks; inspect the diff; update the checklist and graph; commit. Record the command and result, not just “tested.”

## Scope and invariants

- Keep the three advertised workspaces: Radio, ADS-B and Meteor. Keep Setup Guide and Settings as independent floating windows. Preserve the independent B receiver.
- Do not restore retired AI/Discord/scanner/scheduler/theme utility windows, add satellite missions, implement a plugin system, or pursue general SDR++/SatDump feature parity.
- Preserve capture clocks, rational sample rates, channel identity, recording completeness, Meteor product boundaries/finalization and existing bounded queues. Never relabel samples to pretend a requested clock was achieved.
- Preserve protocol version 13 unless a repair genuinely changes a wire format. These repairs should not require a wire-version migration or replacing bincode.
- Changing startup defaults, observer/appearance settings or unrelated services must not retune/start the RF receiver. Stopped receivers stay stopped. Only actual acquisition-setting changes may restart hardware.
- Preserve missing-audio-device errors; do not silently choose another sound device. Preserve authentication, loopback defaults, Origin/Host validation and numeric endpoint restrictions.
- Never hold `SharedState` or a commonly used channel-table lock while joining a worker, waiting for hardware, or performing network/disk I/O.
- Use temporary directories, fake hardware and local loopback services for tests. Do not send real Discord notifications or call paid AI providers.
- Historical files under `audit/2026-10-13/` describe the broken revision. Leave them intact. Put new regression tests in the existing crate test modules and new evidence in a new directory.

## Important: the audit probes are negative reproductions

Several audit probes deliberately assert the **buggy** result. After a correct repair, those assertions should fail. For example, `popup-ppm-probe.rs` expects 37 ppm to be overwritten with 3 ppm, and `side-services-probe.rs` expects a dropped notifier to retain a hot worker.

Do not preserve the bugs to make that runner green. Copy the useful setup into a proper regression test, invert the expected result, and verify the corrected behavior. The core probe prints observations rather than asserting every one; build deterministic assertions in the crate tests. Do not edit old audit results to imply the original code passed.

## Batch 0 — make verification reliable first: F15 and the mechanical part of F20

Files: [discord.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/discord.rs), [config.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/config.rs), [app.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs), [adsb_panel.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/adsb_panel.rs).

1. In `DiscordNotifier::new`, replace the unconditional receive loop with a loop that terminates on receiver disconnection. Currently `Err` from `rx.recv()` immediately repeats forever.
2. Provide a deterministic way to observe worker termination in tests, such as a completion receiver/guard. Keep notification publication bounded and nonblocking. Do not introduce an unbounded synchronous join in GUI `Drop`; queued network requests must have finite deadlines before any owned teardown can wait for them. Full delivery/error improvements come in Batch 8.
3. Remove the duplicate consecutive `#[test]` attributes on `config_default_has_empty_custom_theme_gallery` and `config_missing_custom_themes_fall_back_to_defaults`. Keep the tests themselves.
4. Format the changed Rust code and the existing formatting failures in `app.rs`/`adsb_panel.rs`. Inspect the diff for unintended edits.

Acceptance: repeatedly create/drop notifiers with no notifications, and observe every worker terminate within a bounded deadline. This must work with audio/RTL defaults disabled. Do not rely on exact global thread counts or CPU percentages in a concurrently running test suite. `cargo fmt --check` and strict no-default Clippy must pass or expose a new specific error to resolve.

The old full GUI run accumulated 51 spinning notifier workers. It also failed `demod_frame_versions_identify_latest_controls_under_iq_backlog` at its two-second deadline; the isolated rerun passed in 0.50 seconds. Fix teardown before diagnosing that test. Do not disable it, weaken the final-control assertions or silently increase timeouts to hide lost settings. If it still fails in a clean process, investigate synchronization and use an explicit completion/version signal where appropriate.

Focused checks:

```sh
cargo test -p ez-gui --locked --no-default-features discord::tests::
cargo test -p ez-gui --locked --no-default-features demod::tests::demod_frame_versions_identify_latest_controls_under_iq_backlog -- --exact --nocapture
cargo fmt --check
cargo clippy --workspace --locked --all-targets --no-default-features -- -D warnings
```

## Batch 1 — exact ADS-B clock validation: F02

Files: [state.rs](/home/lupc/Documents/ez-sdr/ez-daemon/src/state.rs), [channelizer.rs](/home/lupc/Documents/ez-sdr/ez-daemon/src/channelizer.rs), [packet.rs](/home/lupc/Documents/ez-sdr/ez-daemon/src/pipelines/packet.rs), [capture.rs](/home/lupc/Documents/ez-sdr/ez-proto/src/capture.rs).

Failure: `validate_spec` checks the **requested** 2.4 MHz ADS-B rate. `add_channel` uses integer decimation, so a 3.2 MHz source actually produces 3.2 MHz. `PacketPipeline::new` logs a mismatch and still constructs `Demod2400`.

Required implementation:

1. Choose rejection of incompatible clocks for this repair, not a new resampler. Preserve exact compatible clocks and existing provenance.
2. Validate the physical rate before allocating a tap or starting an ADS-B worker. Apply the same check to creation and `retune_configured`; an invalid retune must leave the old live channel intact.
3. Do not use rounded integer output-rate metadata as proof of an exact clock. For nominal source rate `R` and integer factor `D`, require `R == 2_400_000 * D` using checked/widened integer arithmetic. Thus 4,800,000/2 is valid, but 4,800,001/2 is not.
4. Add defense at the packet decoder boundary: incompatible blocks must not reach `Demod2400`. `CaptureClock` has `source_sample_rate_hz` and `sample_stride`; require a nonzero stride and exact `source_sample_rate_hz == 2_400_000 * sample_stride` where clock metadata is present. Account for the existing unannotated-block fallback. Clear partial overlap/decoder continuity on a rejected boundary rather than stitching through it.
5. Ensure any failure after tap allocation removes that tap. Return actionable errors; logging alone is insufficient. Keep constructor/API changes limited to affected callers and tests.

Regression cases: 2.4 MHz and 4.8 MHz valid; 3.2 MHz and 4,800,001 Hz invalid; zero/too-narrow rates invalid; direct incompatible block rejected; invalid retune leaves spec/tap/recording unchanged; rejected subscription leaks no tap or pipeline. Cover the actual metadata, not just error text.

```sh
cargo test -p ez-daemon --locked --no-default-features state::tests::
cargo test -p ez-daemon --locked --no-default-features pipelines::packet::
```

## Batch 2 — serialize capture geometry changes: F03

Files: [state.rs](/home/lupc/Documents/ez-sdr/ez-daemon/src/state.rs), [channelizer.rs](/home/lupc/Documents/ez-sdr/ez-daemon/src/channelizer.rs), [ingest.rs](/home/lupc/Documents/ez-sdr/ez-daemon/src/ingest.rs). Reuse the gated source setup from [core-probe.rs](/home/lupc/Documents/ez-sdr/audit/2026-10-13/core-probe.rs).

Failure sequence: rate setter sees no virtual channels → hardware call waits → another caller inserts an audio channel → hardware applies 1.2 MHz → channelizer refuses the change → stale geometry still accepts a 2.4 MHz spectrum request.

Recommended implementation: a dedicated geometry-operation mutex/reservation, separate from the frequently read channel table. It serializes the full check/apply/publish transaction and all mutators that could invalidate it. Read-only status and DSP ticks must not acquire it.

- Guard `set_sample_rate` and first-channel creation from before validation through final installation. Review frequency changes, configured retunes and removal for geometry consistency and give mutators a consistent entry point.
- Do not take the gate while already holding `channels`, `recordings`, `channelizer` or a pipeline lock. Document the ordering. Avoid recursive acquisition through wrappers such as `retune` → `retune_configured`; use one guarded entry point and private helpers where needed.
- Keep the channel table unlocked during the hardware wait. Do not “fix” this by holding `channels` over a driver call or making every reader take the new gate.
- Revalidate after obtaining the gate. A subscriber that waits for a rate change must validate against the **new** geometry when it proceeds.
- A hardware error must leave coherent state; an ambiguous hardware outcome must be reported explicitly, not advertised as a successful old clock. Use existing ingestion acknowledgement/status contracts rather than assuming a failed request could never have applied.
- Preserve channel caps, existing same-ID subscription sharing, spectrum-only rate changes and active-recording removal protection.

Deterministic tests: gate hardware application with channels, not arbitrary sleeps; race rate change with subscribe; verify hardware status and channelizer acceptance agree afterward; verify hardware rejection leaves the previous state; verify reads remain available during the gated hardware call; rerun concurrent first-subscriber and recording lifecycle tests. Ensure every test cleanup path releases its fake driver gate.

## Batch 3A — one owner for pending settings: F01 and F04

Files: [app.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs), [config.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/config.rs), [secondary_panel.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/secondary_panel.rs), [radio_ui.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/radio_ui.rs), [advanced_panel.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/advanced_panel.rs), [app_ui_tests.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/app_ui_tests.rs).

Failure: Settings edits/replaces live `state.config` during UI rendering; later Radio rendering copies old source/spectrum values into it; `needs_apply` is consumed only next logic frame.

Implementation direction: keep a **popup draft plus queued apply snapshot**, separate from the live config used by Radio. Initialize the draft when Settings opens. Popup edits, Import and Reset operate on that draft. Apply/Import/Reset enqueue a validated snapshot. Commit/apply the snapshot at a controlled logic boundary before live-to-config persistence can touch it. Keep transient editor/job state out of serialized `AppConfig`.

- Refactor the current `AppConfig::ui`/`render_settings` action flow enough that it cannot silently replace live config mid-render. Do not use widget ordering or closing the Source section as the fix.
- A changed draft PPM must survive every subsequent frame until apply/cancel. Do not copy the old source PPM into the draft.
- An imported/reset FFT size/window, rate, waterfall parameters, smoothing, ranges, grid and colormap must reach the spectrum before `persist_display_settings` reads it back.
- Normalize before persistence/application. Show save failure clearly and retain unsaved changes for retry; do not replace the config with defaults on an I/O error.
- Prevent an old async import result from replacing a newer editor session after close/reopen. Preserve existing background file-dialog behavior.
- Reopening Settings must show the effective current configuration, not a stale earlier draft. Define closing without Apply as discarding the uncommitted draft.
- Applying startup defaults must preserve active frequency/rate/gain and reception state. Preserve actual PPM/hardware-option apply behavior when those acquisition settings change.
- A queued apply followed immediately by app close must not be overwritten by exit-time persistence of older live values.

Regression tests must exercise actual app UI/logic sequencing, not just `AppConfig` serialization. Include: source=3 ppm / draft=37 ppm, Hardware Source expanded; collapsed Source; non-Radio tab; importing distinct display settings while Radio is visible; Reset while nondefault display is live; close/reopen; stale import completion; Save then close; save failure and retry. Use temporary config paths and injected file results; do not open native dialogs or overwrite the user's settings in tests.

Retain these existing contracts:

- `save_and_apply_preserves_live_capture_when_defaults_and_unrelated_settings_change`.
- `saved_receiver_profile_restores_before_first_render_and_round_trips`.
- `saved_receiver_profile_import_applies_calibration_and_only_changed_hardware_restarts`.
- `setup_guide_and_settings_render_as_independent_popups`.

## Batch 3B — daemon credentials and main audio reconciliation: F06 and F07

Files: [radio_ui.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/radio_ui.rs), [app.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs), [source_manager.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/source_manager.rs), [audio_output.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/audio_output.rs), [secondary_vfo.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/secondary_vfo.rs).

F06: mount a masked daemon-token input in the **active** Radio source controls, next to Host/Reconnect. The legacy `SourceManager::ui_controls` password field is not mounted. Edit the actual selected source credential, persist through `SourcePreferences`, and reconnect using that credential. Empty tokens remain valid for an unauthenticated local daemon. Never show the token in status/errors/Debug. Test an authenticated loopback daemon through the current control path, persistence/restart, incorrect-token errors and clearing the token.

F07: track the main output's desired identity as `(AudioOutputSelection, input_channels)`, where selection has `device_id` and `sample_rate`. Use the existing B-receiver `ensure_output` behavior as a reference; do not duplicate its worker.

- Compare the effective selection every logic tick, including changes from imported settings. An actual identity change shuts down/restarts output asynchronously; unchanged identity does not.
- Preserve RF capture and daemon channel identity when only the sound device changes. Reset PCM/resampler state as required for the actual new output clock, without changing capture provenance.
- Preserve selected-device-unavailable errors and exact sample-rate validation. Do not fall back to another device.
- Respect active WAV recording geometry/completeness contracts; do not rewrite an active file's clock/header midway through a device change.

Use mocked output startup/selection to verify requested device/rate, restart counts and error visibility without a physical sound device. Verify A and B behave consistently under imported settings.

## Batch 3C — stable integrations and truthful service health: F08 and F17

Files: [app.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs), [web_remote.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/web_remote.rs), [rigctl.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/rigctl.rs), [secondary_panel.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/secondary_panel.rs), [config.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/config.rs).

- Make setters reconcile **desired configuration versus actual worker state**. The same enabled/port configuration on a healthy service must be a no-op. A previously failed service may retry explicitly; merely returning early on equal desired fields would incorrectly suppress retry.
- Applying observer/appearance/PPM settings must keep an unchanged rigctl connection and WebSocket alive. Preserve MQTT's existing epoch/backpressure handling.
- Clone the effective configuration under `SharedState`, release that mutex, then perform necessary service reconciliation. Joining/stopping a worker must not happen under it or synchronously block UI rendering.
- Expose status snapshots: stopped/starting/listening/failed, actual bound endpoint and sanitized error. Desired `enabled=true` is not proof a listener exists. Use the project's existing cross-thread snapshot/channel style; do not add an unbounded event queue.
- A port collision must appear in Settings and the common error surface. Freeing the port and retrying must recover. Changed ports must release the old listener and bind the new one.
- Change the Web Remote tooltip to describe loopback-only access. Keep `127.0.0.1`; do not make it LAN-accessible to satisfy the old text.

Tests: hold an occupied port; observe failure; release and retry; keep clients across identical apply; change port; repeated enable/disable; stop with idle and non-reading clients; assert bounded cleanup and no worker leaks.

## Batch 4 — visible errors and accurate Setup Guide: F05 and F09

Files: [app.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs), [status_bar.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/status_bar.rs), [quick_start.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/quick_start.rs), [app_ui_tests.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/app_ui_tests.rs).

F05: provide a compact shared error/status surface on every workspace, including Radio and ADS-B. Reuse the existing status messages; avoid a second disconnected error store. Preserve ADS-B receiver summary and map attribution. Important audio, recording, settings-save and daemon/control failures must be visible in the active workspace. Render alerts without requiring Settings to be opened. Test actual rendered text/shapes with injected failures in all three tabs, not just internal message insertion. Routine notifications may expire; persistent failures should remain actionable.

F09: replace external-only UAT wording with native IQ at exactly 2,083,333 samples/s and 978 MHz, with optional dump978-fa JSON compatibility. In `Workflow::apply_with_source`, propagate `config.save()` failure instead of unconditional success. In the wizard, retain an actionable error/retry state instead of advancing to Complete after failure. Preserve Meteor offline setup: it must not start/reconfigure an SDR or interrupt an ordinary radio recording. Test save failure with a temporary unwritable/invalid destination, retry success and existing offline-workflow tests.

## Batch 5 — browser session and input ownership: F10 and F11

Files: [daemon app.js](/home/lupc/Documents/ez-sdr/ez-daemon/src/web/frontend/app.js), [desktop web_remote.html](/home/lupc/Documents/ez-sdr/ez-gui/src/web_remote.html).

F10: give each connection attempt a monotonically increasing generation and capture its socket locally. Invalidate the generation on manual disconnect. Every socket callback and async `api('status')`/refresh completion must check that generation/socket still owns the active session before changing globals or UI. An old `onclose` must never close the replacement connection. Do not mark fully connected before the required control session is established. Prevent delayed polling from recreating streams after disconnect. Keep data-socket identity guards, authentication subprotocol and stream-gap handling.

Retaining finalized Meteor images for export is allowed. Label retained products with their session/product identity; retire prior products on the existing product-boundary message. Do not erase finalized images merely because the old probe observed retention.

F11: keep the telemetry frequency readout current but preserve a focused/dirty edit buffer. Distinguish edited/requested frequency from the last acknowledged value. Validate finite numeric input; do not send `NaN`. Clear/reconcile dirty state on successful submission/acknowledgement or explicit cancel/blur policy, not on every telemetry update.

Add deterministic JS tests derived from the audit VM harnesses, asserting corrected behavior: delayed old close after reconnect; delayed initial status after manual disconnect; obsolete refresh completion; focused edit survives telemetry; invalid frequency rejected; subsequent acknowledged value displayed. Maintain the numeric-IP/Origin security rules. Verify the tests evaluate the real shipped script, not a duplicate implementation.

## Batch 6 — real rigctl semantics and bounded client service: F12 and F13

Files: [rigctl.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/rigctl.rs), [app.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs), [README.md](/home/lupc/Documents/ez-sdr/README.md). Protocol reference: [Hamlib rigctld manual](https://hamlib.sourceforge.net/snapshots/man1/rigctld.1.html).

Required mappings:

- `f/F`: retain frequency get/set and existing error propagation.
- `m/M`: retain mode/passband support; verify returned tokens and framing against Hamlib. Do not turn unsupported modes into successful requests.
- `v`: return the current supported VFO token, not numeric volume.
- `V`: implement supported VFO selection semantics or explicitly return an unsupported error for unsupported VFOs. Do not reinterpret a token as volume or fake a B receiver that does not exist.
- `l AF`: return normalized audio gain in 0.0–1.0.
- `L AF <value>`: apply validated normalized gain; reject invalid/nonfinite values and unsupported level names.
- `q`: close that client, retaining the existing behavior.

Do not claim a full rigctld implementation. If capability discovery is required for the real client used in testing, add a truthful supported subset rather than invented capabilities. Update README's current incorrect `v/V` volume claim.

Serve a bounded number of independent clients (use a named small limit, such as eight). A sleeping first client must not prevent a second client's request. Track/reap client workers, bound command queues and replies, and set read/write deadlines so a client that stops reading cannot hang shutdown. Preserve fragmented commands, oversized-line discard/recovery, per-client request ordering and stop-aware waits. Do not spawn unlimited untracked threads.

Tests: two simultaneous clients with the first idle; normalized AF roundtrip; VFO tokens/unsupported errors; existing fragmented and oversized commands; excessive clients; non-reading client; stop and unchanged-config apply. Use an actual Hamlib client when available; otherwise retain that interoperability check as explicitly unverified.

## Batch 7 — owned daemon teardown on every exit: F14

Files: [daemon app.rs](/home/lupc/Documents/ez-sdr/ez-daemon/src/app.rs), [ingest.rs](/home/lupc/Documents/ez-sdr/ez-daemon/src/ingest.rs), [resilience.rs](/home/lupc/Documents/ez-sdr/ez-daemon/tests/resilience.rs).

- Validate configuration and bind both listeners before starting ingestion where possible. A TCP or web bind failure must not start/retain the source. Release a successfully bound first listener if the second bind fails.
- Own the running flag, server task handles and ingest join handle in a cleanup guard/lifecycle owner. Clearing the flag on a successful `select!` path alone is insufficient.
- On ordinary shutdown/error, stop both servers, await their cleanup and join ingestion via blocking-pool/off-thread work. Keep both task handles by selecting mutable references or another explicit ownership pattern; do not detach the unselected task.
- On future cancellation/drop, synchronously signal stop and abort/close owned listener tasks; arrange off-thread ingest reaping. `Drop` cannot await, and must not block the async executor. Ensure cleanup does not depend on a permanently detached task.
- Keep source read/cancel contracts and graceful recording finalization; cancelled/incomplete recordings must not be labelled complete. Match the `run` doc comment to the actual guarantees.

Tests: occupied TCP port; occupied web port; source-start failure; normal flag shutdown; abort the `run` task while the runtime remains alive. After each, wait for fake-source stop and listener release with a deadline; check no further sample production and no surviving ingest worker. Never make a test “clean up” a leaked worker by manually clearing a flag before asserting the daemon did so itself.

## Batch 8 — retained integrations and credentials: F16 and F19

Files: [discord.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/discord.rs), [config.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/config.rs), [bookmarks.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/bookmarks.rs), [secondary_panel.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/secondary_panel.rs).

F16: all plain/attachment/test send paths must check HTTP success, not merely `.send()` returning a response. Use finite request deadlines, bounded/sanitized response diagnostics and a delivery-status channel/snapshot consumed by the app. Distinguish enqueue/submission from delivery. Handle 401/403/429/5xx as failures; any retry must be bounded and honor rate-limit information without sleeping on the GUI thread. Only require `user_id` when pinging is enabled. Use an internal injectable HTTP target/transport for tests; production remains Discord's endpoint. Test with synthetic credentials and a loopback mock server, including timeout and queue saturation. Do not revive the retired Discord panel.

F19:

- Default config export must omit credentials. Provide an explicit opt-in for a private backup containing secrets and explain that choice in the export flow.
- Redacted import must preserve existing credentials for omitted keys; an explicitly supplied empty credential can clear it. Do not silently wipe credentials just because serde fills omitted fields with empty defaults.
- Replace secret-bearing derived `Debug` implementations with redacted output for `AppConfig`/Discord settings and any newly added status/config types. Keep daemon-token redaction.
- Use protected settings/secret-backup writes: on Unix create the temporary file owner-only (0600), then atomically rename. Do not write it world-readable first and chmod later. Preserve Windows per-user storage/ACL behavior. Use unique temporary filenames where concurrent saves/exports can target one destination; clean up failed temporary files.
- Keep ordinary bookmark CSV exports functional. Do not apply secret-handling defaults indiscriminately to nonsecret formats. Preserve existing load compatibility and atomic-save error feedback.

Tests: sentinel secrets absent from normal export and all Debug/status strings; explicit private backup roundtrip; omitted import secrets preserved; explicit clearing works; Unix permissions; failure leaves old settings intact; concurrent writes cannot share the same temporary file. Use temporary paths, never real credentials.

## Batch 9 — stable platform data paths: F21

Files: [app_data.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/app_data.rs), [main.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/main.rs), config/bookmark/history/map/recording path consumers, [DISTRIBUTION.md](/home/lupc/Documents/ez-sdr/DISTRIBUTION.md).

Introduce one platform path resolver and route file consumers through it rather than sprinkling OS-specific paths across widgets. Use Linux XDG config/data/cache locations with standard HOME fallbacks; macOS Application Support for durable data/config and the user cache location for tiles; retain Windows `%LOCALAPPDATA%\EZ-SDR` behavior and migration precedence. Ignore invalid relative XDG roots and fall back to a valid absolute user location.

- Capture the original launch directory before any relocation. Normalize saved relative replay/custom-output paths against their old base; do not redirect them into a new folder silently.
- Migrate legacy files only when the destination is absent. Existing per-user files win; originals remain intact. Reuse the established Windows migration behavior/tests where possible.
- Default recordings and tile cache use stable writable user locations. Do not move bundled resources or rely on the executable installation directory being writable.
- Show preparation/save failures explicitly. A missing source file is a normal user-facing replay error, not permission to discard its saved path.
- Make path resolution/migration testable with injected HOME/XDG/launch/executable paths. Avoid process-global `chdir` or environment mutation in parallel tests.

Acceptance: launch-location changes do not change effective settings/bookmarks/cache paths; read-only install directories work; relative replay/custom output retain their original meaning; migration is repeatable and non-overwriting; existing Windows tests pass. Native Finder/clean-platform launch remains a separate real-system gate.

## Batch 10 — finish scope cleanup carefully: F18

Files: app/mode bar/keyboard composition, [lib.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/lib.rs), [web_remote_async.rs](/home/lupc/Documents/ez-sdr/ez-gui/src/web_remote_async.rs), renderer-only portions of AI/Discord/scanner/scheduler/recorder/theme/source modules.

Use `rg`/graph queries to prove reachability from the current application before deleting anything. The recording/bookmark/scanner/scheduler **engines are not all dead**. Preserve keyboard/remote recording, quick bookmarks/spectrum markers, scheduler cooldown/runtime contracts, B-receiver and Meteor internals. Preserve test-only FEC/reference helpers.

Remove private never-opened Jump/Glossary flags/windows and truly unused renderer-only state. Correct README's frequency-jump claim to describe the actual direct-entry/marker paths. Remove or clearly deprecate the unused heartbeat-only `WebRemoteAsync` facade; it does not implement the real remote. Handle unused legacy config fields compatibly when reading old JSON. Do not delete entire public modules merely because their UI is unmounted; preserve documented reusable API or explicitly document intentional removal with tests/call-site cleanup.

For retained integrations, document their supported configuration/control route. Do not reintroduce old utility windows or make a repository-wide stylistic refactor. Record what was removed versus intentionally retained; `allow(dead_code)` alone is not proof.

## Batch 11 — packaging and honest field checklist: F22 and F23

Files: [Windows package script](/home/lupc/Documents/ez-sdr/scripts/package-windows-release.sh), [Linux package script](/home/lupc/Documents/ez-sdr/scripts/package-linux-release.sh), [release.yml](/home/lupc/Documents/ez-sdr/.github/workflows/release.yml), README/DISTRIBUTION/HARDWARE_VALIDATION.

F22: create the Windows package's `SHA256SUMS` before Inno Setup reads the directory. Ensure both installer and portable ZIP contain the same internal manifest. Keep the manifest from hashing itself and retain matching-source/native-license contents. Generate the Linux tarball's outer `.sha256` after assembly, and upload it in the release workflow beside the archive. Keep docs consistent with the actual filename/working directory for verification. Do not publish a release or push tags as part of these repairs.

Verify script syntax and manifest contents. Execute platform packaging on available matching runners/clean committed source; verify hashes after extraction/installation. A shell syntax pass is not a Windows installer test. Report unavailable platform execution honestly.

F23: update the field checklist to the current promises: receiver A/B, native UAT at 2,083,333 samples/s and 978 MHz, optional JSON compatibility, live/imported Meteor, pass finalization/export, visible errors, sound-device switching, RF reconnect and simultaneous recording. Keep unsigned/unnotarized Intel macOS scope explicit.

These gates require hardware/original data or target machines and cannot be “implemented” by a model:

- Actual antenna reception, audible playback and unplug/reconnect recovery.
- Concurrent receivers/recording soak with sample-loss/completeness evidence.
- Original off-air 80k Meteor comparison, not a re-encoded 72k fixture.
- Clean supported-platform installer/startup/native-library validation.

Leave them open until real evidence exists. Software fixes can be complete while these release gates remain unverified. Do not raise the project grade or claim field reliability from synthetic tests alone.

## Final verification and completion contract

Run focused tests as each batch changes behavior. Once repairs are integrated, run the repository's actual gates in a clean test process, without the audit's worker-priority/affinity intervention:

```sh
cargo fmt --check
cargo check --workspace --locked --all-targets
cargo clippy --workspace --locked --all-targets -- -D warnings
cargo clippy --workspace --locked --all-targets --no-default-features -- -D warnings
cargo test --workspace --locked
cargo test --workspace --locked --no-default-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --locked --no-deps
cargo test --workspace --locked --doc
cargo audit
cargo deny check
bash -n scripts/package-linux-release.sh scripts/package-windows-release.sh scripts/package-macos-release.sh
```

Run JS regression tests and syntax checks for the edited shipped scripts. Render actual widgets with the existing ignored preview test and inspect Radio/ADS-B/Meteor plus the Settings/Setup workflows at small and large sizes. Use a **new** private output directory, for example:

```sh
EZ_SDR_PREVIEW_DIR=/tmp/ez-sdr-fixed-previews cargo test -p ez-gui --locked --no-default-features --lib app::app_ui_tests::render_workspaces -- --exact --ignored --nocapture
```

Use the CI/native dependency setup already documented in README. If hardware features, cargo-audit/cargo-deny or platform tools are unavailable, record the missing check and use the appropriate available CI runner; do not write “all checks pass.” Preserve the existing documented bincode advisory exception; do not add new broad suppressions.

For each finding marked fixed, provide: commit, changed behavior, meaningful regression test, exact command/result and any remaining platform/field limitation. Preserve historical audit evidence and save fresh logs separately. No failed test, suppressed warning, discarded setting or unverified platform gate may be hidden behind a blanket “ship-ready” claim.

## Progress checklist

- [x] Batch 0: F15 worker teardown; F20 duplicate attributes/formatting repaired.
  - Verification: `cargo test -p ez-gui --locked --no-default-features discord::tests::` (31 passed); `cargo test -p ez-gui --locked --no-default-features demod::tests::demod_frame_versions_identify_latest_controls_under_iq_backlog -- --exact --nocapture` (1 passed in 0.49s); `cargo fmt --check` passed; `cargo clippy --workspace --locked --all-targets --no-default-features -- -D warnings` passed; `graphify update .` completed.
- [x] Batch 1: F02 exact ADS-B clock checks and failed-retune invariants.
  - Verification: `cargo test -p ez-daemon --locked --no-default-features state::tests::` (34 passed); `cargo test -p ez-daemon --locked --no-default-features pipelines::packet::` (33 passed); `cargo fmt --check` passed; `graphify update .` completed. The pre-fix local `core-probe` reproduced a 3.2 MHz tap and accepted ADS-B subscription.
- [x] Batch 2: F03 serialized geometry changes and deterministic race tests.
  - Verification: `cargo test -p ez-daemon --locked --no-default-features state::tests::` (37 passed); `cargo test -p ez-daemon --locked --no-default-features recording::capture_tests::` (2 passed); `cargo fmt --check` passed; `cargo clippy --workspace --locked --all-targets --no-default-features -- -D warnings` passed; `graphify update .` completed. The gated rate/subscribe test confirmed status readers remain available, first-channel creation waits, and both hardware and channelizer report 1.2 MHz afterward. Gated rejection preserved the verified 2.4 MHz geometry; a timed-out acknowledgement blocked new geometry until its final outcome was reconciled.
- [x] Batch 3A: F01/F04 popup draft/import/reset ownership and persistence ordering.
  - Verification: `cargo test -p ez-gui --locked --no-default-features app::app_ui_tests::settings_` (5 passed); `cargo test -p ez-gui --locked --no-default-features config::tests::` (23 passed); `cargo fmt --check` passed; `cargo clippy --workspace --locked --all-targets --no-default-features -- -D warnings` passed; `graphify update .` completed. The broader `app::app_ui_tests::` run reported 41 passed, 1 failed, 1 ignored; `daemon_squelch_spectrum_and_websocket_routes_apply_to_real_audio` also failed in isolation while waiting for audible synthetic daemon PCM. Its cause remains open; the Settings tests and retained receiver-profile/popup tests passed.
- [x] Batch 3B: F06 authenticated source UI; F07 audio A/B reconciliation.
  - Verification: `cargo test -p ez-gui --locked --no-default-features app::app_ui_tests::active_daemon_controls_mask_authenticate_persist_reconnect_and_clear_token -- --exact --nocapture` (1 passed; edited the mounted password field, connected to an authenticated loopback daemon through Reconnect, round-tripped preferences across app construction, rejected a wrong token without exposing it, then cleared it and connected to an unauthenticated daemon); `cargo test -p ez-gui --locked --no-default-features app::app_ui_tests::imported_audio_selection_restarts_a_and_b_once_without_changing_capture -- --exact --nocapture` (1 passed; mocked A/B startup saw exactly the requested device/rate once per identity, preserved capture identity, and finalized the 44.1 kHz WAV before switching to 48 kHz); `cargo test -p ez-gui --locked --no-default-features app::app_ui_tests::unavailable_main_audio_selection_keeps_exact_request_and_reports_error -- --exact --nocapture` (1 passed; exact missing-device request and visible error); `cargo test -p ez-gui --locked app::app_ui_tests::imported_audio_selection_restarts_a_and_b_once_without_changing_capture -- --exact --nocapture` (1 passed with default audio features); `cargo test -p ez-gui --locked --no-default-features audio_output::tests::` (6 passed); `cargo test -p ez-gui --locked --no-default-features secondary_vfo::tests::` (5 passed); `cargo fmt --check` passed; `cargo clippy --workspace --locked --all-targets --no-default-features -- -D warnings` passed; `cargo clippy -p ez-gui --locked --all-targets -- -D warnings` passed; `graphify update .` completed. An initial unavailable-output test fixture had an idle source and therefore never requested audio; the fixture was corrected to model active reception and the exact test then passed. Hardware device switching remains unverified without a physical output device.
- [x] Batch 3C: F08 unchanged service preservation; F17 operational health.
  - Verification: `cargo test -p ez-gui --locked --no-default-features web_remote::tests::` (22 passed); `cargo test -p ez-gui --locked --no-default-features rigctl::tests::` (8 passed, 2 existing loopback tests remain ignored); `cargo test -p ez-gui --locked --no-default-features app::app_ui_tests::unrelated_settings_apply_preserves_web_remote_and_rigctl_workers -- --exact --nocapture` (1 passed); `cargo test -p ez-gui --locked --no-default-features app::app_ui_tests::occupied_web_remote_port_is_visible_in_settings_and_status_surface -- --exact --nocapture` (1 passed); `cargo fmt --all --check` passed; `cargo clippy --workspace --locked --all-targets --no-default-features -- -D warnings` passed; `graphify update .` completed. The first new WebSocket test exposed that the 500 ms shutdown timeout was applied to the entire server lifetime; shutdown now starts the grace timer only after stop is requested, and the regression confirms the listener remains live beyond 600 ms. Tests also cover occupied-port retry, actual endpoint, same-config WebSocket/client preservation, changed-port release, repeated toggles, and bounded idle/non-reading-client stop. The currently mounted status strip is still workspace-dependent; Batch 4 covers cross-workspace visibility.
- [x] Batch 4: F05 visible failures; F09 guide accuracy/save feedback.
  - Verification: `cargo test -p ez-gui --locked --no-default-features status_bar::tests::` (5 passed); `cargo test -p ez-gui --locked --no-default-features quick_start::tests::` (5 passed, including invalid-destination save failure/retry, exact native UAT description and Meteor offline preservation); `cargo test -p ez-gui --locked --no-default-features app::app_ui_tests::shared_failure_surface_renders_on_radio_adsb_and_meteor -- --exact --nocapture` (1 passed with injected audio, daemon, recording, source and config-save errors rendered on all three tabs while retaining ADS-B summary/map attribution); `cargo fmt --all --check` passed; `cargo clippy --workspace --locked --all-targets --no-default-features -- -D warnings` passed; `graphify update .` completed. The first rendered-shape check found the shared panel had no reserved height on the newly covered tabs; restored dynamic 26/64 px sizing and reran successfully. Updated `UAT_RECEIVER.md` to make native 978 MHz / 2,083,333 samples/s the default and document dump978-fa as optional JSON compatibility.
- [x] Batch 5: F10 session-generation guards; F11 protected edit buffer.
  - Verification: `node --check ez-daemon/src/web/frontend/app.js` passed; `node --check ez-daemon/tests/web_frontend_regressions.js` passed; `node ez-daemon/tests/web_frontend_regressions.js` passed. The harness evaluates the shipped daemon script and extracts/evaluates the inline desktop page script. It covers delayed old close after reconnect, delayed initial status and Welcome after disconnect, stale refresh completion, focused frequency edits, invalid input rejection and acknowledged frequency reconciliation. `graphify update .` completed. The first harness assertion incorrectly expected the retired stream to stay open after reconnect; corrected it to verify the replacement stream remains open while the retired stream is closed.
- [x] Batch 6: F12 Hamlib semantics; F13 bounded independent clients.
  - Verification: `cargo test -p ez-gui --locked --no-default-features rigctl::tests::` (9 passed, 2 pre-existing permission-gated tests ignored); `cargo test -p ez-gui --locked --no-default-features app::app_ui_tests::rigctl_subset_uses_hamlib_vfo_modes_and_normalized_audio_gain -- --exact --nocapture` (1 passed through a live loopback server and app request path); `cargo fmt --all --check` passed; `cargo clippy --workspace --locked --all-targets --no-default-features -- -D warnings` passed; `graphify update .` completed. The concurrent-client test leaves client one idle while client two sends and receives a command; additional tests cover the eight-client cap, bounded request queue, non-reading stop, fragmentation, oversized recovery and listener reconfiguration. The app-level test verifies `v`/`V`, Hamlib `FM` mode/passband framing, unsupported mode/VFO errors, and normalized `AF` roundtrip. Hamlib manual checked: `f/F`, `m/M`, `v/V`, and `l/L` semantics; no `rigctl` executable is installed, so external-client interoperability remains unverified. Initial strict Clippy found a test-only redundant pattern assertion, which was simplified and the full check rerun successfully.
- [x] Batch 7: F14 owned startup/error/cancellation cleanup.
  - Verification: `cargo test -p ez-daemon --locked --no-default-features app::tests::` (7 passed, including occupied TCP/web bind, source-start failure, flag shutdown with an active WebSocket and `run` cancellation); `cargo test -p ez-daemon --locked --no-default-features ingest::tests::` (12 passed); `cargo test -p ez-daemon --locked --no-default-features --test resilience` (5 passed); `cargo test -p ez-daemon --locked --no-default-features web::ws::tests::` (10 passed); `cargo fmt --all --check` passed; `cargo clippy --workspace --locked --all-targets --no-default-features -- -D warnings` passed; `graphify update .` completed. Both listeners are bound before source startup; source startup failures stop the source, ordinary shutdown joins both servers and ingestion, and cancellation signals stop/aborts listener tasks and reaps ingestion off the async executor. Fake-source tests verified stop and no further reads after shutdown.
- [x] Batch 8: F16 delivery errors; F19 private persistence/redacted exports.
  - Verification: `cargo test -p ez-gui --locked --no-default-features discord::tests::` (40 passed; loopback tests covered 201/401/403/429/500 responses, multipart and test sends, a 30 ms timeout, queue saturation, sanitized delivery snapshots, and ping-disabled configuration); `cargo test -p ez-gui --locked --no-default-features config::tests::` (28 passed; redacted export, private roundtrip, preserve-on-omission, explicit credential clearing, Debug redaction, private file mode and save failure/retry); `cargo test -p ez-gui --locked --no-default-features bookmarks::tests::` (11 passed; atomic temp uniqueness/cleanup, Unix 0600, and ordinary CSV behavior); `cargo test -p ez-gui --locked --no-default-features app::app_ui_tests::discord_delivery_result_reaches_shared_status_surface -- --exact --nocapture` (1 passed); `cargo fmt --all --check` passed; strict workspace Clippy passed; `graphify update .` completed. HTTP tests used only loopback endpoints; production transport remains fixed to Discord. Windows ACL inheritance is preserved by same-directory temporary creation but was not verified on Windows.
- [x] Batch 9: F21 platform paths/migration.
  - Verification: `cargo test -p ez-gui --locked --no-default-features app_data::tests::` (7 passed; Linux/XDG, macOS, simulated Windows, launch-location independence, migration precedence/repeatability, and invalid storage); `cargo test -p ez-gui --locked --no-default-features spectrum::tests::signal_history_uses_configured_path_and_reports_save_errors -- --exact` (1 passed); `cargo test -p ez-gui --locked --no-default-features app::app_ui_tests::restored_file_source_and_mode_stay_idle_with_usable_legacy_settings -- --exact` (1 passed; missing replay path remains saved); `cargo test -p ez-gui --locked --no-default-features bookmarks::tests::` (11 passed); `cargo fmt --all --check` passed; `cargo clippy -p ez-gui --locked --all-targets --no-default-features -- -D warnings` passed; `graphify update .` completed. macOS and Windows path resolution were simulated on Linux; Finder startup and native Windows ACL behavior still need platform runners.
- [x] Batch 10: F18 scoped dead UI/state cleanup; useful engines retained.
  - Verification: `cargo fmt --all --check` passed; `cargo check -p ez-gui --locked --all-targets --no-default-features` passed; `cargo clippy --workspace --locked --all-targets --no-default-features -- -D warnings` passed; `cargo test -p ez-gui --locked --no-default-features app::app_ui_tests::` (50 passed, 1 ignored); `cargo test -p ez-daemon --locked --no-default-features synthetic_daemon_voice_tone_emits_fm_pcm_after_channelization` (1 passed); focused legacy-config and frequency-entry regressions passed; `graphify update .` completed. Removed the unreachable Jump/Glossary windows and unused render state while retaining reusable integration panels and engines with their actual configuration routes documented. The app test fixture previously constructed reqwest's blocking client inside async Tokio tests; those synchronous fixture creations now run in a blocking region. The audio observer also uses its own connection-owned channel ID and mirrored squelch settings. The formerly failing synthetic audio check now passes with the observer reading RAW PCM; a separate daemon regression proves the generated -400 kHz FM voice tone produces nonzero PCM through channelization and demodulation.
- [ ] Batch 11: F22 manifests; F23 current validation scope documented.
- [ ] Final software gates run with exact results; remaining load-sensitive test investigated.
- [ ] Commits complete; only intentional work staged; graph refreshed after code changes.
- [ ] Hardware/platform/off-air gates separately marked passed with evidence or still open.

Suggested handoff prompt: **“Read `/home/lupc/Documents/ez-sdr/fix.md` and implement its software repairs in batch order. Preserve existing unrelated changes. Add meaningful regression tests, run the specified checks, update the checklist and commit each verified batch. Keep hardware/platform gates explicitly open when evidence is unavailable.”**
