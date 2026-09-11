# EZ-SDR — Full UI Redesign: "3 Modes + a Deck"

## Context

`ez-gui` (Rust + egui/eframe desktop SDR app) has grown **13 top-level navigation targets**:

- **6 main tabs** (48px left rail): 📻 SDR · ✈ ADS-B · 🛰 Satellite · 🤖 AI · 🛸 Decoding · 🎨 Customize
- **7 "secondary tools"** (slide-out panel): ⭐ Bookmarks · 🔍 Scanner · ⏺ Recorder · 🗓 Scheduler · 💬 Discord · ❓ HowTo · ⚙ Settings

A newcomer is confronted with all 13 at once — the "million useless side tabs" problem. The goal is a **full redesign into the simplest, most intuitive SDR app**: collapse navigation to a handful of task-oriented modes, make the common path (tune → listen) require zero radio knowledge, and hide (never delete) power features behind one drawer that grows with the user's skill level.

**Direction locked with user:**
- Navigation shape = **Task modes** (Listen / Planes / Satellites).
- Only **Tune + Spectrum** is always-visible core. Bookmarks, AI, and the mode content are on-demand.
- Rarely-used tools = **hide, don't delete** — one "More" drawer, progressively revealed by the existing user-level system.

This plan reuses existing machinery wherever possible (the slide-over panel system, `UserLevel` gating already threaded through 8 files, the band-ID table, the freq-jump parser, per-panel entry points). It is **additive-then-subtractive**: build the new shell, move panels into it, then delete the old rail.

---

## Design principles (the "how" it gets simpler)

1. **One mental model, three verbs.** A normal person uses an SDR to *Listen*, watch *Planes*, or catch *Satellites*. Top-level nav = exactly those 3 buttons. Everything else is a tool used *inside* a mode, not a peer destination.
2. **Progressive disclosure.** Default = Beginner: 3 modes + a nearly-empty "More". Flipping User Level to Advanced/Expert unlocks Scanner/Recorder/Scheduler, then Discord/MQTT/Remote/Layout. Nothing is deleted; complexity is earned. (Leverages existing `UserLevel::show_advanced_controls()` / `simplify_layout()` / `has_inline_expand()`.)
3. **No frequency knowledge required.** Preset tiles ("📻 FM Radio", "✈ Air Band", "🌦 Weather") tune + set mode + start in one tap. An **Auto** demod mode picks AM/FM/WFM/SSB from the frequency so beginners never learn modulation.
4. **Plain language over jargon.** Signal shown as "Strong ✓ / Weak / Quiet" (dB kept small for experts). One tune box accepts "145.5", "FM", or "weather".
5. **The assistant is ambient, not a place.** AI becomes a "🤖 Ask" slide-over available in every mode, not a tab you navigate to.
6. **Reuse, don't rewrite.** Panels keep their internals; only their host (top-level tab vs. drawer/subtab) changes.

---

## Target information architecture

```
┌─────────────────────────────────────────────────────────────┐
│  🎧 Listen    ✈ Planes    🛰 Satellites          🤖 Ask   ⚙ More │ ← mode bar (top, ~40px)
├─────────────────────────────────────────────────────────────┤
│                                                               │
│                     [ active mode content ]                   │
│                                                               │
├─────────────────────────────────────────────────────────────┤
│ ● 145.500 MHz  Auto→FM   ▮▮▮ Strong ✓    ● REC     <flash>    │ ← status strip (kept)
└─────────────────────────────────────────────────────────────┘
```

- **Primary nav**: 3 mode buttons (icon + word). Replaces the 13-icon rail.
- **Right of the bar**: `🤖 Ask` (AI slide-over) + `⚙ More` (the deck of hidden tools).
- **Status strip** at the bottom is kept (already implemented) and humanized.

### 🎧 Listen (home / default) — where 90% of use lives
Reuse `render_sdr_tab` → `render_listen`. Center = spectrum/waterfall (`SpectrumAnalyzer::ui`). Additions, top to bottom:
- **Preset tiles** — one-tap strip: `📻 FM Radio · ✈ Air Band · 🌦 Weather Radio · 🚓 Public Safety · 📡 Ham 2m · 🚢 Marine · ⭐ Saved`. Each tile tunes a representative frequency, sets the demod mode, sets bandwidth, and starts the source. Built from the existing band table (`sdr_panel::identify_frequency`, `sdr_panel.rs:2436`) + the freq-jump `bands` list (`app.rs:1624`). `⭐ Saved` opens the Bookmarks drawer.
- **Frequency hero + smart tune box** — large readout, `±` buttons, and one input: *"Type a frequency or name (145.5, FM, weather)"*. Reuse the freq-jump parser inline (`app.rs:1614-1666`: numeric → band-name → bookmark search) instead of the J-key modal.
- **Auto mode chip** — shows `Auto → WFM`; click to pin a specific mode. Backed by new `DemodMode::Auto` + `DemodMode::for_frequency(hz)`.
- **Plain-language signal meter** — colored bar + word from SNR buckets (SNR already computed in `logic()`); small dB number retained.
- **"⚙ Adjust ▾" expander** — folds Gain, Squelch, PPM, LO offset, Sample rate, Bias-T, VFO B, LPF. `SdrPanel::ui_source` already gates these via `show_advanced` / `has_inline_expand` / `is_beginner` (`sdr_panel.rs:194-196, 524`; fields `expand_ppm/lo/vfo` at `124-126`) — the redesign makes the split explicit and default-collapsed.
- **⭐ Bookmarks drawer** — right slide-over reusing `render_bookmarks_full` (`app.rs:2481`).

### ✈ Planes
Reuse `render_adsb_tab` (`app.rs:2290`) as-is: map center, aircraft list right, collapsible antenna guide. **Simplicity add:** entering Planes auto-configures the SDR. Extract the Start-button body (`adsb_panel.rs:1338-1347`: set 1090 MHz + 2.048 MSps, `source.start()`, `adsb_running=true`) into `AdsBPanel::begin()` and call it on mode entry, so aircraft simply appear. A single "⏸ Stop" remains.

### 🛰 Satellites (absorbs Decoding)
Reuse `render_satellite_tab` (`app.rs:2345`). Add a third subtab to `SatelliteSubTab` (`satellite_panel.rs:15`): **`Track · Align · Decode`**. The existing decode handoff (`app.rs:603-606`) changes from `current_tab = AppTab::Decoding` to `satellite_subtab = Decode`; `DecodingPanel::ui` renders inside the mode. `tick_decode()` already runs unconditionally every frame (`app.rs:613`), so no lifecycle change. The standalone 🛸 Decoding top-level tab is removed.

### ⚙ More — the deck (hide-don't-delete, progressive)
One slide-over reusing the existing `active_secondary_tool` + `egui::Panel::left("secondary_panel")` + `render_secondary_panel` machinery (`app.rs:1582, 2095`). Contents grouped and revealed by `UserLevel`:
- **Always:** Settings (`AppConfig::ui`, `config.rs:339` — incl. observer location, theme via `CustomizePanel::ui` `customize_panel.rs:53`, **User Level selector** `config.rs:544`, font scale), Help (`HowToPanel::ui`, `howto_panel.rs:134`), Bookmarks manager.
- **Advanced+** (`show_advanced_controls`): Scanner, Recorder, Scheduler.
- **Expert / ClerkMaxwell:** Discord, MQTT, Web Remote, Layout customizer.

A Beginner opening "More" sees ~3 friendly entries; an Expert sees all ~10. Satisfies both "single drawer" and "gate by level."

###  Ask (AI) — ambient 
AI stops being a tab. The `🤖 Ask` bar-button opens `AiPanel::ui` as a right slide-over — machinery already exists (`sdr_ai_panel_open` + right `egui::Panel` in `render_sdr_tab:2173`). Generalize it so it's available in every mode. Stays context-aware (already fed freq/mode/SNR via `pending_ai_freq`).

---

## Structural changes (reuse-first)

| Area | File:anchor | Change |
|---|---|---|
| Tab enum | `app.rs:35` `AppTab` | `Sdr, AdsB, Satellite, Ai, Decoding, Customize` → **`Listen, Planes, Satellites`**. Drop Ai/Decoding/Customize as top-level. |
| Nav render | `app.rs:1983` `render_main_nav` | Replace 48px left rail → **`render_mode_bar`** (top: 3 mode buttons + `🤖 Ask` + `⚙ More`). |
| Tab/tool metadata | `app.rs:48` `MAIN_TABS`, `app.rs:71` `SecondaryTool` | Repurpose `SecondaryTool` list as the **More-drawer** contents; add a `min_level()` per tool for gating. |
| Listen | `app.rs:2170` `render_sdr_tab` | → `render_listen`: add preset tiles, inline smart-tune, Auto chip, plain-language meter, "Adjust ▾" expander (mostly via `SdrPanel`). |
| Demod | `sdr_panel.rs:31` `DemodMode` | Add `Auto` variant + `for_frequency(hz)` (map band ranges from `identify_frequency` tips). Resolve Auto→concrete in demod loop (`app.rs:542`) + title/status (`app.rs:1096, 1808`). |
| Satellites | `satellite_panel.rs:15` `SatelliteSubTab` | Add `Decode`; subtab bar (`app.rs:2353`) gains it; handoff `app.rs:603-606` sets subtab instead of tab. |
| Planes | `adsb_panel.rs:1338` | Extract `AdsBPanel::begin()`; call on entering Planes. |
| Layout defaults | `config.rs:38` `LayoutConfig` | Update tab ids to `listen/planes/satellites`; keep plumbing + migration pattern (`config.rs:318`). |

**Deletions (after the above compile):** old left-rail rendering, the `AppTab::Ai/Decoding/Customize` match arms and their now-unused standalone render wrappers where fully absorbed. Keep all *panel* code — only their hosting changes.


## Phasing (each phase: `cargo build -p ez-gui` → `cargo test -p ez-gui` → `graphify update .`)

- **Phase 0 — Deliverable doc.** Write this spec to `Plan#2.md` at repo root (the artifact requested).
- **Phase 1 — Nav shell.** New `render_mode_bar`; collapse `AppTab` to 3; route Listen/Planes/Satellites; wire `⚙ More` drawer (all old secondary tools inside, ungated for now) and `🤖 Ask` slide-over. App compiles and runs with 3 modes.
- **Phase 2 — Listen simplicity.** `DemodMode::Auto` + `for_frequency`; preset tiles; inline smart-tune (reuse parser); plain-language signal meter; default-collapsed "Adjust ▾".
- **Phase 3 — Modes fold-in.** `AdsBPanel::begin()` auto-start on Planes entry; `SatelliteSubTab::Decode` + reroute decode handoff; delete standalone Decoding tab.
- **Phase 4 — Progressive gating + cleanup.** `min_level()` gating in the More drawer; update `LayoutConfig` defaults + migration; humanize status strip; remove dead rail code and unused fields.


## Verification

- **Build/tests:** `cargo build -p ez-gui` and `cargo test -p ez-gui` green after every phase (existing tests: `parse_hhmm_*`, `user_level_*`, `layout_config_*`, `config_*`).
- **Fresh Beginner run** (`cargo run -p ez-gui`, default config): only `Listen / Planes / Satellites` + `Ask` + `More` visible; tapping **📻 FM Radio** tunes ~88–108 MHz, sets WFM via Auto, starts source, produces audio — with no mode/frequency interaction.
- **Auto mode:** tune 145.5 → chip reads `Auto → FM`; tune 14.2 MHz → `Auto → USB`; tune 120 MHz → `Auto → AM`.
- **Planes:** entering the mode shows aircraft/list without a manual Start.
- **Satellites:** `Track · Align · Decode` subtabs; a satellite recording routes into **Decode** in-place (no tab jump); decode progress advances.
- **Progressive More:** at Beginner, "More" shows ~3 entries; setting User Level = Advanced adds Scanner/Recorder/Scheduler; Expert adds Discord/MQTT/Web-Remote/Layout.
- **No regressions:** keyboard shortcuts, recording, scanner, and the J jump dialog still function.

---

## Open choices deferred to build time (sensible defaults chosen)
- Preset-tile frequency set: start from FM/Air/Weather/Public-Safety/Ham-2m/Marine (extend later). Default chosen; easy to edit.
- Command-palette upgrade of the J dialog (run "record"/"scan"/"planes") is a low-cost **bonus**, included only if Phase 1–4 land cleanly.
