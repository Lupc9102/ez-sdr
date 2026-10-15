# EZ-SDR deep ship-readiness audit

**Grade: 64/100. Verdict: suitable for a constrained beta; not ready for a broad stable release.**

Audited source: `e0f2d3b6cde2f9601e7a18fdd2d89ae75b2836be`. Work began 13 October and concluded 14 October 2026. This is a fresh audit of the current three-workspace application, not an endorsement of older audit reports. Application code was not repaired as part of this request.

The project has substantial, real receiver and decoder engineering. Its strongest assets are bounded streaming, physical capture-clock metadata, RF/audio separation, native ADS-B/UAT, and increasingly thorough DSP fixtures. The largest remaining weaknesses are settings ownership, user-visible error reporting, transactional daemon controls, protocol interoperability, and release validation. A large test count does not cover these integration boundaries automatically.

The score is a judgment of readiness for ordinary users within the advertised Radio/ADS-B/Meteor scope, not a measured probability of success or a percentage of competitor features implemented. Missing NOAA/GOES decoding, unlimited VFOs, and a plugin ecosystem are scope choices. They are not counted as bugs. The recent retirement of utility windows is likewise treated as a product decision; residual unreachable code and active legacy machinery are maintenance debt.

## Scoring rubric

- **Signal/data correctness: 20/25.** Good rational-clock, alias-rejection, provenance and decoding fixtures; an accepted ADS-B clock mismatch remains.
- **Desktop workflows/settings: 11/20.** Compact working receiver UI and genuine floating settings/setup windows; calibration edits, imported settings, authentication entry and error feedback have holes.
- **Lifecycle/concurrency: 10/15.** Bounded queues and many recovery tests; a deterministic rate-change race, daemon startup cleanup leak, browser reconnect race and notifier lifetime leak remain.
- **Security/network boundaries: 8/10.** Loopback defaults, optional authenticated transports, Origin/Host checks and bounded frames are good. Credential persistence and diagnostics need hardening; remote deployment still requires the documented trusted-network/TLS boundary.
- **Maintenance/coherent scope: 6/10.** A six-crate workspace with real module separation, but large coupled GUI modules and retained obsolete renderers obscure ownership and inflate test coverage impressions.
- **Distribution: 4/10.** Platform scripts, matching-source archives and licenses exist. Checksum packaging has concrete defects, macOS remains unsigned/unnotarized, and this audit did not run installers on clean target systems.
- **Validation/release gates: 5/10.** Extensive automated tests exist; current formatting and strict Clippy gates fail, field evidence remains open, and the interrupted default-feature suite must not be presented as a complete pass.

Total: **64/100**. Several high-impact defects can be fixed without redesigning the DSP. The remaining field/platform gates require actual target-system evidence.

## Method and limits

The tracked Rust corpus contains 184 files and approximately 115,915 lines, including tests and translated decoder code. Review covered the six workspace crates, the current GUI composition and configuration paths, daemon state/control transports, channelization, decoder handoff, recording/replay boundaries, side services, both browser clients, CI and all three packaging scripts. Graphify was queried first for navigation, then findings were checked against current source. Historical reports were treated as leads only.

Five focused local probes exercise existing compiled code or evaluate the actual shipped JavaScript with a minimal DOM/WebSocket harness. They cover concurrent capture-rate changes, actual ADS-B tap rates, failed daemon startup, rigctl command parsing/client lifetime, Discord teardown, browser reconnects and focused frequency input. The popup calibration probe renders the real Radio sidebar with a pending settings value. JavaScript probes validate logic and event ordering; they are not full browser layout or autoplay tests.

No SDR was attached in this session's USB inventory. No antenna reception, audible output, Windows installer execution, macOS Finder launch, clean-machine Linux package install, or new original off-air 80k Meteor comparison was performed. HackRF/Soapy native libraries and cargo-audit/cargo-deny were unavailable locally. Their CI definitions are not substitutes for fresh successful runs. Validation results and exact logs are recorded in [verification.md](/home/lupc/Documents/ez-sdr/audit/2026-10-13/verification.md).

P1 means a defect that should be resolved before a stable release because it breaks a central advertised workflow or data correctness. P2 means a substantive conditional workflow, reliability or interoperability defect. P3 means cleanup/documentation debt. Confirmed reproduction, source-confirmed behavior and unverified runtime consequences are distinguished below.

## Highest-priority findings

### F01 — P1: the Settings calibration edit is overwritten by the Radio sidebar

**Evidence:** [config.rs:958](/home/lupc/Documents/ez-sdr/ez-gui/src/config.rs:958), [app.rs:2151](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:2151), [radio_ui.rs:503](/home/lupc/Documents/ez-sdr/ez-gui/src/radio_ui.rs:503).

The popup writes `config.ppm_correction`. In the same UI frame the active Radio workspace renders its Source section, which unconditionally copies `source.ppm_correction` back into that field. With Hardware selected and Source expanded, a new popup value is erased by the old live value before the next application step. Keeping Source collapsed or editing the Radio-side PPM control changes the outcome, which makes the problem difficult to diagnose.

The focused probe sets the pending popup preference to 37 ppm while the source remains at 3 ppm, renders the real sidebar, and checks that the preference becomes 3 ppm. This is a state-ownership defect, not a cosmetic slider issue. Give the popup draft settings or a pending command, apply them before rendering live-to-config synchronization, and add a real popup-edit/apply regression test.

### F02 — P1: accepted daemon ADS-B channels can run at the wrong physical clock

**Evidence:** [state.rs:212](/home/lupc/Documents/ez-sdr/ez-daemon/src/state.rs:212), [channelizer.rs:469](/home/lupc/Documents/ez-sdr/ez-daemon/src/channelizer.rs:469), [state.rs:512](/home/lupc/Documents/ez-sdr/ez-daemon/src/state.rs:512), [packet.rs:70](/home/lupc/Documents/ez-sdr/ez-daemon/src/pipelines/packet.rs:70).

Validation requires the requested ADS-B channel rate to equal 2.4 MHz, but the channelizer guarantees a minimum rate through integer decimation. A 3.2 MHz capture requesting 2.4 MHz uses decimation one and produces 3.2 MHz blocks. `PacketPipeline` logs that the rate is wrong and still constructs the fixed `Demod2400` decoder. Its block path does not reject the incompatible rate.

The probe observed `requested=2400000; actual=3200000` and a successfully accepted ADS-B subscription. This is an exact configuration error, not a demonstrated quantitative sensitivity loss; no off-air comparison was performed. Reject incompatible actual rates before creating the pipeline, or implement a validated exact-rate conversion. Cover every supported capture rate and retune path.

### F03 — P1: capture-rate changes race with channel creation and leave inconsistent state

**Evidence:** [state.rs:273](/home/lupc/Documents/ez-sdr/ez-daemon/src/state.rs:273), [state.rs:371](/home/lupc/Documents/ez-sdr/ez-daemon/src/state.rs:371), [channelizer.rs:554](/home/lupc/Documents/ez-sdr/ez-daemon/src/channelizer.rs:554).

`set_sample_rate` checks for virtual channels, releases the channel lock, waits for the hardware change, and only then updates the channelizer. A concurrent subscriber can insert an audio channel during that wait. The hardware setter succeeds, while the channelizer setter rejects the update because the new virtual channel exists. The returned error does not roll hardware back.

A gated test source reproduced hardware at 1.2 MHz after an error while the state still accepted a new 2.4 MHz spectrum specification against its old geometry. Subsequent controls and DSP validation disagree with the physical source. Serialize geometry-changing controls and channel creation using an explicit transaction/reservation; avoid holding a frequently used mutex through potentially slow driver operations. Test concurrent creation and rejected hardware operations.

## Settings, popup and desktop workflow findings

### F04 — P2: import/reset can lose display values before they are applied

**Evidence:** [config.rs:910](/home/lupc/Documents/ez-sdr/ez-gui/src/config.rs:910), [config.rs:1017](/home/lupc/Documents/ez-sdr/ez-gui/src/config.rs:1017), [app.rs:1307](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:1307), [app.rs:2596](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:2596), [radio_ui.rs:1941](/home/lupc/Documents/ez-sdr/ez-gui/src/radio_ui.rs:1941).

Import completion and Reset replace the config in popup rendering and set `needs_apply`. The pending config is applied in the next logic step. Radio rendering meanwhile persists the existing spectrum into the newly imported/reset config. FFT/window, waterfall, smoothing, grid, range and color fields can therefore inherit the old display instead of the requested profile. Source-confirmed frame ordering; a full native file-dialog interaction was not exercised. Stage replacements and apply them before synchronization, then test importing deliberately different display settings with Radio visible and hidden.

### F05 — P2: transient failures disappear from Radio and much of ADS-B

**Evidence:** [app.rs:2167](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:2167), [app.rs:2448](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:2448), [app.rs:1777](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:1777).

Radio omits the shared status strip entirely. The ADS-B branch displays receiver status and aircraft count but does not render `status_bar.current()`. Code still publishes audio failure/retry messages, recording and runtime warnings, and bookmark feedback through that status bar. Consequently some correctly detected failures are invisible in the user's active workspace. Settings save errors remain visible inside Settings; ADS-B has its own receiver feedback and toasts, so this is not a claim that all errors are hidden. Add a compact persistent error/toast surface shared by every workspace and verify injected audio/recording/control errors are visible.

### F06 — P2: authenticated daemon credentials have no current GUI entry control

**Evidence:** [radio_ui.rs:279](/home/lupc/Documents/ez-sdr/ez-gui/src/radio_ui.rs:279), [source_manager.rs:2483](/home/lupc/Documents/ez-sdr/ez-gui/src/source_manager.rs:2483), [config.rs:500](/home/lupc/Documents/ez-sdr/ez-gui/src/config.rs:500).

The current daemon source controls expose Host and Reconnect, but not `daemon_token`. A password control exists in the old `SourceManager::ui_controls` renderer, which has no current production caller. Authentication itself exists and the token persists; an ordinary user must configure JSON outside the current source UI and restart with that preference. Mount a masked token input in the active controls, apply it to reconnect, and test an authenticated handshake through that workflow.

### F07 — P2: changing sound-device settings through config import does not reconcile the main audio output

**Evidence:** [app.rs:1307](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:1307), [app.rs:1344](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:1344), [app.rs:2522](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:2522), [secondary_vfo.rs:468](/home/lupc/Documents/ez-sdr/ez-gui/src/secondary_vfo.rs:468).

The normal Audio sidebar sets `audio_changed` and shuts down the old stream. Import/apply bypasses that flag, and the main output's reset identity tracks capture geometry and channel count rather than the selected output device/rate. A running output can retain the old selection until another reset. The B receiver explicitly compares `(selection, channels)`, showing inconsistent reconciliation. Source-confirmed; physical audio-device switching was not tested. Make the main output reconcile the selected device/rate directly, including imported settings.

### F08 — P2: every Settings apply restarts unrelated remote services

**Evidence:** [app.rs:1321](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:1321), [rigctl.rs:80](/home/lupc/Documents/ez-sdr/ez-gui/src/rigctl.rs:80), [web_remote.rs:232](/home/lupc/Documents/ez-sdr/ez-gui/src/web_remote.rs:232).

Applying observer/calibration or other settings calls `set_enabled` on Web Remote and rigctl even when their configuration is identical. Both setters stop and restart. The rigctl lifecycle probe confirms an existing client disconnects on identical configuration. These calls happen while holding shared GUI state. The stop-aware rigctl polling normally bounds shutdown to about one poll; this audit does not claim a 750 ms freeze. Preserve services when effective configuration is unchanged, and move necessary shutdown/rebind work outside GUI state locks.

### F09 — P2/P3: Setup Guide still gives obsolete UAT requirements and ignores save failure

**Evidence:** [quick_start.rs:67](/home/lupc/Documents/ez-sdr/ez-gui/src/quick_start.rs:67), [quick_start.rs:85](/home/lupc/Documents/ez-sdr/ez-gui/src/quick_start.rs:85), [quick_start.rs:404](/home/lupc/Documents/ez-sdr/ez-gui/src/quick_start.rs:404), [quick_start.rs:441](/home/lupc/Documents/ez-sdr/ez-gui/src/quick_start.rs:441).

The guide says 978 MHz UAT requires external dump978-fa, while the current default native IQ decoder and distribution documentation say it does not. `Workflow::apply_with_source` also discards the boolean result of `config.save` and returns success. A failed disk save can advance to Complete with settings retained only in memory. The error is retained elsewhere in config, but the success flow should show the distinction. Update the guide and make completion conditional on or explicit about persistence success.

## Browser, protocol and side-service findings

### F10 — P2: an obsolete control-socket close event disconnects a new browser session

**Evidence:** [app.js:36](/home/lupc/Documents/ez-sdr/ez-daemon/src/web/frontend/app.js:36), [app.js:62](/home/lupc/Documents/ez-sdr/ez-daemon/src/web/frontend/app.js:62).

Disconnect closes the old socket; reconnect installs a new global `control` and sets `connected=true`. The old socket's delayed `onclose` checks that global flag and calls `disconnect`, which tears down the new socket. The shipped-JavaScript probe reproduced this event ordering. Bind callbacks to a session generation/socket identity, as the data-socket close callback already does. Also guard in-flight connect/refresh completions against a later manual disconnect.

The probe additionally observed retained Meteor products after disconnect. Keeping finalized images for export can be intentional, so retention alone is not counted as a bug; label their capture/session and avoid presenting them as fresh data.

### F11 — P2: telemetry overwrites a focused frequency input in desktop Web Remote

**Evidence:** [web_remote.html:147](/home/lupc/Documents/ez-sdr/ez-gui/src/web_remote.html:147), [web_remote.html:156](/home/lupc/Documents/ez-sdr/ez-gui/src/web_remote.html:156).

Every state update assigns `freqInput.value` without checking focus or whether the user is editing. The probe changed focused text to `118700000`; the next update replaced it with `100000000`. Normal telemetry can defeat manual entry before Tune is clicked. Preserve editing state and distinguish requested/current/acknowledged frequency.

### F12 — P2: rigctl uses incompatible Hamlib command semantics

**Evidence:** [rigctl.rs:272](/home/lupc/Documents/ez-sdr/ez-gui/src/rigctl.rs:272), [app.rs:584](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:584), [Hamlib's current rigctld manual](https://hamlib.sourceforge.net/snapshots/man1/rigctld.1.html).

EZ-SDR implements `v/V` as volume get/set, in percentage units. Hamlib defines them as VFO get/set; audio gain uses `l AF` / `L AF` with a normalized value. The live loopback probe observed `v` parsed as `GetVolume`, a numeric response, and `RPRT -1` for `V VFOB`, `l AF` and `L AF 0.5`. Frequency/mode support is a useful subset, but volume semantics make the stated compatibility misleading. Implement the standard mappings, expose capabilities honestly, and test through a real Hamlib client. These are protocol defects, not merely missing optional commands.

### F13 — P2: one idle rigctl client prevents another from receiving service

**Evidence:** [rigctl.rs:113](/home/lupc/Documents/ez-sdr/ez-gui/src/rigctl.rs:113), [rigctl.rs:162](/home/lupc/Documents/ez-sdr/ez-gui/src/rigctl.rs:162).

The accept loop processes each connection synchronously until it closes. An idle first connection monopolizes the worker; a second TCP connection can establish but its commands are not read. The lifecycle probe confirms this condition. Support a bounded set of independent connections, or enforce an explicit idle timeout/single-client policy. Writes also lack a timeout; shutdown under a client that stops reading is a source-level concern that was not stress-reproduced.

### F14 — P2: daemon startup failure leaks the ingest worker in a surviving process

**Evidence:** [app.rs:155](/home/lupc/Documents/ez-sdr/ez-daemon/src/app.rs:155), [app.rs:168](/home/lupc/Documents/ez-sdr/ez-daemon/src/app.rs:168), [app.rs:183](/home/lupc/Documents/ez-sdr/ez-daemon/src/app.rs:183).

Ingestion begins before either listener is bound. A bind error returns through `?`, leaving the shared running flag true and dropping the join handle. The probe deliberately occupied the TCP port and found one surviving ingestion thread after `run` returned an error. Linux truncates thread names to 15 bytes; the probe handles that explicitly. The CLI's process exit cleans up OS resources, so the demonstrated persistent leak applies to embedded/test/retry callers that keep their process alive. Cancelling the run future also lacks the cleanup guard promised in its doc comment and can detach spawned server tasks; that cancellation consequence is source-confirmed, not dynamically reproduced here. Use an owned teardown guard and await/abort all owned workers on every exit path.

### F15 — P2: Discord teardown spins after its queue is disconnected

**Evidence:** [discord.rs:1019](/home/lupc/Documents/ez-sdr/ez-gui/src/discord.rs:1019).

The detached worker loops forever and only handles `Ok` from `rx.recv()`. Dropping the notifier disconnects the receiver, making `recv` immediately return an error indefinitely. The local probe showed retained tasks and approximately 68 CPU ticks over 700 ms after drop. A normal single-window process exit terminates them, but repeated construction/teardown and the GUI test harness accumulate hot workers. The full GUI test process was observed with more than 100 threads and roughly 1,100% CPU while waiting for remaining DSP tests; the isolated probe establishes the leak, while that process snapshot does not attribute every thread or CPU cycle to Discord. Break on disconnect and own/join the worker where appropriate.

### F16 — P2: Discord HTTP errors are treated as successful sends

**Evidence:** [discord.rs:1066](/home/lupc/Documents/ez-sdr/ez-gui/src/discord.rs:1066), [discord.rs:1092](/home/lupc/Documents/ez-sdr/ez-gui/src/discord.rs:1092), [discord.rs:1234](/home/lupc/Documents/ez-sdr/ez-gui/src/discord.rs:1234).

Plain, attachment and test sends call `.send()` and return success without examining the HTTP status. Authentication, permission, rate-limit and server-error responses can be reported as success. No real Discord message was sent during this audit. This is a source-confirmed response-handling error in retained integration code; its current GUI is retired, but imported existing credentials can still enable the notifier. Check status/body, handle rate limits, use finite network deadlines and expose delivery failures. Requiring a nonempty `user_id` even when pings are disabled is another avoidable configuration restriction.

### F17 — P2/P3: remote-service UI does not reflect operational state

**Evidence:** [config.rs:990](/home/lupc/Documents/ez-sdr/ez-gui/src/config.rs:990), [web_remote.rs:274](/home/lupc/Documents/ez-sdr/ez-gui/src/web_remote.rs:274), [rigctl.rs:97](/home/lupc/Documents/ez-sdr/ez-gui/src/rigctl.rs:97).

The Web Remote tooltip promises access from a browser on the LAN, but the server binds only to `127.0.0.1`. The loopback restriction is a sensible security boundary; fix the promise rather than exposing unauthenticated controls. Web Remote and rigctl bind failures go to stderr while their enabled preferences remain true. Settings has detailed MQTT health but no equivalent bound/failed/retrying state for these services. An occupied port can therefore look enabled while serving nothing. Show operational status and errors separately from user preference.

## Dead code, incomplete retirement and maintenance

### F18 — P3: retired utilities leave substantial orphan renderers and state

**Evidence:** [mode_bar.rs:16](/home/lupc/Documents/ez-sdr/ez-gui/src/mode_bar.rs:16), [app.rs:1336](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:1336), [app.rs:2180](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:2180), [lib.rs:23](/home/lupc/Documents/ez-sdr/ez-gui/src/lib.rs:23).

Only Radio, ADS-B and Meteor are selectable; Tools contains Setup Guide and Settings. Current production composition does not mount `RecorderPanel::ui`, `FrequencyScanner::ui`, the scheduler panel UI, `AiPanel::ui`, `DiscordPanel::ui`, `ThemeConfig::ui_editor`, or the legacy `SourceManager::ui_controls`. The Satellites renderer remains behind a nonselectable enum variant and private legacy transition. `show_freq_jump` and `show_glossary` start false and have no production assignment to true. The README's frequency-jump workflow is consequently stale.

This does **not** mean entire recording, bookmark, scanner or scheduler engines are dead: recording still has keyboard/remote/runtime entry points, bookmarks still have quick creation and spectrum markers, and several services continue ticking. Public library renderers can also be called by external consumers. The finding is application-level reachability and incomplete scope cleanup, not a fabricated linker dead-code percentage.

Specific safe cleanup candidates after checking external API commitments:

- Orphan renderer entry points and their renderer-only state/tests.
- The two never-opened popup implementations and their flags.
- `WebRemoteAsync`, which has no application callers and only advances a heartbeat; its comment claiming it replaces the actual Web Remote implementation is false.
- `AdvancedSettings.ai_context_window`, which is declared/defaulted but never consumed.
- Staged antenna-alignment UI and AI vision helpers with explicit dead-code allowances and no mounted workflow.

Keep test-only FEC encoders, decoder reference helpers and public reusable APIs where they have a documented purpose. `allow(dead_code)` alone is not evidence of a defect. Decide which integrations remain supported, then either expose their operational configuration intentionally or retire their startup/tick state as well. Do not restore every old window merely to increase a feature count.

The largest GUI modules have thousands of lines, and `CentralApp` coordinates many optional subsystems through shared state. Refactor around command application and explicit effective configurations rather than splitting files mechanically. The settings overwrite and unconditional restart bugs show the practical cost of multiple owners, not just a style preference.

### F19 — P2/P3: credential persistence/export is insufficiently isolated

**Evidence:** [config.rs:581](/home/lupc/Documents/ez-sdr/ez-gui/src/config.rs:581), [config.rs:863](/home/lupc/Documents/ez-sdr/ez-gui/src/config.rs:863), [config.rs:1028](/home/lupc/Documents/ez-sdr/ez-gui/src/config.rs:1028), [bookmarks.rs:236](/home/lupc/Documents/ez-sdr/ez-gui/src/bookmarks.rs:236).

Full config serialization includes daemon, AI and Discord credentials. Export writes that complete object, and the generic atomic writer uses ordinary `std::fs::write` without enforcing owner-only Unix permissions. Under a common 022 umask a new file can be group/world readable; this is a conditional platform exposure, not a measured leak of actual user credentials. `SourcePreferences::Debug` redacts the daemon token, but derived Debug on the surrounding config/Discord settings still includes other credentials. Offer a credential-free export, redact diagnostic types consistently, and use OS secret storage or protected local files. No real secrets were printed or transmitted by the probes.

## Release and documentation findings

### F20 — P2: current CI quality gates fail on the audited revision

**Evidence:** [config.rs:1285](/home/lupc/Documents/ez-sdr/ez-gui/src/config.rs:1285), [config.rs:1292](/home/lupc/Documents/ez-sdr/ez-gui/src/config.rs:1292), [.github/workflows/ci.yml](/home/lupc/Documents/ez-sdr/.github/workflows/ci.yml).

`cargo fmt --check` fails in `adsb_panel.rs` and `app.rs`. The strict no-default-features all-targets Clippy command fails on duplicated `#[test]` attributes in two config tests. Those duplicate attributes also cause repeated harness executions, so raw totals overstate unique checks. These are inexpensive repairs but are current release-gate failures, not hypothetical style concerns. Fix them and rerun the declared CI commands on the exact release commit.

The no-default-features workspace run also failed `demod_frame_versions_identify_latest_controls_under_iq_backlog`: its two-second deadline expired without the final frame. The isolated rerun passed. This establishes a failing full-run gate and load-sensitive behavior, not proof that the production worker permanently drops settings. The orphaned Discord workers provide a plausible source of starvation; that causal inference needs a clean-worker full rerun after repair. The GUI harness reported 893 passed, one failed and five ignored; earlier harnesses passed. Remaining protocol/LRPT/UAT crates were verified separately because Cargo stops after the failed GUI harness.

### F21 — P2: Unix desktop data storage still depends on launch directory

**Evidence:** [main.rs:6](/home/lupc/Documents/ez-sdr/ez-gui/src/main.rs:6), [app_data.rs:111](/home/lupc/Documents/ez-sdr/ez-gui/src/app_data.rs:111), [config.rs:840](/home/lupc/Documents/ez-sdr/ez-gui/src/config.rs:840).

Per-user relocation is Windows-only. On Linux/macOS, settings, bookmarks/cache and default relative output still depend on process working directory. Launching from a different directory can appear to lose saved preferences; a read-only launch directory causes persistence failure. The recent save-error feedback helps but does not create a stable storage location. Native Finder behavior was not tested, so this is a source-level packaging risk rather than a claimed reproduced macOS startup failure. Use XDG and macOS Application Support locations, preserving explicit replay/output path bases during migration.

### F22 — P2/P3: release checksum artifacts do not consistently match documentation

**Evidence:** [package-windows-release.sh:263](/home/lupc/Documents/ez-sdr/scripts/package-windows-release.sh:263), [package-windows-release.sh:268](/home/lupc/Documents/ez-sdr/scripts/package-windows-release.sh:268), [package-linux-release.sh](/home/lupc/Documents/ez-sdr/scripts/package-linux-release.sh), [DISTRIBUTION.md](/home/lupc/Documents/ez-sdr/DISTRIBUTION.md).

The Windows installer is compiled before `SHA256SUMS` is created in its source directory, so it omits the manifest the documentation tells installed users to verify. The portable ZIP is created afterward and does include it. The Linux script creates an internal manifest and tarball, but no outer `.sha256`; its release job uploads only the tarball, despite the general outer-container checksum promise. Reorder Windows manifest creation and generate/upload Linux outer hashes. This was verified from script ordering, not by executing a Windows installer.

### F23 — P3/open gate: stale field-validation scope and target-platform claims

**Evidence:** [HARDWARE_VALIDATION.md:52](/home/lupc/Documents/ez-sdr/HARDWARE_VALIDATION.md:52), [README.md](/home/lupc/Documents/ez-sdr/README.md), [DISTRIBUTION.md](/home/lupc/Documents/ez-sdr/DISTRIBUTION.md).

The field checklist still describes one desktop radio chain, external-only UAT and imported-only Meteor, while current README documents receiver B, native UAT and live Meteor. Update the checklist so it validates the current promises: native UAT at its exact clock, both receivers and active recordings, live pass finalization, imported sound settings, reconnects and visible errors. The original off-air 80k Meteor gate remains open. The unsigned/unnotarized Intel-only macOS release is openly documented; it limits broad distribution but is not concealed deception. Keep stable-release platform claims aligned with clean-machine installation and startup evidence.

## Industry comparison

This is a workflow comparison based on primary project documentation read during the audit. It is not a head-to-head performance or RF sensitivity benchmark.

- **SDR++:** its documented baseline includes multiple VFOs, broad direct/Soapy hardware support, modular plugins, SIMD DSP and Windows/Linux/macOS/BSD support. EZ-SDR supplies a compact receiver, an independent B receiver, common demodulation, spectrum/waterfall, recording/replay and a daemon, but has narrower direct desktop hardware and extension support. Its settings/control coherence and interoperability defects matter more immediately than adding another plugin. [SDR++ features](https://www.sdrpp.org/), [user manual](https://www.sdrpp.org/manual.pdf).
- **SDR Console:** documents up to 24 receivers, per-mode filter definitions, docking layouts, IQ/audio recording and playback, an advanced scheduler and GPU FFT options. EZ-SDR's two-receiver and CF32/raw/WAV paths are useful, but recorder configuration/scheduling are poorly discoverable after utility retirement. Do not claim mature wideband monitoring or a recording-management workflow equivalent to Console. [Console features](https://www.sdr-radio.com/Console), [recording format/scheduler](https://www.sdr-radio.com/data-recording).
- **SatDump:** documents a broad satellite pipeline/product catalog including Meteor, NOAA-family, GOES, Fengyun and other missions. EZ-SDR has a focused Meteor LRPT chain with live/imported products and geolocation; it is not a general satellite decoder. The declared missing APT/HRPT/GOES support is honest. Ship the focused pipeline with original off-air 80k and target-receiver evidence before expanding mission breadth. [SatDump pipeline catalog](https://docs.satdump.org/md_docs_2pages_2Pipelines.html).
- **readsb/tar1090:** readsb documents Beast/SBS networking, history and aggregation-oriented controls; tar1090 adds adjustable history, multiple maps, multi-aircraft selection and optional heatmaps. EZ-SDR offers native aviation decoding, identity/quality metadata, a map/list and bounded trails, but does not establish equivalent long-term operational history or ecosystem integration. ADS-B clock correctness and field reception are prerequisites to that comparison. [readsb](https://github.com/wiedehopf/readsb/blob/dev/README.md), [tar1090](https://github.com/wiedehopf/tar1090).
- **Hamlib ecosystem:** frequency/mode endpoints are useful, but the observed `v/V` mismatch is an interoperability defect rather than a scope choice. Test against an actual standard client before calling the server compatible. [rigctld protocol](https://hamlib.sourceforge.net/snapshots/man1/rigctld.1.html).

## What deserves credit

- Both transports have substantial resilience tests, protocol versioning, frame bounds and channel identities. Optional authentication and browser-boundary checks are real implementations.
- Local/daemon audio distinguishes RF selection from output clocks, preserves capture provenance and has meaningful antialiasing, sideband, squelch, stereo and partition tests.
- Recording code tracks discontinuities/completeness; Meteor products have explicit boundaries/finalization rather than relying only on preview pixels.
- Native UAT is implemented as a dedicated decoder crate, not a label wrapped around an external process. ADS-B preserves address classes and optional measurements rather than inventing unavailable values.
- The project already documents physical-hardware limits and license/source-distribution obligations. These are good foundations to preserve while repairing the remaining workflow boundaries.

## Recommended release sequence

1. **Make controls transactional.** Fix F01–F04, exact ADS-B clock validation and the channel-creation/rate-change race. Test pending settings edits and concurrent client operations.
2. **Make failures visible.** Fix F05–F08 and expose daemon credentials, main audio selection reconciliation, service bind/delivery state and stable client connections across unrelated Settings changes.
3. **Repair interoperability/lifetimes.** Fix the browser session generation, focused input, rigctl mappings/client policy, ingest cleanup and notifier worker/status handling. Keep integration tests local and deterministic.
4. **Finish retirement deliberately.** Preserve useful backend APIs, remove truly orphan renderer state, update guides and document the supported control routes for recording/bookmarks/automation.
5. **Clear exact-commit gates.** Formatting, strict Clippy, tests with/without defaults, fresh dependency/license checks, packaging hashes and clean-machine install/startup on supported platforms.
6. **Collect field evidence.** Real antenna/audio/USB recovery runs, a concurrent-receiver/recording soak and original off-air 80k Meteor comparison. Record failures and signal-loss metadata as well as successes.

A narrowly advertised beta can be reasonable after the high-impact control/data fixes. A broad stable release should wait for the remaining quality, distribution and field gates. The quickest path to shipping is to make the existing advertised workflows reliable and reviewable; feature expansion can follow.
