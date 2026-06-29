# EZ-SDR Autonomous Night Session
Started: 2026-06-29 (8-hour window)

## Improvement Backlog (ordered by impact)

### Session 1 — UX & Beginner Polish (CURRENT)
- [x] Tab labels with emoji icons
- [x] Band presets auto-set demod mode
- [x] Demo mode badge in status bar
- [x] File browse button for replay mode
- [x] Keyboard shortcut M = toggle audio mute
- [x] Auto-save config on Apply
- [x] Add more band presets (APRS, Marine, PMR446, NOAA WX, Emergency)
- [x] SDR panel +10k/-10k step buttons (currently only in source panel)

### Session 2 — Spectrum Visual Improvements
- [x] Waterfall: show bookmark frequency markers as vertical lines
- [x] Spectrum: animated noise floor line (pulsing blue)
- [x] SNR badge on spectrum (live, colour coded) — was already done
- [x] Spectrum control bar tooltips improvements — was already done
- [x] Show "ZOOM: Nx" badge on spectrum when zoomed — was already done

### Session 3 — Scanner Improvements
- [x] Scanner hits bar chart (strength histogram) — frequency distribution chart above hits table
- [x] Scanner: deduplicate hits within ±step_hz — was already done
- [ ] Scanner: show current sweep position marker on spectrum
- [x] Scanner: visual alert on hit — flashing green "● HIT!" badge

### Session 4 — Config & Persistence
- [ ] Config: font size slider
- [ ] Config: export/import JSON button
- [ ] Config: recently used frequencies list
- [ ] Config: persist zoom/waterfall settings

### Session 5 — Bookmarks
- [x] Bookmarks: edit existing bookmark (name/freq/mode) — was already done
- [x] Bookmarks: tune + shows notes in tooltip — added notes to Tune button tooltip
- [x] Bookmarks: show bookmark count per category in collapsing header — was already done

### Session 6 — Scheduler
- [x] Scheduler: show countdown timer to next event — was already done
- [x] Scheduler: visual timeline of today's passes — 24-hr bar with pass blocks + hover tooltip

### Session 7 — Misc QoL
- [x] Add "Copy to clipboard" on frequency display — click status bar freq to copy to clipboard
- [ ] Add "Reset all panels to default layout" button
- [ ] Keyboard shortcut cheatsheet improvements
- [x] Status bar: show recording duration when recording — was already done

### Session 8 — Final Polish
- [ ] Build and verify no warnings
- [ ] Review and clean up any leftover TODOs
- [ ] Update HowTo panel with new features

## Session Log
- Session 1: Starting now (emoji tabs, band auto-demod, demo badge, file picker, shortcuts, presets)
- Session 19 (Night Shift): Started 2026-06-29T20:12Z, end target 2026-06-30T04:12Z. Goal: work through backlog from Session 2 onward — spectrum improvements, scanner, config, bookmarks, scheduler, misc QoL. Timer: python3 /home/lupc/Documents/ez-sdr/timer.py check
