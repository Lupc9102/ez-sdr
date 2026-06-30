# EZ-SDR Autonomous Night Session
Started: 2026-06-30 (night shift)

## Mission
Make ez-sdr the best and easiest to use SDR app on the market for beginners.
Work autonomously: no questions, no waiting. Build + verify after each change.

## Build environment
Rust toolchain installed at ~/.cargo (stable 1.96). ALSA + SoapySDR dev headers
extracted to /tmp/opencode/sysroot (no sudo needed). Build helper: /tmp/opencode/build.sh
Usage: `/tmp/opencode/build.sh build --release -p ez-gui`

## Improvement Backlog (ordered by beginner impact)

### A. First-run / Onboarding (CRITICAL)
- [x] A1. Fix welcome-banner-shows-every-launch bug — add `welcome_seen` to AppConfig, persist, set true on dismiss
- [x] A2. Rewrite welcome banner: drop "rebuild with --features rtlsdr", lead with demo mode + "Listen to FM Radio" CTA button
- [x] A3. One-click "Listen to…" quick-start: band presets also start audio + set gain + filter BW so a beginner hears sound in one click

### B. Help System for Absolute Beginners (HIGH)
- [x] B1. Add "Getting Started / Your First Signal" help section (5-step recipe, insert as section 0)
- [x] B2. Add "No SDR hardware? Start here" help section (demo mode + file replay + sample IQ)
- [x] B3. De-jargon "What is SDR?" opening — lead with plain-language summary, push I/Q theory to a "deeper dive"
- [x] B4. Add "Why is the waterfall blank?" FAQ entry to Troubleshooting

### C. Empty States + Action Hints (HIGH)
- [x] C1. Make spectrum empty-state a clickable "▶ Start" button (not plain text)
- [x] C2. Add "what to expect" action hint under band-info ("💡 Press 🔊 Start Audio to hear: music/talk")
- [x] C3. Add "Record a 30-second sample" CTA to empty recorder list

### D. De-jargon Controls (MEDIUM)
- [x] D1. Add tooltips to Bias Tee / Direct Sampling / PPM in source_manager.ui
- [x] D2. Add inline explanations to spectrum toolbar (FFT/Win/Avg labels) — already had on_hover_text on all controls
- [x] D3. Add "❓" glossary badge to status bar (dBFS, SNR, squelch, VFO, BW, LPF, MSps, PPM, LO)
- [x] D4. Add tooltips to cryptic status bar labels (LPF, MSps) — already implemented

### E. Layout (MEDIUM)
- [x] E1. Move Start/Stop + source-mode selector to TOP of SDR panel (hoist above frequency)

### F. Polish (LOW)
- [x] F1. Clean up dump1090 lib warnings (unused vars / dead code) — optional
- [x] F2. Final warning-free build verification — clean build confirmed

## Session Log
- T+0:   Build env set up (Rust 1.96 + ALSA/SoapySDR sysroot). Fixed 18 compile errors + 3 warnings from prior session. Committed.
- T+30m: Completed beginner-friendliness audit (14 concrete improvements identified). Wrote new task list.
- T+60m: Fixed 5 compile errors from prior session (borrow lifetime, missing field, match type). Implemented A1–A3, B1–B4 (welcome banner, howto sections). Implemented C1 (clickable ▶ Start overlay on spectrum), C2 (band-info what-to-hear hint), C3 (recorder empty CTA), D1 (Bias Tee/Direct Sampling/PPM tooltips), E1 (Start/Stop hoisted to top of SDR panel). Clean build.
- T+morning: Resumed after context compaction. Fixed NaN crash risks in spectrum.rs (3 unwrap() panics — partial_cmp + pts.last). Fixed web_remote.rs network unwraps (Runtime/bind/serve). Fixed UNIX_EPOCH unwraps in tle_engine.rs + database.rs. Added D3 SDR Glossary popup (❓ button in status bar, 20-term reference). Added MQTT reconnection with atomic connection flag and auto-retry. Added recorder disk-space pre-flight check (<500MB warns user). Added "first signal" celebration flash (8s green message on first SNR>20). Added Band/Service column to scanner hits table (calls identify_frequency()). Dead code cleanup: removed unused SharedState.db, iq_tx, audio_tx fields; removed dead min_level() function; removed unnecessary #[allow(dead_code)] suppressor annotations. All items complete. Clean build.
