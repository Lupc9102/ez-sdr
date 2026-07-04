# Graph Report - ez-sdr  (2026-07-04)

## Corpus Check
- 58 files · ~138,653 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 1716 nodes · 3422 edges · 91 communities (62 shown, 29 thin omitted)
- Extraction: 99% EXTRACTED · 1% INFERRED · 0% AMBIGUOUS · INFERRED: 28 edges (avg confidence: 0.82)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `6d28b21a`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- [[_COMMUNITY_UI Application Core|UI Application Core]]
- [[_COMMUNITY_Spectrum Visualization|Spectrum Visualization]]
- [[_COMMUNITY_Community 2|Community 2]]
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
- [[_COMMUNITY_Community 81|Community 81]]
- [[_COMMUNITY_Community 82|Community 82]]
- [[_COMMUNITY_Community 83|Community 83]]
- [[_COMMUNITY_Community 84|Community 84]]
- [[_COMMUNITY_Community 85|Community 85]]
- [[_COMMUNITY_Community 86|Community 86]]
- [[_COMMUNITY_Community 87|Community 87]]
- [[_COMMUNITY_Community 88|Community 88]]
- [[_COMMUNITY_Community 89|Community 89]]
- [[_COMMUNITY_Community 91|Community 91]]

## God Nodes (most connected - your core abstractions)
1. `CentralApp` - 46 edges
2. `SharedState` - 41 edges
3. `SpectrumAnalyzer` - 38 edges
4. `HowToPanel` - 37 edges
5. `AdsBPanel` - 34 edges
6. `make_shared_state()` - 34 edges
7. `AiPanel` - 31 edges
8. `FrequencyScanner` - 30 edges
9. `MqttPublisher` - 26 edges
10. `RecorderPanel` - 26 edges

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
- 2-file cycle: `ez-gui/src/app.rs -> ez-gui/src/sdr_panel.rs -> ez-gui/src/app.rs`
- 2-file cycle: `ez-gui/src/app.rs -> ez-gui/src/satellite_panel.rs -> ez-gui/src/app.rs`
- 2-file cycle: `ez-gui/src/app.rs -> ez-gui/src/user_level.rs -> ez-gui/src/app.rs`
- 2-file cycle: `ez-gui/src/adsb_panel.rs -> ez-gui/src/app.rs -> ez-gui/src/adsb_panel.rs`
- 2-file cycle: `ez-gui/src/ai_panel.rs -> ez-gui/src/app.rs -> ez-gui/src/ai_panel.rs`
- 2-file cycle: `ez-gui/src/app.rs -> ez-gui/src/recorder_panel.rs -> ez-gui/src/app.rs`
- 2-file cycle: `ez-gui/src/app.rs -> ez-gui/src/tutorial.rs -> ez-gui/src/app.rs`
- 3-file cycle: `ez-gui/src/app.rs -> ez-gui/src/tutorial.rs -> ez-gui/src/user_level.rs -> ez-gui/src/app.rs`
- 3-file cycle: `ez-gui/src/adsb_decoder.rs -> ez-gui/src/adsb_panel.rs -> ez-gui/src/app.rs -> ez-gui/src/adsb_decoder.rs`
- 3-file cycle: `ez-gui/src/adsb_panel.rs -> ez-gui/src/app.rs -> ez-gui/src/mqtt.rs -> ez-gui/src/adsb_panel.rs`
- 4-file cycle: `ez-gui/src/adsb_decoder.rs -> ez-gui/src/demod.rs -> ez-gui/src/sdr_panel.rs -> ez-gui/src/app.rs -> ez-gui/src/adsb_decoder.rs`

## Hyperedges (group relationships)
- **Spectrum Visualization Ecosystem** — readme_spectrum_analyser, readme_waterfall, readme_band_plan_overlay, anchored_summary_vfo_b_marker, anchored_summary_waterfall_colormap [EXTRACTED 0.95]
- **Demodulation and Signal Analysis Chain** — readme_demodulators, readme_adsb_decoder, night_work_frequency_based_mode, night_work_rf_filter_presets [INFERRED 0.85]
- **Web Remote Control Interface** — ez_gui_src_web_remote_html_frequency_control, ez_gui_src_web_remote_html_demodulation, ez_gui_src_web_remote_html_gain_recording, ez_gui_src_web_remote_html_satellite_passes [EXTRACTED 1.00]

## Communities (91 total, 29 thin omitted)

### Community 0 - "UI Application Core"
Cohesion: 0.09
Nodes (27): CreationContext, Demodulator, AppTab, CentralApp, parse_hhmm_today(), parse_hhmm_today_at(), Arc, Context (+19 more)

### Community 1 - "Spectrum Visualization"
Cohesion: 0.10
Nodes (10): Complex32, Arc, Option, Pos2, String, TextureHandle, Vec, VecDeque (+2 more)

### Community 2 - "Community 2"
Cohesion: 0.31
Nodes (6): Demod2400, Default, Self, test_valid_df_short_no_fix(), test_valid_df_short_with_fix(), valid_df_short()

### Community 3 - "Configuration & UI"
Cohesion: 0.12
Nodes (24): bg_luminance(), color_row(), mix_color(), Rgba, rgba_from_rgb_sets_alpha_255(), rgba_from_rgba_preserves_alpha(), rgba_to_egui_maps_correctly(), rgba_with_alpha_changes_only_alpha() (+16 more)

### Community 4 - "Tutorial & Help"
Cohesion: 0.14
Nodes (19): case_insensitive_keyword(), case_insensitive_section_title(), HowToPanel, keyword_contains_query(), keyword_with_multiple_sections(), matches_keyword(), matches_keyword_partial(), matches_section_title() (+11 more)

### Community 5 - "Demodulation"
Cohesion: 0.05
Nodes (70): BufWriter, AntennaChecklist, ChecklistItem, crit(), item(), Arc, Mutex, Option (+62 more)

### Community 6 - "AI & Chat"
Cohesion: 0.11
Nodes (30): AiPanel, ChatMessage, Arc, AtomicBool, Color32, Instant, Mutex, Option (+22 more)

### Community 7 - "Sample Conversion"
Cohesion: 0.09
Nodes (32): Display, add_accumulates_adaptive_gain_seconds(), add_accumulates_cpr_counters(), add_accumulates_demod_accepted(), add_accumulates_demod_preambles(), add_accumulates_messages_total_and_by_df(), add_accumulates_samples(), add_merges_start_ms() (+24 more)

### Community 8 - "SDR Hardware Interface"
Cohesion: 0.10
Nodes (23): broadcast_state_with_listener_no_crash(), new_creates_disabled_instance(), no_crash_broadcast_without_listeners(), no_crash_poll_without_channel(), poll_commands_after_stop_is_empty(), RemoteCommand, Drop, JoinHandle (+15 more)

### Community 9 - "Frequency Scanner"
Cohesion: 0.12
Nodes (25): embed_recording_error(), embed_recording_error_builds_correctly(), embed_recording_started(), embed_recording_started_all_combos(), embed_recording_stopped(), embed_recording_stopped_builds_correctly(), embed_sat_los(), embed_sat_los_builds_correctly() (+17 more)

### Community 10 - "Community 10"
Cohesion: 0.09
Nodes (38): compute_passes_contains_expected_sats(), compute_passes_empty_tles(), compute_passes_observer_at_equator(), compute_passes_observer_at_north_pole(), compute_passes_returns_sorted(), compute_passes_short_window_yields_fewer_passes(), doppler_shift_for_sat_iss(), doppler_shift_for_sat_large_freq() (+30 more)

### Community 11 - "AI Integration"
Cohesion: 0.11
Nodes (21): c_char, last_err(), c_int, Default, Drop, Option, Result, Self (+13 more)

### Community 12 - "Community 12"
Cohesion: 0.11
Nodes (24): rand_f64(), rand_f64_deterministic(), recv_samples_returns_none_when_idle(), Arc, AtomicBool, c_void, JoinHandle, Option (+16 more)

### Community 13 - "Configuration"
Cohesion: 0.09
Nodes (28): adaptive_gain_configure_gain_sets_limits(), adaptive_gain_gain_changed_updates_threshold(), adaptive_gain_new_defaults(), adaptive_gain_set_duty_cycle_clamps(), adaptive_gain_set_gain_clamps(), adaptive_gain_update_returns_none_when_disabled(), adaptive_gain_update_with_burst_detection(), AdaptiveGain (+20 more)

### Community 14 - "Hardware Interface"
Cohesion: 0.21
Nodes (9): BTreeMap, BTreeSet, DiscordNotifier, DiscordSettings, is_enabled(), is_starred(), Default, Instant (+1 more)

### Community 15 - "AI Tools"
Cohesion: 0.12
Nodes (18): cpr_airborne_decode(), cpr_dlon_function(), cpr_mod(), cpr_mod_double(), cpr_mod_double_negative(), cpr_mod_double_positive(), cpr_n_function(), cpr_nl_function() (+10 more)

### Community 16 - "Community 16"
Cohesion: 0.19
Nodes (3): peak_freq_hz_matches_injected_tone_offset(), Self, Ui

### Community 17 - "Community 17"
Cohesion: 0.08
Nodes (18): check_crc(), crc24(), crc24_parity(), test_crc24_generates_correct_parity(), test_crc24_parity_short_msg_panics(), AircraftMessage, decode_altitude(), decode_callsign() (+10 more)

### Community 18 - "Demodulation Core"
Cohesion: 0.06
Nodes (36): HackRf, HackRfConfig, HackRfCtx, HackrfDevice, HackrfTransfer, c_int, c_void, Default (+28 more)

### Community 19 - "Community 19"
Cohesion: 0.09
Nodes (28): beast_frame_all_0x1a_payload(), beast_frame_all_0x1a_payload_long(), beast_frame_contains_signal_byte(), beast_frame_empty_message(), beast_frame_escapes_0x1a_in_payload_only(), beast_frame_frame_length_matches_input(), beast_frame_indicator_defaults_to_long(), beast_frame_indicator_for_14byte_msg() (+20 more)

### Community 20 - "AI & Tools"
Cohesion: 0.14
Nodes (18): audio_output_mark_failed(), audio_output_mark_failed_after_stop(), audio_output_mark_failed_twice(), audio_output_new_defaults(), audio_output_start_fails_without_feature(), audio_output_stop_after_stop(), audio_output_stop_clears_state(), audio_output_stop_idempotent() (+10 more)

### Community 21 - "Community 21"
Cohesion: 0.15
Nodes (16): Condvar, capacity_exceeded_returns_item(), Fifo, fifo_ordering(), halt_wakes_waiters_and_clears_queue(), Inner, multiple_items_in_sequence(), new_creates_empty_fifo() (+8 more)

### Community 22 - "Community 22"
Cohesion: 0.38
Nodes (4): Self, Ui, test_render_toasts(), test_ui_methods_no_crash()

### Community 23 - "Community 23"
Cohesion: 0.22
Nodes (12): add_duplicate(), capacity_many_addresses(), clear_removes_all(), contains_after_add(), contains_multiple_addresses(), edge_case_max_icao(), hash_is_deterministic(), IcaoFilter (+4 more)

### Community 24 - "Community 24"
Cohesion: 0.11
Nodes (20): mqtt_disconnect_no_client(), mqtt_is_connected_requires_enabled_and_client(), mqtt_new_defaults_disabled(), mqtt_new_no_reconnect(), mqtt_new_not_connected(), mqtt_publish_noop_when_disabled(), mqtt_publish_without_client_no_crash(), mqtt_set_enabled_disabled_does_not_connect() (+12 more)

### Community 26 - "Web Remote"
Cohesion: 0.12
Nodes (21): Connection, Airport, AirportDb, AirportEntry, AirportFreq, antenna_dims(), AntennaDims, csv_split() (+13 more)

### Community 27 - "Community 27"
Cohesion: 0.20
Nodes (11): IqFormat, Args, compute_magbuf_stats(), Demodulator, main(), NetOutput, Default, Option (+3 more)

### Community 29 - "AI Agents"
Cohesion: 0.50
Nodes (3): Dependency Auditing, Large UI Panel Integration Tests, Remaining Work

### Community 30 - "Community 30"
Cohesion: 0.11
Nodes (25): AsRef, cleanup(), create_temp_file(), ifile_eof_returns_zero(), ifile_loop_rewinds(), ifile_new_sc16q11_bytes_per_sample(), ifile_new_with_path(), ifile_new_with_stdin() (+17 more)

### Community 31 - "Community 31"
Cohesion: 0.17
Nodes (12): DemodStats, MagBuf, MagBufFlags, receiveclock_ms_elapsed(), FnMut, Vec, slice_phase0(), slice_phase1() (+4 more)

### Community 32 - "Community 32"
Cohesion: 0.21
Nodes (13): ModesMessage, AircraftState, make_msg(), multiple_aircraft_tracked_separately(), new_creates_empty_tracker(), prune_older_than_removes_idle_entries(), Default, HashMap (+5 more)

### Community 33 - "Community 33"
Cohesion: 0.18
Nodes (12): apply_bit_errors(), correct_message(), decode_mode_s_message(), mode_s_message_len_by_type(), Result, score_mode_s_message(), ScoreRank, single_bit_syndrome() (+4 more)

### Community 34 - "Community 34"
Cohesion: 0.25
Nodes (7): CprFrame, AircraftState, Instant, Option, String, AircraftEntry, Default

### Community 35 - "Community 35"
Cohesion: 0.42
Nodes (7): altitude_100ft_to_squawk(), mode_a_to_mode_c(), mode_c_altitude_ground_level(), mode_c_altitude_lower_bound(), mode_c_known_good_example(), mode_c_round_trip(), Option

### Community 43 - "Hardware Control"
Cohesion: 0.15
Nodes (12): Build, Build & Run, Controls, Dependencies, Development, EZ-SDR Unified, Features, Install from source (+4 more)

### Community 45 - "Community 45"
Cohesion: 0.08
Nodes (22): Tab, advanced_steps(), beginner_steps(), clerk_maxwell_steps(), intermediate_steps(), render_tutorial(), Arc, Mutex (+14 more)

### Community 46 - "Community 46"
Cohesion: 0.17
Nodes (11): Autonomous Daemon — Final Comprehensive Status Report, Code Metrics Summary, Daemon Protocol Status, Do These In The Future, Enhancement Opportunities (Non-Critical), Executive Summary, Final Assessment, If Continuing Development (+3 more)

### Community 47 - "Community 47"
Cohesion: 0.06
Nodes (46): demod_mode_roundtrip(), DemodMode, format_hz(), FreqIdInfo, identify_frequency(), identify_frequency_adsb(), identify_frequency_airband(), identify_frequency_amateur_2m() (+38 more)

### Community 48 - "Community 48"
Cohesion: 0.08
Nodes (13): FrequencyScanner, HitsSort, parse_json_f32_valid(), Arc, Instant, Mutex, Option, Self (+5 more)

### Community 49 - "Community 49"
Cohesion: 0.16
Nodes (21): agc_attack_on_loud_signal(), agc_clamps_gain_and_output(), agc_decay_on_quiet_signal(), apply_lpf_bypass_when_alpha_near_one(), apply_lpf_filters_when_alpha_small(), demod_am_dc_signal_envelope(), demod_fm_constant_phase_near_zero(), demod_lsb_produces_finite_output() (+13 more)

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
Cohesion: 0.06
Nodes (27): compute_magnitude(), compute_magnitude_sc16(), compute_magnitude_sc16q11(), compute_magnitude_uc8(), generate_damage_set(), InputFormat, magnitude_sc16_sample(), magnitude_sc16q11_sample() (+19 more)

### Community 77 - "Community 77"
Cohesion: 0.13
Nodes (15): category_color(), category_color_alt_names(), category_color_amateur(), category_color_aviation(), category_color_broadcast(), category_color_is_case_insensitive(), category_color_marine(), category_color_scanner() (+7 more)

### Community 78 - "Community 78"
Cohesion: 0.11
Nodes (28): convert_sc16_to_mag(), convert_sc16_to_mag_all_zero(), convert_sc16_to_mag_both_negative(), convert_sc16_to_mag_empty_dst(), convert_sc16_to_mag_empty_src(), convert_sc16_to_mag_i16_min(), convert_sc16_to_mag_known(), convert_sc16_to_mag_max_iq() (+20 more)

### Community 79 - "Community 79"
Cohesion: 0.21
Nodes (13): AircraftData, DiscordEmbed, embed_aircraft(), embed_aircraft_fields_and_image(), embed_sat_aos(), embed_sat_aos_builds_correctly(), embed_sat_upcoming(), embed_sat_upcoming_builds_correctly() (+5 more)

### Community 80 - "Community 80"
Cohesion: 0.23
Nodes (8): Box, Error, embed_generic(), embed_generic_builds_correctly(), is_url_valid(), Result, Self, Value

### Community 81 - "Community 81"
Cohesion: 0.18
Nodes (9): window_blackman_flat_start(), window_blackman_generates_correct_length(), window_hamming_generates_correct_length(), window_hamming_sum_approximate_half_length(), window_hann_generates_correct_length(), window_hann_sum_approximate_half_length(), window_peaks_in_center(), window_tapers_to_zero_at_ends() (+1 more)

### Community 82 - "Community 82"
Cohesion: 0.20
Nodes (10): categories(), categories_contains_expected(), categories_no_duplicates(), categories_not_empty(), categories_sorted(), kinds_in(), kinds_in_known_category(), kinds_in_unknown_returns_empty() (+2 more)

### Community 83 - "Community 83"
Cohesion: 0.20
Nodes (12): DiscordPanel, Arc, Mutex, Self, String, Ui, test_new_defaults(), test_search_filter() (+4 more)

### Community 84 - "Community 84"
Cohesion: 0.22
Nodes (8): color_map(), color_map_all_variants_return_valid_rgb(), color_map_classic_boundaries(), ColorMap, lerp_color(), lerp_color_at_half_returns_midpoint(), sample_palette(), sample_palette_at_half()

### Community 85 - "Community 85"
Cohesion: 0.07
Nodes (40): App, Bookmark, BookmarkDb, default_bookmarks_have_categories(), default_has_all_bookmarks(), default_is_stable_without_file(), import_csv_adds_bookmarks(), import_csv_dedup_by_frequency() (+32 more)

### Community 86 - "Community 86"
Cohesion: 0.12
Nodes (15): AppConfig, config_default_has_reasonable_freq(), config_default_has_theme_and_discord(), config_default_observer_at_london(), config_default_sample_rate(), config_default_theme_is_dark(), config_load_or_default_nonexistent_returns_default(), config_save_and_load_roundtrip() (+7 more)

### Community 87 - "Community 87"
Cohesion: 0.16
Nodes (4): CprFrame, try_cpr_decode(), try_cpr_decode_identical_timestamps(), try_cpr_decode_matching_frames()

### Community 88 - "Community 88"
Cohesion: 0.13
Nodes (16): AdsBNotification, AdsBPanel, AircraftInfo, Arc, Context, HashMap, HashSet, Instant (+8 more)

### Community 89 - "Community 89"
Cohesion: 0.24
Nodes (7): AircraftState, AdsBDecoder, decode_altitude(), decoder_new_starts_empty(), HashMap, Self, Vec

### Community 91 - "Community 91"
Cohesion: 0.14
Nodes (12): AcCategory, classify_aircraft(), draw_plane_model(), Color32, Pos2, test_bearing(), test_bearing_known_values(), test_check_for_new_aircraft_adds_new() (+4 more)

## Knowledge Gaps
- **101 isolated node(s):** `SoapySDRRange`, `ProviderPreset`, `Goal`, `Constraints & Preferences`, `Done` (+96 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **29 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `AdsBDecoder` connect `Community 89` to `UI Application Core`, `Community 2`, `Community 31`, `Community 87`?**
  _High betweenness centrality (0.161) - this node is a cross-community bridge._
- **Why does `SharedState` connect `Demodulation` to `UI Application Core`, `Spectrum Visualization`, `AI & Chat`, `Community 10`, `Community 12`, `Community 45`, `Community 47`, `Community 48`, `Community 83`, `Community 85`, `Community 22`, `Community 86`, `Community 88`, `Community 91`?**
  _High betweenness centrality (0.155) - this node is a cross-community bridge._
- **Why does `CentralApp` connect `UI Application Core` to `Tutorial & Help`, `Demodulation`, `AI & Chat`, `SDR Hardware Interface`, `Community 12`, `Community 45`, `Hardware Interface`, `Community 47`, `Community 48`, `Community 83`, `AI & Tools`, `Community 85`, `Community 88`, `Community 89`, `Community 24`?**
  _High betweenness centrality (0.136) - this node is a cross-community bridge._
- **What connects `SoapySDRRange`, `ProviderPreset`, `Goal` to the rest of the system?**
  _111 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `UI Application Core` be split into smaller, more focused modules?**
  _Cohesion score 0.0946938775510204 - nodes in this community are weakly interconnected._
- **Should `Spectrum Visualization` be split into smaller, more focused modules?**
  _Cohesion score 0.09956709956709957 - nodes in this community are weakly interconnected._
- **Should `Configuration & UI` be split into smaller, more focused modules?**
  _Cohesion score 0.12435897435897436 - nodes in this community are weakly interconnected._