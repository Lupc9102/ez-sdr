# Graph Report - .  (2026-07-01)

## Corpus Check
- 47 files · ~93,624 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 837 nodes · 1626 edges · 45 communities (38 shown, 7 thin omitted)
- Extraction: 99% EXTRACTED · 1% INFERRED · 0% AMBIGUOUS · INFERRED: 23 edges (avg confidence: 0.84)
- Token cost: 40,661 input · 0 output

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
- [[_COMMUNITY_Demod Pipeline|Demod Pipeline]]
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

## God Nodes (most connected - your core abstractions)
1. `HowToPanel` - 38 edges
2. `SpectrumAnalyzer` - 36 edges
3. `SharedState` - 33 edges
4. `AiPanel` - 31 edges
5. `CentralApp` - 31 edges
6. `FrequencyScanner` - 30 edges
7. `RecorderPanel` - 27 edges
8. `AdsBPanel` - 22 edges
9. `MqttPublisher` - 22 edges
10. `AdaptiveGain` - 21 edges

## Surprising Connections (you probably didn't know these)
- `VFO B Frequency Marker` --rationale_for--> `Spectrum Analyser`  [INFERRED]
  ANCHORED_SUMMARY.md → README.md
- `S-Meter Signal Indicator` --rationale_for--> `Spectrum Analyser`  [INFERRED]
  NIGHT_WORK.md → README.md
- `Frequency Memory Labels M1-M9` --rationale_for--> `SDR Panel.rs Control Module`  [INFERRED]
  ANCHORED_SUMMARY.md → README.md
- `Gain Recording Scanner Control` --references--> `SDR Panel.rs Control Module`  [INFERRED]
  ez-gui/src/web_remote.html → README.md
- `Gain Optimization Suggestions` --rationale_for--> `SDR Panel.rs Control Module`  [INFERRED]
  NIGHT_WORK.md → README.md

## Import Cycles
- 2-file cycle: `ez-gui/src/ai_panel.rs -> ez-gui/src/app.rs -> ez-gui/src/ai_panel.rs`
- 2-file cycle: `ez-gui/src/app.rs -> ez-gui/src/recorder_panel.rs -> ez-gui/src/app.rs`
- 2-file cycle: `ez-gui/src/app.rs -> ez-gui/src/sdr_panel.rs -> ez-gui/src/app.rs`
- 2-file cycle: `ez-gui/src/adsb_panel.rs -> ez-gui/src/app.rs -> ez-gui/src/adsb_panel.rs`
- 2-file cycle: `ez-gui/src/app.rs -> ez-gui/src/satellite_panel.rs -> ez-gui/src/app.rs`
- 3-file cycle: `ez-gui/src/adsb_decoder.rs -> ez-gui/src/adsb_panel.rs -> ez-gui/src/app.rs -> ez-gui/src/adsb_decoder.rs`
- 3-file cycle: `ez-gui/src/adsb_panel.rs -> ez-gui/src/app.rs -> ez-gui/src/mqtt.rs -> ez-gui/src/adsb_panel.rs`
- 4-file cycle: `ez-gui/src/adsb_decoder.rs -> ez-gui/src/demod.rs -> ez-gui/src/sdr_panel.rs -> ez-gui/src/app.rs -> ez-gui/src/adsb_decoder.rs`

## Hyperedges (group relationships)
- **Spectrum Visualization Ecosystem** — readme_spectrum_analyser, readme_waterfall, readme_band_plan_overlay, anchored_summary_vfo_b_marker, anchored_summary_waterfall_colormap [EXTRACTED 0.95]
- **Demodulation and Signal Analysis Chain** — readme_demodulators, readme_adsb_decoder, night_work_frequency_based_mode, night_work_rf_filter_presets [INFERRED 0.85]
- **Web Remote Control Interface** — ez_gui_src_web_remote_html_frequency_control, ez_gui_src_web_remote_html_demodulation, ez_gui_src_web_remote_html_gain_recording, ez_gui_src_web_remote_html_satellite_passes [EXTRACTED 1.00]

## Communities (45 total, 7 thin omitted)

### Community 0 - "UI Application Core"
Cohesion: 0.05
Nodes (52): App, BufWriter, CreationContext, Demodulator, DockState, CentralApp, FreqMemEntry, parse_hhmm_today() (+44 more)

### Community 1 - "Spectrum Visualization"
Cohesion: 0.07
Nodes (20): Complex32, category_color(), color_map(), ColorMap, lerp_color(), Arc, Color32, Option (+12 more)

### Community 2 - "ADS-B Decoder"
Cohesion: 0.07
Nodes (29): AircraftState, CprFrame, AdsBDecoder, AircraftState, CprFrame, decode_altitude(), HashMap, Instant (+21 more)

### Community 3 - "Configuration & UI"
Cohesion: 0.11
Nodes (20): AppConfig, ProviderPreset, Default, Self, String, Ui, Vec, bg_luminance() (+12 more)

### Community 4 - "Tutorial & Help"
Cohesion: 0.25
Nodes (5): HowToPanel, Self, String, Ui, Vec

### Community 5 - "Demodulation"
Cohesion: 0.10
Nodes (20): Demodulator, Self, Vec, DemodMode, format_hz(), FreqIdInfo, identify_frequency(), Arc (+12 more)

### Community 6 - "AI & Chat"
Cohesion: 0.17
Nodes (17): AiPanel, ChatMessage, Arc, AtomicBool, Color32, Instant, Mutex, Option (+9 more)

### Community 7 - "Sample Conversion"
Cohesion: 0.10
Nodes (13): AsRef, IqFormat, IFileSdr, Option, Result, Self, String, Vec (+5 more)

### Community 8 - "SDR Hardware Interface"
Cohesion: 0.10
Nodes (20): c_char, Send, SdrSource, last_err(), c_int, Default, Drop, Option (+12 more)

### Community 9 - "Frequency Scanner"
Cohesion: 0.13
Nodes (11): FrequencyScanner, HitsSort, Arc, Instant, Mutex, Option, Self, String (+3 more)

### Community 10 - "Community 10"
Cohesion: 0.12
Nodes (16): CustomTask, Option, Self, String, Vec, ScheduledJob, Scheduler, format_time() (+8 more)

### Community 11 - "AI Integration"
Cohesion: 0.12
Nodes (17): HackRf, HackRfConfig, HackRfCtx, HackrfDevice, HackrfTransfer, c_int, c_void, Default (+9 more)

### Community 12 - "Community 12"
Cohesion: 0.12
Nodes (19): rand_f64(), Arc, AtomicBool, c_void, Default, Option, Receiver, Self (+11 more)

### Community 13 - "Configuration"
Cohesion: 0.16
Nodes (7): AdaptiveGain, count_loud_samples(), DecodedMessage, RangeScanState, Option, Self, Vec

### Community 14 - "Hardware Interface"
Cohesion: 0.15
Nodes (12): find_device_index(), Drop, Option, Result, Self, Send, String, Vec (+4 more)

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
Cohesion: 0.18
Nodes (14): compute_magnitude(), compute_magnitude_sc16(), compute_magnitude_sc16q11(), compute_magnitude_uc8(), decode_mode_s_message(), InputFormat, mode_s_message_len_by_type(), Result (+6 more)

### Community 19 - "Community 19"
Cohesion: 0.20
Nodes (10): Client, hex_digit(), NetIo, Arc, Mutex, Result, Self, Vec (+2 more)

### Community 20 - "AI & Tools"
Cohesion: 0.12
Nodes (10): AudioOutput, Arc, Mutex, Option, Receiver, Result, Self, String (+2 more)

### Community 21 - "Community 21"
Cohesion: 0.17
Nodes (8): Condvar, Fifo, Inner, Mutex, Option, Self, Vec, VecDeque

### Community 22 - "Community 22"
Cohesion: 0.21
Nodes (9): Connection, AircraftRecord, BookmarkRecord, Database, PassRecord, Result, Self, String (+1 more)

### Community 23 - "Community 23"
Cohesion: 0.18
Nodes (10): check_crc(), crc24(), crc24_parity(), test_crc24_generates_correct_parity(), apply_bit_errors(), correct_message(), Default, score_mode_s_message() (+2 more)

### Community 24 - "Community 24"
Cohesion: 0.22
Nodes (7): Bookmark, BookmarkDb, Default, Option, Self, String, Vec

### Community 25 - "Community 25"
Cohesion: 0.14
Nodes (14): Waterfall/CSV Export Sidecar Metadata, Frequency Memory Labels M1-M9, Frequency Control Section, Gain Recording Scanner Control, ADS-B 3D View with Distance/Bearing, Gain Optimization Suggestions, Quick Setup Wizard, ADS-B Panel.rs UI (+6 more)

### Community 26 - "Web Remote"
Cohesion: 0.20
Nodes (8): RemoteCommand, Option, Receiver, Self, Sender, String, Vec, WebRemote

### Community 27 - "Demod Pipeline"
Cohesion: 0.23
Nodes (9): Args, compute_magbuf_stats(), Demodulator, main(), NetOutput, Option, Result, Self (+1 more)

### Community 28 - "Visualization Core"
Cohesion: 0.20
Nodes (10): VFO B Frequency Marker, Waterfall Colormap Persistence, Demodulation Control Section, Frequency-based Mode Suggestion, RF Filter Presets, S-Meter Signal Indicator, Band Plan Overlay, Demodulators (+2 more)

### Community 29 - "AI Agents"
Cohesion: 0.29
Nodes (3): IcaoFilter, Default, Self

### Community 30 - "Settings"
Cohesion: 0.36
Nodes (6): Demod2400, generate_damage_set(), Self, test_generate_damage_set(), valid_df_long(), valid_df_short()

### Community 31 - "Demod Modules"
Cohesion: 0.25
Nodes (7): DemodStats, MagBuf, MagBufFlags, receiveclock_ms_elapsed(), FnMut, Vec, FnMut

### Community 32 - "Community 32"
Cohesion: 0.31
Nodes (5): ModesMessage, AircraftState, HashMap, Self, Tracker

### Community 33 - "Community 33"
Cohesion: 0.48
Nodes (5): AircraftMessage, decode_mode_s(), decode_mode_s_message(), Option, String

### Community 34 - "Community 34"
Cohesion: 0.60
Nodes (5): check(), init(), _load(), _save(), status()

### Community 37 - "Satellite Tracking"
Cohesion: 0.67
Nodes (3): Satellite Passes Display, Satellite Tracking, TLE Engine.rs Module

## Knowledge Gaps
- **17 isolated node(s):** `SoapySDRRange`, `ProviderPreset`, `EZ-SDR Unified`, `Band Plan Overlay`, `ADS-B Decoder` (+12 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **7 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `CentralApp` connect `UI Application Core` to `ADS-B Decoder`, `Tutorial & Help`, `Demodulation`, `AI & Chat`, `Frequency Scanner`, `Community 12`, `Community 17`, `AI & Tools`, `Web Remote`?**
  _High betweenness centrality (0.140) - this node is a cross-community bridge._
- **Why does `AdsBDecoder` connect `ADS-B Decoder` to `UI Application Core`, `Settings`, `Demod Modules`?**
  _High betweenness centrality (0.135) - this node is a cross-community bridge._
- **Why does `SharedState` connect `UI Application Core` to `Spectrum Visualization`, `ADS-B Decoder`, `Configuration & UI`, `Demodulation`, `AI & Chat`, `Frequency Scanner`, `Community 10`, `Community 12`, `Community 24`?**
  _High betweenness centrality (0.126) - this node is a cross-community bridge._
- **What connects `SoapySDRRange`, `ProviderPreset`, `EZ-SDR Unified` to the rest of the system?**
  _27 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `UI Application Core` be split into smaller, more focused modules?**
  _Cohesion score 0.05257312106627175 - nodes in this community are weakly interconnected._
- **Should `Spectrum Visualization` be split into smaller, more focused modules?**
  _Cohesion score 0.07149758454106281 - nodes in this community are weakly interconnected._
- **Should `ADS-B Decoder` be split into smaller, more focused modules?**
  _Cohesion score 0.07308970099667775 - nodes in this community are weakly interconnected._