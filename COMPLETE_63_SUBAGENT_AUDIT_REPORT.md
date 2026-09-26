# EZ-SDR Master Production Audit: Complete 63-Subagent Swarm Report

**Project:** EZ-SDR (All-in-One Software Defined Radio Suite)  
**Workspace:** `/home/lupc/Documents/ez-sdr`  
**Date:** 2026-09-20  
**Authors:** Swarm Audit Fleet (Claude Fable 5.1 & Gemini 3.8 Flash High Specialist Subagents)  
**Overall Readiness Grade:** **C- (NOT PRODUCTION READY)**  

---

## Table of Contents
1. [Swarm Origin & Execution Context](#1-swarm-origin--execution-context)
2. [Executive Scorecard & Defect Metrics](#2-executive-scorecard--defect-metrics)
3. [The 24 Critical Showstoppers](#3-the-24-critical-showstoppers)
4. [Domain 1: DSP Pipelines & Digital Down-Conversion](#4-domain-1-dsp-pipelines--digital-down-conversion)
5. [Domain 2: LRPT Satellite Decoder (`lrpt-decode`)](#5-domain-2-lrpt-satellite-decoder-lrpt-decode)
6. [Domain 3: ADS-B Mode S & Flight Tracker (`dump1090`)](#6-domain-3-ads-b-mode-s--flight-tracker-dump1090)
7. [Domain 4: Daemon Server, Network & Concurrency (`ez-daemon`)](#7-domain-4-daemon-server-network--concurrency-ez-daemon)
8. [Domain 5: GUI Core & Real-Time Audio Chain (`ez-gui`)](#8-domain-5-gui-core--real-time-audio-chain-ez-gui)
9. [Domain 6: GUI Features, Satellite & Scanner (`ez-gui`)](#9-domain-6-gui-features-satellite--scanner-ez-gui)
10. [Domain 7: Frontend Web Application (`ez-web`)](#10-domain-7-frontend-web-application-ez-web)
11. [Domain 8: Unsafe Code, FFI & Memory Safety](#11-domain-8-unsafe-code-ffi--memory-safety)
12. [Phased Production Remediation Roadmap](#12-phased-production-remediation-roadmap)

---

## 1. Swarm Origin & Execution Context

During a massive automated production readiness evaluation of the `ez-sdr` repository, an initial Claude Fable 5.1 orchestration session was launched with a planned fleet of 63 subagents. Claude completed initial critical path and hardware driver analyses before exhausting its API quota mid-run.

The audit was resumed and completed by an Antigravity Gemini 3.8 Flash swarm. The swarm recovered state from `AGENT_SWARM_TRACKER.md` and `PRODUCTION_AUDIT_TASK.md`, divided the unfinished work across eight specialized subagents, and audited every file, algorithm, FFI binding, and UI component down to the exact line number.

Every internal unit test in the repository passes, but direct inspection revealed that these tests rely almost exclusively on synthetic, internally generated test vectors that completely bypass the physical constraints of real radio frequency (RF) hardware and transmission standards.

---

## 2. Executive Scorecard & Defect Metrics

| Subsystem / Domain | Critical | High | Medium | Low | Total Findings |
|---|:---:|:---:|:---:|:---:|:---:|
| **DSP Pipelines & Channelizer** | 1 | 6 | 6 | 1 | **14** |
| **LRPT Satellite Decoder** | 5 | 3 | 2 | 1 | **11** |
| **ADS-B & dump1090** | 5 | 6 | 3 | 2 | **16** |
| **Daemon Server & Concurrency** | 3 | 5 | 4 | 2 | **14** |
| **GUI Core & Demodulation** | 3 | 4 | 4 | 2 | **13** |
| **GUI Features & Satellite/Scanner** | 3 | 5 | 3 | 2 | **13** |
| **Frontend Web Application** | 2 | 4 | 4 | 5 | **15** |
| **Unsafe Code & C FFI Safety** | 2 | 5 | 5 | 1 | **13** |
| **TOTALS** | **24** | **38** | **31** | **16** | **109+** |

### Core Architectural Diagnosis
- **Memory Safety:** Severe vulnerabilities exist in the C FFI boundaries (`librtlsdr`, `libhackrf`, `SoapySDR`). Missing `catch_unwind` in C callback functions guarantees process aborts if Rust panics, and an asynchronous Use-After-Free exists on HackRF stream termination.
- **DSP Correctness:** The SSB demodulator is mathematically incomplete (missing its second mixing stage, causing spectral folding), the NCO phase overflows during retunes, and decimation lacks fractional resampling, causing cyclic audio buffer overflows.
- **Protocol & Decoding Fidelity:** The LRPT satellite decoder is **completely incapable of decoding real Meteor satellite passes** due to a missing CCSDS dual-basis Reed-Solomon conversion, an unnormalized Viterbi trellis that freezes after ~12 seconds, and a scanline validator that rejects all scanlines after line 0.
- **Concurreny & UI Stability:** Real-time DSP and FFT are executed directly on the egui UI thread. Opening a native file dialog freezes the UI thread, causing hardware sample buffers to overflow in 128 ms and terminating the SDR driver.

---

## 3. The 24 Critical Showstoppers

The following 24 defects represent immediate crash vectors, undefined behavior, memory corruption, or total feature failure:

1. **LRPT Missing CCSDS Dual-Basis Conversion** (`lrpt-decode/src/reed_solomon.rs:86`): Meteor M2 satellites transmit RS(255, 223) in CCSDS dual basis. The codec operates only in conventional basis. 100% of real codewords fail RS check, causing `lib.rs:339` to drop all frames.
2. **LRPT Viterbi Trellis Metric Overflow Stall** (`lrpt-decode/src/viterbi.rs:59`): `ViterbiDecoder` never normalizes metrics; at 80 kbaud, all 64 states exceed `INF = 1_000_000` within 12.5s, permanently freezing trellis decoding.
3. **LRPT FrameSync Flywheel 1-Bit Demolition** (`lrpt-decode/src/frame_sync.rs:185`): On an ASM sync error in lock state, the synchronizer drains 1 bit instead of a full frame (`CADU_BITS`), irreversibly corrupting frame alignment within 5 iterations.
4. **LRPT Scanline 1+ Rejection Bug** (`lrpt-decode/src/image_builder.rs:267`): `decode_segment` checks `payload[9] != 0 || payload[10] != 0`. In real packets, bytes 9–10 are the 16-bit scanline number. All scanlines after line 0 are discarded as corrupt.
5. **LRPT Irreversible Hypothesis Lockout** (`lrpt-decode/src/lib.rs:279`): `selected_lane` is never reset to `None` on carrier phase slip, permanently deafening the decoder for the rest of a pass.
6. **dump1090 Preamble Bounds Read Panic** (`dump1090/src/demod.rs:683`): Loop guard `< 289` permits entry when `len == 289`. In phase 8, sampling index accesses offset 289, panicking with slice index out of bounds.
7. **dump1090 Address/Parity Array Index Panic** (`dump1090/src/demod.rs:483, 962`): Surveillance replies return error code `-1`, cast to `usize::MAX`. Indexing `stats.demod_accepted[correctedbits]` panics the fixed 3-element array.
8. **dump1090 Flight Tracking Disconnected** (`dump1090/src/main.rs:301`): `mode_s::decode_mode_s` returns empty fields; `cpr::CprDecoder` is never called; coordinates, altitudes, and callsigns are never extracted or tracked.
9. **HackRF Missing `catch_unwind` Across FFI** (`dump1090/src/sdr/hackrf.rs:85`, `ez-daemon/src/hardware/hackrf.rs:101`): `extern "C" rx_callback` performs heap allocations and channel sends without `catch_unwind`. A Rust panic crossing the C boundary aborts the process.
10. **HackRF Use-After-Free on Stream Stop** (`dump1090/src/sdr/hackrf.rs:250`, `ez-daemon/src/hardware/hackrf.rs:245`): `hackrf_stop_rx` return code is ignored; `self.ctx` is immediately freed via `Box::from_raw` while in-flight USB transfer callbacks are accessing it.
11. **HackRF Callback Unchecked Pointer Dereference** (`dump1090/src/sdr/hackrf.rs:86`, `ez-daemon/src/hardware/hackrf.rs:101`): `transfer`, `ctx`, and `buffer` are dereferenced prior to null checks, causing segfaults on driver error.
12. **Weaver SSB Incomplete Demodulator Folds Pitch** (`ez-daemon/src/pipelines/audio.rs:267`): Omission of the second quadrature mixer folds spectrum across 0 Hz, inverting pitches and making voice unintelligible.
13. **Daemon Cascading Mutex Poisoning** (`ez-daemon/src/state.rs:650`, `server.rs`): DSP threads lock with `.lock().unwrap()`. A panic in audio or telemetry poisons the lock, causing any subsequent REST call to crash Axum workers.
14. **Daemon Thread Exhaustion DoS via WebSockets** (`ez-daemon/src/web/mod.rs:59`, `server.rs:384`): WebSocket listener lacks connection bounds. Opening stream sockets spawns dedicated OS threads via `.expect()`, crashing the daemon when OS limits are reached.
15. **Daemon Integer Negation Overflow Panic** (`ez-daemon/src/state.rs:161`): `spec.center_offset_hz.abs()` panics in debug and release mode if `center_offset_hz == i64::MIN`.
16. **GUI Synchronous File Dialogs Kill SDR Worker** (`ez-gui/src/spectrum.rs:665`, `source_manager.rs:306`): `rfd::FileDialog` blocks the UI thread. The sample buffer overflows in 128 ms, causing the hardware thread to terminate and close the SDR.
17. **GUI In-Session Retuning Broken in RTL-SDR** (`ez-gui/src/source_manager.rs:202, 901`): Hardware loop captures parameters by value without a command queue. Tuning sliders update UI labels while the physical tuner never retunes.
18. **GUI Real-Time DSP Synchronously on UI Thread** (`ez-gui/src/app.rs:572`): Demodulation, ADS-B slicing, and FFT run on the main thread; UI stalls starve the audio stream and cause buffer overruns.
19. **GUI Decoder Thread Panic Deadlocks Panel** (`ez-gui/src/decoding_panel.rs:138`): If `lrpt_decode` panics on a file, `tick_decode` ignores channel disconnect, leaving `self.running = true` permanently and deadlocking the UI.
20. **GUI Scanner Squelch Uses Global FFT Peak** (`ez-gui/src/app.rs:953`, `scanner.rs:706`): Squelch evaluates `peak_level()` across all 2048 FFT bins (2 MHz) rather than channel power, triggering false hits across ±1 MHz around any carrier.
21. **GUI WAV Recorder Missing Drop Finalization** (`ez-gui/src/recorder_panel.rs:19`): `RecorderPanel` lacks `impl Drop`; exiting or crashing while recording leaves WAV RIFF headers with size 0, corrupting files.
22. **Frontend Canvas Cleared Every Frame** (`ez-web/src/workers/spectrum-worker.ts:130`): `transferToImageBitmap()` resets `OffscreenCanvas` to transparent black per HTML spec. Subsequent `drawImage` copies blank pixels, erasing waterfall history.
23. **Frontend Audio Autoplay Policy Violation** (`ez-web/src/audio/monitor.ts:26`): `AudioContext` is created inside a network callback rather than the click handler, causing browsers to permanently mute audio.
24. **RTL-SDR FFI ABI Mismatch & 100% CPU Spin** (`ez-gui/src/source_manager.rs:298`): `n_read` declared as `*mut u32` instead of `*mut c_int`; return codes are ignored; disconnecting the device triggers a 100% CPU infinite loop or out-of-bounds slice panic.

---

## 4. Domain 1: DSP Pipelines & Digital Down-Conversion

### ISSUE-DSP-01 [CRITICAL]: Incomplete Weaver SSB Demodulator Folds Spectrum
- **File:** `ez-daemon/src/pipelines/audio.rs:267–292`
- **Root Cause:** In `demod_ssb`, baseband is shifted by $f_0 = 1650$ Hz and lowpass-filtered to remove the opposite sideband. However, the essential second heterodyne mixing stage ($\cos(2\pi f_0 t)$ and $\sin(2\pi f_0 t)$) is omitted. Direct extraction of `2.0 * filtered.re` takes the real part while still at DC, folding positive and negative baseband frequencies across 0 Hz.
- **Impact:** USB/LSB voice speech is completely unintelligible. Pitch is inverted: higher voice pitches become low rumbles and lower voice pitches become high whines.
- **Fix:** Complete Weaver's second oscillator stage: $y(t) = 2 \cdot [I_{filt}(t)\cos(2\pi f_0 t) \mp Q_{filt}(t)\sin(2\pi f_0 t)]$, or replace with an analytic Hilbert filter.

### ISSUE-DSP-02 [HIGH]: Unbounded NCO Phase Growth and Overflow on Retuning
- **File:** `ez-daemon/src/channelizer.rs:49–54, 380–386`
- **Root Cause:** `Nco::mix` wraps `self.phase` using single-step subtraction (`if phase > PI { phase -= 2*PI }`). When retuning the hardware across bands, $|offset\_hz| > wideband\_rate$ produces $|phase\_increment| > 2\pi$. On every sample, `phase` increases by $|phase\_increment| - 2\pi$.
- **Impact:** `phase` grows into the millions, loses precision, overflows to `±Inf`, and produces `NaN`s in `Complex32::from_polar`. Downstream audio and telemetry pipelines receive continuous `NaN`s.
- **Fix:** Normalize `phase_increment` to $(-\pi, \pi]$ on retuning, and use `(phase + PI).rem_euclid(2.0 * PI) - PI`.

### ISSUE-DSP-03 [HIGH]: 289-Sample ADS-B Preamble Dead Zone at Block Boundaries
- **File:** `ez-daemon/src/pipelines/packet.rs:132–171`, `dump1090/src/demod.rs:680–686`
- **Root Cause:** `PacketPipeline` prepends an overlap tail of 320 samples (`mag.overlap`). `dump1090::Demod2400::demodulate` starts its preamble loop at `j = last_message_end.max(mag.overlap)`, skipping the overlap entirely. At the end of the block, it breaks 289 samples early. The 289-sample tail of block $k$ is never scanned in either block $k$ or $k+1$.
- **Impact:** ~3.5% of the continuous RF timeline is an invisible dead zone where preambles are never detected.
- **Fix:** Scan preambles into the overlap region: `j = self.last_message_end.min(mag.overlap)`.

### ISSUE-DSP-04 [HIGH]: Rail-Clipping in AM Demodulation from Unnormalized Samples
- **File:** `ez-daemon/src/pipelines/audio.rs:212–216, 225–236`
- **Root Cause:** RTL-SDR and HackRF output samples in range $[-128, 128]$. In `demod_am`, envelope is computed as `s.norm()` ($\sim 180$) and passed directly into `y.clamp(-1.0, 1.0)`.
- **Impact:** AM audio is hard-clipped into severe square-wave distortion. Squelch thresholds calibrated in dBFS fail by 46 dB depending on whether RTL-SDR or SoapySDR is used.
- **Fix:** Normalize all input `Complex32` streams to $[-1.0, 1.0]$ upon ingest and implement audio AGC.

### ISSUE-DSP-05 [HIGH]: Permanent NaN/Inf Latch-Up in IIR Filters
- **File:** `ez-daemon/src/pipelines/audio.rs:225–236, 255–265`
- **Root Cause:** AM DC blocker and WFM de-emphasis are single-pole recursive IIR filters. A single `NaN` or `Inf` from hardware disconnect poisons `dc_prev_y` or `deemph_prev` permanently.
- **Impact:** Audio stream latches into permanent silence or emits continuous NaNs until manually switching modes.
- **Fix:** Guard filter state with `.is_finite()`, resetting to 0.0 on non-finite values.

### ISSUE-DSP-06 [HIGH]: Channelizer Monotonic Counter Masks Dropped Samples
- **File:** `ez-daemon/src/channelizer.rs:198–206`
- **Root Cause:** `VirtualChannel::process` stamps outgoing blocks using a local counter, ignoring incoming `block.start_sample`. Dropped blocks do not increment NCO phase or reset FIR history.
- **Impact:** Downstream packet continuity detection assumes zero lost samples, corrupting decoder timelines.
- **Fix:** Advance NCO phase and reset FIR history when gaps in `start_sample` are detected.

### ISSUE-DSP-07 [MEDIUM]: Inadequate 63-Tap FIR Filter at High Decimations
- **File:** `ez-daemon/src/channelizer.rs:18, 144–153`
- **Root Cause:** `FIR_TAPS` is fixed at 63 taps regardless of decimation. At $D = 50$, transition width is wider than the cutoff, failing to attenuate alias bands.
- **Impact:** Severe high-frequency aliasing folds into decimated audio baseband.
- **Fix:** Scale filter taps proportionally to decimation ($N \propto D$) or use cascaded decimation stages.

### ISSUE-DSP-08 [MEDIUM]: Missing Audio Resampling to Standard 48 kHz
- **File:** `ez-daemon/src/pipelines/audio.rs:218–222`
- **Root Cause:** Publishes `AudioFrame` at arbitrary channelizer rates (e.g. 48,761 Hz or 200 kHz).
- **Impact:** Web Audio API crashes or drifts, causing audio popping and pitch errors.
- **Fix:** Embed a polyphase resampler converting output to standard 48 kHz PCM.

---

## 5. Domain 2: LRPT Satellite Decoder (`lrpt-decode`)

### ISSUE-LRPT-01 [CRITICAL]: Missing CCSDS Dual-Basis Conversion in Reed-Solomon Codec
- **File:** `lrpt-decode/src/reed_solomon.rs:86–92, 195–291`
- **Root Cause:** CCSDS 131.0-B §4.3 mandates dual-basis representation for RS(255, 223) when concatenated with convolutional coding. `reed_solomon.rs` assumes conventional basis throughout.
- **Impact:** Syndromes computed over authentic Meteor-M2-3/4 captures are completely scrambled. Every single codeword fails correction, dropping 100% of received satellite frames.
- **Fix:** Add dual-basis to conventional basis conversion table before syndrome generation.

### ISSUE-LRPT-02 [CRITICAL]: Unbounded Trellis Metric Growth Freezes Viterbi Decoder
- **File:** `lrpt-decode/src/viterbi.rs:59–74`
- **Root Cause:** Path metrics are monotonically accumulated without normalization. At 80 kbaud, all 64 states reach `INF = 1_000_000` in ~12.5 seconds. Once at `INF`, line 62 skips all states.
- **Impact:** Viterbi decoder dies after 12.5 seconds of acquisition, outputting all-zero junk for the rest of the pass.
- **Fix:** Normalize path metrics after each step by subtracting `min_metric`.

### ISSUE-LRPT-03 [CRITICAL]: FrameSync Flywheel Drains 1 Bit on Marker Miss
- **File:** `lrpt-decode/src/frame_sync.rs:185–194`
- **Root Cause:** In lock mode, if an ASM marker fails threshold, the synchronizer drains 1 bit instead of a full frame (`CADU_BITS`).
- **Impact:** A single bit error drops lock within 5 bits and misaligns the entire bit buffer by 5 bits.
- **Fix:** In `LOCK` state, advance by `CADU_BITS` on sync error and only drop to `SEARCH` after 5 consecutive missed frames.

### ISSUE-LRPT-04 [CRITICAL]: Scanline 1+ Rejection in ImageBuilder
- **File:** `lrpt-decode/src/image_builder.rs:267–270`
- **Root Cause:** Checks `payload[9] != 0 || payload[10] != 0`. In Meteor MSU-MR packets, bytes 9–10 are the 16-bit scanline number.
- **Impact:** Only scanline 0 is accepted. Scanlines 1 through 65,535 are discarded as corrupt.
- **Fix:** Parse `u16::from_be_bytes([payload[9], payload[10]])` as the line index.

### ISSUE-LRPT-05 [CRITICAL]: Irreversible Hypothesis Lockout
- **File:** `lrpt-decode/src/lib.rs:279–285`
- **Root Cause:** `self.selected_lane` is set on the first frame and never cleared to `None`.
- **Impact:** Fades or phase slips break synchronization permanently; other lanes are never checked again.
- **Fix:** Reset `selected_lane = None` after 5 consecutive frame misses.

### ISSUE-LRPT-06 [HIGH]: Corrupted Image Scanline Divisor (`/ 43`)
- **File:** `lrpt-decode/src/image_builder.rs:71–84`
- **Root Cause:** Line index is calculated as `sequence / 43`. Each channel contains exactly 14 segments per line.
- **Impact:** Consecutive scanlines overwrite each other, compressing images vertically to 1/3 height.
- **Fix:** Divide per-APID sequence by 14 (`SEGMENTS_PER_LINE`).

### ISSUE-LRPT-07 [HIGH]: Hard-Decision Viterbi Discards 2.5–3 dB Coding Gain
- **File:** `lrpt-decode/src/viterbi.rs:23–27`, `lib.rs:187`
- **Root Cause:** Slices complex IQ to hard 0/1 bits before trellis decoding.
- **Impact:** 2.5–3 dB penalty overwhelms RS correction on marginal VHF satellite signals.
- **Fix:** Retain 8-bit soft decision metrics from Costas loop and use soft branch metrics.

---

## 6. Domain 3: ADS-B Mode S & Flight Tracker (`dump1090`)

### ISSUE-ADSB-01 [CRITICAL]: Insufficient Preamble Bounds Check Panic
- **File:** `dump1090/src/demod.rs:683–686, 826, 854`
- **Root Cause:** Guard check `preamble.len() < 289` permits entry when `len == 289`. In phase 8, sampling index reaches offset 289 (`20 + 269 = 289`), which panics on a slice of length 289 (`0..288`).
- **Impact:** The demodulator crashes with an out-of-bounds index panic whenever an ADS-B preamble arrives 289 samples from the buffer boundary.
- **Fix:** Change guard check to `if preamble.len() < 290`.

### ISSUE-ADSB-02 [CRITICAL]: Address/Parity Frames Set `correctedbits = usize::MAX`
- **File:** `dump1090/src/demod.rs:481–484, 962`
- **Root Cause:** Surveillance replies (DF0, DF4, DF5) have non-zero syndrome (the address) and return `-1`. Line 483 executes `mm.correctedbits = corrections as usize`, producing `usize::MAX`. Line 962 indexes `stats.demod_accepted[decoded_mm.correctedbits]`.
- **Impact:** Fixed array `[u64; 3]` panics immediately on the first accepted surveillance reply.
- **Fix:** Clamp: `mm.correctedbits = corrections.max(0) as usize;` and guard array access.

### ISSUE-ADSB-03 [CRITICAL]: Complete Disconnection of Flight Tracking in `main.rs`
- **File:** `dump1090/src/main.rs:301–306`, `dump1090/src/mode_s.rs:215–225`
- **Root Cause:** `decode_mode_s` is a dummy function returning empty fields. `cpr::CprDecoder` is never called. `Tracker` only stores packet counts and addresses.
- **Impact:** Standalone `dump1090` cannot display or decode any aircraft coordinates, altitudes, callsigns, or speeds.
- **Fix:** Wire `cpr::CprDecoder` and `decode_mode_s_message` into `main.rs`.

### ISSUE-ADSB-04 [HIGH]: Backward Time Clock Panic and Stale Pairing in CPR
- **File:** `dump1090/src/cpr.rs:329, 359, 446`
- **Root Cause:** `now.duration_since(even.time)` panics on backward time adjustments. `saturating_duration_since` returns 0.0s when `now < even.time`, erroneously pairing stale frames.
- **Impact:** Coordinate corruption during replay or out-of-order packet arrival.
- **Fix:** Use bidirectional absolute time difference `let dt = (now - even.time).abs()`.

### ISSUE-ADSB-05 [HIGH]: Catastrophic CPR Cache Wipeout Under Load
- **File:** `dump1090/src/cpr.rs:341–347`
- **Root Cause:** When `cache.len() >= 2048`, if pruning does not reduce size, `cache.clear()` wipes all entries.
- **Impact:** Every aircraft in the airspace loses its even/odd pairing baseline simultaneously.
- **Fix:** Replace with an LRU cache that evicts only the single oldest entry.

### ISSUE-ADSB-06 [HIGH]: 4096-bit ICAO Filter Saturation and 60s Flush
- **File:** `dump1090/src/icao_filter.rs:38–42, 76–78`
- **Root Cause:** Filter is a 4096-bit bitset that clears itself every 60s or after 8192 insertions.
- **Impact:** When cleared, all known aircraft become unknown, causing surveillance replies to be rejected.
- **Fix:** Implement rotating generational bitsets instead of instantaneous total flushes.

---

## 7. Domain 4: Daemon Server, Network & Concurrency (`ez-daemon`)

### ISSUE-NET-01 [CRITICAL]: Cascading Mutex Poisoning on Pipeline Panic
- **File:** `ez-daemon/src/state.rs:650`, `server.rs`
- **Root Cause:** DSP tick threads lock with `.lock().unwrap()`. A panic poisons the lock. Subsequent calls to `/api/channels` or `/api/status` call `.unwrap()` on the poisoned mutex.
- **Impact:** A single pipeline error crashes the entire daemon process and drops all network clients.
- **Fix:** Catch panics or recover with `lock().unwrap_or_else(|p| p.into_inner())`.

### ISSUE-NET-02 [CRITICAL]: Unbounded WebSocket Spawning & Thread Exhaustion DoS
- **File:** `ez-daemon/src/web/mod.rs:59–62`, `server.rs:384–387`
- **Root Cause:** `web::serve` accepts unlimited connections. Each stream connection spawns an OS thread via `.expect("spawning forwarder thread")`.
- **Impact:** Hitting OS thread limits (`ulimit -u`) triggers an unhandled panic, crashing the daemon.
- **Fix:** Add a connection semaphore limit and transition forwarders to Tokio tasks.

### ISSUE-NET-03 [CRITICAL]: Integer Negation Overflow Panic on `i64::MIN`
- **File:** `ez-daemon/src/state.rs:161`
- **Root Cause:** `validate_spec` calls `spec.center_offset_hz.abs()`. `abs()` on `i64::MIN` overflows and panics.
- **Impact:** Submitting `center_offset_hz = -9223372036854775808` crashes the request handler and daemon.
- **Fix:** Use `checked_abs()` or bounds validation before arithmetic.

### ISSUE-NET-04 [HIGH]: Channelizer Mutex Starvation During Ingest Timeout
- **File:** `ez-daemon/src/state.rs:129`, `channelizer.rs:403–413`
- **Root Cause:** `Channelizer::tick` holds the channelizer mutex while calling `recv_timeout(100ms)`.
- **Impact:** Ingestion stalls starve control commands (`set_frequency`, `subscribe`), causing multi-hundred-millisecond UI freezes.
- **Fix:** Receive from the sample bus before acquiring the channelizer lock.

### ISSUE-NET-05 [HIGH]: Missing Origin Validation Enables CSWSH
- **File:** `ez-daemon/src/web/ws.rs:42–46`
- **Root Cause:** WebSocket routes perform no verification of the HTTP `Origin` header.
- **Impact:** Malicious websites can hijack local daemons, retune frequencies, or exfiltrate demodulated audio.
- **Fix:** Validate the `Origin` header against localhost and configured domains.

### ISSUE-NET-06 [HIGH]: Fatal Ingestion Thread Termination on 5s TCP Timeout
- **File:** `ez-daemon/src/hardware/tcp_iq.rs:71`, `ingest.rs:164`
- **Root Cause:** Socket read timeout is 5 seconds; any timeout propagates via `?` and breaks the ingestion loop.
- **Impact:** Momentary network hiccups permanently terminate the hardware ingest thread.
- **Fix:** Differentiate transient timeouts (`WouldBlock`/`TimedOut`) and implement an auto-reconnect loop.

---

## 8. Domain 5: GUI Core & Real-Time Audio Chain (`ez-gui`)

### ISSUE-GUI-01 [CRITICAL]: Synchronous File Dialogs Freeze UI and Kill SDR
- **File:** `ez-gui/src/spectrum.rs:665–670, 701–706`
- **Root Cause:** `rfd::FileDialog::save_file` is called synchronously on the UI thread. The 32-capacity bounded sample channel overflows in 128 ms, causing `source_manager.rs:306` to terminate the worker thread and close the SDR device.
- **Impact:** Opening a save file dialog for $>0.13$ seconds crashes the SDR receiver.
- **Fix:** Offload file dialogs to a background thread and handle channel overflow gracefully.

### ISSUE-GUI-02 [CRITICAL]: Live Tuning Missing in Local RTL-SDR Mode
- **File:** `ez-gui/src/source_manager.rs:202–315, 901–950`
- **Root Cause:** The RTL-SDR worker thread captures parameters by value and lacks a command queue. `rtlsdr_set_center_freq` is only called once at startup.
- **Impact:** Frequency and gain sliders update only GUI labels while the physical tuner never retunes.
- **Fix:** Add an `mpsc` control channel to the hardware thread to apply tuning commands dynamically.

### ISSUE-GUI-03 [CRITICAL]: Real-Time DSP and FFT Run Synchronously on UI Thread
- **File:** `ez-gui/src/app.rs:572–633`
- **Root Cause:** `CentralApp::logic` executes demodulation, ADS-B decoding, and FFT directly on the UI frame.
- **Impact:** UI rendering lag starves the CPAL audio stream, and minimizing the window kills sample ingestion.
- **Fix:** Move DSP and FFT into a dedicated worker thread decoupled from egui rendering.

### ISSUE-GUI-04 [HIGH]: Audio Sample Rate Drift & Cyclic Buffer Overflows
- **File:** `ez-gui/src/demod.rs:274–283`, `audio_output.rs:67`
- **Root Cause:** Decimation is computed using integer division (`2048000 / 48000 = 42`), yielding 48,761.9 Hz output. The ring buffer accumulates +762 samples/s, overflowing in ~126 seconds.
- **Impact:** Periodic cyclic audio popping, clicks, and buffer dropouts every two minutes.
- **Fix:** Implement a fractional polynomial resampler to convert output to exact host audio rates.

### ISSUE-GUI-05 [HIGH]: Zombie Daemon Connection Clobbers UI Status
- **File:** `ez-gui/src/sdr_panel.rs:268`, `app.rs:462`
- **Root Cause:** Switching source mode does not stop the previous source. `recv_daemon_event()` continues running and overwrites `self.status` with daemon status while local hardware is active.
- **Impact:** UI reports "Idle" while local SDR hardware is actively streaming.
- **Fix:** Trigger `source.stop()` on mode change and guard daemon polling by active mode.

---

## 9. Domain 6: GUI Features, Satellite & Scanner (`ez-gui`)

### ISSUE-FEAT-01 [CRITICAL]: Scanner Squelch Uses Global FFT Peak
- **File:** `ez-gui/src/app.rs:953–964`, `scanner.rs:706–710`
- **Root Cause:** Squelch evaluates `state.spectrum.peak_level()` across all 2048 FFT bins (2 MHz wide) rather than bin power at `current_freq_hz`.
- **Impact:** A single strong station causes every scanned step within ±1 MHz to falsely trigger as an active signal.
- **Fix:** Measure channel power within filter bandwidth centered at `current_freq_hz`.

### ISSUE-FEAT-02 [CRITICAL]: Decoder Thread Panic Deadlocks UI Permanently
- **File:** `ez-gui/src/decoding_panel.rs:138–146, 158–188`
- **Root Cause:** If `lrpt_decode` panics on a corrupt file, `done_tx` drops. `tick_decode` ignores `TryRecvError::Disconnected`, leaving `self.running = true`.
- **Impact:** UI spinner runs forever; clicking "Decode" is blocked indefinitely until application restart.
- **Fix:** Handle channel disconnection, capture thread panic, and reset `self.running = false`.

### ISSUE-FEAT-03 [CRITICAL]: WAV Recorder Missing Drop Finalization
- **File:** `ez-gui/src/recorder_panel.rs:19–20, 451–485`
- **Root Cause:** `hound::WavWriter` writes chunk sizes only on `finalize()`. `RecorderPanel` lacks `Drop`.
- **Impact:** Closing the app or crashing leaves WAV RIFF chunk sizes set to 0, corrupting recordings.
- **Fix:** Implement `Drop for RecorderPanel` to finalize active writers.

### ISSUE-FEAT-04 [HIGH]: Scanner MQTT/Discord Alerts Silenced After 200 Hits
- **File:** `ez-gui/src/app.rs:963–971`, `scanner.rs:737–740`
- **Root Cause:** App checks `if hits.len() > prev_hits`. Once `hits.len()` reaches `max_hits = 200`, `hits.remove(0)` keeps length constant.
- **Impact:** Automated scanning silences all notifications after the first 200 hits.
- **Fix:** Maintain a monotonic `total_hits_logged: u64` or dedicated event channel.

### ISSUE-FEAT-05 [HIGH]: TLE Importer Hardcoded 3-Satellite Whitelist
- **File:** `ez-gui/src/tle_engine.rs:112–175`
- **Root Cause:** `update_tles_from_text` strictly filters parsed satellites against Meteor-M2-3, Meteor-M2-4, and ISS NORAD IDs, and aborts on any single malformed line.
- **Impact:** Tracking NOAA, Tiangong, or amateur cubesats is impossible; Celestrak files fail to import.
- **Fix:** Skip malformed lines and append all valid satellites to the catalog.

---

## 10. Domain 7: Frontend Web Application (`ez-web`)

### ISSUE-WEB-01 [CRITICAL]: `transferToImageBitmap()` Erases Waterfall History
- **File:** `ez-web/src/workers/spectrum-worker.ts:130–133`
- **Root Cause:** Per the WHATWG spec, `transferToImageBitmap()` empties the source `OffscreenCanvas`. The subsequent `drawImage` call to scroll the waterfall copies transparent black pixels.
- **Impact:** Waterfall history never accumulates; display remains blank black except for the top single row.
- **Fix:** Maintain an independent offscreen canvas or circular pixel buffer for waterfall history.

### ISSUE-WEB-02 [CRITICAL]: `AudioContext` Violates Browser Autoplay Policy
- **File:** `ez-web/src/audio/monitor.ts:26–41`
- **Root Cause:** `AudioContext` creation and resumption are deferred until the first network audio frame arrives, losing the user gesture token.
- **Impact:** Browsers refuse to start the context; audio remains permanently muted.
- **Fix:** Create or resume `AudioContext` synchronously within the user click event handler.

### ISSUE-WEB-03 [HIGH]: `StreamSocket` Reconnect Permanently Aborts After 5 Attempts
- **File:** `ez-web/src/api/stream.ts:15, 48–58`
- **Root Cause:** If 5 reconnection attempts fail sequentially, `this.closed = true` terminates all retries.
- **Impact:** Any daemon restart or transient network blip exceeding 8s permanently freezes the web UI.
- **Fix:** Continue retrying with exponential backoff indefinitely while displaying a reconnecting banner.

### ISSUE-WEB-04 [HIGH]: 2,000,000-Iteration Scalar Pixel Loop Freezes UI
- **File:** `ez-web/src/ui/telemetry-panel.ts:50–87`
- **Root Cause:** On every telemetry packet, a scalar loop executes 2M iterations on the main UI thread to unpack RGBA pixels and resets `images.innerHTML = ""`.
- **Impact:** Freezes the browser for 50–150 ms per packet, stalling waterfall animation.
- **Fix:** Reuse canvas elements and unpack grayscale bytes using a `Uint32Array` view.

---

## 11. Domain 8: Unsafe Code, FFI & Memory Safety

### Master Unsafe Audit Metrics
- **Total `unsafe` blocks audited:** **131** (`dump1090`: 68, `ez-daemon`: 49, `ez-gui`: 14, `lrpt-decode`: 0, `ez-proto`: 0).

### ISSUE-SEC-01 [CRITICAL]: Unsafe Callback Dereference Without Null Check
- **File:** `dump1090/src/sdr/hackrf.rs:85–95`, `ez-daemon/src/hardware/hackrf.rs:101–113`
- **Root Cause:** `transfer`, `(*transfer).ctx`, and `(*transfer).buffer` are dereferenced prior to null checks.
- **Impact:** Segfaults on driver cancellation or USB error.
- **Fix:** Validate all pointers are non-null before dereferencing.

### ISSUE-SEC-02 [CRITICAL]: Missing `catch_unwind` Across C FFI Boundaries
- **File:** `dump1090/src/sdr/hackrf.rs:85–102`, `ez-daemon/src/hardware/hackrf.rs:101–120`
- **Root Cause:** `extern "C" rx_callback` performs heap allocations and channel sends without `catch_unwind`.
- **Impact:** Panics crossing the C ABI boundary cause undefined behavior and abort the entire process.
- **Fix:** Wrap callback execution in `std::panic::catch_unwind`.

### ISSUE-SEC-03 [CRITICAL]: Use-After-Free Race on HackRF Teardown
- **File:** `dump1090/src/sdr/hackrf.rs:246–264`, `ez-daemon/src/hardware/hackrf.rs:245–266`
- **Root Cause:** `self.ctx` is freed via `Box::from_raw` immediately after calling `hackrf_stop_rx`, while transfer callbacks are still in flight.
- **Impact:** Memory corruption and crashes during device teardown.
- **Fix:** Manage `RxCtx` via `Arc<RxCtx>` rather than raw pointers.

### ISSUE-SEC-04 [HIGH]: Unchecked `n_read` Slice Index Panic in RTL-SDR
- **File:** `ez-daemon/src/hardware/rtlsdr.rs:277–285`
- **Root Cause:** `n_read as usize` is not checked for negative values and not clamped to buffer capacity.
- **Impact:** Out-of-bounds slice indexing panic on USB error.
- **Fix:** Clamp `bytes_read = (n_read as usize).min(self.read_buf.len())`.

### ISSUE-SEC-05 [HIGH]: C FFI Struct ABI Size Mismatch in HackRF
- **File:** `dump1090/src/sdr/hackrf.rs:20–27`, `ez-daemon/src/hardware/hackrf.rs:62–69`
- **Root Cause:** `HackrfTransfer` in Rust has 5 fields (32 bytes); C `hackrf_transfer` has 6 fields (40 bytes), omitting `tx_ctx`.
- **Impact:** Stride and offset mismatch on struct pointer arithmetic.
- **Fix:** Add `pub tx_ctx: *mut c_void` to match C header.

---

## 12. Phased Production Remediation Roadmap

```
+-------------------------------------------------------------------------+
|                  EZ-SDR PRODUCTION READINESS ROADMAP                     |
+-------------------------------------------------------------------------+
| Phase 1: Memory Safety & Crash Elimination (Days 1–2)                  |
| - Add catch_unwind to C FFI callbacks                                   |
| - Fix dump1090 preamble bounds (< 290) and clamp correctedbits          |
| - Wrap spec.center_offset_hz in checked_abs                             |
| - Replace mutex unwrap() with poison recovery across daemon             |
+-------------------------------------------------------------------------+
                                    |
                                    v
+-------------------------------------------------------------------------+
| Phase 2: Core DSP & Protocol Fidelity (Days 3–4)                       |
| - Implement CCSDS dual-basis conversion in lrpt-decode                  |
| - Normalize path metrics in Viterbi decoder                             |
| - Complete Weaver second mixer stage in audio pipeline                  |
| - Fix scanline index validation (payload[9..11]) in image builder       |
| - Connect CPR position decoding and Mode S parsing in dump1090 main.rs  |
+-------------------------------------------------------------------------+
                                    |
                                    v
+-------------------------------------------------------------------------+
| Phase 3: GUI & Frontend Stabilization (Days 5–6)                       |
| - Offload file dialogs and real-time DSP from egui UI thread            |
| - Add control queue for in-session tuning in RTL-SDR mode               |
| - Preserve waterfall canvas history in spectrum-worker.ts               |
| - Create AudioContext synchronously on user click in monitor.ts         |
| - Compute scanner squelch from channel power instead of global peak     |
+-------------------------------------------------------------------------+
                                    |
                                    v
+-------------------------------------------------------------------------+
| Phase 4: Hardware & Network Hardening (Days 7–8)                        |
| - Move sample bus receive outside Channelizer mutex                     |
| - Add connection semaphore limits to Axum WebSocket server              |
| - Implement graceful reconnection and timeouts in TcpIqSource           |
| - Add Drop finalizer to hound::WavWriter in recorder_panel.rs           |
+-------------------------------------------------------------------------+
```

---
*End of Master Production Audit Report.*
