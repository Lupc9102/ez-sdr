# Graph Report - ez-sdr  (2026-07-03)

## Corpus Check
- 60 files · ~124,756 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 1336 nodes · 2557 edges · 81 communities (51 shown, 30 thin omitted)
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS · INFERRED: 12 edges (avg confidence: 0.84)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `b246c010`
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
- [[_COMMUNITY_Community 30|Community 30]]
- [[_COMMUNITY_Community 31|Community 31]]
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
- [[_COMMUNITY_Community 76|Community 76]]
- [[_COMMUNITY_Community 77|Community 77]]
- [[_COMMUNITY_Community 78|Community 78]]
- [[_COMMUNITY_Community 79|Community 79]]
- [[_COMMUNITY_Community 80|Community 80]]

## God Nodes (most connected - your core abstractions)
1. `CentralApp` - 46 edges
2. `SharedState` - 40 edges
3. `HowToPanel` - 37 edges
4. `SpectrumAnalyzer` - 35 edges
5. `AdsBPanel` - 34 edges
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
- `StreamParams` --references--> `Client`  [EXTRACTED]
  ez-gui/src/ai_panel.rs → dump1090/src/net_io.rs
- `MqttPublisher` --references--> `Client`  [EXTRACTED]
  ez-gui/src/mqtt.rs → dump1090/src/net_io.rs
- `Demodulation Control Section` --references--> `Demodulators`  [INFERRED]
  ez-gui/src/web_remote.html → README.md

## Import Cycles
- 1-file cycle: `ez-gui/src/audio_output.rs -> ez-gui/src/audio_output.rs`
- 2-file cycle: `ez-gui/src/app.rs -> ez-gui/src/satellite_panel.rs -> ez-gui/src/app.rs`
- 2-file cycle: `ez-gui/src/app.rs -> ez-gui/src/user_level.rs -> ez-gui/src/app.rs`
- 2-file cycle: `ez-gui/src/adsb_panel.rs -> ez-gui/src/app.rs -> ez-gui/src/adsb_panel.rs`
- 2-file cycle: `ez-gui/src/app.rs -> ez-gui/src/sdr_panel.rs -> ez-gui/src/app.rs`
- 2-file cycle: `ez-gui/src/ai_panel.rs -> ez-gui/src/app.rs -> ez-gui/src/ai_panel.rs`
- 2-file cycle: `ez-gui/src/app.rs -> ez-gui/src/recorder_panel.rs -> ez-gui/src/app.rs`
- 2-file cycle: `ez-gui/src/app.rs -> ez-gui/src/tutorial.rs -> ez-gui/src/app.rs`
- 3-file cycle: `ez-gui/src/app.rs -> ez-gui/src/tutorial.rs -> ez-gui/src/user_level.rs -> ez-gui/src/app.rs`
- 3-file cycle: `ez-gui/src/adsb_decoder.rs -> ez-gui/src/adsb_panel.rs -> ez-gui/src/app.rs -> ez-gui/src/adsb_decoder.rs`
- 3-file cycle: `ez-gui/src/adsb_panel.rs -> ez-gui/src/app.rs -> ez-gui/src/mqtt.rs -> ez-gui/src/adsb_panel.rs`

## Hyperedges (group relationships)
- **Spectrum Visualization Ecosystem** — readme_spectrum_analyser, readme_waterfall, readme_band_plan_overlay, anchored_summary_vfo_b_marker, anchored_summary_waterfall_colormap [EXTRACTED 0.95]
- **Demodulation and Signal Analysis Chain** — readme_demodulators, readme_adsb_decoder, night_work_frequency_based_mode, night_work_rf_filter_presets [INFERRED 0.85]
- **Web Remote Control Interface** — ez_gui_src_web_remote_html_frequency_control, ez_gui_src_web_remote_html_demodulation, ez_gui_src_web_remote_html_gain_recording, ez_gui_src_web_remote_html_satellite_passes [EXTRACTED 1.00]

## Communities (81 total, 30 thin omitted)

### Community 0 - "UI Application Core"
Cohesion: 0.13
Nodes (21): App, CreationContext, Demodulator, AppTab, CentralApp, parse_hhmm_today(), Arc, Context (+13 more)

### Community 1 - "Spectrum Visualization"
Cohesion: 0.07
Nodes (20): Complex32, category_color(), color_map(), ColorMap, lerp_color(), Arc, Color32, Option (+12 more)

### Community 2 - "ADS-B Decoder"
Cohesion: 0.05
Nodes (41): AircraftState, CprFrame, AdsBDecoder, AircraftState, CprFrame, decode_altitude(), decoder_new_starts_empty(), HashMap (+33 more)

### Community 3 - "Configuration & UI"
Cohesion: 0.09
Nodes (28): AppConfig, config_default_has_reasonable_freq(), config_default_has_theme_and_discord(), config_default_observer_at_london(), config_default_sample_rate(), config_default_theme_is_dark(), config_serde_roundtrip_preserves_fields(), ProviderPreset (+20 more)

### Community 4 - "Tutorial & Help"
Cohesion: 0.26
Nodes (5): HowToPanel, Self, String, Ui, Vec

### Community 5 - "Demodulation"
Cohesion: 0.12
Nodes (26): BufWriter, apply_filename_template_all_tokens(), apply_filename_template_basic(), apply_filename_template_empty_template(), apply_filename_template_no_tokens(), apply_filename_template_spaces_replaced(), free_disk_space_dot_returns_something(), free_disk_space_nonexistent_path_returns_fallback() (+18 more)

### Community 6 - "AI & Chat"
Cohesion: 0.15
Nodes (18): AiPanel, ChatMessage, Arc, AtomicBool, Color32, Instant, Mutex, Option (+10 more)

### Community 7 - "Sample Conversion"
Cohesion: 0.13
Nodes (16): find_device_index(), Drop, Option, Result, Self, Send, String, Vec (+8 more)

### Community 8 - "SDR Hardware Interface"
Cohesion: 0.11
Nodes (18): broadcast_state_with_listener_no_crash(), new_creates_disabled_instance(), no_crash_broadcast_without_listeners(), no_crash_poll_without_channel(), poll_commands_after_stop_is_empty(), RemoteCommand, Option, Receiver (+10 more)

### Community 9 - "Frequency Scanner"
Cohesion: 0.12
Nodes (23): AntennaChecklist, ChecklistItem, crit(), item(), Arc, Mutex, Option, Self (+15 more)

### Community 10 - "Community 10"
Cohesion: 0.07
Nodes (30): mqtt_is_connected_requires_enabled_and_client(), mqtt_new_defaults_disabled(), mqtt_new_no_reconnect(), mqtt_new_not_connected(), mqtt_publish_without_client_no_crash(), mqtt_set_enabled_disabled_noop(), MqttPublisher, Arc (+22 more)

### Community 11 - "AI Integration"
Cohesion: 0.12
Nodes (7): Display, Default, Result, Self, Stats, Duration, Formatter

### Community 12 - "Community 12"
Cohesion: 0.13
Nodes (18): rand_f64(), Arc, AtomicBool, c_void, Option, Receiver, Self, Sender (+10 more)

### Community 13 - "Configuration"
Cohesion: 0.09
Nodes (28): adaptive_gain_configure_gain_sets_limits(), adaptive_gain_gain_changed_updates_threshold(), adaptive_gain_new_defaults(), adaptive_gain_set_duty_cycle_clamps(), adaptive_gain_set_gain_clamps(), adaptive_gain_update_returns_none_when_disabled(), adaptive_gain_update_with_burst_detection(), AdaptiveGain (+20 more)

### Community 14 - "Hardware Interface"
Cohesion: 0.07
Nodes (43): Box, BTreeMap, BTreeSet, Error, AircraftData, categories(), DiscordEmbed, DiscordNotifier (+35 more)

### Community 15 - "AI Tools"
Cohesion: 0.12
Nodes (18): cpr_airborne_decode(), cpr_dlon_function(), cpr_mod(), cpr_mod_double(), cpr_mod_double_negative(), cpr_mod_double_positive(), cpr_n_function(), cpr_nl_function() (+10 more)

### Community 16 - "Community 16"
Cohesion: 0.16
Nodes (19): compute_magnitude(), compute_magnitude_sc16(), compute_magnitude_sc16q11(), compute_magnitude_uc8(), DemodStats, InputFormat, MagBuf, MagBufFlags (+11 more)

### Community 17 - "Community 17"
Cohesion: 0.08
Nodes (17): check_crc(), crc24(), test_crc24_generates_correct_parity(), test_crc24_parity_short_msg_panics(), AircraftMessage, decode_altitude(), decode_callsign(), decode_mode_s() (+9 more)

### Community 18 - "Demodulation Core"
Cohesion: 0.11
Nodes (21): c_char, last_err(), c_int, Default, Drop, Option, Result, Self (+13 more)

### Community 19 - "Community 19"
Cohesion: 0.11
Nodes (19): beast_frame_contains_signal_byte(), beast_frame_escapes_0x1a_in_payload_only(), beast_frame_indicator_defaults_to_long(), beast_frame_indicator_for_14byte_msg(), beast_frame_indicator_for_2byte_msg(), beast_frame_indicator_for_7byte_msg(), beast_frame_starts_with_unescaped_marker(), beast_frame_timestamp_is_6_bytes_be() (+11 more)

### Community 20 - "AI & Tools"
Cohesion: 0.13
Nodes (14): audio_output_mark_failed(), audio_output_new_defaults(), audio_output_start_fails_without_feature(), audio_output_stop_clears_state(), AudioOutput, Arc, Mutex, Option (+6 more)

### Community 21 - "Community 21"
Cohesion: 0.17
Nodes (8): Condvar, Fifo, Inner, Mutex, Option, Self, Vec, VecDeque

### Community 22 - "Community 22"
Cohesion: 0.24
Nodes (10): convert_sc16_to_mag(), convert_sc16_to_mag_known(), convert_sc16_to_mag_negative_iq(), convert_sc16q11_to_mag(), convert_sc16q11_to_mag_clamp(), convert_sc16q11_to_mag_known(), convert_uc8_to_mag(), convert_uc8_to_mag_max_input() (+2 more)

### Community 23 - "Community 23"
Cohesion: 0.24
Nodes (9): clear_removes_all(), contains_after_add(), contains_multiple_addresses(), hash_is_deterministic(), IcaoFilter, new_filter_is_empty(), Default, Self (+1 more)

### Community 24 - "Community 24"
Cohesion: 0.15
Nodes (12): Bookmark, BookmarkDb, default_bookmarks_have_categories(), default_has_all_bookmarks(), default_is_stable_without_file(), import_csv_adds_bookmarks(), import_csv_dedup_by_frequency(), Default (+4 more)

### Community 26 - "Web Remote"
Cohesion: 0.12
Nodes (21): Connection, Airport, AirportDb, AirportEntry, AirportFreq, antenna_dims(), AntennaDims, csv_split() (+13 more)

### Community 27 - "Community 27"
Cohesion: 0.19
Nodes (11): Args, compute_magbuf_stats(), Demodulator, main(), NetOutput, Default, FnMut, Option (+3 more)

### Community 29 - "AI Agents"
Cohesion: 0.25
Nodes (7): CI / Infrastructure, Dead Code / Lint Suppressions to Clean Up, Documentation, Infinity Loop — Remaining Work, Missing Test Coverage, Stub / TODO Implementations, Technical Debt / Cleanup

### Community 30 - "Community 30"
Cohesion: 0.16
Nodes (8): AsRef, IqFormat, IFileSdr, Option, Result, Self, String, Vec

### Community 31 - "Community 31"
Cohesion: 0.26
Nodes (8): Demod2400, generate_damage_set(), Default, FnMut, Self, test_generate_damage_set(), valid_df_long(), valid_df_short()

### Community 32 - "Community 32"
Cohesion: 0.22
Nodes (7): ModesMessage, ScoreRank, AircraftState, Default, HashMap, Self, Tracker

### Community 33 - "Community 33"
Cohesion: 0.48
Nodes (7): crc24_parity(), apply_bit_errors(), correct_message(), decode_mode_s_message(), Result, score_mode_s_message(), single_bit_syndrome()

### Community 34 - "Community 34"
Cohesion: 0.60
Nodes (5): check(), init(), _load(), _save(), status()

### Community 43 - "Hardware Control"
Cohesion: 0.20
Nodes (9): Build, Build & Run, Controls, Dependencies, EZ-SDR Unified, Features, Licence, Project Structure (+1 more)

### Community 45 - "Community 45"
Cohesion: 0.08
Nodes (22): Tab, advanced_steps(), beginner_steps(), clerk_maxwell_steps(), intermediate_steps(), render_tutorial(), Arc, Mutex (+14 more)

### Community 46 - "Community 46"
Cohesion: 0.07
Nodes (26): All Subsystems Verified, Architecture, Autonomous Daemon — Final Comprehensive Status Report, Beginner Experience (Verified), Build & Test, Code Metrics Summary, Code Quality, Daemon Protocol Status (+18 more)

### Community 47 - "Community 47"
Cohesion: 0.07
Nodes (36): demod_mode_roundtrip(), DemodMode, format_hz(), FreqIdInfo, identify_frequency(), identify_frequency_adsb(), identify_frequency_airband(), identify_frequency_amateur_2m() (+28 more)

### Community 48 - "Community 48"
Cohesion: 0.13
Nodes (11): FrequencyScanner, HitsSort, Arc, Instant, Mutex, Option, Self, String (+3 more)

### Community 49 - "Community 49"
Cohesion: 0.20
Nodes (12): demod_am_dc_signal_envelope(), demod_fm_constant_phase_near_zero(), demod_lsb_produces_finite_output(), demod_raw_output_length(), demod_usb_produces_finite_output(), demod_wfm_produces_output(), Demodulator, make_iq_dc() (+4 more)

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

### Community 76 - "Community 76"
Cohesion: 0.18
Nodes (18): active_job_finds_job_in_window(), active_job_prefers_first_match(), active_job_returns_none_outside_window(), active_job_returns_none_when_disabled(), CustomTask, make_scheduler(), new_scheduler_starts_empty(), poll_custom_tasks_marks_as_fired() (+10 more)

### Community 77 - "Community 77"
Cohesion: 0.19
Nodes (9): HackRf, HackRfCtx, Drop, Option, Receiver, Result, Send, Vec (+1 more)

### Community 78 - "Community 78"
Cohesion: 0.25
Nodes (5): Source, Send, SdrSource, Seek, SeekFrom

### Community 79 - "Community 79"
Cohesion: 0.53
Nodes (5): HackrfDevice, HackrfTransfer, c_int, c_void, rx_callback()

### Community 80 - "Community 80"
Cohesion: 0.50
Nodes (3): HackRfConfig, Default, Self

## Knowledge Gaps
- **114 isolated node(s):** `SoapySDRRange`, `ProviderPreset`, `Goal`, `Constraints & Preferences`, `Done` (+109 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **30 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `CentralApp` connect `UI Application Core` to `ADS-B Decoder`, `Tutorial & Help`, `Demodulation`, `AI & Chat`, `SDR Hardware Interface`, `Frequency Scanner`, `Community 10`, `Community 12`, `Community 45`, `Hardware Interface`, `Community 47`, `Community 48`, `AI & Tools`?**
  _High betweenness centrality (0.141) - this node is a cross-community bridge._
- **Why does `SharedState` connect `Frequency Scanner` to `UI Application Core`, `Spectrum Visualization`, `ADS-B Decoder`, `Configuration & UI`, `Demodulation`, `AI & Chat`, `Community 10`, `Community 76`, `Community 12`, `Hardware Interface`, `Community 47`, `Community 48`, `Community 45`, `Community 24`?**
  _High betweenness centrality (0.126) - this node is a cross-community bridge._
- **Why does `AdsBDecoder` connect `ADS-B Decoder` to `Community 16`, `UI Application Core`, `Community 31`?**
  _High betweenness centrality (0.105) - this node is a cross-community bridge._
- **What connects `SoapySDRRange`, `ProviderPreset`, `Goal` to the rest of the system?**
  _124 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `UI Application Core` be split into smaller, more focused modules?**
  _Cohesion score 0.13090418353576247 - nodes in this community are weakly interconnected._
- **Should `Spectrum Visualization` be split into smaller, more focused modules?**
  _Cohesion score 0.07171717171717172 - nodes in this community are weakly interconnected._
- **Should `ADS-B Decoder` be split into smaller, more focused modules?**
  _Cohesion score 0.05115089514066496 - nodes in this community are weakly interconnected._