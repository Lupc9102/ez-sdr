# EZ-SDR Refactor: Detailed Handoff Plan

**Prepared by**: Claude Code CLI, Opus 5 (1M context)  
**Date**: 2026-09-10  
**For**: Next agent continuing this refactor

---

## Executive Summary

This is a **complete refactor** of ez-sdr to transform it from a functional but messy application into a truly beginner-friendly, unified SDR tool that replaces dump1090, sdrpp, and satdump.

**Current Status**: Project compiles clean, zero warnings. Analysis complete. Task tracking document created (`EZ_SDR_REFACTOR_TASK.md`). Ready for implementation phase.

**Your Mission**: Execute the phased refactor plan, starting with completing the three critical tasks from Plan#2.md (ADS-B map zoom, tutorial removal, AI model updates), then continue to UX simplification and architecture cleanup.

---

## Project Overview

### What This Is
A cross-platform SDR application combining:
- Real-time spectrum analyzer/waterfall
- Multiple demodulation modes (AM/FM/SSB/WFM)
- Satellite tracking & image decoding
- ADS-B aircraft tracking
- AI assistant integration
- Web remote control
- All in one GPU-accelerated GUI

### Architecture (5 Rust Crates)
```
dump1090/       ADS-B Mode-S decoder library
ez-gui/         Desktop app (egui/eframe) - monolithic, runs SDR directly
ez-daemon/      Headless daemon with REST API + WebSocket
ez-web/         TypeScript web frontend (Vite, framework-free)
lrpt-decode/    LRPT satellite image decoder
```

### Current State
✅ **Compiles**: `cargo build --workspace` succeeds, zero warnings  
✅ **Tests exist**: Framework in place, though coverage has gaps  
✅ **Feature-complete**: All major features implemented  
✅ **Recent fixes**: Spectrum canvas rendering bug fixed 2026-07-10  

❌ **UX Problems**: Too complex for beginners, no guided workflows  
❌ **Known Bugs**: ADS-B map zoom over-sensitive, tile caching issues  
❌ **Dead Code**: Tutorial system partially removed, outdated AI references  
❌ **Documentation**: README technical, lacks beginner tutorials  

---

## Key Files You Must Read

### Immediate Context
1. **`EZ_SDR_REFACTOR_TASK.md`** (just created) - Main task tracker with checkboxes
2. **`Plan#2.md`** - Exact implementation specs for Tasks 1-3 (lines verified)
3. **`README.md`** - Current feature list, build instructions
4. **`CLAUDE.md`** (project root) - Project-specific conventions

### Background Context
5. **`plan#1.md`** - Web architecture migration (completed 2026-07-10)
6. **`task.md`** - Original web handoff document
7. **`~/.claude/CLAUDE.md`** - Global agent protocols (subagent rules, sprite gen, etc.)

### Architecture Reference
8. **`ez-gui/src/app.rs`** - Main desktop app structure (3586 lines)
9. **`ez-daemon/src/main.rs`** - Daemon CLI entry point
10. **`ez-web/src/main.ts`** - Web frontend bootstrap

---

## Immediate Next Steps (Phase 1)

### Task 1: Fix ADS-B Map Zoom (6 subtasks)
**File**: `ez-gui/src/adsb_panel.rs`  
**Status**: Not started  
**Details**: See Plan#2.md lines 14-238 for exact implementation

Current problem: Trackpad generates many small scroll deltas, each triggers full zoom level jump. No pinch gesture support.

**Subtasks**:
1. Add zoom accumulator + throttling constant (dampen small deltas)
2. Implement bounded LRU tile cache (prevent memory growth)
3. Add concurrent download limiting (max 8 simultaneous)
4. Implement disk cache (`tile_cache/` directory)
5. Add adjacent zoom level prefetching
6. Test in real browser (Firefox/Chrome) - verify trackpad/pinch

**Implementation Guide**: Plan#2.md has:
- Exact line numbers (verified 2026-07-10)
- Complete code blocks to insert
- Field names, constants, formulas
- Integration points clearly marked

**Testing**: Manual GUI test required - launch app, open ADS-B tab, test zoom with mouse wheel and trackpad.

### Task 2: Remove Tutorial System (6 subtasks)
**Files**: `tutorial.rs`, `main.rs`, `user_level.rs`, `app.rs`, `config.rs`  
**Status**: Not started  
**Details**: See Plan#2.md lines 240-343

Current problem: Tutorial system exists but confusing. Restart button present. Dead code paths.

**Subtasks**:
1. Delete `ez-gui/src/tutorial.rs` entirely
2. Remove tutorial imports from `main.rs`
3. Delete `TutorialState` struct from `user_level.rs`
4. Remove `Tab` enum and `focus_tab()` from `app.rs`
5. Remove config fields (`tutorial_seen`, `tutorial_step`, `welcome_seen`)
6. Verify How-To panel and antenna checklists still work

**Critical**: Do NOT touch `howto_panel.rs` or `antenna_checklist.rs` - these are separate systems.

**Testing**: Launch app, verify no tutorial dialog appears. Check Settings → User Experience, confirm restart button gone but Knowledge Level slider still works.

### Task 3: Update AI Model References (2 subtasks)
**Files**: `config.rs`, `howto_panel.rs`  
**Status**: Not started  
**Details**: See Plan#2.md lines 345-376

Current problem: Default AI models reference old Claude 3.x Haiku, should be 4.5.

**Changes**:
- OpenRouter: `anthropic/claude-3-haiku` → `anthropic/claude-haiku-4.5`
- Anthropic: `claude-3-5-haiku-20241022` → `claude-haiku-4-5-20251001`
- 7 total string replacements (5 in config.rs, 2 in howto_panel.rs)

**Testing**: Open AI tab, verify model defaults show new values. If API key available, test message streaming.

---

## After Phase 1: Commit and Proceed

Once Tasks 1-3 complete:
```bash
cargo build --workspace
cargo clippy --workspace --all-targets
cargo test --workspace
graphify update .
git add ez-gui/src/adsb_panel.rs ez-gui/src/config.rs ez-gui/src/howto_panel.rs ez-gui/src/main.rs ez-gui/src/user_level.rs ez-gui/src/app.rs
git rm ez-gui/src/tutorial.rs
git commit -m "fix: ADS-B map zoom, remove tutorial system, update AI models

- Add zoom accumulator with 50pt/level threshold for smooth trackpad
- Implement LRU tile cache (400 tiles ~100MB) with disk backing
- Limit concurrent downloads to 8, prefetch adjacent zoom levels
- Remove tutorial system entirely (tutorial.rs, TutorialState, Tab enum)
- Migrate to claude-haiku-4.5 as default AI model

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

**Then continue** to Phase 2 (UX Simplification) in `EZ_SDR_REFACTOR_TASK.md`.

---

## Build & Test Commands

### Build
```bash
# Full workspace (takes ~2min in debug)
cargo build --workspace

# Individual crates
cargo build -p ez-gui
cargo build -p ez-daemon
cargo build -p dump1090

# Release build (strongly recommended for testing - debug is slow)
cargo build --workspace --release

# Web frontend
cd ez-web
npm install
npx tsc -b            # Typecheck
npx vite build        # Output to dist/
npm test              # Run wire protocol tests
```

### Test & Lint
```bash
cargo test --workspace
cargo test --workspace --all-features
cargo clippy --workspace --all-targets
cargo fmt --check
make audit            # Security audit
```

### Run
```bash
# Desktop app
cargo run --release -p ez-gui

# Daemon (no hardware needed)
cargo run --release -p ez-daemon -- --source synthetic --web-static-dir ./ez-web/dist
# Then open http://127.0.0.1:7891/

# With real SDR
cargo run --release -p ez-daemon -- --source rtlsdr
```

---

## Critical Constraints

### MUST Follow (from CLAUDE.md)
1. **Never commit `.claude/settings.json`** - contains suspicious third-party API key
2. **Run `graphify update .`** after code changes (AST-only, no cost)
3. **Commit only when complete and verified** (build + tests pass)
4. **Use real browser for GUI testing** - headless Firefox screenshots unreliable
5. **No workflow tools** - no declarative multi-agent frameworks

### Project Conventions
- Line numbers in Plan#2.md were verified 2026-07-10 - **re-check before editing** (lines shift)
- Use `--source synthetic` for testing without SDR hardware
- Release builds recommended (debug pins 150% CPU on synthetic source)
- Match existing code style (comment density, naming)
- Error handling: use `thiserror`, avoid `unwrap()` without safety comment

---

## Known Gotchas

### Build Issues
- **SoapySDR dependency**: Requires `libsoapysdr-dev` on Ubuntu/Debian
- **Audio support**: Default `audio` feature needs ALSA dev headers
- **First build**: Slow (~2min), compiles dump1090 + all GUI deps
- **Incremental**: Fast after initial build

### Testing Issues
- **No headless UI tests**: Manual testing required for GUI
- **Firefox screenshots**: Capture at `load` event, miss async work
- **Daemon CPU**: Debug build starves event loop under load
- **Workaround**: Use `--release` or `--sample-rate 256000`

### Code Issues
- **Line numbers shift**: Verify before editing (Plan#2.md was frozen 2026-07-10)
- **Web spectrum bug**: Fixed 2026-07-10 (worker-owned OffscreenCanvas pattern)
- **Backend gaps**: `set_demod_mode` doesn't update mirror, no retune command (documented in plan#1.md)

---

## Architecture Deep Dive

### Desktop App (`ez-gui`)
- **Framework**: egui 0.35 / eframe
- **Rendering**: GPU-accelerated via wgpu
- **Structure**: Monolithic, all modules in one binary
- **Entry**: `src/main.rs` → `src/app.rs::CentralApp`
- **Key modules**:
  - `spectrum.rs` - FFT, waterfall rendering
  - `adsb_panel.rs` - Aircraft tracking UI + map
  - `satellite_panel.rs` - TLE engine, pass predictions
  - `demod.rs` - Demodulation (AM/FM/SSB/WFM)
  - `audio_output.rs` - CPAL audio playback

### Daemon (`ez-daemon`)
- **Framework**: tokio async runtime
- **Web layer**: axum REST + tungstenite WebSockets
- **Structure**: Headless, serves `ez-web/dist/`
- **Entry**: `src/main.rs` CLI
- **Key modules**:
  - `state.rs` - Central daemon state
  - `web/api.rs` - REST endpoints (`/api/*`)
  - `web/ws.rs` - Control socket + data streams
  - `web/wire.rs` - Binary framing protocol

### Web Frontend (`ez-web`)
- **Framework**: None (vanilla TS + DOM)
- **Build**: Vite bundler
- **Structure**: Class-per-panel, no virtual DOM
- **Entry**: `src/main.ts` - wires panels to API clients
- **Key modules**:
  - `api/control.ts` - Control WebSocket
  - `api/stream.ts` - Data WebSocket (spectrum/audio/adsb)
  - `api/wire.ts` - Binary decoders
  - `workers/spectrum-worker.ts` - OffscreenCanvas renderer
  - `ui/spectrum-view.ts` - Main spectrum display

### Protocol (Daemon ↔ Web)
- **Control WS** (`/ws/control`): JSON messages, server pushes `Welcome` + periodic `Hardware`
- **Data WS** (`/ws/stream/{kind}/{id}`): Binary frames with little-endian headers
- **Spectrum**: 24-byte header (center_hz, sample_rate, timestamp, bin_count) + f32 dB array
- **Audio**: 16-byte header + PCM samples
- **ADS-B**: JSON text frames (bare array, not wrapped)

---

## Testing Strategy

### Unit Tests
- **Where**: `#[cfg(test)] mod tests` in each module
- **What**: Pure functions (demod, DSP, protocol parsing)
- **Run**: `cargo test --workspace`

### Integration Tests
- **Where**: `ez-daemon/tests/`, `ez-web/tests/`
- **What**: End-to-end with synthetic source, wire protocol
- **Run**: `cargo test -p ez-daemon`, `cd ez-web && npm test`

### Manual GUI Tests
- **Why**: No headless UI framework available
- **How**: Launch app, follow checklist in Plan#2.md verification section
- **Tools**: Firefox DevTools, watch Console for errors

### What to Test After Each Task
1. **Build**: `cargo build --workspace` succeeds
2. **Clippy**: `cargo clippy --workspace --all-targets` clean
3. **Tests**: `cargo test --workspace` passes
4. **GUI**: Launch app, exercise changed feature
5. **Verify**: Compare behavior to Plan#2.md expected result

---

## Debugging Tips

### Compile Errors
- Check line numbers haven't shifted from Plan#2.md reference
- Verify field/method names match actual code
- Look for missing imports after deletions
- Watch for unused variable warnings (may need `_` prefix)

### Runtime Issues
- Check Console output for panics
- Use `RUST_LOG=debug` for verbose tracing
- Browser DevTools for web frontend issues
- Confirm `--source synthetic` for hardware-free testing

### Performance Problems
- Switch to `--release` build
- Lower sample rate (`--sample-rate 256000`)
- Profile with `cargo flamegraph` if needed
- Check memory growth in long runs (tile cache)

---

## Phase 2+ Preview (After Tasks 1-3)

### UX Simplification (Tasks 4-6)
- Create "Quick Start" wizard (replaces removed tutorial)
- Simplify panel layout (consolidate tabs)
- Add Simple/Advanced mode toggle
- Build frequency database (FM stations, weather, aircraft, satellites)

### Architecture Clarity (Tasks 7-9)
- Document when to use desktop vs daemon+web
- Clean up legacy TCP protocol (vs REST/WS)
- Remove dead code (cargo-udeps, grep audit)

### Documentation (Tasks 10-12)
- Rewrite README for beginners
- Add inline help/tooltips everywhere
- Create workflow examples (FM radio, aircraft, satellites)

### Reliability (Tasks 13-15)
- Improve error handling (no unwrap(), clear messages)
- Add more integration tests
- Performance profiling

### Polish (Tasks 16-18)
- Settings audit and reorganization
- Accessibility (keyboard nav, screen reader)
- Packaging scripts

---

## Success Criteria

You're done when:
1. All checkboxes in `EZ_SDR_REFACTOR_TASK.md` are marked
2. `cargo build --workspace` zero warnings
3. `cargo test --workspace` passes
4. `cargo clippy --workspace` clean
5. A beginner can install and listen to FM radio in <5 minutes
6. README explains value prop vs alternatives
7. All "What's Broken/Messy" items resolved
8. Knowledge graph updated (`graphify update .`)
9. Changes committed (excluding `.claude/settings.json`)

---

## Questions & Escalation

### If You Get Stuck
1. Read the relevant Plan#2.md section again
2. Check if line numbers have shifted
3. Grep for related code (`rg "pattern"`)
4. Test in isolation (create minimal repro)
5. Document the blocker in Tips section

### If Requirements Unclear
1. Check existing code for patterns
2. Look at tests for expected behavior
3. Consult README / plan#1.md for context
4. Make conservative choice, note in commit message

### If Scope Too Large
1. Complete what you can
2. Update task checkboxes
3. Document exact stopping point in Tips
4. Commit completed work
5. Update this handoff with current status

---

## Agent Handoff Checklist

When passing to next agent:
- [ ] Mark completed tasks with your identifier
- [ ] Update Tips section in `EZ_SDR_REFACTOR_TASK.md`
- [ ] Note exact stopping point and any blockers
- [ ] Run `cargo build` and document any new errors
- [ ] Commit completed phases
- [ ] Update both task tracker and this handoff file

**For Receiving Agent**:
- [ ] Read this entire document
- [ ] Read `EZ_SDR_REFACTOR_TASK.md`
- [ ] Verify build still works
- [ ] Review Tips section for latest discoveries
- [ ] Continue from first unchecked task

---

## Final Notes

This is a **well-scoped, achievable refactor**. Plan#2.md provides exact implementation details for the first 3 tasks. The remaining phases are clearly defined with specific deliverables.

**Start with Task 1.1** (zoom accumulator). The code changes are small, well-specified, and low-risk. Build incrementally, test each subtask, and commit when complete.

The end goal is worth it: a truly beginner-friendly SDR app that rivals commercial software. Let's make it happen.

Good luck! 🚀
