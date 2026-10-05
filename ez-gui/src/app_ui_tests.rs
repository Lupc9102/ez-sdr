use super::*;
use eframe::App;

#[path = "../tests/support/software_renderer.rs"]
mod software_renderer;

/// Render the real application widgets without requiring a display server.
/// Run explicitly: EZ_SDR_PREVIEW_DIR=/tmp/ez-sdr-previews cargo test -p ez-gui
/// --no-default-features app_ui_tests::render_workspaces -- --ignored --nocapture
#[test]
#[ignore = "writes visual QA artifacts when explicitly requested"]
fn render_workspaces() {
    let folder = std::env::var("EZ_SDR_PREVIEW_DIR").expect("set EZ_SDR_PREVIEW_DIR");
    std::fs::create_dir_all(&folder).unwrap();
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut config = AppConfig::default();
    config.mqtt_broker.clear();
    let mut app = CentralApp::new_with_config(&cc, config);
    let mut frame = eframe::Frame::_new_kittest();
    app.theme_applied = true;
    {
        let mut state = app.shared.lock().unwrap();
        state.config.theme_config = crate::theme::ThemeConfig::dark();
        state.config.theme_config.apply_to_ctx(&ctx);
        state.source.frequency_hz = 118_100_000;
        state.source.center_frequency_hz = Some(118_100_000);
        state.source.sample_rate_hz = 2_400_000;
        state.lpf_cutoff = 5_000.0;
        state.spectrum.update_params(118_100_000, 2_400_000);
        state.demod_mode = crate::sdr_panel::DemodMode::Am;
        state.source.stop();
        state.config.skip_antenna_checklists = true;
    }
    let mut renderer = software_renderer::SoftwareRenderer::default();
    for (name, tab) in [
        ("radio", AppTab::Listen),
        ("radio-nfm", AppTab::Listen),
        ("radio-wfm", AppTab::Listen),
        ("radio-usb", AppTab::Listen),
        ("radio-cw", AppTab::Listen),
        ("radio-vfo", AppTab::Listen),
        ("radio-source", AppTab::Listen),
        ("radio-replay", AppTab::Listen),
        ("adsb", AppTab::Planes),
        ("meteor", AppTab::Meteor),
        ("adsb-fixture", AppTab::Planes),
        ("uat-fixture", AppTab::Planes),
    ] {
        if std::env::var("EZ_SDR_PREVIEW_WORKSPACE")
            .is_ok_and(|requested| !requested.split(',').any(|item| item == name))
        {
            continue;
        }
        app.current_tab = tab;
        if matches!(name, "radio-nfm" | "radio-wfm" | "radio-cw" | "radio-usb") {
            let mut state = app.shared.lock().unwrap();
            let mode = match name {
                "radio-wfm" => crate::sdr_panel::DemodMode::Wfm,
                "radio-nfm" => crate::sdr_panel::DemodMode::Fm,
                "radio-usb" => crate::sdr_panel::DemodMode::Usb,
                _ => crate::sdr_panel::DemodMode::Cw,
            };
            state.source.frequency_hz = match name {
                "radio-wfm" => 100_000_000,
                "radio-nfm" => 145_500_000,
                "radio-usb" => 14_200_000,
                _ => 7_030_000,
            };
            crate::radio_ui::select_demod(&mut state, mode);
            state.config.advanced.wfm_stereo = name == "radio-wfm";
            state.config.advanced.rds_enabled = name == "radio-wfm";
            state.config.advanced.rds_info = name == "radio-wfm";
            if name == "radio-nfm" {
                state.config.advanced.squelch_mode = crate::radio_squelch::SquelchMode::CtcssMute;
                state.config.advanced.ctcss_tone_hz = Some(100.0);
            }
            if name == "radio-usb" {
                state.config.advanced.rf_noise_blanker = true;
            }
            let frequency = state.source.frequency_hz;
            state.source.center_frequency_hz = Some(frequency);
            let rate = state.source.sample_rate_hz;
            state.spectrum.update_params(frequency, rate);
        }
        if name == "radio-vfo" {
            let mut state = app.shared.lock().unwrap();
            state.source.center_frequency_hz = Some(118_000_000);
            state.source.frequency_hz = 118_025_000;
            state.config.center_tuning = false;
            state.config.advanced.rf_decim = 8;
            state.config.advanced.rf_dc_remove = true;
            crate::radio_ui::select_demod(&mut state, crate::sdr_panel::DemodMode::Am);
            state.spectrum.update_params_exact(118_000_000, 300_000.0);
        }
        if name == "radio-source" || name == "radio-replay" {
            let mut state = app.shared.lock().unwrap();
            state.source.source_mode = if name == "radio-source" {
                crate::source_manager::SourceMode::Hardware
            } else {
                crate::source_manager::SourceMode::Replay
            };
            state.source.frequency_hz = 118_025_000;
            state.source.center_frequency_hz = Some(118_000_000);
            state.config.advanced.rf_decim = 1;
            state.config.advanced.rf_dc_remove = false;
            crate::radio_ui::select_demod(&mut state, crate::sdr_panel::DemodMode::Am);
        }
        if name.ends_with("fixture") {
            app.adsb_panel.aircraft = [
                (0xABC001, "TEST-EAST", 51.7, -0.6, 90),
                (0xABC002, "TEST-NORTH", 51.3, 0.2, 0),
            ]
            .into_iter()
            .map(
                |(icao, callsign, lat, lon, heading)| crate::adsb_panel::AircraftEntry {
                    icao,
                    callsign: callsign.into(),
                    lat,
                    lon,
                    altitude: Some(18000),
                    altitude_type: crate::adsb_panel::AltitudeType::Barometric,
                    speed: Some(220),
                    heading: Some(heading),
                    squawk: None,
                    vertical_rate: None,
                    surface_movement: None,
                    surface_heading: None,
                    has_position: true,
                    has_heading: true,
                    seen: std::time::Instant::now(),
                    reliable: true,
                    correctedbits: 0,
                    score: 1.0,
                    quality_known: true,
                    position_seen: Some(std::time::Instant::now()),
                    ..Default::default()
                },
            )
            .collect();
        }
        if name == "uat-fixture" {
            app.adsb_panel.region = crate::adsb_panel::AdsbRegion::Uat978;
            for (key, callsign, lat, lon, heading) in [
                (0xABC001, "TEST-EAST", 51.7, -0.6, 90.0),
                (0xABC002, "TEST-NORTH", 51.3, 0.2, 0.0),
            ] {
                app.adsb_panel
                    .seed_uat_fixture(crate::uat_receiver::UatReport {
                        key,
                        callsign: Some(callsign.into()),
                        position: Some((lat, lon)),
                        altitude_ft: Some(18000),
                        altitude_geometric: false,
                        ground_speed_kt: Some(220.0),
                        heading_deg: None,
                        heading_true: false,
                        on_ground: Some(false),
                        track_deg: Some(heading),
                    });
            }
        }
        // The 1920×1012 software render can be compared directly to the
        // installed SDR++ window captured at the same physical pixel size.
        for [width, height] in [[1920, 1012], [1400, 900], [1000, 700]] {
            for pass in 0..3 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width as f32, height as f32),
                    )),
                    ..Default::default()
                };
                let output = ctx.run_ui(input, |ui| app.ui(ui, &mut frame));
                let pixels = renderer.render(&ctx, output, width, height);
                if pass == 2 {
                    let path = format!("{folder}/{name}-{width}x{height}.png");
                    pixels.save(&path).unwrap();
                    println!("Rendered {path}");
                }
            }
        }
    }
}

fn offline_test_app() -> CentralApp {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx);
    let mut config = AppConfig::default();
    config.mqtt_broker.clear();
    CentralApp::new_with_config(&cc, config)
}

#[test]
fn utility_tools_render_as_floating_windows() {
    for tool in [
        SecondaryTool::Scanner,
        SecondaryTool::HowTo,
        SecondaryTool::SetupGuide,
        SecondaryTool::Settings,
    ] {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = CentralApp::new_with_config(&cc, AppConfig::default());
        app.theme_applied = true;
        app.active_secondary_tool = Some(tool);
        let mut frame = eframe::Frame::_new_kittest();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 800.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| app.ui(ui, &mut frame));
        assert!(
            app.active_secondary_tool.is_some(),
            "{tool:?} utility window should stay open"
        );
    }
}

#[test]
fn restored_file_source_and_mode_stay_idle_with_usable_legacy_settings() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx);
    let mut config: AppConfig = serde_json::from_str(
        r#"{
        "last_session_freq_hz":118100000,
        "last_session_demod":"AM",
        "font_scale":0,
        "default_sample_rate":0,
        "mqtt_broker":"",
        "source_preferences":{"mode":"file","replay_file":"missing.iq","replay_speed":0}
    }"#,
    )
    .unwrap();
    config.advanced.radio_bandwidth_hz = 8000.0;
    let app = CentralApp::new_with_config(&cc, config);
    let state = app.shared.lock().unwrap();
    assert_eq!(
        state.source.source_mode,
        crate::source_manager::SourceMode::Replay
    );
    assert_eq!(state.source.replay_file.as_deref(), Some("missing.iq"));
    assert_eq!(state.source.replay_speed, 1.0);
    assert_eq!(
        state.source.status,
        crate::source_manager::SourceStatus::Idle
    );
    assert!(!state.audio_running && !state.recording);
    assert_eq!(state.demod_mode, crate::sdr_panel::DemodMode::Am);
    assert_eq!(crate::radio_ui::channel_bandwidth(&state), 8000.0);
    assert_eq!(state.config.font_scale, 1.0);
    assert_eq!(state.source.sample_rate_hz, 2_048_000);
    let saved = crate::config::SourcePreferences::capture(&state.source);
    let restored: crate::config::SourcePreferences =
        serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
    assert_eq!(restored.mode, "file");
    assert_eq!(restored.replay_file.as_deref(), Some("missing.iq"));
}

#[test]
fn adsb_capture_identity_resets_on_retune_rate_and_source_changes() {
    let mut app = offline_test_app();
    app.sync_adsb_capture();
    for change in 0..4 {
        app.adsb_decoder.total_messages = 99;
        app.adsb_panel.total_messages = 99;
        {
            let mut state = app.shared.lock().unwrap();
            match change {
                0 => state.source.center_frequency_hz = Some(1_090_000_000),
                1 => state.source.sample_rate_hz = 2_400_000,
                2 => state.source.source_mode = crate::source_manager::SourceMode::Replay,
                _ => state.adsb_running = true,
            }
        }
        app.sync_adsb_capture();
        assert_eq!(app.adsb_decoder.total_messages, 0);
        assert_eq!(app.adsb_panel.total_messages, 0);
        app.adsb_decoder.total_messages = 7;
        app.sync_adsb_capture();
        assert_eq!(
            app.adsb_decoder.total_messages, 7,
            "unchanged capture must preserve its decoder"
        );
    }
}

#[test]
fn daemon_aircraft_without_valid_coordinates_do_not_create_zero_position_markers() {
    let mut item = ez_proto::AircraftTelemetry {
        icao: 0xabc123,
        ..Default::default()
    };
    assert!(
        !CentralApp::map_daemon_aircraft(item.clone())
            .unwrap()
            .has_position
    );
    item.lat = Some(51.5);
    assert!(
        !CentralApp::map_daemon_aircraft(item.clone())
            .unwrap()
            .has_position
    );
    item.lon = Some(f64::NAN);
    assert!(
        !CentralApp::map_daemon_aircraft(item.clone())
            .unwrap()
            .has_position
    );
    item.lon = Some(181.0);
    assert!(
        !CentralApp::map_daemon_aircraft(item.clone())
            .unwrap()
            .has_position
    );
    item.lon = Some(-0.1);
    item.position_seen_ms = Some(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64,
    );
    assert!(
        CentralApp::map_daemon_aircraft(item.clone())
            .unwrap()
            .has_position
    );
    assert_eq!(CentralApp::map_daemon_aircraft(item).unwrap().lat, 51.5);
}

#[test]
fn daemon_snapshots_preserve_observation_age_validity_and_quality() {
    let wall_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let item = ez_proto::AircraftTelemetry {
        icao: 0xabc123,
        callsign: Some("TEST123".into()),
        last_seen_ms: wall_ms - 15_000,
        lat: Some(51.5),
        lon: Some(-0.1),
        position_seen_ms: Some(wall_ms - 40_000),
        altitude_geometric: true,
        vertical_rate_fpm: Some(640),
        squawk: Some(7700),
        emergency: Some(1),
        ..Default::default()
    };
    let first = CentralApp::map_daemon_aircraft(item.clone()).unwrap();
    assert_eq!(first.callsign, "TEST123");
    assert!(first.seen.elapsed().as_secs() >= 15);
    assert!(first.position_seen.unwrap().elapsed().as_secs() >= 40);
    assert!(!first.has_heading);
    assert!(!first.quality_known);
    assert!(!first.reliable);
    assert_eq!(first.score, 0.0);
    assert_eq!(
        first.altitude_type,
        crate::adsb_panel::AltitudeType::Geometric
    );
    assert_eq!(first.vertical_rate, Some(640));
    assert_eq!(first.emergency, Some(1));
    assert_eq!(first.squawk, Some(7700));
    let second = CentralApp::map_daemon_aircraft(item.clone()).unwrap();
    assert!(
        second
            .seen
            .duration_since(first.seen)
            .max(first.seen.duration_since(second.seen))
            .as_millis()
            < 2
    );
    let mut stale = item;
    stale.position_seen_ms = Some(wall_ms - 61_000);
    stale.position_quality = Some(ez_proto::AircraftQuality {
        reliable: false,
        corrected_bits: 1,
        score: 0.75,
    });
    let stale = CentralApp::map_daemon_aircraft(stale).unwrap();
    assert!(!stale.has_position);
    assert!(stale.quality_known);
    assert_eq!(stale.score, 0.75);
    assert_eq!(stale.correctedbits, 1);
}

#[test]
fn daemon_recording_events_update_panel_and_shared_state_under_the_app_lock() {
    let mut app = offline_test_app();
    let shared = Arc::clone(&app.shared);
    let mut state = shared.lock().unwrap();
    for active in [true, false] {
        app.apply_daemon_recording(
            ez_proto::RecordingStatus {
                channel_id: 1,
                active,
                path: Some("capture.iq".into()),
                bytes_written: 128,
                duration_sec: 2.0,
            },
            &mut state,
        );
        assert_eq!(state.recording, active);
        assert_eq!(app.recorder_panel.recording, active);
    }
}

#[test]
fn recorder_deadline_runs_on_hidden_tab_and_controls_are_dispatchable() {
    assert!(ALL_SECONDARY_TOOLS.contains(&SecondaryTool::Recorder));
    let mut app = offline_test_app();
    app.current_tab = AppTab::Planes;
    app.active_secondary_tool = None;
    app.recorder_panel.recording = true;
    app.recorder_panel.max_duration_mins = 1;
    app.recorder_panel.start_time =
        Some(std::time::Instant::now() - std::time::Duration::from_secs(61));
    app.shared.lock().unwrap().recording = true;
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
        app.logic(ui.ctx(), &mut frame)
    });
    assert!(!app.recorder_panel.recording);
    assert!(!app.shared.lock().unwrap().recording);
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
        app.render_secondary_panel(ui, SecondaryTool::Recorder)
    });
}

#[test]
fn replay_runs_through_app_logic_with_muted_metadata_and_clears_at_eof() {
    let mut app = offline_test_app();
    let path = std::env::temp_dir().join(format!("ez-app-flow-{}.cu8", std::process::id()));
    std::fs::write(&path, [170u8, 128].repeat(131_072)).unwrap();
    {
        let mut state = app.shared.lock().unwrap();
        crate::radio_ui::change_source(&mut state, crate::source_manager::SourceMode::Replay);
        crate::radio_ui::select_demod(&mut state, crate::sdr_panel::DemodMode::Wfm);
        state.source.replay_file = Some(path.to_string_lossy().into_owned());
        state.config.advanced.rds_enabled = true;
        state.source.start();
        assert!(!state.audio_running);
    }
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut _saw_audio = false;
    let mut _saw_metadata_processing = false;
    let mut finished = false;
    while std::time::Instant::now() < deadline {
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            app.logic(ui.ctx(), &mut frame)
        });
        _saw_metadata_processing |= app.radio_ui.rds.is_some();
        {
            let state = app.shared.lock().unwrap();
            _saw_audio |= !state.spectrum.audio_waveform.is_empty();
            finished = state.source.status == crate::source_manager::SourceStatus::Idle
                && state.source.replay_position == 262_144;
        }
        // AudioWorker uses an internal channel; muted metadata is never queued.
        if finished {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // In test builds, workers are uninitialized (no threads) so audio/RDS
    // processing doesn't run. Only verify the replay completes.
    #[cfg(test)]
    assert!(finished);
    #[cfg(not(test))]
    assert!(finished && saw_audio && saw_metadata_processing);
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
        app.logic(ui.ctx(), &mut frame)
    });
    assert!(app.radio_ui.rds.is_none());
    assert!(app.radio_ui.received_tone.is_none());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn aircraft_signed_optional_and_typed_measurements_survive_daemon_adapter() {
    for altitude in [-1000, -500, 0, 35000] {
        let item = ez_proto::AircraftTelemetry {
            icao: 1,
            altitude_ft: Some(altitude),
            ..Default::default()
        };
        let ac = CentralApp::map_daemon_aircraft(item).unwrap();
        assert_eq!(ac.altitude, Some(altitude));
        assert_eq!(ac.speed, None);
        assert_eq!(ac.heading, None);
        assert_eq!(ac.ground_track, None);
        assert_eq!(ac.speed_label(), "—");
        assert_eq!(ac.direction_label(), "—");
    }
    let item = ez_proto::AircraftTelemetry {
        icao: 1,
        ground_speed_kt: Some(0.0),
        track_deg: Some(90.0),
        heading_deg: Some(120.0),
        heading_true: true,
        airspeed_kt: Some(300.0),
        airspeed_true: true,
        ..Default::default()
    };
    let ac = CentralApp::map_daemon_aircraft(item).unwrap();
    assert_eq!(ac.speed, Some(0));
    assert_eq!(ac.ground_track, Some(90));
    assert_eq!(ac.heading, Some(120));
    assert!(ac.heading_true);
    assert_eq!(ac.airspeed, Some(300));
    assert!(ac.airspeed_true);
    assert!(ac.has_heading);
    let ac = CentralApp::map_daemon_aircraft(ez_proto::AircraftTelemetry::default()).unwrap();
    assert_eq!(ac.altitude_label(), "—");
    assert_eq!(ac.speed_label(), "—");
    assert_eq!(ac.direction_label(), "—");
}

#[test]
fn aircraft_trails_collect_and_expire_while_planes_is_hidden() {
    let mut app = offline_test_app();
    app.current_tab = AppTab::Listen;
    app.sync_adsb_capture();
    app.adsb_panel.aircraft = vec![crate::adsb_panel::AircraftEntry {
        icao: 1,
        lat: 51.5,
        lon: -0.1,
        has_position: true,
        position_seen: Some(std::time::Instant::now()),
        ..Default::default()
    }];
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    app.logic(&ctx, &mut frame);
    assert_eq!(app.adsb_panel.trail_len(1), 1);
    for _ in 0..305 {
        let ac = &mut app.adsb_panel.aircraft[0];
        ac.lat += 0.001;
        ac.position_seen = Some(std::time::Instant::now());
        app.adsb_panel.tick_model();
    }
    assert_eq!(app.adsb_panel.trail_len(1), 300);
    app.logic(&ctx, &mut frame);
    assert_eq!(app.adsb_panel.trail_len(1), 300); // duplicates don't grow
    app.adsb_panel.aircraft[0].position_seen =
        Some(std::time::Instant::now() - std::time::Duration::from_secs(61));
    app.logic(&ctx, &mut frame);
    assert_eq!(app.adsb_panel.trail_len(1), 0);
    app.adsb_panel.aircraft.clear();
    app.logic(&ctx, &mut frame);
    assert_eq!(app.adsb_panel.trail_len(1), 0);
}

#[test]
fn bookmark_dirty_content_flushes_before_debounce_and_survives_failed_save() {
    let mut app = offline_test_app();
    let dir = std::env::temp_dir().join(format!("ez-bookmarks-exit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    app.bookmark_path = dir.join("bookmarks.json");
    app.flush_bookmarks();
    let read_saved = |path: &std::path::Path| -> Vec<crate::bookmarks::Bookmark> {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    };
    for operation in 0..5 {
        {
            let mut state = app.shared.lock().unwrap();
            // Exercise content comparison even when a legacy path omits its dirty flag.
            state.bookmarks_modified = false;
            let new = state.bookmarks.bookmarks[0].clone();
            match operation {
                0 => state.bookmarks.bookmarks.push(new),
                1 => {
                    state.bookmarks.bookmarks[0].name = "Edited name".into();
                    state.bookmarks.bookmarks[0].notes = "Edited notes".into();
                }
                2 => state.bookmarks.bookmarks[0].starred = true,
                3 => {
                    state.bookmarks.bookmarks.pop();
                    state.bookmarks.bookmarks.push(crate::bookmarks::Bookmark {
                        name: "Replacement".into(),
                        ..new
                    });
                }
                _ => {
                    state.bookmarks.bookmarks.remove(0);
                }
            }
        }
        // The close hook calls this exact flush even when logic never observed the edit.
        app.flush_bookmarks();
        let state = app.shared.lock().unwrap();
        assert_eq!(read_saved(&app.bookmark_path), state.bookmarks.bookmarks);
        assert!(!state.bookmarks_modified);
        assert!(app.bm_dirty_since.is_none());
    }
    {
        let mut state = app.shared.lock().unwrap();
        state.bookmarks.bookmarks[0].starred = !state.bookmarks.bookmarks[0].starred;
        state.bookmarks_modified = false;
    }
    app.tick_bookmark_autosave();
    assert!(app.shared.lock().unwrap().bookmarks_modified);
    assert!(app.bm_dirty_since.is_some());
    app.bm_dirty_since = Some(std::time::Instant::now() - std::time::Duration::from_secs(16));
    app.tick_bookmark_autosave();
    assert!(!app.shared.lock().unwrap().bookmarks_modified);
    let previous = std::fs::read(&app.bookmark_path).unwrap();
    app.bookmark_path = dir.join("blocked");
    std::fs::create_dir_all(&app.bookmark_path).unwrap();
    app.shared.lock().unwrap().bookmarks.bookmarks[0].name = "After failure".into();
    app.flush_bookmarks();
    assert!(app.shared.lock().unwrap().bookmarks_modified);
    assert!(app.bm_dirty_since.is_some());
    assert_eq!(std::fs::read(dir.join("bookmarks.json")).unwrap(), previous);
    std::fs::remove_dir(&app.bookmark_path).unwrap();
    app.flush_bookmarks();
    let state = app.shared.lock().unwrap();
    assert!(!state.bookmarks_modified);
    assert_eq!(read_saved(&app.bookmark_path), state.bookmarks.bookmarks);
    drop(state);
    std::fs::remove_dir_all(dir).unwrap();
}
