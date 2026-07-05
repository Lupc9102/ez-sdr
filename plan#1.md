# EZ-SDR: Map Zoom Optimization, Tutorial Removal, AI Model Migration

## Context

Three independent cleanup/polish items for the `ez-gui` crate (Rust + egui/eframe desktop app), continuing the project's recent cleanup direction (e.g. `7b1a312 fix: wire up dead EditorPanel, resolve clippy lints`):

1. The ADS-B map's scroll-to-zoom is over-sensitive (any nonzero scroll delta fires a full zoom-level step) and has no pinch-gesture support at all; tile loading has no disk cache, no eviction, and no concurrency limit, causing visible latency/blank tiles when zooming.
2. The Settings panel's "Restart Tutorial" button and the entire first-run onboarding-tutorial system it re-triggers should be fully abolished — not just the button, but every dead field/struct/enum that exists only to serve it — while leaving the unrelated per-page antenna checklists and the How-To help page completely untouched.
3. The AI Tab's default/suggested model strings still point at old Claude 3/3.5 Haiku releases; they should target the current OpenRouter Haiku release.

This plan is written as an exact, ordered execution blueprint (exact file/line anchors, exact field names, exact formulas) for a downstream low-reasoning-tier model to follow mechanically. All line numbers below were verified by direct file reads (not estimated) as of the start of this planning session — re-check with the current file state before editing, since line numbers shift as earlier edits in this same plan are applied.

**Execution order matters**: do Task 3 first (smallest, zero-risk, builds confidence), then Task 2 (mechanical but touches 4 files), then Task 1 (most architecturally involved). Run `cargo build -p ez-gui` after each task before moving to the next.

---

## Task 1: ADS-B Map Zoom Mechanics Optimization

**File**: `ez-gui/src/adsb_panel.rs`. Struct `AdsBPanel` fields today at lines 5-42. Zoom/pan/tile-render code lives in `ui_map()` (lines 853-1180). Tile I/O helpers: `lon_to_tile_x`/`lat_to_tile_y` (666-675), `request_tile` (677-705), `process_tile_downloads` (707-719).

### Root cause of "too sensitive" zoom

`ui.input(|i| i.smooth_scroll_delta.y)` returns a *continuous magnitude* — a mouse wheel notch is ~50 points, but trackpads emit many small nonzero deltas (5-15 points) per physical gesture. The current code (lines 890-909) treats **any** nonzero delta as a full ±1 zoom-level step:
```rust
let dz = if scroll > 0.0 { 1i32 } else { -1 };
let new_zoom = (self.tile_zoom as i32 + dz).clamp(2, 18) as u32;
```
So one trackpad flick can fire many full zoom-level jumps in rapid succession. There is currently **no pinch/touch handling anywhere** in the codebase (verified via grep) — only scroll and drag. egui 0.35's `ctx.input(|i| i.zoom_delta())` is the idiomatic pinch API (multiplicative per-frame factor, 1.0 = no change) and needs to be added, not just tuned.

**On double-counting**: egui's platform layer (egui-winit) converts a physical mouse-wheel-while-Ctrl-held gesture into a Zoom event (driving `zoom_delta()`) *instead of* a Scroll event — the two are mutually exclusive per physical gesture, both for ctrl+scroll trackpad-pinch emulation and genuine touch pinch. So `smooth_scroll_delta` and `zoom_delta()` can safely be read as two independent, non-overlapping input sources feeding one shared mechanism, with no manual modifier-key suppression needed. (Verify this empirically after implementing — see Verification section — since it can't be tested during planning.)

### 1.1 — Unified throttled zoom accumulator (replaces the instant-step scroll logic)

Add two new fields to the `AdsBPanel` struct (near the existing `tile_zoom`/`tile_cx`/`tile_cy` fields, ~line 40):
```rust
zoom_accum: f64,
```
(initialize to `0.0` in the constructor alongside the other `tile_*` field initializers, ~line 331 area where `tile_zoom: 8` etc. are set).

Add a module-level constant near the top of the tile-helpers section (~line 664, next to the `// --- OSM tile helpers ---` comment):
```rust
const SCROLL_POINTS_PER_ZOOM_LEVEL: f64 = 50.0;
```
This represents how many scroll "points" equal one full zoom level — approximates one traditional mouse-wheel notch, so conventional mice keep a responsive one-notch-per-level feel while trackpad jitter gets damped.

**Replace lines 890-909 entirely** with:
```rust
if response.hovered() {
    let scroll = ui.input(|i| i.smooth_scroll_delta.y);
    let zoom_delta = ui.input(|i| i.zoom_delta());
    if scroll != 0.0 {
        self.zoom_accum += scroll / SCROLL_POINTS_PER_ZOOM_LEVEL;
    }
    if zoom_delta != 1.0 {
        self.zoom_accum += zoom_delta.ln() / std::f64::consts::LN_2; // log2(zoom_delta): doubling scale == +1 zoom level
    }
    while self.zoom_accum.abs() >= 1.0 {
        let dz = if self.zoom_accum > 0.0 { 1i32 } else { -1i32 };
        let new_zoom = (self.tile_zoom as i32 + dz).clamp(2, 18) as u32;
        if new_zoom == self.tile_zoom {
            // Already at clamp boundary — stop accumulating in that direction.
            self.zoom_accum = 0.0;
            break;
        }
        let factor = 2.0_f64.powi(if dz > 0 { 1 } else { -1 });
        if let Some(mouse) = response.hover_pos() {
            let mx = f64::from(mouse.x) - f64::from(rect.center().x);
            let my = f64::from(mouse.y) - f64::from(rect.center().y);
            let tile_mx = mx / 256.0 + self.tile_cx;
            let tile_my = my / 256.0 + self.tile_cy;
            self.tile_cx = tile_mx * factor - mx / 256.0;
            self.tile_cy = tile_my * factor - my / 256.0;
        }
        self.tile_zoom = new_zoom;
        self.zoom_accum -= dz as f64;
    }
}
```
Notes for the executor:
- The `while` (not `if`) lets one large fast fling step multiple zoom levels in a single frame, while small trackpad deltas need several frames to accumulate to `1.0` before the first level change — this **is** the damping/throttling mechanism.
- The clamp-boundary `break` prevents an infinite loop / stuck accumulator when the user keeps scrolling past zoom level 2 or 18.
- Drag-to-pan (lines 911-916 in the original file, now shifted by however many lines the replacement above adds/removes) is unchanged — leave it exactly as-is.

### 1.2 — Bounded in-memory tile cache (LRU-style eviction)

Today `tile_cache: HashMap<(u32,u32,u32), egui::TextureHandle>` grows forever. Each cached 256×256 RGBA8 texture is 256×256×4 = 262,144 bytes (256 KiB). Add a cap.

New struct fields (next to `tile_cache`):
```rust
tile_last_used: std::collections::HashMap<(u32, u32, u32), u64>,
tile_frame_counter: u64,
```
New constant (near `SCROLL_POINTS_PER_ZOOM_LEVEL`):
```rust
const MAX_CACHED_TILES: usize = 400; // ~400 * 256 KiB ≈ 100 MiB
```
In `ui_map()`, immediately after the existing `self.process_tile_downloads(ui.ctx());` call (line 878), add:
```rust
self.tile_frame_counter += 1;
```
In the tile render loop (lines 934-960), inside the `if let Some(handle) = self.tile_cache.get(&key)` branch — i.e. every time a tile is actually drawn — add a line recording recency:
```rust
self.tile_last_used.insert(key, self.tile_frame_counter);
```
After the full tile render loop (after the closing `}` of the `for tx in tx_s..tx_e { for ty in ty_s..ty_e { ... } }` nested loop, before the click-handler section that currently starts at line 963), add an eviction pass:
```rust
if self.tile_cache.len() > MAX_CACHED_TILES {
    let mut by_age: Vec<((u32, u32, u32), u64)> = self.tile_last_used.iter().map(|(k, v)| (*k, *v)).collect();
    by_age.sort_by_key(|(_, frame)| *frame);
    let excess = self.tile_cache.len() - MAX_CACHED_TILES;
    for (key, _) in by_age.into_iter().take(excess) {
        self.tile_cache.remove(&key);
        self.tile_last_used.remove(&key);
    }
}
```
This only runs the O(n log n) sort when over the cap, so it's cheap in the common case. Dropping an `egui::TextureHandle` from the map releases its GPU-side texture once the handle's refcount hits zero — this is the correct/only way to free tile memory in egui.

### 1.3 — Bounded concurrent tile downloads

Today `request_tile` calls `std::thread::spawn` with no limit — rapid zoom churn can spawn dozens of concurrent OS threads/HTTP connections.

New struct field:
```rust
tile_inflight: std::sync::Arc<std::sync::atomic::AtomicUsize>,
```
Initialize in the constructor: `tile_inflight: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),`

New constant: `const MAX_CONCURRENT_TILE_DOWNLOADS: usize = 8;`

Modify `request_tile` (lines 677-705) — add the cap check right after the existing pending-check, and thread the inflight counter through the spawned closure:
```rust
fn request_tile(&mut self, z: u32, x: u32, y: u32) {
    if self.tile_pending.contains(&(z, x, y)) {
        return;
    }
    if self.tile_inflight.load(std::sync::atomic::Ordering::Relaxed) >= MAX_CONCURRENT_TILE_DOWNLOADS {
        return; // not marked pending — the per-frame render loop retries automatically next frame
    }
    self.tile_pending.insert((z, x, y));
    self.tile_inflight.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tx = self.tile_download_tx.clone();
    let inflight = std::sync::Arc::clone(&self.tile_inflight);
    std::thread::spawn(move || {
        let bytes = Self::load_or_fetch_tile(z, x, y); // see 1.4 below — replaces the inline ureq call
        let _ = tx.send(((z, x, y), bytes));
        inflight.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    });
}
```
**Important correctness note**: `AtomicUsize` has no signed decrement — use `fetch_sub(1, ...)`, never a negative `fetch_add`. `Ordering::Relaxed` is sufficient since this counter only rate-limits; it doesn't need to synchronize access to any other shared memory.

Because uncapped requests are simply never marked `tile_pending`, the existing per-frame tile-render loop (which already calls `request_tile` for every visible-but-uncached tile) will naturally retry them on a later frame once a slot frees up — no explicit retry queue needed.

### 1.4 — Disk-backed tile cache

No `dirs`/`directories` crate dependency exists in Cargo.toml, and the project's existing convention is simple relative paths in the working directory (e.g. `output_directory: "./recordings"`, `ez_sdr_config.json` saved to CWD) — follow that same convention rather than adding a new dependency.

Add a new helper method on `AdsBPanel` (near `lon_to_tile_x`/`lat_to_tile_y`, ~line 675):
```rust
fn tile_disk_path(z: u32, x: u32, y: u32) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("tile_cache/{z}/{x}/{y}.png"))
}
```
Add a new associated function that replaces the inline network-fetch logic currently inside `request_tile`'s spawned closure — extract it so both the disk-check and the concurrency-cap changes above compose cleanly:
```rust
fn load_or_fetch_tile(z: u32, x: u32, y: u32) -> Vec<u8> {
    let path = Self::tile_disk_path(z, x, y);
    if let Ok(cached) = std::fs::read(&path) {
        return cached;
    }
    let url = format!("https://tile.openstreetmap.org/{z}/{x}/{y}.png");
    let fetched = match ureq::get(&url).header("User-Agent", "ez-sdr/0.1").call() {
        Ok(resp) => {
            let mut buf = Vec::new();
            match resp.into_body().into_reader().read_to_end(&mut buf) {
                Ok(_) => buf,
                Err(_) => Vec::new(),
            }
        }
        Err(_) => Vec::new(),
    };
    if !fetched.is_empty() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, &fetched);
    }
    fetched
}
```
This is called from within the `std::thread::spawn` closure in 1.3 above, so both the disk read and disk write happen on the background thread — never on the UI/render thread. `process_tile_downloads` (lines 707-719) does not need to change at all — it already just decodes whatever bytes arrive over the channel, regardless of whether they came from disk or network.

### 1.5 — Prefetch adjacent zoom levels

After the main visible-tile render loop (i.e., after the eviction pass added in 1.2, still inside `ui_map()`), add a call to a new helper:
```rust
self.prefetch_adjacent_zoom(rect);
```
New method:
```rust
fn prefetch_adjacent_zoom(&mut self, rect: egui::Rect) {
    let zoom = self.tile_zoom;
    let tile_px = 256.0_f64;
    let half_w = f64::from(rect.width() / 2.0);
    let half_h = f64::from(rect.height() / 2.0);
    for &z2 in &[zoom.wrapping_sub(1), zoom + 1] {
        if z2 < 2 || z2 > 18 || z2 == zoom {
            continue;
        }
        let scale = 2.0_f64.powi(z2 as i32 - zoom as i32);
        let cx2 = self.tile_cx * scale;
        let cy2 = self.tile_cy * scale;
        let n = 1u64 << z2;
        let tx_s = (cx2 - half_w / tile_px).floor() as i64;
        let tx_e = (cx2 + half_w / tile_px).ceil() as i64;
        let ty_s = (cy2 - half_h / tile_px).floor() as i64;
        let ty_e = (cy2 + half_h / tile_px).ceil() as i64;
        for tx in tx_s..tx_e {
            for ty in ty_s..ty_e {
                let wt = tx.rem_euclid(n as i64) as u32;
                let wu = ty.rem_euclid(n as i64) as u32;
                if !self.tile_cache.contains_key(&(z2, wt, wu)) {
                    self.request_tile(z2, wt, wu);
                }
            }
        }
    }
}
```
Note: `zoom.wrapping_sub(1)` at `zoom == 2` wraps to a huge `u32`, which the `z2 < 2` check correctly filters out — this avoids a panic from unsigned underflow (do NOT use plain `zoom - 1`).

Because this is called *after* the current-zoom tile loop within the same frame, and because of the concurrency cap from 1.3, current-zoom tile requests always claim the 8 available download slots first — prefetch requests only actually launch once current-zoom demand is satisfied. No explicit priority system is needed; call order plus the existing cap gives this for free.

### Task 1 file summary
Only `ez-gui/src/adsb_panel.rs` is touched. No `Cargo.toml` changes needed (all new code uses `std`, `ureq`, and `image`, all already dependencies).

---

## Task 2: Remove "Restart Tutorial" and All Onboarding-Tutorial Machinery

Verified exhaustively via `grep -rn` across every `.rs` file for `tutorial`, `welcome_seen`, `focus_tab`, `enum Tab`, `highlight_target`, `skip_confirm_phase`, `level_chosen`, `asked_resume`, `resume_response`, `tab_to_open` — the list below is the complete set of every reference in the crate. Two unrelated systems must NOT be touched: `ez-gui/src/howto_panel.rs` (the How-To help page) and `ez-gui/src/antenna_checklist.rs` (the per-panel setup checklists, gated by the separate `skip_antenna_checklists` config field) — grep confirms zero shared imports/state with anything below.

**Decision call**: the user asked to "abolish all traces" of the tutorial, not just the button. This plan therefore also removes the legacy `welcome_seen` migration field (kept only to feed the now-removed `tutorial_seen`) and the `Tab`/`focus_tab` machinery (which exists only to support tutorial-driven tab navigation and is never constructed/called from anywhere else) — this is a deliberate, broader interpretation of "abolish," not scope creep from a smaller ask.

Delete/edit in this order so the crate stays as close to compiling as possible at each step (leaf files first, then their callers):

### 2.1 — Delete `ez-gui/src/tutorial.rs` entirely
The whole 97-line file (the `render_tutorial()` function and its own `#[cfg(test)] mod tests`).

### 2.2 — `ez-gui/src/main.rs`
Delete line 36: `mod tutorial;`

### 2.3 — `ez-gui/src/user_level.rs` (208 lines total — KEEP the `UserLevel` enum, it's used independently for the "Knowledge Level" setting)
- Delete the `TutorialState` struct, lines 71-108 (its `new()` and `dismiss()` methods included).
- Inside `#[cfg(test)] mod tests` (starts line 110), delete only the two tutorial-specific tests: `tutorial_state_new` (lines 189-196) and `tutorial_state_dismiss` (lines 198-207). Keep every other test in that module untouched (`user_level_from_str_valid`, `user_level_from_str_invalid_defaults_to_beginner`, `user_level_to_str_roundtrip`, `user_level_labels`, `user_level_descriptions`, `show_advanced_controls`, `simplify_layout`, `has_inline_expand`, `levels_returns_all_four` — lines 114-187).
- Note: `TutorialState` currently holds `pub tab_to_open: Option<Tab>`, which is why `use crate::app::Tab;` (line 1) exists in this file — that import must also be deleted once `TutorialState` is gone, since `Tab` itself is deleted in 2.4 below.

### 2.4 — `ez-gui/src/app.rs` (3586 lines)
In order:
- Line 31: delete `use crate::tutorial;`
- Line 32: delete `use crate::user_level::{TutorialState, UserLevel};` in full (grep confirms nothing else in `app.rs` uses `UserLevel` — its only other appearance, line 450, is inside the block deleted below).
- Lines 36-44: delete the entire `pub enum Tab { ... }` definition (grep confirms `Tab` is constructed/matched only inside `focus_tab`, deleted next).
- Line 189: delete the struct field `tutorial: TutorialState,` from `CentralApp`.
- Lines ~442-453: delete the `tutorial: { ... }` field initializer block inside `CentralApp::new()`'s struct literal:
  ```rust
  // Tutorial: first boot welcome dialog
  tutorial: {
      let state = shared.lock().expect("shared state mutex poisoned");
      let mut t = TutorialState::new();
      if state.config.tutorial_seen {
          t.active = false;
      } else {
          t.active = true;
          t.level = UserLevel::from_str(&state.config.user_level);
      }
      t
  },
  ```
  Delete the whole block including its trailing comma; this is one field among many comma-separated fields in a large struct literal, so double-check the field immediately before and after (`bm_dirty_since: None,` before, `show_starred_only: false,` after per the verified read) remain correctly comma-terminated once this block is gone.
- Lines 475-503: delete the entire `impl CentralApp { fn focus_tab(&mut self, tab: &Tab) { match tab { ... } } }` block (including its doc comment `/// Programmatically focus a tab (used by tutorial navigation).`). This `impl` block contains only this one method — safe to delete wholesale. Confirmed via grep this is the only place `Tab::` variants or `focus_tab(` are referenced anywhere.
- Lines ~1660-1678 (immediately after the `match self.current_tab { ... }` tab-dispatch block, before the "Frequency jump dialog" section): delete both blocks:
  ```rust
  // Tutorial / first-run onboarding
  if self.tutorial.active {
      let dismissed = tutorial::render_tutorial(&mut self.tutorial, &self.shared, ui);
      if dismissed && !self.tutorial.active {
          if let Ok(mut state) = self.shared.try_lock() {
              state.config.tutorial_seen = true;
              state.config.user_level = self.tutorial.level.to_str().to_string();
              state.config.tutorial_step = 0;
              state.config.save();
          }
      }
      return; // Don't render main UI during tutorial
  }

  // Handle tab navigation from tutorial
  if let Some(tab) = self.tutorial.tab_to_open.take() {
      self.focus_tab(&tab);
  }
  ```
  After deletion, execution simply falls through directly from the tab-dispatch `match` to whatever follows (the frequency jump dialog) — this is correct, there is no longer any tutorial gate to early-`return` from.
- Lines ~2050-2053 inside `fn on_exit(...)`: delete
  ```rust
  // Save tutorial state
  if self.tutorial.active {
      cfg.user_level = self.tutorial.level.to_str().to_string();
  }
  ```
  leaving the surrounding `on_exit` body's `cfg.save();` call immediately following, unchanged.

### 2.5 — `ez-gui/src/config.rs` (831 lines)
- Delete the struct field `pub welcome_seen: bool,` (line 224) and its doc comment directly above it.
- Delete the struct fields and doc comments at lines 256-260:
  ```rust
  /// Whether the interactive tutorial has been seen.
  #[serde(default)]
  pub tutorial_seen: bool,
  /// Current tutorial step index.
  #[serde(default)]
  pub tutorial_step: usize,
  ```
- In `impl Default for AppConfig`: delete `welcome_seen: false,` (line 304), `tutorial_seen: false,` (line 315), `tutorial_step: 0,` (line 316).
- In `load_or_default()` (lines ~323-337): delete the migration block
  ```rust
  // Migrate from old welcome_seen to tutorial_seen
  if cfg.welcome_seen && !cfg.tutorial_seen {
      cfg.tutorial_seen = true;
  }
  ```
  and trim the doc comment above the function from "Load configuration from `ez_sdr_config.json`, or return defaults if the file does not exist or cannot be parsed. Also migrates the legacy `welcome_seen` flag to the `tutorial_seen` field." down to just the load/default sentence (drop the second sentence about migration).
- In `AppConfig::ui()`, the "User Experience" collapsing section (~lines 553-578): delete only the trailing `ui.add_space(4.0);` plus the `if ui.button("🔁 Restart Tutorial")...` block:
  ```rust
  ui.add_space(4.0);
  if ui.button("🔁 Restart Tutorial").on_hover_text("Re-open the first-run tutorial on next launch.").clicked() {
      self.tutorial_seen = false;
      self.tutorial_step = 0;
      self.needs_apply = true;
  }
  ```
  Keep the Knowledge Level label/slider/description code immediately above it (lines ~554-570) completely untouched — that setting is unrelated to the tutorial and stays.

### Task 2 file summary
`tutorial.rs` (deleted), `main.rs`, `user_level.rs`, `app.rs`, `config.rs`. Do **not** touch `howto_panel.rs` or `antenna_checklist.rs`.

---

## Task 3: AI Tab — Migrate Default/Suggested Model to Latest OpenRouter Haiku

**Clarification up front**: "latest OpenRouter Haiku endpoint" does not mean a URL changes — OpenRouter's endpoint (`https://openrouter.ai/api/v1/chat/completions`) is model-agnostic; the model is selected via the JSON body's `"model"` field. This is a pure string-constant migration, not a payload/dispatch refactor.

**Verified via web search**: OpenRouter's current slug for the latest Claude Haiku is `anthropic/claude-haiku-4.5` (released 2025-10-15, 200K context, extended-thinking support). The native Anthropic API model ID is `claude-haiku-4-5-20251001`. OpenRouter also exposes a rolling alias `anthropic/claude-haiku-latest` that always tracks the newest Haiku — worth adding as an extra quick-pick, but the pinned `anthropic/claude-haiku-4.5` should remain the default (pinned > rolling for reproducibility).

**Why this is safe / zero risk**: in `ez-gui/src/ai_panel.rs`, provider dispatch is keyed purely on the provider preset's display-name string (`let is_anthropic = provider == "Anthropic";`, line 412, branching to `stream_anthropic` vs `stream_openai_compat` at lines 438-442). Both functions pass the model ID through as an opaque `"model": model` value in the JSON body — no logic anywhere branches on the specific model string. So this task requires zero changes to `ai_panel.rs`, `stream_anthropic`, `stream_openai_compat`, or `StreamParams`.

Exhaustive grep for `claude|haiku|anthropic/` (case-insensitive) across every `.rs` file found exactly 6 hardcoded reference sites, all in 2 files (`ai_panel.rs`/`editor_panel.rs` correctly read the model from config at runtime with no hardcoded strings):

### 3.1 — `ez-gui/src/config.rs`
1. Line 55: `pub const DEFAULT_AI_MODEL: &str = "anthropic/claude-3-haiku";` → `"anthropic/claude-haiku-4.5"`
2. Line 75: OpenRouter preset `default_model: "anthropic/claude-3-5-haiku",` → `"anthropic/claude-haiku-4.5"`
3. Line 82: Anthropic (native) preset `default_model: "claude-3-5-haiku-20241022",` → `"claude-haiku-4-5-20251001"` (secondary consistency change — keeps the native-API preset current alongside the primary OpenRouter ask)
4. Line 444: OpenRouter "Popular models" quick-pick array — change the first entry from `"anthropic/claude-3-5-haiku"` to `"anthropic/claude-haiku-4.5"`; leave the other three entries (`google/gemini-flash-1.5`, `meta-llama/llama-3.1-8b-instruct:free`, `mistralai/mistral-7b-instruct:free`) untouched.
5. Line 464: Anthropic "Popular models" quick-pick array — change the first entry from `"claude-3-5-haiku-20241022"` to `"claude-haiku-4-5-20251001"`; leave the sonnet/opus entries untouched (out of scope).

### 3.2 — `ez-gui/src/howto_panel.rs`
Provider-comparison table, two note strings (~lines 2661-2669):
6. OpenRouter row: `"Access to many models with one key; claude-3-haiku is cheap"` → `"Access to many models with one key; claude-haiku-4.5 is cheap"`
7. Anthropic row: `"Claude models; claude-3-5-haiku is best value"` → `"Claude models; claude-haiku-4.5 is best value"`

### On existing users' saved configs
`ai_model`/`ai_endpoint` are user-editable, persisted fields in `ez_sdr_config.json` with `#[serde(default)]`. Changing `DEFAULT_AI_MODEL` and the preset table only affects **new** configs or a user explicitly re-picking the OpenRouter/Anthropic preset (which overwrites `ai_model` with the preset's `default_model`) or clicking a quick-pick button — it does not silently change any existing user's already-saved model choice. This is expected/acceptable behavior; no migration logic is warranted (adding one would risk overriding a deliberate user override of the model field, which would be worse).

### Task 3 file summary
`ez-gui/src/config.rs`, `ez-gui/src/howto_panel.rs`. No changes to `ai_panel.rs` or `editor_panel.rs`.

---

## Verification

After each task, run from the repo root:
```
cargo build -p ez-gui
cargo clippy -p ez-gui --all-targets
cargo test -p ez-gui
```
Then, per this project's CLAUDE.md convention, run `graphify update .` to refresh the knowledge graph, and commit once build/tests/clippy are clean (don't leave finished work uncommitted).

Manual/GUI checks (launch the app, e.g. via the project's existing run skill):
- **Task 1**: Open the ADS-B tab. Scroll the mouse wheel over the map — one notch should move roughly one zoom level, not jump multiple levels. If a trackpad is available, test a small two-finger scroll (should barely move the zoom) versus a deliberate fast scroll (should zoom several levels smoothly). Test Ctrl+scroll and/or an actual pinch gesture if available — confirm it zooms without also double-triggering a plain-scroll jump (this is the one behavior that couldn't be verified during planning — watch specifically for double-speed zoom when pinching). Zoom in/out repeatedly across several levels and confirm no growing lag or memory growth over a few minutes (informal check — no strict profiling required). Restart the app and confirm tiles for a previously-viewed area load instantly from `tile_cache/` on disk without a network fetch delay.
- **Task 2**: Open Settings → User Experience — confirm the "🔁 Restart Tutorial" button is gone and the Knowledge Level slider still works normally. Confirm no tutorial dialog ever appears on launch (since the whole system is gone, not just hidden). Open the How-To page and any panel's setup checklist (e.g. ADS-B or Satellite tab on first open) — confirm both still work exactly as before.
- **Task 3**: Open the AI tab / Settings → AI Agent. With provider set to "OpenRouter", confirm the model field defaults to (or the quick-pick button shows) `anthropic/claude-haiku-4.5`. Switch provider to "Anthropic" and confirm its quick-pick shows `claude-haiku-4-5-20251001`. If an API key is available, send a test message and confirm a response streams back successfully.
