# Streaming RDS implementation report

Harness/model: built-in Codex session subagent `/root/radio_rebuild`, inherited GPT-6-family route; exact provider identifier not exposed. No Morph or agy.

## Delivered module

`ez-gui/src/radio_rds.rs` provides a streaming WFM multiplex decoder with a57kHz complex mixer, fourth-order3000Hz anti-alias filter, exact-rate interpolation to19000Hz,16 staggered biphase timing candidates, complex differential detection, and CRC-validated26bit A/B/C-or-C-prime/D groups. Acquisition scans for A; synchronized reception preserves exact26bit block boundaries. C-prime must repeat the current PI. A complete valid four-block group is required before metadata changes.

Each decoder retains fixed DSP/filter/parser buffers,64 constellation points maximum,8-byte PS and64-byte RadioText assembly. It does not retain an input waveform or allocate per sample. Processing is linear in input sample count; expensive protocol work runs at19kHz. Snapshot strings and constellation copies are bounded. Input must be finite-rate120kHz–20MHz multiplex; invalid rates reset. Nonfinite samples becomezero to protect state.

## API

- `RdsDecoder::new()`, `reset()`.
- `set_incremental(bool)` and `set_region(RdsRegion)`; defaults incrementaltrue, Europe.
- `process_multiplex(samples: &[f32], sample_rate: f64)`.
- `snapshot() -> RdsSnapshot`.
- Snapshot: synchronized, PI, PTY, TP, TA, music, program_service, radio_text, valid_groups, crc_errors, region, recent_symbols.
- Snapshot helpers: `pty_label`, `country_code`, `program_coverage`, `reference_number`.

Root owns module export and config/UI/app wiring. Demod owner supplies `set_rds_tap_enabled` and `take_wfm_multiplex` pre-deemphasis raw FM discriminator output at its exact sample rate. Decoding should continue while audible output is muted. Reset on retune/source generation/demod mode changes or RDS disable; sample-rate changes reset internally.

## Functional behavior

Group0A/0B assemble eight-character PS and expose TA/music. All valid groups update PI/PTY/TP. Group2A assembles64-byte text,2B32-byte text; carriage return terminates messages. A/B flag or2A↔2B change clears prior RadioText. Station PI change clears previous PS/RT/flags. Incremental mode displays received segments with spaces for missing ones; complete mode waits for all segments through termination, or the complete field. Region selects EU/RBDS PTY wording; country/coverage/reference helpers expose encoded European PI fields only in Europe and return None in North America, where RBDS PI may encode a callsign. No country or callsign name is guessed.

The symbol diagram receives actual normalized matched-filter complex samples from the selected timing phase,64 most recent. Carrier phase is intentionally unrestricted: differential detection tolerates phase rotation, so a carrier offset can rotate this diagram. Synchronization expires aftertwo seconds without a validated group. Last decoded labels remain available with synchronized=false until an explicit reset.

## Validation

Eight optimized standalone tests passed. After peer review, final crate verification also passed: `cargo test -p ez-gui --lib radio_rds::tests -- --nocapture` —9 passed,0 failed,0.93 seconds (628 unrelated tests filtered).

1. CRC offsets and every single-bit error in representative26bit words.
2. Arbitrary-bit acquisition, corrupted-group rejection and recovery.
3. Correct C-prime and repeated-PI requirements for versionB.
4. Complete/incremental text state, A/B message changes, region labels and station/reset clearing.
5. Complete MPX reception at250000Hz/−3Hz subcarrier mismatch,240000.5Hz/+3Hz, and2400000Hz/arbitrary phase, with pilot/audio/stereo interferers and random noise. Recovered `ASTRA FM` and `CRC checked radio text 2026`, at least20 validated groups per case.
6. Independent root-raised-cosine shaped biphase transmitter, version2B RadioText,−12Hz carrier offset and+100ppm symbol-clock drift. Recovered `PULSE FM` and `Shaped RDS signal`.
7. Exact results across arbitrary input chunk partitions; lock expiration after silence, NaN recovery, retune and sample-rate reset.
8. Noise/pilot/audio without RDS produces no decoded station metadata.

The shaped test found a genuine parser defect: searching forA at arbitrary bit positions after a valid group could latch payload-dependent accidental CRC matches and repeatedly skip certain text segments. Keeping the nextA aligned26bits afterD fixed it.

## Limits

No physical RF reception was claimed. No bit-error correction is attempted; corrupt groups are discarded. Basic RDS ASCII is decoded; extended RDS glyphs currently render replacement characters instead of guessed Unicode. No AF/EON/clock-time/ODA metadata is invented. Carrier recovery uses complex differential symbols and parallel timing phases, rather than reproducing SDR++'s full PLL implementation. Synthetic reception proves the implemented signal path, not universal weak-signal equivalence. FM IF noise reduction is upstream and may attenuate RDS modulation; benchmark/quality results with it enabled must be reported separately.

Measured inline decoder storage:2016bytes on this64bit build, plus bounded decoded text strings and snapshot copies. Full IQ→demod→RDS throughput is owned by the integration agent and is not claimed from these module-only tests.

Peer review by `/root/meteor_completion` found and prompted the North American PI-field fix. A ninth regression test verifies regional helper availability and distinct EU/RBDS PTY labels. CRC sequencing and metadata masks otherwise reviewed cleanly.
