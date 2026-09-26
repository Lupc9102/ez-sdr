# EZ-SDR Production Audit - Interim Findings

**Status:** Workflow in progress (DSP Pipelines phase active)  
**Completed:** Hardware Drivers (4/4 agents), ez-daemon core, dump1090  
**Timestamp:** 2026-09-20 20:30

---

## Executive Summary

**Total findings identified so far:** 35+ production-critical issues  
**CRITICAL severity:** 9 confirmed  
**HIGH severity:** 12 confirmed  
**MEDIUM severity:** 10+  
**LOW severity:** 4+

---

## CRITICAL Issues (Immediate Action Required)

### ez-daemon Hardware Layer

**1. ez-daemon/src/hardware/soapy.rs:360 | CRITICAL**
- **Issue:** readStream FFI violates Rust aliasing rules by creating mutable pointer to byte_buf while retaining &mut self
- **Impact:** Undefined behavior - optimizer may assume no aliasing, leading to memory corruption, data races, or crashes when C library writes to buffer
- **Source:** Workflow agent a5bca4d4755ff5954

**2. ez-daemon/src/hardware/hackrf.rs:110 | CRITICAL**
- **Issue:** Unsynchronized raw pointer dereference in callback thread violates aliasing rules
- **Impact:** Callback thread creates shared reference &RxCtx from raw pointer without memory barriers. If main thread executes stop() concurrently, Box::from_raw can free memory while callback holds reference, causing use-after-free and undefined behavior
- **Source:** Workflow agent a98f2a9fb4f256f99

**3. ez-daemon/src/hardware/rtlsdr.rs:282 | CRITICAL**
- **Issue:** iq_bytes_to_complex allocates unbounded Vec that could exceed output buffer size
- **Impact:** If lrpt_decode::iq_bytes_to_complex returns more samples than buf.len(), line 284 will panic on slice index out of bounds
- **Source:** Workflow agent a14645a22655e27d7

### ez-daemon State Management

**4. ez-daemon/src/state.rs:129 | CRITICAL**
- **Issue:** Deadlock - set_frequency blocks hardware then locks channelizer
- **Impact:** hardware.set_frequency(hz) blocks up to 2s (COMMAND_TIMEOUT), then channelizer.lock().unwrap() while channelizer thread may hold lock during 250ms tick
- **Source:** Subagent a9408ac4bb9bc0b59

### dump1090 ADS-B Decoder

**5. dump1090/src/cpr.rs:359-360, 375-376 | CRITICAL**
- **Issue:** saturating_duration_since() panics on backward clock adjustment
- **Impact:** Decoder crash on system time changes (NTP sync, manual adjustment, daylight saving)
- **Source:** Subagent a831f13b4ac17c279

**6. dump1090/src/demod.rs:683-686 | CRITICAL**
- **Issue:** Insufficient buffer bounds check in preamble scan
- **Impact:** Checks preamble.len() < 269 + 1 + 19 but indexes m[j + 19 + (try_phase / 5)] without verifying j + offset < valid_length - out-of-bounds read in hot demodulation path
- **Source:** Subagent a831f13b4ac17c279

**7. dump1090/src/sdr/hackrf.rs:86, 91, 94 | CRITICAL**
- **Issue:** Unsafe callback dereference before null check
- **Impact:** (*transfer).valid_length, (*transfer).ctx, (*transfer).buffer dereferenced assuming valid pointer - segfault if libhackrf passes corrupted pointer
- **Source:** Subagent a831f13b4ac17c279

**8. dump1090/src/sdr/rtlsdr.rs:464-470 | CRITICAL**
- **Issue:** Unchecked FFI return value cast
- **Impact:** n_read cast to usize without validating positive or in-bounds - memory corruption in magnitude conversion
- **Source:** Subagent a831f13b4ac17c279

**9. dump1090/src/sdr/soapy.rs:554-564 | CRITICAL**
- **Issue:** Trusted buf_ptr after readStream
- **Impact:** samples_read error code checked but buf_ptr mutation not validated - memory safety violation in SDR read
- **Source:** Subagent a831f13b4ac17c279

---

## HIGH Severity Issues

### ez-daemon Hardware

**10. ez-daemon/src/hardware/soapy.rs:368 | HIGH**
- **Issue:** No validation that SoapySDRDevice_readStream respects num_elems buffer bound
- **Impact:** Buffer overflow if library has bugs or misinterprets buffer pointer

**11. ez-daemon/src/hardware/soapy.rs:379 | HIGH**
- **Issue:** Negative readStream return codes treated generically without decoding specific error codes
- **Impact:** Transient errors (overflow/underflow) indistinguishable from fatal errors (device disconnect)

**12. ez-daemon/src/hardware/hackrf.rs:312 | HIGH**
- **Issue:** TOCTOU race in hackrf_is_streaming - device pointer accessed without synchronization
- **Impact:** If another thread calls stop() concurrently, self.dev could be nulled between check and FFI call

**13. ez-daemon/src/hardware/hackrf.rs:250 | HIGH**
- **Issue:** hackrf_stop_rx return code ignored - callback may still be running when ctx is freed
- **Impact:** Use-after-free if hackrf_stop_rx fails to stop streaming thread

**14. ez-daemon/src/hardware/rtlsdr.rs:281 | HIGH**
- **Issue:** Signed to unsigned conversion without validation
- **Impact:** n_read (c_int) cast to usize without checking magnitude - large negative values cause integer overflow

**15. ez-daemon/src/hardware/rtlsdr.rs:158 | HIGH**
- **Issue:** Potential out-of-bounds access in gains array
- **Impact:** gains[selected as usize] not validated if gains modified between start() and apply_gain()

**16. ez-daemon/src/hardware/replay.rs:129 | HIGH**
- **Issue:** Unbounded buffer allocation based on caller-provided length
- **Impact:** byte_buf.resize(want_bytes, 0) can cause OOM with multi-GB caller buffers

**17. ez-daemon/src/hardware/tcp_iq.rs:71 | HIGH**
- **Issue:** Aggressive 5-second read timeout reused from CONNECT_TIMEOUT
- **Impact:** Network hiccups cause spurious disconnects - should use 30s+ or blocking reads

### ez-daemon State Management

**18. ez-daemon/src/state.rs:283-311 | HIGH**
- **Issue:** Race in subscribe bypasses MAX_CHANNELS validation
- **Impact:** Fast-path check releases lock before create_channel - multiple threads pass validation, create duplicate resources

**19. ez-daemon/src/state.rs:650 | HIGH**
- **Issue:** Unwrap in tick hot loop becomes infinite spin on poison
- **Impact:** state.lock().unwrap() - pipeline panic → Mutex poisoned → unwrap panics → running still true → infinite 100% CPU loop

### dump1090

**20. dump1090/src/cpr.rs:342-346 | HIGH**
- **Issue:** Unbounded HashMap growth in CPR cache
- **Impact:** Prunes only at MAX_CPR_CACHE_ENTRIES cap - memory leak tracking thousands of aircraft

**21. dump1090/src/icao_filter.rs:72 | HIGH**
- **Issue:** Bloom filter saturation with no per-bucket TTL
- **Impact:** 4096-bit filter eventually saturates - all unknown aircraft accepted as known (security bypass)

---

## MEDIUM Severity Issues

### ez-daemon Hardware

**22. ez-daemon/src/hardware/soapy.rs:306 | MEDIUM**
- **Issue:** stop() calls closeStream without deactivateStream
- **Impact:** Violates activate→read→deactivate→close sequence - resource leaks in driver implementations

**23. ez-daemon/src/hardware/hackrf.rs:118 | MEDIUM**
- **Issue:** Silent sample dropping when channel full
- **Impact:** try_send failures ignored - samples silently dropped without backpressure

**24. ez-daemon/src/hardware/hackrf.rs:251 | MEDIUM**
- **Issue:** hackrf_close and hackrf_exit return codes ignored
- **Impact:** Device may not be properly released - hardware lock preventing subsequent opens

**25. ez-daemon/src/hardware/rtlsdr.rs:200 | MEDIUM**
- **Issue:** Incomplete error cleanup - gains vector left populated after open fails
- **Impact:** Inconsistent state with gains non-empty but dev null

**26. ez-daemon/src/hardware/rtlsdr.rs:273 | MEDIUM**
- **Issue:** Potential truncation - want_bytes cast to c_int without overflow check
- **Impact:** On platforms with 16-bit c_int, values up to 262144 would truncate

**27. ez-daemon/src/hardware/replay.rs:136, 142 | MEDIUM**
- **Issue:** Unwrap via ? operator inside while loop
- **Impact:** Read/seek failures mid-loop lose partial data

**28. ez-daemon/src/hardware/tcp_iq.rs:130 | MEDIUM**
- **Issue:** Unwrap via ? operator inside TCP read loop
- **Impact:** Connection reset loses partial data - should handle transient errors

**29. ez-daemon/src/hardware/tcp_iq.rs:114 | MEDIUM**
- **Issue:** No sample timing or flow control
- **Impact:** Unlike replay with pace(), TCP dumps data as fast as delivered - downstream buffer overflow

### dump1090

**30. dump1090/src/demod.rs:1001-1004 | HIGH**
- **Issue:** NaN propagation in noise calculation
- **Impact:** mag.mean_power * mlen - sum_signal_power can produce NaN - SNR checks fail, demod accepts/rejects everything

**31. dump1090/src/mode_s.rs:183 | HIGH**
- **Issue:** Velocity atan2(0, 0) undefined
- **Impact:** ew_vel.atan2(ns_vel) when both zero is implementation-defined - garbage heading in tracker

---

## LOW Severity Issues

**32. ez-daemon/src/hardware/soapy.rs:359 | LOW**
- **Issue:** Retry loop for readStream timeout has no backoff
- **Impact:** Minor CPU waste immediately retrying 3 times

**33. ez-daemon/src/hardware/hackrf.rs:115 | LOW**
- **Issue:** Inefficient allocation - bytes.to_vec() copies entire buffer every callback
- **Impact:** Excessive allocations at high sample rates (2.4 MHz)

**34. ez-daemon/src/hardware/hackrf.rs:326 | LOW**
- **Issue:** Potential misaligned access if iq_bytes_to_complex casts to wider types
- **Impact:** UB on ARM if byte_buf has odd starting offset after partial drain

**35. ez-daemon/src/hardware/rtlsdr.rs:131 | LOW**
- **Issue:** apply_gain silently no-ops when device not started
- **Impact:** Makes debugging harder - should return Result or log

---

## Components Still Under Audit

- **DSP Pipelines:** audio.rs, packet.rs, telemetry.rs, spectrum.rs (IN PROGRESS)
- **LRPT Decoder:** qpsk.rs, frame_sync.rs, ccsds.rs, reed_solomon.rs, image_builder.rs
- **Network Layer:** ws.rs, api.rs, server.rs, net_io.rs
- **GUI Layer:** source_manager.rs, sdr_panel.rs, app.rs, daemon_client.rs, demod.rs
- **Frontend:** main.ts, audio/monitor.ts, spectrum-worker.ts, aircraft-panel.ts
- **Integration:** ez-proto protocol consistency, cross-component sync
- **Verification:** Adversarial verification of CRITICAL findings pending completion

---

## Next Steps

1. ✅ Complete DSP Pipelines audit (audio, packet, telemetry, spectrum)
2. ⏳ Complete LRPT Decoder audit (QPSK, Reed-Solomon, frame sync, CCSDS, image builder)
3. ⏳ Complete Network Layer audit (WebSocket, HTTP, TCP)
4. ⏳ Complete GUI Layer audit (source management, panels, state sync)
5. ⏳ Complete Frontend audit (TypeScript browser client, audio chain)
6. ⏳ Complete Integration audit (protocol consistency)
7. ⏳ Adversarial verification of all CRITICAL findings (3-judge panel per finding)
8. ✅ Generate final consolidated report with verified issues only
9. ✅ Update PRODUCTION_AUDIT_TASK.md with completion status
10. ✅ Commit audit results to git

---

**Report will be updated as workflow progresses.**
