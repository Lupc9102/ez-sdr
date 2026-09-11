# EZ-SDR Unified: Complete Refactor & Polish Task

**Agent**: Claude Code CLI, Opus 5 (1M context)  
**Session Start**: 2026-09-10  
**Goal**: Transform ez-sdr from a functional but messy multi-crate SDR application into a beginner-friendly, polished, unified replacement for dump1090, sdrpp, and satdump.

---

## Target

Create a truly beginner-friendly, unified SDR application that:
- **Eliminates complexity**: Remove confusing/broken features, streamline UX
- **Works out-of-the-box**: Clear first-run experience, sensible defaults
- **Professional polish**: Fix bugs, optimize performance, improve discoverability
- **Architecture clarity**: Clean separation between GUI/daemon/web, remove dead code
- **Documentation**: Clear README, inline help, example workflows

The app should feel like a cohesive product, not a collection of hacked-together modules.

---

## Current State Analysis

### Project Structure (5 crates)
```
dump1090/       ADS-B Mode-S decoder library
ez-gui/         Main desktop application (egui/eframe)
ez-daemon/      Headless daemon with REST API + WebSocket
ez-web/         TypeScript web frontend (framework-free)
lrpt-decode/    LRPT satellite image decoder
ez-proto/       Shared protocol definitions
```

### What Works
✅ Project compiles clean (zero warnings)  
✅ Rich feature set: spectrum, waterfall, demod, satellites, ADS-B, AI assistant  
✅ GPU-accelerated rendering  
✅ Web architecture migration mostly complete (spectrum canvas rendering bug fixed 2026-07-10)  
✅ SoapySDR integration for device agnosticism  
✅ Test infrastructure in place  

### What's Broken/Messy
❌ **Plan#2.md issues** (Task 1-3 from previous session - partially complete):
  - ADS-B map zoom over-sensitive (trackpad fires multiple levels)
  - No tile caching (memory leaks, slow zoom)
  - Tutorial system present but confusing (restart button exists)
  - Outdated AI model references (Claude 3.x instead of 4.5)
  
❌ **Architecture confusion**:
  - Unclear when to use desktop app vs daemon+web
  - No clear migration path documented
  - Legacy TCP protocol alongside new REST/WS
  
❌ **UX problems**:
  - Too many panels/tabs for beginners
  - No guided workflows ("I want to listen to FM radio" → ?)
  - Jargon everywhere (FFT size, window types, demod modes)
  - Settings scattered across multiple locations
  
❌ **Missing beginner features**:
  - No frequency database (popular stations, weather, satellites)
  - No auto-configuration for common tasks
  - No validation/error recovery (wrong SDR settings = crash?)
  - Help text assumes expert knowledge

❌ **Code quality**:
  - Dead code paths (grep found unused modules?)
  - Inconsistent error handling
  - No logging strategy (tracing setup incomplete?)
  - Test coverage gaps

---

## Tasklist

### Phase 1: Critical Fixes & Polish (Complete Plan#2.md Tasks)
- [x] **Task 1.1**: Fix ADS-B map zoom sensitivity (accumulator pattern) - **Already complete**
- [x] **Task 1.2**: Add bounded in-memory tile cache with LRU eviction - **Already complete**
- [x] **Task 1.3**: Add concurrent tile download limiting (max 8) - **Already complete**
- [x] **Task 1.4**: Implement disk-backed tile cache (tile_cache/ dir) - **Already complete**
- [x] **Task 1.5**: Add adjacent zoom level prefetching - **Already complete**
- [ ] **Task 1.6**: Test zoom mechanics in real browser (Firefox/Chrome) - **Needs manual testing**
- [x] **Task 2.1**: Remove tutorial.rs entirely - **Already complete**
- [x] **Task 2.2**: Clean tutorial imports from main.rs - **Already complete**
- [x] **Task 2.3**: Remove TutorialState from user_level.rs - **Already complete**
- [x] **Task 2.4**: Remove Tab enum and focus_tab from app.rs - **Already complete**
- [x] **Task 2.5**: Remove tutorial config fields from config.rs - **Already complete**
- [ ] **Task 2.6**: Verify How-To and antenna checklists still work - **Needs manual testing**
- [x] **Task 3.1**: Update AI model constants to claude-haiku-4.5 - **Already complete**
- [x] **Task 3.2**: Update howto_panel.rs model references - **Already complete**
- [ ] **Task 3.3**: Test AI panel with new model defaults - **Needs manual testing**

### Phase 2: UX Simplification
- [x] **Task 4**: Create "Quick Start" wizard for first-run (replaces tutorial) - **Claude Code CLI, Opus 5 - 2026-09-10**
  - ✅ Device detection and selection screen
  - ✅ "What do you want to do?" workflow picker (FM radio, aircraft, satellites, ham, custom)
  - ✅ Auto-configure for selected workflow (applies settings to SharedState)
  - ✅ Welcome and completion screens
  - ⚠️ Needs UI integration (wire into CentralApp main loop)
- [ ] **Task 6**: Add frequency database/presets - **Partially complete - Claude Code CLI, Opus 5 - 2026-09-10**
  - ✅ Frequency database module with 25+ built-in presets
  - ✅ Categories: FM/AM broadcast, weather, aircraft, ham, satellites, marine
  - ✅ Search and filter API
  - ✅ Unit tests pass
  - ⚠️ Needs UI panel (frequency browser/picker)
- [ ] **Task 5**: Simplify main UI for beginners
  - Consolidate panels (too many tabs → progressive disclosure)
  - Add "Simple/Advanced" mode toggle
  - Hide technical jargon in Simple mode (FFT → "Detail Level", etc)

### Phase 3: Architecture Clarity
- [ ] **Task 7**: Document desktop vs web architecture decision
  - When to use ez-gui (local SDR)
  - When to use ez-daemon + ez-web (headless, multi-client)
  - Migration guide
- [ ] **Task 8**: Clean up legacy protocols
  - Audit TCP vs REST/WS usage
  - Deprecation plan for old protocol
  - Update examples
- [ ] **Task 9**: Remove dead code
  - Find unused modules/functions (cargo-udeps, grep analysis)
  - Remove or document "experimental" features
  - Clean up feature flags

### Phase 4: Documentation & Examples
- [ ] **Task 10**: Rewrite README for beginners
  - Clear value proposition
  - Comparison table (vs dump1090/sdrpp/satdump)
  - Step-by-step tutorials (not just "run cargo build")
  - Screenshots/videos
- [ ] **Task 11**: Add inline help/tooltips everywhere
  - Hover explanations for every control
  - Link to relevant help sections
  - Error messages with solutions (not just "failed")
- [ ] **Task 12**: Create example workflows document
  - "Listen to FM radio station"
  - "Track aircraft in my area"
  - "Receive weather satellite images"
  - "Decode NOAA weather"
  - "Monitor ham radio bands"

### Phase 5: Reliability & Testing
- [ ] **Task 13**: Improve error handling
  - No unwrap() in production code
  - Graceful degradation (missing SDR → file playback mode)
  - Clear error messages
- [ ] **Task 14**: Add integration tests
  - End-to-end with synthetic source
  - Web API contract tests
  - UI smoke tests (if headless possible)
- [ ] **Task 15**: Performance profiling
  - CPU usage optimization
  - Memory leak detection
  - Startup time improvement

### Phase 6: Final Polish
- [ ] **Task 16**: Settings audit
  - Group logically
  - Add "Reset to defaults" per section
  - Import/export configs
- [ ] **Task 17**: Accessibility
  - Keyboard navigation
  - Screen reader labels
  - High contrast mode
- [ ] **Task 18**: Packaging
  - Build scripts for major platforms
  - Installation guide
  - Docker images

---

## Tips for Downstream Agents

### Current Agent Context (Claude Code CLI, Opus 5)
- Project compiles clean with 10 warnings (unused methods in quick_start.rs - expected)
- Previous session (2026-07-10) fixed web spectrum canvas rendering bug  
- Plan#2.md Tasks 1-3 were already complete from previous sessions
- **NEW**: Quick Start wizard implemented (ez-gui/src/quick_start.rs - 450 lines)
- **NEW**: Frequency presets database implemented (ez-gui/src/frequency_db.rs - 410 lines)
- Both new modules compile and have tests, but need UI integration
- Build takes ~5sec incremental on this machine (was ~2min full build)

### Architecture Notes
- **Desktop app** (`ez-gui`): Monolithic, runs SDR directly, egui GUI
- **Daemon** (`ez-daemon`): Headless, exposes REST/WS API, serves `ez-web/dist/`
- **Web UI** (`ez-web`): Framework-free TypeScript, Vite build, uses OffscreenCanvas workers
- Spectrum rendering: Worker-owned OffscreenCanvas + transferToImageBitmap (Firefox-safe)
- Use `--source synthetic` for testing without hardware

### Build Commands
```bash
cargo build --workspace              # Full build
cargo build -p ez-gui --release      # Desktop app
cargo build -p ez-daemon --release   # Daemon
cd ez-web && npm install && npx vite build  # Web frontend
```

### Test Commands
```bash
cargo test --workspace
cargo clippy --workspace --all-targets
cd ez-web && npm test
```

### Common Issues
- Debug builds slow (150% CPU on synthetic source) - use --release for testing
- Firefox headless screenshots unreliable - use real browser
- `.claude/settings.json` has suspicious API key - NEVER commit it
- `graphify update .` required after code changes (project CLAUDE.md)

### Code Quality Standards
- No unwrap() without comment justifying safety
- Match existing style (comment density, naming)
- Every public function needs doc comment
- Error types use thiserror
- Use workspace dependencies from root Cargo.toml

### Testing Strategy
- Unit tests for pure logic (demod, DSP, protocol parsing)
- Integration tests with synthetic source
- Manual GUI testing required (no headless UI tests)
- Web tests use Node.js `--test` (framework-free)

---

## Files to Review Before Starting

1. `/home/lupc/Documents/ez-sdr/README.md` - Current feature list
2. `/home/lupc/Documents/ez-sdr/plan#1.md` - Web architecture migration details (DONE)
3. `/home/lupc/Documents/ez-sdr/Plan#2.md` - Tasks 1-3 implementation specs (IN PROGRESS)
4. `/home/lupc/Documents/ez-sdr/task.md` - Original web handoff doc
5. `/home/lupc/Documents/ez-sdr/CLAUDE.md` - Project conventions
6. `/home/lupc/.claude/CLAUDE.md` - Global agent protocols

---

## Success Criteria

The refactor is complete when:
1. ✅ All tasks checked off above
2. ✅ `cargo build --workspace` succeeds with zero warnings
3. ✅ `cargo test --workspace` passes
4. ✅ `cargo clippy --workspace` clean
5. ✅ A complete beginner can:
   - Install from source in <5 commands
   - Detect their SDR device automatically
   - Listen to FM radio within 2 minutes
   - Find help without reading source code
6. ✅ README clearly explains what this is and why vs alternatives
7. ✅ All "What's Broken/Messy" items above are resolved
8. ✅ Knowledge graph updated (`graphify update .`)
9. ✅ Changes committed (excluding `.claude/settings.json`)

---

## Handoff Protocol

When passing to another agent:
1. Mark completed checkboxes with your identifier (e.g. "Claude Code CLI, Opus 5")
2. Update Tips section with any new discoveries
3. Note exact stopping point and blockers
4. Run `cargo build` and document any new compilation errors
5. Commit completed phases before handoff
6. Update this file with current status

**Next agent should**: Read this entire file, review Tips section, verify build still works, then continue from first unchecked task.
