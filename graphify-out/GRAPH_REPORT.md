# Graph Report - ez-sdr  (2026-07-02)

## Corpus Check
- 62 files · ~118,068 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 1140 nodes · 2159 edges · 76 communities (46 shown, 30 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS · INFERRED: 10 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `9d95d8d5`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- [[_COMMUNITY_UI Application Core|UI Application Core]]
- [[_COMMUNITY_Spectrum Visualization|Spectrum Visualization]]
- [[_COMMUNITY_ADS-B Decoder|ADS-B Decoder]]
- [[_COMMUNITY_Configuration & UI|Configuration & UI]]
- [[_COMMUNITY_Tutorial & Help|Tutorial & Help]]
- [[_COMMUNITY_Demodulation|Demodulation]]
- [[_COMMUNITY_AI & Chat|AI & Chat]]
- [[_COMMUNITY_Sample Conversion|Sample Conversion]]
- [[_COMMUNITY_SDR Hardware Interface|SDR Hardware Interface]]
- [[_COMMUNITY_Frequency Scanner|Frequency Scanner]]
- [[_COMMUNITY_Community 10|Community 10]]
- [[_COMMUNITY_AI Integration|AI Integration]]
- [[_COMMUNITY_Community 12|Community 12]]
- [[_COMMUNITY_Configuration|Configuration]]
- [[_COMMUNITY_Hardware Interface|Hardware Interface]]
- [[_COMMUNITY_AI Tools|AI Tools]]
- [[_COMMUNITY_Community 16|Community 16]]
- [[_COMMUNITY_Community 17|Community 17]]
- [[_COMMUNITY_Demodulation Core|Demodulation Core]]
- [[_COMMUNITY_Community 19|Community 19]]
- [[_COMMUNITY_AI & Tools|AI & Tools]]
- [[_COMMUNITY_Community 21|Community 21]]
- [[_COMMUNITY_Community 22|Community 22]]
- [[_COMMUNITY_Community 23|Community 23]]
- [[_COMMUNITY_Community 24|Community 24]]
- [[_COMMUNITY_Community 25|Community 25]]
- [[_COMMUNITY_Web Remote|Web Remote]]
- [[_COMMUNITY_Community 27|Community 27]]
- [[_COMMUNITY_Visualization Core|Visualization Core]]
- [[_COMMUNITY_AI Agents|AI Agents]]
- [[_COMMUNITY_Settings|Settings]]
- [[_COMMUNITY_Demod Modules|Demod Modules]]
- [[_COMMUNITY_Community 32|Community 32]]
- [[_COMMUNITY_Community 33|Community 33]]
- [[_COMMUNITY_Community 34|Community 34]]
- [[_COMMUNITY_Community 35|Community 35]]
- [[_COMMUNITY_Community 36|Community 36]]
- [[_COMMUNITY_Satellite Tracking|Satellite Tracking]]
- [[_COMMUNITY_Remote Control|Remote Control]]
- [[_COMMUNITY_Community 40|Community 40]]
- [[_COMMUNITY_Community 42|Community 42]]
- [[_COMMUNITY_Hardware Control|Hardware Control]]
- [[_COMMUNITY_MQTT Publishing|MQTT Publishing]]
- [[_COMMUNITY_Community 45|Community 45]]
- [[_COMMUNITY_Community 46|Community 46]]
- [[_COMMUNITY_Community 47|Community 47]]
- [[_COMMUNITY_Community 48|Community 48]]
- [[_COMMUNITY_Community 49|Community 49]]
- [[_COMMUNITY_Community 50|Community 50]]
- [[_COMMUNITY_Community 51|Community 51]]
- [[_COMMUNITY_Community 52|Community 52]]
- [[_COMMUNITY_Community 53|Community 53]]
- [[_COMMUNITY_Community 54|Community 54]]
- [[_COMMUNITY_Community 55|Community 55]]
- [[_COMMUNITY_Community 56|Community 56]]
- [[_COMMUNITY_Community 57|Community 57]]
- [[_COMMUNITY_Community 58|Community 58]]
- [[_COMMUNITY_Community 59|Community 59]]
- [[_COMMUNITY_Community 60|Community 60]]
- [[_COMMUNITY_Community 61|Community 61]]
- [[_COMMUNITY_Community 62|Community 62]]
- [[_COMMUNITY_Community 63|Community 63]]
- [[_COMMUNITY_Community 64|Community 64]]
- [[_COMMUNITY_Community 65|Community 65]]
- [[_COMMUNITY_Community 66|Community 66]]
- [[_COMMUNITY_Community 67|Community 67]]
- [[_COMMUNITY_Community 68|Community 68]]
- [[_COMMUNITY_Community 69|Community 69]]
- [[_COMMUNITY_Community 70|Community 70]]
- [[_COMMUNITY_Community 71|Community 71]]
- [[_COMMUNITY_Community 72|Community 72]]
- [[_COMMUNITY_Community 73|Community 73]]
- [[_COMMUNITY_Community 74|Community 74]]
- [[_COMMUNITY_Community 75|Community 75]]

## God Nodes (most connected - your core abstractions)
1. `CentralApp` - 46 edges
2. `SharedState` - 40 edges
3. `HowToPanel` - 39 edges
4. `AdsBPanel` - 37 edges
5. `SpectrumAnalyzer` - 36 edges
6. `AiPanel` - 31 edges
7. `FrequencyScanner` - 29 edges
8. `RecorderPanel` - 26 edges
9. `MqttPublisher` - 22 edges
10. `AdaptiveGain` - 21 edges

## Surprising Connections (you probably didn't know these)
- `AdsBDecoder` --references--> `DemodStats`  [EXTRACTED]
  ez-gui/src/adsb_decoder.rs → dump1090/src/demod.rs
- `AdsBDecoder` --references--> `Demod2400`  [EXTRACTED]
  ez-gui/src/adsb_decoder.rs → dump1090/src/demod.rs
- `MqttPublisher` --references--> `Client`  [EXTRACTED]
  ez-gui/src/mqtt.rs → dump1090/src/net_io.rs
- `Demodulation Control Section` --references--> `Demodulators`  [INFERRED]
  ez-gui/src/web_remote.html → README.md
- `Satellite Passes Display` --references--> `Satellite Tracking`  [INFERRED]
  ez-gui/src/web_remote.html → README.md

## Import Cycles
- 1-file cycle: `ez-gui/src/audio_output.rs -> ez-gui/src/audio_output.rs`
- 2-file cycle: `ez-gui/src/app.rs -> ez-gui/src/satellite_panel.rs -> ez-gui/src/app.rs`
- 2-file cycle: `ez-gui/src/app.rs -> ez-gui/src/tutorial.rs -> ez-gui/src/app.rs`
- 2-file cycle: `ez-gui/src/app.rs -> ez-gui/src/user_level.rs -> ez-gui/src/app.rs`
- 2-file cycle: `ez-gui/src/ai_panel.rs -> ez-gui/src/app.rs -> ez-gui/src/ai_panel.rs`
- 2-file cycle: `ez-gui/src/adsb_panel.rs -> ez-gui/src/app.rs -> ez-gui/src/adsb_panel.rs`
- 2-file cycle: `ez-gui/src/app.rs -> ez-gui/src/recorder_panel.rs -> ez-gui/src/app.rs`
- 2-file cycle: `ez-gui/src/app.rs -> ez-gui/src/sdr_panel.rs -> ez-gui/src/app.rs`
- 3-file cycle: `ez-gui/src/app.rs -> ez-gui/src/tutorial.rs -> ez-gui/src/user_level.rs -> ez-gui/src/app.rs`
- 3-file cycle: `ez-gui/src/adsb_decoder.rs -> ez-gui/src/adsb_panel.rs -> ez-gui/src/app.rs -> ez-gui/src/adsb_decoder.rs`
- 3-file cycle: `ez-gui/src/adsb_panel.rs -> ez-gui/src/app.rs -> ez-gui/src/mqtt.rs -> ez-gui/src/adsb_panel.rs`

## Hyperedges (group relationships)
- **Spectrum Visualization Ecosystem** — readme_spectrum_analyser, readme_waterfall, readme_band_plan_overlay, anchored_summary_vfo_b_marker, anchored_summary_waterfall_colormap [EXTRACTED 0.95]
- **Demodulation and Signal Analysis Chain** — readme_demodulators, readme_adsb_decoder, night_work_frequency_based_mode, night_work_rf_filter_presets [INFERRED 0.85]
- **Web Remote Control Interface** — ez_gui_src_web_remote_html_frequency_control, ez_gui_src_web_remote_html_demodulation, ez_gui_src_web_remote_html_gain_recording, ez_gui_src_web_remote_html_satellite_passes [EXTRACTED 1.00]

## Communities (76 total, 30 thin omitted)

### Community 0 - "UI Application Core"
Cohesion: 0.13
Nodes (21): App, CreationContext, Demodulator, AppTab, CentralApp, parse_hhmm_today(), Arc, Context (+13 more)

### Community 1 - "Spectrum Visualization"
Cohesion: 0.07
Nodes (20): Complex32, category_color(), color_map(), ColorMap, lerp_color(), Arc, Color32, Option (+12 more)

### Community 2 - "ADS-B Decoder"
Cohesion: 0.09
Nodes (26): AcCategory, AdsBNotification, AdsBPanel, AircraftEntry, AircraftInfo, classify_aircraft(), draw_plane_model(), Arc (+18 more)

### Community 3 - "Configuration & UI"
Cohesion: 0.11
Nodes (20): AppConfig, ProviderPreset, Default, Self, String, Ui, Vec, bg_luminance() (+12 more)

### Community 4 - "Tutorial & Help"
Cohesion: 0.24
Nodes (5): HowToPanel, Self, String, Ui, Vec

### Community 5 - "Demodulation"
Cohesion: 0.07
Nodes (41): AntennaChecklist, ChecklistItem, crit(), item(), Arc, Mutex, Option, Self (+33 more)

### Community 6 - "AI & Chat"
Cohesion: 0.16
Nodes (17): AiPanel, ChatMessage, Arc, AtomicBool, Color32, Instant, Mutex, Option (+9 more)

### Community 7 - "Sample Conversion"
Cohesion: 0.09
Nodes (13): AsRef, IqFormat, IFileSdr, Option, Result, Self, String, Vec (+5 more)

### Community 8 - "SDR Hardware Interface"
Cohesion: 0.06
Nodes (32): c_char, Send, SdrSource, find_device_index(), Drop, Option, Result, Self (+24 more)

### Community 9 - "Frequency Scanner"
Cohesion: 0.13
Nodes (11): FrequencyScanner, HitsSort, Arc, Instant, Mutex, Option, Self, String (+3 more)

### Community 10 - "Community 10"
Cohesion: 0.07
Nodes (24): CustomTask, Option, Self, String, Vec, ScheduledJob, Scheduler, format_time() (+16 more)

### Community 11 - "AI Integration"
Cohesion: 0.12
Nodes (17): HackRf, HackRfConfig, HackRfCtx, HackrfDevice, HackrfTransfer, c_int, c_void, Default (+9 more)

### Community 12 - "Community 12"
Cohesion: 0.13
Nodes (18): rand_f64(), Arc, AtomicBool, c_void, Option, Receiver, Self, Sender (+10 more)

### Community 13 - "Configuration"
Cohesion: 0.16
Nodes (7): AdaptiveGain, count_loud_samples(), DecodedMessage, RangeScanState, Option, Self, Vec

### Community 14 - "Hardware Interface"
Cohesion: 0.07
Nodes (42): Box, BTreeMap, BTreeSet, Error, categories(), DiscordEmbed, DiscordNotifier, DiscordSettings (+34 more)

### Community 15 - "AI Tools"
Cohesion: 0.21
Nodes (16): cpr_airborne_decode(), cpr_dlon_function(), cpr_mod(), cpr_mod_double(), cpr_n_function(), cpr_nl_function(), CprCacheEntry, CprDecoder (+8 more)

### Community 16 - "Community 16"
Cohesion: 0.12
Nodes (7): Display, Default, Result, Self, Stats, Duration, Formatter

### Community 17 - "Community 17"
Cohesion: 0.16
Nodes (7): MqttPublisher, Arc, AtomicBool, Instant, Option, Self, String

### Community 18 - "Demodulation Core"
Cohesion: 0.19
Nodes (15): crc24_parity(), apply_bit_errors(), compute_magnitude(), compute_magnitude_sc16(), compute_magnitude_sc16q11(), compute_magnitude_uc8(), correct_message(), decode_mode_s_message() (+7 more)

### Community 19 - "Community 19"
Cohesion: 0.18
Nodes (11): Client, hex_digit(), NetIo, Arc, Default, Mutex, Result, Self (+3 more)

### Community 20 - "AI & Tools"
Cohesion: 0.12
Nodes (10): AudioOutput, Arc, Mutex, Option, Receiver, Result, Self, String (+2 more)

### Community 21 - "Community 21"
Cohesion: 0.17
Nodes (8): Condvar, Fifo, Inner, Mutex, Option, Self, Vec, VecDeque

### Community 22 - "Community 22"
Cohesion: 0.21
Nodes (9): AircraftRecord, BookmarkRecord, Database, PassRecord, Connection, Result, Self, String (+1 more)

### Community 23 - "Community 23"
Cohesion: 0.33
Nodes (3): check_crc(), crc24(), test_crc24_generates_correct_parity()

### Community 24 - "Community 24"
Cohesion: 0.22
Nodes (7): Bookmark, BookmarkDb, Default, Option, Self, String, Vec

### Community 26 - "Web Remote"
Cohesion: 0.13
Nodes (19): Airport, AirportDb, AirportFreq, antenna_dims(), AntennaDims, csv_split(), FreqType, Color32 (+11 more)

### Community 27 - "Community 27"
Cohesion: 0.22
Nodes (10): Args, compute_magbuf_stats(), Demodulator, main(), NetOutput, Default, Option, Result (+2 more)

### Community 29 - "AI Agents"
Cohesion: 0.29
Nodes (3): IcaoFilter, Default, Self

### Community 30 - "Settings"
Cohesion: 0.31
Nodes (7): Demod2400, generate_damage_set(), Default, Self, test_generate_damage_set(), valid_df_long(), valid_df_short()

### Community 31 - "Demod Modules"
Cohesion: 0.17
Nodes (12): DemodStats, MagBuf, MagBufFlags, receiveclock_ms_elapsed(), FnMut, Vec, slice_phase0(), slice_phase1() (+4 more)

### Community 32 - "Community 32"
Cohesion: 0.29
Nodes (5): AircraftState, Default, HashMap, Self, Tracker

### Community 33 - "Community 33"
Cohesion: 0.43
Nodes (6): ModesMessage, AircraftMessage, decode_mode_s(), decode_mode_s_message(), Option, String

### Community 34 - "Community 34"
Cohesion: 0.60
Nodes (5): check(), init(), _load(), _save(), status()

### Community 43 - "Hardware Control"
Cohesion: 0.22
Nodes (8): Build, Build & Run, Controls, Dependencies, EZ-SDR Unified, Features, Licence, Project Structure

### Community 45 - "Community 45"
Cohesion: 0.10
Nodes (21): Tab, advanced_steps(), beginner_steps(), clerk_maxwell_steps(), intermediate_steps(), render_tutorial(), Arc, Mutex (+13 more)

### Community 46 - "Community 46"
Cohesion: 0.07
Nodes (26): All Subsystems Verified, Architecture, Autonomous Daemon — Final Comprehensive Status Report, Beginner Experience (Verified), Build & Test, Code Metrics Summary, Code Quality, Daemon Protocol Status (+18 more)

### Community 47 - "Community 47"
Cohesion: 0.15
Nodes (15): BufWriter, free_disk_space_with_timeout(), RecorderPanel, RecordingFile, Arc, Instant, Mutex, Option (+7 more)

### Community 48 - "Community 48"
Cohesion: 0.12
Nodes (14): AircraftState, CprFrame, AdsBDecoder, AircraftState, CprFrame, decode_altitude(), decoder_new_starts_empty(), HashMap (+6 more)

### Community 49 - "Community 49"
Cohesion: 0.21
Nodes (10): demod_am_dc_signal_envelope(), demod_fm_constant_phase_near_zero(), demod_raw_output_length(), demod_wfm_produces_output(), Demodulator, make_iq_dc(), Self, Vec (+2 more)

### Community 50 - "Community 50"
Cohesion: 0.10
Nodes (19): Architecture Notes, `ez-gui/src/adsb_panel.rs` — ADS-B Aircraft Tracking, `ez-gui/src/app.rs` — Central Application + TabViewer, `ez-gui/src/config.rs` — AppConfig + Settings UI, `ez-gui/src/main.rs` — Entry Point, `ez-gui/src/satellite_panel.rs` — Satellite Tracking, `ez-gui/src/scanner.rs` — Frequency Scanner, `ez-gui/src/sdr_panel.rs` — SDR Controls Panel (+11 more)

### Community 51 - "Community 51"
Cohesion: 0.11
Nodes (17): Autonomous Session Log — 2026-07-01, Commit 1: Optional Audio Feature, Commit 2: Fix CPR Test, Commits Made, Iteration 12+ Work (Current), Iteration 3 & 4 Work, Iterations 5-11 Work, Key Achievements (+9 more)

### Community 52 - "Community 52"
Cohesion: 0.14
Nodes (13): A. First-run / Onboarding (CRITICAL), B. Help System for Absolute Beginners (HIGH), Build environment, C. Empty States + Action Hints (HIGH), Completed (11 improvements), D. De-jargon Controls (MEDIUM), E. Layout (MEDIUM), EZ-SDR Autonomous Night Session (+5 more)

### Community 53 - "Community 53"
Cohesion: 0.17
Nodes (11): Anchored Working Summary, Constraints & Preferences, Critical Context, Current Codebase State, Done, Goal, In Progress, Key Decisions (+3 more)

### Community 54 - "Community 54"
Cohesion: 0.20
Nodes (9): 1. The Core Autonomous Loop, 2. Project Environment & Tooling, 3. Guardrails & Termination, CLAUDE.md — Autonomous 24/7 Daemon Protocol, graphify, Phase 1: Observe & Discover, Phase 2: Self-Directed Planning, Phase 3: Surgical Execution (+1 more)

## Knowledge Gaps
- **107 isolated node(s):** `SoapySDRRange`, `ProviderPreset`, `Goal`, `Constraints & Preferences`, `Done` (+102 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **30 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `CentralApp` connect `UI Application Core` to `ADS-B Decoder`, `Tutorial & Help`, `Demodulation`, `AI & Chat`, `Frequency Scanner`, `Community 10`, `Community 12`, `Community 45`, `Hardware Interface`, `Community 47`, `Community 48`, `Community 17`, `AI & Tools`?**
  _High betweenness centrality (0.155) - this node is a cross-community bridge._
- **Why does `SharedState` connect `Demodulation` to `UI Application Core`, `Spectrum Visualization`, `ADS-B Decoder`, `Configuration & UI`, `AI & Chat`, `Frequency Scanner`, `Community 10`, `Community 12`, `Community 45`, `Hardware Interface`, `Community 47`, `Community 24`?**
  _High betweenness centrality (0.139) - this node is a cross-community bridge._
- **Why does `AdsBDecoder` connect `Community 48` to `UI Application Core`, `Settings`, `Demod Modules`?**
  _High betweenness centrality (0.124) - this node is a cross-community bridge._
- **What connects `SoapySDRRange`, `ProviderPreset`, `Goal` to the rest of the system?**
  _117 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `UI Application Core` be split into smaller, more focused modules?**
  _Cohesion score 0.13090418353576247 - nodes in this community are weakly interconnected._
- **Should `Spectrum Visualization` be split into smaller, more focused modules?**
  _Cohesion score 0.06956521739130435 - nodes in this community are weakly interconnected._
- **Should `ADS-B Decoder` be split into smaller, more focused modules?**
  _Cohesion score 0.08792270531400966 - nodes in this community are weakly interconnected._