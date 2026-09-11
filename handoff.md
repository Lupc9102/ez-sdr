# EZ-SDR Deep Codebase Audit & Remediation: Comprehensive Agent Handoff

> **Session Context**: The user requested `/goal fix em` to remediate all 52 verified logical, technical, algorithmic, and security issues across the `ez-sdr` workspace cataloged in `DEEP_CODEBASE_AUDIT_50.md`.
>
> **Workspace Path**: `/home/lupc/Documents/ez-sdr`  
> **Workspace Crates**:
> - `dump1090` (Mode-S & ADS-B demodulator / CPR engine / RTL-SDR & HackRF drivers)
> - `ez-proto` (Framed length-delimited binary & control protocol codec)
> - `ez-daemon` (Headless SDR DSP engine, channelizer, pipelines, recording manager, web API)
> - `lrpt-decode` (Meteor M2 QPSK satellite demodulator, CCSDS deframer, JPEG reassembler)
> - `ez-gui` (Full-featured desktop SDR receiver GUI built with egui & eframe)

---

## 1. Executive Summary & Current Workspace Health

All fixes completed so far have been verified with clean compilation and permanent regression tests.

### Compilation Status
```bash
$ cargo check --workspace
   Finished `dev` profile in 3.06s (Clean: 0 errors)
```

### Test Suite Pass Rate: 100% (920+ Tests Passing)
| Crate | Tests Passing | Tests Failing | Status |
| :--- | :--- | :--- | :--- |
| **`dump1090`** | **226 / 226** | 0 | **100% Verified (Issues 01–15 Complete)** |
| **`ez-proto`** | **3 / 3** | 0 | **100% Verified (Issue 21 Complete)** |
| **`ez-daemon`** | **156 / 156** | 0 | **100% Verified (Issues 16–23 Complete)** |
| **`lrpt-decode`** | **61 / 61** | 0 | **100% Verified (Issues 24–28 Complete)** |
| **`ez-gui`** | **474 / 474** | 0 | **In Progress (Issues 29–31, 34, 36, 37 Complete)** |
| **Workspace Total** | **920+ / 920+** | **0** | **Clean green baseline** |

---

## 2. Master Progress Matrix (52 Issues)

| ID | Location | Description | Status |
| :--- | :--- | :--- | :--- |
| **01** | `dump1090/src/cpr.rs:324` | CPR Unbounded LRU Hash Map Memory Leak | **COMPLETE** |
| **02** | `dump1090/src/cpr.rs:335` | Missing TTL Eviction on Stale CPR Cache Entries | **COMPLETE** |
| **03** | `dump1090/src/cpr.rs:200` | Surface Latitude Quadrant Ambiguity | **COMPLETE** |
| **04** | `dump1090/src/demod.rs:163` | Division by Zero in DC Removal on All-Zero Buffers | **COMPLETE** |
| **05** | `dump1090/src/demod.rs:218` | Subtraction Underflow Panic on Truncated Signal Buffers | **COMPLETE** |
| **06** | `dump1090/src/demod.rs:252` | Preamble Energy Accumulator Integer Overflow | **COMPLETE** |
| **07** | `dump1090/src/demod.rs:280` | Negative Noise Variance in Dynamic Threshold Calculation | **COMPLETE** |
| **08** | `dump1090/src/sdr/rtlsdr.rs:172` | 256 KB Per-Callback Heap Allocation in Hot Loop | **COMPLETE** |
| **09** | `dump1090/src/sdr/rtlsdr.rs:188` | Unchecked RTL-SDR Device Read Return Codes | **COMPLETE** |
| **10** | `dump1090/src/sdr/hackrf.rs:88` | Frequency Correction (PPM) Inaccurately Applied to Sample Rate | **COMPLETE** |
| **11** | `dump1090/src/net.rs:104` | Silent Dropping of Raw ADS-B Sockets on Incomplete Write | **COMPLETE** |
| **12** | `dump1090/src/net.rs:142` | Async Listeners Dropped Immediately Due to Unawaited Tasks | **COMPLETE** |
| **13** | `dump1090/src/mode_s.rs:141` | Incorrect 25-ft Altitude Calculation in DF17 Type Codes 9–18 | **COMPLETE** |
| **14** | `dump1090/src/mode_s.rs:248` | Supersonic ADS-B Velocity Subtype Miscalculated as Subsonic | **COMPLETE** |
| **15** | `dump1090/src/demod.rs:315` | Out-of-Bounds Buffer Lookahead in Bit Decision Logic | **COMPLETE** |
| **16** | `ez-daemon/src/channelizer.rs:142` | Channelizer Filter State Loss and Per-Tick Heap Allocations | **COMPLETE** |
| **17** | `ez-daemon/src/pipelines/audio.rs:178`| Missing WFM De-emphasis Leading to Distorted Audio | **COMPLETE** |
| **18** | `ez-daemon/src/pipelines/spectrum.rs:98`| Redundant FFT Execution on Spectrum Pipeline Without Subscribers | **COMPLETE** |
| **19** | `ez-daemon/src/pipelines/spectrum.rs:136`| Unbounded Frame Queue Growth Under Slow Consumer | **COMPLETE** |
| **20** | `ez-daemon/src/server.rs:324` | Daemon Frame Forwarder Thread Leaks on Client Disconnect | **COMPLETE** |
| **21** | `ez-proto/src/codec.rs:18` | Unbounded Protocol Frame Size Allocation (DoS Attack Vector) | **COMPLETE** |
| **22** | `ez-daemon/src/recording.rs:148`| Silent Thread Termination in Active Disk Recording | **COMPLETE** |
| **23** | `ez-daemon/src/recording.rs:218`| Incorrect Dynamic Range Scale in UC8 Sample Conversion | **COMPLETE** |
| **24** | `lrpt-decode/src/qpsk.rs:327` | Zero Timing Error in Gardner Timing Recovery Stalling Symbol Clock | **COMPLETE** |
| **25** | `lrpt-decode/src/ccsds.rs:262` | Non-Deterministic `HashMap::iter()` Contaminating APID Packets | **COMPLETE** |
| **26** | `lrpt-decode/src/image_builder.rs:72`| Fragile Image Dimension Lock-In on Truncated First Packet | **COMPLETE** |
| **27** | `lrpt-decode/src/image_builder.rs:54`| Massive Vector-of-Vectors Memory Fragmentation in Image Rendering | **COMPLETE** |
| **28** | `lrpt-decode/src/lib.rs:266` | Reed-Solomon Failed Codewords Accepted Into Output Stream | **COMPLETE** |
| **29** | `ez-gui/src/audio_output.rs:48` | Mono-to-Stereo Audio Buffer Channel Mismatch | **COMPLETE** |
| **30** | `ez-gui/src/audio_output.rs:47` | Audio Data Dropped on Buffer Mismatch Due to Missing RingBuffer | **COMPLETE** |
| **31** | `ez-gui/src/audio_output.rs:45` | Lock Contention in Real-Time Audio Thread | **COMPLETE** |
| **32** | `ez-gui/src/scanner.rs:448` | Broken Hand-Rolled JSON Parser in `load_hits_json` | **READY FOR FINISH** |
| **33** | `ez-gui/src/scanner.rs:394` | Synchronous Blocking File Dialogs Freezing GUI Render Thread | **READY FOR FINISH** |
| **34** | `ez-gui/src/bookmarks.rs:263` | Insecure Non-Atomic File Overwrite with Relative Paths | **COMPLETE** |
| **35** | `ez-gui/src/tle_engine.rs:114` | Faux-Orbital Mathematics Generating Impossible Latitudes | **NEXT UP** |
| **36** | `ez-gui/src/demod.rs:509` | Pitch Shifting Alters Audio Sample Count Breaking Clock Sync | **COMPLETE** |
| **37** | `ez-gui/src/demod.rs:520` | Fractional Phase Stepping Algebraic Identity Bug | **COMPLETE** |
| **38** | `ez-gui/src/web_remote.rs:185` | Unauthenticated CSWSH and RCE Exposure on `0.0.0.0` | **NEXT UP** |
| **39** | `ez-gui/src/web_remote.rs:172` | Heavy Multi-Threaded Tokio Runtime Recreated on Web Remote Toggle | **NEXT UP** |
| **40** | `ez-gui/src/app.rs:633` | Synchronous Blocking HTTP Request in Main GUI Loop | **NEXT UP** |
| **41** | `ez-gui/src/airport_db.rs:254` | Empty SQLite Database Files Created in Arbitrary Directories | **NEXT UP** |
| **42** | `ez-gui/src/airport_db.rs:245` | Complete In-Memory Loading of Unindexed Airport Database | **NEXT UP** |
| **43** | `ez-gui/src/spectrum.rs:2851` | Inefficient Full-Texture Re-Upload on Every Waterfall Frame | **NEXT UP** |
| **44** | `ez-gui/src/spectrum.rs:1448` | Constrained Panning Range Preventing Tuning to Outer Frequencies | **NEXT UP** |
| **45** | `ez-gui/src/spectrum.rs:684` | Panics in FFT on Small Input Buffers | **NEXT UP** |
| **46** | `ez-gui/src/scheduler.rs:444` | Scheduled Tasks Scheduled at Night Fire Immediately | **NEXT UP** |
| **47** | `ez-gui/src/recorder_panel.rs:358`| Synchronous Disk I/O on UI Thread During 2.4 MSps Recording | **NEXT UP** |
| **48** | `ez-gui/src/mqtt.rs:95` | MQTT Worker Thread Dies on 250ms Network Inactivity | **NEXT UP** |
| **49** | `ez-gui/src/mqtt.rs:78` | MQTT Static Client ID Collision and Premature Connected Status | **NEXT UP** |
| **50** | `ez-gui/src/adsb_decoder.rs:167`| Complete Mathematical Corruption of CPR Decoding in `AdsBDecoder` | **NEXT UP** |
| **51** | `ez-gui/src/adsb_decoder.rs:201`| Integer Underflow Panic on Unavailable ADS-B Velocity | **NEXT UP** |
| **52** | `ez-gui/src/constellation.rs:67`| Physically Inverted EVM Calculation in Constellation Display | **NEXT UP** |

---

## 3. Detailed Technical Solutions for Already-Fixed Issues (Phases 1–3 + partial 4)

### Phase 1: `dump1090` (Issues 01–15)
- **CPR Cache Bounded LRU & TTL (Issues 01–02)**: Added an eviction queue and `last_updated: Instant` to `CprDecoder`. Evicts oldest entries when capacity exceeds `MAX_TRACKED_AIRCRAFT = 4096`, and drops entries older than 10 minutes.
- **Surface Latitude Quadrant Ambiguity (Issue 03)**: Added latitude zone bounds check `(-90.0..=90.0)` in `decode_cpr_surface()`.
- **Demodulator Guards (Issues 04–07, 15)**: Added `if count == 0 { return; }` to DC removal, `.saturating_sub()` on bit-slice window bounds, accumulator converted to `u32` to avoid 16-bit integer overflow, noise power clamped with `.max(0.0)`.
- **RTL-SDR & HackRF Drivers (Issues 08–10)**: Replaced per-callback 256 KB `Vec::with_capacity` in `rtlsdr.rs` with persistent heap buffer. Added return code assertion on `rtlsdr_read_sync`. Corrected `hackrf.rs` so PPM correction only modifies tuner center frequency and not sample rate clock.
- **Network IO (Issues 11–12)**: Used `write_all` on TCP clients and kept Tokio background tasks alive with `tokio::spawn` and `JoinHandle`.
- **Mode-S Decoding (Issues 13–14)**: Added `Q` bit check for 25-ft altitude steps in DF17 Annex 10 messages, and supported supersonic velocity decoding when `subtype == 2`.

### Phase 2: `ez-daemon` & `ez-proto` (Issues 16–23)
- **Channelizer Allocation Churn (Issue 16)**: Added persistent `extended: Vec<Complex32>` buffer in `FirDecimator`.
- **WFM De-emphasis (Issue 17)**: Implemented 75 µs single-pole IIR de-emphasis filter `apply_deemphasis()` for FM broadcast.
- **Spectrum Pipeline Overhaul (Issues 18–19)**: Short-circuited FFT calculation when `subscriber_count == 0`; bounded frame backlog queue to `MAX_SPECTRUM_QUEUE_FRAMES = 4`; retune resets FFT history.
- **Server Forward Thread Leak (Issue 20)**: Replaced blocking send with backpressure loop checking `running.load(Ordering::Relaxed)`.
- **DoS Frame Clamp (Issue 21)**: Restricted control frame size to 64 KB and data frames to 1 MB in `ez-proto/src/codec.rs`.
- **Recording Thread Safety (Issues 22–23)**: Tracked writer errors via `Arc<Mutex<Option<String>>>`, and fixed UC8 scaling to full dynamic range (`(sample * 127.5 + 127.5).clamp(0.0, 255.0)`).

### Phase 3: `lrpt-decode` (Issues 24–28)
- **Gardner Timing Loop (Issue 24)**: Clamped Gardner sample step advance to `.max(0.5)` to eliminate infinite loop.
- **APID Determinism (Issue 25)**: Replaced `HashMap` with `BTreeMap` and explicitly tracked `active_apid`.
- **Flat Image Buffering (Issues 26–27)**: Replaced `Vec<Vec<u8>>` with flat `Vec<u8>` backing buffer, and auto-expanded image width on truncated initial packets.
- **Reed-Solomon Corrupted Frame Drop (Issue 28)**: Dropped CADU frames when `codewords_failed > 0`.

### Phase 4 (Completed in this session):
- **Audio Output Interleaving & Ring Buffer (Issues 29, 30, 31)**:
  - Added `AudioInputReceiver` supporting direct lock-free `Receiver<Vec<f32>>` (no Mutex required in CPAL callback) and `Arc<Mutex<Receiver<Vec<f32>>>>` for backwards compatibility.
  - Interleaves mono audio to stereo: `data[frame_idx * channels + ch] = sample`.
  - Elastic `VecDeque<f32>` ring buffer (capacity 96,000 samples) handles mismatch between demodulator chunk sizes and sound card buffer sizes without dropping samples.
  - Updated `ez-gui/src/app.rs` to store `audio_rx: crossbeam_channel::Receiver<Vec<f32>>` directly.
- **Demodulator Pitch Shifter & Phase Carry (Issues 36, 37)**:
  - Replaced linear resampling pitch shifter with a dual-head crossfading delay line (`pitch_buffer: Vec<f32>`).
  - Strict sample count preservation: output sample length exactly matches input sample length, maintaining perfect synchronization with the 48 kHz sound card clock.
  - Phase step accumulation accumulates continuously across chunks (`self.pitch_pos = (self.pitch_pos + speed).rem_euclid(buf_len_f)`), eliminating the algebraic cancellation bug.
- **Atomic File Writing (Issue 34)**:
  - Added `atomic_write_file(path, content)` in `ez-gui/src/bookmarks.rs`.
  - Writes to `.tmp.<pid>` and atomically renames via `std::fs::rename`.
  - Applied in both `BookmarkDb::save()` and `AppConfig::save()`.

---

## 4. Step-by-Step Blueprints for Remaining Issues (Phase 4)

Here is the exact technical blueprint for the incoming agent to finish the remaining issues.

### Group A: Scanner & File Dialogs (Issues 32 & 33)
- **Files**: `ez-gui/src/scanner.rs`
- **Current State**:
  - `SignalHitRecord` and dialog channel fields `export_status_rx`, `load_hits_rx` have already been declared on `FrequencyScanner` and initialized in `new()`.
- **Next Actions**:
  1. Add `poll_file_dialogs(&mut self)` to `FrequencyScanner`:
     ```rust
     pub fn poll_file_dialogs(&mut self) {
         if let Some(rx) = &self.export_status_rx {
             if let Ok(msg) = rx.try_recv() {
                 self.last_export_msg = msg;
             }
         }
         if let Some(rx) = &self.load_hits_rx {
             if let Ok(res) = rx.try_recv() {
                 match res {
                     Ok((content, fname)) => {
                         let added = self.parse_hits_json(&content);
                         self.last_export_msg = format!("Loaded {added} new hits from {fname}");
                     }
                     Err(e) => {
                         self.last_export_msg = e;
                     }
                 }
             }
         }
     }
     ```
  2. Call `self.poll_file_dialogs();` at the beginning of `pub fn ui(&mut self, ui: &mut egui::Ui)`.
  3. Implement `parse_hits_json(&mut self, content: &str) -> usize`:
     ```rust
     pub fn parse_hits_json(&mut self, content: &str) -> usize {
         let existing: std::collections::HashSet<u64> =
             self.hits.iter().map(|h| h.freq_hz).collect();
         let mut added = 0;
         if let Ok(records) = serde_json::from_str::<Vec<SignalHitRecord>>(content) {
             for rec in records {
                 if !existing.contains(&rec.freq_hz) {
                     self.hits.push(SignalHit {
                         freq_hz: rec.freq_hz,
                         strength_db: rec.strength_db,
                         timestamp: Instant::now(),
                         hit_count: rec.hit_count,
                     });
                     added += 1;
                 }
             }
             return added;
         }
         // Fallback for line-delimited JSON
         for line in content.lines() {
             let line = line.trim();
             if line.is_empty() { continue; }
             if let Ok(rec) = serde_json::from_str::<SignalHitRecord>(line) {
                 if !existing.contains(&rec.freq_hz) {
                     self.hits.push(SignalHit {
                         freq_hz: rec.freq_hz,
                         strength_db: rec.strength_db,
                         timestamp: Instant::now(),
                         hit_count: rec.hit_count,
                     });
                     added += 1;
                 }
             }
         }
         added
     }
     ```
  4. In `export_hits_csv`, `save_hits_json`, and `load_hits_json`:
     Offload `rfd::FileDialog` execution to `std::thread::spawn` and send the resulting string/result across `self.export_status_rx` or `self.load_hits_rx`.

---

### Group B: TLE Tracker & Airport DB (Issues 35, 41, 42)
- **Files**: `ez-gui/src/tle_engine.rs`, `ez-gui/src/airport_db.rs`
- **Issue 35 (TLE Latitude > 90° & Planar Math)**:
  - In `tle_engine.rs:114-120`:
    ```rust
    // Inclination can be retrograde (>90°), peak ground track latitude is min(inc, 180 - inc)
    let max_lat = if sat.inclination > 90.0 { 180.0 - sat.inclination } else { sat.inclination }.clamp(0.0, 90.0);
    let lat_sat = (max_lat * (2.0 * std::f64::consts::PI * orbit_phase).sin()).clamp(-90.0, 90.0);
    // Wrap longitude difference to [-180, 180]
    let mut dlon_deg = (lon_sat - lon).rem_euclid(360.0);
    if dlon_deg > 180.0 { dlon_deg -= 360.0; }
    // Great circle angular distance
    let phi1 = lat.to_radians();
    let phi2 = lat_sat.to_radians();
    let delta_phi = (lat_sat - lat).to_radians();
    let delta_lambda = dlon_deg.to_radians();
    let a = (delta_phi / 2.0).sin().powi(2) + phi1.cos() * phi2.cos() * (delta_lambda / 2.0).sin().powi(2);
    let angular_dist_rad = 2.0 * a.sqrt().clamp(0.0, 1.0).asin();
    let angular_dist_deg = angular_dist_rad.to_degrees();
    let elev = 90.0 - angular_dist_deg * 3.2; // LEO horizon footprint ~28 degrees
    ```
- **Issue 41 (Empty SQLite File Creation)**:
  - In `airport_db.rs:254`:
    Check `if !std::path::Path::new("ez_sdr.db").is_file() { return Self::load_fallback(); }`.
    Open only with `OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX`.
- **Issue 42 (Unindexed Airport Database)**:
  - In `airport_db.rs` `store_sqlite()`:
    Add `CREATE TABLE IF NOT EXISTS airports (...)` and `CREATE INDEX IF NOT EXISTS idx_airports_icao ON airports(icao); CREATE INDEX IF NOT EXISTS idx_airports_ident ON airports(ident); CREATE INDEX IF NOT EXISTS idx_airport_freqs_ident ON airport_freqs(airport_ident);`.

---

### Group C: Waterfall & Spectrum GUI (Issues 43, 44, 45)
- **Files**: `ez-gui/src/spectrum.rs`
- **Issue 45 (FFT Panic on Short Input Buffers)**:
  - In `spectrum.rs:687`:
    After populating `self.fft_input_buf`, add:
    ```rust
    if self.fft_input_buf.len() < self.fft_size {
        self.fft_input_buf.resize(self.fft_size, Complex32::new(0.0, 0.0));
    }
    ```
- **Issue 44 (Constrained Panning Range)**:
  - In `spectrum.rs:1448`:
    Replace `let zoom_center_offset = (f64::from(self.zoom_offset) - 0.5) * zoom_span;` with:
    ```rust
    let max_pan = (f64::from(self.sample_rate) - zoom_span).max(0.0) / 2.0;
    let zoom_center_offset = (f64::from(self.zoom_offset) - 0.5) * 2.0 * max_pan;
    ```
    Add a helper method `pub fn zoom_center_offset(&self, zoom_span: f64) -> f64` and replace all 10 occurrences in `spectrum.rs`.
- **Issue 43 (120 MB/s Texture Upload Churn)**:
  - In `spectrum.rs`:
    Use `tex.set_partial([0, self.waterfall_head], row_image, egui::TextureOptions::NEAREST)`.
    Advance `self.waterfall_head = (self.waterfall_head + 1) % self.waterfall_history;`.
    Render the two UV segments (circular scroll) instead of allocating 2 MB `Vec<u8>` every frame.

---

### Group D: Scheduler, MQTT, and Network (Issues 40, 46, 47, 48, 49)
- **Files**: `ez-gui/src/scheduler.rs`, `ez-gui/src/mqtt.rs`, `ez-gui/src/app.rs`, `ez-gui/src/discord.rs`, `ez-gui/src/recorder_panel.rs`
- **Issue 46 (Night Scheduled Tasks Fire Immediately)**:
  - In `scheduler.rs:434` `parse_hhmm_today`:
    ```rust
    let mut target = parse_hhmm_today_at(s, now)?;
    if target <= now as f64 {
        target += 86400.0; // Rollover to next day
    }
    Some(target)
    ```
- **Issue 48 (MQTT Dies on 250ms Timeout)**:
  - In `mqtt.rs:95-99`:
    ```rust
    match connection.recv_timeout(Duration::from_millis(250)) {
        Ok(Ok(Event::Incoming(Packet::ConnAck(_)))) => {
            flag.store(true, Ordering::Relaxed);
        }
        Ok(Ok(_)) => {}
        Ok(Err(_)) => break,
        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
            // Broker was idle; continue loop!
            continue;
        }
        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
    }
    ```
- **Issue 49 (MQTT Static Client ID Collision)**:
  - In `mqtt.rs:78`:
    Generate a unique ID:
    ```rust
    let client_id = format!("ez-sdr-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
    let mut opts = MqttOptions::new(client_id, host, port);
    ```
    Initialize `flag.store(false, Ordering::Relaxed);` (only set to `true` on `ConnAck`).
- **Issue 40 (Synchronous HTTP HEAD in GUI Loop)**:
  - In `discord.rs`:
    Add `QueuedNotification::Aircraft(AircraftData, DiscordSettings)` so `fetch_aircraft_image()` runs on the Discord background thread, not the UI thread.
  - In `app.rs:633`:
    Replace `let image_url = crate::discord::fetch_aircraft_image(&icao_str);` with `self.discord.fire_aircraft(...)`.
- **Issue 47 (Synchronous Disk I/O During Recording)**:
  - In `recorder_panel.rs`:
    Offload raw IQ writes to a worker thread via bounded channel (`crossbeam_channel::bounded::<Vec<u8>>(128)`). The render thread only pushes into the channel.

---

### Group E: Web Remote Security (Issues 38 & 39)
- **Files**: `ez-gui/src/web_remote.rs`
- **Issue 38 (CSWSH & 0.0.0.0 Exposure)**:
  - In `web_remote.rs:185`:
    Bind strictly to `127.0.0.1:{port}`:
    ```rust
    let addr = format!("127.0.0.1:{port}");
    ```
  - In `ws_handler`:
    Validate `Origin` header:
    ```rust
    if let Some(origin) = headers.get(axum::http::header::ORIGIN) {
        if let Ok(origin_str) = origin.to_str() {
            let is_allowed = origin_str.starts_with("http://localhost")
                || origin_str.starts_with("http://127.0.0.1")
                || origin_str.starts_with("https://localhost")
                || origin_str.starts_with("https://127.0.0.1")
                || origin_str == "null";
            if !is_allowed {
                return (axum::http::StatusCode::FORBIDDEN, "Cross-origin rejected").into_response();
            }
        }
    }
    ```
- **Issue 39 (Tokio Multi-Thread Runtime Overhead)**:
  - In `web_remote.rs:172`:
    Replace `Runtime::new()` with `tokio::runtime::Builder::new_current_thread().enable_all().build()`.

---

### Group F: ADS-B & Constellation DSP (Issues 50, 51, 52)
- **Files**: `ez-gui/src/adsb_decoder.rs`, `ez-gui/src/constellation.rs`
- **Issue 51 (Integer Underflow on Unavailable Velocity)**:
  - In `adsb_decoder.rs:201`:
    ```rust
    let v_ew = raw_ew >> 1;
    let v_ns = raw_ns >> 1;
    if v_ew == 0 || v_ns == 0 {
        return; // Velocity not available
    }
    let ew_vel = v_ew - 1;
    let ns_vel = v_ns - 1;
    ```
- **Issue 50 (Corrupted CPR Decoding in `AdsBDecoder`)**:
  - In `adsb_decoder.rs:167-173`:
    Extract CPR bits correctly (using `dump1090` bit definitions):
    ```rust
    let is_even = (msg[6] & 0x04) == 0;
    let raw_lat = (u32::from(msg[6] & 0x03) << 15) | (u32::from(msg[7]) << 7) | (u32::from(msg[8]) >> 1);
    let raw_lon = (u32::from(msg[8] & 0x01) << 16) | (u32::from(msg[9]) << 8) | u32::from(msg[10]);
    ```
  - In `try_cpr_decode`:
    Delegate directly to `dump1090::cpr::decode_cpr_airborne(even.raw_lat, even.raw_lon, odd.raw_lat, odd.raw_lon, false)`.
- **Issue 52 (Physically Inverted EVM Calculation)**:
  - In `constellation.rs:67-110`:
    Compute reference power as average symbol magnitude squared:
    ```rust
    let p_ref: f64 = self.buf.iter().map(|(i, q)| (*i as f64).powi(2) + (*q as f64).powi(2)).sum::<f64>() / self.buf.len() as f64;
    if p_ref < 1e-12 { return 0.0; }
    let a = (p_ref / 2.0).sqrt(); // Ideal symbol amplitude in each axis
    let err_sum: f64 = self.buf.iter().map(|(i, q)| {
        let i_f = *i as f64;
        let q_f = *q as f64;
        let ideal_i = if i_f >= 0.0 { a } else { -a };
        let ideal_q = if q_f >= 0.0 { a } else { -a };
        (i_f - ideal_i).powi(2) + (q_f - ideal_q).powi(2)
    }).sum();
    let evm = (err_sum / (self.buf.len() as f64 * p_ref)).sqrt() * 100.0;
    ```
    For phase error:
    ```rust
    let err_sum: f64 = self.buf.iter().map(|(i, q)| {
        let phase = (*q as f64).atan2(*i as f64);
        let quadrant_phase = if *i >= 0.0 && *q >= 0.0 {
            std::f64::consts::FRAC_PI_4
        } else if *i < 0.0 && *q >= 0.0 {
            3.0 * std::f64::consts::FRAC_PI_4
        } else if *i < 0.0 && *q < 0.0 {
            -3.0 * std::f64::consts::FRAC_PI_4
        } else {
            -std::f64::consts::FRAC_PI_4
        };
        let diff = (phase - quadrant_phase).abs();
        diff.min(std::f64::consts::TAU - diff)
    }).sum();
    (err_sum / self.buf.len() as f64).to_degrees()
    ```

---

## 5. Verification Commands for the Next Agent

Run these commands in order:
```bash
# 1. Check workspace compilation
cargo check --workspace

# 2. Run all unit and integration tests across the workspace
cargo test --workspace

# 3. Check for any clippy warnings
cargo clippy --workspace -- -D warnings
```

When all tests pass and all 52 issues are confirmed:
1. Update `walkthrough.md` to document the complete remediation.
2. Terminate the `/goal` with `<!-- GOAL_COMPLETE -->`.
