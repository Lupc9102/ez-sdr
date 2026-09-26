# EZ-SDR Master Production Audit Report
**Session & Swarm Audit Consolidated Findings**  
**Date:** 2026-09-20  
**Scope:** Entire workspace (`dump1090`, `ez-daemon`, `ez-gui`, `lrpt-decode`, `ez-proto`, `ez-web`)  
**Assessment:** **NOT PRODUCTION READY (Grade: C-)**  

---

## 1. Executive Summary

Following the interruption of the initial audit fleet, a specialized swarm was deployed across all eight core subsystems:
1. **DSP Pipelines** (`channelizer.rs`, `pipelines/audio.rs`, `pipelines/packet.rs`, `pipelines/telemetry.rs`)
2. **LRPT Satellite Decoder** (`qpsk.rs`, `viterbi.rs`, `frame_sync.rs`, `reed_solomon.rs`, `image_builder.rs`, `ccsds.rs`)
3. **Daemon Server, Network & Concurrency** (`server.rs`, `web/ws.rs`, `web/api.rs`, `state.rs`, `hardware/tcp_iq.rs`)
4. **GUI Core & Demodulation** (`source_manager.rs`, `sdr_panel.rs`, `app.rs`, `daemon_client.rs`, `demod.rs`, `spectrum.rs`)
5. **GUI Features & Satellite/Scanner** (`satellite_panel.rs`, `tle_engine.rs`, `scanner.rs`, `recorder_panel.rs`, `bookmarks.rs`, `decoding_panel.rs`)
6. **Frontend Web Application** (`main.ts`, `audio/monitor.ts`, `workers/spectrum-worker.ts`, `ui/aircraft-panel.ts`, `ui/telemetry-panel.ts`, `api/wire.ts`)
7. **Unsafe Code, FFI & Memory Safety** (131 `unsafe` blocks across `dump1090`, `ez-daemon`, `ez-gui`, `SoapySDR`, `librtlsdr`, `libhackrf`)
8. **ADS-B Mode S / CPR Flight Tracker** (`dump1090/src/demod.rs`, `cpr.rs`, `icao_filter.rs`, `mode_s.rs`, `track.rs`)

### Master Defect Metrics
- **Total Confirmed Defects:** **110+**
- **CRITICAL Severity (Memory UB, Fatal Crashes, Deadlocks, Total Algorithmic Failure):** **24**
- **HIGH Severity (Severe Data Loss, Protocol Violations, Resource Leaks, Broken Features):** **38**
- **MEDIUM Severity (DSP Inaccuracies, Performance Thrashing, Missing Resampling):** **32**
- **LOW Severity (Logging, Path Leaks, Code Quality):** **18**

---

## 2. Top 15 Showstopper Defects Requiring Immediate Fixes

### 1. LRPT: Missing CCSDS Dual-Basis (Berlekamp) Conversion (`lrpt-decode/src/reed_solomon.rs`)
- **Severity: CRITICAL**
- Meteor M2-x downlinks transmit RS(255, 223) in CCSDS dual basis. The codec operates strictly in conventional basis without conversion. As a result, **100% of real off-air frames fail Reed-Solomon correction**, causing `lib.rs:339` to discard every frame.

### 2. LRPT: Unbounded Trellis Metric Growth Panics Decoder (`lrpt-decode/src/viterbi.rs:59–74`)
- **Severity: CRITICAL**
- `ViterbiDecoder` never normalizes path metrics. At 72k/80k baud, all 64 states exceed `INF = 1_000_000` within 12–15 seconds. Once exceeded, line 62 skips all states, locking the metric array to `[INF; 64]` and outputting junk/zeros indefinitely.

### 3. LRPT: Scanline 1+ Rejection Bug (`lrpt-decode/src/image_builder.rs:267–270`)
- **Severity: CRITICAL**
- `decode_segment` asserts `payload[9] != 0 || payload[10] != 0`. In authentic MSU-MR packets, bytes 9–10 contain the 16-bit scanline index. Only scanline 0 is ever decoded; **all subsequent scanlines are discarded as corrupted**.

### 4. dump1090: Demodulator Preamble Bounds Read Panic (`dump1090/src/demod.rs:683–686`)
- **Severity: CRITICAL**
- Guard `preamble.len() < 289` permits entry when `len == 289`. In phase 8, sampling index reaches offset 289. Slicing panics with `index out of bounds: the len is 289 but the index is 289`, crashing the receiver on real RF preambles.

### 5. dump1090: Address/Parity Frames Set `correctedbits = usize::MAX` (`dump1090/src/demod.rs:483, 962`)
- **Severity: CRITICAL**
- In surveillance replies (DF0/4/5/16/20/21), parity contains the address and returns error code `-1`. Line 483 casts `-1 as usize` (`18446744073709551615`). Line 962 indexes `stats.demod_accepted[decoded_mm.correctedbits]`, immediately panicking the fixed 3-element array.

### 6. dump1090: Flight Tracking & CPR Disconnected in Runtime (`dump1090/src/main.rs:301–306`)
- **Severity: CRITICAL**
- In standalone `dump1090`, `mode_s::decode_mode_s` is a stub returning empty fields. `cpr::CprDecoder` is never instantiated or called. Aircraft tracking never records coordinates, altitude, callsign, or speed.

### 7. Hardware: Missing `catch_unwind` in C FFI Callbacks (`dump1090/src/sdr/hackrf.rs:85`, `ez-daemon/src/hardware/hackrf.rs:101`)
- **Severity: CRITICAL**
- `extern "C" rx_callback` performs heap allocations (`to_vec()`) and channel sends without `catch_unwind`. A Rust panic unwinding across the C ABI boundary triggers immediate, uncatchable process abort.

### 8. Hardware: Use-After-Free Race on HackRF Teardown (`dump1090/src/sdr/hackrf.rs:250`, `ez-daemon/src/hardware/hackrf.rs:245`)
- **Severity: CRITICAL**
- `hackrf_stop_rx` return codes are ignored and `self.ctx` is immediately deallocated with `Box::from_raw` while in-flight USB transfer callbacks are actively accessing `ctx`.

### 9. DSP: Incomplete Weaver SSB Demodulator Folds Audio Pitch (`ez-daemon/src/pipelines/audio.rs:267–292`)
- **Severity: CRITICAL**
- `demod_ssb` shifts baseband by 1650 Hz and low-pass filters, but **omits the second quadrature mixer**. Direct extraction of `2.0 * filtered.re` folds positive and negative frequencies across 0 Hz, inverting pitches and rendering voice speech unintelligible.

### 10. Daemon: Cascading Mutex Poisoning on Pipeline Panics (`ez-daemon/src/state.rs:650`, `server.rs`)
- **Severity: CRITICAL**
- DSP threads run with `.lock().unwrap()`. If an audio/telemetry thread panics, its mutex is poisoned. Any subsequent REST call (`/api/channels`, `/api/status`) calls `.lock().unwrap()` on the poisoned lock and cascades crashes to Axum worker threads.

### 11. Daemon: Unbounded WebSocket Spawning & Thread Exhaustion DoS (`ez-daemon/src/web/mod.rs:59`, `server.rs:384`)
- **Severity: CRITICAL**
- Unlike the TCP server, the WebSocket server has no connection semaphore. Opening multiple `/ws/stream/...` connections spawns dedicated OS threads via `.expect("spawning forwarder thread")`, crashing the daemon process when thread limits are reached.

### 12. GUI: Synchronous File Dialogs Kill Hardware Worker (`ez-gui/src/spectrum.rs:665`, `source_manager.rs:306`)
- **Severity: CRITICAL**
- `rfd::FileDialog::save_file` runs synchronously on the UI thread. In 128 ms, the 32-slot sample buffer channel overflows. `source_manager.rs` treats channel overflow as fatal, terminates the hardware worker, and closes the SDR.

### 13. GUI: Missing In-Session Retuning in Local RTL-SDR Mode (`ez-gui/src/source_manager.rs:202, 901`)
- **Severity: CRITICAL**
- The RTL-SDR hardware worker loop captures frequency and gain by value and contains no command queue. Tuning the waterfall or changing bookmarks in the GUI updates only UI labels while the physical tuner remains locked to the initial frequency.

### 14. GUI: Scanner Squelch Uses Global Bandwidth Peak (`ez-gui/src/app.rs:953`, `scanner.rs:706`)
- **Severity: CRITICAL**
- Squelch evaluates `state.spectrum.peak_level()` (the maximum across the entire 2.048 MHz FFT window) rather than the channel power at `current_freq_hz`. Any single strong transmitter triggers false positive hits across every scanned frequency within ±1 MHz.

### 15. Frontend: `transferToImageBitmap()` Erases Waterfall History (`ez-web/src/workers/spectrum-worker.ts:130`)
- **Severity: CRITICAL**
- Per the HTML specification, calling `transferToImageBitmap()` clears the source canvas. Subsequent calls to `drawImage(canvas, ..., dy=1)` copy transparent black pixels, preventing waterfall history from accumulating.

---

## 3. Subsystem Breakdown & Comprehensive Findings

### A. DSP & Demodulation Subsystem (`ez-daemon`, `dump1090`)
- **ISSUE-DSP-01 [CRITICAL]:** Weaver SSB missing second mixer folds spectrum and inverts audio frequencies (`pipelines/audio.rs:267`).
- **ISSUE-DSP-02 [HIGH]:** Unbounded NCO phase accumulation during wideband retunes overflows to `±Inf` and injects `NaN`s (`channelizer.rs:49`).
- **ISSUE-DSP-03 [HIGH]:** 289-sample dead zone between packet blocks causes systematic ~3.5% ADS-B packet loss (`pipelines/packet.rs:132`, `dump1090/demod.rs:680`).
- **ISSUE-DSP-04 [HIGH]:** RTL-SDR/HackRF samples output in $[-128, 128]$ rail-clip AM demodulation into square waves (`pipelines/audio.rs:212`).
- **ISSUE-DSP-05 [HIGH]:** AM DC blocker and WFM de-emphasis single-pole IIR filters latch permanently into `NaN` upon a single non-finite sample (`pipelines/audio.rs:225, 255`).
- **ISSUE-DSP-06 [HIGH]:** Channelizer monotonic counter masks sample drops, corrupting downstream decoder timelines (`channelizer.rs:198`).
- **ISSUE-DSP-07 [MEDIUM]:** Fixed 63-tap FIR filter provides inadequate stopband attenuation for decimation factors $D > 10$ (`channelizer.rs:18`).
- **ISSUE-DSP-08 [MEDIUM]:** No audio resampler to standard 48 kHz PCM, emitting arbitrary rates to clients (`pipelines/audio.rs:218`).
- **ISSUE-DSP-09 [MEDIUM]:** Complete omission of CW / BFO injection demodulator (`pipelines/audio.rs:185`).

### B. LRPT Satellite Subsystem (`lrpt-decode`)
- **ISSUE-LRPT-01 [CRITICAL]:** Missing CCSDS dual-basis (Berlekamp) conversion; 100% of real codewords fail RS check (`reed_solomon.rs:86`).
- **ISSUE-LRPT-02 [CRITICAL]:** Viterbi path metrics unnormalized; overflows `INF` in ~12.5s and permanently halts decoding (`viterbi.rs:59`).
- **ISSUE-LRPT-03 [CRITICAL]:** FrameSync flywheel drains 1 bit instead of a full frame on sync miss, corrupting frame boundaries (`frame_sync.rs:185`).
- **ISSUE-LRPT-04 [CRITICAL]:** `payload[9..11] != 0` check discards all scanlines $\ge 1$ (`image_builder.rs:267`).
- **ISSUE-LRPT-05 [CRITICAL]:** `selected_lane` never resets to `None` on carrier phase slip, permanently muting the decoder (`lib.rs:279`).
- **ISSUE-LRPT-06 [HIGH]:** Scanline calculation divides sequence by 43 instead of 14, compressing images to 1/3 height (`image_builder.rs:71`).
- **ISSUE-LRPT-07 [HIGH]:** Hard-decision Viterbi slicing discards 2.5–3 dB of coding gain (`viterbi.rs:23`).
- **ISSUE-LRPT-08 [HIGH]:** 4th-power Costas loop exhibits severe noise amplification and insufficient pull-in for uncompensated Doppler (`qpsk.rs:369`).
- **ISSUE-LRPT-09 [MEDIUM]:** Inner Viterbi loop generates 640,000 heap allocations per second (`viterbi.rs:84`).

### C. ADS-B & dump1090 Subsystem (`dump1090`)
- **ISSUE-ADSB-01 [CRITICAL]:** Preamble scan bounds check allows read out of bounds at index 289 (`demod.rs:683`).
- **ISSUE-ADSB-02 [CRITICAL]:** Address/Parity frames set `correctedbits = -1 as usize`, panicking fixed-size array (`demod.rs:483, 962`).
- **ISSUE-ADSB-03 [CRITICAL]:** Flight tracking, CPR coordinate resolution, and Mode S fields completely unhooked in `main.rs` (`main.rs:301`).
- **ISSUE-ADSB-04 [HIGH]:** Backward time clock adjustments cause panic in `duration_since` and stale frame pairing (`cpr.rs:329, 359`).
- **ISSUE-ADSB-05 [HIGH]:** Reaching 2048 CPR cache entries wipes the entire cache across all aircraft (`cpr.rs:341`).
- **ISSUE-ADSB-06 [HIGH]:** 4096-bit ICAO filter saturates and performs total wipeout every 60 seconds (`icao_filter.rs:38`).
- **ISSUE-ADSB-07 [HIGH]:** Interrogated DF11 All-Call replies rejected due to non-zero Interrogator Identifier in parity (`mode_s.rs:33`).
- **ISSUE-ADSB-08 [HIGH]:** Noise floor calculation underflow produces `log10(0.0) = -inf` and `NaN` (`demod.rs:1001`).
- **ISSUE-ADSB-09 [MEDIUM]:** Unconstrained 2-bit error correction modifies ICAO addresses and generates phantom aircraft (`demod.rs:270`).
- **ISSUE-ADSB-10 [MEDIUM]:** Negative barometric altitudes below sea level clamped to 0 ft (`mode_s.rs:119`).

### D. Server, Network & Concurrency Subsystem (`ez-daemon`)
- **ISSUE-NET-01 [CRITICAL]:** Cascading mutex poisoning on pipeline thread panic (`state.rs:650`, `state.rs:129`).
- **ISSUE-NET-02 [CRITICAL]:** Unbounded WebSocket connection acceptance exhausts OS threads (`web/mod.rs:59`).
- **ISSUE-NET-03 [CRITICAL]:** Integer negation overflow panic on `center_offset_hz: i64::MIN` (`state.rs:161`).
- **ISSUE-NET-04 [HIGH]:** Channelizer mutex locked during 100ms receive timeout starves control commands (`state.rs:129`, `channelizer.rs:403`).
- **ISSUE-NET-05 [HIGH]:** Missing `Origin` header validation enables Cross-Site WebSocket Hijacking (CSWSH) (`web/ws.rs:42`).
- **ISSUE-NET-06 [HIGH]:** `TcpIqSource` terminates ingestion thread on transient 5s socket timeout (`tcp_iq.rs:71`, `ingest.rs:164`).
- **ISSUE-NET-07 [HIGH]:** Race condition in `subscribe` overwrites active channel and leaves client orphaned (`state.rs:291`).
- **ISSUE-NET-08 [HIGH]:** TOCTOU race between hardware and channelizer during sample rate change (`state.rs:180`).
- **ISSUE-NET-09 [MEDIUM]:** Synchronous disk I/O and thread joins on Tokio worker threads (`web/api.rs:228`).
- **ISSUE-NET-10 [MEDIUM]:** Orphaned recording files when channels are deleted (`state.rs:496`).

### E. GUI Core & Features Subsystem (`ez-gui`)
- **ISSUE-GUI-01 [CRITICAL]:** Synchronous file dialogs freeze UI and kill SDR hardware thread (`spectrum.rs:665`, `source_manager.rs:306`).
- **ISSUE-GUI-02 [CRITICAL]:** Live frequency and gain controls non-functional in Local RTL-SDR mode (`source_manager.rs:202`).
- **ISSUE-GUI-03 [CRITICAL]:** Real-time DSP, Demodulation, and FFT run synchronously on the Main UI thread (`app.rs:572`).
- **ISSUE-GUI-04 [CRITICAL]:** Decoder background thread panic permanently deadlocks Decoding Panel (`decoding_panel.rs:138`).
- **ISSUE-GUI-05 [CRITICAL]:** Scanner squelch uses global FFT peak instead of channel power (`app.rs:953`, `scanner.rs:706`).
- **ISSUE-GUI-06 [CRITICAL]:** WAV recorder lacks `Drop` finalization, leaving RIFF headers corrupt on exit (`recorder_panel.rs:19`).
- **ISSUE-GUI-07 [HIGH]:** Integer decimation drift (+762 samples/s) overflows 96k audio ring buffer in ~126s (`demod.rs:274`).
- **ISSUE-GUI-08 [HIGH]:** Source mode switching leaves zombie daemon client clobbering UI status (`sdr_panel.rs:268`).
- **ISSUE-GUI-09 [HIGH]:** Daemon audio mode bypasses volume and squelch controls (`app.rs:485`).
- **ISSUE-GUI-10 [HIGH]:** TLE engine rejects all satellites outside hardcoded 3-satellite whitelist (`tle_engine.rs:112`).
- **ISSUE-GUI-11 [HIGH]:** Satellite Doppler auto-tuning is a dead no-op (`satellite_panel.rs:584`).
- **ISSUE-GUI-12 [HIGH]:** Decoding panel clones full multi-megabyte image buffers every UI frame (`decoding_panel.rs:288`).
- **ISSUE-GUI-13 [MEDIUM]:** `atomic_write_file` lacks `fsync`, risking 0-byte truncated configs on power loss (`bookmarks.rs:236`).

### F. Frontend Web Application Subsystem (`ez-web`)
- **ISSUE-WEB-01 [CRITICAL]:** `transferToImageBitmap()` clears canvas, erasing waterfall history (`workers/spectrum-worker.ts:130`).
- **ISSUE-WEB-02 [CRITICAL]:** `AudioContext` initialized inside network callback violates browser autoplay policy (`audio/monitor.ts:26`).
- **ISSUE-WEB-03 [HIGH]:** `StreamSocket` permanently terminates reconnection after 5 failed attempts (`api/stream.ts:15`).
- **ISSUE-WEB-04 [HIGH]:** Missing audio sample rate conversion and clock drift compensation in Worklet (`public/audio-processor.js:14`).
- **ISSUE-WEB-05 [HIGH]:** Startup race condition creates duplicate default channels and emits error banner (`main.ts:95`).
- **ISSUE-WEB-06 [HIGH]:** 2,000,000-iteration scalar pixel unpack loop freezes UI on telemetry frames (`ui/telemetry-panel.ts:50`).
- **ISSUE-WEB-07 [MEDIUM]:** Stored XSS vulnerability via unescaped recording path in channel controls (`ui/channel-controls.ts:72`).
- **ISSUE-WEB-08 [MEDIUM]:** DOM layout thrashing and full table rebuilds on every ADS-B packet (`ui/aircraft-panel.ts:65`).
- **ISSUE-WEB-09 [MEDIUM]:** ArrayBuffer slicing memory churn in binary frame decoders (`api/wire.ts:21`).

### G. Unsafe Code & Memory Safety Subsystem
- **ISSUE-SEC-01 [CRITICAL]:** Raw pointer dereferences without null checks in HackRF C callbacks (`dump1090/hackrf.rs:85`, `ez-daemon/hackrf.rs:101`).
- **ISSUE-SEC-02 [CRITICAL]:** Panics unwinding across `extern "C"` FFI boundaries without `catch_unwind` (`dump1090/hackrf.rs:85`).
- **ISSUE-SEC-03 [CRITICAL]:** Asynchronous Use-After-Free race on HackRF teardown (`dump1090/hackrf.rs:250`).
- **ISSUE-SEC-04 [HIGH]:** Unchecked `n_read as usize` triggers out-of-bounds slice indexing panic in RTL-SDR (`ez-daemon/rtlsdr.rs:277`).
- **ISSUE-SEC-05 [HIGH]:** Inlined RTL-SDR FFI ABI mismatch (`u32` vs `c_int`) and return code ignored in `ez-gui` (`source_manager.rs:298`).
- **ISSUE-SEC-06 [HIGH]:** `HackrfTransfer` struct size mismatch between Rust (32 bytes) and C `hackrf.h` (40 bytes) (`dump1090/hackrf.rs:20`).
- **ISSUE-SEC-07 [HIGH]:** Unchecked `samples_read` slice bounds in SoapySDR and missing `deactivateStream` (`ez-daemon/soapy.rs:360`).

---

## 4. Production Readiness Roadmap & Recommendations

To achieve production readiness on par with SDR++, SatDump, and dump1090:

1. **Phase 1: Memory Safety & Crash Prevention (Days 1–2)**
   - Add `catch_unwind` and pointer validation to all C FFI callbacks (`libhackrf`, `librtlsdr`, `SoapySDR`).
   - Fix preamble scan bounds (`< 290`) and clamp `correctedbits` in `dump1090`.
   - Wrap `abs()` in `validate_spec` with `checked_abs()`.
   - Replace mutex `.lock().unwrap()` with poison recovery (`unwrap_or_else`).

2. **Phase 2: Signal Processing & Protocol Correctness (Days 3–4)**
   - Implement dual-basis conversion in `lrpt-decode/src/reed_solomon.rs`.
   - Normalize path metrics in `lrpt-decode/src/viterbi.rs`.
   - Complete the second quadrature mixing stage in `ez-daemon/src/pipelines/audio.rs` for Weaver SSB.
   - Fix scanline index validation (`payload[9..11] != 0`) and vertical line calculation in `image_builder.rs`.
   - Wire CPR position decoding and Mode S message parsing into `dump1090/src/main.rs`.

3. **Phase 3: GUI & Frontend Robustness (Days 5–6)**
   - Offload file dialogs and real-time DSP from the egui UI thread.
   - Implement control channels for in-session tuning in `source_manager.rs`.
   - Fix OffscreenCanvas snapshot copying in `spectrum-worker.ts` to preserve waterfall history.
   - Initialize `AudioContext` synchronously on user gesture in `monitor.ts`.
   - Compute scanner squelch from channel/bin power instead of the global wideband peak.

4. **Phase 4: Concurrency & Hardware Driver Hardening (Days 7–8)**
   - Move sample bus receive outside the `Channelizer` mutex lock.
   - Implement graceful reconnection and differentiated timeouts in `TcpIqSource`.
   - Add connection limits to the Axum WebSocket listener.
   - Implement RAII `Drop` handlers for `hound::WavWriter` in `recorder_panel.rs`.

---
*Consolidated Audit Report Complete.*
