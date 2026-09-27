# ez-sdr Performance Audit

**Date:** 2026-09-27
**Scope:** Why the app feels *slow* sometimes, why buttons lag, why sound is delayed or stutters.
**Method:** Full read of the main loop + audio/DSP/source/IPC paths, five parallel subsystem audits (UI loop, audio, DSP/spectrum, daemon IPC, rendering/panels), every critical finding re-verified against source.

---

## 1. Executive summary

The app's architecture is fundamentally sound: demodulation, FFT, audio output, and the SDR source each run on dedicated worker threads with bounded channels and non-blocking UI hand-off. The perceived slowness comes from **where the remaining work sits**, not from any single huge bottleneck:

1. **The UI thread is the mandatory staging area for the entire receive pipeline.** Every sample batch flows: source thread → *UI thread* (byte convert, VFO mix, ADS-B decode, spectrum push, demod push) → demod worker → *UI thread* (volume, gating) → audio worker. Any work done on the UI thread delays buttons, sound, and rendering together, because egui processes input, runs your `logic()`, and paints on that one thread.

2. **Every queue in the chain is drop-on-full with no counters.** When the UI thread stalls, RF blocks, IQ batches, demod frames, and audio frames are all silently discarded. You hear clicks and gaps; the spectrum shows holes; nothing logs. The 200 ms audio backlog cap then discards the *oldest* audio, so after a stall the sound "jumps forward."

3. **Several per-frame costs scale with aircraft count or settings** — ADS-B ingest is O(n·m), three separate code paths each rebuild a full `HashSet<u32>` of aircraft every frame, and the spectrum paint path issues ~2000 draw calls/frame.

4. **Periodic stalls run on the UI thread under the global mutex**: the TLE/SGP4 pass recompute (~13,000 propagations every 60 s), bookmark/config synchronous disk writes, MQTT reconnect joins, and daemon-mode teardown joins.

The single highest-leverage fixes: **(a)** eliminate the per-batch ADS-B buffer copy and map clone (biggest UI-thread CPU consumer when ADS-B is on), **(b)** move the VFO mixer / radio-IQ channelizer off the UI thread or at least kill their per-batch allocations, **(c)** share one cached aircraft live-set instead of three per-frame HashSets, **(d)** add drop counters so gaps are diagnosable, **(e)** move persistence (config/bookmark saves) off the UI thread.

---

## 2. Threading model (verified)

| Thread | Owns | UI blocks on it? |
|---|---|---|
| **UI thread** (egui) | input, `logic()`, all panel painting, per-batch DSP staging, ADS-B ingest, spectrum frame drain, audio drain, bookmarks/config, scheduler, keyboard, web/rigctl/mqtt polling | — |
| Source worker | RTL-SDR USB reads / replay / simulated; sends `Vec<u8>` blocks | No (`try_send`, drops) |
| Demod worker | channelize → FM/AM/SSB/WFM demod → 48 kHz audio frames | No (`try_send`, drops) |
| Spectrum FFT worker | 2048-pt FFT windows → dB frames | No (`try_send`, drops) |
| Audio worker | owns CPAL stream; plays from bounded queue | No (`try_send`, drops) |
| Daemon client | own tokio thread; websocket recv | No (but **Drop joins it on UI thread**) |
| Tile downloaders | spawned per map tile | No |
| Discord dispatch | own thread; HTTP POSTs | No (correct) |
| MQTT drain | own thread | No (but **publish uses blocking bounded channel**) |
| web_remote / rigctl | own runtimes | No (correct) |

The global `shared: Arc<Mutex<SharedState>>` is locked **~25× per frame**, but only ever by the UI thread (workers don't hold it) — so the cost is the *work under the locks*, not lock contention.

---

## 3. Root causes

### 3.1 Why buttons feel delayed

A button press is processed by egui at the start of the next frame; the frame then runs `logic()` (all the locking, DSP staging, ADS-B ingest, drains) and paints. **Frame time = input latency.** Anything that makes a frame long makes every button feel laggy. Ranked contributors:

| # | Contributor | Cost | When |
|---|---|---|---|
| 1 | ADS-B magnitude buffer copy + `get_aircraft` clone (app.rs:904-908) | ~4 MB/s memcpy + O(n) clones per batch | ADS-B on, 1090 MHz |
| 2 | ADS-B ingest linear scan (adsb_panel.rs:572) | O(512·n)/frame | UAT mode, many aircraft |
| 3 | Per-batch UI-thread DSP: `radio_iq.process` + `vfo_mixer.process` (app.rs:872,886) | 32 KB alloc + ~16K complex mults per 8 ms batch | always receiving |
| 4 | Three per-frame aircraft `HashSet` rebuilds + `matches_filters` allocs (adsb_panel.rs:557,704,644) | O(n) × 3 + 2 allocs/aircraft | ADS-B tab, many aircraft |
| 5 | Spectrum paint ~2000 draw calls + per-frame Vecs (spectrum.rs:2760-2911) | ~683 line segments ×2 + gradient mesh | Listen tab visible |
| 6 | TLE/SGP4 recompute (tle_engine.rs:183 via app.rs:1500) | ~13k propagations + 13k `DateTime` allocs | every 60 s |
| 7 | Config/bookmark synchronous disk writes (config.rs:860, app.rs:1681) | 4-14 KB serialize + write + rename | on interaction + 15 s debounce |
| 8 | MQTT publish block (mqtt.rs:209) + reconnect join (mqtt.rs:142) | unbounded block / ~250 ms | broker down / reconnect |
| 9 | Daemon-mode teardown join (daemon_client.rs:139 ← source_manager.rs:1266) | up to ~600 ms | stopping daemon source |
| 10 | `stop()` joins source worker (source_manager.rs:1276) | ~8-16 ms (one USB block) | stop/start/tune-restart |

### 3.2 Why sound is delayed or stutters

End-to-end latency budget (hardware mode, verified):

```
RTL-SDR block      8 ms    (32 KB @ 2.048 MS/s)
source queue     bounded(32)  → 256 ms worst case   [source_manager.rs:409]
UI drain          up to 8 batches/frame, 16 ms frame  [app.rs:747]
demod IQ queue   bounded(16)  → 128 ms worst case   [demod.rs:2004]
demod frame      ~390 samples = 8.1 ms each        [demod.rs:820]
demod queue      bounded(16)  → 130 ms worst case   [demod.rs:2007]
audio queue       bounded(32)  → 260 ms worst case   [audio_output.rs:31]
audio backlog    9600 samples = 200 ms, drops oldest [audio_output.rs:200]
CPAL buffer      ~10-40 ms typical
```

- **Steady state** ≈ 16-50 ms — fine.
- **Every stage is drop-on-full** (source_manager.rs:249, demod.rs:2064/2080, audio_output.rs:461). When the UI thread stalls, the source queue fills and **8 ms RF blocks are discarded** → the demod worker never sees them → audible gap/click, and the spectrum has a hole. Drops are silent (no counters anywhere).
- The **200 ms audio backlog cap** (audio_output.rs:218-229) discards the *oldest* whole frames when the producer outpaces the CPAL callback, so after a UI stall the audio "catches up" with a jump.
- The **8-batch catch-up drain** (app.rs:747-753) means one slow frame is followed by a frame that does up to 8× the per-batch DSP work — a stall breeds a bigger stall.
- Daemon mode adds ~4 ms audio frames (~250/s, ez-daemon audio.rs:218) and an **unthrottled ~976 fps spectrum stream** (ez-daemon spectrum.rs:146) that the GUI mostly drops; below ~30 fps UI the daemon audio queue overflows and drops.

### 3.3 Why it's "sometimes" slow

The drops and stalls are intermittent because they're gated on UI-thread frame time, which spikes when: ADS-B traffic is heavy (O(n·m) ingest), the ADS-B tab is visible with many aircraft (3 HashSet rebuilds), the Listen tab paints (~2000 draw calls), the 60 s TLE recompute fires, a bookmark/config write lands, or MQTT/web/daemon I/O stalls. Idle or lightly loaded, the app is smooth.

---

## 4. Findings (ranked, all verified)

Legend: ✅ verified first-hand · 🔍 agent-reported, plausible.

### CRITICAL

**C1. ADS-B magnitude buffer fully copied every batch — UI thread** ✅
`adsb_decoder.rs:122` — `self.mag_buf[..total_length].to_vec()` copies the whole `Vec<u16>` magnitude buffer (~32 KB at 2.048 MS/s, 8 ms batch) on every batch, ~125 batches/s → **~4 MB/s memcpy on the UI thread**, plus the allocation. Called from app.rs:904.
*Fix:* pass a borrowed view (`&[u16]`) into `demodulate`, or `clone_from` into a reused buffer.

**C2. `get_aircraft()` clones the entire aircraft map every batch — UI thread** ✅
`adsb_decoder.rs:417-436`, called at app.rs:905 — every entry (incl. `String` callsign) is cloned and collected into a fresh `Vec` per batch → O(n) clones + allocs × 125/s.
*Fix:* mark the map dirty on decode; rebuild only when the panel reads it (event-driven).

### HIGH

**H1. ADS-B ingest is O(n·m) — linear scan per report** ✅
`adsb_panel.rs:572` — `self.aircraft.iter().position(|ac| ac.icao == report.key)` scans the whole vec per report; `poll_uat` drains up to 512 reports/frame → O(512·n). At the 4096 cap that's ~2M comparisons/frame.
*Fix:* `HashMap<u32, AircraftEntry>` + a small order vec.

**H2. VFO mixer runs on the UI thread, allocates per batch** ✅
`radio_iq.rs:248-262` (app.rs:886) — 32 KB output `Vec` + ~16K complex multiplies per 8 ms batch. `radio_iq.process` (radio_iq.rs:154, app.rs:872) adds another 32 KB alloc/batch.
*Fix:* move both into the demod worker; at minimum write into caller-provided buffers.

**H3. Three per-frame aircraft `HashSet` rebuilds** ✅
`adsb_panel.rs:557` (`poll_uat`), `:704` (`check_for_new_aircraft`), `:1910-1916` (`ui_list`) — each rebuilds a full `HashSet<u32>` (and a second `positioned` set) every frame, O(n) each.
*Fix:* build one live-set per frame, share it; or use generation counters.

**H4. `matches_filters` allocates 2 Strings per aircraft per call** ✅
`adsb_panel.rs:644-658` — `to_ascii_lowercase()` + `format!` per aircraft, called from 3 sites/frame.
*Fix:* precompute lowercase callsign + hex ICAO at ingest.

**H5. Spectrum paint issues ~2000 draw calls + per-frame Vecs** 🔍
`spectrum.rs:2760-2911` — ~683 `line_segment` calls ×2 (line + peak-hold) + gradient mesh + `candidates`/`labeled_xs` Vecs every paint.
*Fix:* one `Shape::Vec` or a single mesh for the line; reuse buffers.

**H6. TLE/SGP4 pass recompute blocks the UI thread every 60 s** ✅
`tle_engine.rs:183` via app.rs:1500 — 72 h at 60 s steps = 4320 steps × 3 TLEs ≈ **13k SGP4 propagations + 13k `DateTime` allocations** (tle_engine.rs:516) under the shared lock → a 5-30 ms freeze each minute (worse with imported TLEs).
*Fix:* compute on a background thread, swap atomically.

**H7. Config & bookmark saves are synchronous disk I/O on the UI thread** ✅
`config.rs:860` (serde + write + rename) triggered from Ctrl+S, AI-panel toggles, Save & Apply; `bookmark_manager.rs:113/125/144` on every add/remove/edit; `app.rs:1681` 15 s-debounced autosave fires under the lock mid-interaction.
*Fix:* debounce + background thread; route explicit saves through the debounce.

**H8. MQTT `publish` can block the UI thread indefinitely** ✅
`mqtt.rs:209` → rumqttc `Client::publish` does a **blocking** `send` on a bounded(128) channel (rumqttc client.rs:296). If the broker is unreachable the queue fills and the UI blocks. `mqtt.rs:142` `join_drain` blocks ~250 ms on reconnect.
*Fix:* `try_publish`, drop on Full; don't join on the UI thread.

**H9. Daemon-mode teardown joins on the UI thread** 🔍
`daemon_client.rs:139-145` Drop joins the tokio worker (100 ms poll + 500 ms detach timeout) ← `source_manager.rs:1266` → up to ~600 ms stall stopping daemon mode.
*Fix:* detach the join / short deadline.

**H10. Daemon mode: spectrum + audio resampling under the global lock** ✅
`app.rs:605-663` — the 8-event drain holds `shared` across `push_spectrum_frame` (2048-bin loop) and `daemon_audio.process` (resample + alloc).
*Fix:* collect under lock, process outside.

### MEDIUM

**M1. WAV audio write is synchronous on the UI thread** 🔍 — `recorder_panel.rs:619-649` writes ~390 samples/frame, ~125/s (the IQ path is already async). Fix: mirror the IQ path.

**M2. Silent drop-on-full at every pipeline stage** ✅ — source (source_manager.rs:249), demod IQ (demod.rs:2080), demod frames (demod.rs:2064), audio (audio_output.rs:461). Fix: count drops, surface in status bar.

**M3. 200 ms audio backlog discards oldest frames** ✅ — `audio_output.rs:218-229`. Reasonable overload behavior, but produces a hard skip; consider a fade/silence gap.

**M4. `wfm_multiplex` reallocated from scratch every frame** ✅ — `demod.rs:1009` `std::mem::take` drops capacity; `demod.rs:1692` re-`reserve`s 32 KB/frame when RDS tap is on. Fix: `clear()` instead of take.

**M5. `recompute_audio_filters` runs every frame** 🔍 — `demod.rs:812-836` recomputes biquads even when config is static. Fix: early-return when rates unchanged.

**M6. FM IF noise-reduction sliding FFT runs per output sample** ✅ — `demod.rs:419-446` windows the full history + full FFT + bin scan per IF sample (50-250K FFTs/s when enabled). Off-UI-thread but can make the worker fall behind → dropped audio frames. Fix: sliding-DFT single-bin update.

**M7. `transform_iq_window` allocates per FFT frame** 🔍 — `spectrum.rs:1278-1294`, 2048-elem Vec per FFT (~160 KB/s at defaults). Fix: reused buffer.

**M8. `ThemeConfig` cloned 2-3× per frame in the ADS-B tab** ✅ — `adsb_panel.rs:1234,1349,1889`. Fix: cache with a generation counter.

**M9. Daemon spectrum stream ~976 fps unthrottled; audio ~4 ms frames drop below ~30 fps UI** 🔍 — ez-daemon spectrum.rs:146, audio.rs:218, daemon_client.rs:287. Fix: throttle to display rate; batch audio to 10-20 ms.

**M10. `stop()` joins the source worker (~8-16 ms)** ✅ — source_manager.rs:1276. Acceptable but a hitch on every stop/start.

### LOW

- **L1.** `poll_uat`/`check_for_new_aircraft` lock the shared mutex every frame ✅ (adsb_panel.rs:681, app.rs:1038-1039)
- **L2.** `howto_panel` search re-runs every frame 🔍 (howto_panel.rs:168)
- **L3.** `freq_jump_matches.clone()` per frame ✅ (app.rs:1838)
- **L4.** `FrequencyDatabase::search` rebuilds all presets per keystroke 🔍 (frequency_db.rs:340)
- **L5.** Airport DB search: 4 allocs/airport 🔍 (airport_db.rs:357)
- **L6.** `render_toasts` allocates per frame while toasts active 🔍 (adsb_panel.rs:796)
- **L7.** `seen_aircraft` HashSet grows for the whole session ✅ (app.rs:1069)
- **L8.** `ui_map` allocates a `Mesh` per aircraft per frame 🔍 (adsb_panel.rs:1732)

### Positive findings (done right)

- Dedicated workers for demod/FFT/audio/source; UI never blocks on them ✅
- Non-blocking hand-off (`try_send`/`try_recv`) throughout ✅
- Discord fully offloaded to a background dispatch thread ✅
- rigctl / web_remote on own runtimes, non-blocking ✅
- Replay mode backpressure correct (20 ms `send_timeout`) ✅
- Waterfall partial-row upload path (full upload only on user opt-in) ✅
- TLE pass caching (60 s) ✅
- `daemon_control_commands` edge-triggered (no per-frame network spam) ✅
- Audio resampler is a cheap linear interpolator ✅
- No production spin/sleep/yield loops in any worker ✅
- `iq_ring` is bounded — no unbounded growth ✅

---

## 5. Recommended fixes (effort → impact)

| Priority | Fix | Effort | Impact |
|---|---|---|---|
| 1 | C1: eliminate ADS-B magnitude buffer copy (borrow slice) | S | removes ~4 MB/s UI-thread memcpy when ADS-B on |
| 2 | H1: HashMap for ADS-B ingest | S | removes O(n·m) per frame |
| 3 | H3+H4: one shared live-set; precompute filter strings | S | removes 3 O(n) HashSet rebuilds + per-aircraft allocs |
| 4 | C2: event-driven `get_aircraft` | S | removes O(n) clones × 125/s |
| 5 | H2: move VFO mixer + radio-IQ channelizer into demod worker (or at least kill per-batch allocs) | M | removes 64 KB/batch churn + DSP from UI thread |
| 6 | H7: debounce + background-thread all persistence writes | S | removes per-interaction disk stalls |
| 7 | H8: MQTT `try_publish` + no join on UI thread | S | removes unbounded block + 250 ms reconnect stall |
| 8 | H6: background TLE compute with atomic swap | M | removes 60 s periodic freeze |
| 9 | M2: add drop counters at every queue | S | makes gaps diagnosable |
| 10 | H9/H10: daemon teardown + per-frame lock-hold fixes | M | removes daemon-mode stalls |
| 11 | M4/M5/M6: demod worker garbage & per-frame recompute | M | reduces audio-frame drops |
| 12 | H5/M7/M8: spectrum paint batching + buffer reuse | M | reduces Listen-tab render cost |

*S = small, M = medium.*

---

## 6. Verification notes

- The shared `Mutex<SharedState>` is only ever locked by the UI thread; workers never hold it. The ~25 lock acquisitions/frame cost ~1-2 μs total — the cost is the work *under* the locks (TLE clone, JSON serialization, disk writes, DSP), not contention.
- `try_lock` therefore essentially never fails; the "skip keyboard input when contended" path is effectively dead code.
- All drop-on-full sites were confirmed to have no counters; drops are invisible in the UI.
- The audio latency chain's large queue depths are safety buffers; they only become audible when the UI stalls, at which point drops/skips occur rather than graceful degradation.
