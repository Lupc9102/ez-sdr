# ez-web — Handoff / Remaining Work

Status as of 2026-07-10. This document is a handoff for another agent to continue the
**web-architecture migration**: replacing the native `ez-gui` desktop paradigm with a
browser UI (`ez-web/`) served by the headless `ez-daemon`, which now exposes a REST API +
control/data WebSockets alongside its legacy TCP control plane.

The frontend is **fully written and builds clean**, the daemon serves it, and the entire
client↔daemon protocol has been **verified live end-to-end**. There is **one known
rendering bug** (spectrum canvas paints black) and a handful of **unverified-in-browser**
features. Details below.

---

## Hard constraints (do not violate)

1. **No Workflows.** Do not use any workflow-engine framework — neither in your own agentic
   execution (no `Workflow` tool / declarative multi-stage scripts) nor in the app codebase
   (no workflow-engine abstractions in Rust or TS). Use direct async primitives, plain
   channels/WebSockets, native control flow only. The current code already respects this.
2. **Never touch `.claude/settings.json`.** It contains an uncommitted, third-party
   credential change (`ANTHROPIC_API_KEY` / `ANTHROPIC_API_BASE` → `cc.freemodel.dev`) that
   was flagged as suspicious and is **not ours**. Do not read it into commits, stage it, or
   modify it without explicit user instruction. Keep it out of every `git add`.
3. **UI must be verified in a real browser** before claiming a UI task complete — or the
   inability to do so must be stated explicitly. See "Verification" below for why the
   headless path is unreliable here and what to do instead.
4. After modifying code, run `graphify update .` (AST-only, no API cost) — see Housekeeping.
5. Commit only once a task is complete and verified (build/tests pass). Don't leave finished
   work uncommitted. Exclude `.claude/settings.json` (constraint 2).

---

## Architecture (quick orientation)

- **Rust workspace** members: `dump1090, ez-gui, lrpt-decode, ez-proto, ez-daemon`.
  `ez-web` is intentionally a **standalone npm/TypeScript project**, NOT a Cargo member.
- **`ez-daemon` web layer** (`ez-daemon/src/web/`): `api` (axum REST `/api/*`),
  `ws` (`/ws/control` JSON control socket; `/ws/stream/{kind}/{id}` binary/text data
  sockets), `wire` (hand-rolled little-endian binary framing), `serve` (static files).
- **`ez-daemon` CLI** (`ez-daemon/src/main.rs`): `--web-listen` (default `127.0.0.1:7891`),
  `--web-static-dir` (default `./ez-web/dist`), `--listen` (legacy TCP, default `:7890`),
  `--source synthetic|replay` (**`synthetic` needs no SDR hardware — use it for testing**),
  `--freq`, `--sample-rate`, `--recording-dir`, plus `--replay-*` flags.
- **`ez-web` frontend** — framework-free TS + Vite. No React/Vue runtime by design.
  - `src/main.ts` — bootstrap; threads callbacks between UI panels and api/* clients.
  - `src/api/` — `control.ts` (control WS), `stream.ts` (data WS), `rest.ts` (REST),
    `wire.ts` (binary decoders).
  - `src/ui/` — one plain class per DOM subtree: `hardware-panel`, `display-panel`,
    `channel-list`, `channel-controls`, `aircraft-panel`, `spectrum-view`.
  - `src/workers/spectrum-worker.ts` — OffscreenCanvas spectrum+waterfall renderer,
    runs entirely off-thread. Main thread never touches a raw spectrum frame.
  - `src/audio/monitor.ts` — Web Audio playback of demodulated audio.
  - `src/render/colormap.ts` — waterfall palettes.

### Wire formats (verified live — trust these)

- **Spectrum binary frame** (`/ws/stream/spectrum/{id}`): byte 0–7 `center_hz` u64 LE,
  8–11 `sample_rate_hz` u32 LE, 12–19 `timestamp_ms` u64 LE, 20–23 `bin_count` u32 LE,
  24+ `bin_count × f32` LE dB magnitudes. For a 2048-bin FFT: `24 + 2048*4 = 8216` bytes.
  Decoder: `src/api/wire.ts::decodeSpectrum` — byte-exact match confirmed.
- **Control WS** (`/ws/control`): server pushes `Welcome{protocol_version,active_channels}`
  then `Hardware(status)` immediately on connect, then a `Hardware` tick ~every 250ms.
  No client handshake required. Confirmed live.
- **ADS-B** (`/ws/stream/adsb-packets/{id}`): each **JSON text** frame is a bare
  `Vec<AircraftTelemetry>` array (NOT wrapped in a `ServerEvent::Aircraft` envelope).
  Decoder path exists (`wire.ts::decodeTelemetry` / stream `onText`); **not yet live-tested.**

---

## Current state — DONE and VERIFIED

- `ez-web` builds clean: `npm install`, `npx tsc -b`, `npx vite build` all succeed with no
  errors. Output in `ez-web/dist/`, served correctly by the daemon (correct content-types
  confirmed via curl; JS served as `text/javascript`).
- Full Rust workspace compiles with **zero warnings** (`cargo build --workspace`).
- **Protocol verified live** against a running `--source synthetic` daemon:
  - REST: status, hardware, channel CRUD, demod-mode/volume/squelch, recording start/stop.
  - Control WS: `Welcome → Hardware → periodic Hardware`; `SetGain` round-trips.
  - Spectrum data WS: 8216-byte binary frames decode byte-exact.
- **Dashboard shell renders correctly in-browser** (headless screenshot): topbar +
  connection status ("connected"), hardware panel populated with live values
  (synthetic / 433 MHz / 2.048 MSps / 20 dB gain), display/palette panel, channel list
  showing the auto-created Spectrum channel (id 0, 2.048 MHz BW), channel-create form,
  channel-controls, aircraft panel. CSS/layout all good.
- **Spectrum data reaches the browser and the worker runs.** Instrumented capture showed
  the `/ws/stream/spectrum/0` socket opening and **460+ binary frames (`len=8216`)**
  arriving, AND the frequency-scale overlay rendering with the real center frequency —
  which only happens after the worker posts `meta` back, i.e. the worker received frames,
  decoded them, and executed its paint calls. See the bug below for the one gap.
- **Isolated OffscreenCanvas + Worker smoke test rendered perfectly** in this same headless
  Firefox (a worker painted red/green rects to a transferred OffscreenCanvas and they
  showed) — so the rendering *mechanism* works in this environment.

- **Spectrum canvas rendering bug (Bug #1) RESOLVED 2026-07-10.** Root cause was the
  `transferControlToOffscreen` + in-worker-resize compositing failure in Firefox (not the
  data path and not the dB range). Fixed by switching to worker-owned `OffscreenCanvas` +
  `transferToImageBitmap` + main-thread `drawImage` blit (see Bug #1 section). Also added
  automatic dB auto-ranging. `npm run build` + `npm test` both pass; the live app was
  confirmed to composite a non-black spectrum in headless Chromium. Firefox confirmation is
  still the user's to make (headless Firefox GL is broken in this environment; no display).

---

## KNOWN BUG #1 — spectrum canvas renders black (top priority) — RESOLVED 2026-07-10

**Status:** Fixed and verified rendering in headless Chromium (the app's `#spectrum-canvas`
composites a non-black trace+waterfall: ~68% non-black pixels, peak luminance 507/765).
Firefox verification is still pending on the user's machine — see "Verification" below.

**Root cause (revised):** The original code used `canvas.transferControlToOffscreen()` and
let the worker resize its `OffscreenCanvas` from inside the worker. Resizing a *transferred*
OffscreenCanvas from the worker fails to re-composite on the main-thread placeholder in
Firefox, leaving the canvas displaying a stale/empty 300×150 bitmap — a fully black canvas
even though the worker was painting (meta posted, overlay rendered). The earlier "pre-size the
main-thread canvas before transfer" experiment was tested in isolation and found to make
things *worse* in Chromium (and Firefox differs from Chromium here), so it was abandoned.

**Fix:** Replaced `transferControlToOffscreen` with the canonical cross-browser pattern. The
worker now owns its **own** `OffscreenCanvas` (`new OffscreenCanvas(w,h)`), paints the spectrum
+ waterfall onto it, snapshots via `canvas.transferToImageBitmap()`, and posts the `ImageBitmap`
to the main thread, which blits it onto a normal `#spectrum-canvas` via `drawImage`. The
main-thread canvas is never transferred, so it can be resized directly and composites reliably.
The main thread still never touches a raw spectrum frame — only finished bitmaps — preserving
the original off-thread design intent. Changes: `src/ui/spectrum-view.ts` (normal canvas +
`bitmap` blit), `src/workers/spectrum-worker.ts` (own OffscreenCanvas + `transferToImageBitmap`),
`src/workers/spectrum-protocol.ts` (new `bitmap` message, `init` now carries width/height).

**Bonus fix (suspect #2):** The worker now auto-ranges the dB window (smoothed EMA of live
frame min/max) instead of the fixed -120..0 default, so synthetic frames (≈ -86..-10 dB) render
with full contrast instead of near-black. A manual `range` message from the main thread still
overrides it.

---

**Symptom (historical):** In the running app the `#spectrum-canvas` area was solid black; no dB
trace, no waterfall. The frequency-scale overlay (bottom axis labels) DOES render.

**What is proven working (ruled out as the cause):**
- WS connect, binary framing, `decodeSpectrum` — frames arrive, `len=8216`, valid.
- The worker receives frames and runs `handleFrame` → `drawSpectrum` → `drawWaterfallRow`
  → posts `meta`. We know all of this executed because the overlay only paints after a
  `meta` message with a real `centerHz`, and neither draw call threw (an exception would
  have prevented the `meta` post that follows them).

**So the defect is in the display/compositing of the worker-painted OffscreenCanvas, not
the data path.** The difference vs. the working smoke test: the smoke-test canvas had
explicit `width/height` attributes and was painted at native size; the real
`#spectrum-canvas` has NO width/height attributes (so its transferred OffscreenCanvas
defaults to 300×150), is styled `position:absolute; inset:0; width:100%; height:100%`
(`ez-web/src/style.css:101`), and is resized *after* transfer by a `{type:"resize"}`
message the worker applies via `canvas.width/height = …` (`spectrum-view.ts` constructor +
`resize()`, worker `spectrum-worker.ts:52`).

**Prime suspects, in order:**
1. **Resize/paint ordering or the transferred-canvas display size.** The placeholder
   `<canvas>`'s displayed content may not reflect the worker's `canvas.width/height`
   reassignment as expected. First diagnostic (cheap, decisive): in
   `spectrum-worker.ts`, add an **unconditional** bright `ctx.fillStyle="#f0f";
   ctx.fillRect(0,0,canvas.width,canvas.height)` at the end of the `init` and `resize`
   handlers, rebuild, and load the app in a real browser. If magenta shows → paint reaches
   the screen and the issue is that real frames map to near-black colors (adjust dB range,
   suspect #2); if it stays black → the display/compositing of the OffscreenCanvas is the
   problem (sizing/CSS/transfer).
2. **Default dB range makes the signal near-black.** `main.ts` never calls
   `spectrumView.setDbRange(...)`, so the worker uses defaults `minDb=-120, maxDb=0`.
   Live synthetic frames measured `dbRange≈[-86,-10]`. With the "Classic" palette
   (`#000020 → #0000ff → #00ff00 → #ffff00 → #ff0000`), low-normalized values are very
   dark blue and can read as black in a screenshot. Consider wiring an auto-range (or a
   sensible default like `minDb=-100, maxDb=-20`) and confirm. This alone probably won't
   explain a *fully* black canvas including the dB gridlines the worker draws, but it's
   worth eliminating.
3. **CSS stacking:** confirm `#spectrum-overlay` (drawn on top, transparent via
   `clearRect`) isn't masking `#spectrum-canvas`. Both are `absolute; inset:0`. The overlay
   should be transparent except axis/markers; verify it isn't filling an opaque background.

**Recommended workflow:** open `http://127.0.0.1:7891/` in a **real interactive browser**
(the user already has Firefox running) and use DevTools — check the Console for worker
errors, inspect `#spectrum-canvas` rendered size vs. its OffscreenCanvas backing size, and
step through with the magenta-fill diagnostic. This sidesteps the headless timing problem
entirely (see Verification).

---

## Remaining implementation / verification tasks

Ordered roughly by priority. Items marked (verify) are believed code-complete but need
real-browser confirmation.

1. **Fix spectrum canvas rendering** (Bug #1 above). **DONE 2026-07-10** — switched to
   worker-owned `OffscreenCanvas` + `transferToImageBitmap` + main-thread blit; verified
   non-black render in headless Chromium (see Bug #1 section). Firefox confirmation pending
   on the user's machine.
2. **(verify) Live audio monitoring** — `src/audio/monitor.ts` + `main.ts::onToggleMonitor`
   are wired but never exercised with real audio playback. Create an `Audio` channel, click
   monitor, confirm the Web Audio pipeline plays demodulated audio and the
   `/ws/stream/audio/{id}` decode (`wire.ts::decodeAudio`) is correct. No audio-device test
   has been done.
3. **(verify) ADS-B / aircraft feed** — `src/ui/aircraft-panel.ts` + `main.ts` wiring +
   `main.ts::AircraftPanel.onCreateChannel` are done and the REST create path works, but
   the actual `/ws/stream/adsb-packets/{id}` **text** stream (bare `Vec<AircraftTelemetry>`
   JSON array) has not been live-tested. Confirm frames decode and the panel populates.
   Note: synthetic source may not emit ADS-B — you may need `--source replay` with a
   suitable capture, or to confirm the panel's empty/idle state is correct.
4. **Demod controls polish** — mode/volume/squelch are wired and REST-verified. **Bandpass
   filter-width (bandwidth) editing is intentionally unwired** (visual-only drag in
   `spectrum-view.ts`) because the daemon has no retune command — see Backend Gaps. Decide
   whether to (a) leave it visual-only with the current explanatory comment, or (b) add the
   backend command and wire it. Currently (a).
5. **Broader browser QA** — exercise channel create/select/delete-attempt, palette
   switching, recording start/stop UI, error-banner behavior, reconnect (kill+restart the
   daemon and confirm the UI self-heals via `ensureDefaultsAndRefresh` on the next
   `Welcome`).
6. **Consider unit/integration tests** for the wire decoders (`wire.ts`) and the daemon web
   layer — the DoD mentions "unit + integration tests." Check what exists in
   `ez-daemon/src/web/` and `ez-web` (currently none in ez-web) and add coverage for the
   binary framing at minimum. Keep it framework-free.
   **DONE 2026-07-10** — added `ez-web/tests/wire.test.ts` (framework-free, `node --test`):
   byte-exact decoding of spectrum/audio/telemetry binary frames + colormap LUT sanity.
   `npm test` passes 5/5.

---

## Backend gaps discovered (Rust side — out of scope so far, documented for decision)

These were found via live testing. They were **not** fixed (to avoid unrequested backend
changes); the frontend works around them. An implementing agent may choose to close them.

- **`set_demod_mode` doesn't update the `ChannelSpec` mirror.**
  `DaemonState::set_demod_mode` (`ez-daemon/src/state.rs` ~L337) mutates the live
  `AudioPipeline` (audio DOES change) but never updates `ChannelSpec.demod_mode` in
  `self.channels`, so `GET /api/channels` reports the *creation-time* mode forever.
  **Frontend workaround:** `main.ts::onSetDemodMode` optimistically patches local state
  instead of refetching (see the comment there). A proper fix updates the spec server-side.
- **No retune command for an existing channel.** `DaemonState::subscribe()` is a no-op
  re-attach for an already-existing id — there is no way to change a live channel's
  frequency/bandwidth. This is why passband drag-editing is visual-only (task #4).
- **No effective channel teardown.** There's no working delete/tear-down command. The UI
  reduce-to-max-id scheme for new channel ids assumes ids are never reclaimed.

---

## How to build, run, and test

```bash
# Frontend build (from ez-web/):
cd ez-web
npm install
npx tsc -b            # typecheck
npx vite build        # emits ez-web/dist/

# Rust build (from repo root):
cargo build --workspace           # zero warnings expected
# (release build strongly recommended for testing — see note)

# Run the daemon (no SDR hardware needed), from repo root:
./target/debug/ez-daemon --source synthetic --web-static-dir ./ez-web/dist
# then open http://127.0.0.1:7891/

# Quick protocol smoke tests (Node, no browser):
#   /tmp/ws-control-test.mjs   — control WS handshake + SetGain round-trip
#   /tmp/ws-spectrum-test.mjs  — spectrum binary frame decode
# (these are ephemeral /tmp scripts, not committed; recreate if gone.)
```

**IMPORTANT perf note for testing:** the **debug** `ez-daemon` pins ~150% CPU processing the
2.048 MSps synthetic stream. Under a headless browser this starves the event loop and every
async hop takes seconds. **Build/run the daemon in `--release`** (`cargo build --release
-p ez-daemon`, then `./target/release/ez-daemon …`) for smooth browser testing, and/or lower
the load with `--sample-rate 256000`.

---

## Verification — why headless screenshots are unreliable here

- No Chromium / Playwright / Puppeteer / geckodriver / Selenium in this environment.
- Firefox 151 is installed. `firefox --headless --screenshot` **captures at the page `load`
  event with NO wait for post-load async JS** (WS connect, fetch, worker paint), so it
  routinely captures an un-hydrated page.
- Workaround used this session: embed a deliberately **slow `<img>`** (a Python server that
  `sleep`s N seconds before responding) so the `load` event — and thus the screenshot — is
  delayed until async work settles. Files (ephemeral, /tmp): `/tmp/ff-shots/delay-server.py`
  (edit `DELAY_SECONDS`), and instrumented pages. This works but is finicky under the CPU
  contention above.
- **Best path forward: use the real interactive Firefox** the user already has open — just
  navigate to `http://127.0.0.1:7891/` and use DevTools. This is the recommended way to
  chase Bug #1 and to verify audio/ADS-B.

---

## Housekeeping before commit

- **Remove debug artifacts from `ez-web/dist/`** — this session wrote
  `dist/debug-dashboard.html` and `dist/debug-worker-test.html` for instrumentation. They
  are in the build output dir (git-ignored if `dist/` is ignored — check) but should not be
  shipped or committed. A clean `vite build` regenerates `dist/` without them.
- **Run `graphify update .`** after code changes (project CLAUDE.md requirement; AST-only,
  no API cost). This was NOT yet run for this session's changes.
- **Commit** the completed frontend once Bug #1 is resolved and verified. Review
  `git status` first; **exclude `.claude/settings.json`** (constraint 2). The tree also has
  unrelated untracked dirs (`ez-web/tile_cache/`, `tile_cache/`, `ez-gui/tile_cache/…`) and
  modified `graphify-out/*` — don't sweep those in blindly; stage `ez-web/` source
  deliberately.

---

## Files changed/created this session (all in `ez-web/`, all build-clean)

- `src/main.ts` (new) — app bootstrap wiring all panels + sockets + REST.
- `src/vite-env.d.ts` (new) — `/// <reference types="vite/client" />` so `tsc` resolves the
  Vite `?worker` import.
- `src/api/rest.ts` — `getRecording` given an explicit `request<RecordingStatus>` type arg.
- `src/ui/channel-list.ts`, `src/ui/hardware-panel.ts`, `src/ui/spectrum-view.ts` — dropped
  `private readonly` on constructor params that were never read as fields (TS6138).

Everything else in `ez-web/src/` was written in prior sessions and cross-checked this
session against `main.ts`'s usage — signatures all match.
