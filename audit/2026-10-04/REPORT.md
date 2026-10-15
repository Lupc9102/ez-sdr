# ADS-B and Rust GUI logic audit

Audited source: `823514d6bfd73155a622d58adce622280d668320` on 2026-10-04. This is a behavioral audit of acquisition, decoding, timing, identity, state transitions, rendering inputs, control delivery, recording, persistence and integrations. Code formatting, idiomatic Rust and visual polish were not used as acceptance criteria. No production fixes are included.

The receiver and GUI are **not reliable enough to treat all displayed aircraft measurements, recording-success indications or applied settings as ground truth**. This is supported by specific failures below, not by an aesthetic judgement. Ordinary decoding and many isolated DSP routines do work; the most consequential defects occur where two otherwise plausible components disagree about clocks, units, validity, ownership or lifecycle.

The audit records **42 findings: 12 P1, 29 P2 and one P3**. P1 means an urgent correctness or data-loss problem in a supported path; P2 means a material functional defect or a narrower/dormant contract defect; P3 means a misleading secondary measurement. There is no P0 claim. Nineteen executable probes reproduce failures underlying 21 findings; overlapping findings sometimes share one probe. The remaining findings are source-confirmed control/data-flow defects, with prospective regression tests rather than invented runtime results. R07 and G08 explicitly distinguish dormant UI surfaces from active workflows.

## Verification and provenance

- `cargo test --workspace --all-targets` completed successfully: **1,457 test executions passed, zero failed, eight ignored**. This includes the same 249 dump1090 unit tests executed through both library and binary targets; counting that duplicated set once gives 1,208 passed executions. These counts do not imply 1,208 independent end-to-end scenarios. The GUI unit target contributed 698 passed and four ignored tests.
- Compilation/tests used **rustc 1.96.1**, the installed stable toolchain. The repository pins 1.92, which is not installed on this host. Compatibility with the pinned compiler was not verified. No dependency upgrades or toolchain installations were made for Rust.
- The behavioral harness calls actual public APIs from the built workspace libraries. To reach private seams without editing production Rust, it also compiles the unchanged local ADS-B decoder and daemon packet module into a test-only harness, and extracts the original daemon mapper and AI executor functions verbatim. Packet module inner documentation comments are removed solely to make `include!` valid; its executable source is unchanged. Small private squelch helpers are likewise extracted so the AI function compiles; the probed tools do not exercise those helpers. Each tested source file is hashed in `evidence.json`.
- The probes assert **the observed defects**, so a successful probe run means the audited behavior was reproduced. These are forensic demonstrations, not a passing regression suite for desired behavior. After fixes, convert the listed regression requirements into assertions of correct behavior and retire/reverse these assertions.
- The probes create only temporary recordings and a temporary loopback rigctl socket. They do not start USB capture, send Discord messages, call an AI provider or contact an external MQTT broker. The expected injected CF32 open failure is printed to stderr and is part of the evidence.
- No live SDR, USB-unplug scenario, actual aircraft emergency, real broker backpressure, external UAT source, GPU rendering session or real audio output device was exercised. RF decode sensitivity, physical antenna performance, screen-reader accessibility, monitor pacing and sound quality were not certified. Headless egui was used for the recorder deadline probe, and the repository's GUI tests ran.
- All 62 Rust files under `ez-gui/src` were inventoried, including tests/helpers and retained legacy modules. Live routing and major model/worker boundaries were followed into dump1090, ez-daemon, ez-proto and the IQ conversion/offline decoder dependencies. This is broad and deep source/contract coverage, **not proof that every Rust branch was executed or every possible defect found**.
- The checkout already had **672 tracked deletions** and an untracked `AGENTS.md` before this work. They were kept outside the audit commit. No production `.rs` changes were introduced.
- `graphify-out/graph.json` was absent initially, so the instruction to query an existing graph did not apply. After creating the reproducible audit script, `graphify update .` completed with AST extraction only: 5,969 nodes and 12,439 edges, no LLM call. Its generated ignored `graph.json` remains current; tracked graph metadata that the user had already deleted was left deleted rather than bundled into this commit.

## The contracts that fail

The local path is RTL/replay bytes → GUI-bounded queue → GUI-frame drain → ADS-B magnitude/demodulation → field/CPR merge → `AircraftEntry` → list/map/alerts/exports. The daemon path is IQ source → `SampleBus` → channelizer → packet pipeline → tracker/semantic merge → telemetry snapshot → daemon client queue → GUI mapper → the same list/map. UAT has a separate bounded JSON reader and merge path. The audit followed all three because sharing a final aircraft shape does not make their semantics equivalent.

The most important cross-component contracts are:

1. **A sample gap remains a gap.** Local byte-only messages cannot express it; daemon channelization renumbers separated data as contiguous. Dropping blocks for responsiveness must not silently fabricate a continuous RF clock.
2. **One address key identifies one transmitter class.** Probabilistic ICAO membership is used as an acceptance gate, and DF18 non-ICAO identifiers are merged with ICAO identifiers.
3. **A field carries its own validity, age, source and units.** Aircraft shapes collapse missing/signed data to zero, position confidence follows unrelated messages, and daemon snapshot arrival resets observation age.
4. **Success means the underlying operation happened.** The CF32 worker can fail while the GUI retains an active handle; AI recording flags do not start a recorder; saved settings are only partly applied.
5. **A recording has a reconstructable clock and amplitude convention.** CF32 write/read units disagree, and untagged IQ recording can cross tuning/rate epochs under one metadata header.
6. **Model work does not depend on painting a panel.** Scanner measurements, aircraft trails, recording deadlines and AI stream consumption violate this in different ways.
7. **One client's subscription is not global channel ownership.** Fixed daemon IDs and `RemoveChannel` make an ordinary local stop affect another attached client.

## Coverage by subsystem

- **1090 decoder and tracking:** preamble/slicing/scoring, CRC and repair policy, exact frame length, ICAO acceptance, DF11/17/18/address-parity formats, altitude, velocity, status/squawk, airborne/surface CPR, receiver references, overlap, byte partitioning, sample-rate gating, field retention, counters and quality. Findings A01-A16. The fixed weak-signal threshold and position plausibility are listed as follow-up experiments below rather than asserted failures without RF evidence.
- **Aircraft presentation and UAT:** local/daemon/UAT adapters, map/list filtering, age/position validity, trails, enrichment, alert eligibility, region switching, socket lifecycle, bounded feed parsing, namespace handling and tile loading/cache limits. Findings A03, A05-A07, A11-A17 and I03. UAT parsing preserves address namespaces and validates bounded finite coordinates, but its merged heading and altitude type still lose semantics.
- **Capture/source lifecycle:** selected source restoration, replay pacing/looping/end-of-stream, format conversion, live overflow, local worker stop/restart, capture generation and daemon control shadow state. Findings A08-A10, R01-R02, R05 and G04. USB blocking reads still need a real failure-path experiment.
- **Radio, audio and DSP:** VFO mixing and capture-center reconciliation, RF/audio clocks, filtered decimation, demod worker control/input/output queues, mode changes, power/CTCSS squelch, stereo/RDS side taps, audio device/rate selection and playback buffering. Findings A09-A10, G03-G05 and G09. Existing signal-based tests cover many isolated DSP invariants; hardware playback and composite latency were not measured.
- **Scanner, keyboard and history:** range/memory scans, dwell/hold/exclusion behavior, frequency identity of measurements, stale observations, control routing, focus-aware shortcuts and persistence. Findings G01-G02, G06 and S01. Pure history bounds/undo-redo and keyboard text-input focus are supported by source and existing tests; they are not a blanket pass for every workflow.
- **Recorders:** local IQ worker, WAV write/finalize path, daemon recording status/commands, satellite CF32 writer, deadlines, filenames, metadata, overflow and capture changes. Findings R01-R07. Ordinary recorder tests do not establish failure reporting, round-trip fidelity or capture-epoch integrity.
- **Satellite/TLE/offline Meteor:** pass scheduling, observer state/cache identity, selected satellite behavior, Doppler information, map projection/camera interaction, CF32 capture, worker cancellation, file/import/export completion and telemetry ingestion. Findings R01/R03-R05, S01-S03 and V01. Current Meteor behavior is deliberately offline; the disabled live daemon Meteor subscription was **not** classified as a defect.
- **Config/navigation/legacy surfaces:** startup restoration, import normalization, Save & Apply, profile delivery, font/theme application, tabs/tool routing and persisted settings. Findings G04-G08 and R07. The curated Tools menu dispatches Scanner, How-To, Setup Guide and Settings. Recorder, BookmarkManager, Advanced, Customize and several retained integration panels are not routed through that menu/secondary renderer; their presence and tests must not be mistaken for accessible current features. Integrations themselves can still be enabled through Settings/config.
- **AI:** request dispatch/stream consumption, optional tool approval, executor-to-model actions, status truth and prompt routing. Findings G07-G08. No real inference call was made; approval defaults do not cure an executor that claims a nonexistent operation.
- **Integrations:** daemon subscriptions/queues/data mapping, MQTT publish cadence/endpoint changes, Discord aircraft payloads and deduplication, loopback web-remote command routing/origin checks, and Hamlib framing/cancellation. Findings G09-G10, I01-I03 and L01. Notifications were inspected, not sent. `web_remote_async` is retained scaffolding rather than the active remote implementation.
- **Supporting modules:** airport/frequency databases, antenna guidance, fonts, status/error plumbing, theme/effects, reusable UI helpers and user-level gating were inspected at their live call boundaries or data/helper level. Formatting and decorative rendering were outside the requested logic criteria. Some physical antenna guidance needs field validation; a source audit cannot validate propagation or alignment effectiveness.

## Things that are already protected

These are concrete guards found in source and/or exercised by existing tests; they do not offset the findings:

- The local ADS-B decoder rejects sample rates other than 2.4 MHz, keeps incomplete byte pairs across chunks and bounds its aircraft map.
- PacketPipeline explicitly detects non-contiguous packet-input blocks and resets overlap. A10 explains why gaps earlier in its pipeline can disappear before reaching that guard.
- CPR stores even/odd state with bounded cache/type handling. The replay-time and observation-age contracts are still wrong outside that helper.
- UAT ingestion caps JSON line length, validates numeric coordinates and address classes, bounds queues/tracks and uses cancellable connect/read/retry behavior. It does not invent a positioned marker for a missing UAT position.
- Map tiles have bounded response/image allocation checks, expected 256×256 dimensions, validation before caching, limited concurrent downloads and retry backoff.
- The daemon client separates control events from bounded stream/spectrum queues, has a bounded handshake/shutdown path and existing loopback lifecycle tests. Bounded stream loss still has no observation-gap contract in audio playback.
- The main radio sidebar disables unsupported daemon DSB/CW/RAW modes, channel bandwidth and local advanced DSP controls. G09 isolates the still-active spectrum and web squelch entry points rather than treating those guarded controls as defects.
- The playback resampler maintains phase across frame partitions, cleans non-finite samples and tests rate conversion. The local radio path includes filtered decimation, VFO-specific power squelch, CTCSS and RDS tests against signals/noise rather than only shape assertions.
- Offline Meteor work uses background operations and cancellation checks; current tests cover malformed files and asynchronous import/export completion. No claim is made that its decoder is validated on every real recording.

## Protocol references and an excluded suspicion

Protocol comparisons used primary implementation/source references, not general search snippets. FlightAware's [Mode S implementation](https://raw.githubusercontent.com/flightaware/dump1090/master/mode_s.c) distinguishes DF18 CF/IMF address classes, TC19 airspeed versus ground-speed fields, and TC28 subtype versus emergency fields. Its [ICAO filter implementation](https://raw.githubusercontent.com/flightaware/dump1090/master/icao_filter.c) stores actual addresses in rolling tables; the Rust one-bit membership implementation changes that acceptance property.

An early suspicion about the **numeric encoding** of TC20-22 GNSS altitude was not promoted to a finding. [The 1090 MHz Riddle](https://mode-s.org/1090mhz/content/ads-b/3-airborne-position.html) describes a meters interpretation, while the current [FlightAware decoder](https://raw.githubusercontent.com/flightaware/dump1090/master/mode_s.c) uses its AC12 decoding path and marks the result geometric. That disagreement needs a versioned authoritative specification or a vetted capture/vector before accusing this app's numeric formula. The confirmed issues are separate: daemon CPR omits those type codes, telemetry loses altitude reference type, and signed/missing values are mishandled. The local adapter already distinguishes geometric versus barometric type.

## Findings

Each finding below includes the concrete trigger, observed/source behavior, consequence, proposed correction and a regression requirement. Source links refer to the audited checkout and one-based line numbers. Findings are listed by subsystem so related contracts remain together; P1 tags identify the first repair candidates.


### A01 · P1 · A hash collision authorizes an aircraft that was never learned

**Evidence:** Reproduced. **Area:** ADS-B acceptance.

**Trigger:** Learn ICAO ABCDEF, then present a DF4 address/parity message whose syndrome is 000151.

**Logic and evidence:** IcaoFilter stores one bit per hash bucket, rather than the actual address. Both addresses hash to the same bit. The demodulator treats contains(syndrome) as proof of a recently observed aircraft and accepts the unlearned address. The probe accepted 000151 after learning only ABCDEF. With k distinct addresses, false membership is approximately 1 - exp(-k/4096), assuming uniform hashing; at 100 addresses that is about 2.4%. This is a filter estimate, not a measured RF false-aircraft rate.

**Consequence:** The address/parity acceptance gate is weakened. Noise or corrupted surveillance frames can acquire an unrelated identity and update aircraft fields. Attractive map rendering does not repair that trust boundary.

**Correction:** Use an exact address table with bounded rolling expiry. Keep source/address namespaces in the key; do not use probabilistic membership as the final acceptance decision.

**Regression requirement:** Seed one address, exercise colliding addresses, and require rejection of unlearned AP syndromes while accepting a genuinely learned address.

**Source:** [dump1090/src/icao_filter.rs:86](/home/lupc/Documents/ez-sdr/dump1090/src/icao_filter.rs:86), [dump1090/src/demod.rs:366](/home/lupc/Documents/ez-sdr/dump1090/src/demod.rs:366).

**Recorded probes:** `icao_collision_ap`. See `evidence.json`.

### A02 · P2 · Repeated messages erase the entire known-aircraft filter

**Evidence:** Reproduced. **Area:** ADS-B acceptance.

**Trigger:** Call add for the same valid aircraft 8,192 times, without waiting for the 60-second expiry.

**Logic and evidence:** insertions counts messages, including duplicates. At 8,192 insertions, add clears every bit, including the aircraft just added. The probe confirms that the only learned address disappears. At 1,000 insertions per second this rotates the filter in roughly eight seconds; that rate is an illustrative load, not a hardware measurement.

**Consequence:** Surveillance frames from previously learned aircraft lose eligibility until another qualifying direct-address message arrives. Recently active aircraft are expired together regardless of recency.

**Correction:** Track actual address recency and rotate time windows without clearing the current observation. Bound unique-address storage rather than counting duplicate messages.

**Regression requirement:** Repeatedly refresh one address beyond 8,192 messages and verify it remains known; independently expire an inactive address.

**Source:** [dump1090/src/icao_filter.rs:73](/home/lupc/Documents/ez-sdr/dump1090/src/icao_filter.rs:73).

**Recorded probes:** `icao_duplicate_rotation`. See `evidence.json`.

### A03 · P1 · DF18 non-ICAO addresses are merged into ICAO aircraft

**Evidence:** Reproduced. **Area:** ADS-B identity.

**Trigger:** A DF17 aircraft and a DF18 CF=1 transmitter use the same 24-bit value.

**Logic and evidence:** The demodulator and stateless field decoder extract bytes 1-3 as an ICAO identity for every DF18 control field. Local tracking keys only that u32. The probe submits CRC-valid DF17 and anonymous/non-ICAO CF=1 messages and obtains one aircraft entry. CF and IMF can change address meaning, and some DF18 payloads are not ordinary ADS-B extended squitter. UAT already namespaces analogous address classes, so the inconsistency is within this application too.

**Consequence:** Different transmitters can overwrite the same callsign, position, altitude and metadata. Non-aircraft or anonymous addresses may be sent to ICAO enrichment and aircraft notifications.

**Correction:** Decode DF18 CF/IMF before selecting payload interpretation or track identity. Carry address type and source through local tracking, daemon telemetry, enrichment and exports.

**Regression requirement:** Pair equal numeric addresses in DF17, DF18 CF=1 and other supported non-ICAO classes; require distinct tracks and no ICAO lookup for anonymous targets.

**Source:** [dump1090/src/demod.rs:535](/home/lupc/Documents/ez-sdr/dump1090/src/demod.rs:535), [dump1090/src/mode_s.rs:30](/home/lupc/Documents/ez-sdr/dump1090/src/mode_s.rs:30), [ez-gui/src/adsb_decoder.rs:225](/home/lupc/Documents/ez-sdr/ez-gui/src/adsb_decoder.rs:225).

**Recorded probes:** `df18_identity_merge`. See `evidence.json`.

### A04 · P1 · Aircraft status messages corrupt vertical rate

**Evidence:** Reproduced. **Area:** ADS-B fields.

**Trigger:** Receive a TC28 subtype-1 status message after a valid velocity message, even when the emergency bits indicate no emergency.

**Logic and evidence:** The code reads msg[4] & 7, which is the subtype, as emergency status and writes that value into AircraftEntry.vertical_rate. A CRC-valid no-emergency message changed the probe's 5,568 ft/min vertical rate to 1 ft/min. Emergency and squawk from this status message are not represented correctly.

**Consequence:** A valid flight measurement becomes invented data. A real emergency is not exposed as an emergency, and routine status traffic overwrites climb/descent information.

**Correction:** Decode subtype and emergency/squawk from their own bit fields. Add explicit emergency state and preserve the velocity-derived vertical rate.

**Regression requirement:** Use CRC-valid TC28 subtype-1 vectors for no emergency and each emergency state. Require an unchanged prior vertical rate and correct emergency/squawk fields.

**Source:** [ez-gui/src/adsb_decoder.rs:601](/home/lupc/Documents/ez-sdr/ez-gui/src/adsb_decoder.rs:601).

**Recorded probes:** `status_corrupts_vertical_rate`. See `evidence.json`.

### A05 · P2 · Negative and unavailable aircraft measurements become zero

**Evidence:** Reproduced. **Area:** ADS-B fields.

**Trigger:** An aircraft reports -500 ft, or a partial track has not yet reported altitude, speed or track.

**Logic and evidence:** The common decoder clamps signed altitude into u32; the probe's Q-bit altitude for -500 ft becomes Some(0). AircraftEntry also uses zero-valued altitude/speed/heading without validity flags. The daemon mapper unwraps missing values to zero and clamps negative altitude again. Thus a real zero, a negative value and unavailable data cannot be distinguished at the presentation boundary.

**Consequence:** The list, altitude coloring, details and exports can assert sea-level altitude or zero speed that was never measured. Below-sea-level aircraft are misrepresented.

**Correction:** Preserve signed altitude and Option/validity for each measurement through the model. Render unavailable values as unavailable and retain altitude reference type.

**Regression requirement:** Round-trip -1,000, -500, 0 and positive altitudes. Display a callsign-only track without inventing altitude, speed or heading.

**Source:** [dump1090/src/mode_s.rs:107](/home/lupc/Documents/ez-sdr/dump1090/src/mode_s.rs:107), [ez-gui/src/adsb_panel.rs:107](/home/lupc/Documents/ez-sdr/ez-gui/src/adsb_panel.rs:107), [ez-gui/src/app.rs:608](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:608).

**Recorded probes:** `negative_altitude`, `daemon_adapter`. See `evidence.json`.

### A06 · P2 · Airspeed, heading and ground-track semantics are conflated

**Evidence:** Reproduced. **Area:** ADS-B fields.

**Trigger:** Receive TC19 subtype 3 or 4 with valid heading and indicated/true airspeed, or switch between airborne and surface reports.

**Logic and evidence:** Subtype 3/4 writes heading but leaves has_heading false; the probe obtains heading=90 and has_heading=false. It writes airspeed into the same speed field as ground speed without keeping IAS/TAS type. Surface movement and heading are stored separately, leaving generic airborne speed/heading potentially stale. UAT heading ingestion similarly does not set has_heading.

**Consequence:** A valid heading can be suppressed, and the same displayed speed can change meaning without any indication. On-ground aircraft may retain their previous airborne measurements.

**Correction:** Represent ground speed, IAS/TAS, ground track and heading as distinct typed optional fields. Update validity and air/ground transition behavior consistently across local, UAT and daemon ingestion.

**Regression requirement:** Exercise every TC19 subtype and airborne-to-surface transitions; verify value, units, type and validity together, including UAT heading.

**Source:** [ez-gui/src/adsb_decoder.rs:503](/home/lupc/Documents/ez-sdr/ez-gui/src/adsb_decoder.rs:503), [ez-gui/src/adsb_decoder.rs:539](/home/lupc/Documents/ez-sdr/ez-gui/src/adsb_decoder.rs:539), [ez-gui/src/adsb_panel.rs:736](/home/lupc/Documents/ez-sdr/ez-gui/src/adsb_panel.rs:736).

**Recorded probes:** `airspeed_heading_validity`. See `evidence.json`.

### A07 · P2 · Position confidence is replaced by the quality of unrelated messages

**Evidence:** Static confirmed. **Area:** ADS-B validity.

**Trigger:** A corrected or uncertain position is followed by a clean callsign or surveillance message, or a good position is followed by an unreliable non-position message.

**Logic and evidence:** process_decoded overwrites reliable, correctedbits and score before determining what fields the message updates. AircraftEntry documents reliability as quality of the last position, but the implementation records quality of the last message of any type. No per-field quality accompanies the retained position.

**Consequence:** The marker's confidence can improve or worsen without a new position. Enrichment and UI consumers cannot tell whether coordinates actually came from reliable data.

**Correction:** Attach source, timestamp and decode quality to each retained measurement, especially position. Keep message-level receiver statistics separate.

**Regression requirement:** Create a position with known quality, then send identification/velocity/surveillance messages of different quality and require unchanged position quality.

**Source:** [ez-gui/src/adsb_decoder.rs:268](/home/lupc/Documents/ez-sdr/ez-gui/src/adsb_decoder.rs:268), [ez-gui/src/adsb_panel.rs:107](/home/lupc/Documents/ez-sdr/ez-gui/src/adsb_panel.rs:107).

### A08 · P2 · CPR pairing uses processing time instead of the capture clock

**Evidence:** Static confirmed. **Area:** ADS-B clocks.

**Trigger:** Replay a file at a speed other than 1x, pause under backpressure, or loop the recording.

**Logic and evidence:** The demodulator produces sample timestamps, but local process_decoded receives no timestamp and CPR submissions use Instant::now. The daemon uses the same processing-time approach. GUI replay rewinds without a capture generation or discontinuity event. Frames close in the recording can expire at slow playback; frames far apart can become eligible at fast playback, and frames across loop boundaries can share state.

**Consequence:** Position decoding and age are functions of CPU scheduling and replay speed rather than the recording. Repeated playback is not a faithful analysis of captured aircraft timing.

**Correction:** Carry sample index, epoch and acquisition timestamp into each decoded message. Pair CPR on the capture clock and reset epoch-dependent state at rewind and genuine gaps.

**Regression requirement:** Decode the same timestamped capture at 0.1x, 1x and 10x; require equivalent positions. Put incompatible frames across a loop boundary and require no cross-loop pairing.

**Source:** [ez-gui/src/adsb_decoder.rs:197](/home/lupc/Documents/ez-sdr/ez-gui/src/adsb_decoder.rs:197), [ez-gui/src/adsb_decoder.rs:206](/home/lupc/Documents/ez-sdr/ez-gui/src/adsb_decoder.rs:206), [ez-gui/src/source_manager.rs:767](/home/lupc/Documents/ez-sdr/ez-gui/src/source_manager.rs:767), [ez-daemon/src/pipelines/packet.rs:203](/home/lupc/Documents/ez-sdr/ez-daemon/src/pipelines/packet.rs:203).

### A09 · P1 · Local GUI draining cannot keep up at ordinary 60 Hz pacing and hides gaps

**Evidence:** Static confirmed + capacity calculation. **Area:** ADS-B acquisition.

**Trigger:** Run RTL-SDR at 2.4 MS/s while the GUI processes two 16,384-complex-sample blocks per frame at 60 frames/s, or at its nominal 16 ms repaint interval.

**Logic and evidence:** The app drains at most two blocks and requests a 16 ms repaint. At 60 Hz, capacity is 1,966,080 samples/s, 18.08% below 2.4 MS/s. At exactly 16 ms/frame, capacity is 2,048,000 samples/s, 14.67% below input. Input can exceed capacity even at 2.048 MS/s with 60 Hz frames. Live source overflow discards entire blocks. SourceMessage::Samples contains only bytes, so the receiver has no sample index or gap marker; mark_discontinuous has no production caller. The decoder stitches old overlap onto later data and constructs an apparently continuous clock. These are conditional bounds, not a measured real-monitor frame rate; input events or a faster repaint loop could raise capacity.

**Consequence:** Reception and recordings lose blocks by design under those pacing conditions, while decoder overlap and filters treat separated samples as adjacent. UI load directly alters RF capture correctness.

**Correction:** Move continuous ingestion/decoding off the UI frame budget; coalesce visual snapshots instead. Include sample index and gap/epoch metadata in every block and reset affected state on overflow.

**Regression requirement:** Feed a paced 2.4 MS/s capture at a controlled 60 Hz UI cadence. Measure zero unaccounted loss and prove gap propagation with deliberate overflow and cross-block messages.

**Source:** [ez-gui/src/app.rs:765](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:765), [ez-gui/src/app.rs:1192](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:1192), [ez-gui/src/source_manager.rs:250](/home/lupc/Documents/ez-sdr/ez-gui/src/source_manager.rs:250), [ez-gui/src/adsb_decoder.rs:187](/home/lupc/Documents/ez-sdr/ez-gui/src/adsb_decoder.rs:187).

### A10 · P1 · The channelizer erases upstream sample discontinuities

**Evidence:** Reproduced. **Area:** Daemon acquisition.

**Trigger:** Two input blocks of length four arrive at sample indices 0 and 1,000, for example after SampleBus overflow.

**Logic and evidence:** VirtualChannel::process ignores block.start_sample, feeds ongoing NCO/FIR history and assigns a new contiguous output_sample_counter. The real channelizer probe produces starts 0 and 4. PacketPipeline's explicit gap detector therefore cannot see a gap that happened before channelization. Correct downstream gap handling is neutralized by an upstream adapter.

**Consequence:** ADS-B overlap, timing, radio filtering and recording clocks can silently bridge missing capture data in daemon mode too.

**Correction:** Track upstream sequence/clock epochs through channelization, reset DSP history on gaps, and express downstream gaps in the output sample clock without inventing continuity.

**Regression requirement:** Inject missing input blocks at decimation 1 and greater than 1. Require a detectable downstream gap and reset histories; include a packet spanning the missing interval.

**Source:** [ez-daemon/src/channelizer.rs:192](/home/lupc/Documents/ez-sdr/ez-daemon/src/channelizer.rs:192), [ez-daemon/src/pipelines/packet.rs:123](/home/lupc/Documents/ez-sdr/ez-daemon/src/pipelines/packet.rs:123).

**Recorded probes:** `channelizer_erases_gap`. See `evidence.json`.

### A11 · P1 · Daemon mode hides every aircraft that lacks a position

**Evidence:** Reproduced. **Area:** Daemon aircraft GUI.

**Trigger:** The daemon has decoded identification, altitude, velocity or surveillance messages but not a complete position.

**Logic and evidence:** map_daemon_aircraft starts with (item.lat?, item.lon?). filter_map consequently discards the whole track. The probe supplies a valid partial aircraft and gets None. Local and UAT modes keep such tracks for the list and represent position validity separately.

**Consequence:** Users see fewer aircraft and lose valid callsign/altitude/velocity information merely by selecting daemon mode. Surface aircraft without a receiver-reference bootstrap are especially affected.

**Correction:** Map all valid identities and preserve optional position. Require position only for map placement, range calculations and position-dependent alerts.

**Regression requirement:** Send callsign-only, altitude-only, velocity-only and positioned daemon snapshots; require all valid tracks in the list and only positioned tracks on the map.

**Source:** [ez-gui/src/app.rs:595](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:595), [ez-gui/src/app.rs:668](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:668).

**Recorded probes:** `daemon_adapter`. See `evidence.json`.

### A12 · P1 · The daemon adapter manufactures freshness, heading validity and perfect confidence

**Evidence:** Reproduced. **Area:** Daemon aircraft GUI.

**Trigger:** Map any positioned daemon snapshot, including an old track with missing heading and a reported vertical rate.

**Logic and evidence:** The mapper ignores last_seen_ms and sets seen=Instant::now for every snapshot. It sets has_heading=true even when track_deg is absent; reliable=true, correctedbits=0 and score=1.0 despite no quality fields in the protocol. It drops vertical_rate_fpm and squawk is always None. It labels altitude Barometric because altitude type is absent from telemetry. The probe confirms fresh age, missing-track validity, score 1.0 and a discarded 640 ft/min vertical rate.

**Consequence:** The GUI presents unknown confidence as certainty and retained observations as new observations. Age filters, north-pointing markers and field displays can be wrong even when daemon decoding is correct.

**Correction:** Extend telemetry with address/source type, field validity, acquisition times, position age and real quality. Preserve unknown quality explicitly. Do not infer fresh observations from snapshot arrival.

**Regression requirement:** Replay repeated snapshots with unchanged observation timestamps, missing track, geometric altitude and known quality; require ages to increase and optional fields to survive.

**Source:** [ez-gui/src/app.rs:592](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:592), [ez-proto/src/messages.rs:142](/home/lupc/Documents/ez-sdr/ez-proto/src/messages.rs:142).

**Recorded probes:** `daemon_adapter`. See `evidence.json`.

### A13 · P2 · A position can remain current indefinitely while unrelated traffic continues

**Evidence:** Static confirmed. **Area:** Daemon tracking.

**Trigger:** An aircraft stops transmitting positions but continues identification or surveillance messages.

**Logic and evidence:** PacketPipeline merges each field into AircraftTelemetry and prunes only by the tracker's last message timestamp. No position timestamp or position-expiry state exists. Surface decoding may reuse any prior_position without a freshness test. Snapshot publication/pruning itself occurs after a block count rather than an independent idle timer. The GUI then refreshes seen for the entire retained track as described in A12.

**Consequence:** An old coordinate remains a current map location until the whole aircraft stops sending messages. A stale surface reference can be used again, and an idle pipeline cannot publish expiry promptly.

**Correction:** Expire individual measurements independently; carry their ages through snapshots and run expiry on a bounded timer even without fresh input.

**Regression requirement:** Receive one position, continue non-position messages past the position TTL, and require the aircraft to remain in the list without a map position. Stop input and require an expiry snapshot.

**Source:** [ez-daemon/src/pipelines/packet.rs:195](/home/lupc/Documents/ez-sdr/ez-daemon/src/pipelines/packet.rs:195), [ez-daemon/src/pipelines/packet.rs:253](/home/lupc/Documents/ez-sdr/ez-daemon/src/pipelines/packet.rs:253), [ez-daemon/src/pipelines/packet.rs:248](/home/lupc/Documents/ez-sdr/ez-daemon/src/pipelines/packet.rs:248).

### A14 · P2 · Geometric-position messages and newly acquired surface tracks cannot produce daemon positions

**Evidence:** Reproduced + static surface-path trace. **Area:** Daemon position support.

**Trigger:** An aircraft supplies TC20-22 geometric-altitude position reports, or a surface aircraft is acquired before any airborne position.

**Logic and evidence:** The packet pipeline submits airborne CPR only for TC9-18 although the common decoder and local GUI recognize TC20-22. Surface CPR is submitted only when prior_position already exists, and the daemon has no receiver-position reference in this path. This is distinct from stale-reference handling in A13: these valid inputs cannot bootstrap position at all. A CRC-valid TC20 pair produces a position in the local decoder and no position in the actual daemon merge implementation in the probe.

**Consequence:** Valid aircraft remain unpositioned or absent from the daemon GUI because of A11. Local and daemon reception expose different protocol coverage.

**Correction:** Handle both airborne-position families, and provide a validated configured receiver reference for surface decoding with explicit range constraints.

**Regression requirement:** Use even/odd TC20-22 pairs and a surface-only capture near a configured receiver. Compare supported local and daemon output.

**Source:** [ez-daemon/src/pipelines/packet.rs:202](/home/lupc/Documents/ez-sdr/ez-daemon/src/pipelines/packet.rs:202), [ez-daemon/src/pipelines/packet.rs:205](/home/lupc/Documents/ez-sdr/ez-daemon/src/pipelines/packet.rs:205).

**Recorded probes:** `geometric_cpr_omission`. See `evidence.json`.

### A15 · P2 · DF11 interrogator parity is treated as a CRC error before IID scoring

**Evidence:** Reproduced. **Area:** ADS-B decoder contracts.

**Trigger:** A previously learned aircraft sends a valid DF11 reply with a nonzero interrogator-identifier overlay, with CRC repair disabled as in the local receiver.

**Logic and evidence:** score_mode_s_message first calls correct_message, whose short-frame helper requires a zero CRC syndrome. It then examines IID, but the correction count is already -1 for a valid nonzero overlay, causing the DF11 scoring branch to return Uncorrectable. The probe learns ABCDEF and submits a valid IID=1 reply with max_errors=0: both demodulation acceptance and the stateless field decoder reject it. With repair enabled, overlay bits can instead be treated as errors to correct. The field decoder independently demands an all-zero syndrome for DF11, so fixing only the demodulator would leave a downstream contract mismatch.

**Consequence:** Legitimate all-call replies with IID overlays are lost or mislabeled as repaired data. The dedicated IID scoring paths do not establish correct behavior because the earlier generic correction stage has already interpreted the parity incorrectly.

**Correction:** Handle parity by downlink format before generic CRC repair: preserve valid DF11 IID overlays, validate the non-IID syndrome bits, and pass validated ModesMessage/address metadata to semantic consumers.

**Regression requirement:** Supply valid DF11 IID=0 and nonzero-IID vectors, known and unknown addresses, under repair limits 0/1/2. Require correct acceptance policy, no false correction count and matching downstream identity.

**Source:** [dump1090/src/demod.rs:210](/home/lupc/Documents/ez-sdr/dump1090/src/demod.rs:210), [dump1090/src/mode_s.rs:33](/home/lupc/Documents/ez-sdr/dump1090/src/mode_s.rs:33), [ez-daemon/src/pipelines/packet.rs:191](/home/lupc/Documents/ez-sdr/ez-daemon/src/pipelines/packet.rs:191).

**Recorded probes:** `df11_iid_contract`. See `evidence.json`.

### A16 · P2 · 1090 message-rate and daemon totals are disconnected from message reception

**Evidence:** Static confirmed. **Area:** ADS-B receiver status.

**Trigger:** Receive local or daemon 1090 MHz traffic and inspect message rate or totals.

**Logic and evidence:** AdsBPanel::record_message updates the rolling message-rate deque only from poll_uat. The local path assigns total_messages and decode_stats but never records message arrival in that deque. The daemon aircraft path replaces aircraft without updating total_messages or decode_stats. Rate pruning also happens on new messages, so a stopped UAT feed can retain its last rate state.

**Consequence:** A functioning 1090 receiver can display zero message rate, and daemon reception can retain zero/stale totals. These indicators cannot be used to diagnose antenna, gain or decode performance.

**Correction:** Publish per-receiver counter/rate snapshots with a common clock and expire the rate window on time, including idle reception.

**Regression requirement:** Feed each receiver mode at a known message rate, then stop. Require correct totals and a rate that reaches zero after the window.

**Source:** [ez-gui/src/adsb_panel.rs:565](/home/lupc/Documents/ez-sdr/ez-gui/src/adsb_panel.rs:565), [ez-gui/src/adsb_panel.rs:659](/home/lupc/Documents/ez-sdr/ez-gui/src/adsb_panel.rs:659), [ez-gui/src/app.rs:934](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:934).

### A17 · P2 · Trail collection depends on whether the Planes tab is rendered

**Evidence:** Static confirmed. **Area:** Aircraft history.

**Trigger:** Receive aircraft while Listen or Satellites is the active tab, then open Planes.

**Logic and evidence:** The app continuously updates aircraft, but the trail append/prune loop is inside AdsBPanel::ui_list. It does not run as a reception task. Switching back starts or resumes history from the current displayed snapshot, without the positions observed while the tab was hidden. Local position-validity masking is likewise refreshed on feed/get_aircraft rather than an independent aircraft-model tick; this requires care on prolonged idle input.

**Consequence:** Trail history is a history of viewed frames rather than received flight positions. Tab selection changes the analytical result.

**Correction:** Collect trails when timestamped position observations arrive, and prune on a model timer. Renderers should consume history without controlling it.

**Regression requirement:** Feed a moving aircraft on another tab, then show Planes and require the same timestamped trail as continuously visible reception.

**Source:** [ez-gui/src/adsb_panel.rs:2166](/home/lupc/Documents/ez-sdr/ez-gui/src/adsb_panel.rs:2166), [ez-gui/src/app.rs:2251](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:2251).

### R01 · P1 · The app's CF32 writer and replay reader disagree about amplitude units

**Evidence:** Reproduced. **Area:** Recording and replay.

**Trigger:** Replay a CF32 file produced by the satellite recorder; daemon recordings from the raw-ADC complex path have the same convention issue.

**Logic and evidence:** The writer stores byte-minus-127.4 values, approximately -127.4 to +127.6. The replay reader assumes normalized -1 to +1, clamps, then multiplies by 127. The real writer-to-reader probe changes [90,110,127,128,140,160,180,200] into [0,0,77,203,254,254,254,254]. This is severe nonlinear clipping, not ordinary 8-bit quantization.

**Consequence:** Recorded signals cannot be faithfully replayed in the GUI. Demodulation, spectrum and ADS-B behavior on those files can be radically different from acquisition.

**Correction:** Choose and document a single CF32 amplitude convention, write it in metadata, and support the existing convention explicitly when importing older files.

**Regression requirement:** Round-trip representative low, medium and full-scale IQ, complex tones and ADS-B pulses through each app writer and reader. Require a bounded quantization error and preserved decode output.

**Source:** [ez-gui/src/satellite/recorder.rs:163](/home/lupc/Documents/ez-sdr/ez-gui/src/satellite/recorder.rs:163), [ez-gui/src/source_manager.rs:781](/home/lupc/Documents/ez-sdr/ez-gui/src/source_manager.rs:781), [ez-daemon/src/hardware/rtlsdr.rs:282](/home/lupc/Documents/ez-sdr/ez-daemon/src/hardware/rtlsdr.rs:282), [ez-daemon/src/recording.rs:256](/home/lupc/Documents/ez-sdr/ez-daemon/src/recording.rs:256).

**Recorded probes:** `cf32_roundtrip`. See `evidence.json`.

### R02 · P2 · CF32 replay progress advances in output bytes instead of file bytes

**Evidence:** Reproduced. **Area:** Recording and replay.

**Trigger:** Play CF32 data and inspect replay_position/replay_size before EndOfStream.

**Logic and evidence:** Eight input bytes per complex sample are converted to two output bytes. recv_samples increments replay_position by the converted chunk length, while replay_size is the original file length. The probe reads the entire 32-byte file and reports position=8, size=32 before EOF processing.

**Consequence:** The progress indicator runs at one quarter of the true file progress and jumps at EOF. Any duration or seek reasoning based on those fields is misleading.

**Correction:** Track consumed input bytes or capture sample indices separately from the converted payload size.

**Regression requirement:** Advance through CU8 and CF32 files with known sample counts and require equivalent fractional progress and duration.

**Source:** [ez-gui/src/source_manager.rs:1301](/home/lupc/Documents/ez-sdr/ez-gui/src/source_manager.rs:1301), [ez-gui/src/source_manager.rs:349](/home/lupc/Documents/ez-sdr/ez-gui/src/source_manager.rs:349).

**Recorded probes:** `cf32_roundtrip`. See `evidence.json`.

### R03 · P1 · CF32 recording failures and overflow are reported as success

**Evidence:** Reproduced. **Area:** Satellite recorder.

**Trigger:** File creation fails in the worker, disk writing/flush fails, or the bounded 64-chunk queue overflows.

**Logic and evidence:** start returns Ok before File::create runs. Creation/write failures only print to stderr; write ignores try_send failure; is_running checks handle presence rather than worker completion. stop discards join, flush and sidecar errors and returns Ok. The probe forces a file-open failure: start succeeds, is_running remains true after the worker exits, and stop succeeds with zero bytes. The invalid path is a controlled fault injection; normal permission/disk failures take the same error branch.

**Consequence:** A user can believe a satellite pass was recorded when no usable recording exists, or obtain silently shortened data with no gap accounting. Sidecar success cannot be trusted either.

**Correction:** Open synchronously or acknowledge worker readiness, report failures and overflow to the GUI, account for lost samples, and return finalization errors. Base running state on worker liveness plus error state.

**Regression requirement:** Inject open, write, flush and sidecar failures and queue overflow. Require truthful status, actionable errors and accurate retained/lost sample counts.

**Source:** [ez-gui/src/satellite/recorder.rs:34](/home/lupc/Documents/ez-sdr/ez-gui/src/satellite/recorder.rs:34), [ez-gui/src/satellite/recorder.rs:112](/home/lupc/Documents/ez-sdr/ez-gui/src/satellite/recorder.rs:112), [ez-gui/src/satellite/recorder.rs:118](/home/lupc/Documents/ez-sdr/ez-gui/src/satellite/recorder.rs:118), [ez-gui/src/satellite/recorder.rs:138](/home/lupc/Documents/ez-sdr/ez-gui/src/satellite/recorder.rs:138).

**Recorded probes:** `cf32_failure_reporting`. See `evidence.json`.

### R04 · P1 · Restarting CF32 recording in the same second overwrites the previous capture

**Evidence:** Reproduced. **Area:** Satellite recorder.

**Trigger:** Start, stop and restart for the same satellite and frequency within one UTC second.

**Logic and evidence:** The filename contains only second-resolution time, satellite and frequency. File::create truncates an existing file. The probe obtains the same path twice and observes the first 16-byte capture replaced by an 8-byte capture; the sidecar shares the collision. The daemon recorder already uses create_new, illustrating that this is avoidable.

**Consequence:** Previously recorded data is destroyed without a distinct overwrite action or warning.

**Correction:** Use create_new with a unique session identifier/collision retry, and bind metadata to that exact file identity.

**Regression requirement:** Rapidly start several sessions with identical satellite/frequency and require unique files, preserved contents and matching metadata.

**Source:** [ez-gui/src/satellite/recorder.rs:44](/home/lupc/Documents/ez-sdr/ez-gui/src/satellite/recorder.rs:44), [ez-gui/src/satellite/recorder.rs:59](/home/lupc/Documents/ez-sdr/ez-gui/src/satellite/recorder.rs:59).

**Recorded probes:** `cf32_overwrite`. See `evidence.json`.

### R05 · P1 · A single IQ recording can contain multiple tuning or sample-rate epochs under one metadata header

**Evidence:** Static confirmed. **Area:** Recorder capture integrity.

**Trigger:** Record IQ, then retune or change sample rate while recording remains active, including changes from scanner, settings or scheduler.

**Logic and evidence:** RecorderPanel captures frequency, rate, gain and offset once at start. write_samples subsequently accepts bytes without capture generation/rate metadata, and the app does not split or stop IQ recording when its capture tuple changes. The satellite CF32 writer similarly keeps its start-time metadata while receiving untagged byte batches. Audio-device rate changes are handled for WAV, but that does not solve raw IQ capture changes.

**Consequence:** The file's declared rate/frequency can be wrong for part of its contents. Offline timing, spectral coordinates and ADS-B decoding cannot reconstruct those changes.

**Correction:** Bind a recording to an immutable capture epoch. Split files or store an indexed per-segment clock/tuning manifest on retune, rate change and loss.

**Regression requirement:** Record across tuner/rate changes and assert either an intentional stop/split or recoverable segment metadata. Verify each segment independently against its source samples.

**Source:** [ez-gui/src/recorder_panel.rs:312](/home/lupc/Documents/ez-sdr/ez-gui/src/recorder_panel.rs:312), [ez-gui/src/recorder_panel.rs:586](/home/lupc/Documents/ez-sdr/ez-gui/src/recorder_panel.rs:586), [ez-gui/src/app.rs:890](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:890).

### R06 · P2 · Daemon recording events cannot update shared recording state while the app holds its lock

**Evidence:** Reproduced. **Area:** Daemon recording status.

**Trigger:** drain_daemon_events handles a Recording event while holding SharedState, as it normally does.

**Logic and evidence:** RecorderPanel::handle_daemon_recording updates its own recording flag, then tries to acquire the already held non-reentrant mutex. try_lock fails and the shared flag remains unchanged. The probe reproduces panel=true and SharedState.recording=false under the same lock order.

**Consequence:** The REC indicator and remote/MQTT/Discord status using shared state can say idle while daemon recording is active, or remain active after a stop depending on prior state.

**Correction:** Apply the shared status through the existing guard, or collect panel actions and execute them after releasing the shared lock.

**Regression requirement:** Drive active and inactive Recording events through the actual app event path and assert consistent panel, shared, status-strip and exported state.

**Source:** [ez-gui/src/app.rs:625](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:625), [ez-gui/src/app.rs:677](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:677), [ez-gui/src/recorder_panel.rs:103](/home/lupc/Documents/ez-sdr/ez-gui/src/recorder_panel.rs:103).

**Recorded probes:** `daemon_recorder_lock`. See `evidence.json`.

### R07 · P2 · Recording duration enforcement lives in an unrendered panel

**Evidence:** Reproduced; dormant configuration surface. **Area:** Recorder lifecycle.

**Trigger:** A recording has a duration limit, but RecorderPanel::ui is not called.

**Logic and evidence:** max_duration_mins and quick-duration expiry are enforced only in ui. The probe sets a 60-second limit and a start time 61 seconds ago: write_samples accepts more data and recording remains active; a headless egui rendering of ui then stops it. Current CentralApp has no RecorderPanel::ui call and the curated menu omits Recorder, although Ctrl-R and web commands still start recording. The duration controls themselves are currently inaccessible through that menu, so this is a lifecycle defect with a dormant settings surface, not a claim that every Ctrl-R recording has an active limit.

**Consequence:** Any enabled deadline depends on painting the settings window; the actual app has no periodic recorder deadline check. Recorder controls and recorded-file browser are also absent from the current UI routing.

**Correction:** Move deadline checks into a model tick or recording worker and wire the intended recorder controls into an accessible current surface.

**Regression requirement:** Start a limited recording, change tabs/close controls, and require timely stop. Verify the current navigation exposes every supported recording control.

**Source:** [ez-gui/src/recorder_panel.rs:842](/home/lupc/Documents/ez-sdr/ez-gui/src/recorder_panel.rs:842), [ez-gui/src/mode_bar.rs:184](/home/lupc/Documents/ez-sdr/ez-gui/src/mode_bar.rs:184), [ez-gui/src/app.rs:1411](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:1411).

**Recorded probes:** `hidden_recorder_deadline`. See `evidence.json`.

### G01 · P1 · The scanner logs an off-channel signal as a hit at the selected frequency

**Evidence:** Reproduced. **Area:** Scanner measurements.

**Trigger:** Scan 100 MHz with a 12.5 kHz channel while a strong signal exists elsewhere in the 2.048 MHz captured band.

**Logic and evidence:** The app supplies SpectrumAnalyzer::peak_level, the maximum across all FFT bins, to FrequencyScanner::tick. The scanner records that value against current_freq_hz. The probe places a -10 dB peak at 100.2 MHz while the selected 100 MHz channel is -120 dB; it records a hit at 100 MHz. vfo_signal_level already provides an in-band measurement but is not used here.

**Consequence:** Hit logs, hold-on-active, exports and notifications can identify signals at the wrong frequency. One strong adjacent signal can hold the sweep on an empty channel.

**Correction:** Use the selected channel's passband measurement with observation frequency/epoch metadata, or explicitly log the measured peak frequency for a wideband discovery mode.

**Regression requirement:** Place a strong out-of-band carrier and a quiet selected channel; require no channel hit or hold. Then add an in-band carrier and require correct frequency/strength.

**Source:** [ez-gui/src/app.rs:1435](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:1435), [ez-gui/src/scanner.rs:609](/home/lupc/Documents/ez-sdr/ez-gui/src/scanner.rs:609), [ez-gui/src/spectrum.rs:1053](/home/lupc/Documents/ez-sdr/ez-gui/src/spectrum.rs:1053).

**Recorded probes:** `scanner_off_channel_hit`. See `evidence.json`.

### G02 · P2 · The scanner continues making decisions from stale spectra when Listen is hidden or the source is stopped

**Evidence:** Static confirmed. **Area:** Scanner lifecycle.

**Trigger:** Start a scan, switch from Listen to Planes/Satellites, freeze the spectrum, or stop acquisition.

**Logic and evidence:** Local spectrum_batch is filled and FFT results drained only for AppTab::Listen, but scanner.tick runs every app frame without checking source running state or measurement age. It can reuse cached_peak_level while changing current_freq_hz. There is no observation epoch tying a spectrum to the scanner's dwell/tune request.

**Consequence:** Stale signals produce hits or indefinite holds after a tab/source change. The scan result depends on which screen is visible and can attach an old measurement to a new tuning.

**Correction:** Compute scanner measurements independently of display rendering, require a fresh matching tuning epoch after settling, and pause/error on stopped sources or unavailable measurements.

**Regression requirement:** Run the same scan with Listen visible/hidden/frozen and then stop the source; require identical valid hits and no decisions from stale data.

**Source:** [ez-gui/src/app.rs:644](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:644), [ez-gui/src/app.rs:1432](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:1432), [ez-gui/src/scanner.rs:695](/home/lupc/Documents/ez-sdr/ez-gui/src/scanner.rs:695).

### G03 · P2 · The demodulation control queue discards the newest settings instead of coalescing them

**Evidence:** Reproduced. **Area:** DSP control delivery.

**Trigger:** Queue four demodulation settings while no IQ arrives, then request a fifth setting before the next batch.

**Logic and evidence:** configure and configure_advanced use try_send into a capacity-four FIFO. The worker drains controls only after receiving IQ. The fifth setting is lost; callers largely ignore the returned false. The probe queues four FM configurations and a final AM configuration, then feeds IQ: AM is rejected and the frame runs as FM. configure_advanced has the same mechanism. The app can also mark last_radio_profile as applied before knowing the queued update succeeded.

**Consequence:** Audio can use stale mode/filter/profile settings despite a changed UI. Some profiles may remain stale until another change triggers resending; per-batch settings may recover on later batches but the first affected output is still wrong.

**Correction:** Use replaceable latest-value control state with an acknowledgement/version applied to each batch. Do not mark a profile applied when delivery failed.

**Regression requirement:** Flood controls without IQ, then process one batch and require the final requested mode/profile. Change settings under backlog and verify frame configuration version.

**Source:** [ez-gui/src/demod.rs:2125](/home/lupc/Documents/ez-sdr/ez-gui/src/demod.rs:2125), [ez-gui/src/demod.rs:2133](/home/lupc/Documents/ez-sdr/ez-gui/src/demod.rs:2133), [ez-gui/src/app.rs:747](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:747).

**Recorded probes:** `demod_config_not_coalesced`. See `evidence.json`.

### G04 · P2 · Saved hardware and spectrum controls are not consistently restored or applied

**Evidence:** Static confirmed. **Area:** Configuration application.

**Trigger:** Restart with saved advanced device/AGC/direct-sampling/bias/FFT settings, or import a configuration and Save & Apply calibration changes.

**Logic and evidence:** Startup restores source mode/frequency/rate/gain and selected top-level fields but does not call push_advanced, the function that applies advanced hardware selection, AGCs, bias tee, direct-sampling branch and FFT/window settings. CentralApp::apply_advanced configures only DemodWorker. The needs_apply branch also applies only demod advanced controls before handling top-level source values; it does not copy changed config.ppm_correction/lo_offset to the active source. Some spectrum fields are restored separately, so this is partial application, not a claim that all persistence is broken. Rendering persists live display settings back to config, potentially replacing a saved unapplied setting with a runtime default.

**Consequence:** The saved JSON and visible config can disagree with the actual receiver or FFT. A user may open the wrong device, miss expected bias/AGC behavior, or believe a calibration/import took effect when it did not.

**Correction:** Centralize validated configuration-to-runtime application with explicit source/DSP/display groups, ordering and acknowledgement. Restore all supported fields once before capture and apply imported changes through the same path.

**Regression requirement:** Start and import a deliberately nondefault profile covering each hardware/DSP/display field. Compare runtime values and hardware commands, including PPM and LO offset, and require round-trip persistence.

**Source:** [ez-gui/src/app.rs:307](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:307), [ez-gui/src/app.rs:474](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:474), [ez-gui/src/app.rs:1481](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:1481), [ez-gui/src/advanced_panel.rs:103](/home/lupc/Documents/ez-sdr/ez-gui/src/advanced_panel.rs:103).

### G05 · P2 · Applying appearance or integration settings retunes the active receiver to the startup default

**Evidence:** Static confirmed. **Area:** Configuration application.

**Trigger:** Tune away from default_freq_hz, alter a font/theme/integration setting in Settings, then press Save & Apply.

**Logic and evidence:** All needs_apply changes share a branch that sets source rate/gain to defaults and calls radio_ui::tune(default_freq_hz), irrespective of which settings changed. The settings UI describes default frequency as the frequency used when first started. Thus applying unrelated settings changes the current capture rather than only its intended runtime group. Legacy Customize also sets the same flag, but that panel is currently unrendered.

**Consequence:** An appearance or integration change can interrupt a live ADS-B/radio session and mutate an ongoing recording's capture epoch. In ADS-B mode it may move reception away from 1090 MHz.

**Correction:** Track dirty settings by group and preserve the active receiver for appearance/integration-only changes. Treat startup defaults and an explicit live tune request as separate commands.

**Regression requirement:** Change theme/font/MQTT options during a nondefault live tune and assert unchanged center, sample rate, gain and capture generation unless explicitly requested.

**Source:** [ez-gui/src/app.rs:1497](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:1497), [ez-gui/src/config.rs:942](/home/lupc/Documents/ez-sdr/ez-gui/src/config.rs:942).

### G06 · P2 · Bookmark autosave watches collection length and clean exit does not flush pending bookmarks

**Evidence:** Static confirmed. **Area:** Bookmark persistence.

**Trigger:** Add a bookmark and exit within the 15-second debounce, or make a same-length edit through a supported/legacy bookmark path.

**Logic and evidence:** The app checks only cur_len != bm_last_len and ignores bookmarks_modified for autosave. on_exit saves configuration and signal history but not BookmarkDb. A new bookmark remains only in memory until the debounce elapses; same-length name/star/notes edits are not noticed by this autosave path. Some legacy actions explicitly call save, which helps only those actions.

**Consequence:** A clean exit can lose newly added bookmarks, and a dirty flag does not guarantee persistence. Same-size modifications through paths without an explicit save can be lost indefinitely.

**Correction:** Use a mutation revision/dirty flag and flush successful dirty writes at clean exit. Surface save failures rather than equating mutation with saved state.

**Regression requirement:** Add/edit/star/delete-and-add bookmarks and immediately exit/reload. Require all intended changes, including equal-length edits, to persist.

**Source:** [ez-gui/src/app.rs:1699](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:1699), [ez-gui/src/app.rs:2003](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:2003), [ez-gui/src/bookmark_manager.rs:337](/home/lupc/Documents/ez-sdr/ez-gui/src/bookmark_manager.rs:337).

### G07 · P2 · The current UI can send an AI request but never polls or renders its result

**Evidence:** Static confirmed. **Area:** AI workflow routing.

**Trigger:** Use the Scanner's AI action; other current surfaces also populate AI prompts.

**Logic and evidence:** The scanner dispatch calls AiPanel::send_message, but CentralApp does not call AiPanel::ui and current mode-bar routing has no AI panel/popup. poll_stream is called only from AiPanel::ui. Worker events therefore remain unconsumed; completion, response display and proposed tool actions do not reach the user. Comments referring to a compact AI popup do not create a route.

**Consequence:** A request can consume an external inference call while the user sees no result or completion. A constructed panel and passing panel tests do not establish a working product flow.

**Correction:** Provide an accessible AI response surface and poll worker state in the app model independently of panel visibility. Avoid dispatching requests that have no result route.

**Regression requirement:** Inject a deterministic AI response through the current Scanner action, hide its window, and require completion plus an accessible response; no live provider is needed.

**Source:** [ez-gui/src/secondary_panel.rs:59](/home/lupc/Documents/ez-sdr/ez-gui/src/secondary_panel.rs:59), [ez-gui/src/ai_panel.rs:1476](/home/lupc/Documents/ez-sdr/ez-gui/src/ai_panel.rs:1476), [ez-gui/src/mode_bar.rs:194](/home/lupc/Documents/ez-sdr/ez-gui/src/mode_bar.rs:194).

### G08 · P2 · Several AI tools report completed operations after changing only flags or shadow settings

**Evidence:** Reproduced; dormant result consumer. **Area:** AI action semantics.

**Trigger:** Execute the approved start_recording, stop_recording or start_adsb tool; change local gain/sample rate/bias/PPM through the tool executor.

**Logic and evidence:** Recording tools set SharedState.recording but do not call RecorderPanel. start_adsb sets adsb_running without starting acquisition or setting 1090/2.4 MHz. The probe reports 'Recording started' with RecorderPanel idle, and ADS-B enabled with the source stopped at 2.048 MS/s. Hardware settings change SourceManager fields without the restart needed by the local hardware worker. Direct frequency edits are later reconciled by the app, so tuning alone is not classified here as permanently ineffective. G07 currently prevents ordinary AI result/tool consumption; these executor defects become user-facing if that routing is restored.

**Consequence:** If exposed, the tools can claim recording/reception success without files or capture, or alter declared clocks without applying hardware settings. Reported status is requested state, not completed action.

**Correction:** Dispatch tools through the same typed commands/lifecycle as keyboard, web and receiver controls, and report success only from actual applied/started state.

**Regression requirement:** Execute each tool through a fake local/daemon adapter and assert file creation, capture center/rate, worker generation and truthful error/success results.

**Source:** [ez-gui/src/ai_panel.rs:807](/home/lupc/Documents/ez-sdr/ez-gui/src/ai_panel.rs:807), [ez-gui/src/ai_panel.rs:866](/home/lupc/Documents/ez-sdr/ez-gui/src/ai_panel.rs:866), [ez-gui/src/ai_panel.rs:885](/home/lupc/Documents/ez-sdr/ez-gui/src/ai_panel.rs:885).

**Recorded probes:** `ai_tool_actions`. See `evidence.json`.

### G09 · P2 · Daemon squelch changes through the spectrum and remote control never reach audio playback

**Evidence:** Static confirmed. **Area:** Daemon radio controls.

**Trigger:** While receiving daemon audio, use the spectrum context menu to set a closing squelch threshold, or move the web remote squelch slider.

**Logic and evidence:** The main radio sidebar correctly disables advanced DSP and unsupported DSB/CW/RAW choices in daemon mode. However, the spectrum context menu still exposes Set squelch and its pending value is applied unconditionally; the web remote also sends SetSquelch and the app unconditionally updates local state. The daemon playback branch uses frame samples, sample rates and volume only, bypassing the local power/CTCSS gate. The protocol and daemon already implement ClientCommand::SetSquelch, but the GUI does not transmit that command. The daemon therefore retains its independent threshold while the GUI and remote snapshot show the new local threshold.

**Consequence:** The spectrum or remote control can show a closed squelch while daemon audio continues according to the old server threshold. This is a reachable control-delivery defect, not a claim that the guarded sidebar exposes every unsupported DSP control.

**Correction:** Route both squelch entry points through one typed command that sends SetSquelch to the active daemon audio channel and tracks acknowledgment/error state, or explicitly disable those entry points for daemon sources. Keep displayed and applied thresholds consistent.

**Regression requirement:** Through the current spectrum and web command routes, raise squelch above a deterministic daemon signal and require muted output plus matching applied-state telemetry. Alternatively require an explicit unsupported-control response and no local-state mutation.

**Source:** [ez-gui/src/spectrum.rs:3554](/home/lupc/Documents/ez-sdr/ez-gui/src/spectrum.rs:3554), [ez-gui/src/app.rs:2222](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:2222), [ez-gui/src/app.rs:1309](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:1309), [ez-gui/src/app.rs:651](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:651), [ez-gui/src/radio_ui.rs:623](/home/lupc/Documents/ez-sdr/ez-gui/src/radio_ui.rs:623), [ez-daemon/src/server.rs:293](/home/lupc/Documents/ez-sdr/ez-daemon/src/server.rs:293).

### G10 · P2 · Stopping a GUI workflow removes a globally shared daemon channel

**Evidence:** Static confirmed. **Area:** Daemon multi-client lifecycle.

**Trigger:** Two GUI clients attach to the same daemon; one stops audio or ADS-B, or changes audio mode.

**Logic and evidence:** Every GUI uses fixed channel IDs 1-4. The daemon subscribe operation attaches to an existing matching-kind ID, and workflow stop sends both Unsubscribe and RemoveChannel. RemoveChannel stops/removes the shared pipeline rather than just this client's subscription. Other clients can still believe they are subscribed because their source manager shadow state has not changed. Aircraft events additionally have no channel identifier, and the GUI audio event path does not check frame.channel_id.

**Consequence:** One client can interrupt another client's reception/recording workflow through an ordinary local stop. Shared configuration and per-client subscriptions are mixed without a clear ownership boundary.

**Correction:** Separate channel ownership from subscription lifetime; either allocate client/session-specific IDs or reference-count explicitly shared channels. Route data and deletion state by identity.

**Regression requirement:** Attach two clients, stop/change modes in one, and require the other to keep receiving or receive an explicit coordinated channel-deletion event. Exercise recordings attached to the channel too.

**Source:** [ez-gui/src/source_manager.rs:394](/home/lupc/Documents/ez-sdr/ez-gui/src/source_manager.rs:394), [ez-gui/src/source_manager.rs:1194](/home/lupc/Documents/ez-sdr/ez-gui/src/source_manager.rs:1194), [ez-daemon/src/server.rs:307](/home/lupc/Documents/ez-sdr/ez-daemon/src/server.rs:307), [ez-daemon/src/state.rs:278](/home/lupc/Documents/ez-sdr/ez-daemon/src/state.rs:278).

### I01 · P2 · A rate-limit condition publishes on every frame for a one-second window and may block the UI

**Evidence:** Static confirmed. **Area:** MQTT scheduling.

**Trigger:** Enable MQTT while the receiver is active, especially with many aircraft or a slow broker.

**Logic and evidence:** The app publishes when last_scheduler_update.elapsed().as_secs() < 1. That timestamp is refreshed about every five seconds, so the condition is true on every frame in the following one-second window, not once every five seconds. At 60 frames/s it emits roughly 60 sets per window: SDR state, signal, passes and one publication per aircraft. publish uses blocking rumqttc::Client::publish into a bounded request queue while the shared GUI lock is held. The burst math is a pacing example; broker backpressure was not exercised against an external service.

**Consequence:** Unnecessary QoS traffic can flood telemetry and turn broker slowness into UI stalls and RF block loss. The scheduler clock also controls an unrelated integration's publish cadence.

**Correction:** Use a dedicated last-published deadline and non-blocking/coalesced publishing on a worker. Bound stale telemetry and expose backpressure counters.

**Regression requirement:** Run a synthetic 60 Hz loop with a fake broker and count exactly one set per intended interval. Saturate the broker queue and require bounded GUI latency.

**Source:** [ez-gui/src/app.rs:1653](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:1653), [ez-gui/src/mqtt.rs:209](/home/lupc/Documents/ez-sdr/ez-gui/src/mqtt.rs:209).

### I02 · P2 · Changing the broker while enabled does not change the active connection

**Evidence:** Static confirmed. **Area:** MQTT configuration.

**Trigger:** Use Settings to change mqtt_broker while MQTT already has a Client.

**Logic and evidence:** set_enabled replaces the public broker/topic fields then calls connect. connect immediately returns if client.is_some, so the existing socket and MqttOptions keep the old endpoint. The new prefix may take effect on publications while their destination remains the old broker. This is separate from reconnect-after-failure behavior, which already has a grace period and retry logic.

**Consequence:** The displayed/saved endpoint can disagree with where telemetry is actually sent until a disconnect or failure forces reconnection.

**Correction:** Compare connection-affecting settings and reconnect atomically on endpoint changes, reporting the endpoint actually connected.

**Regression requirement:** Use two loopback fake brokers, change the endpoint while connected, and require the old socket to close and publications to reach only the new endpoint.

**Source:** [ez-gui/src/mqtt.rs:74](/home/lupc/Documents/ez-sdr/ez-gui/src/mqtt.rs:74), [ez-gui/src/mqtt.rs:86](/home/lupc/Documents/ez-sdr/ez-gui/src/mqtt.rs:86).

### I03 · P2 · Unpositioned tracks are exported with numeric zero coordinates

**Evidence:** Static confirmed. **Area:** Aircraft exports and notifications.

**Trigger:** Local reception creates a callsign-only aircraft, or a retained aircraft no longer has a valid position.

**Logic and evidence:** MQTT publish_aircraft serializes lat/lon/altitude/speed without position validity or optional measurements. The app's first-seen Discord aircraft path likewise passes numeric fields without requiring a valid position, and its seen_aircraft deduplication prevents a later positioned observation from replacing that first-arrival notification. Numeric zero is valid geography and cannot stand for unavailable data.

**Consequence:** External consumers can receive phantom coordinates at (0,0), unavailable values presented as measurements and stale first-seen information. The map correctly suppressing a marker does not protect exports.

**Correction:** Serialize missing measurements as null with validity/source/age, and trigger position-dependent notifications only from an eligible observation or an explicit update policy.

**Regression requirement:** Export callsign-only, stale-position and positioned tracks; require no invented coordinate and a clear update when a position becomes available.

**Source:** [ez-gui/src/mqtt.rs:254](/home/lupc/Documents/ez-sdr/ez-gui/src/mqtt.rs:254), [ez-gui/src/app.rs:1109](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:1109).

### S01 · P2 · A later pass of the same satellite can fail to auto-tune

**Evidence:** Static confirmed. **Area:** Satellite scheduling.

**Trigger:** A satellite pass auto-tunes, the pass ends, the user tunes elsewhere, then the next active job is the same satellite.

**Logic and evidence:** The auto-tune guard compares only the satellite name against last_auto_tuned_satellite. LOS clears last_active_pass_sat but never clears the auto-tuned name. A new pass of the same satellite is therefore treated as already handled. Manual-tune cooldown is updated in selected GUI/keyboard paths, but web, rigctl and scanner paths do not consistently update it. The sidebar's auto_tune boolean is also separate from the scheduler's enabling state.

**Consequence:** Scheduled reception can remain on the wrong frequency for a later pass, or unexpectedly override some manual tuning paths. A green auto-tune indication is not proof of a completed tune for this pass.

**Correction:** Key scheduled actions by pass/job identity and capture applied state. Reset at job completion and use one manual-tune command that updates the same cooldown for all callers.

**Regression requirement:** Run two passes of the same satellite with a manual tune between them and require one tune per pass. Repeat using GUI, web, rigctl and scanner manual changes.

**Source:** [ez-gui/src/app.rs:1373](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:1373), [ez-gui/src/app.rs:1396](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:1396), [ez-gui/src/satellite_panel.rs:55](/home/lupc/Documents/ez-sdr/ez-gui/src/satellite_panel.rs:55).

### S02 · P2 · Changing observer location leaves cached passes and trajectories for the old location

**Evidence:** Static confirmed. **Area:** Satellite observer state.

**Trigger:** Populate upcoming passes/trajectory, then change observer latitude/longitude while the cache is fresh.

**Logic and evidence:** TleEngine's upcoming_passes cache is invalidated by a 60-second age, not by observer coordinates. Settings directly assigns observer fields without invalidating cached_at. SatellitePanel loads a trajectory once until selection/import resets trajectory_loaded, without considering observer changes. The UI can show the new observer text alongside old observer-dependent pass times or azimuth/elevation trajectories.

**Consequence:** Timing and alignment advice can remain wrong after relocation even though location controls appear applied. The pass cache usually recovers after 60 seconds; the trajectory can remain stale longer.

**Correction:** Include observer coordinates and TLE/version/time horizon in cache identity, and invalidate all dependent products on observer changes.

**Regression requirement:** Compute at two distant observers without waiting for expiry; require new pass/trajectory data immediately and consistent observer labels.

**Source:** [ez-gui/src/tle_engine.rs:177](/home/lupc/Documents/ez-sdr/ez-gui/src/tle_engine.rs:177), [ez-gui/src/app.rs:356](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:356), [ez-gui/src/satellite_panel.rs:141](/home/lupc/Documents/ez-sdr/ez-gui/src/satellite_panel.rs:141).

### S03 · P2 · Map panning is reset every second and satellite markers use stale projection parameters

**Evidence:** Static confirmed. **Area:** Satellite map interaction.

**Trigger:** Pan or zoom the satellite map while tracking continues.

**Logic and evidence:** tick_realtime assigns map center back to the observer on each roughly one-second update, undoing user pan. MapRenderer stores the satellite marker as a projected Pos2 in set_satellite_position; ui changes center/zoom and recomputes trajectory drawing but retains that already projected marker until the next tracking update. Thus interaction temporarily puts marker and trajectory in different coordinate systems.

**Consequence:** Users cannot keep a selected map viewport, and the aircraft/satellite geometry can appear inconsistent while zooming or panning. This is coordinate/state logic, not a visual-style objection.

**Correction:** Separate observer position from camera center, recenter only by an explicit follow action, and store geographic satellite coordinates for projection during each render.

**Regression requirement:** Pan without follow mode and require stable center across tracking ticks. Zoom/pan a known satellite and require marker and trajectory projections to agree every frame.

**Source:** [ez-gui/src/satellite_panel.rs:109](/home/lupc/Documents/ez-sdr/ez-gui/src/satellite_panel.rs:109), [ez-gui/src/satellite/map_renderer.rs:37](/home/lupc/Documents/ez-sdr/ez-gui/src/satellite/map_renderer.rs:37), [ez-gui/src/satellite/map_renderer.rs:61](/home/lupc/Documents/ez-sdr/ez-gui/src/satellite/map_renderer.rs:61).

### L01 · P2 · Ordinary TCP fragmentation can corrupt a valid rigctl command

**Evidence:** Reproduced. **Area:** Rigctl transport.

**Trigger:** Send 'F 1090000000
' in two TCP writes separated by more than the 20 ms socket read timeout.

**Logic and evidence:** read_line may append partial bytes before returning a timeout. The handler clears its line buffer at the top of every loop, discarding those bytes. The probe sends 'F 1', waits 80 ms, then sends '090000000
'; the server emits RPRT -1 and queues no tuning request. TCP does not guarantee delivery aligned to application writes or newline boundaries.

**Consequence:** A legitimate local radio-control client can fail sporadically under scheduling delay or packet fragmentation, despite a syntactically correct command.

**Correction:** Keep partial line bytes across read timeouts, apply a bounded line limit and process only complete newline-terminated commands.

**Regression requirement:** Split each command at every byte boundary with delays spanning the poll interval and require exactly one equivalent request and response; stop mid-line must remain prompt.

**Source:** [ez-gui/src/rigctl.rs:173](/home/lupc/Documents/ez-sdr/ez-gui/src/rigctl.rs:173), [ez-gui/src/rigctl.rs:174](/home/lupc/Documents/ez-sdr/ez-gui/src/rigctl.rs:174).

**Recorded probes:** `rigctl_fragmentation`. See `evidence.json`.

### V01 · P3 · Raw wideband IQ is presented as symbol-level QPSK quality

**Evidence:** Static confirmed. **Area:** Satellite signal-quality metrics.

**Trigger:** Inspect the satellite signal-quality constellation/EVM during a non-QPSK signal, carrier offset, or ordinary oversampled raw capture.

**Logic and evidence:** The app supplies source bytes directly to ConstellationDisplay before VFO mixing or modem recovery. EVM and phase-error routines compare those arbitrarily selected ADC samples with ideal QPSK quadrants without matched filtering, symbol timing or carrier recovery. The display is available for non-Meteor selected satellites too. Density counts are cumulative even when their points are evicted from the bounded deque, and capture changes do not reset this display, so density and current points also describe different time windows.

**Consequence:** The numbers can look precise while measuring carrier rotation, waveform transitions, other channels or old captures rather than recovered symbol errors. They cannot support a claim of measured decoder link quality.

**Correction:** Label this as raw IQ geometry and omit symbol-quality claims, or feed timestamped recovered symbols with explicit modulation/reference/clock state. Define and reset the density window consistently.

**Regression requirement:** Compare a known QPSK waveform before and after carrier/timing recovery, plus non-QPSK/noise controls. Require clearly qualified metrics and no previous-capture density after a reset.

**Source:** [ez-gui/src/app.rs:892](/home/lupc/Documents/ez-sdr/ez-gui/src/app.rs:892), [ez-gui/src/constellation.rs:67](/home/lupc/Documents/ez-sdr/ez-gui/src/constellation.rs:67), [ez-gui/src/constellation.rs:57](/home/lupc/Documents/ez-sdr/ez-gui/src/constellation.rs:57).

## Recorded behavioral observations

These are exact observed values from the successful 19-probe run. Missing fields in adapter probes were deliberately constructed to test validity, not taken from a live aircraft.

- **icao_collision_ap**: `{"accepted": true, "learned": "ABCDEF", "unlearned_accepted": "000151"}`
- **icao_duplicate_rotation**: `{"duplicate_adds": 8192, "known_after": false}`
- **status_corrupts_vertical_rate**: `{"after_no_emergency_fpm": 1, "before_fpm": 5568}`
- **negative_altitude**: `{"decoded_ft": 0, "encoded_ft": -500}`
- **airspeed_heading_validity**: `{"has_heading": false, "heading": 90}`
- **df18_identity_merge**: `{"df17_and_non_icao_df18_cf1_entries": 1, "merged_id": 11259375}`
- **df11_iid_contract**: `{"crc_repair_limit": 0, "demod_accepted": false, "iid": 1, "previously_learned": true, "semantic_accepted": false}`
- **geometric_cpr_omission**: `{"daemon_position": false, "local_position": true, "type_code": 20}`
- **daemon_adapter**: `{"altitude": 0, "mapped_age_ms": 0, "missing_track_has_heading": true, "no_position_dropped": true, "quality": 1.0, "reliable": true, "reported_age_ms": 1, "vertical_rate": null}`
- **cf32_roundtrip**: `{"input_file_bytes": 32, "original": [90, 110, 127, 128, 140, 160, 180, 200], "progress_bytes": 8, "replayed": [0, 0, 77, 203, 254, 254, 254, 254]}`
- **cf32_failure_reporting**: `{"bytes": 0, "is_running_after_worker_exit": true, "start_ok": true, "stop_ok": true}`
- **cf32_overwrite**: `{"first_bytes": 16, "remaining_bytes": 8, "same_path": true}`
- **daemon_recorder_lock**: `{"panel_recording": true, "shared_recording": false}`
- **hidden_recorder_deadline**: `{"bytes_accepted_after_limit": 8, "duration_limit_s": 60, "elapsed_s": 61, "stopped_only_after_ui": true}`
- **ai_tool_actions**: `{"adsb_claimed": true, "recorder_recording": false, "result": "Recording started", "sample_rate": 2048000, "shared_recording": true, "source_generation_changed": false, "source_running": false}`
- **channelizer_erases_gap**: `{"input_block_len": 4, "input_starts": [0, 1000], "output_starts": [0, 4]}`
- **demod_config_not_coalesced**: `{"frame_mode": "FM", "latest_accepted": false, "latest_requested": "AM"}`
- **scanner_off_channel_hit**: `{"actual_peak_hz": 100200000, "logged_hit_hz": 100000000, "selected_channel_db": -120.0, "wideband_peak_db": -10.0}`
- **rigctl_fragmentation**: `{"delay_ms": 80, "reply": "RPRT -1\n", "requests": 0, "sent_command": "F 1090000000"}`

## Repair order and acceptance gates

1. **Protect capture and recordings first.** Resolve A09/A10 and R01/R03-R05: remove continuous processing from the frame-rate budget, propagate clock/epoch/gap metadata, settle CF32 units, prevent filename overwrite and acknowledge writer failure. The acceptance gate is a byte/signal-faithful round trip plus explicit, recoverable loss/tuning epochs under controlled overload and recording fault injection.
2. **Restore decoder identity and status correctness.** Resolve A01-A04 and A15: exact recent-address membership, expiry based on actual recency, DF18 namespaces and payload interpretation, TC28 status fields, and format-aware parity treatment. The gate is vetted positive/negative wire vectors through demodulation and both semantic paths, including deliberate collisions and mixed address classes.
3. **Unify aircraft measurement contracts.** Resolve A05-A07 and A11-A14 before trusting confidence/age displays. Carry signed optional typed measurements with acquisition times and provenance through telemetry. Keep unpositioned aircraft visible and position-dependent presentation gated separately. The gate is local/daemon/UAT equivalence on the supported shared fields, explicit unsupported fields and correct independent expiry during continuing unrelated traffic.
4. **Make lifecycle work independent of rendering.** Resolve A08/A16/A17, R06/R07, G01/G02/G07 and S01-S03. Collection, expiry, clocks, scanner observations, stream consumption and job identity belong in model/worker logic. The gate is identical observable behavior with tabs visible, hidden and frozen, slow/fast replay, idle input and repeated passes.
5. **Make controls truthful.** Resolve G03-G05/G08/G09: central typed commands, latest-value DSP controls with applied versions, complete validated configuration application, supported-mode/capability UI and actual recorder/source actions. Requested values, acknowledged runtime values and errors must remain distinguishable. The gate is current-route tests, not only direct panel/helper tests, through local and daemon fake adapters.
6. **Finish persistence and integration behavior.** Resolve G06/G10/I01-I03/L01 and qualify V01. The gate includes immediate clean exit, equal-length bookmark edits, two-client daemon isolation, broker changes/backpressure on loopback fakes, nullable aircraft exports and every TCP fragmentation boundary.

Avoid another broad clean-up pass that merely adjusts Rust syntax or adds tests reproducing the implementation's current assumptions. Tests should assert the contract at the boundary: measured channel versus logged frequency, recorded units versus replayed units, observation age versus snapshot age, accepted wire identity versus tracked key, and acknowledged command versus actual file/hardware/worker state.

## Additional experiments, not counted as confirmed findings

- **RF sensitivity and the weak-signal policy:** the local semantic stage discards signal_level below 0.001 (about -30 dBFS if this is normalized power). Test calibrated low-amplitude but high-SNR messages against a reference decoder before accepting the comment that such traffic is almost certainly noise. No calibrated RF measurement was made here.
- **Position plausibility:** test CRC-valid but implausible jumps and geographically ambiguous surface frames against vetted captures. Per-message CRC is necessary but does not establish physical continuity, NIC/SIL meaning or an appropriate receiver-relative range policy. Do not silently add a motion filter without specifying its validity behavior.
- **Real USB and disk shutdown:** local stop joins the synchronous hardware-reading worker on the GUI thread; recording stop joins writer work. Exercise unplug, slow/full filesystem and device-driver blocking with bounded GUI-latency instrumentation. Ordinary file tests cannot prove those operations remain responsive under hardware failure.
- **GUI performance on actual displays:** measure p50/p95/p99 frame and capture-processing time under dense aircraft, map interaction, spectrum rendering and recordings at 2.4 MS/s. A09 provides conditional capacity arithmetic, not an observed refresh rate or CPU benchmark.
- **Mixed/slow network streams:** saturate daemon audio/spectrum/aircraft queues and confirm snapshot coalescing, stream gap visibility and control delivery. Apply MQTT backpressure with a loopback fake rather than publishing test traffic to the user's real broker.
- **Observer/pass recomputation cost:** TLE pass calculations are synchronous under the shared lock when the cache expires. Benchmark realistic catalog sizes and avoid turning orbit refresh into capture stalls; no timing result is fabricated in this report.
- **Airport database replacement:** deletion of current tables precedes the insertion transaction in store_sqlite. A controlled insert-failure test should establish whether the previous usable database survives. This supporting-module concern is kept outside the formal core finding count because the full current download workflow was not fault-injected.
- **Pinned compiler and real front end:** rerun with the repository's Rust 1.92 toolchain, then exercise the running application and real supported receiver/audio devices. The audit artifacts do not certify hardware support, all screen sizes, accessibility or long-duration physical reception.

## Reproducing and reading the evidence

The committed files alongside this report are:

- `findings.json`: structured finding records, validated source locations and per-finding probe associations.
- `evidence.json`: probe observations, Rust version, audited source commit and SHA-256 hashes for the tested source files.
- `reproduce.py`: the test-only harness generator/runner, using existing workspace rlibs and the matching Rust toolchain. It can be run from any directory because it resolves the repository relative to its own location.
- `generated-probes.rs.txt`: the exact Rust harness used for the recorded run. Private code is included/extracted for visibility only; it is not a production patch. Temporary include paths are run-specific, so use the Python generator for a new execution.
- `probe-output.txt`: full probe stdout/stderr, including the deliberately injected CF32 creation failure and loopback rigctl response.
- `baseline-test-output.txt`: complete existing workspace test output, including ignored-test counts.
- `graphify-update-output.txt`: successful AST-only graph update output.
- `audit-dashboard.canvas.tsx.txt`: a versioned snapshot of the self-contained interactive audit canvas source; the working canvas is in the managed canvas directory outside the repository.

To reproduce the recorded contracts on this checkout, first build/test with the same compiler and then run the script:

```bash
PATH="$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$PATH" cargo test --workspace --all-targets
python3 audit/2026-10-04/reproduce.py
```

`EZ_AUDIT_RUST_BIN` can select a matching Rust toolchain directory for the harness. The compiled rlibs must have been built with that compiler. On this host librtlsdr is under `/usr/local/lib`; the script adds that native library search path. Adapt it to another host if its libraries live elsewhere. A repaired implementation is expected to make these defect-preserving assertions fail until they are converted to the desired regression assertions described per finding.

No real external notifications or provider calls are needed for these probes. An audit commit preserves the evidence, not a claim that the defects have been fixed.
