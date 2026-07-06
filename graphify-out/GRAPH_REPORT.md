# Graph Report - ez-sdr  (2026-07-06)

## Corpus Check
- 83 files · ~167,967 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 2238 nodes · 4488 edges · 113 communities (81 shown, 32 thin omitted)
- Extraction: 99% EXTRACTED · 1% INFERRED · 0% AMBIGUOUS · INFERRED: 45 edges (avg confidence: 0.81)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `e89421df`
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
- [[_COMMUNITY_Community 90|Community 90]]
- [[_COMMUNITY_Community 91|Community 91]]
- [[_COMMUNITY_Community 92|Community 92]]
- [[_COMMUNITY_Community 93|Community 93]]
- [[_COMMUNITY_Community 94|Community 94]]
- [[_COMMUNITY_Community 95|Community 95]]
- [[_COMMUNITY_Community 96|Community 96]]
- [[_COMMUNITY_Community 97|Community 97]]
- [[_COMMUNITY_Community 98|Community 98]]
- [[_COMMUNITY_Community 99|Community 99]]
- [[_COMMUNITY_Community 100|Community 100]]
- [[_COMMUNITY_Community 101|Community 101]]
- [[_COMMUNITY_Community 102|Community 102]]
- [[_COMMUNITY_Community 103|Community 103]]
- [[_COMMUNITY_Community 104|Community 104]]
- [[_COMMUNITY_Community 105|Community 105]]
- [[_COMMUNITY_Community 106|Community 106]]
- [[_COMMUNITY_Community 107|Community 107]]
- [[_COMMUNITY_Community 108|Community 108]]
- [[_COMMUNITY_Community 109|Community 109]]
- [[_COMMUNITY_Community 110|Community 110]]
- [[_COMMUNITY_Community 111|Community 111]]

## God Nodes (most connected - your core abstractions)
1. `CentralApp` - 49 edges
2. `make_shared_state()` - 43 edges
3. `SharedState` - 42 edges
4. `SpectrumAnalyzer` - 40 edges
5. `AdsBPanel` - 38 edges
6. `HowToPanel` - 37 edges
7. `FrequencyScanner` - 37 edges
8. `AiPanel` - 33 edges
9. `SatellitePanel` - 30 edges
10. `MqttPublisher` - 26 edges

## Surprising Connections (you probably didn't know these)
- `AdsBDecoder` --references--> `DemodStats`  [EXTRACTED]
  ez-gui/src/adsb_decoder.rs → dump1090/src/demod.rs
- `AdsBDecoder` --references--> `Demod2400`  [EXTRACTED]
  ez-gui/src/adsb_decoder.rs → dump1090/src/demod.rs
- `StreamParams` --references--> `Client`  [EXTRACTED]
  ez-gui/src/ai_panel.rs → dump1090/src/net_io.rs
- `MqttPublisher` --references--> `Client`  [EXTRACTED]
  ez-gui/src/mqtt.rs → dump1090/src/net_io.rs
- `DecodingPanel` --references--> `DecodeProgress`  [EXTRACTED]
  ez-gui/src/decoding_panel.rs → lrpt-decode/src/lib.rs

## Import Cycles
- 1-file cycle: `ez-gui/src/audio_output.rs -> ez-gui/src/audio_output.rs`
- 1-file cycle: `lrpt-decode/src/reed_solomon.rs -> lrpt-decode/src/reed_solomon.rs`
- 2-file cycle: `ez-gui/src/app.rs -> ez-gui/src/satellite_panel.rs -> ez-gui/src/app.rs`
- 2-file cycle: `ez-gui/src/app.rs -> ez-gui/src/sdr_panel.rs -> ez-gui/src/app.rs`
- 2-file cycle: `ez-gui/src/app.rs -> ez-gui/src/recorder_panel.rs -> ez-gui/src/app.rs`
- 2-file cycle: `ez-gui/src/adsb_panel.rs -> ez-gui/src/app.rs -> ez-gui/src/adsb_panel.rs`
- 2-file cycle: `ez-gui/src/ai_panel.rs -> ez-gui/src/app.rs -> ez-gui/src/ai_panel.rs`
- 3-file cycle: `ez-gui/src/adsb_panel.rs -> ez-gui/src/app.rs -> ez-gui/src/mqtt.rs -> ez-gui/src/adsb_panel.rs`
- 3-file cycle: `ez-gui/src/adsb_decoder.rs -> ez-gui/src/adsb_panel.rs -> ez-gui/src/app.rs -> ez-gui/src/adsb_decoder.rs`

## Hyperedges (group relationships)
- **Spectrum Visualization Ecosystem** — readme_spectrum_analyser, readme_waterfall, readme_band_plan_overlay, anchored_summary_vfo_b_marker, anchored_summary_waterfall_colormap [EXTRACTED 0.95]
- **Demodulation and Signal Analysis Chain** — readme_demodulators, readme_adsb_decoder, night_work_frequency_based_mode, night_work_rf_filter_presets [INFERRED 0.85]
- **Web Remote Control Interface** — ez_gui_src_web_remote_html_frequency_control, ez_gui_src_web_remote_html_demodulation, ez_gui_src_web_remote_html_gain_recording, ez_gui_src_web_remote_html_satellite_passes [EXTRACTED 1.00]

## Communities (113 total, 32 thin omitted)

### Community 0 - "UI Application Core"
Cohesion: 0.07
Nodes (35): CreationContext, Demodulator, AppTab, CentralApp, parse_hhmm_today(), parse_hhmm_today_at(), Arc, Context (+27 more)

### Community 1 - "Spectrum Visualization"
Cohesion: 0.09
Nodes (12): peak_freq_hz_matches_injected_tone_offset(), Arc, Complex32, Option, Pos2, Self, String, TextureHandle (+4 more)

### Community 2 - "Community 2"
Cohesion: 0.12
Nodes (19): AtomicU64, Cf32RecordingMeta, Cf32StreamWriter, converts_byte_pair_to_two_le_floats(), odd_trailing_byte_is_dropped(), output_length_is_4x_input_length(), raw_iq_bytes_to_cf32_le(), Arc (+11 more)

### Community 3 - "Configuration & UI"
Cohesion: 0.06
Nodes (55): install(), Context, gradient_rect_vertical(), gradient_rect_vertical_no_crash_empty_stops(), paint_glow(), paint_glow_disabled_is_noop(), Painter, Rect (+47 more)

### Community 4 - "Tutorial & Help"
Cohesion: 0.14
Nodes (19): case_insensitive_keyword(), case_insensitive_section_title(), HowToPanel, keyword_contains_query(), keyword_with_multiple_sections(), matches_keyword(), matches_keyword_partial(), matches_section_title() (+11 more)

### Community 5 - "Demodulation"
Cohesion: 0.18
Nodes (10): BufWriter, RecorderPanel, RecordingFile, Instant, Option, String, Vec, VecDeque (+2 more)

### Community 6 - "AI & Chat"
Cohesion: 0.09
Nodes (33): AiPanel, ChatMessage, Arc, AtomicBool, Color32, Instant, Mutex, Option (+25 more)

### Community 7 - "Sample Conversion"
Cohesion: 0.09
Nodes (33): Display, add_accumulates_adaptive_gain_seconds(), add_accumulates_cpr_counters(), add_accumulates_demod_accepted(), add_accumulates_demod_preambles(), add_accumulates_messages_total_and_by_df(), add_accumulates_samples(), add_merges_start_ms() (+25 more)

### Community 8 - "SDR Hardware Interface"
Cohesion: 0.08
Nodes (31): broadcast_state_with_listener_no_crash(), handle_socket(), index_handler(), new_creates_disabled_instance(), no_crash_broadcast_without_listeners(), no_crash_poll_without_channel(), poll_commands_after_stop_is_empty(), RemoteCommand (+23 more)

### Community 9 - "Frequency Scanner"
Cohesion: 0.10
Nodes (32): categories(), categories_contains_expected(), categories_no_duplicates(), categories_not_empty(), categories_sorted(), embed_recording_stopped(), embed_recording_stopped_builds_correctly(), embed_sat_los() (+24 more)

### Community 10 - "Community 10"
Cohesion: 0.08
Nodes (38): compute_passes_contains_expected_sats(), compute_passes_empty_tles(), compute_passes_observer_at_equator(), compute_passes_observer_at_north_pole(), compute_passes_returns_sorted(), compute_passes_short_window_yields_fewer_passes(), doppler_shift_for_sat_iss(), doppler_shift_for_sat_large_freq() (+30 more)

### Community 11 - "AI Integration"
Cohesion: 0.06
Nodes (35): HackRf, HackRfConfig, HackRfCtx, HackrfDevice, HackrfTransfer, c_int, c_void, Default (+27 more)

### Community 12 - "Community 12"
Cohesion: 0.11
Nodes (24): rand_f64(), rand_f64_deterministic(), recv_samples_returns_none_when_idle(), Arc, AtomicBool, c_void, JoinHandle, Option (+16 more)

### Community 13 - "Configuration"
Cohesion: 0.09
Nodes (28): adaptive_gain_configure_gain_sets_limits(), adaptive_gain_gain_changed_updates_threshold(), adaptive_gain_new_defaults(), adaptive_gain_set_duty_cycle_clamps(), adaptive_gain_set_gain_clamps(), adaptive_gain_update_returns_none_when_disabled(), adaptive_gain_update_with_burst_detection(), AdaptiveGain (+20 more)

### Community 14 - "Hardware Interface"
Cohesion: 0.18
Nodes (14): Box, BTreeMap, BTreeSet, Error, DiscordNotifier, DiscordSettings, is_starred(), QueuedNotification (+6 more)

### Community 15 - "AI Tools"
Cohesion: 0.12
Nodes (18): cpr_airborne_decode(), cpr_dlon_function(), cpr_mod(), cpr_mod_double(), cpr_mod_double_negative(), cpr_mod_double_positive(), cpr_n_function(), cpr_nl_function() (+10 more)

### Community 16 - "Community 16"
Cohesion: 0.21
Nodes (20): build_synthetic_iq_stream(), build_synthetic_vcdu_payload(), decode_file(), decode_file_reports_no_sync_on_pure_noise(), DecodeProgress, DecodeResult, full_pipeline_round_trip_recovers_original_payload_gray_mapping(), full_pipeline_round_trip_recovers_original_payload_natural_mapping() (+12 more)

### Community 17 - "Community 17"
Cohesion: 0.08
Nodes (18): check_crc(), crc24(), crc24_parity(), test_crc24_generates_correct_parity(), test_crc24_parity_short_msg_panics(), AircraftMessage, decode_altitude(), decode_callsign() (+10 more)

### Community 18 - "Demodulation Core"
Cohesion: 0.14
Nodes (25): active_job_finds_job_in_window(), active_job_prefers_first_match(), active_job_returns_none_outside_window(), active_job_returns_none_when_disabled(), CustomTask, make_scheduler(), new_scheduler_starts_empty(), poll_custom_tasks_marks_as_fired() (+17 more)

### Community 19 - "Community 19"
Cohesion: 0.09
Nodes (28): beast_frame_all_0x1a_payload(), beast_frame_all_0x1a_payload_long(), beast_frame_contains_signal_byte(), beast_frame_empty_message(), beast_frame_escapes_0x1a_in_payload_only(), beast_frame_frame_length_matches_input(), beast_frame_indicator_defaults_to_long(), beast_frame_indicator_for_14byte_msg() (+20 more)

### Community 20 - "AI & Tools"
Cohesion: 0.14
Nodes (21): audio_output_mark_failed(), audio_output_mark_failed_after_stop(), audio_output_mark_failed_twice(), audio_output_new_defaults(), audio_output_start_fails_without_feature(), audio_output_stop_after_stop(), audio_output_stop_clears_state(), audio_output_stop_idempotent() (+13 more)

### Community 21 - "Community 21"
Cohesion: 0.15
Nodes (16): Condvar, capacity_exceeded_returns_item(), Fifo, fifo_ordering(), halt_wakes_waiters_and_clears_queue(), Inner, multiple_items_in_sequence(), new_creates_empty_fifo() (+8 more)

### Community 22 - "Community 22"
Cohesion: 0.12
Nodes (14): DecodePreset, DecodeRequest, DecodingPanel, request_from_satellite_sets_no_decoder_reason_when_preset_none(), JoinHandle, Option, Receiver, Result (+6 more)

### Community 23 - "Community 23"
Cohesion: 0.22
Nodes (12): add_duplicate(), capacity_many_addresses(), clear_removes_all(), contains_after_add(), contains_multiple_addresses(), edge_case_max_icao(), hash_is_deterministic(), IcaoFilter (+4 more)

### Community 24 - "Community 24"
Cohesion: 0.11
Nodes (20): mqtt_disconnect_no_client(), mqtt_is_connected_requires_enabled_and_client(), mqtt_new_defaults_disabled(), mqtt_new_no_reconnect(), mqtt_new_not_connected(), mqtt_publish_noop_when_disabled(), mqtt_publish_without_client_no_crash(), mqtt_set_enabled_disabled_does_not_connect() (+12 more)

### Community 26 - "Web Remote"
Cohesion: 0.08
Nodes (23): Connection, Airport, AirportDb, AirportEntry, AirportFreq, antenna_dims(), AntennaDims, csv_split() (+15 more)

### Community 27 - "Community 27"
Cohesion: 0.16
Nodes (14): IqFormat, DemodStats, ModesMessage, Args, compute_magbuf_stats(), Demodulator, main(), NetOutput (+6 more)

### Community 29 - "AI Agents"
Cohesion: 0.40
Nodes (4): Additional Achievements, Completed Work, Dependency Auditing ✅, Large UI Panel Integration Tests ✅

### Community 30 - "Community 30"
Cohesion: 0.12
Nodes (26): AsRef, cleanup(), create_temp_file(), ifile_eof_returns_zero(), ifile_loop_rewinds(), ifile_new_sc16q11_bytes_per_sample(), ifile_new_with_path(), ifile_new_with_stdin() (+18 more)

### Community 31 - "Community 31"
Cohesion: 0.17
Nodes (13): Demod2400, MagBuf, MagBufFlags, receiveclock_ms_elapsed(), Default, FnMut, Self, Vec (+5 more)

### Community 32 - "Community 32"
Cohesion: 0.22
Nodes (12): AircraftState, make_msg(), multiple_aircraft_tracked_separately(), new_creates_empty_tracker(), prune_older_than_removes_idle_entries(), Default, HashMap, Self (+4 more)

### Community 33 - "Community 33"
Cohesion: 0.18
Nodes (12): apply_bit_errors(), correct_message(), decode_mode_s_message(), mode_s_message_len_by_type(), Result, score_mode_s_message(), ScoreRank, single_bit_syndrome() (+4 more)

### Community 34 - "Community 34"
Cohesion: 0.16
Nodes (14): lat_lon_to_screen(), MapRenderer, Color32, Option, Painter, Pos2, Rect, Self (+6 more)

### Community 35 - "Community 35"
Cohesion: 0.42
Nodes (7): altitude_100ft_to_squawk(), mode_a_to_mode_c(), mode_c_altitude_ground_level(), mode_c_altitude_lower_bound(), mode_c_known_good_example(), mode_c_round_trip(), Option

### Community 43 - "Hardware Control"
Cohesion: 0.15
Nodes (12): Build, Build & Run, Controls, Dependencies, Development, EZ-SDR Unified, Features, Install from source (+4 more)

### Community 45 - "Community 45"
Cohesion: 0.11
Nodes (3): levels_returns_all_four(), Self, UserLevel

### Community 46 - "Community 46"
Cohesion: 0.17
Nodes (11): Autonomous Daemon — Final Comprehensive Status Report, Code Metrics Summary, Daemon Protocol Status, Do These In The Future, Enhancement Opportunities (Non-Critical), Executive Summary, Final Assessment, If Continuing Development (+3 more)

### Community 47 - "Community 47"
Cohesion: 0.06
Nodes (45): demod_mode_roundtrip(), format_hz(), FreqIdInfo, identify_frequency(), identify_frequency_adsb(), identify_frequency_airband(), identify_frequency_amateur_2m(), identify_frequency_aviation_info() (+37 more)

### Community 48 - "Community 48"
Cohesion: 0.07
Nodes (15): CalibrationMeasurement, CalibrationPhase, FrequencyScanner, HitsSort, parse_json_f32_valid(), Arc, Instant, Mutex (+7 more)

### Community 49 - "Community 49"
Cohesion: 0.14
Nodes (26): agc_attack_on_loud_signal(), agc_clamps_gain_and_output(), agc_decay_on_quiet_signal(), apply_lpf_bypass_when_alpha_near_one(), apply_lpf_filters_when_alpha_small(), demod_am_dc_signal_envelope(), demod_fm_constant_phase_near_zero(), demod_lsb_produces_finite_output() (+18 more)

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

### Community 76 - "Community 76"
Cohesion: 0.06
Nodes (30): compute_magnitude(), compute_magnitude_sc16(), compute_magnitude_sc16q11(), compute_magnitude_uc8(), generate_damage_set(), InputFormat, magnitude_sc16_sample(), magnitude_sc16q11_sample() (+22 more)

### Community 77 - "Community 77"
Cohesion: 0.12
Nodes (16): category_color(), category_color_alt_names(), category_color_amateur(), category_color_aviation(), category_color_broadcast(), category_color_is_case_insensitive(), category_color_marine(), category_color_scanner() (+8 more)

### Community 78 - "Community 78"
Cohesion: 0.11
Nodes (28): convert_sc16_to_mag(), convert_sc16_to_mag_all_zero(), convert_sc16_to_mag_both_negative(), convert_sc16_to_mag_empty_dst(), convert_sc16_to_mag_empty_src(), convert_sc16_to_mag_i16_min(), convert_sc16_to_mag_known(), convert_sc16_to_mag_max_iq() (+20 more)

### Community 79 - "Community 79"
Cohesion: 0.38
Nodes (7): AircraftData, embed_aircraft(), embed_aircraft_fields_and_image(), fetch_aircraft_image(), is_url_valid(), Option, String

### Community 80 - "Community 80"
Cohesion: 0.17
Nodes (21): ApidBuffer, build_space_packet(), mpdu_no_packet_start_sentinel(), MpduHeader, PacketReassembler, parse_mpdu_header(), parse_space_packet_header(), parse_vcdu_header() (+13 more)

### Community 81 - "Community 81"
Cohesion: 0.18
Nodes (9): window_blackman_flat_start(), window_blackman_generates_correct_length(), window_hamming_generates_correct_length(), window_hamming_sum_approximate_half_length(), window_hann_generates_correct_length(), window_hann_sum_approximate_half_length(), window_peaks_in_center(), window_tapers_to_zero_at_ends() (+1 more)

### Community 82 - "Community 82"
Cohesion: 0.15
Nodes (13): DiscordEmbed, embed_generic(), embed_generic_builds_correctly(), embed_recording_error(), embed_recording_error_builds_correctly(), embed_recording_started(), embed_recording_started_all_combos(), embed_sat_aos() (+5 more)

### Community 83 - "Community 83"
Cohesion: 0.20
Nodes (12): DiscordPanel, Arc, Mutex, Self, String, Ui, test_new_defaults(), test_search_filter() (+4 more)

### Community 84 - "Community 84"
Cohesion: 0.22
Nodes (8): color_map(), color_map_all_variants_return_valid_rgb(), color_map_classic_boundaries(), ColorMap, lerp_color(), lerp_color_at_half_returns_midpoint(), sample_palette(), sample_palette_at_half()

### Community 85 - "Community 85"
Cohesion: 0.10
Nodes (18): App, Bookmark, BookmarkDb, default_bookmarks_have_categories(), default_has_all_bookmarks(), default_is_stable_without_file(), import_csv_adds_bookmarks(), import_csv_dedup_by_frequency() (+10 more)

### Community 86 - "Community 86"
Cohesion: 0.08
Nodes (29): AppConfig, config_custom_themes_roundtrip_through_json(), config_default_has_empty_custom_theme_gallery(), config_default_has_reasonable_freq(), config_default_has_theme_and_discord(), config_default_observer_at_london(), config_default_sample_rate(), config_default_theme_is_dark() (+21 more)

### Community 87 - "Community 87"
Cohesion: 0.12
Nodes (9): CprFrame, AircraftState, CprFrame, Instant, Option, String, try_cpr_decode(), try_cpr_decode_identical_timestamps() (+1 more)

### Community 88 - "Community 88"
Cohesion: 0.14
Nodes (12): AtomicUsize, AdsBPanel, Context, HashMap, HashSet, PathBuf, Receiver, Rect (+4 more)

### Community 89 - "Community 89"
Cohesion: 0.19
Nodes (15): bits_to_bytes_msb(), bits_to_u32(), find_sync(), finds_exact_marker_at_known_offset(), finds_marker_with_a_few_bit_errors(), frame_sync_extracts_full_cadu_from_stream(), frame_sync_handles_incremental_feeding(), FrameSync (+7 more)

### Community 90 - "Community 90"
Cohesion: 0.12
Nodes (25): build_satellite_catalog(), current_unix_time(), pending_decode_request_carries_full_path_not_just_filename(), Arc, Duration, Instant, Mutex, Option (+17 more)

### Community 91 - "Community 91"
Cohesion: 0.41
Nodes (12): corrects_injected_byte_errors_within_capacity(), decode_interleaved(), deinterleave(), encode_decode_round_trip_no_errors(), encode_interleaved(), interleave_data(), interleave_deinterleave_round_trip(), other_codewords_unaffected_by_errors_in_one() (+4 more)

### Community 92 - "Community 92"
Cohesion: 0.33
Nodes (9): derandomize(), derandomize_in_place(), derandomize_in_place_matches_non_mutating(), derandomize_is_involution(), derandomize_wraps_at_255_bytes(), pn_table(), pn_table_has_correct_length_and_is_deterministic(), pn_table_starts_with_known_seed_derived_byte() (+1 more)

### Community 93 - "Community 93"
Cohesion: 0.11
Nodes (21): c_char, last_err(), c_int, Default, Drop, Option, Result, Self (+13 more)

### Community 94 - "Community 94"
Cohesion: 0.22
Nodes (16): Dibit, bits_bytes_round_trip(), bits_to_bytes(), bytes_to_bits(), dibit_gray_mapping_is_distinct_permutation(), dibit_natural_mapping_round_trips_all_values(), dibit_to_bits(), DibitMapping (+8 more)

### Community 95 - "Community 95"
Cohesion: 0.06
Nodes (30): 2a. Composite presets, 2b. Expression parser, 4a. State, 4b. UI Layout, 4c. Image viewer interaction, 4d. Histogram, 4e. Channel thumbnails, 5a. Add EditorPanel to CentralApp (+22 more)

### Community 96 - "Community 96"
Cohesion: 0.16
Nodes (20): Agc, agc_normalizes_toward_target(), costas_phase_error(), costas_phase_error_has_restoring_force_around_lock_point(), costas_phase_error_treats_all_four_constellation_points_as_equal_lock_points(), demod_produces_symbols_from_upsampled_qpsk_stream(), FirFilter, lowpass_taps() (+12 more)

### Community 97 - "Community 97"
Cohesion: 0.07
Nodes (27): 1a. Add image_url field to ChatMessage, 1b. Update `build_api_messages()` for OpenAI-compatible providers, 1c. Update `build_api_messages()` for Anthropic, 3a. Recommendation struct, 3b. Vision prompt, 4a. Add state fields, 4b. Add method to request AI recommendation, 4c. Add polling in tick method (+19 more)

### Community 98 - "Community 98"
Cohesion: 0.25
Nodes (10): alignment_info_ui(), azimuth_to_cardinal(), compass_rose_ui(), compute_dipole_alignment(), dipole_tutorial_ui(), DipoleAlignment, DipoleType, Option (+2 more)

### Community 99 - "Community 99"
Cohesion: 0.19
Nodes (19): AntennaChecklist, ChecklistItem, crit(), item(), Arc, Mutex, Option, Self (+11 more)

### Community 100 - "Community 100"
Cohesion: 0.19
Nodes (14): apid_channel_label(), ChannelImage, ImageBuilder, line_count_tracks_pushed_scanlines(), missing_apid_returns_none(), multiple_scanlines_stack_top_to_bottom(), GrayImage, HashMap (+6 more)

### Community 101 - "Community 101"
Cohesion: 0.17
Nodes (25): apply_filename_template_adjacent_tokens(), apply_filename_template_all_tokens(), apply_filename_template_basic(), apply_filename_template_case_sensitive_no_match(), apply_filename_template_empty_template(), apply_filename_template_no_tokens(), apply_filename_template_only_date(), apply_filename_template_only_freq() (+17 more)

### Community 102 - "Community 102"
Cohesion: 0.33
Nodes (6): Arc, Mutex, Self, Ui, test_render_toasts(), test_ui_methods_no_crash()

### Community 103 - "Community 103"
Cohesion: 0.15
Nodes (12): 3a. Add calibration phase enum, 3b. Add calibration fields to `FrequencyScanner` struct, 3c. Initialize in `new()`, 3d. Add calibration methods, 3e. Add calibration UI, Context, Files to modify, SDR Frequency Scanner Refactor & Calibration Upgrade (+4 more)

### Community 104 - "Community 104"
Cohesion: 0.24
Nodes (7): AircraftState, AdsBDecoder, decode_altitude(), decoder_new_starts_empty(), HashMap, Self, Vec

### Community 105 - "Community 105"
Cohesion: 0.14
Nodes (12): AcCategory, classify_aircraft(), draw_plane_model(), Color32, Painter, Pos2, test_bearing(), test_bearing_known_values() (+4 more)

### Community 106 - "Community 106"
Cohesion: 0.08
Nodes (23): 1.1 — Unified throttled zoom accumulator (replaces the instant-step scroll logic), 1.2 — Bounded in-memory tile cache (LRU-style eviction), 1.3 — Bounded concurrent tile downloads, 1.4 — Disk-backed tile cache, 1.5 — Prefetch adjacent zoom levels, 2.1 — Delete `ez-gui/src/tutorial.rs` entirely, 2.2 — `ez-gui/src/main.rs`, 2.3 — `ez-gui/src/user_level.rs` (208 lines total — KEEP the `UserLevel` enum, it's used independently for the "Knowledge Level" setting) (+15 more)

### Community 108 - "Community 108"
Cohesion: 0.29
Nodes (7): AdsBNotification, AircraftEntry, AircraftInfo, Default, Instant, Option, String

### Community 109 - "Community 109"
Cohesion: 0.32
Nodes (11): cf32_le_bytes_to_complex(), cf32_le_known_value_round_trip(), cf32_le_trailing_partial_pair_ignored(), cf32_le_zero_bytes_map_to_zero(), iq_bytes_to_complex(), mid_value_maps_near_zero(), multiple_pairs_convert_in_order(), odd_trailing_byte_is_ignored() (+3 more)

### Community 110 - "Community 110"
Cohesion: 0.29
Nodes (7): Option, String, Ui, satellite_picker_ui(), String, SatelliteCatalogEntry, FnOnce

## Knowledge Gaps
- **172 isolated node(s):** `@opencode-ai/plugin`, `SoapySDRRange`, `ProviderPreset`, `Scope`, `Prerequisites` (+167 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **32 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `SharedState` connect `Community 99` to `UI Application Core`, `Spectrum Visualization`, `Community 101`, `Community 102`, `AI & Chat`, `Demodulation`, `Community 105`, `Community 10`, `Community 12`, `Community 47`, `Community 48`, `Community 49`, `Demodulation Core`, `Community 83`, `Community 85`, `Community 86`, `Community 88`, `Community 90`?**
  _High betweenness centrality (0.173) - this node is a cross-community bridge._
- **Why does `CentralApp` connect `UI Application Core` to `Tutorial & Help`, `Demodulation`, `AI & Chat`, `SDR Hardware Interface`, `Community 12`, `Hardware Interface`, `AI & Tools`, `Community 22`, `Community 24`, `Community 47`, `Community 48`, `Community 49`, `Community 83`, `Community 85`, `Community 86`, `Community 88`, `Community 90`, `Community 99`, `Community 104`?**
  _High betweenness centrality (0.163) - this node is a cross-community bridge._
- **Why does `AdsBDecoder` connect `Community 104` to `UI Application Core`, `Community 27`, `Community 31`, `Community 87`?**
  _High betweenness centrality (0.130) - this node is a cross-community bridge._
- **Are the 25 inferred relationships involving `make_shared_state()` (e.g. with `test_bearing()` and `test_bearing_known_values()`) actually correct?**
  _`make_shared_state()` has 25 INFERRED edges - model-reasoned connections that need verification._
- **What connects `@opencode-ai/plugin`, `SoapySDRRange`, `ProviderPreset` to the rest of the system?**
  _182 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `UI Application Core` be split into smaller, more focused modules?**
  _Cohesion score 0.07191961924907457 - nodes in this community are weakly interconnected._
- **Should `Spectrum Visualization` be split into smaller, more focused modules?**
  _Cohesion score 0.08571428571428572 - nodes in this community are weakly interconnected._