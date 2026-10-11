# Audit remediation — in progress

The original REPORT.md, findings.json and validation.json describe the audited baseline and remain historical evidence. This ledger tracks repairs against all 33 findings without changing their original repair gates.

32 findings have software repairs with passing regressions; 1 remains open. The goal is active. Off-air mission compatibility and physical hardware/audio/disk release evidence remain unverified. The user no longer has the earlier Meteor recordings. A public Meteor-M2-4 recording, reference CADUs and an image were downloaded; a 20-second 72k RF slice now matches 175 reference CADUs; complete-pass RF/80k mission compatibility remains pending; corrected-CADU image comparison passes, and the ZIQ stream has an unfinished compressed tail. See public-meteor-reference.json.

## Implemented and software verified

- **R19 — Daemon narrowband listening uses a wide fixed channel without mode-specific RF selection**: Separate RF width, channel rate and PCM rate in protocol v12; filter complex RF before detection and selected-RF squelch. Preserve independent RF/PCM clocks and validate atomic desktop retunes.
  Evidence: 47 focused tests, all-target Clippy, default daemon/GUI builds, formatting and AST graph refresh pass. See repair-daemon-selection-verification.json.

- **R12 — Daemon SSB rejection degrades sharply at the actual GUI channel rate**: Define the sideband response in Hz at the exact channel clock, and restore the mixer translation before real projection to preserve voice pitch.
  Evidence: all 336 voice checks and 21 unique affected tests pass; default daemon build, all-target Clippy, formatting and AST graph refresh pass. Minimum measured opposite rejection is 92.681208 dB. See repair-ssb-selectivity-verification.json.

- **R09 — AM audio aliases when the selected RF band exceeds output audio Nyquist**: Filter detected AM, SSB, DSB and CW audio before device-rate conversion, including nonlinear envelope harmonics and CW beat mixing. Preserve headroom, stream duration and device-clock audio controls.
  Evidence: six focused regressions and all 64 unique affected tests pass, alongside GUI build, Clippy, formatting and graph refresh. See repair-detected-antialias-verification.json.

- **R20 — Scanning can wait forever on a frequency a fixed replay capture cannot contain**: Validate complete scan grids and bookmark mode profiles against shared, sided capture bounds before tuning. Reject unsupported coverage with an explicit frequency/range result and revalidate source/rate/decimation/center/bandwidth changes.
  Evidence: five new regressions, 32 edge tuning checks and all 80 affected scanner/tuning/mode tests pass. Existing fresh-observation guards remain active. See repair-scan-verification.json.

- **R06 — USB/LSB power squelch measures a symmetric band instead of the selected sideband**: Share actual nominal RF geometry with demodulation, native measurement and scanner selection, including DSB upper/lower.
  Evidence: 810 sideband cases, 12 native squelch/recording-monitor cases and 12 scanner cases pass. Maximum wanted gain error is 0.00156 dB; maximum opposite reading is −95.37 dBFS. See repair-power-verification.json.
- **R07 — Narrow power measurements can return silence because no FFT bin fits**: Infer fractional spectral peaks with adaptive resolution and controlled leakage. The metric is carrier peak amplitude in dBFS; complete narrow observations take up to 200 ms.
  Evidence: all 451 integer CW widths and 72 edge/fractional-clock cases pass, alongside chunk/reset/window-cadence checks. The original carrier fixture now supplies a complete adaptive observation without changing its amplitude gate. See repair-power-verification.json.

- **R05 — Local FM loudness changes with RF sample rate and RF decimation**: Use physical 5 kHz NFM / 75 kHz WFM amplitude units at the actual discriminator clock. Correct interval averaging, RF filter phase distortion and IF interpolation/reconstruction loss; share mono/stereo deviation measurements and reset semantics.
  Evidence: 216 independent tone cases, 12 continuous stereo-MPX cases and 24 stereo-downmix comparisons pass strict gain/pitch/count/deviation gates. See repair-fm-verification.json.

- **S03 — Viterbi metrics saturate and permanently kill the decoder**: Normalize finite path metrics and retain odd coded bits across chunks.
  Evidence: viterbi::tests::noisy_aged_paths_recover_clean_bits_and_arbitrary_bit_partitions; 8,192,000-noise-step audit probe: subsequent clean comparison has zero errors.
- **S05 — Frame search skips an earlier valid marker in favor of a later cleaner one**: Select admissible markers chronologically; validate RS before consuming a candidate boundary.
  Evidence: frame_sync::tests::earlier_imperfect_marker_is_preserved_at_every_chunk_size; frame_sync::tests::invalid_overlapping_candidate_does_not_consume_valid_frame.
- **S06 — Space-packet headers split across transfer frames are discarded**: Retain incomplete primary headers and bodies across packet zones and idle-only frames.
  Evidence: ccsds::tests::every_header_and_body_split_survives_continuations_and_idle_frames; ccsds::tests::split_header_continuation_ends_at_new_header_pointer; ccsds::tests::gap_discards_partial_header_until_a_known_new_header.
- **S07 — Same-APID packets on different virtual channels overwrite each other**: Own in-progress bytes by virtual channel, including independent gap handling.
  Evidence: ccsds::tests::same_apid_on_two_channels_keeps_bytes_and_frame_gaps_isolated.
- **S09 — JPEG dequantization retains an extra half-unit in every table entry**: Use the mission integer quantization rule before dequantization.
  Evidence: image_builder::tests::mission_quantizers_are_integers_on_both_quality_branches; image_builder::tests::nonflat_independent_libjpeg_pixels_match_with_one_level_idct_tolerance; image_builder::tests::nonzero_dc_exact_vector_decodes_uniform_pixel_160.
- **R01 — Listen-tab CS8 replay interprets signed bytes as unsigned IQ**: Convert CS8 signed components to CU8 before byte publication.
  Evidence: demod::native::tests::audit_signed_file_replay.
- **R02 — Local float replay discards CF32 dynamic range before radio DSP**: Keep normalized native CF32 samples in capture blocks and radio DSP; preserve CF32 export precision. Explicitly label the optional CU8 compatibility export and record its conversion in metadata.
  Evidence: source_manager::tests::native_float_replay_preserves_weak_iq_headroom_and_clock; radio_iq::tests::native_float_processing_is_chunk_invariant_at_sub_byte_amplitudes; demod::native::tests::native_float_replay_reaches_rf_preview_and_audio_at_sub_byte_amplitudes; raw_recording::tests::cf32_recording_keeps_native_float_precision_and_headroom.
- **R08 — Old capture generations are accepted under a new receiver session's controls**: Reject stale capture generations before DSP/publication, without sending errors to new-session audio recorders.
  Evidence: demod::native::tests::audit_reject_old_capture_generation; demod::native::tests::native_radio_resets_every_state_at_capture_boundaries.
- **R11 — Reset disables the configured audio high-pass until coefficients are recomputed**: Retain configured HPF coefficients when resetting delay state.
  Evidence: demod::tests::reset_preserves_configured_highpass_from_the_first_block.
- **R21 — IQ-only recording does not finish when the capture session ends**: Bind pre-armed recorders to their first session, drain EOF without waiting for another block, and finalize before a replacement session. Propagate source-end metadata to the UI finalizer.
  Evidence: raw_recording::tests::iq_only_recorder_drains_eof_and_never_crosses_a_restarted_session; app::app_ui_tests::native_iq_recorders_preserve_every_sample_at_60_hz_and_during_a_gui_stall; app::app_ui_tests::hidden_app_recorders_finalize_each_real_worker_epoch_before_retuned_samples.

- **R14 — Rejected hardware tuning changes capture metadata anyway**: Commit backend caches only after accepted calls. Recover failures by reopening the source at the last verified frequency/rate and original gain request/mode; retire acquisition if restoration fails. Preserve verified terminal status and reset clocks after recovery.
  Evidence: ingest::tests::mutation_before_error_reopens_at_verified_geometry_or_stops_without_reads; ingest::tests::unrecoverable_control_errors_retire_source_and_preserve_verified_status; audit_ingest_control example.
- **R16 — Soapy timeout is treated as a fatal driver failure on its first occurrence**: Classify Soapy -1/zero as bounded retry, -4 as an unknown acquisition gap requiring a new epoch, and other negative codes as terminal errors. Each native read has a 200 ms deadline and returns to ingestion controls/shutdown. Decode native-endian CS16 and reject oversized driver counts.
  Evidence: hardware::soapy_read::tests::documented_timeout_overflow_and_fatal_codes_use_distinct_contracts; ingest::tests::soapy_timeout_retries_overflow_retires_clock_and_fatal_error_stops; ingest::tests::endless_timeouts_keep_controls_available_and_shutdown_bounded.
- **R17 — Daemon RTL gain control reports success without checking the hardware result**: Propagate RTL gain-mode/gain errors at startup and during live control. Use supported gain steps without a sentinel. Publish selected gain, original requested gain, and automatic/manual mode separately. Match GUI acknowledgements against the original request while displaying the selected step.
  Evidence: hardware::rtlsdr::tests::gain_step_and_error_contract_is_shared_by_startup_and_live_control; source_manager::tests::daemon_selected_gain_step_acknowledges_original_request_and_updates_ui; messages::hardware_status_tests::hardware_gain_capabilities_are_backward_compatible_and_distinguish_manual_zero.
- **R18 — rtl_tcp gain is a successful no-op**: Send rtl_tcp tuner mode 0x03 and tenths-dB gain 0x04 at startup and during live changes. Validate controls and update cached state only after socket writes. Document that rtl_tcp has no hardware readback/acknowledgement; keep headerless metadata semantics separate.
  Evidence: hardware::tcp_iq::tests::rtl_tcp_gain_wire_commands_cover_startup_manual_and_auto; hardware::tcp_iq::tests::rtl_tcp_start_applies_cached_tuning_and_reads_uc8; hardware::tcp_iq::tests::invalid_tcp_controls_preserve_cached_metadata.

- **R15 — HackRF callback overflow is silently stitched into continuous capture**: Assign acquisition sample indices before callback queue admission, retain odd components until a complete pair, count rejected complete samples, and return only contiguous transfer ranges with gap/discontinuity clocks. Preserve loss counters through stop/restart. HackRF empty reads yield after 200 ms.
  Evidence: hardware::callback_iq::tests::overflow_counts_acquired_samples_and_never_stitches_retained_transfers; hardware::callback_iq::tests::odd_callback_partitions_preserve_sample_pairs_and_signedness; hardware::callback_iq::tests::silent_callback_queue_returns_to_cancellation_within_one_deadline; recording::capture_tests::callback_overflow_resets_channel_dsp_and_marks_recording_prefix_incomplete.

- **R22 — Read cancellation and timed-out hardware commands lack a terminal contract**: Cancel expired queued commands atomically before device setters, with correlated queued/applying/applied/rejected/cancelled receipts. Report an already-started operation as pending with its eventual outcome rather than cancelled. Use shared RTL read_async pumps for daemon and local GUI, bounded 200 ms callback reads, repeated startup-safe cancel_async and join-before-close ownership. Pause/join before live RTL setters, discard old queued geometry, retain acquisition clocks/drop indices, and bound TCP live reads/writes while preserving fragmented pairs.
  Evidence: ingest::tests::queued_deadline_cancellation_prevents_late_tuning_after_a_stalled_read; ingest::tests::applying_deadline_returns_correlated_pending_outcome_and_retains_ownership; ingest::tests::dropping_queued_command_on_shutdown_publishes_cancelled_receipt; hardware::stream_worker::tests::repeated_cancel_survives_startup_race_and_joins_before_device_release; hardware::rtl_async::tests::silent_stream_read_yields_and_stop_joins_the_quiet_worker; hardware::rtl_async::tests::asynchronous_failure_preserves_queued_prefix_then_reports_the_driver_error; hardware::rtl_async::tests::native_callback_copies_usb_bytes_before_buffer_reuse; hardware::tcp_iq::tests::stalled_tcp_peer_yields_and_fragmented_iq_keeps_the_first_component; source_manager::tests::native_callback_acquisition_clock_is_preserved_before_preview_loss_and_stop.

- **S04 — Permanent ambiguity lane selection**: Advance every lane through fades and mapping changes, merge candidates by recovered-bit position, reject missing-energy candidates, suppress equivalent observations, and expire current frame lock after two missing CADUs. Plain/NRZ-M regressions compare recovered payload bytes and chronology across partitions. The original full-IQ 180° slip now recovers all six frames.

- **S08 — Received pass bounds and cross-channel alignment**: Store sparse segments on a shared sequence/CDS clock. Crop to common received bounds, preserve holes, handle long gaps and reorderings, and enforce the cap on relative extent. Six permanent off-air fixtures and the complete independent public pass verify alignment and pixels.
- **S10 — Current lock and honest decoded counts**: Count unique segments and complete scanlines separately from image extent. Publish at quarter-second symbol cadence and on lock transitions. Earlier carrier-energy and frame-expiry repairs remain covered by their regressions.

- **S02 — Absolute-phase-dependent OQPSK recovery**: Correct matched complex samples before projecting staggered branches; scale the second-order loop to its sample clock, retain both deskew directions and isolate their erasure histories.
  Evidence: all 60 independent pulse-shaped carrier cases recover byte-identical payloads at both tested chunk sizes; permanent frequency/clock-offset fixtures pass. A 20-second public 72k off-air slice yields 175 matching reference CADUs. See repair-carrier-verification.json.

- **R03 — Daemon narrow-channel decimation aliases strong adjacent signals**: Use bounded half-band stages and an output-phase FIR designed from actual passband/transition/stopband requirements. Production frequency and strong-adjacent mixture sweeps exceed the 80 dB rejection gate. Prime-factor, exact source-clock, chunk and reset regressions pass.
  Evidence: repair-channelizer-verification.json and channelizer evidence logs.

- **R04 — Daemon playback downsamples mono PCM without an anti-alias filter**: Filter PCM with a streaming fractional-delay FIR before converting to the exact playback clock. Real WFM MPX, AM/NFM aliases, rational capture rates and arbitrary network splits pass known-rejection and phase/count checks.
  Evidence: repair-playback-verification.json and playback evidence logs.

## Remaining findings

- [ ] **S01 · P1** — Advertised interleaved 80 ksym/s Meteor transport is missing

## Verification and limits

Normal workspace tests pass with hardware/audio disabled (`cargo test --workspace --no-default-features -- --test-threads=4`). The additional native float RF/audio regression passes. Default workspace build/all-target checks, Clippy with warnings denied, formatting, and all daemon hardware feature checks pass; the hardware check reports that libhackrf and SoapySDR are unavailable on this host. No receiver was opened.

The original long Viterbi counterexample now reports zero errors in the 1,920 aligned clean bits after 8,192,000 noisy trellis steps. The chronological-marker probe extracts both frames. Split primary headers complete; the old cross-VCID probe produces AAAA instead of BBAA. The independent 896-pixel libjpeg fixture has maximum error 1 and mean error 0.021205; the exact DC vector produces pixel 160.

The historical reproduce.py verifies the baseline counterexamples, so it intentionally no longer passes unchanged on repaired code. Active regressions and repair-verification.json are the evidence for this batch. R06/R07 were pending in that batch; their acceptance probes are now active under the carrier-power repair below.

Graphify was refreshed with the AST-only update command. It skipped HTML because the graph exceeds 5,000 nodes; code navigation remains available. Pre-existing user deletions and the untracked AGENTS.md are excluded from repair commits. The formatter normalized three previously unformatted areas and the audit example now uses the equivalent Clippy-approved parity expression.

JPEG integer quantization was checked against [the pinned SatDump mission implementation](https://github.com/SatDump/SatDump/blob/1aedd602055c154401384a8cc7876713bb371254/plugins/meteor_support/meteor/instruments/msumr/lrpt/huffman.cpp). Packet-header and idle-frame handling follows [CCSDS AOS 732.0-B-4](https://ccsds.org/Pubs/732x0b4.pdf).

Next priority: S01/S11 mission transport profiles, then remaining radio selectivity/clocks and monitoring repairs.

## Receiver-control batch verification

The full workspace suite passes again. RTL-feature daemon tests pass without opening a receiver; all four rtl_tcp loopback tests pass, including the previously opt-in wire checks. Default builds and workspace Clippy pass. Clippy also passes with every daemon hardware feature enabled. Hardware fault injection covers mutation-before-error, failed restoration, gain mode/value errors, Soapy timeout/overflow/fatal codes and timeout-only shutdown. Actual HackRF/Soapy libraries and physical error/overload runs remain unavailable; an independent rtl_tcp hardware endpoint was not tested.

The binary protocol version is now 10 because HardwareStatus carries requested gain and mode in addition to selected gain. Rebuild daemon and GUI together; mismatched binary clients are rejected during the existing handshake. JSON clients may omit the new optional fields. The GUI accepts a supported rounded step without retaining a pending exact-value request. rtl_tcp status describes commands written to the peer, not independently confirmed tuner readback.

The updated mutation-before-error audit example asserts that rejected/restoration-failed tuning publishes zero unverified blocks and retains the original frequency. The baseline logs remain unchanged. Evidence and source hashes for this batch are in repair-controls-verification.json.

Protocol commands were checked against [upstream rtl_tcp command handling](https://github.com/osmocom/rtl-sdr/blob/master/src/rtl_tcp.c); stream error codes were checked against [SoapySDR Errors.h](https://github.com/pothosware/SoapySDR/blob/master/include/SoapySDR/Errors.h).

## Callback acquisition batch verification

All 207 active daemon library tests pass (two existing socket tests remain opt-in). The callback bridge and real ingestion/channelizer/recording integration test pass. Both recording formats stop before the retained post-overflow transfer, preserve exact accepted bytes, and mark the file incomplete. The post-gap channel output matches a freshly initialized DDC at the acquisition index. Arbitrary odd callback partitions preserve IQ pair alignment and signed conversion. Queue loss is counted before publication, and readers never concatenate across a missing transfer.

Default workspace build, formatting and all-feature daemon Clippy pass. Native HackRF callback safety tests compile under all features, but cannot be linked/run on this host without libhackrf. No physical USB overload or worst-case driver shutdown test was performed. The 200 ms callback receive deadline addresses the silent-stream read loop; RTL sync reads and late control-command cancellation still leave R22 open. Evidence is in repair-callback-verification.json.

## Cancellation batch verification

The full workspace suite passes, as do all 222 active RTL-feature daemon library tests, five rtl_tcp socket tests, the local capture-clock bridge regression, default build, workspace/all-feature Clippy, and formatting. Fault injection proves that queued expiry never enters a later setter, an already-applying call retains ownership until terminal completion, startup cancellation is retried after an initial ignored request, silent reads yield, native buffer reuse cannot corrupt published IQ, and asynchronous failure preserves only the accepted prefix. Source/command shutdown discards unconsumed cancelled requests and late reads.

RTL uses librtlsdr's asynchronous cancellation API instead of its synchronous zero-timeout bulk read. The caller joins before closing or retuning; no worker is detached and no device is released while a read still uses it. The inspected upstream event loop has a one-second event wait and drains cancelled transfers; physical USB/driver worst-case cancellation latency and alternate driver forks remain unverified. The 200 ms bound is the IQ consumer wait, not a claimed physical USB shutdown benchmark. See [upstream async read and cancellation](https://github.com/osmocom/rtl-sdr/blob/master/src/librtlsdr.c) and repair-cancellation-verification.json.

The prior callback-batch note about open RTL/command cancellation is historical; R22 now has the software repair above. Other field/release evidence remains unverified, and the full goal is active.

## Lane reacquisition and lock verification

All 90 LRPT tests pass. Mapping-change tests include inversion of both branches, conjugation and axis exchange in plain and NRZ-M streams; every accepted payload matches its transmitted bytes, remains in chronological order and matches across input partitions. Repeated identical transmitted frames count separately; equivalent observations from different lanes count once. Zero-energy symbol intervals cannot renew frame lock even if hard decisions form algebraically valid RS payloads. Current frame lock expires after 16,384 recovered-symbol observations without a valid frame; completed images and historical decode success remain available. All hypotheses remain warm, increasing decoding CPU relative to the prior permanently selected single lane.

Carrier lock now requires normalized input power above 1e-12, matched-symbol power above 0.01 after AGC, average fourth-power error below 0.2 and 64 consecutive qualifying symbols. Silence withdraws lock on the next symbol observation. The deterministic white-noise and silence controls pass. These checks measure observed loop coherence; they do not claim a calibrated SNR estimate or fix S02's absolute-phase recovery. At this checkpoint S10 remained open because allocated image extent still overstated decoded scanlines; the following image-placement batch repairs that remaining portion.

The unchanged synthetic RF audit probe reports 24 RS codewords after a 180° slip, matching its six-frame clean control, and zero-input Costas lock is false. Other phase/fractional-rate probes still demonstrate S02. The public capture is independent evidence for upcoming comparison, not a passing mission-compatibility result.

Full workspace tests, default build, workspace Clippy with warnings denied, formatting and AST graph update pass for this batch. Command logs and source hashes are in repair-lane-verification.json. The graph contains 7,189 nodes and 15,965 edges; the existing user-deleted graph sidecars remain deleted.

## Image placement and progress verification

The public corrected CADUs contain 7,338 frames, 28,473 image packets and 678 telemetry packets. The repaired packet/image path accepts every image segment and reconstructs three channels at 1,568 × 5,504 on common CDS-time/sequence pass bounds. It reports 16,192 complete scanlines separately from partial segments and output extent. A single late segment now produces an eight-row image, one segment and zero complete full-width scanlines. Missing groups stay black; duplicates do not inflate counts.

An independent Python CCSDS parser and Pillow/libjpeg entropy decoder reconstruct the entire pass using CDS timestamps rather than application sequence placement. All 25,890,816 pixels match with maximum error 1; channel mean errors are 0.01780–0.01802 gray levels. The source spans JPEG quality 36–100, covering both mission quantization branches. Exact mission integer DQT values are supplied to libjpeg because its quality convenience API truncates the scale before rounding table entries. This also strengthens S09 beyond the earlier q50 vector.

The installed SatDump v1.2.2 independently regenerates images from the same corrected CADUs. Its per-channel crops omit the final eight-row groups and differ at a small number of partial/missing segments; it is supporting evidence, while the full CDS/libjpeg oracle checks every retained pixel and mask. The older image in the public share has an 8,552-row extent caused by its known initial-counter alignment issue, so that extent is not used as the repair target. The received packet timestamps and independent full-pass oracle prove the corrected bounds.

Permanent off-air fixtures verify first/last groups across three channels, multiple counter wraps, reverse arrival order and non-flat pixels. Additional tests cover missing groups, late starts, partial/full-line counts, duplicates, daily timestamp rollover, reordered old segments, long gaps and cap rejection without clock poisoning. The nominal 1.231142-second group interval is corroborated by this independent pass and refined by adjacent observed timestamps; absent timestamps use nearest modular sequence observations, whose epoch is inherently ambiguous after a gap longer than half a sequence cycle.

Progress uses a quarter-second recovered-symbol cadence, independent of image size. Carrier/frame lock transitions bypass throttling and are emitted at the next caller chunk boundary when queue space is available. The queue remains nonblocking and bounded to two retained snapshots. The GUI labels complete lines and segments explicitly. RF demodulation/mission transport compatibility remains open under S01/S02/S11; this batch validates the packet and product layers using independently corrected off-air CADUs.

The full workspace suite passes with all 97 LRPT tests and the existing GUI/daemon/protocol checks. Default build, workspace Clippy with warnings denied, formatting and AST graph update pass. Evidence is recorded in repair-image-verification.json; original audit baseline evidence remains unchanged.

## Carrier recovery verification

All 60 independently generated carrier cases pass at chunk sizes 997 and 16,384. The phase sweep spans 0–345 degrees in 15-degree steps, both half-symbol stagger directions and pulse-shaped 80k symbols at 640 kS/s. Additional cases cover 2.048 MS/s (25.6 samples/symbol), 288 kS/s with 72k symbols, ±1 kHz carrier offset and ±150 ppm clock drift. Every accepted corrected 892-byte payload equals its transmitted reference, with exactly three CADUs and twelve successful RS codewords per case.

The generator imports no application encoder, filter or waveform code. It builds RS parity with pinned quiet/libfec, constructs NRZ-M/convolutional bits independently and evaluates analytical transmit RRC pulses at fractional sample times. Two complete waveform fixtures and golden payloads are committed, so default tests retain both stagger directions with frequency and clock offsets without requiring an external library. The larger corpus can be regenerated with the documented reference build and command; its hashes and passing log are retained.

Carrier correction now precedes I/Q branch projection and half-symbol pairing. The loop runs on matched complex samples with second-order gains scaled to that sample clock. Both deskew orientations feed continuously advanced sign/swap hypotheses. Missing-energy observations belong to their own pairing, so an erased orientation cannot either renew lock or reject a valid frame from the other orientation. All sixteen hypotheses remain warm; this increases decoding CPU compared with the previous eight-lane implementation.

The public off-air recording supplies an additional independent carrier check: the 180–200-second slice at 288 kS/s/72k yields 175 accepted corrected CADUs and 700 valid RS codewords, with every payload matching the producer's independently decoded reference and no additional frames. This is a slice comparison, not complete-pass RF certification. The earlier full-pass corrected-CADU image oracle remains valid. The public ZIQ has an unfinished compressed tail, and no independently decoded interleaved 80k recording has yet been validated.

The unchanged audit probes now recover twelve RS codewords at every originally tested phase at both sampling rates, including 45, 60 and 90 degrees. The 180-degree slip recovers the same twenty-four codewords as the clean control; zero-input carrier lock is false. Active LRPT regressions also cover noise, silence, fades, lane reacquisition, byte equality and chunk invariance. Software verification records commands, source hashes and limits in repair-carrier-verification.json. S01/S11 and all original field/release gates remain in the active goal.

The recovery-stage ordering was checked against [the pinned independent PSK demodulator](https://github.com/SatDump/SatDump/blob/1aedd602055c154401384a8cc7876713bb371254/src-core/pipeline/modules/demod/module_psk_demod.cpp). The independent RS fixture library is [quiet/libfec at the pinned revision](https://github.com/quiet/libfec/tree/9750ca0a6d0a786b506e44692776b541f90daa91). No reference implementation code is added to the production decoder.

Final carrier-batch checks pass: the full workspace suite, all 99 active LRPT tests, default build, workspace Clippy with warnings denied, formatting and AST graph refresh. The graph has 7,231 nodes and 16,074 edges. An overlapping test attempt timed out in the unchanged scanner observation test; its isolated recheck and the clean full-suite retry pass. The failed attempt and successful retry are both retained in the carrier evidence, with no scanner timeout or source changes.

## Meteor mission transport and clock verification

The offline decoder now selects explicit 72k NRZ-M or marker-bearing interleaved 80k NRZ-M transport. The new streaming stage acquires three 0x27 markers at 80-bit spacing, removes markers, and applies 36 branch delays before Viterbi. Missing input and unknown startup history follow the same permutation; phase loss invalidates partial transport. Marker and coded-bit results match an external native C decoder across three chunk sizes. Independent analytical 80k IQ yields exactly three expected CADUs in both stagger directions and both chunk sizes. Production code contains our own streaming implementation.

A bounded Gardner period loop tracks continuous clock error through the long interleaver startup. All 60 independent carrier cases still produce the expected bytes. The explicit 72k mission profile recovers 175 CADUs from the public 180–200 second slice, all matching the original producer reference. A separately documented HTTP range prefix covers that interval; it is not a complete RF-pass verification.

Mission entry points require four input samples per symbol: 288 kS/s for 72k or 320 kS/s for 80k. Additional independent lower-rate probes lose frames in some cases with frequency/clock offsets, so their negative result is retained and those rates are rejected. Offline sampling validation runs before starting a worker. A received-image regression verifies that stream gaps preserve the product and a new product retires images and counters. The daemon is not yet wired to these APIs.

S01 remains open until independent off-air 80k CADUs and images are compared. S11 also remains open. Evidence, source hashes, public prefix provenance and lower-rate probe limits are in repair-transport-verification.json. Original audit findings and baseline evidence are unchanged.

The full workspace rerun exposed a scheduling-sensitive native-radio snapshot failure: a skipped new-generation block retained the prior epoch. Clearing epoch and radio/DSP snapshot state at the processed generation boundary fixes it. A deterministic processor regression covers the interleaving, and the failing run is preserved alongside subsequent verification.

Final workspace verification passes after the snapshot fix: 832 GUI tests and 107 LRPT tests pass, along with the daemon/protocol/integration suites. Default workspace build, all-target Clippy with warnings denied, formatting and the AST graph refresh pass. The three external LRPT comparisons were also run explicitly and passed; their ignored status in routine tests records their external inputs.

## Daemon Meteor profile and product verification

S11 is software-verified. LRPT ChannelSpec requests require an explicit Oqpsk72 or Oqpsk80Interleaved profile; the selected profile controls the entire mission transport. Before allocating a channel/tap/thread, the daemon validates the occupied band and computes the actual integer-decimated output rate against the four-samples-per-symbol receiver requirement. Invalid retunes leave the live spec unchanged, and resubscription cannot silently select another transport.

A stable source-product session is carried separately from the capture epoch. The independent 72k nonflat-image IQ fixture produces three received segments and twelve valid RS codewords. Deep pixel/counter comparisons verify preservation across a gap/new epoch and retirement on a source change, frequency retune or explicit same-frequency product start. Automatic source changes retain queued new-session IQ; explicit product boundaries drain the old queue. Injected driver overflow preserves source-product identity; actual replay loops and frequency control changes retire it.

Every telemetry preview carries product_id. A zero-size APID-zero update announces the boundary; clients also detect a new ID on any subsequent image if the notice was coalesced. The GUI clears all retired APID images and ignores old queued IDs; Welcome retires IDs from a previous daemon process. The binary protocol is version 11, so rebuild daemon and GUI together. The raw telemetry WebSocket header adds a little-endian product ID at offset 28 and starts pixels at offset 36. README.md documents the profile fields and StartTelemetryProduct command.

Final workspace verification passes: 219 daemon, 833 GUI, 20 protocol and 107 LRPT tests, plus integration/remaining workspace suites. Default build, all-target Clippy, formatting, all daemon hardware-feature compilation and AST graph refresh pass. No receiver was opened; missing HackRF/Soapy libraries and original field/audio/disk gates remain unverified. The initial rerun's error-text compatibility failure and the passing final run are retained in repair-mission-verification.json and its evidence logs.

There are now 21 software-verified findings and 12 open findings. S01 retains its independent off-air 80k CADU/image gate. Next priority: R03 channelizer anti-alias filtering and the remaining radio clocks/selectivity/monitoring findings.

## Channelizer anti-alias verification

R03 is software-verified. The decimator now preserves a passband through 0.4 of the exact output sample rate, uses 0.4–0.5 as its transition band and rejects from output Nyquist onward. Half-band stages reduce the working rate before an output-phase Blackman-Harris FIR. Integer source-grid cadence is preserved even for prime factors; fractional coefficient selection keeps physical group delay constant. Decimation one remains exact full-band passthrough. Final coefficient memory stays below 1 MiB even at the largest u32 decimation.

The real SampleBus/DDC sweep covers 622 signed-frequency measurements at source rates 2 MS/s, 2,048,003 S/s and 2.4 MS/s, with requested output rates 12.5k, 25k, 200k, 288k and 320k. Maximum passband error is 0.000041 dB and minimum stopband rejection is 111.150040 dB, against limits of 0.02 dB and 80 dB. Fifteen mixtures each contain four interferers 27.604 dB above the wanted carrier, with a bounded peak below clipping. Maximum wanted-normalized mixture error is 0.000004545. Different chunk sizes are used for reference and mixture streams.

Permanent regressions add Nyquist/folding-tone and mixture checks at D=4093, exercising interpolated phase banks; independent analytic tone phase is compared against the exact source-clock/filter delay. Counts, partitions, off-grid gap resets, capture strides and bounded memory are checked separately. The original coarse probe is retained: its f32 phase generator and 50-output exclusion include generated in-band phase noise and new-filter startup. A separately labelled f64/fully settled probe reports the ideal carrier response. Neither the historical audit evidence nor those coarse results were replaced to obtain a rejection claim.

Final full-workspace tests pass: 223 daemon, 833 GUI, 20 protocol and 107 LRPT tests, plus integration and remaining workspace suites. All 37 focused release channelizer/state tests pass. Default build, all-target Clippy with warnings denied, formatting, hardware-feature compilation and AST graph refresh pass. Command logs and source hashes are in repair-channelizer-verification.json. The graph has 7,318 nodes and 16,333 edges; HTML export was skipped at its configured size limit. No receiver was opened. Physical hardware/audio/disk and independent 80k off-air gates remain unverified.

There are now 22 software-verified findings and 11 open findings. Next priority: R04 playback anti-alias resampling, followed by the remaining radio gain/selectivity/timebase/monitoring findings.

## Playback anti-alias and clock verification

R04 is software-verified. Playback now uses a streaming Blackman-Harris fractional-delay FIR before rational output sampling. The passband reaches 0.4 of the lower input/output rate, transitions to 0.5 and requires at least 80 dB rejection there and above. Exact integer phase uses the capture root rate and stride rather than its rounded display rate. Network splits preserve output samples; missing frames, changed clocks/rates/channels and corrupt geometry retire history. Gain is applied after filtering, including when muted. Equal-rate playback remains sanitized passthrough. The causal FIR contributes a known delay of about 1 ms at the usual 200k-to-48k geometry; startup samples and exact stream duration are retained.

There are 122 independent PCM frequency measurements across seven source rates and both 44.1/48 kHz destinations, with minimum rejection 112.350624 dB. Analytic wanted-tone phase and amplitude checks verify constant filter delay. Actual SampleBus/daemon WFM production feeds 32 stereo/RDS-range MPX cases at four input rates into playback, with minimum rejection 115.348143 dB. Eight mixtures retain wanted audio with relative error at most 0.000000738; whole and split frames produce identical output. Real AM/NFM 49k audio cannot fold into 4.9/1k output. Additional checks cover 204800.3 S/s exact capture time, single-sample/empty partitions, 48:1 and 1:48 ratios, volume, nonfinite samples and history retirement.

The original unchanged radio probe now reports the 49k-to-1k alias near 0.000003 rather than 0.410990; that aggregate retains FIR startup. Its real WFM 57k-to-9k case rounds to zero at six decimal places. Remaining FM gain, RAW, local AM and SSB probes still demonstrate their separate open findings. The first gain/pitch test assumptions and first AM measurement included startup behavior; the failed results are retained alongside final settled checks. AM's own DC-blocker is allowed 100 ms to settle before its alias projection. The converter's frequency-response checks exclude only finite FIR startup, and the 80 dB threshold is unchanged.

All twelve focused playback tests and the full workspace pass: 841 GUI, 223 daemon, 20 protocol and 107 LRPT tests, plus integration and remaining workspace suites. Default build, all-target Clippy with warnings denied, formatting and AST graph refresh pass. Source hashes, exact commands and preserved intermediate results are in repair-playback-verification.json. The graph has 7,339 nodes and 16,393 edges; HTML export was skipped at its configured size limit. No physical receiver/audio device was opened. Original field/audio/disk and independent off-air release gates remain unverified.

There are now 23 software-verified findings and 10 open findings. Next priority: R05 physical FM gain normalization across source/IF clocks and mono/stereo paths.

## Physical FM gain and deviation verification

Local NFM and WFM now use the same physical amplitude conventions as daemon producers: unit amplitude represents 5 kHz or 75 kHz deviation before audio controls. Normalization uses the actual RF-decimated or IF-resampled clock. The shared frequency meter excludes initial absolute phase and carrier offset, refines between-sample peaks, publishes completed 100 ms spans and retires its history at capture/clock resets. Stereo and mono report exactly the same observed deviation.

The independent analytical 1 kHz sweep covers 216 cases: three source rates (including fractional RF clocks), RF factors 1/2/4, all IF presets, NFM and WFM mono/stereo fallback, and 44.1/48 kHz playback. Maximum gain error is 0.046177 dB and maximum relative deviation error is below 0.84%; every duration and pitch assertion passes. An additional NFM regression covers RF factor eight. AGC and intentional audio filtering/deemphasis are disabled for calibration, with their existing behavior checked separately.

A continuous stereo fixture analytically integrates a seven-component multiplex with independently specified L/R amplitudes and 13.5 kHz peak deviation. This avoids the sampling-dependent gain of the older discrete phase-integrator fixture. Twelve source/RF/IF/playback combinations recover both channel amplitudes within 0.104252 dB and deviation within 2.10%. Twenty-four downmix comparisons differ from mono by at most 0.007001 dB. Final gates remain 0.15 dB for gain and 3% for deviation.

The failed development checks exposed real attenuation from single-bin IF reconstruction, sample-interval averaging, RF IIR phase distortion and linear IF interpolation. The repair preserves clean FM through envelope-informed reconstruction, uses a bounded symmetric frequency equalizer, shares the verified linear-phase FIR RF decimator and applies causal four-point IQ interpolation. The IF channel filter is applied once. The original strongest-bin reference and noisy-NFM SNR-improvement regression still pass. Failed logs remain alongside the final evidence.

All 75 demodulator tests and the full workspace suite pass, including 845 GUI, 223 daemon, 20 protocol and 107 LRPT tests. Default workspace build, all-target Clippy with warnings denied and formatting pass. The AST graph refresh passes with 7,359 nodes and 16,472 edges; the eight user-deleted tracked graph sidecars remain deleted. Commands, logs, measured cases and source hashes are recorded in repair-fm-verification.json. No physical receiver/audio device was opened, and all original field and off-air release gates remain unverified where independent evidence is still missing. The goal remains active.

## Carrier-power repair verification (R06/R07)

All focused DSP and native/scanner-consumer regressions pass, as do the default workspace build, all-target Clippy with warnings denied and formatting. The AST refresh passes with 7,385 nodes and 16,553 edges; the eight user-deleted graph sidecars remain deleted. The full workspace test attempt was stopped at the user request to stop pegging CPU (SIGTERM, exit 143); it is incomplete and is not counted as passing. Command logs, failed development compilations, source hashes, acquisition limits and every measured sweep are retained in repair-power-verification.json. No additional CPU-heavy checks were started after that request. Original hardware/off-air/audio/disk gates remain unchanged, and the goal remains active.

## Fixed-capture scan repair verification (R20)

All five focused regressions and all 80 affected-module tests pass; GUI all-target Clippy with warnings denied, GUI default build, formatting and AST refresh pass. Verification was serialized under a verified 20%-of-one-CPU cgroup quota, nice 19, one build job and one test thread. The full workspace suite remains off after the user CPU request. The graph has 7,399 nodes and 16,609 edges; user-deleted graph sidecars remain deleted. The initial test-fixture failure and corrected final results, commands, resource policy and source hashes are retained in repair-scan-verification.json. All 33 original repair gates and all original off-air/hardware/audio/disk release requirements remain intact. The goal remains active.

## Automatic recording and monitor verification (R13)

R13 is software-verified. Explicit Power/enabled state covers the complete -100..0 dBFS range. Native observations advance only on complete RF windows and expire with capture receipt age. Daemon monitoring requires acknowledged capture settings and power threshold, then fresh revision-tagged raw measurements; gain/VFO changes retire history. Frozen displays and hidden Listen tabs keep ingesting observations. Scanner startup must establish its own token, and active scanner/monitor consumers share that boundary.

Local counts advance after successful file open. Daemon starts remain pending until an owned-channel active acknowledgement in the current session. Failures, timeouts, cancelled starts, duplicate statuses and closed queues do not count or flood commands. Recording progress coalesces within a phase while start/stop acknowledgements stay in order.

All ten focused regressions pass, including 202 threshold/source cases, plus all 62 unique affected tests across recorder, native RF, spectrum freshness, receipt age and daemon squelch modules. GUI default build, all-target Clippy with warnings denied, formatting and AST graph refresh pass. Everything ran serially with a verified 20%-of-one-CPU cgroup quota, nice 19, one build job and one test thread. The full workspace suite and expensive DSP sweeps remain off. Failed development checks and final results, exact commands, resource policy and source hashes are retained in repair-trigger-verification.json.

There are now 28 software-verified findings and five open findings: S01, R09, R10, R12 and R19. Original repair gates and off-air/hardware/audio/disk release requirements remain unchanged; this batch uses controlled daemon protocol fixtures rather than field recordings. The AST graph has 7,425 nodes and 16,706 edges; HTML export was skipped at the configured size limit. User-deleted graph sidecars remain deleted. The goal remains active.

## RAW playback and WAV verification (R10)

R10 is software-verified. Local RAW means stereo I/Q audio, I on the left and Q on the right, filtered before conversion to the selected audio clock. It uses receiver IQ/RF correction and decimation, volume and audio gain with 0.3 headroom, bypassing the mono voice chain. Full-band IQ export retains the original RF clock. Equal input/output clocks preserve scaled I/Q exactly without new delay. Bounded integer phase follows the exact f64 clock, and a separate root-capture budget includes partial RF strides and tiny blocks. Native playback, WAV headers and metadata carry two channels; scalar samples and per-channel frames are distinct.

All eight focused regressions and all 102 unique affected tests pass. The clock sweep covers 45 rate/partition pairs and 27 RF-stride cases across five tiny partitions. Six independent I/Q checks have maximum gain error 0.00000613 dB and minimum alias rejection 142.6997 dB, against unchanged 0.1 dB/80 dB gates. Three native playback/WAV cases produce exactly one second with 48k, 44.1k and 384k audio, including fractional RF clocks and 64x RF decimation. Native playback samples and WAV samples match after int16 quantization. The existing RF blanker impulse regression passed unchanged after adding equal-clock passthrough.

GUI default build, all-target Clippy with warnings denied, formatting and AST graph refresh pass. Verification was serialized with a confirmed 20%-of-one-CPU quota, nice 19, one build job and one test thread. The full workspace suite and unrelated expensive DSP sweeps remain off. Failed development outcomes and final command logs, resource policy and source hashes are retained in repair-raw-audio-verification.json. User deletions remain untouched.

There are now 29 software-verified findings and four open findings: S01, R09, R12 and R19. All original repair and off-air/hardware/audio/disk release gates remain intact. No physical receiver or audio device was opened; the goal remains active.

## Detected audio anti-alias verification (R09)

R09 is software-verified. AM envelope, SSB real output and DSB/CW product detection retain the full IQ clock until a streaming FIR filters and converts the detected audio to the selected device clock. CW beat mixing runs before conversion, so beat components above output Nyquist are removed. DC removal, CW gating, audio filtering, AGC and effects run at the device rate. Envelopes retain full-scale carrier headroom until the amplitude controls. Native conversion follows the root capture duration through fractional RF clocks and incomplete strides, including float-only blocks with no byte preview.

All six focused and all 64 unique affected tests pass. The mode sweep covers 12 RF-clock cases with 8k/44.1k/48k output. Nonlinear rectification tests also use 16k output. The largest measured 6k-to-2k alias is 0.0000032077 (95.90 dB rejection relative to the 0.2 input component), below the 0.00002/80 dB limit; wanted-mixture gain stays within 0.1 dB. CW beat aliases, carrier headroom, physical notch frequency, tiny chunks, resets and three native float-only partial-stride cases pass. All 16 native consumer tests pass; the continuous GUI-independent consumer processes 1,966,080 RF samples and 39,322 audio frames with zero reported loss.

The unchanged constant-carrier DC regression exposed a pre-existing constructor clock inconsistency: 2.048 MHz source/input with 1:1 decimation and a 48k audio clock. The constructor now starts consistently at 48 kHz; capture workers apply their actual source rate before DSP. The original DC window and amplitude gate remain unchanged. Explicit constructor invariants, default FM/WFM and worker mode/reset regressions pass.

GUI default build, all-target Clippy with warnings denied, formatting and AST graph refresh pass. The failed development outcome, final command logs, source hashes and resource policy are retained in repair-detected-antialias-verification.json. All checks ran serially under 10% of one CPU, nice 19, one Cargo job and one test thread. User deletions remain untouched. The ledger now has 30 software-verified findings and three open findings: S01, R12 and R19. Original off-air/hardware/audio/disk release requirements remain unverified; the goal remains active.

## Daemon SSB response — focused verification, 2026-10-10

R12 has an implemented repair and three passing focused tests; it remains open pending affected regression, default-feature build and Clippy verification. A 36th-order complex Butterworth filter defines the response in Hz at the exact channel clock. Translating the selected sideband back before real projection preserves voice pitch. This also removes the old 1,650 Hz voice-frequency shift.

The real AudioPipeline sweep checks USB and LSB at six clocks, including 200 kS/s and fractional 204,800.3 S/s. All 336 voice points from 300 through 3,000 Hz pass: maximum wanted gain error 0.02520669 dB, minimum opposite-sideband rejection 92.681208 dB, and maximum gain error with an opposite signal 12.5 times stronger (21.94 dB) of 0.00251525 dB. Exact chunk equality, epoch/discontinuity/gap/mode resets, filter-clock changes and nonfinite input checks also pass.

The completed focused run used 10% of one CPU core, nice 19, one Cargo job and one test thread. No further build or test job was started after the user requested relief from CPU load. The log, exact command and source hashes are saved in repair-ssb-selectivity-verification.json. The ledger remains at 30 software-verified findings and three open findings; R19 and all original field/off-air/hardware/audio/disk gates remain unchanged.

## Daemon SSB response — completed affected verification, 2026-10-10

R12 is software-verified. All 18 existing audio regressions pass alongside the three focused tests described above (21 unique tests total). Default daemon build, all-target Clippy with warnings denied and formatting pass. The AST graph refresh completed with 7,474 nodes and 16,863 edges; user-deleted graph sidecars remain deleted. No full-workspace suite was run.

The affected checks used an enforced 10% quota of one CPU core, nice 19, one Cargo job and one test thread. Exact commands, retained logs, source hashes and the guarded runner are saved in repair-ssb-selectivity-verification.json. The ledger now contains 31 software-verified findings and two open findings: S01 and R19. Original repair gates and physical/off-air/hardware/audio/disk release requirements remain unchanged; the goal remains active.

## Daemon audio shared DSP foundation — R19 still open, 2026-10-10

The existing GUI audio-rate FIR and I/Q/detected-audio converter now live in a shared daemon module, with thin GUI re-exports. After normalizing visibility/type names and the added read-only storage accessor, the existing coefficient design, arithmetic, state, gain, finite-input handling and duration-accounting source matches commit 344a543 exactly. Three existing test-only delay checks now read the same tap count through the accessor. Their gates and expected delays are unchanged; the initial compiler failure is retained.

A reusable linear-phase RF selector defines its protected passband at +/- half the requested RF width and its stopband from +/- 0.625 times that width. Fixed overlap-save FFT windows replace a long direct convolution per sample. Allocation is bounded to 32,769 FIR taps; unsupported geometry returns an error before allocating. Across six clocks and three RF widths, all 108 passband/stopband cases pass, including neighbors 25 times stronger (27.96 dB). Maximum wanted gain error is 0.00000454 dB; minimum measured stopband tone rejection is 123.199381 dB; maximum mixed wanted-tone gain error is 0.00000278 dB. Chunk equality, resets, nonfinite inputs and invalid/storage-limit requests also pass.

All 25 affected tests pass: two RF-selector tests, five RAW conversion tests, twelve daemon-playback conversion tests and six detected-audio tests. Daemon and GUI all-target Clippy with warnings denied, default GUI build, formatting and AST graph refresh pass. The checks ran serially under an enforced 10% one-core quota, nice 19, one Cargo job and one test thread. No full-workspace suite was run. Logs, exact commands, source hashes and the initial failed compile are in repair-daemon-audio-foundation-verification.json.

R19 remains open: AudioPipeline has not yet adopted the new RF selector, and its protocol still needs distinct RF bandwidth, channel sample rate and audio output rate. Selected-RF squelch and end-to-end AM/FM mixture checks remain required. The ledger stays at 31 software-verified findings and two open findings (S01/R19); all original repair and field/off-air/hardware/audio/disk release gates remain intact.

## R19 integration checkpoint — consumer verification pending, 2026-10-10

The working tree now integrates mode-specific complex RF selection before AM/FM/WFM detection and measures squelch on the selected signal. Protocol version 12 separates channel sample rate, RF width and PCM output rate; audio frames retain the RF capture clock independently of their PCM clock. Mode, RF width, PCM rate and channel geometry can change in one validated retune. The desktop requests 48 kS/s mono PCM and uses the generated clock for playback.

Thirty focused daemon/protocol tests pass: 23 audio pipeline tests, two exact rational-clock conversion tests, two configuration tests and three protocol tests. Twelve AM/FM/WFM mixtures across integer and fractional production clocks reject carriers 25 times stronger than the wanted carrier while preserving wanted audio and selected-RF squelch behavior. Maximum relative PCM error is 0.000003865. The existing 336 SSB voice-band checks still pass with unchanged measured gain and rejection. PCM partition equality, known/unknown UTC origins, capture-gap resets, exact coherent sample counts, invalid geometry and atomic widening/narrowing are covered. Logs and command records are retained under repair-evidence/r19-*.

The initial configuration test incorrectly assumed the shared synthetic fixture supplied 2.0 MS/s; it actually supplies 2.048 MS/s. The corrected fixture initializes channel geometry from the applied hardware clock and checks precomputed actual rates. The failed log and command records are preserved. The initial workspace all-target compatibility check passed before the new tests were wired in; subsequent daemon/protocol test builds pass. GUI workflow/playback checks, current all-target Clippy, default builds, final formatting and graph refresh remain pending. R19 stays open and the ledger remains at 31 software-verified findings. All jobs run serially at 10% of one CPU core, nice 19, one Cargo job and one test thread; the original repair and field-release gates remain unchanged.

## R19 integration verified — 2026-10-10

All 47 unique affected daemon/protocol/GUI tests pass, including the 12 strong-neighbor mixtures, exact coherent PCM counts, atomic widening/narrowing and the unchanged 336 SSB voice checks. The existing 122-case playback sweep and 32 WFM MPX stop cases pass; WFM mixture error is 0.000025807 relative to wanted audio. The AM/NFM audio-alias fixture explicitly selects an RF passband containing its 49 kHz modulation, so RF rejection cannot conceal failure of the separate audio-rate stage.

Current daemon/GUI/protocol all-target Clippy with warnings denied, default daemon and GUI builds, formatting and AST graph refresh pass. Failed fixture/Clippy runs, source hashes, commands and guarded runners are retained in repair-daemon-selection-verification.json. Builds and tests remain serial within a combined 10% limit of one CPU core, nice 19, one Cargo job and one test thread. Independent reference downloads and prefix restoration share that limit at 1% while the build receives 9%. User deletions and the original 33 repair gates are preserved. The ledger now has 32 software-verified findings and S01 open; physical/off-air/audio/disk release requirements remain unverified. Public recording availability checks are recorded separately in public-meteor-candidates-1010.json. The goal remains active.

## Actual public 72k RF packet/image comparison — 2026-10-10

The 20-second slice and complete usable 522,059,776-byte IQ prefix pass packet-byte and image comparisons from the actual RF decoder. The full prefix recovers 6,899 corrected frames and compares 6,055,292 completed packet bytes and 25,025,280 image pixels against the independent original CADUs and Python/Pillow/CDS-time oracle. Maximum pixel error remains at most one. 439 reference frames are missing and are not counted as recovered. The compressed recording is truncated.

Clippy, optimized actual-RF tests, formatting and AST graph refresh pass. Source hashes, commands, full-frame coverage and comparison summaries are in repair-public-rf-verification-1010.json. The saved RF tests used the former 10% CPU allowance; resumed oracle checks and finalization used the full CPU access authorized on 2026-10-11. User deletions and all 33 original repair gates are preserved. The ledger remains at 32 software-verified findings with S01 open: interleaved 80k off-air comparison and original physical hardware/audio/disk release checks are still unverified.
