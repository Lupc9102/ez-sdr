# Anchored Working Summary

## Goal
Implement autonomous night-long codebase improvements for an SDR application (ez-sdr), fixing all compiler warnings and adding meaningful UI/UX enhancements.

## Constraints & Preferences
- Work until 5:45 AM GMT+3 continuously, committing after each change
- Must survive context compactions by keeping a clean anchored summary
- Build uses PKG_CONFIG_PATH=/tmp/opencode/sysroot/usr/lib/x86_64-linux-gnu/pkgconfig for ALSA
- User wants to see continuous progress, not a summary of past work

## Progress

### Done
- Fixed RichText::bold() → strong() for egui 0.35 compatibility in mode guide (commit d5924f8)
- Phase 1: Fixed all 52 compiler warnings in dump1090 (commit 2d10908)
- Phase 2: ADSB altitude layer — observer marker, range rings, altitude legend (commit ccabfbf)
- Phase 3: Waterfall colormap persistence in config (commit 8d40b45)
- Phase 4: Enhanced MQTT status indicator (commit 4f2fb9f)
- Phase 5: Frequency band presets dropdown (commit c23126f)
- Added Inferno and Turbo waterfall colormaps (commit c5ab379)
- **VFO B frequency marker** — dashed blue line on spectrum + waterfall, toggle button, right-click "Set as VFO B" (commits eb717d7, 7eabc8e)
- **Waterfall/CSV export sidecar metadata** — each PNG screenshot and CSV export now writes a companion .json with capture parameters (commit 5a5acfb, 6f39144)
- **Frequency memory labels** — M1-M9 now store labels alongside frequencies, auto-named on save (commit cf7cb8c)

### In Progress
- (none currently)

## Key Decisions
- `.bold()` → `.strong()` because egui 0.35 removed `.bold()` from RichText
- Dead code kept with `#[allow(dead_code)]` for SoapySDR FFI surface
- MQTT status propagated via SharedState for sdr_panel display
- VFO B marker uses dashed line to distinguish from solid bookmark/VFO BW edge lines
- Band presets stored inline as const slice

## Current Codebase State
- **Zero compiler warnings** on both `ez-gui` and `dump1090`
- All phases 1–5 complete + VFO B markers + colormaps + export sidecars + freq mem labels

## Next Steps (if continuing)
1. Audio waveform display below signal history chart
2. Signal history persistence to disk
3. CPU/memory usage in status bar
4. Persist freq memory labels in AppConfig
5. Add label editing UI for M1-M9 in SDR panel

## Critical Context
- Build: `PKG_CONFIG_PATH=/tmp/opencode/sysroot/usr/lib/x86_64-linux-gnu/pkgconfig cargo check -p ez-gui`
- No git remote; commits local on master
- ColorMap: 8 variants (Classic, Viridis, Plasma, Magma, Grayscale, Hot, Inferno, Turbo)
- Current time: ~23:25 GMT+3 on June 30, 2026

## Relevant Files
- `ez-gui/src/spectrum.rs`: Waterfall/spectrum rendering, ColorMap, signal_history, VFO B marker, palette, context menus, export sidecars
- `ez-gui/src/sdr_panel.rs`: Central SDR control, frequency, gain, mode, band presets, step controls, memory display
- `ez-gui/src/app.rs`: Main app shell, status badges, keyboard shortcuts, config save/restore, FreqMemEntry definition, VFO B handling
- `ez-gui/src/sdr_panel.rs`: Central SDR control, frequency, gain, mode, band presets, step controls
- `ez-gui/src/app.rs`: Main app shell, status badges, keyboard shortcuts, config save/restore
- `ez-gui/src/config.rs`: AppConfig persistence
- `ez-gui/src/adsb_panel.rs`: ADSB aircraft map, altitude layers, observer
- `ez-gui/src/demod.rs`: FM/AM/SSB/WFM demodulator
- `ez-gui/src/recorder_panel.rs`: Recording with sidecar metadata
- `ez-gui/src/scanner.rs`: Frequency scanner with memory scan
- `ez-gui/src/scheduler.rs`: Satellite pass scheduler
- `ez-gui/src/bookmarks.rs`: Bookmark DB with categories
- `ez-gui/src/mqtt.rs`: MQTT publisher
- `dump1090/src/demod.rs`: Mode S demodulator
