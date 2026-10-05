# Audit remediation progress

Scope: every finding and the follow-up experiments in REPORT.md. The original forensic evidence remains unchanged. An item is complete only after its intended contract is tested; hardware experiments remain explicit until measured.

## Findings

- [x] A01 (P1): A hash collision authorizes an aircraft that was never learned
- [x] A02 (P2): Repeated messages erase the entire known-aircraft filter
- [x] A03 (P1): DF18 non-ICAO addresses are merged into ICAO aircraft
- [x] A04 (P1): Aircraft status messages corrupt vertical rate
- [ ] A05 (P2): Negative and unavailable aircraft measurements become zero
- [ ] A06 (P2): Airspeed, heading and ground-track semantics are conflated
- [x] A07 (P2): Position confidence is replaced by the quality of unrelated messages
- [ ] A08 (P2): CPR pairing uses processing time instead of the capture clock
- [ ] A09 (P1): Local GUI draining cannot keep up at ordinary 60 Hz pacing and hides gaps
- [ ] A10 (P1): The channelizer erases upstream sample discontinuities
- [x] A11 (P1): Daemon mode hides every aircraft that lacks a position
- [x] A12 (P1): The daemon adapter manufactures freshness, heading validity and perfect confidence
- [ ] A13 (P2): A position can remain current indefinitely while unrelated traffic continues
- [ ] A14 (P2): Geometric-position messages and newly acquired surface tracks cannot produce daemon positions
- [x] A15 (P2): DF11 interrogator parity is treated as a CRC error before IID scoring
- [ ] A16 (P2): 1090 message-rate and daemon totals are disconnected from message reception
- [ ] A17 (P2): Trail collection depends on whether the Planes tab is rendered
- [ ] R01 (P1): The app's CF32 writer and replay reader disagree about amplitude units
- [ ] R02 (P2): CF32 replay progress advances in output bytes instead of file bytes
- [x] R03 (P1): CF32 recording failures and overflow are reported as success
- [x] R04 (P1): Restarting CF32 recording in the same second overwrites the previous capture
- [ ] R05 (P1): A single IQ recording can contain multiple tuning or sample-rate epochs under one metadata header
- [x] R06 (P2): Daemon recording events cannot update shared recording state while the app holds its lock
- [x] R07 (P2): Recording duration enforcement lives in an unrendered panel
- [ ] G01 (P1): The scanner logs an off-channel signal as a hit at the selected frequency
- [ ] G02 (P2): The scanner continues making decisions from stale spectra when Listen is hidden or the source is stopped
- [ ] G03 (P2): The demodulation control queue discards the newest settings instead of coalescing them
- [ ] G04 (P2): Saved hardware and spectrum controls are not consistently restored or applied
- [ ] G05 (P2): Applying appearance or integration settings retunes the active receiver to the startup default
- [ ] G06 (P2): Bookmark autosave watches collection length and clean exit does not flush pending bookmarks
- [ ] G07 (P2): The current UI can send an AI request but never polls or renders its result
- [ ] G08 (P2): Several AI tools report completed operations after changing only flags or shadow settings
- [ ] G09 (P2): Daemon squelch changes through the spectrum and remote control never reach audio playback
- [ ] G10 (P2): Stopping a GUI workflow removes a globally shared daemon channel
- [ ] I01 (P2): A rate-limit condition publishes on every frame for a one-second window and may block the UI
- [ ] I02 (P2): Changing the broker while enabled does not change the active connection
- [ ] I03 (P2): Unpositioned tracks are exported with numeric zero coordinates
- [ ] S01 (P2): A later pass of the same satellite can fail to auto-tune
- [ ] S02 (P2): Changing observer location leaves cached passes and trajectories for the old location
- [ ] S03 (P2): Map panning is reset every second and satellite markers use stale projection parameters
- [x] L01 (P2): Ordinary TCP fragmentation can corrupt a valid rigctl command
- [ ] V01 (P3): Raw wideband IQ is presented as symbol-level QPSK quality

## Additional experiments

- [ ] Calibrated weak-signal/decode sensitivity comparison
- [ ] Vetted position plausibility vectors and surface ambiguity policy
- [ ] USB unplug/blocking reads and disk failure/shutdown latency
- [ ] Display/capture processing latency at 2.4 MS/s
- [ ] Saturated mixed daemon streams and loopback MQTT backpressure
- [ ] Observer/pass recomputation cost
- [ ] Atomic airport database replacement under insert failure
- [ ] Pinned Rust 1.92 validation and supported hardware/front-end checks

## Verification log

- Initial checkout preserved: pre-existing tracked deletions and untracked AGENTS.md are outside remediation commits.
- Batch 1: A01/A02 replace probabilistic/global-rotation membership with exact expiring address keys; A15 preserves valid DF11 IID parity under repair limits 0/1/2. Collision, duplicate-refresh, independent expiry and parity regressions pass.
- Batch 1: A03 separates DF18 CF/IMF namespaces end to end and excludes unsupported ES layouts. Local real-IQ and daemon snapshot regressions keep equal addresses distinct. Enrichment already rejects namespaced keys; MQTT now exports address class/source and nullable ICAO.
- Batch 1: A04 decodes all emergency states and status squawk independently of vertical rate, preserves them in daemon telemetry, and exposes emergency status in the aircraft list. CRC-valid local/daemon regressions pass.
- Batch 1: A07 retains position timestamp/source/quality across unrelated traffic. A11 keeps partial daemon tracks; A12 preserves observation age, missing track validity, altitude reference, vertical rate, squawk and available position quality. Unknown quality remains explicit.
- Rust 1.92.0 installed and exercised, closing the audit's compiler-compatibility gap. Workspace all-target tests pass (1,472 executions, zero failures, eight ignored); strict all-target Clippy passes. Hardware/front-end portion of the final additional experiment remains open.
- Changed Rust files formatted with pinned rustfmt. Full workspace format check exposes pre-existing formatting differences in untouched files; these are tracked as a final gate, not silently claimed to pass.
- AST-only graphify update succeeds; generated tracked metadata that was deleted before this task is removed again after refresh. The ignored graph.json is retained and current.
- Binary protocol bumped to 2 for changed telemetry; rebuild GUI and daemon together. Original defect-preserving forensic probes/evidence are preserved; desired behavior is verified by repository regression tests.

- Batch 2: R03/R04 open the CF32 file before acknowledging start, create a unique session file without truncation, expose writer liveness/errors, account for lost samples, propagate write/flush/sidecar failures, and mark incomplete metadata. Failure polling runs independently of satellite rendering. Regression tests cover open failure, queue overflow, failed/exited workers, write/flush/sidecar failure and five rapid sessions. CF32 amplitude compatibility and capture-epoch integrity remain R01/R05 work.
- Batch 2: R06 applies daemon recording state through the app's existing shared-state guard. R07 enforces deadlines in app logic and IQ/audio ingestion, and exposes Recorder through Tools. Hidden-tab deadline/current-route and locked daemon active/inactive regressions pass.
- Batch 2: L01 preserves partial TCP commands across timeouts, requires newline framing, bounds lines at 1,024 bytes and remains cancellable. Loopback regression splits frequency/mode/volume requests and queries at every byte boundary, checks oversized-line recovery, and stops mid-line.
- Next work: aircraft optional/signed and velocity semantics (A05/A06); sample-clock, source/queue/epoch and channelizer contracts (A08/A09/A10/R05); recording/replay amplitude and progress (R01/R02), followed by the remaining model, control, persistence and integration findings.
- Batch 2 final gates: Rust 1.92.0 workspace all-target tests pass (1,480 executions, zero failures, eight ignored), strict all-target Clippy passes, changed Rust files pass pinned rustfmt, and graphify is refreshed. Thirteen of 42 formal findings are checked off; the goal remains active.
