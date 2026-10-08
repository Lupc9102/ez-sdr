# Audit remediation — in progress

The original REPORT.md, findings.json and validation.json describe the audited baseline and remain historical evidence. This ledger tracks repairs against all 33 findings without changing their original repair gates.

20 findings have software repairs with passing regressions; 13 remain open. The goal is active. Off-air mission compatibility and physical hardware/audio/disk release evidence remain unverified. The user no longer has the earlier Meteor recordings. A public Meteor-M2-4 recording, reference CADUs and an image were downloaded; a 20-second 72k RF slice now matches 175 reference CADUs; complete-pass RF/80k mission compatibility remains pending; corrected-CADU image comparison passes, and the ZIQ stream has an unfinished compressed tail. See public-meteor-reference.json.

## Implemented and software verified

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

## Remaining findings

- [ ] **S01 · P1** — Advertised interleaved 80 ksym/s Meteor transport is missing
- [ ] **S11 · P2** — Daemon LRPT accepts unusable sampling and cannot select the 72k profile
- [ ] **R03 · P1** — Daemon narrow-channel decimation aliases strong adjacent signals
- [ ] **R04 · P2** — Daemon playback downsamples mono PCM without an anti-alias filter
- [ ] **R05 · P2** — Local FM loudness changes with RF sample rate and RF decimation
- [ ] **R06 · P2** — USB/LSB power squelch measures a symmetric band instead of the selected sideband
- [ ] **R07 · P2** — Narrow power measurements can return silence because no FFT bin fits
- [ ] **R09 · P2** — AM audio aliases when the selected RF band exceeds output audio Nyquist
- [ ] **R10 · P2** — RAW output has the wrong channel count and timebase for playback/WAV
- [ ] **R12 · P2** — Daemon SSB rejection degrades sharply at the actual GUI channel rate
- [ ] **R13 · P2** — Automatic recording/monitoring has an undocumented -90 dB cutoff and no daemon power feed
- [ ] **R19 · P2** — Daemon narrowband listening uses a wide fixed channel without mode-specific RF selection
- [ ] **R20 · P2** — Scanning can wait forever on a frequency a fixed replay capture cannot contain

## Verification and limits

Normal workspace tests pass with hardware/audio disabled (`cargo test --workspace --no-default-features -- --test-threads=4`). The additional native float RF/audio regression passes. Default workspace build/all-target checks, Clippy with warnings denied, formatting, and all daemon hardware feature checks pass; the hardware check reports that libhackrf and SoapySDR are unavailable on this host. No receiver was opened.

The original long Viterbi counterexample now reports zero errors in the 1,920 aligned clean bits after 8,192,000 noisy trellis steps. The chronological-marker probe extracts both frames. Split primary headers complete; the old cross-VCID probe produces AAAA instead of BBAA. The independent 896-pixel libjpeg fixture has maximum error 1 and mean error 0.021205; the exact DC vector produces pixel 160.

The historical reproduce.py verifies the baseline counterexamples, so it intentionally no longer passes unchanged on repaired code. Active regressions and repair-verification.json are the evidence for this batch. R06/R07 acceptance probes remain ignored pending their repairs.

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
