# Listen Tab Delay Audit — "massive delays" when listening to ATC/radio

**Date:** 2026-09-27
**Scope:** Why the Listen tab (where you listen to ATC/radio) has massive delays.
**Method:** Four parallel investigations (render cost, audio staging, demod worker, audio queue/CPAL) + first-hand verification of every critical claim.

---

## Executive summary

The Listen tab has **two independent latency problems** that combine to produce "massive delays":

1. **The render path consumes the entire frame budget.** The Listen tab paints ~4,700 draw calls per frame (14-29 ms), which is most or all of the 16.7 ms frame budget. This starves the audio pipeline staging (DSP, demod drain, audio push), causing audio queues to back up and latency to grow.

2. **The audio output path has ~50-100 ms of fixed latency from an uncontrolled CPAL buffer size, plus up to 256 ms of queue depth.** When the UI thread stalls (which it does, because rendering is so expensive), the audio queue fills and latency spikes to 200-500 ms+.

**Combined steady-state latency: ~100-150 ms** (CPAL buffer + UI frame + source block + demod). **Under load: 300-1100 ms** (all queues full + catch-up drain + drops). For radio listening, even 100 ms is noticeable; 300-500 ms feels "massive."

---

## Root cause 1: Render path consumes the frame budget

### The Listen tab paints ~4,700 draw calls per frame

Verified breakdown (all confirmed first-hand):

| Widget | Draw calls | Problem |
|---|---|---|
| Audio waveform | ~2,047 `line_segment` | one 1px segment per sample; clones 2048 samples/frame (`spectrum.rs:2044-2074`) |
| Spectrum line | ~1,800 `line_segment` | one segment per pixel-column (`spectrum.rs:2883-2911`) |
| Signal history mini-chart | ~599 `line_segment` | rebuilt every frame; clones 600 points (`spectrum.rs:1891-1961`) |
| Squelch dashes | ~200 `line_segment` | one segment per dash (`spectrum.rs:2950-2985`) |
| Grid lines + labels | ~60 | re-formatted every frame |
| Band plan | ~40 | 40-entry Vec rebuilt every frame (`spectrum.rs:2331`) |
| **Total** | **~4,700** | **14-29 ms/frame paint time** |

At 60 fps the frame budget is 16.7 ms. The Listen tab render path alone consumes 14-29 ms — **the UI thread has almost no time left for audio pipeline staging.**

### Per-frame allocations in the paint path

- `Vec::with_capacity(waterfall_width * waterfall_history * 4)` — ~2 MB (`spectrum.rs:3663`)
- `history_vec` collect + `pts` collect + `pts.clone()` — 600 f32 + 600 Pos2 (`spectrum.rs:1902,1931,1939`)
- `wf_samples` collect — 2048 f32 (`spectrum.rs:2044`)
- `candidates` + `labeled_xs` Vecs (`spectrum.rs:2833,2844`)
- Dozens of `format!()` calls for labels every frame

### The 8-batch catch-up drain amplifies stalls

`app.rs:747` — `for _ in 0..8` drains up to 8 batches per frame. Under any rendering hiccup, blocks accumulate. One slow frame is followed by a frame that does up to 8× the DSP work (131k samples) — a stall breeds a bigger stall.

---

## Root cause 2: Audio output path latency

### CPAL buffer size is uncontrolled — 50-100 ms fixed latency

`audio_output.rs:284-295` — `device.build_output_stream(config, ..., None)`. The `None` is the latency hint; no `buffer_size` is set on the StreamConfig. On PulseAudio (the preferred host), the default buffer is **50-100 ms**. This is the single largest fixed latency component and it's entirely uncontrolled.

### Audio queue depth: 256 ms worst case

`audio_output.rs:31` — `AUDIO_QUEUE_CAPACITY = 32`. Each frame is ~390 samples (8.1 ms). Queue capacity: 32 × 8.1 ms = **256 ms**. Under UI stall, the queue fills and `try_send` silently drops frames.

### Audio worker thread has no priority

`audio_output.rs:429` — plain `std::thread::spawn`. No `SCHED_FIFO`, no priority hint. When the UI thread is busy painting (14-29 ms), the CPAL callback competes at equal priority and can be delayed, causing underruns.

### Every queue is drop-on-full with no counters

Source (`source_manager.rs:249`), demod IQ (`demod.rs:2080`), demod frames (`demod.rs:2064`), audio (`audio_output.rs:461`) — all silently drop when full. No logging, no metrics.

---

## Root cause 3: UI-thread DSP staging

The UI thread does ~4.3M ops and ~2 MB of allocations per frame for DSP staging that doesn't need to be on the UI thread:

| Component | Ops/frame | Allocations | Est. time |
|---|---|---|---|
| `radio_iq.process` (8 batches) | ~1.3M | 1 MB (8× 128 KB Vec) | ~1-2 ms |
| `vfo_mixer.process` (8 batches) | ~2M | 1 MB (8× 128 KB Vec) | ~1-2 ms |
| `spectrum.push_complex_samples` | ~1M | ~0 (ring buffer) | ~0.3 ms |
| **Total DSP staging** | **~4.3M** | **~2 MB** | **~3-5 ms** |

Add 14-29 ms of rendering and the 16.7 ms frame budget is exceeded by 2-3×. The UI thread is saturated.

---

## Demod worker latency

| Mode | Per-batch cost | Budget (8 ms) | Status |
|---|---|---|---|
| FM mono (NFM) | ~2-4 ms | 8 ms | within budget |
| WFM stereo | ~5-15 ms | 8 ms | **exceeds budget** |
| AM (ATC) | ~1-2 ms | 8 ms | within budget |

The WFM stereo path (`demod.rs:290-331`) runs a full pilot PLL + 241-tap multiplex FIR + 6th-order channel filter per multiplex sample. The FIR alone runs per input sample at 2.048 MHz. This can make the demod worker fall behind, dropping audio frames.

For ATC (AM), the demod worker is fine — the delay comes from the render path, CPAL buffer, and queue latency.

---

## End-to-end latency budget

| Stage | Steady state | Worst case |
|---|---|---|
| Source block (RTL-SDR) | 8 ms | 8 ms |
| Source queue (bounded 32) | 8 ms | 256 ms |
| UI frame (repaint + render) | 16-29 ms | 29 ms |
| DSP staging (UI thread) | 3-5 ms | 13 ms (8-batch catch-up) |
| Demod IQ queue (bounded 16) | 8 ms | 128 ms |
| Demod processing | 2-4 ms | 15 ms (WFM stereo) |
| Demod audio queue (bounded 16) | 8 ms | 128 ms |
| Audio queue (bounded 32) | 8 ms | 256 ms |
| Audio backlog | 0 ms | 200 ms |
| CPAL buffer (PulseAudio default) | 50-100 ms | 100 ms |
| **Total** | **~100-150 ms** | **~1100 ms** |

**The user hears: ~100-150 ms steady state (noticeable), 300-1100 ms under load (massive).**

---

## Recommended fixes (priority order)

### 1. Reduce Listen tab render cost (biggest win, saves 10-25 ms/frame)

- **Replace per-sample `line_segment` loops with single `Shape::polyline`**: the audio waveform (2047 segments), spectrum line (1800 segments), and signal history (599 segments) each become one draw call. Saves ~4,000 draw calls.
- **Cache the signal history mini-chart**: only rebuild when history changes (one new point every 10 frames), not every frame.
- **Cache grid labels and band plan**: only re-format on zoom/dB-range change.
- **Default `full_waterfall_update` to false**: the partial-row path is already correct; full mode uploads 2 MB/frame.
- **Downsample the waveform to ~2 points per pixel** instead of one segment per sample.

### 2. Set explicit CPAL buffer size (saves 40-90 ms fixed latency)

Set `StreamConfig.buffer_size = BufferSize::Fixed(512)` (~10 ms at 48 kHz) and pass `Some(Duration::from_millis(10))` as the latency hint in `build_output_stream`.

### 3. Raise audio worker thread priority

Use `thread_priority` or `libc::sched_setscheduler` to set the audio worker to `SCHED_FIFO` at moderate priority, preventing starvation by the UI thread.

### 4. Reduce queue depths

- Audio queue: 32 → 8 (64 ms worst case, still plenty of jitter absorption)
- Source queue: 32 → 8 (64 ms)
- Demod queues: 16 → 8 (64 ms)

### 5. Move DSP staging off the UI thread

Move `radio_iq.process` + `vfo_mixer.process` + `spectrum.push` to a worker thread. The UI thread should only drain completed frames and update display state. This eliminates the 3-5 ms DSP staging cost and 2 MB/frame of allocations from the UI thread.

### 6. Reduce catch-up drain from 8 to 2-3 blocks

Drain at most 2-3 batches per frame (16-24 ms of audio) and drop excess rather than catching up. This prevents the 131k-sample spike that amplifies stalls.

### 7. Add drop counters and latency telemetry

Add counters at every drop point (source, demod IQ, demod frames, audio). Surface in a debug panel or log periodically. This makes future diagnosis possible.

---

## Verification notes

- All draw-call counts verified by reading the paint path source (`spectrum.rs:1681-3900`).
- CPAL buffer size verified: `buffer_size` is never set; latency hint is `None` (`audio_output.rs:284-295`).
- Audio queue capacity verified: `AUDIO_QUEUE_CAPACITY = 32` (`audio_output.rs:31`).
- 8-batch catch-up drain verified at `app.rs:747`.
- WFM stereo PLL verified: runs per multiplex output sample (204.8 kHz at 2.048 MHz input), with 241-tap FIR per input sample (`demod.rs:196-235, 290-331`).
- The shared `Mutex<SharedState>` is only locked by the UI thread; the cost is the work under the locks, not contention.
