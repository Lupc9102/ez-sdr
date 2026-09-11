# Splashscreen + Adaptive User Level System — Review Document

## Overview

Added a **skill-level-adaptive tutorial system** and **adaptive UI** across the entire application. Users choose a level on first boot (Beginner/Intermediate/Advanced/ClerkMaxwell), get a guided tutorial overlay, and panels dynamically hide/show advanced controls.

---

## New Files

### `ez-gui/src/user_level.rs` — User Level Model

Core types for the adaptive system.

**Enums:**
- `UserLevel` — `Beginner`, `Intermediate`, `Advanced`, `ClerkMaxwell`
- `HintLevel` — `Verbose`, `Normal`, `Minimal`, `Off`
- `TutorialState` — struct tracking overlay/render state, resume/skip confirm

**Key methods on `UserLevel`:**
- `show_advanced_controls(&self) -> bool` — `true` for Advanced+ only
- `has_inline_expand(&self) -> bool` — `true` for all except ClerkMaxwell (shows `⚙` inline expand buttons)
- `simplify_layout(&self) -> bool` — `true` for Beginner only (shows "Ask AI" buttons instead of complex controls)
- `hint_verbosity(&self) -> HintLevel` — maps level to hint detail: `Beginner→Verbose`, `Intermediate→Normal`, `Advanced→Minimal`, `ClerkMaxwell→Off`
- `show_experimental_features(&self) -> bool` — `true` for ClerkMaxwell only
- `from_str(s: &str) -> Self` / `to_str(&self) -> &'static str` — serde conversion (lowercase strings: `"beginner"`, `"intermediate"`, etc.)

### `ez-gui/src/tutorial.rs` — Tutorial Engine

Full tutorial wizard with content for all 4 levels.

**Structs:**
- `TutorialStep { title, body, highlight, tab, action }` — a single step with optional glow highlight target, optional tab to switch to, optional action callback
- `TutorialAction` — `TuneFreq(u64)`, `SetDemod(&str)`, `StartAudio`, `OpenSettings`

**Step content by level:**
- **Beginner** (11 steps): Welcome, No Hardware?, SDR Panel, Listening (FM Radio action), Spectrum, AI Agent, ADS-B, Satellite, Scanner, Bookmarks & Recorder, You're Ready
- **Intermediate** (9 steps): Welcome, SDR Panel, Spectrum, AI Agent, ADS-B + Satellites, Scanner & Recorder, Bookmarks & Scheduler, Settings & Integrations, You're All Set
- **Advanced** (8 steps): Welcome, SDR Panel (Complete), Spectrum (Full), AI Agent Integration, ADS-B + Satellites, Scanner & Recorder, Full Configuration (w/ OpenSettings action), Ready
- **Clerk Maxwell** (8 steps): Welcome, SDR Panel (Everything), DSP & Signal Chain, Developer Tools, ADS-B + Satellites (Full), Scanner & Recorder (Full), Configuration & Integrations, Hidden Features Overview

**Key function:**
- `render_tutorial(state, shared, ui) -> bool` — renders a single tutorial step window with navigation, double-cascade skip confirmation, resume prompt
  - Parameters: `&mut TutorialState`, `&Arc<Mutex<SharedState>>`, `&mut egui::Ui`
  - Returns `true` when dismissed (tutorial completed)
  - Writes `config.tutorial_seen = true` and saves config on final step or confirmed skip via `SharedState`

---

## Modified Files

### `ez-gui/src/config.rs` — AppConfig + Settings UI

**New fields on `AppConfig`:**
- `user_level: String` — serialized as JSON string, defaults to `"beginner"` (lowercase)
- `tutorial_seen: bool` — whether tutorial has been completed (replaces legacy `welcome_seen`)
- `tutorial_step: usize` — last completed step index (resume across restarts)

**Migration:**
- `load_or_default()` copies `welcome_seen` → `tutorial_seen` for backwards compat

**Settings UI additions:**
- New collapsible section labeled "User Experience" with:
  - `Slider` 0..=3 with `step_by(1.0)`, labels "Beginner / Intermediate / Advanced / Clerk_Maxwell"
  - Live update: sets `config.user_level` directly (no confirmation dialog)
  - "Restart Tutorial" button that resets `tutorial_seen` + `tutorial_step`

### `ez-gui/src/app.rs` — Central Application + TabViewer

**New fields on `CentralApp`:**
- `tutorial: TutorialState` — replaces `show_welcome: bool`

**New method:**
- `focus_tab(&self, target: &Tab, surface: &mut egui_dock::DockSurface)`
  - Uses `dock_state.find_tab(target)` to get `TabPath`, calls `set_active_tab`
  - Requires `Tab: PartialEq + Clone` (already satisfied)

**Tutorial flow in `ui()`:**
1. After skin setup and DockArea rendering: checks `tutorial.active`
2. If active, calls `render_tutorial(...)` — tutorial renders as `egui::Window` overlay that floats above the already-rendered dock
3. Returns early (skips status bar, keyboard help, toasts, etc.) — DockArea panels render beneath the overlay
4. Sets `tutorial.active = false` once dismissed; saves `config.tutorial_seen = true`
5. After tutorial, handles `tab_to_open` navigation (e.g., switches to the tab highlighted in a step)

**`TabViewer` changes:**
- `highlight_target: Option<String>` — field checked by panels
- 6 tabs wrapped with glow border function:
  - `glow(ui, target)` — `Frame::new().fill(TRANSPARENT).stroke(3px, CYAN)` around tab content
  - Tabs with glow: `Tab::Sdr`, `Tab::Spectrum`, `Tab::Satellite`, `Tab::AdsB`, `Tab::Scanner`, `Tab::AiAgent`
- Scheduler tab: beginner AI prompt button (if `user_level.simplify_layout()`)

**`on_exit()`:**
- Saves `config.tutorial_step` for resume support (if `tutorial.active`)
- Saves `config.user_level` from tutorial only if `tutorial.level_chosen` (inside the `tutorial.active` guard)
- `tutorial_seen` is already persisted in `state.config` from the dismiss handler (line 1366) and preserved through the clone in `on_exit`

### `ez-gui/src/sdr_panel.rs` — SDR Controls Panel

**New fields:**
- `expand_vfo_b: bool` — inline expand toggle for VFO B section
- `expand_lo_offset: bool` — inline expand toggle for LO offset section
- `expand_ppm: bool` — inline expand toggle for PPM section
- `expand_memory: bool` — inline expand toggle for frequency memory section
- `expand_airport: bool` — inline expand toggle for airport finder section
- `expand_bias_tee: bool` — inline expand toggle for bias tee/direct sampling section
- `expand_step: bool` — inline expand toggle for step size section

**At top of `ui()`:**
- Acquires user level: `self.shared.try_lock().map(|s| UserLevel::from_str(&s.config.user_level))`
- Reads `show_advanced` and `has_expand` from user level

**Level-adaptive sections:**
- **VFO B section** — hidden for Beginner (`show_advanced`=false) unless `has_expand && expand_vfo_b`
- **LO offset section** — hidden unless Advanced+ (`show_advanced`) unless `has_expand && expand_lo_offset`
- **PPM section** — refactored out of the `if let Ok(mut state)` gain block into its own `try_lock()`, hidden for Beginner/Intermediate unless expanded
- **Memory section** — similarly gated via `has_expand && expand_memory`
- **Airport finder** — gated via `has_expand && expand_airport`
- **Bias tee / Direct sampling** — gated via `has_expand && expand_bias_tee`
- **Step size** — gated via `has_expand && expand_step`

Gain optimization suggestion box is always shown (not gated by level).

### `ez-gui/src/satellite_panel.rs` — Satellite Tracking

**Adaptive addition:**
- After heading, checks `user_level.simplify_layout()` (Beginner)
- Shows prominent `🤖 Ask AI to track a satellite` button (220×28px, blue background)
- Sets `self.pending_ai_prompt` on click (field already existed)

### `ez-gui/src/adsb_panel.rs` — ADS-B Aircraft Tracking

**Adaptive addition:**
- After heading, checks `user_level.simplify_layout()` (Beginner)
- Shows prominent `🤖 Ask AI to start ADS-B tracking` button (240×28px)
- Sets `self.pending_ai_prompt` on click (field already existed)

### `ez-gui/src/scanner.rs` — Frequency Scanner

**Adaptive addition:**
- After heading, checks `user_level.simplify_layout()` (Beginner)
- Shows prominent `🤖 Ask AI to configure scanning` button (240×28px)
- Sets `self.pending_ai_prompt` on click (field already existed)

### `ez-gui/src/main.rs` — Entry Point

**Added module declarations:**
- `mod user_level;`
- `mod tutorial;`

---

## Architecture Notes

### How the level system flows:
1. `AppConfig` stores `user_level: String` (persisted in JSON)
2. `CentralApp` or `TabViewer` reads it via `self.shared.try_lock()`
3. Passes through `UserLevel::from_str()` → gets helper booleans
4. Panels check `show_advanced()`, `has_inline_expand()`, or `simplify_layout()`

### Tutorial rendering:
- `render_tutorial()` is called from `CentralApp::ui()` **after** the egui_dock layout is rendered (floats as overlay)
- If tutorial is active, the function returns early (skipping status bar, keyboard help, toasts)
- Tutorial renders as `egui::Window` (not a blocking CentralPanel) — allows clicking behind
- Step navigation: Prev/Next/Finish buttons at the bottom of the tutorial window
- Skip: two cascading confirmation popups before dismissal

### Glow highlighting:
- `TabViewer` has `highlight_target: Option<String>`
- `glow(ui, target)` helper wraps content in a `Frame::new().fill(TRANSPARENT).stroke(3px, CYAN)`
- Tutorial steps set `self.highlight_target` via `CentralApp`'s reference
- Panels check their key against `highlight_target` before drawing glow

### Inline expand pattern (used in SDR panel):
- If `has_inline_expand()` is true (all except ClerkMaxwell), hidden sections show a `⚙` button
- Toggle stores state in panel fields (e.g., `expand_vfo_b`)
- If `show_advanced()` is true (Advanced+), sections are always shown without toggling

---

## Key Constraints (from implementation)

- egui 0.35: `Frame::new()` not `Frame::none()`, `ui.columns(n)` not `allocate_ui_at_rect`
- egui_dock 0.20.1: `DockState::find_tab()` / `set_active_tab()` for programmatic tab switching
- Config: `user_level` stored as `String` for serde compatibility (not as enum)
- `Tab` enum already has `PartialEq + Clone` derived (required for `find_tab`)
- Compilation: 0 errors (only pre-existing warnings)
