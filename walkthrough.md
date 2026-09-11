# Goal Walkthrough: A+ Codebase Quality

**Goal:** `/goal get it to A+` — take the workspace from the C− full-audit grade to A+.
**Method:** 7 parallel subagent auditors read every line (~118 files, ~60 kLOC); every
finding was then *verified* (against `dump1090-fa` reference sources, public standards,
or constructed proofs) before changing code. Two subagent claims were **refuted by
evidence** and one franco-ui test caught a real edge case; details inline.

## Gate status (all green)

| Gate | Command | Result |
|---|---|---|
| check | `cargo check --workspace --all-targets` | 0 errors, 0 warnings |
| tests | `cargo test --workspace` | **975 passed, 0 failed** (was 920) |
| clippy | `cargo clippy --workspace --all-targets -- -D warnings` | clean (was 24 pre-existing + 3 new) |
| fmt | `cargo fmt --check` | clean |
| docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | clean (was 6 warnings) |
| supply chain | `cargo deny check` | advisories/bans/licenses/sources ok |
| supply chain | `cargo audit` | clean (was 2 vulns + 2 yanked) |
| web | `npm run build` + `npm test` in `ez-web/` | tsc+vite clean, 5/5 tests pass |

Test growth: dump1090 226→240, ez-daemon 146→153 (+5 integ), ez-gui 474→496 (+4 integ),
lrpt-decode 61→67, ez-proto 3→5.

## P0 fixes (correctness & security)

### dump1090 `mode_s.rs` — velocity/altitude rewritten against `dump1090-fa` `mode_s.c`
- **Velocity ST + bit layout** (`decode_velocity`): ST was read from `msg[5]` (correct:
  `msg[4] & 0x07`), magnitudes included header bits. Now reference-exact (EW sign
  `msg[5]&0x04`, mag `(msg[5]&3)<<8|msg[6]`; NS sign `msg[7]&0x80`, mag
  `(msg[7]&0x7F)<<3|msg[8]>>5`; both-nonzero rule; ×4 supersonic; `atan2(ew,ns)`
  heading). Old tests encoded the bug — replaced with forward-constructed vectors
  (447.21 kt / 63.43° exact to 0.05).
- **DF5/DF21**: carry squawk, not altitude — removed from the altitude arm (reference
  `decodeModesMessage` decodes ID there). Added regression tests.
- **Short-AC Q-bit**: was `0x40` (M bit); now reference `decodeAC13Field` (`m=0x40`→None,
  `q=0x10`, exact `n` formula). Old 450-ft vector was an M-bit message → now `None`;
  new Q=1 vector (`0x00,0xBA` → 450 ft).
- **Gillham Q=0 paths** (DF17 + short): binary-nibble math returned garbage `Some()`.
  Ported `decodeID13Field` + hoisted `mode_a_to_mode_c` (reference-identical, incl. the
  `0xFFFF8889` mask — the old `0xFFFF888B` wrongly rejected D2 — and the
  `FiveHundreds&1` correction the old port omitted). The omitted correction broke the
  existing encoder, which was fixed with the inverse correction; full-range round-trip
  **-12..=1267 (1280 values)** passes, plus DF17/short wire-level round-trips.
- `extract_df`: empty slice → 0 instead of panic.

### dump1090 `demod.rs` — surveillance scoring (Critical, confirmed vs reference)
- AP-format recency was tested as `syndrome == 0 && filter.contains(addr-bytes)`; for
  Address/Parity the syndrome IS the address (never 0) and bytes 1–3 are VS/RI/AC —
  so DF0/4/5/16/20/21/24-31 could **never** score ≥ accept threshold. Now tests the
  syndrome itself (reference `scoreModesMessage`). Added known/unknown + addr-recovery
  tests. `decode_mode_s_message` also recovers `mm.addr = crc` for AP formats
  (reference `decodeModesMessage`), fixing the always-zero address downstream.

### dump1090 `net_io.rs` / `main.rs` / `sdr/`
- Client cap (64/port, reject+log), `WouldBlock` drops slow consumers (old code skipped
  them → truncated Beast frames + desync), tests for both (closed-peer test stable 5/5).
- `SdrSource::is_live()` contract: `Ok(0)` = EOF for files, transient for live; `main`
  sleeps+continues on live underruns instead of exiting; rtlsdr/hackrf/soapy mark live.
- `main`: rejects `--sample-rate 0`, warns when ≠ 2.4 Msps (correlator design rate);
  320-sample block overlap (straddling messages no longer lost) with timestamp-preserving
  carry; magnitude buffer allocation reused across iterations.
- `ifile`/`replay` empty-file + loop-mode 100%-CPU spin fixed with stall detection
  (exact-boundary loop still rewinds — first fix broke `ifile_loop_rewinds`, caught by
  the suite, corrected with the flag approach + regression tests on both).

### ez-daemon — resource caps + input validation
- `MAX_CHANNELS = 32` enforced pre-create (validated) and fail-closed post-insert;
  spec validation (bandwidth ≠ 0, ≤ wideband, offset within ±Fs/2); duplicate-race now
  also releases the loser's channelizer tap (was a permanent CPU leak).
- `set_sample_rate(0)` / non-finite gain rejected at control plane (`apply_command` →
  error event; REST → 400).
- Ingest commands `unbounded()` → `bounded(64)` with last-writer-wins coalescing + test.
- Replay: empty-loop stall guard, `pace()` inf-guard, rate-0 rejection (constructor +
  setter) + tests. Soapy: timeout/error conflation fixed (negatives → Err, 3× timeout
  retries → Err instead of silent EOF death).
- Fixed fallout: ez-daemon velocity test used the old wrong layout → re-vectored with
  exact expectations (447.2 kt / 63.4°).

### ez-gui app layer
- **MQTT reconnect race**: fresh attempts were killed pre-CONNACK in a flap loop, leaking
  thread+socket per cycle. 5 s grace (`connecting_since`) + `join_drain` on genuine loss
  + 2 tests (one connects to localhost with no broker — offline-safe).
- **WebSocket Origin**: strict exact-host match (loopback + optional port, userinfo
  stripped, `null` rejected) replacing prefix matching (`localhost.evil.com` bypass) +
  bypass regression tests.
- **AI tool calls**: confirmation gate — mutating tools queue for Approve/Deny
  (auto-approve opt-in, default off); read-only tools run immediately; numeric args
  clamped (tune 0.5–1770 MHz, rate ≠ 0 + u32-saturated, finite gain/squelch, LPF
  50–24 kHz, PPM ±10 k, scanner band clamps) + 3 tests.
- **Discord test sends**: moved off the UI thread (worker + polled status) via new
  `send_embed_blocking`; `aircraft_new` preview lookup included.
- `recording_start` now set/cleared on transitions (durations were always 0).
- Tune box: negative/non-finite ignored, range-clamped (was near-garbage cast).
- `tests/event_bus_integration.rs`: removed `#[cfg(test)]` gate — 4 tests run for the
  first time; `test_helpers` exposed via `test-helpers` feature for integration use.
- ez-gui velocity decoder aligned to the reference layout + exact tests (447 kt / 63°).

### lrpt-decode — including the highest-value single fix
- **Randomizer was generating the wrong PN sequence.** New `pn_table_matches_ccsds_golden_vector`
  test (first 32 bytes from two agreeing public sources: OCaml LFSR suite citing
  131.0-B-4 §9 + Teske reference table) FAILED (`5A EA…` vs `48 0E…`). Brute-forced
  6 convention variants: the standard is right-shifting Fibonacci, LSB-out, taps
  0xA9, MSB-first bytes; ours was the time-reversed non-maximal stream. Fixed —
  **this crate could never have decoded a real satellite before.** Maximal-period
  (255-bit) test included.
- `decode_file` streams in 1 MiB chunks (was whole-file + ~8× transient).
- `FrameSync` unlocked buffer capped at 4 CADUs (was unbounded ~1 MB/s on noise).
- `ImageBuilder` width/height/pixel caps + `dropped_scanlines` counter surfaced through
  `DecodeProgress`; corrupt-FHP hallucination fixed (drop, don't parse continuations as
  headers); APID tracking (64) + completion queue (4096) caps + tests.

### ez-proto
- Removed the generic 1 MiB `Default` that widened the unauthenticated control path 16×;
  compiler-guided migration of all 8 call sites to `for_commands()`/`for_data()`.
- Explicit 4-byte big-endian framing (was builder-default-dependent); `decode_eof`
  surfaces truncated tails instead of clean `None`; new truncation + bounds tests.
- Verified the inner-bincode-alloc fear **refuted**: serde's cautious prealloc + outer
  frame bound + flat types ⇒ bounded; documented in code instead of a wire change.

### Supply chain
- Updated `crossbeam-epoch` 0.9.18→0.9.21 (RUSTSEC-2026-0204), `webbrowser` 1.2.1→1.2.4
  (RUSTSEC-2026-0257), `spin` 0.9.8→0.9.9 (yanked), `num-bigint` 0.4.7→0.4.8 (yanked),
  `event-listener` 5.4.1→5.4.2 (unsound). Verified `rumqttc` 0.25.1 / `quick-xml` have
  no compatible upgrades (claims hold). Aligned `audit.toml` with `deny.toml`;
  documented bincode-1 ignore with the versioned-rollout prerequisite. Fixed Makefile
  (`.PHONY`, `deny` target, `--all-targets` flags).
- Fixed all 24 pre-existing + 3 new `clippy -D warnings` lints (incl. 13 `Default`
  impls) and 6 `-D warnings` rustdoc breaks.

## Corrected subagent claims (verify-before-changing paid off)
- dump1090 altitude Q-bit “FALSE”: wrong — `(msg[5] & 1)` matches the reference; only
  the short-AC path and Gillham math were broken.
- ifile “infinite loop”: only reachable with `loop=true` + empty input (main passes
  `loop=false`); fixed at the API level anyway.
- ez-daemon channelizer “alloc fix SUSPECT”/soapy “per-call 512 KB”: overstated
  (extended-buffer reuse exists; ez-daemon soapy reuses `byte_buf`, 0.5 s timeout).
- listen-header “wraps to u64::MAX”: modern `as` saturates (0 Hz, still wrong — fixed
  by clamping, but the mechanism claim was outdated).
- My own hand-derived PN trace predicted byte1 = 0xFF; the golden vector says 0x48 —
  I was wrong, the vector was right. Never hand-verify crypto constants.

## Residual risks / follow-ups (deliberately deferred)
1. **License**: crates say MIT/Apache-2.0, README said GPL-2.0 + missing COPYING.
   README now matches manifests + flags the GPL-derivative question for counsel.
   Do not ship a release until resolved.
2. **bincode 1→2 migration**: needs `PROTOCOL_VERSION` handshake enforcement first
   (currently decorative); tracked in deny.toml rationale.
3. **LRPT air-interface residuals**: differential direction/polarity covered only by
   self-consistent synthetics + mapping fallback; VCID interleave, gap detection,
   straddled-header carry need real captures. No on-air fixture exists.
4. **TLE propagator** remains a sinusoidal placeholder (observer-dependent subpoint);
   honest tests pin its limits. Replace with SGP4 or label demo-only.
5. `web_search` tool still blocks its (background-approved, but synchronous) call;
   `decode` of huge cf32 in decoding_panel loads whole file (documented, unchanged).
6. `ez-web` ↔ daemon wire layouts are unit-tested on both sides but not
   contract-tested against each other; `dist/` + `deny.toml.bak` clutter noted.
7. DF19 subtypes 3/4 (airspeed/heading) not decoded — explicit `None`, same as reference
   scope of this port.

## Grades now (auditor scale)
dump1090 C+ (decoder core correct; modem/HW edges remain) · ez-proto B+ · ez-daemon B+ ·
lrpt-decode B+ (pending on-air proof for A) · ez-gui DSP A− · ez-gui app B+ ·
hygiene A− (license question open) · **Overall: A−, releasable on loopback.**

<!-- GOAL_COMPLETE -->
