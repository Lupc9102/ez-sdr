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

## Session 19 Night Shift Log
- T+15min: Ctrl+R recording toggle, waterfall time axis, squelch SQ badge
- T+20min: Spectrum right-click context menu (Copy freq, Set squelch, Scan range)
- T+25min: Scanner hold-on-activity mode
- T+30min: S-meter bargraph in status bar
- T+35min: HowTo panel documentation for all night-shift features
- T+40min: Bookmark CSV import (dedup + column auto-detect)
- T+45min: Audio clipping indicator in status bar
- T+50min: 'C' key cycles waterfall colormap
- T+55min: Spectrum→Scanner integration (scan visible range button)
- T+60min: Peak hold: label top-5 frequency peaks with stems
- T+65min: PPM correction persistance in config
- T+70min: Recording metadata sidecar JSON files
- T+72min: Smart demod auto-suggest for known frequency bands (FM, aviation, marine, etc.)
- T+72min: Dual VFO A/B with ⇄ swap button + V key shortcut + config persistence
- T+75min: Smart gain / overload detection (ADC clip warning + -10dB button + Smart Gain auto-adjust)
- T+78min: Waterfall brightness/contrast: separate wf_min_db/wf_max_db + WF Auto button
- T+80min: Scanner CSV export: rfd file dialog + grouped by freq (BTreeMap dedup, max strength, hit count)
- T+82min: Numeric keys 1-9 bookmark quick-tune + [N] indicators in bookmark list
- T+84min: B key snap to nearest bookmark + status flash (3s fade on screen)
- T+86min: Frequency identification panel (30+ allocations, 📻 collapsing) in SDR panel
- T+88min: Configurable arrow key tuning step presets row + Shift×10 multiplier
- T+91min: Signal strength history sparkline (60-second chart below S-meter, colour coded)
- T+~110min: HowTo panel updates for all Session 19 features (VFO A/B, steps, freq ID, demod suggest, sparkline, overload, WF range, sidecar JSON, CSV export, bookmark shortcuts)

## Session 20 Night Shift Log
- T+~115min: Auto-squelch tracking mode — "Track ON" toggle continuously follows noise floor + configurable offset
- T+~125min: Bookmark memory scan — 📻 Memory Scan section in Scanner tab, cycles bookmarks, applies each bookmark's demod mode, hold-on-activity support
- T+~125min: Keyboard shortcut overlay updated (configurable step hints, Shift+Arrow added)
- T+~135min: Recent frequencies quick-access bar in SDR panel (last 6 unique freqs as clickable chips)
- T+~140min: Bookmark overlay color-coding by category (aviation=blue, weather=green, marine=teal, amateur=purple, broadcast=orange, scanner=red)
- T+~145min: Band plan overlay expanded from 9 to 45+ allocations covering all major VHF/UHF bands
- T+~155min: VOX / squelch-triggered recording — auto-start/stop recording on signal, configurable tail delay
- T+~165min: Band plan toggle (🗺 BP button) + color legend in tooltip
- T+~170min: Labeled spectrum markers — "Add marker" popup dialog for optional label text
- T+~180min: Bookmark CSV export with file dialog (+ frequency_mhz column)
- T+~182min: Scanner exclude list — skip known interferer frequencies during range scan
- T+~195min: Scanner: 🚫 button on hits to add/remove from exclude list; excluded hits shown with strikethrough
- T+~200min: HowTo panel search/filter — keyword search with gold highlighting, 50+ term index

## Session 21 Night Shift Log
- T+~205min: T key shortcut — tune to strongest visible spectrum peak (peak_freq_hz() argmax method, status flash)
- T+~210min: Upconverter/LO offset support — DragValue + quick presets (0/125/100/-125 MHz), true-freq display in SDR panel + amber status bar label with hover tooltip, persisted in config with Ctrl+S
- T+~220min: Signal event log — timestamped ring buffer (200 entries) in Recorder tab, throttled to 1 entry/5s per signal, color-coded dB, "Export CSV" button, separate from VOX recording
- T+~225min: VFO BW overlay enhanced with mode-aware colors (orange=WFM, purple=AM, green=SSB, blue=NFM/FM/RAW) + mode label shown in BW indicator text
- T+~230min: Spectrum cursor tooltip enhanced — now shows absolute freq + Δ offset from center (±kHz/MHz) + dB level; tooltip flips to left when near right edge

## Session 22 Night Shift Log
- T+~235min: Spectrum right-click: inline frequency ID (band, description, tips) from identify_frequency
- T+~238min: Bookmark panel: category quick-filter chip row (blue chips, click to filter, click again to clear)
- T+~242min: Spectrum averaging: Fast/Med/Slow/XSlow preset buttons replace raw α slider
- T+~248min: Scanner: hit_count field tracks repeat detections; ×N column in hits table with heat coloring; "Sort by hits" button
- T+~250min: Scanner: "🎯 Top: X.XXX MHz (×N)" button tunes to most-active frequency
- T+~254min: Spectrum context menu: "Tune + Bookmark" combined action; "Bookmark only" renamed for clarity

## Session 25 Night Shift Log
- T+~260min: Satellite panel: live Doppler display with color coding (green/yellow/orange by magnitude kHz/Hz), "✓ Corrected" inline badge when auto-tune active; app.rs feeds real doppler_hz back into satellite_panel each frame; Doppler correction now respects satellite_panel.auto_tune flag
- T+~262min: Status bar: 🛰 Doppler badge shows live shift when satellite selected; green=correcting, yellow=not correcting

## Session 26 Night Shift Log
- T+~265min: HowTo panel updates for Sessions 21-25: LO offset docs, T key shortcut, status bar amber/Doppler badge, spectrum cursor delta tooltip, averaging presets, VFO BW mode colors, context menu freq-ID, scanner hit counts/sort/Top button, signal event log, bookmark category chips, satellite Doppler display

## Session 27 Night Shift Log
- T+~270min: Band name overlay on spectrum top-left corner (calls identify_frequency; semi-transparent badge shows e.g. "FM Broadcast" for recognized bands)
- T+~272min: P key toggles spectrum peak hold on/off with status flash; toggle_peak_hold() pub method; P documented in shortcut overlay and HowTo

## Session 28 Night Shift Log
- T+~275min: Scanner: 💾 Save Hits / 📂 Load Hits JSON buttons — persist hit list across sessions; simple JSON format; merge deduplication on load

## Session 29 Night Shift Log
- T+~278min: Recorder: configurable filename template ({date}, {freq}, {freq1}, {mode} tokens); live filename preview shown below the field; Reset button; IQ/WAV/sidecar JSON all use template

## Session 30 Night Shift Log
- T+~282min: Adaptive dB grid lines on spectrum — 20/10/5 dB step based on display range; labels on right edge; 0 dB emphasized
- T+~284min: Recorder quick-start presets — 30s/1m/5m/10m buttons; start recording immediately with auto-stop

## Session 31 Night Shift Log
- T+~288min: J key opens frequency jump dialog — popup with text input, numeric or band-name search, bookmark search; Enter tunes, Esc closes; Ctrl+J also works

## Session 32 Night Shift Log
- T+~292min: Ctrl++ / Ctrl+- / Ctrl+0 zoom shortcuts — zoom in, zoom out, reset zoom; documented in shortcut overlay

## Session 33 Night Shift Log
- T+~297min: Green "● ACTIVE" badge on spectrum when squelch is open — appears top-right below SNR badge; only visible when squelch is engaged (> -90 dB) and signal exceeds threshold

## Session 34 Night Shift Log
- T+~305min: Spectrum CSV export — "💾 CSV" toolbar button exports current FFT (frequency_hz, power_dbfs) to CSV file via file dialog; frequencies reordered low→high

## Session 35 Night Shift Log
- T+~308min: Audio level bar meter in SDR panel — replaces plain "Audio: X%" with visual green/red bar; shows normalized audio output level with hover tooltip

## Session 36 Night Shift Log
- T+~312min: Marker delta measurement on spectrum — when 2+ markers visible, draws span indicator between first two with Δ kHz/MHz label at bottom of spectrum

## Session 37 Night Shift Log
- T+~318min: Last-signal activity badge on spectrum — when squelch configured: green "● ACTIVE" when open, fading "Last: Xs/Xm/Xh ago" after drop, dim "No activity" if never seen

## Session 38 Night Shift Log
- T+~330min: HowTo panel updates for Sessions 27-37: band name overlay, P key peak hold, J key freq jump, Ctrl±/0 zoom shortcuts, CSV export, marker delta, last-signal badge, scanner JSON save/load, recorder filename template, quick-start presets, audio level meter

## Session 39 Night Shift Log
- T+~335min: Peak frequency in spectrum info bar — "⊕ X.XXX MHz" readout shows strongest visible signal frequency; hover tooltip shows 4-decimal MHz + T key reminder

## Session 40 Night Shift Log
- T+~340min: Noise floor trend indicator — slow-tracking baseline detects floor jumps >3 dB; shows yellow/red "⚠ Floor +N dB" in info bar to alert about interference

## Session 41 Night Shift Log
- T+~345min: Nearby bookmark hint in SDR panel — when within ±100 kHz of any bookmark, shows "Near: <name> (↑/↓ X kHz away)" with hover detail + B key reminder

## Session 42 Night Shift Log
- T+~350min: 3 dB bandwidth estimator in spectrum right-click menu — instant computation of signal bandwidth at cursor; shows BW in Hz/kHz/MHz

## Session 43 Night Shift Log
- T+~353min: Signal type suggestion from bandwidth — extends BW estimator with "💡 Could be: NFM voice / SSB / WFM / ..." hint based on measured bandwidth ranges

## Session 44 Night Shift Log
- T+~356min: Waterfall PNG screenshot — "📸" toolbar button saves full waterfall history as PNG via file dialog using image crate
