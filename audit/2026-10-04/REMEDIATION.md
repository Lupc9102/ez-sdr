# Audit remediation progress

Scope: every finding and the follow-up experiments in REPORT.md. The original forensic evidence remains unchanged. An item is complete only after its intended contract is tested; hardware experiments remain explicit until measured.

## Findings

- [x] A01 (P1): A hash collision authorizes an aircraft that was never learned
- [x] A02 (P2): Repeated messages erase the entire known-aircraft filter
- [x] A03 (P1): DF18 non-ICAO addresses are merged into ICAO aircraft
- [x] A04 (P1): Aircraft status messages corrupt vertical rate
- [x] A05 (P2): Negative and unavailable aircraft measurements become zero
- [x] A06 (P2): Airspeed, heading and ground-track semantics are conflated
- [x] A07 (P2): Position confidence is replaced by the quality of unrelated messages
- [ ] A08 (P2): CPR pairing uses processing time instead of the capture clock
- [ ] A09 (P1): Local GUI draining cannot keep up at ordinary 60 Hz pacing and hides gaps
- [x] A10 (P1): The channelizer erases upstream sample discontinuities
- [x] A11 (P1): Daemon mode hides every aircraft that lacks a position
- [x] A12 (P1): The daemon adapter manufactures freshness, heading validity and perfect confidence
- [x] A13 (P2): A position can remain current indefinitely while unrelated traffic continues
- [x] A14 (P2): Geometric-position messages and newly acquired surface tracks cannot produce daemon positions
- [x] A15 (P2): DF11 interrogator parity is treated as a CRC error before IID scoring
- [x] A16 (P2): 1090 message-rate and daemon totals are disconnected from message reception
- [x] A17 (P2): Trail collection depends on whether the Planes tab is rendered
- [x] R01 (P1): The app's CF32 writer and replay reader disagree about amplitude units
- [x] R02 (P2): CF32 replay progress advances in output bytes instead of file bytes
- [x] R03 (P1): CF32 recording failures and overflow are reported as success
- [x] R04 (P1): Restarting CF32 recording in the same second overwrites the previous capture
- [ ] R05 (P1): A single IQ recording can contain multiple tuning or sample-rate epochs under one metadata header
- [x] R06 (P2): Daemon recording events cannot update shared recording state while the app holds its lock
- [x] R07 (P2): Recording duration enforcement lives in an unrendered panel
- [x] G01 (P1): The scanner logs an off-channel signal as a hit at the selected frequency
- [ ] G02 (P2): The scanner continues making decisions from stale spectra when Listen is hidden or the source is stopped
- [x] G03 (P2): The demodulation control queue discards the newest settings instead of coalescing them
- [ ] G04 (P2): Saved hardware and spectrum controls are not consistently restored or applied
- [x] G05 (P2): Applying appearance or integration settings retunes the active receiver to the startup default
- [x] G06 (P2): Bookmark autosave watches collection length and clean exit does not flush pending bookmarks
- [x] G07 (P2): The current UI can send an AI request but never polls or renders its result
- [ ] G08 (P2): Several AI tools report completed operations after changing only flags or shadow settings
- [ ] G09 (P2): Daemon squelch changes through the spectrum and remote control never reach audio playback
- [ ] G10 (P2): Stopping a GUI workflow removes a globally shared daemon channel
- [ ] I01 (P2): A rate-limit condition publishes on every frame for a one-second window and may block the UI
- [ ] I02 (P2): Changing the broker while enabled does not change the active connection
- [x] I03 (P2): Unpositioned tracks are exported with numeric zero coordinates
- [ ] S01 (P2): A later pass of the same satellite can fail to auto-tune
- [ ] S02 (P2): Changing observer location leaves cached passes and trajectories for the old location
- [ ] S03 (P2): Map panning is reset every second and satellite markers use stale projection parameters
- [x] L01 (P2): Ordinary TCP fragmentation can corrupt a valid rigctl command
- [x] V01 (P3): Raw wideband IQ is presented as symbol-level QPSK quality

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

- Batch 3: A05 preserves signed optional altitude end to end, including Q/Gillham decoding, local IQ, daemon, wire codec, UAT, table/details/alerts and exports. Unavailable values render as unavailable; real negative and zero measurements remain distinct.
- Batch 3: A06 centralizes velocity/surface fields, separates GS/ground track from magnetic or true heading and IAS/TAS, fixes surface ME bit extraction and movement scale, and clears obsolete fields on air/ground transitions. All TC19 subtypes and unavailable fields have local real-IQ and daemon regressions; UAT carries true/magnetic reference, distinct track and altitude source. Surface direction remains explicitly track-or-heading until operation-status disambiguation is supported.
- Batch 3: A13 expires daemon positions independently of other traffic and publishes/prunes on idle timeouts. Stale positions cannot bootstrap surface CPR. A14 handles TC20-22 and uses explicitly configured actual capture-site coordinates for initial surface CPR, with the shared 45 NM plausibility policy. Daemon CLI accepts paired --receiver-lat/--receiver-lon; no coordinates are assumed.
- Batch 3: A17 moves bounded trail history/expiry into app logic, verified while Planes is hidden. I03 exports nullable partial/stale positions, validity/source/age and typed optional measurements; Discord eligibility has independent deduplication and retries until a current position arrives. No notifications were sent.
- Batch 3 final gates: Rust 1.92.0 workspace all-target tests pass (1,492 executions, zero failures, eight ignored), strict all-target Clippy passes, changed files pass pinned rustfmt, and graphify is refreshed. Binary protocol is now 3; rebuild GUI and daemon together. Nineteen of 42 formal findings are fixed and committed after this batch; the goal remains active.

- Batch 4: R02 carries absolute input-file offsets with replay blocks, so progress uses raw/CF32 on-disk bytes rather than converted CU8 output bytes. Each loop resets the offset. Regressions verify every block, a short final block, 100% before EOF is consumed and three loops. R01 amplitude and R05 epoch integrity remain explicit.
- Batch 4: G06 detects bookmark content changes (including equal-length edits and paths missing a dirty flag), retains the 15-second debounce, and flushes pending data through the clean-exit hook. Checked atomic saves clear dirty state only after success; failures retain dirty state and retry. Temporary-file regressions add, rename, edit notes, star, delete-and-add, delete, immediately flush/reload, and inject a failed destination without changing cwd or user files.
- Batch 4 final gates: Rust 1.92.0 workspace all-target tests pass (1,495 executions, zero failures, eight ignored); strict all-target Clippy and changed-file format checks pass; graphify is refreshed. Twenty-one of 42 formal findings are fixed; the goal remains active. Eight untouched files still differ under the full workspace format check.

- Batch 5: R01 makes all daemon input sources and new CF32 recordings use normalized full-scale components (CU8 0/255 maps to -1/+1), declares units in both writer sidecars, and uses exact rounded CU8 replay conversion. GUI replay, daemon replay and offline LRPT imports honor bounded metadata; known legacy satellite metadata and explicit GUI/CLI/API/sidecar choices import old byte-minus-127.4 captures without signal-peak guessing. Replay preference persists. Real writer/reader regressions preserve every ADC byte, weak/full-scale tones, ADS-B messages and coded LRPT frames.
- Batch 5 replaces a timing-based FFT test wait with test-only explicit worker completion accounting after a parallel-load race exposed its stale-frame assertion; production FFT behavior is unchanged. Changed config, decoding and spectrum files also now pass pinned formatting.
- Batch 5 final gates: Rust 1.92.0 workspace all-target tests pass (1,500 executions, zero failures, eight ignored); strict all-target Clippy and changed-file format checks pass; graphify is refreshed. Twenty-two of 42 formal findings are fixed; five untouched files still differ under the full workspace format check. Capture epochs, remaining controls/integrations and hardware experiments remain active.

- Batch 6: A16 publishes typed receiver counter/rate snapshots from actual local Mode S framing, the UAT ingest worker and the daemon packet pipeline. A shared fixed five-second window uses at most 50 buckets; idle reads expire rates without arrivals or tab rendering, and receiver-counter resets retire old rate state. Aircraft snapshot republication never counts as reception. UAT accepted/drop/rejection counters remain separate. Daemon idle snapshots and disconnected GUI snapshots reach zero; daemon lifetime totals survive aircraft pruning.
- Batch 6 regressions exercise real Mode S IQ locally and in the daemon (100 messages, total 100, rate 20/s), UAT parsing/queue overflow, idle expiry, receiver resets, bounded known-cadence accounting and wire preservation of counters and channel identity. Protocol is now 4; rebuild GUI and daemon together.
- Batch 6 final gates: Rust 1.92.0 workspace all-target tests pass (1,505 executions, zero failures, eight ignored); strict all-target Clippy and changed-file formatting pass; graphify is refreshed. Twenty-three of 42 formal findings are fixed. The goal remains active.

- Batch 7: G03 replaces the four-entry settings FIFO with synchronized latest-value batch/profile/reset state and one coalesced wakeup. Controls apply and acknowledge their revision even without IQ. Frames carry their exact batch settings, control revision and advanced-profile revision; the app discards superseded audio and records a submitted profile only when delivery succeeds. Unchanged settings preserve the acknowledgement.
- Batch 7 regressions flood 100 settings/profiles before IQ, verify independent idle acknowledgement and final AM/muted output, change controls under IQ backlog, check frame revisions/settings and verify missing-worker requests report failure.
- Batch 7 final gates: Rust 1.92.0 workspace all-target tests pass (1,508 executions, zero failures, eight ignored); strict all-target Clippy and changed-file formatting pass; graphify is refreshed. Twenty-four of 42 formal findings are fixed. Four untouched files still differ under the full workspace format check; the goal remains active.

- Batch 8: V01 labels the satellite view as raw ADC IQ geometry before carrier/timing recovery and removes unsupported QPSK EVM/phase-error claims. Scatter points and density now cover the same bounded selected-sample window; eviction and capacity shrink subtract old density, Reset clears both, and capture center/rate/source/generation changes clear both even while frozen. Large pushes select at most 512 points.
- Batch 8 regressions render known QPSK points, rotating-carrier and noise controls with explicit raw-IQ qualification and no symbol-quality metrics; verify density equality/expiry, capacity shrink/reset, and capture-change invalidation while frozen.
- Batch 8 final gates: Rust 1.92.0 workspace all-target tests pass (1,508 executions, zero failures, eight ignored); strict all-target Clippy and changed-file formatting pass; graphify is refreshed. Twenty-five of 42 formal findings are fixed and committed. The remaining seventeen findings and additional experiments stay open; the goal remains active.

- Batch 9: A10 retains original acquisition sample indices, exact rational rates, capture epochs and known/unknown UTC origins through channelization. Missing samples, frequency/rate changes and source rewinds reset NCO/FIR history and publish an explicit new epoch; gaps smaller than the decimation factor remain visible even when output indices happen to be adjacent. NCO phase resumes from the original absolute source clock. Live sources without acquisition timestamps and replay files without an origin keep UTC unknown rather than substituting processing time.
- Batch 9: Daemon replay never fills one block across a loop boundary. Ingestion changes epochs after successful changed hardware settings. Each audio, spectrum, packet and LRPT consumer checks its own queue's sequence/clock: gaps reset discriminator/DC/SSB state, FFT partial windows/averages, packet overlap/CPR and partial LRPT coding/frame/packet state. LRPT completed images and lifetime counts survive a gap and decoding reacquires.
- Batch 9 regressions cover real SampleBus overflow at decimation 1/4, injected gaps at 1/4/7, fractional sample clocks, adjacent-index epoch/retune changes, tiny blocks that defer the gap marker, absolute NCO phase, actual ADS-B and LRPT messages split across missing intervals, downstream audio/FFT history, and daemon replay rewind segmentation. A08 capture-clock CPR pairing, A09 continuous GUI ingestion and R05 recording epochs remain open.
- Batch 9 final gates: Rust 1.92.0 workspace all-target tests pass (1,520 executions, zero failures, eight ignored); strict all-target Clippy and changed-file formatting pass; graphify is refreshed. Twenty-six of 42 formal findings are fixed. The remaining sixteen findings and additional experiments stay open; the goal remains active.

- Batch 10: G01 routes scanner decisions through the selected RF passband measurement and refuses a mismatched selected VFO. A -10 dB carrier 200 kHz away cannot log a hit or hold the empty selected 12.5 kHz channel; a weaker in-band carrier logs its correct channel and strength. G02 freshness, hidden/frozen display processing and tuning-epoch settling remain explicit work.
- Batch 10: G05 treats frequency/rate/gain defaults as startup preferences, labels them accordingly, and preserves the active capture when Save & Apply changes appearance, integrations or those defaults. Real app-logic regressions preserve 1090 MHz, 2.4 MS/s, gain, running state and capture generation while theme/font/topic/default preferences change. Dedicated live receiver controls still apply live changes; G04 persisted hardware/FFT restoration remains open.
- Batch 10: G07 adds an accessible Tools → AI Agent response window, opens it for Scanner requests and current spectrum/aircraft prompts, and polls request/tool events from app logic regardless of window visibility. A loopback fake receives the real Scanner request, withholds its response until the window is closed, and verifies completion and visible text after reopening for both streamed success and HTTP 503 failure. Proposed mutations remain pending approval and never tune during hidden polling. No external provider calls were made. G08 lifecycle/tool semantics remains open.
- Batch 10 final gates: Rust 1.92.0 workspace all-target tests pass (1,524 executions, zero failures, eight ignored); strict all-target Clippy and changed-file formatting pass; graphify is refreshed. Twenty-nine of 42 formal findings are fixed. Thirteen findings and additional experiments remain open; the goal stays active.
