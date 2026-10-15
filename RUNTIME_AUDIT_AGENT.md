# Runtime / DSP audit — current working tree

Audit scope: daemon state, ingest and channelizer lifecycles, recording/replay, ADS-B packet path, and LRPT streaming. Findings are based on the current uncommitted tree (2026-09-13). No production code was changed. Confidence is high unless marked otherwise.

## Critical findings

### P0 — LRPT decoder omits the mandatory convolutional/Viterbi FEC stage (confirmed)

`lrpt-decode/src/lib.rs:3-5` documents a QPSK → differential → CADU → derandomize → Reed-Solomon pipeline. `process_cadu_frame` at `lrpt-decode/src/lib.rs:268-300` slices the 1020-byte transport and immediately derandomizes and RS-decodes it. `lrpt-decode/Cargo.toml:9-17` has no Viterbi/convolutional decoder dependency or implementation. Meteor LRPT CADUs are convolutionally encoded (CCSDS K=7, rate 1/2) before the attached sync/RS stages; a real on-air stream therefore cannot reach a valid 32-bit CADU marker/RS frame through this pipeline. Existing end-to-end tests synthesize uncoded bits and consequently validate only the internal fixture format. This makes advertised real LRPT decoding nonfunctional.

### P0/P1 — LRPT image output is not a Meteor image decoder (confirmed)

`lrpt-decode/src/image_builder.rs:82-98` unconditionally discards 14 payload bytes and treats every remaining byte as one grayscale row. It does not consult `SpacePacketHeader::secondary_header_flag`, packet type, scanline sequence, channel geometry, or the MSU-MR compression/packing format. Packets without a 14-byte instrument header lose real pixels, and packets ≤14 bytes disappear. Real Meteor MSU-MR products are compressed/structured; returning these bytes as grayscale cannot produce a valid image. The code comments explicitly mark the 14-byte assumption as TODO/unverified.

## Daemon runtime/control findings

### P1 — ADS-B pipeline loses frames crossing channelizer block boundaries (confirmed)

`ez-daemon/src/pipelines/packet.rs:117-136` builds each `MagBuf` from only the current block, sets `overlap: 0`, and marks every block `MAGBUF_DISCONTINUOUS`. `dump1090/src/demod.rs:647-650` resets `last_message_end` on that flag; the decoder therefore has no history when a Mode-S preamble/message straddles the 8,192-sample block boundary. Such frames are silently missed.

### P1 — ADS-B timestamps use the wrong unit in the daemon (confirmed)

`dump1090/src/demod.rs:55-56` defines `MagBuf::sample_timestamp` in 12 MHz ticks, and timestamp arithmetic at `dump1090/src/demod.rs:926-928`/`1126-1128` assumes that unit. The standalone reader converts sample indices with `ticks_per_sample` at `dump1090/src/main.rs:217,265-272`; daemon code passes `SampleBlock::start_sample` directly at `ez-daemon/src/pipelines/packet.rs:128-135` (2.4 MHz sample counts). Beast/telemetry timestamps are consequently off by about 5× (and system elapsed conversion is wrong).

### P1 — `set_sample_rate` leaves channelizer geometry stale (confirmed)

`ez-daemon/src/state.rs:181-192` forwards a new rate to the ingestion source after requiring no active channels, but never updates `Channelizer.wideband_rate_hz`. New channels created after the call use stale decimation/NCO geometry (`ez-daemon/src/channelizer.rs:247-255`) while incoming `SampleBlock`s carry the new rate (`ez-daemon/src/ingest.rs:149-153`). The first rate change therefore leaves the DSP configured for the old sample rate until daemon restart.

### P1 — `retune` bypasses channel validation (confirmed)

`ez-daemon/src/state.rs:456-480` accepts arbitrary bandwidth/offset and does not call `validate_spec`. For virtual taps, `Channelizer::retune_channel` only rejects zero output rate and out-of-Nyquist center (`ez-daemon/src/channelizer.rs:275-306`); it does not enforce the requested bandwidth against the channel's kind (e.g. ADS-B's required 2.4 MHz) or full passband edge. Spectrum channels (`internal_channel_id == None`) bypass even those checks. A REST/TCP retune can leave metadata and actual processing inconsistent.

### P1 — Concurrent first-time `subscribe` can return a dead pipeline handle (confirmed)

`ez-daemon/src/state.rs:251-280` checks the channel map and creates a pipeline without holding the map lock. Two callers can create the same ID; insertion at `ez-daemon/src/state.rs:282-301` replaces and stops the earlier `ActiveChannel`, but the earlier caller already received `sub` from the stopped pipeline. That caller receives a successful subscribe response yet no future frames.

### P1 — Removing a channel leaks its recording (confirmed)

`ez-daemon/src/state.rs:492-503` removes/stops the pipeline and channelizer tap but never removes the channel's entry from `RecordingManager`. The recorder thread at `ez-daemon/src/recording.rs:201-225` retains its subscription and polls forever (200 ms timeout), the file remains open, and a later `start_recording` for the same ID is rejected by `recording.rs:121-123` as already recording.

### P1 — Removing a channel leaves client forwarder threads polling forever (confirmed)

`remove_channel` only clears the daemon channel (`state.rs:492-503`). TCP/WS forwarders are controlled by per-connection flags in `ez-daemon/src/server.rs:323-346` and `ez-daemon/src/web/ws.rs:117-128`; no channel-removal signal reaches them. Their handles continue `recv_timeout(FORWARD_POLL)` loops (`server.rs:376-402`) until the client independently disconnects/unsubscribes.

### P1 — `i64::MIN` offset can panic request handlers (confirmed)

`ez-daemon/src/state.rs:159-160` computes `spec.center_offset_hz.abs()`. For network-supplied `i64::MIN`, debug builds panic on `abs()` overflow; near `i64::MAX`, the subsequent addition can overflow too. This is a remotely triggerable daemon request-handler panic.

### P1 — Invalid negative absolute frequency is silently clamped to DC (confirmed)

`ez-daemon/src/state.rs:431-435`, `:471-473`, and recording `:550-556` compute `(wideband_center + offset).max(0)`. A negative requested absolute frequency is converted to zero and the channel spec still reports the original offset, so control metadata and tuned hardware diverge instead of returning an error.

### P1 — LRPT reassembly aliases identical APIDs across VCIDs (confirmed)

`lrpt-decode/src/ccsds.rs:194-203` stores `apid_buffers` keyed only by APID while VCID state is separate. Continuations at `:232-268` and buffer/complete operations at `:305-335` therefore share one byte buffer when two virtual channels use the same APID; interleaving or a gap on one VCID can append/clear the other VCID's packet.

### P1 — LRPT frame sync can discard a valid marker before it searches (confirmed)

`lrpt-decode/src/frame_sync.rs:149-157` caps an unlocked buffer by draining its oldest bits before calling `find_sync`. If a CADU marker begins near the drained boundary (or a chunk contains a marker but not a complete CADU yet), the marker is discarded and the decoder may never lock. The current test set does not exercise marker placement at this cap boundary.

### P1 — LRPT M-PDU headers near payload end are dropped (confirmed)

`lrpt-decode/src/ccsds.rs:284-301` loops only while `cursor + SPACE_PACKET_HEADER_LEN <= payload.len()`. If a valid packet primary header begins in the final 1–5 bytes, it is skipped without retaining those bytes for the next VCDU, so packet reassembly loses data at normal frame segmentation boundaries.

## Lower-severity runtime findings

### P2 — Replay restart does not rewind the file (confirmed)

`ez-daemon/src/hardware/replay.rs:97-103` resets timing counters on `start()` but does not seek the reader to offset zero. Restarting a finite/non-looping replay after EOF returns EOF forever; restarting a looping replay resumes at its current position rather than the beginning.

### P2 — Replay loop mishandles partial sample tails (confirmed)

`ez-daemon/src/hardware/replay.rs:127-163` loops bytes, then conversion silently ignores a trailing incomplete sample (`lrpt-decode/src/iq_source.rs:17-27,30-44`). With a file whose length is not a multiple of 2/8 bytes, the final partial bytes are combined with the next loop's beginning in `byte_buf`, causing sample-boundary corruption.

### P2 — Synthetic source permits zero rate and divides by zero (confirmed)

`ez-daemon/src/hardware/synthetic.rs:184-187` accepts `set_sample_rate(0)`. `generate_block` computes `fs = sample_rate_hz as f32` and divides time/phase by `fs` at `:89-101`; direct use after a zero-rate control call yields infinities/NaNs and invalid IQ.

### P2 — NaN volume/squelch controls inject NaN or disable gating (confirmed)

`ez-daemon/src/state.rs:506-512` performs no finite-value validation. `AudioPipeline::set_volume` stores `level.clamp(0.0, MAX_VOLUME)` (`ez-daemon/src/pipelines/audio.rs:94-99`), and Rust's `f32::clamp` preserves NaN. `process_block` then multiplies samples by NaN at `audio.rs:180-185`. NaN squelch at `audio.rs:108-110,173-181` makes `power_db < NaN` false, disabling squelch.

### P2 — Unchecked absolute-frequency arithmetic during global retune (confirmed risk)

`ez-daemon/src/channelizer.rs:356-360` changes wideband center and rebuilds each NCO from `u64` absolute centers using floating subtraction, without checking whether existing channels remain inside the new Nyquist span. The module documents that out-of-span channels continue emitting meaningless data (`:350-355`), so a large hardware retune silently invalidates existing channels.

### P2 — QPSK interpolation returns zero at newest exact sample (confirmed)

`lrpt-decode/src/qpsk.rs:269-287` rejects interpolation when `idx1 > latest`. For an exact integer decision at `pos == latest`, the available sample is valid but `idx1 = latest + 1` triggers the zero return. Integer SPS configurations commonly produce integer timing instants, injecting zeros into Gardner/Costas decisions at block edges.

### P2 — GUI offline LRPT decode reads entire recording into RAM (confirmed)

`ez-gui/src/decoding_panel.rs:115-123` uses `std::fs::read` before feeding the streaming decoder. A multi-gigabyte capture therefore allocates the complete file plus decoder working buffers, defeating `decode_file`'s documented streaming behavior and risking OOM.

### P2 — LRPT progress clones all accumulated images repeatedly (confirmed)

`lrpt-decode/src/lib.rs:327-344` renders/clones every image on each progress update; daemon publication then clones each image pixel buffer again at `ez-daemon/src/pipelines/telemetry.rs:85-98`. As height grows, each periodic update becomes O(total pixels), causing avoidable CPU/allocation pressure during long passes.

## Validation

`cargo test -q -p dump1090 --lib` passed (240 tests). Existing LRPT tests are synthetic and do not include the mandatory convolutional FEC or real MSU-MR packet/image fixtures; they therefore do not contradict the P0/P1 LRPT findings.
