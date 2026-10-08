# Audit remediation — in progress

The original REPORT.md, findings.json and validation.json describe the audited baseline and remain historical evidence. This ledger tracks repairs against all 33 findings without changing their original repair gates.

10 findings have software repairs with passing regressions; 23 remain open. The goal is active. Off-air mission compatibility and physical hardware/audio/disk release evidence remain unverified. The user no longer has the earlier Meteor recordings; public reference captures still need investigation.

## Implemented and software verified

- **S03 — Viterbi metrics saturate and permanently kill the decoder**: Normalize finite path metrics and retain odd coded bits across chunks.
  Evidence: viterbi::tests::noisy_aged_paths_recover_clean_bits_and_arbitrary_bit_partitions; 8,192,000-noise-step audit probe: subsequent clean comparison has zero errors..
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

## Remaining findings

- [ ] **S01 · P1** — Advertised interleaved 80 ksym/s Meteor transport is missing
- [ ] **S02 · P1** — OQPSK carrier/timing recovery depends on absolute RF phase
- [ ] **S04 · P1** — First successful ambiguity lane is selected forever
- [ ] **S08 · P2** — Image origin is the satellite's sequence zero rather than the received pass
- [ ] **S10 · P2** — Lock and decoded-line diagnostics can report success without current reception
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
- [ ] **R14 · P1** — Rejected hardware tuning changes capture metadata anyway
- [ ] **R15 · P1** — HackRF callback overflow is silently stitched into continuous capture
- [ ] **R16 · P2** — Soapy timeout is treated as a fatal driver failure on its first occurrence
- [ ] **R17 · P2** — Daemon RTL gain control reports success without checking the hardware result
- [ ] **R18 · P2** — rtl_tcp gain is a successful no-op
- [ ] **R19 · P2** — Daemon narrowband listening uses a wide fixed channel without mode-specific RF selection
- [ ] **R20 · P2** — Scanning can wait forever on a frequency a fixed replay capture cannot contain
- [ ] **R22 · P2** — Read cancellation and timed-out hardware commands lack a terminal contract

## Verification and limits

Normal workspace tests pass with hardware/audio disabled (`cargo test --workspace --no-default-features -- --test-threads=4`). The additional native float RF/audio regression passes. Default workspace build/all-target checks, Clippy with warnings denied, formatting, and all daemon hardware feature checks pass; the hardware check reports that libhackrf and SoapySDR are unavailable on this host. No receiver was opened.

The original long Viterbi counterexample now reports zero errors in the 1,920 aligned clean bits after 8,192,000 noisy trellis steps. The chronological-marker probe extracts both frames. Split primary headers complete; the old cross-VCID probe produces AAAA instead of BBAA. The independent 896-pixel libjpeg fixture has maximum error 1 and mean error 0.021205; the exact DC vector produces pixel 160.

The historical reproduce.py verifies the baseline counterexamples, so it intentionally no longer passes unchanged on repaired code. Active regressions and repair-verification.json are the evidence for this batch. R06/R07 acceptance probes remain ignored pending their repairs.

Graphify was refreshed with the AST-only update command. It skipped HTML because the graph exceeds 5,000 nodes; code navigation remains available. Pre-existing user deletions and the untracked AGENTS.md are excluded from repair commits. The formatter normalized three previously unformatted areas and the audit example now uses the equivalent Clippy-approved parity expression.

JPEG integer quantization was checked against [the pinned SatDump mission implementation](https://github.com/SatDump/SatDump/blob/1aedd602055c154401384a8cc7876713bb371254/plugins/meteor_support/meteor/instruments/msumr/lrpt/huffman.cpp). Packet-header and idle-frame handling follows [CCSDS AOS 732.0-B-4](https://ccsds.org/Pubs/732x0b4.pdf).

Next priority: R14/R15/R16/R17/R18/R22 receiver applied-state, loss and bounded cancellation; then S01/S02/S04 real Meteor transport/carrier reacquisition, followed by remaining selectivity and monitoring repairs.
