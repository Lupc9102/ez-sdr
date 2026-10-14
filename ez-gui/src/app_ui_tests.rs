use super::*;
use eframe::App;

use super::software_renderer;

#[test]
fn discord_delivery_result_reaches_shared_status_surface() {
    let mut status_bar = crate::status_bar::StatusBar::new();
    let mut last_sequence = 0;
    publish_discord_delivery_status(
        &mut status_bar,
        &mut last_sequence,
        crate::discord::DiscordDeliverySnapshot {
            sequence: 1,
            submitted: 1,
            failed: 1,
            latest: Some(crate::discord::DiscordDeliveryOutcome::Failed(
                "Discord returned HTTP 401 (Unauthorized)".into(),
            )),
            ..Default::default()
        },
    );
    let current = status_bar.current().unwrap();
    assert_eq!(current.severity, crate::status_bar::StatusSeverity::Error);
    assert!(current.text.contains("HTTP 401"));

    publish_discord_delivery_status(
        &mut status_bar,
        &mut last_sequence,
        crate::discord::DiscordDeliverySnapshot {
            sequence: 2,
            submitted: 2,
            delivered: 1,
            latest: Some(crate::discord::DiscordDeliveryOutcome::Queued),
            ..Default::default()
        },
    );
    assert!(status_bar.current().unwrap().text.contains("HTTP 401"));
}

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
        ("settings", AppTab::Listen),
        ("setup", AppTab::Listen),
    ] {
        if std::env::var("EZ_SDR_PREVIEW_WORKSPACE")
            .is_ok_and(|requested| !requested.split(',').any(|item| item == name))
        {
            continue;
        }
        app.current_tab = tab;
        app.show_settings = name == "settings";
        app.show_setup_guide = name == "setup";
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
            if std::env::var("EZ_SDR_PREVIEW_SIZE").is_ok_and(|requested| {
                !requested
                    .split(',')
                    .any(|size| size == format!("{width}x{height}"))
            }) {
                continue;
            }
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
fn rigctl_subset_uses_hamlib_vfo_modes_and_normalized_audio_gain() {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpStream;
    use std::thread;
    use std::time::Duration;

    fn exchange(
        app: &mut CentralApp,
        stream: &mut TcpStream,
        replies: &mut BufReader<TcpStream>,
        command: &str,
    ) -> String {
        stream.write_all(command.as_bytes()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            app.process_rigctl_requests();
            let mut response = String::new();
            match replies.read_line(&mut response) {
                Ok(count) if count > 0 => return response,
                Ok(_) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(error) => panic!("rigctl reply failed: {error}"),
            }
            assert!(
                Instant::now() < deadline,
                "rigctl did not reply to {command:?}"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    let mut app = offline_test_app();
    app.rigctl.set_enabled(true, 0);
    let deadline = Instant::now() + Duration::from_secs(2);
    let port = loop {
        let port = app.rigctl.bound_port();
        if port != 0 {
            break port;
        }
        assert!(Instant::now() < deadline, "rigctl listener did not start");
        thread::sleep(Duration::from_millis(5));
    };
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_millis(10)))
        .unwrap();
    let mut replies = BufReader::new(stream.try_clone().unwrap());

    assert_eq!(
        exchange(&mut app, &mut stream, &mut replies, "v\n"),
        "VFOA\n"
    );
    assert_eq!(
        exchange(&mut app, &mut stream, &mut replies, "V VFOA\n"),
        "RPRT 0\n"
    );
    assert_eq!(
        exchange(&mut app, &mut stream, &mut replies, "V VFOB\n"),
        "RPRT -1\n"
    );
    assert_eq!(
        exchange(&mut app, &mut stream, &mut replies, "M FM 12500\n"),
        "RPRT 0\n"
    );
    assert_eq!(
        exchange(&mut app, &mut stream, &mut replies, "m\n"),
        "FM 12500\n"
    );
    assert_eq!(
        exchange(&mut app, &mut stream, &mut replies, "M RAW 12500\n"),
        "RPRT -1\n"
    );
    assert_eq!(
        exchange(&mut app, &mut stream, &mut replies, "L AF 0.42\n"),
        "RPRT 0\n"
    );
    assert!((app.shared.lock().unwrap().volume - 0.42).abs() < f32::EPSILON);
    assert_eq!(
        exchange(&mut app, &mut stream, &mut replies, "l AF\n"),
        "0.420\n"
    );
    assert_eq!(
        exchange(&mut app, &mut stream, &mut replies, "L AF NaN\n"),
        "RPRT -1\n"
    );
    assert!((app.shared.lock().unwrap().volume - 0.42).abs() < f32::EPSILON);

    drop(replies);
    drop(stream);
    app.rigctl.stop();
}

fn render_app_ui(
    app: &mut CentralApp,
    ctx: &egui::Context,
    frame: &mut eframe::Frame,
) -> egui::FullOutput {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1400.0, 1000.0),
        )),
        ..Default::default()
    };
    ctx.run_ui(input, |ui| app.ui(ui, frame))
}

fn render_app_ui_events(
    app: &mut CentralApp,
    ctx: &egui::Context,
    frame: &mut eframe::Frame,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1400.0, 1000.0),
        )),
        events,
        ..Default::default()
    };
    ctx.run_ui(input, |ui| app.ui(ui, frame))
}

fn pointer_click(rect: egui::Rect) -> Vec<egui::Event> {
    let pos = rect.center();
    vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        },
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        },
    ]
}

fn daemon_control_rect(ctx: &egui::Context, name: &str) -> egui::Rect {
    ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::new(name)))
        .expect("daemon control should have rendered")
}

async fn wait_for_daemon_source(
    app: &mut CentralApp,
    ready: impl Fn(&crate::source_manager::SourceStatus) -> bool,
    description: &str,
) -> crate::source_manager::SourceStatus {
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        app.drain_daemon_events();
        let status = app.shared.lock().unwrap().source.status.clone();
        if ready(&status) {
            return status;
        }
        assert!(Instant::now() < deadline, "{description}: {status:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn render_app_logic(app: &mut CentralApp, ctx: &egui::Context, frame: &mut eframe::Frame) {
    let _ = ctx.run_ui(egui::RawInput::default(), |_| app.logic(ctx, frame));
}

fn set_settings_source_section_open(ctx: &egui::Context, open: bool) {
    let header_id = ctx
        .data(|data| {
            data.get_temp::<egui::Id>(egui::Id::new(crate::config::SETTINGS_SOURCE_HEADER_TEST_ID))
        })
        .expect("Settings should have rendered the Source header");
    let mut state = egui::containers::collapsing_header::CollapsingState::load_with_default_open(
        ctx, header_id, false,
    );
    state.set_open(open);
    state.store(ctx);
}

fn rendered_text(output: &egui::FullOutput) -> String {
    fn append(shape: &egui::Shape, text: &mut String) {
        match shape {
            egui::Shape::Text(shape) => text.push_str(shape.galley.text()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| append(shape, text)),
            _ => {}
        }
    }
    let mut text = String::new();
    output
        .shapes
        .iter()
        .for_each(|shape| append(&shape.shape, &mut text));
    text
}

#[test]
fn window_close_keeps_repainting_until_recording_finalization_finishes() {
    let mut app = offline_test_app();
    let (release, release_rx) = crossbeam_channel::bounded(1);
    app.recorder_panel
        .test_set_disk_worker(std::thread::spawn(move || {
            release_rx.recv().unwrap();
            Ok(64)
        }));
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    let mut maximum = Duration::ZERO;
    for tick in 0..8 {
        let mut input = egui::RawInput::default();
        if tick == 0 {
            input
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .events
                .push(egui::ViewportEvent::Close);
        }
        ctx.begin_pass(input);
        let started = Instant::now();
        app.logic(&ctx, &mut frame);
        maximum = maximum.max(started.elapsed());
        let output = ctx.end_pass();
        assert!(output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&egui::ViewportCommand::CancelClose));
        assert!(app.recorder_panel.is_finalizing());
        assert!(app.recorder_panel.recording && app.shared.lock().unwrap().recording);
    }
    assert!(
        maximum < Duration::from_millis(50),
        "Close handling blocked for {maximum:?}"
    );
    release.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        ctx.begin_pass(egui::RawInput::default());
        app.logic(&ctx, &mut frame);
        let output = ctx.end_pass();
        if output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&egui::ViewportCommand::Close)
        {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!app.recorder_panel.recording && !app.shared.lock().unwrap().recording);
    assert_eq!(app.recorder_panel.bytes_written, 64);
    eprintln!(
        "Eight close/repaint ticks with blocked disk: max {} us; close allowed only after final result",
        maximum.as_micros()
    );
}

#[test]
fn setup_guide_and_settings_render_as_independent_popups() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = CentralApp::new_with_config(&cc, AppConfig::default());
    app.theme_applied = true;
    app.show_setup_guide = true;
    app.show_settings = true;
    let mut frame = eframe::Frame::_new_kittest();
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1200.0, 800.0),
        )),
        ..Default::default()
    };
    let _ = ctx.run_ui(input, |ui| app.ui(ui, &mut frame));
    assert!(app.show_setup_guide, "Setup Guide popup should stay open");
    assert!(app.show_settings, "Settings popup should stay open");
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn active_daemon_controls_mask_authenticate_persist_reconnect_and_clear_token() {
    use crate::daemon_client::tests::TestDaemon;
    use crate::source_manager::{SourceMode, SourceStatus};

    let auth = TestDaemon::spawn_with_auth("radio-ui-auth", "valid-ui-secret").await;
    let local = TestDaemon::spawn("radio-ui-open").await;
    let mut app = tokio::task::block_in_place(offline_test_app);
    {
        let mut state = app.shared.lock().unwrap();
        state.source.source_mode = SourceMode::Daemon;
        state
            .source
            .set_daemon_credentials(auth.addr.to_string(), String::new());
    }
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();

    let first = render_app_ui(&mut app, &ctx, &mut frame);
    let token_rect = daemon_control_rect(&ctx, "radio.test.daemon_token_rect");
    let _ = render_app_ui_events(&mut app, &ctx, &mut frame, pointer_click(token_rect));
    let visible = render_app_ui_events(
        &mut app,
        &ctx,
        &mut frame,
        vec![egui::Event::Paste("valid-ui-secret".into())],
    );
    let rendered = rendered_text(&visible);
    assert!(
        !rendered.contains("valid-ui-secret"),
        "password input exposed its contents in rendered text: {rendered}"
    );
    {
        let state = app.shared.lock().unwrap();
        assert_eq!(state.source.daemon_token, "valid-ui-secret");
        assert_eq!(
            state.config.source_preferences.daemon_token, "valid-ui-secret",
            "editing the mounted credential should update persisted source preferences"
        );
        assert!(!format!("{:?}", state.config.source_preferences).contains("valid-ui-secret"));
    }
    let reconnect_rect = daemon_control_rect(&ctx, "radio.test.daemon_reconnect_rect");
    let _ = render_app_ui_events(&mut app, &ctx, &mut frame, pointer_click(reconnect_rect));
    let connected = wait_for_daemon_source(
        &mut app,
        |status| *status == SourceStatus::Running,
        "valid token did not connect through the Radio Reconnect button",
    )
    .await;
    assert_eq!(connected, SourceStatus::Running);

    let saved = app.exit_config_snapshot().expect("config snapshot");
    let saved: AppConfig = serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    let ctx_restart = egui::Context::default();
    let cc_restart = eframe::CreationContext::_new_kittest(ctx_restart);
    let restarted = tokio::task::block_in_place(|| CentralApp::new_with_config(&cc_restart, saved));
    {
        let state = restarted.shared.lock().unwrap();
        assert_eq!(state.source.source_mode, SourceMode::Daemon);
        assert_eq!(state.source.daemon_addr, auth.addr.to_string());
        assert_eq!(state.source.daemon_token, "valid-ui-secret");
        assert_eq!(state.source.status, SourceStatus::Idle);
    }
    drop(restarted);

    {
        let mut state = app.shared.lock().unwrap();
        state
            .source
            .set_daemon_credentials(auth.addr.to_string(), "wrong-ui-secret".into());
        crate::radio_ui::reconnect_daemon(&mut state, false);
    }
    let error = wait_for_daemon_source(
        &mut app,
        |status| matches!(status, SourceStatus::Error(_)),
        "incorrect token did not produce an authentication error",
    )
    .await;
    let SourceStatus::Error(error) = error else {
        unreachable!()
    };
    assert!(error.to_lowercase().contains("auth"), "{error}");
    assert!(
        !error.contains("wrong-ui-secret"),
        "credential leaked: {error}"
    );
    assert!(
        !error.contains("valid-ui-secret"),
        "credential leaked: {error}"
    );

    {
        let mut state = app.shared.lock().unwrap();
        state
            .source
            .set_daemon_credentials(local.addr.to_string(), String::new());
        crate::radio_ui::reconnect_daemon(&mut state, false);
    }
    let cleared = wait_for_daemon_source(
        &mut app,
        |status| *status == SourceStatus::Running,
        "clearing the credential should allow an unauthenticated local daemon",
    )
    .await;
    assert_eq!(cleared, SourceStatus::Running);
    let saved_cleared = app.exit_config_snapshot().expect("cleared config snapshot");
    assert!(saved_cleared.source_preferences.daemon_token.is_empty());

    {
        let mut state = app.shared.lock().unwrap();
        state.source.stop();
        state.source.wait_for_shutdown();
    }
    drop(first);
    auth.shutdown().await;
    local.shutdown().await;
}

#[test]
fn imported_audio_selection_restarts_a_and_b_once_without_changing_capture() {
    use crate::audio_output::AudioOutputSelection;
    use crate::source_manager::{SourceMode, SourceStatus};

    let mut app = offline_test_app();
    let first = AudioOutputSelection {
        device_id: Some("mock-output-a".into()),
        sample_rate: 44_100,
    };
    let second = AudioOutputSelection {
        device_id: Some("mock-output-b".into()),
        sample_rate: 48_000,
    };
    let main_requests = Arc::new(Mutex::new(Vec::new()));
    let main_request_log = Arc::clone(&main_requests);
    app.audio.set_test_start_hook(move |selection, channels| {
        main_request_log
            .lock()
            .unwrap()
            .push((selection.clone(), channels));
        Ok(if selection.sample_rate == 0 {
            48_000
        } else {
            selection.sample_rate
        })
    });
    let b_requests = Arc::new(Mutex::new(Vec::new()));
    let b_request_log = Arc::clone(&b_requests);
    app.secondary_vfo
        .set_test_output_start_hook(move |selection, channels| {
            b_request_log
                .lock()
                .unwrap()
                .push((selection.clone(), channels));
            Ok(if selection.sample_rate == 0 {
                48_000
            } else {
                selection.sample_rate
            })
        });
    let capture_before = {
        let mut state = app.shared.lock().unwrap();
        state.source.source_mode = SourceMode::Simulated;
        state.source.status = SourceStatus::Running;
        state.source.center_frequency_hz = Some(100_000_000);
        state.source.frequency_hz = 100_000_000;
        state.source.sample_rate_hz = 2_000_000;
        state.audio_running = true;
        state.config.secondary_vfo.enabled = true;
        state.config.secondary_vfo.frequency_hz = 100_000_000;
        state.config.advanced.audio_output = first.clone();
        (
            state.source.frequency_hz,
            state.source.center_frequency_hz,
            state.source.sample_rate_hz,
            state.source.stream_generation(),
        )
    };
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    app.logic(&ctx, &mut frame);
    assert!(app.audio.is_running());
    assert!(app.secondary_vfo.status.starts_with("Listening on B"));

    let recording_dir =
        std::env::temp_dir().join(format!("ez-audio-device-change-{}", std::process::id()));
    std::fs::create_dir_all(&recording_dir).unwrap();
    app.recorder_panel.output_dir = recording_dir.display().to_string();
    app.recorder_panel.filename_template = "before-switch".into();
    app.recorder_panel.record_iq = false;
    app.recorder_panel.record_audio = true;
    app.recorder_panel.set_audio_stream(None);
    app.recorder_panel.start_recording();
    assert!(
        app.recorder_panel.recording,
        "{}",
        app.recorder_panel.last_error
    );
    app.recorder_panel.write_audio_samples(&vec![0.25; 44_100]);

    let mut imported = app.shared.lock().unwrap().config.clone();
    imported.advanced.audio_output = second.clone();
    imported.needs_apply = true;
    app.pending_settings_apply = Some(imported);
    app.logic(&ctx, &mut frame);
    app.logic(&ctx, &mut frame);
    let finalization_deadline = Instant::now() + Duration::from_secs(2);
    while app.recorder_panel.is_finalizing() {
        app.recorder_panel.tick();
        assert!(
            Instant::now() < finalization_deadline,
            "WAV finalization stalled"
        );
        std::thread::sleep(Duration::from_millis(2));
    }

    let expected = vec![(first.clone(), 1), (second.clone(), 1)];
    assert_eq!(*main_requests.lock().unwrap(), expected);
    assert_eq!(*b_requests.lock().unwrap(), expected);
    assert_eq!(app.audio.sample_rate(), 48_000);
    assert!(!app.recorder_panel.recording);
    assert!(app.recorder_panel.last_error.contains("44100 to 48000 Hz"));
    let wav = hound::WavReader::open(recording_dir.join("before-switch_audio.wav")).unwrap();
    assert_eq!(wav.spec().sample_rate, 44_100);
    assert_eq!(wav.duration(), 44_100);
    drop(wav);
    let state = app.shared.lock().unwrap();
    assert_eq!(
        (
            state.source.frequency_hz,
            state.source.center_frequency_hz,
            state.source.sample_rate_hz,
            state.source.stream_generation(),
        ),
        capture_before,
        "output-device changes must preserve the active capture identity"
    );
    drop(state);
    std::fs::remove_dir_all(recording_dir).unwrap();
}

#[test]
fn unavailable_main_audio_selection_keeps_exact_request_and_reports_error() {
    use crate::audio_output::AudioOutputSelection;

    let mut app = offline_test_app();
    let requested = AudioOutputSelection {
        device_id: Some("missing-output-4c59".into()),
        sample_rate: 44_100,
    };
    let requests = Arc::new(Mutex::new(Vec::new()));
    let request_log = Arc::clone(&requests);
    app.audio.set_test_start_hook(move |selection, channels| {
        request_log
            .lock()
            .unwrap()
            .push((selection.clone(), channels));
        Err("Selected audio output device is unavailable.".into())
    });
    {
        let mut state = app.shared.lock().unwrap();
        state.source.status = crate::source_manager::SourceStatus::Running;
        state.config.advanced.audio_output = requested.clone();
        state.audio_running = true;
    }
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    app.logic(&ctx, &mut frame);

    assert_eq!(*requests.lock().unwrap(), vec![(requested, 1)]);
    assert!(app.audio.has_failed());
    assert!(app.status_bar.current().is_some_and(|message| message
        .text
        .contains("Selected audio output device is unavailable")));
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
                stop_reason: None,
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
fn recorder_deadline_runs_on_hidden_tab() {
    let mut app = offline_test_app();
    app.current_tab = AppTab::Planes;
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

#[test]
fn save_and_apply_preserves_live_capture_when_defaults_and_unrelated_settings_change() {
    let ctx = egui::Context::default();
    let mut app = offline_test_app();
    let mut frame = eframe::Frame::_new_kittest();
    app.theme_applied = true;
    {
        let mut state = app.shared.lock().unwrap();
        state.source.source_mode = crate::source_manager::SourceMode::Hardware;
        state.source.frequency_hz = 1_090_000_000;
        state.source.center_frequency_hz = Some(1_090_000_000);
        state.source.sample_rate_hz = 2_400_000;
        state.source.gain_db = 32.8;
        // No physical receiver is opened: the real logic must preserve this
        // running session rather than restart a device to apply preferences.
        state.source.status = crate::source_manager::SourceStatus::Running;
    }
    for group in ["theme", "font", "integration", "startup defaults"] {
        let expected = {
            let mut state = app.shared.lock().unwrap();
            match group {
                "theme" => state.config.theme_config = crate::theme::ThemeConfig::light(),
                "font" => state.config.font_scale = 1.25,
                "integration" => state.config.mqtt_topic_prefix = "test/preserved".into(),
                _ => {
                    state.config.default_freq_hz = 144_000_000;
                    state.config.default_sample_rate = 1_024_000;
                    state.config.default_gain = 15.0;
                }
            }
            state.config.needs_apply = true;
            (
                state.source.frequency_hz,
                state.source.capture_center_frequency_hz(),
                state.source.sample_rate_hz,
                state.source.gain_db,
                state.source.stream_generation(),
            )
        };
        let _ = ctx.run_ui(egui::RawInput::default(), |_| app.logic(&ctx, &mut frame));
        let state = app.shared.lock().unwrap();
        assert_eq!(
            (
                state.source.frequency_hz,
                state.source.capture_center_frequency_hz(),
                state.source.sample_rate_hz,
                state.source.gain_db,
                state.source.stream_generation(),
            ),
            expected,
            "applying {group} changed the active receiver"
        );
        assert_eq!(
            state.source.status,
            crate::source_manager::SourceStatus::Running
        );
        assert!(!state.config.needs_apply);
    }
    assert_eq!(ctx.pixels_per_point(), 1.25);
}

#[test]
fn settings_popup_draft_keeps_ppm_with_source_section_open_or_closed() {
    let mut app = offline_test_app();
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    app.current_tab = AppTab::Listen;
    app.show_settings = true;
    {
        let mut state = app.shared.lock().unwrap();
        state.source.source_mode = crate::source_manager::SourceMode::Hardware;
        state.source.ppm_correction = 3;
        state.config.ppm_correction = 3;
    }

    render_app_ui(&mut app, &ctx, &mut frame);
    set_settings_source_section_open(&ctx, true);
    assert!(
        egui::containers::collapsing_header::CollapsingState::load_with_default_open(
            &ctx,
            ctx.data(|data| {
                data.get_temp::<egui::Id>(egui::Id::new(
                    crate::config::SETTINGS_SOURCE_HEADER_TEST_ID,
                ))
                .unwrap()
            }),
            false
        )
        .is_open()
    );
    app.settings_editor.draft_mut().unwrap().ppm_correction = 37;
    render_app_ui(&mut app, &ctx, &mut frame);
    {
        let state = app.shared.lock().unwrap();
        assert_eq!(state.source.ppm_correction, 3);
        assert_eq!(state.config.ppm_correction, 3);
    }
    assert_eq!(app.settings_editor.draft().unwrap().ppm_correction, 37);

    set_settings_source_section_open(&ctx, false);
    render_app_ui(&mut app, &ctx, &mut frame);
    assert_eq!(app.settings_editor.draft().unwrap().ppm_correction, 37);
    assert_eq!(app.shared.lock().unwrap().config.ppm_correction, 3);

    app.current_tab = AppTab::Planes;
    render_app_ui(&mut app, &ctx, &mut frame);
    assert_eq!(app.settings_editor.draft().unwrap().ppm_correction, 37);
    assert_eq!(app.shared.lock().unwrap().config.ppm_correction, 3);
    app.current_tab = AppTab::Listen;

    app.settings_editor.draft_mut().unwrap().needs_apply = true;
    render_app_ui(&mut app, &ctx, &mut frame);
    assert_eq!(
        app.pending_settings_apply.as_ref().unwrap().ppm_correction,
        37
    );
    let before = {
        let state = app.shared.lock().unwrap();
        (
            state.source.frequency_hz,
            state.source.sample_rate_hz,
            state.source.gain_db,
        )
    };
    render_app_logic(&mut app, &ctx, &mut frame);
    let state = app.shared.lock().unwrap();
    assert_eq!(state.source.ppm_correction, 37);
    assert_eq!(
        (
            state.source.frequency_hz,
            state.source.sample_rate_hz,
            state.source.gain_db,
        ),
        before,
        "applying calibration must not apply startup frequency, rate, or gain"
    );
}

#[test]
fn settings_import_snapshot_reaches_spectrum_before_radio_persists_it() {
    let mut app = offline_test_app();
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    app.current_tab = AppTab::Listen;
    app.show_settings = true;
    render_app_ui(&mut app, &ctx, &mut frame);

    let imported = imported_receiver_profile();
    app.settings_editor
        .inject_import_result(&ctx, Ok(Some(imported.clone())));
    render_app_ui(&mut app, &ctx, &mut frame);
    assert_eq!(
        app.pending_settings_apply
            .as_ref()
            .unwrap()
            .advanced
            .fft_size,
        8192
    );
    assert_ne!(
        app.shared.lock().unwrap().config.advanced.fft_size,
        8192,
        "import must stay queued until the logic boundary"
    );

    render_app_logic(&mut app, &ctx, &mut frame);
    assert_imported_receiver_profile(&app);
    render_app_ui(&mut app, &ctx, &mut frame);
    let state = app.shared.lock().unwrap();
    assert_eq!(state.config.advanced.fft_size, imported.advanced.fft_size);
    assert_eq!(state.config.advanced.window, imported.advanced.window);
    assert_eq!(state.config.advanced.fft_rate, imported.advanced.fft_rate);
    assert_eq!(state.config.advanced.wf_depth, imported.advanced.wf_depth);
    assert_eq!(state.config.advanced.wf_speed, imported.advanced.wf_speed);
    assert_eq!(state.config.advanced.grid, imported.advanced.grid);
    assert_eq!(state.config.color_map, imported.color_map);
    assert_eq!(state.config.spectrum_min_db, imported.advanced.db_min);
    assert_eq!(state.config.spectrum_max_db, imported.advanced.db_max);
    assert_eq!(state.config.wf_min_db, imported.wf_min_db);
    assert_eq!(state.config.wf_max_db, imported.wf_max_db);
}

#[test]
fn settings_reset_replaces_a_nondefault_live_spectrum() {
    let mut app = offline_test_app();
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    app.current_tab = AppTab::Listen;
    app.shared.lock().unwrap().config = imported_receiver_profile();
    app.apply_advanced();
    assert_eq!(
        app.shared
            .lock()
            .unwrap()
            .spectrum
            .display_settings()
            .fft_size,
        8192
    );
    app.show_settings = true;
    render_app_ui(&mut app, &ctx, &mut frame);

    app.settings_editor.draft_mut().unwrap().reset_to_defaults();
    render_app_ui(&mut app, &ctx, &mut frame);
    assert!(app.pending_settings_apply.is_some());
    render_app_logic(&mut app, &ctx, &mut frame);
    render_app_ui(&mut app, &ctx, &mut frame);

    let defaults = AppConfig::default();
    let state = app.shared.lock().unwrap();
    let display = state.spectrum.display_settings();
    assert_eq!(display.fft_size, defaults.advanced.fft_size);
    assert_eq!(display.fft_rate, defaults.advanced.fft_rate);
    assert_eq!(
        display.window,
        crate::advanced_panel::window_from_str(&defaults.advanced.window)
    );
    assert_eq!(display.waterfall_history, defaults.advanced.wf_depth);
    assert_eq!(display.waterfall_every_n, defaults.advanced.wf_speed);
    assert_eq!(display.grid, defaults.advanced.grid);
    assert_eq!(
        display.color_map,
        crate::advanced_panel::color_from_str(&defaults.color_map)
    );
    assert_eq!(
        (display.db_min, display.db_max),
        (defaults.advanced.db_min, defaults.advanced.db_max)
    );
    assert_eq!(
        (display.wf_min_db, display.wf_max_db),
        (defaults.wf_min_db, defaults.wf_max_db)
    );
    assert_eq!(state.config.advanced.fft_size, defaults.advanced.fft_size);
    drop(state);
    app.mqtt.disconnect();
}

#[test]
fn settings_close_discards_draft_and_late_import_from_old_session() {
    let mut app = offline_test_app();
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    app.show_settings = true;
    render_app_ui(&mut app, &ctx, &mut frame);
    let old_session = app.settings_editor.session_id();
    let effective_prefix = app.shared.lock().unwrap().config.mqtt_topic_prefix.clone();
    app.settings_editor.draft_mut().unwrap().mqtt_topic_prefix = "unapplied draft".into();
    app.settings_editor
        .inject_import_result(&ctx, Ok(Some(imported_receiver_profile())));

    app.show_settings = false;
    render_app_ui(&mut app, &ctx, &mut frame);
    assert!(app.settings_editor.draft().is_none());
    app.show_settings = true;
    render_app_ui(&mut app, &ctx, &mut frame);
    let new_session = app.settings_editor.session_id();
    assert_ne!(new_session, old_session);
    assert_eq!(
        app.settings_editor.draft().unwrap().mqtt_topic_prefix,
        effective_prefix
    );

    let mut stale = imported_receiver_profile();
    stale.mqtt_topic_prefix = "stale import".into();
    app.settings_editor
        .inject_import_result_for_session(&ctx, old_session, Ok(Some(stale)));
    render_app_ui(&mut app, &ctx, &mut frame);
    assert!(app.pending_settings_apply.is_none());
    assert_eq!(
        app.settings_editor.draft().unwrap().mqtt_topic_prefix,
        effective_prefix
    );
}

#[test]
fn settings_save_failure_is_visible_retry_succeeds_and_queued_save_survives_close() {
    let mut app = offline_test_app();
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    let directory = std::env::temp_dir().join(format!(
        "ez-settings-editor-{}-{}",
        std::process::id(),
        ez_proto::new_capture_epoch()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("settings.json");
    std::fs::create_dir_all(&path).unwrap();
    app.config_path = path.clone();
    {
        let mut state = app.shared.lock().unwrap();
        state.source.sample_rate_hz = 2_400_000;
        state.source.ppm_correction = 3;
        state.config.default_sample_rate = 2_400_000;
        state.config.ppm_correction = 3;
    }
    app.show_settings = true;
    render_app_ui(&mut app, &ctx, &mut frame);
    app.settings_editor.draft_mut().unwrap().default_sample_rate = 1_024_000;
    assert!(!app
        .settings_editor
        .draft_mut()
        .unwrap()
        .save_and_apply(&path));
    let failed_output = render_app_ui(&mut app, &ctx, &mut frame);
    assert!(rendered_text(&failed_output).contains("Could not save settings"));
    render_app_logic(&mut app, &ctx, &mut frame);
    assert_eq!(
        app.shared.lock().unwrap().config.default_sample_rate,
        1_024_000
    );

    std::fs::remove_dir(&path).unwrap();
    {
        let draft = app.settings_editor.draft_mut().unwrap();
        draft.default_sample_rate = 1_536_000;
        draft.ppm_correction = 37;
        draft.color_map = "Inferno".into();
        draft.advanced.fft_size = 8192;
        draft.advanced.window = "Blackman".into();
    }
    assert!(app
        .settings_editor
        .draft_mut()
        .unwrap()
        .save_and_apply(&path));
    assert!(app.settings_editor.draft().unwrap().save_error.is_none());
    render_app_ui(&mut app, &ctx, &mut frame);
    let mut close_input = egui::RawInput::default();
    close_input
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .events
        .push(egui::ViewportEvent::Close);
    ctx.begin_pass(close_input);
    app.logic(&ctx, &mut frame);
    let _ = ctx.end_pass();
    assert!(app.pending_settings_apply.is_none());
    assert!(app.shared.lock().unwrap().config.needs_apply);
    let saved_snapshot = app.exit_config_snapshot().unwrap();
    saved_snapshot.save_to(&path).unwrap();
    let reloaded: AppConfig = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(reloaded.default_sample_rate, 1_536_000);
    assert_eq!(reloaded.ppm_correction, 37);
    assert_eq!(reloaded.color_map, "Inferno");
    assert_eq!(reloaded.advanced.fft_size, 8192);
    assert_eq!(reloaded.advanced.window, "Blackman");
    assert_eq!(app.shared.lock().unwrap().source.sample_rate_hz, 2_400_000);
    assert_eq!(app.shared.lock().unwrap().source.ppm_correction, 3);
    assert_eq!(
        app.shared.lock().unwrap().config.default_sample_rate,
        1_536_000
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn mqtt_cadence_is_independent_of_scheduler_frames_and_settings_show_delivery_health() {
    let mut app = offline_test_app();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    app.mqtt.set_enabled(
        true,
        listener.local_addr().unwrap().to_string(),
        "audit".into(),
    );
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    for _ in 0..60 {
        app.last_scheduler_update = std::time::Instant::now();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            app.logic(ui.ctx(), &mut frame)
        });
    }
    assert_eq!(
        app.mqtt.stats().telemetry_sets,
        1,
        "scheduler frame window caused a publication burst"
    );
    app.mqtt
        .set_enabled(true, "127.0.0.1:0".into(), "audit".into());
    let mut text = String::new();
    for _ in 0..3 {
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
                )),
                ..Default::default()
            },
            |ui| {
                app.show_settings = true;
                app.ui(ui, &mut frame);
            },
        );
        for shape in output.shapes {
            if let egui::Shape::Text(shape) = shape.shape {
                text.push_str(shape.galley.text());
            }
        }
    }
    for expected in [
        "Telemetry sets: 1",
        "Pending:",
        "Coalesced:",
        "Backpressure:",
        "Expired:",
        "Dropped events:",
        "Rejected:",
        "valid broker host and port",
    ] {
        assert!(
            text.contains(expected),
            "Settings does not expose {expected}: {text}"
        );
    }
    app.mqtt.disconnect();
}

#[test]
fn applying_observer_settings_invalidates_fresh_pass_cache_and_updates_hidden_tracking() {
    let mut app = offline_test_app();
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    app.current_tab = AppTab::Planes;
    let before: Vec<_> = app
        .shared
        .lock()
        .unwrap()
        .tle
        .upcoming_passes()
        .iter()
        .map(|pass| (pass.satellite.clone(), pass.aos_dt, pass.los_dt))
        .collect();
    {
        let mut state = app.shared.lock().unwrap();
        state.config.observer_lat = -33.8688;
        state.config.observer_lon = 151.2093;
        state.config.needs_apply = true;
    }
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
        app.logic(ui.ctx(), &mut frame)
    });
    let after: Vec<_> = app
        .shared
        .lock()
        .unwrap()
        .tle
        .upcoming_passes()
        .iter()
        .map(|pass| (pass.satellite.clone(), pass.aos_dt, pass.los_dt))
        .collect();
    assert_ne!(before, after);
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
        app.logic(ui.ctx(), &mut frame)
    });
    assert_eq!(
        (
            app.satellite_panel.observer_lat,
            app.satellite_panel.observer_lon
        ),
        (-33.8688, 151.2093)
    );
    {
        let state = app.shared.lock().unwrap();
        assert_eq!(
            (state.tle.observer_lat, state.tle.observer_lon),
            (-33.8688, 151.2093)
        );
    }
    let mut text = String::new();
    let initial = ctx.run_ui(egui::RawInput::default(), |ui| {
        app.satellite_panel.ui_observer_location(ui)
    });
    let heading = initial
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text().contains("Observer Location") => {
                Some(text.pos + text.galley.rect.center().to_vec2())
            }
            _ => None,
        })
        .unwrap();
    for pressed in [true, false] {
        let _ = ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::PointerMoved(heading),
                    egui::Event::PointerButton {
                        pos: heading,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
                ..Default::default()
            },
            |ui| app.satellite_panel.ui_observer_location(ui),
        );
    }
    for _ in 0..3 {
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
                )),
                ..Default::default()
            },
            |ui| app.satellite_panel.ui_observer_location(ui),
        );
        for shape in output.shapes {
            if let egui::Shape::Text(shape) = shape.shape {
                text.push_str(shape.galley.text());
            }
        }
    }
    assert!(
        text.contains("-33.8688"),
        "observer latitude label is stale: {text}"
    );
    assert!(
        text.contains("151.2093"),
        "observer longitude label is stale: {text}"
    );
}

/// Exercises both real GUI command entry points against an isolated synthetic
/// daemon. This test never starts a physical audio device or external provider.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_squelch_spectrum_and_websocket_routes_apply_to_real_audio() {
    use crate::daemon_client::{ConnectionStatus, DaemonClient};
    use ez_proto::{ClientCommand, ServerEvent};
    use futures_util::{SinkExt, StreamExt};
    use std::time::{Duration, Instant};
    use tokio_tungstenite::tungstenite::Message;

    let daemon = crate::daemon_client::tests::TestDaemon::spawn("squelch-app").await;
    let mut app = tokio::task::block_in_place(offline_test_app);
    {
        let mut state = app.shared.lock().unwrap();
        state.source.source_mode = crate::source_manager::SourceMode::Daemon;
        state.source.daemon_addr = daemon.addr.to_string();
        state.source.center_frequency_hz = Some(100_000_000);
        // The synthetic daemon's modulated FM carrier is at -400 kHz. The
        // +250 kHz carrier is intentionally unmodulated and produces silence.
        state.source.frequency_hz = 99_600_000;
        state.source.sample_rate_hz = 2_000_000;
        state.demod_mode = crate::sdr_panel::DemodMode::Fm;
        state.audio_running = true;
        crate::radio_ui::set_power_squelch_preference(&mut state, -100.0);
        state.source.start();
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.drain_daemon_events();
        let ready = {
            let state = app.shared.lock().unwrap();
            state
                .source
                .daemon_squelch_applied()
                .is_some_and(|s| s.db == -100.0)
                && state.source.daemon_squelch_pending_db().is_none()
        };
        if ready {
            break;
        }
        if Instant::now() >= deadline {
            let state = app.shared.lock().unwrap();
            panic!(
                "initial daemon squelch was not applied: status={:?}, ids={:?}, pending={:?}, error={:?}",
                state.source.status,
                state.source.daemon_channel_ids(),
                state.source.daemon_squelch_pending_db(),
                state.source.daemon_squelch_error
            );
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // Use a second connection to observe real pipeline PCM without opening a speaker.
    let monitor = DaemonClient::connect(daemon.addr, "squelch-audio-observer".into());
    while monitor.status() != ConnectionStatus::Connected {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // Channel IDs are connection-owned; subscribe with the observer's assigned ID,
    // rather than reusing the GUI client's independent audio channel ID.
    let audio_id = loop {
        if let Some(channels) = monitor.client_channels() {
            break channels.audio;
        }
        assert!(
            Instant::now() < deadline,
            "observer did not receive channel IDs"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    monitor.send(ClientCommand::Subscribe {
        channel: ez_proto::ChannelSpec {
            audio: None,
            meteor_profile: None,
            id: audio_id,
            center_offset_hz: -400_000,
            channel_sample_rate_hz: 200_000,
            kind: ez_proto::PipelineKind::Audio,
            // A separate RAW monitor confirms nonzero IQ-derived PCM. The app's
            // FM path is covered by the daemon pipeline regression test.
            demod_mode: Some(ez_proto::DemodMode::Raw),
        },
    });
    async fn set_monitor_squelch(
        client: &DaemonClient,
        channel_id: u32,
        request_id: u64,
        db: f32,
    ) -> ez_proto::SquelchState {
        use ez_proto::ServerEvent;

        client.send(ClientCommand::SetSquelch {
            request_id,
            channel_id,
            db,
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(ServerEvent::Squelch {
                channel_id: reply_channel,
                request_id: reply_id,
                result,
            }) = client.try_recv_event()
            {
                if reply_channel == channel_id && reply_id == request_id {
                    let applied = result.expect("observer squelch request must apply");
                    assert_eq!(applied.db, db);
                    return applied;
                }
            }
            assert!(
                Instant::now() < deadline,
                "observer squelch was not applied"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    // The observer owns a separate audio pipeline, so mirror each requested threshold
    // on that pipeline before comparing its actual PCM with the GUI's acknowledged state.
    let initial_squelch = set_monitor_squelch(&monitor, audio_id, 100, -100.0).await;
    async fn audio_at(
        client: &DaemonClient,
        db: f32,
        min_revision: u64,
        require_audible: bool,
    ) -> ez_proto::AudioFrame {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut matching_audio_frames = 0;
        let mut audio_frames = 0;
        let mut last_matching_audio = None;
        let mut last_audio_summary = None;
        let mut other_events = 0;
        loop {
            match client.try_recv_event() {
                Some(ServerEvent::Audio(frame)) => {
                    audio_frames += 1;
                    let max_amplitude = frame
                        .samples
                        .iter()
                        .fold(0.0_f32, |largest, sample| largest.max(sample.abs()));
                    last_audio_summary = Some((
                        frame.channel_id,
                        frame.squelch.db,
                        frame.squelch.revision,
                        frame.samples.len(),
                        max_amplitude,
                    ));
                    if frame.squelch.db == db && frame.squelch.revision >= min_revision {
                        matching_audio_frames += 1;
                        last_matching_audio = Some(max_amplitude);
                        // A correctly revised frame can be silent during acquisition
                        // or after a stream gap. Opening still must produce actual
                        // nonzero PCM within the deadline, not merely an applied ack.
                        if !require_audible || last_matching_audio.unwrap() > 1e-5 {
                            return frame;
                        }
                    }
                }
                Some(ServerEvent::Error { message }) => {
                    panic!("daemon rejected the audio subscription: {message}")
                }
                Some(_) => other_events += 1,
                None => {}
            }
            assert!(
                Instant::now() < deadline,
                "no audible audio at applied squelch {db}; matching frames={matching_audio_frames}, all audio frames={audio_frames}, last matching max amplitude={last_matching_audio:?}, last audio summary={last_audio_summary:?}, other events={other_events}, connection={:?}",
                client.status()
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
    let open = audio_at(&monitor, -100.0, initial_squelch.revision, true).await;
    assert!(open.samples.iter().any(|s| s.abs() > 1e-5));
    let app_audio_id = app.shared.lock().unwrap().source.daemon_channel_ids().audio;
    let open_for_app = ez_proto::AudioFrame {
        channel_id: app_audio_id,
        squelch: app
            .shared
            .lock()
            .unwrap()
            .source
            .daemon_squelch_applied()
            .unwrap(),
        ..open.clone()
    };
    {
        let mut state = app.shared.lock().unwrap();
        state.spectrum.pending_squelch_db = Some(0.0);
    }
    let ctx = egui::Context::default();
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| app.render_sdr_tab(ui));
    {
        let state = app.shared.lock().unwrap();
        assert_eq!(state.source.daemon_squelch_pending_db(), Some(0.0));
        assert_eq!(
            state.squelch, -100.0,
            "request must not pretend to be applied"
        );
        assert_eq!(crate::radio_ui::requested_power_squelch(&state), 0.0);
        assert!(!state.source.accept_daemon_audio(&open_for_app));
    }
    let closed_squelch = set_monitor_squelch(&monitor, audio_id, 101, 0.0).await;
    let closed = audio_at(&monitor, 0.0, closed_squelch.revision, false).await;
    assert!(closed.squelch.revision > open.squelch.revision);
    assert!(closed.samples.iter().all(|s| *s == 0.0));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.drain_daemon_events();
        let ready = {
            let state = app.shared.lock().unwrap();
            let ready = state.squelch == 0.0 && state.source.daemon_squelch_pending_db().is_none();
            if ready {
                let closed_for_app = ez_proto::AudioFrame {
                    channel_id: app_audio_id,
                    squelch: state.source.daemon_squelch_applied().unwrap(),
                    ..closed.clone()
                };
                assert!(state.source.accept_daemon_audio(&closed_for_app));
                assert!(!state.source.accept_daemon_audio(&open_for_app));
            }
            ready
        };
        if ready {
            break;
        }
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    app.web_remote.set_enabled(true, port);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut ws = loop {
        if let Ok((ws, _)) =
            tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws")).await
        {
            break ws;
        }
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    ws.send(Message::Text(
        r#"{"action":"set_squelch","db":-100.0}"#.into(),
    ))
    .await
    .unwrap();
    loop {
        app.process_web_remote_commands();
        if app
            .shared
            .lock()
            .unwrap()
            .source
            .daemon_squelch_pending_db()
            == Some(-100.0)
        {
            break;
        }
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    {
        let state = app.shared.lock().unwrap();
        assert_eq!(state.squelch, 0.0);
        app.web_remote
            .broadcast_state(&crate::web_remote::StreamState::from_receiver(
                &state,
                0,
                &[],
                false,
                0.0,
            ));
    }
    let pending = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .into_text()
        .unwrap();
    let pending: serde_json::Value = serde_json::from_str(&pending).unwrap();
    assert_eq!(pending["squelch_db"], 0.0);
    assert_eq!(pending["squelch_pending_db"], -100.0);
    assert_eq!(pending["squelch_applied"], true);
    // Leave the GUI event loop idle beyond the three-second ACK deadline.
    // The daemon reply is prioritized in the control queue and must be
    // applied before the next timeout check when the GUI resumes.
    tokio::time::sleep(Duration::from_secs(4)).await;
    let reopened_squelch = set_monitor_squelch(&monitor, audio_id, 102, -100.0).await;
    let reopened = audio_at(&monitor, -100.0, reopened_squelch.revision, true).await;
    assert!(reopened.squelch.revision > closed.squelch.revision);
    assert!(reopened.samples.iter().any(|s| s.abs() > 1e-5));
    loop {
        app.drain_daemon_events();
        if app
            .shared
            .lock()
            .unwrap()
            .source
            .daemon_squelch_pending_db()
            .is_none()
        {
            break;
        }
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    {
        let state = app.shared.lock().unwrap();
        assert_eq!(state.squelch, -100.0);
        let reopened_for_app = ez_proto::AudioFrame {
            channel_id: app_audio_id,
            squelch: state.source.daemon_squelch_applied().unwrap(),
            ..reopened.clone()
        };
        let closed_for_app = ez_proto::AudioFrame {
            channel_id: app_audio_id,
            squelch: closed.squelch,
            ..closed.clone()
        };
        assert!(state.source.accept_daemon_audio(&reopened_for_app));
        assert!(!state.source.accept_daemon_audio(&closed_for_app));
    }
    {
        let state = app.shared.lock().unwrap();
        app.web_remote
            .broadcast_state(&crate::web_remote::StreamState::from_receiver(
                &state,
                0,
                &[],
                false,
                0.0,
            ));
    }
    let applied = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .into_text()
        .unwrap();
    let applied: serde_json::Value = serde_json::from_str(&applied).unwrap();
    assert_eq!(applied["squelch_db"], -100.0);
    assert_eq!(applied["squelch_pending_db"], serde_json::Value::Null);
    assert_eq!(applied["squelch_error"], serde_json::Value::Null);
    assert_eq!(applied["squelch_applied"], true);
    // A WebSocket request with no live audio channel reports a command error
    // and leaves both the preference and the last applied threshold intact.
    app.shared.lock().unwrap().source.sync_daemon_workflows(
        false,
        false,
        false,
        crate::sdr_panel::DemodMode::Fm,
    );
    ws.send(Message::Text(
        r#"{"action":"set_squelch","db":-30.0}"#.into(),
    ))
    .await
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        app.process_web_remote_commands();
        if app
            .shared
            .lock()
            .unwrap()
            .source
            .daemon_squelch_error
            .is_some()
        {
            break;
        }
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let failure = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .into_text()
        .unwrap();
    let failure: serde_json::Value = serde_json::from_str(&failure).unwrap();
    assert_eq!(failure["type"], "command_error");
    assert_eq!(failure["action"], "set_squelch");
    assert!(failure["message"].as_str().unwrap().contains("not active"));
    assert_eq!(
        crate::radio_ui::requested_power_squelch(&app.shared.lock().unwrap()),
        -100.0
    );
    ws.close(None).await.unwrap();
    drop(ws);
    app.web_remote.stop();
    app.shared.lock().unwrap().source.stop();
    drop(monitor);
    daemon.shutdown().await;
}

fn imported_receiver_profile() -> AppConfig {
    let mut config = AppConfig::default();
    config.mqtt_broker.clear();
    config.default_freq_hz = 144_000_000;
    config.default_sample_rate = 2_400_000;
    config.default_gain = 32.8;
    config.ppm_correction = -37;
    config.lo_offset_hz = -125_000;
    config.color_map = "Inferno".into();
    config.wf_min_db = -97.0;
    config.wf_max_db = -17.0;
    let a = &mut config.advanced;
    a.rtl_device = crate::source_manager::RtlDeviceSelection {
        index: 3,
        serial: Some("PROFILE-DEVICE".into()),
    };
    a.tuner_agc = true;
    a.rtl_agc = true;
    a.bias_tee = true;
    a.direct_sampling = true;
    a.direct_sampling_branch = crate::source_manager::DirectSamplingBranch::Q;
    a.offset_tuning = false;
    a.fft_size = 8192;
    a.fft_rate = 17;
    a.window = "Blackman".into();
    a.waterfall_visible = false;
    a.wf_depth = 333;
    a.wf_speed = 7;
    a.full_waterfall_update = false;
    a.grid = false;
    a.peak_hold_time = 2.5;
    a.avg_alpha = 0.25;
    a.smoothing_enabled = false;
    a.fast_fft = true;
    a.snr_smoothing = false;
    a.snr_smoothing_secs = 1.7;
    a.persistence = 0.6;
    a.gradient_fill = false;
    a.db_min = -103.0;
    a.db_max = -13.0;
    a.radio_bandwidth_hz = 18_000.0;
    a.radio_snap_hz = 1250;
    a.audio_cutoff_hz = 3500.0;
    a.rf_decim = 4;
    a.rf_dc_remove = true;
    a.invert_iq = true;
    a.rf_noise_blanker = true;
    a.rf_noise_blanker_level = 4.0;
    a.squelch_mode = crate::radio_squelch::SquelchMode::Power;
    a.squelch_level_db = -43.0;
    a.audio_gain = 0.0;
    config
}

fn assert_imported_receiver_profile(app: &CentralApp) {
    let state = app.shared.lock().unwrap();
    let open = state.source.rtl_open_settings().unwrap();
    assert_eq!(open.device.index, 3);
    assert_eq!(open.device.serial.as_deref(), Some("PROFILE-DEVICE"));
    assert_eq!(
        open.frequency_hz,
        state.source.capture_center_frequency_hz() as u32 - 125_000
    );
    assert_eq!(open.sample_rate_hz, state.source.sample_rate_hz);
    assert_eq!(open.gain_db, state.source.gain_db);
    assert_eq!(open.ppm, -37);
    assert!(open.bias_tee && open.tuner_agc && open.rtl_agc);
    assert_eq!(open.direct_sampling_mode, 2);
    assert!(!open.offset_tuning);
    assert_eq!(state.lo_offset_hz, -125_000);
    let d = state.spectrum.display_settings();
    assert_eq!(d.fft_size, 8192);
    assert_eq!(d.fft_rate, 17);
    assert_eq!(d.window, crate::spectrum::WindowType::Blackman);
    assert_eq!((d.waterfall_history, d.waterfall_every_n), (333, 7));
    assert!(!d.waterfall_visible && !d.full_waterfall_update && !d.grid && !d.gradient_fill);
    assert!(!d.smoothing_enabled && !d.snr_smoothing);
    assert!(d.fast_fft);
    assert_eq!(
        (
            d.peak_hold_time,
            d.avg_alpha,
            d.smoothing_speed,
            d.persistence,
            d.snr_smoothing_secs
        ),
        (2.5, 0.25, 0.75, 0.6, 1.7)
    );
    assert_eq!(
        (d.db_min, d.db_max, d.wf_min_db, d.wf_max_db),
        (-103.0, -13.0, -97.0, -17.0)
    );
    assert_eq!(d.color_map, crate::spectrum::ColorMap::Inferno);
    assert_eq!(state.lpf_cutoff, 3500.0);
    assert_eq!(state.tune_step_fine_hz, 1250);
    assert_eq!(crate::radio_ui::channel_bandwidth(&state), 18_000.0);
    assert_eq!(state.squelch, -43.0);
    let iq = crate::radio_ui::radio_iq_config(&state);
    assert_eq!(iq.decimation, 4);
    assert!(iq.dc_remove && iq.invert);
    assert!(state.config.advanced.rf_noise_blanker);
    assert_eq!(state.config.advanced.audio_gain, 0.0);
    assert_eq!(state.config.advanced.fft_hold, 2.5);
    assert_eq!(state.config.advanced.smoothing_speed, 0.75);
}

fn wait_for_saved_dsp_profile(app: &CentralApp) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let requested = app.demod_worker.requested_version();
    while app.demod_worker.applied_version() < requested {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    // The loaded mute gain must really reach the idle worker and its next PCM,
    // rather than only change a displayed config field.
    let tone = (0..16_384)
        .map(|i| num_complex::Complex32::from_polar(0.7, i as f32 * 0.05))
        .collect();
    assert!(app.demod_worker.push_iq(tone));
    loop {
        if let Some(frame) = app.demod_worker.try_recv_frame() {
            assert!(!frame.audio.is_empty());
            assert!(frame.audio.iter().all(|s| *s == 0.0));
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[test]
fn saved_receiver_profile_restores_before_first_render_and_round_trips() {
    let config = imported_receiver_profile();
    let encoded = serde_json::to_vec(&config).unwrap();
    let loaded: AppConfig = serde_json::from_slice(&encoded).unwrap();
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = CentralApp::new_with_config(&cc, loaded);
    app.demod_worker = crate::demod::DemodWorker::new(16);
    app.apply_advanced();
    assert_imported_receiver_profile(&app);
    assert_eq!(
        app.shared.lock().unwrap().source.status,
        crate::source_manager::SourceStatus::Idle
    );
    assert_eq!(app.shared.lock().unwrap().source.stream_generation(), 0);
    let before = app.shared.lock().unwrap().config.advanced.clone();
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| app.render_sdr_tab(ui));
    assert_imported_receiver_profile(&app);
    assert_eq!(app.shared.lock().unwrap().config.advanced, before);
    wait_for_saved_dsp_profile(&app);
    let saved = serde_json::to_vec(&app.shared.lock().unwrap().config).unwrap();
    let reloaded = CentralApp::new_with_config(&cc, serde_json::from_slice(&saved).unwrap());
    assert_imported_receiver_profile(&reloaded);
}

#[test]
fn saved_receiver_profile_import_applies_calibration_and_only_changed_hardware_restarts() {
    let ctx = egui::Context::default();
    let mut app = offline_test_app();
    app.demod_worker = crate::demod::DemodWorker::new(16);
    let mut frame = eframe::Frame::_new_kittest();
    {
        let mut state = app.shared.lock().unwrap();
        state.source.source_mode = crate::source_manager::SourceMode::Hardware;
        state.source.frequency_hz = 1_090_000_000;
        state.source.center_frequency_hz = Some(1_090_000_000);
        state.source.sample_rate_hz = 2_400_000;
        state.source.gain_db = 32.8;
        state.source.status = crate::source_manager::SourceStatus::Running;
        state.config = imported_receiver_profile();
        state.config.needs_apply = true;
    }
    // Unit-test builds reject physical hardware opening. Assert the exact
    // production request and restart attempt without touching a USB device.
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
        app.logic(ui.ctx(), &mut frame)
    });
    assert_imported_receiver_profile(&app);
    let generation = app.shared.lock().unwrap().source.stream_generation();
    assert_eq!(generation, 1);
    assert_eq!(
        app.shared.lock().unwrap().source.frequency_hz,
        1_090_000_000
    );
    assert!(!app.shared.lock().unwrap().config.needs_apply);
    wait_for_saved_dsp_profile(&app);
    {
        let mut state = app.shared.lock().unwrap();
        state.config.needs_apply = true;
        state.config.font_scale = 1.2;
    }
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
        app.logic(ui.ctx(), &mut frame)
    });
    assert_imported_receiver_profile(&app);
    assert_eq!(
        app.shared.lock().unwrap().source.stream_generation(),
        generation
    );
    {
        let mut state = app.shared.lock().unwrap();
        // Model an active device for the next application; the previous
        // test-build open can already have returned its unsupported error.
        state.source.status = crate::source_manager::SourceStatus::Running;
        state.config.advanced.direct_sampling = false;
        state.config.advanced.offset_tuning = true;
        state.config.ppm_correction = 23;
        state.config.lo_offset_hz = 250_000;
        state.config.needs_apply = true;
    }
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
        app.logic(ui.ctx(), &mut frame)
    });
    {
        let mut state = app.shared.lock().unwrap();
        let open = state.source.rtl_open_settings().unwrap();
        assert_eq!(
            (
                open.ppm,
                open.frequency_hz,
                open.direct_sampling_mode,
                open.offset_tuning
            ),
            (23, 1_090_250_000, 0, true)
        );
        assert_eq!(state.source.stream_generation(), generation + 1);
        state.source.stop();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_satellite_passes_honor_gui_web_rigctl_and_scanner_manual_cooldown() {
    use crate::tle_engine::PassInfo;
    use futures_util::SinkExt;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    use tokio_tungstenite::tungstenite::Message;
    let utc = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    for route in ["GUI", "web", "rigctl", "scanner"] {
        let mut app = tokio::task::block_in_place(offline_test_app);
        app.current_tab = AppTab::Planes;
        let pass = |aos, los| PassInfo {
            satellite: "ISS".into(),
            aos: "AOS".into(),
            los: "LOS".into(),
            frequency_hz: 145_800_000,
            aos_dt: aos,
            los_dt: los,
            max_elevation: 45.0,
        };
        let first_id;
        let second_id;
        {
            let mut state = app.shared.lock().unwrap();
            state.source.stop();
            state.scheduler.update_from_passes(&[
                pass(utc - 120.0, utc - 60.0),
                pass(utc + 100.0, utc + 400.0),
            ]);
            first_id = state.scheduler.jobs[0].id;
            second_id = state.scheduler.jobs[1].id;
        }
        app.tick_satellite_jobs(utc - 90.0, Instant::now());
        assert_eq!(
            app.shared.lock().unwrap().source.frequency_hz,
            145_800_000,
            "first {route}"
        );
        assert_eq!(app.last_auto_tuned_job.unwrap().0, first_id);
        assert!(
            app.shared.lock().unwrap().last_manual_tune.is_none(),
            "scheduled tune caused manual cooldown"
        );
        // A forecast refreshed during this pass must not tune a second time.
        app.tick_satellite_jobs(utc - 89.0, Instant::now());
        assert_eq!(app.last_auto_tuned_job.unwrap().0, first_id);
        app.tick_satellite_jobs(utc, Instant::now());
        assert!(
            app.last_auto_tuned_job.is_none(),
            "LOS did not retire pass identity"
        );
        match route {
            "GUI" => {
                let ctx = egui::Context::default();
                let mut frame = eframe::Frame::_new_kittest();
                app.last_scheduler_update = Instant::now();
                let _ = ctx.run_ui(
                    egui::RawInput {
                        events: vec![egui::Event::Key {
                            key: egui::Key::ArrowRight,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::default(),
                        }],
                        ..Default::default()
                    },
                    |ui| app.logic(ui.ctx(), &mut frame),
                );
            }
            "web" => {
                let port = std::net::TcpListener::bind("127.0.0.1:0")
                    .unwrap()
                    .local_addr()
                    .unwrap()
                    .port();
                app.web_remote.set_enabled(true, port);
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut ws = loop {
                    if let Ok((ws, _)) =
                        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws")).await
                    {
                        break ws;
                    }
                    assert!(Instant::now() < deadline);
                    tokio::time::sleep(Duration::from_millis(5)).await;
                };
                ws.send(Message::Text(r#"{"action":"tune","hz":144000000}"#.into()))
                    .await
                    .unwrap();
                loop {
                    app.process_web_remote_commands();
                    if app.shared.lock().unwrap().source.frequency_hz == 144_000_000 {
                        break;
                    }
                    assert!(Instant::now() < deadline, "web tune not dispatched");
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                drop(ws);
            }
            "rigctl" => {
                app.rigctl.set_enabled(true, 0);
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut socket = loop {
                    let port = app.rigctl.bound_port();
                    if port != 0 {
                        if let Ok(socket) =
                            tokio::net::TcpStream::connect(("127.0.0.1", port)).await
                        {
                            break socket;
                        }
                    }
                    assert!(
                        Instant::now() < deadline,
                        "rigctl listener never became ready"
                    );
                    tokio::time::sleep(Duration::from_millis(5)).await;
                };
                socket.write_all(b"F 144000000\n").await.unwrap();
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    app.process_rigctl_requests();
                    if app.shared.lock().unwrap().source.frequency_hz == 144_000_000 {
                        break;
                    }
                    assert!(Instant::now() < deadline);
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                let mut line = String::new();
                tokio::io::BufReader::new(socket)
                    .read_line(&mut line)
                    .await
                    .unwrap();
                assert_eq!(line, "RPRT 0\n");
            }
            "scanner" => {
                app.scanner.start_hz = 144_000_000;
                app.scanner.stop_hz = 144_100_000;
                app.scanner.start();
                app.apply_scanner_tune();
                assert_eq!(app.shared.lock().unwrap().source.frequency_hz, 144_000_000);
            }
            _ => unreachable!(),
        }
        let state = app.shared.lock().unwrap();
        let manual_frequency = state.source.frequency_hz;
        let manual_at = state
            .last_manual_tune
            .expect("route did not use shared manual tune command");
        assert_ne!(manual_frequency, 145_800_000, "{route} didn't tune");
        drop(state);
        app.tick_satellite_jobs(utc + 150.0, manual_at + Duration::from_secs(14));
        assert_eq!(
            app.shared.lock().unwrap().source.frequency_hz,
            manual_frequency,
            "cooldown ignored for {route}"
        );
        assert!(app.last_auto_tuned_job.is_none());
        app.tick_satellite_jobs(utc + 152.0, manual_at + Duration::from_secs(16));
        let applied = app
            .last_auto_tuned_job
            .expect("later pass of same satellite was skipped");
        assert_eq!((applied.0, applied.1), (second_id, 145_800_000));
        assert_eq!(app.shared.lock().unwrap().last_manual_tune, Some(manual_at));
        // A manual change later in this handled pass stays in place, including
        // after its cooldown: the automatic job executes once for this pass.
        {
            let mut state = app.shared.lock().unwrap();
            crate::radio_ui::tune(&mut state, manual_frequency);
        }
        app.tick_satellite_jobs(utc + 200.0, manual_at + Duration::from_secs(100));
        assert_eq!(
            app.shared.lock().unwrap().source.frequency_hz,
            manual_frequency
        );
        assert_eq!(app.last_auto_tuned_job, Some(applied));
        let last_manual = app.shared.lock().unwrap().last_manual_tune;
        app.handle_events();
        let state = app.shared.lock().unwrap();
        assert_eq!(
            state.source.frequency_hz, manual_frequency,
            "old frequency notification overwrote {route}"
        );
        assert_eq!(state.last_manual_tune, last_manual);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unrelated_settings_apply_preserves_web_remote_and_rigctl_workers() {
    use futures_util::StreamExt;

    let mut app = tokio::task::block_in_place(offline_test_app);
    let free_port = || {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        port
    };
    let web_port = free_port();
    let rigctl_port = free_port();
    app.web_remote.set_enabled(true, web_port);
    app.rigctl.set_enabled(true, rigctl_port);

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let web = app.web_remote.status_snapshot();
        let rigctl = app.rigctl.status_snapshot();
        if matches!(web.state, crate::web_remote::ListenerState::Listening(_))
            && matches!(rigctl.state, crate::web_remote::ListenerState::Listening(_))
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "integration listeners did not start: {web:?}, {rigctl:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{web_port}/ws"))
        .await
        .expect("connect WebSocket before config apply");
    let socket_deadline = Instant::now() + Duration::from_secs(1);
    while app
        .web_remote
        .tx
        .as_ref()
        .is_none_or(|tx| tx.receiver_count() == 0)
        && Instant::now() < socket_deadline
    {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(app
        .web_remote
        .tx
        .as_ref()
        .is_some_and(|tx| tx.receiver_count() > 0));
    {
        let mut state = app.shared.lock().unwrap();
        state.config.web_remote_enabled = true;
        state.config.web_remote_port = web_port;
        state.config.rigctl_enabled = true;
        state.config.rigctl_port = rigctl_port;
        state.config.needs_apply = true;
    }
    let web_generation = app.web_remote.status_snapshot().generation;
    let rigctl_generation = app.rigctl.status_snapshot().generation;
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    render_app_logic(&mut app, &ctx, &mut frame);
    assert_eq!(app.web_remote.status_snapshot().generation, web_generation);
    assert_eq!(app.rigctl.status_snapshot().generation, rigctl_generation);

    {
        let mut state = app.shared.lock().unwrap();
        state.config.observer_lat = 47.5;
        state.config.observer_lon = 19.0;
        state.config.ppm_correction = 37;
        state.config.font_scale = 1.1;
        state.config.needs_apply = true;
    }
    render_app_logic(&mut app, &ctx, &mut frame);
    assert_eq!(app.web_remote.status_snapshot().generation, web_generation);
    assert_eq!(app.rigctl.status_snapshot().generation, rigctl_generation);

    {
        let state = app.shared.lock().unwrap();
        app.web_remote
            .broadcast_state(&crate::web_remote::StreamState::from_receiver(
                &state,
                0,
                &[],
                false,
                0.0,
            ));
    }
    let message = tokio::time::timeout(Duration::from_secs(1), socket.next())
        .await
        .expect("existing WebSocket should receive state after apply")
        .expect("WebSocket should remain open")
        .expect("read state frame");
    assert!(message.into_text().unwrap().contains("frequency_hz"));

    socket.close(None).await.unwrap();
    app.web_remote.stop();
    app.rigctl.stop();
}

#[test]
fn occupied_web_remote_port_is_visible_in_settings_and_status_surface() {
    let blocker = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = blocker.local_addr().unwrap().port();
    let ctx = egui::Context::default();
    let mut app = offline_test_app();
    let mut frame = eframe::Frame::_new_kittest();
    {
        let mut state = app.shared.lock().unwrap();
        state.config.web_remote_enabled = true;
        state.config.web_remote_port = port;
        state.config.needs_apply = true;
    }
    render_app_logic(&mut app, &ctx, &mut frame);

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let snapshot = app.web_remote.status_snapshot();
        if matches!(snapshot.state, crate::web_remote::ListenerState::Failed(_)) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "listener did not report bind failure: {snapshot:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    render_app_logic(&mut app, &ctx, &mut frame);
    let message = app
        .status_bar
        .current()
        .expect("failure should reach status surface");
    assert!(message.text.contains("Web Remote listener failed"));
    assert!(message.text.contains(&port.to_string()));

    let web_status = app.web_remote.status_snapshot();
    let rigctl_status = app.rigctl.status_snapshot();
    let mut editor = crate::config::SettingsEditor::default();
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900.0, 1000.0),
        )),
        ..Default::default()
    };
    let output = ctx.run_ui(input, |ui| {
        app.status_bar.render_strip(ui, &app.shared);
        app.status_bar.render_alert(ui);
        crate::secondary_panel::render_settings(
            &app.shared,
            &app.mqtt,
            &web_status,
            &rigctl_status,
            &mut editor,
            std::path::Path::new("unused-test-config.json"),
            ui,
        );
    });
    let text = rendered_text(&output);
    assert!(text.contains("Web Remote actual status"), "{text}");
    assert!(text.contains("Failed"), "{text}");
    assert!(text.contains(&port.to_string()), "{text}");
    assert!(text.contains("Web Remote listener failed"), "{text}");
}

#[test]
fn shared_failure_surface_renders_on_radio_adsb_and_meteor() {
    let mut app = offline_test_app();
    app.status_bar
        .error("Audio output unavailable · Daemon control failed".into());
    app.recorder_panel
        .test_inject_failure("Recording finalization failed");
    {
        let mut state = app.shared.lock().unwrap();
        state.config.save_error = Some("Could not save settings to test path".into());
        state.source.status =
            crate::source_manager::SourceStatus::Error("daemon connection lost".into());
    }
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    for tab in [AppTab::Listen, AppTab::Planes, AppTab::Meteor] {
        app.current_tab = tab;
        let output = render_app_ui(&mut app, &ctx, &mut frame);
        let text = rendered_text(&output);
        assert!(text.contains("Audio output unavailable"), "{tab:?}: {text}");
        assert!(
            text.contains("Recording finalization failed"),
            "{tab:?}: {text}"
        );
        assert!(text.contains("Could not save settings"), "{tab:?}: {text}");
        assert!(text.contains("daemon connection lost"), "{tab:?}: {text}");
        if tab == AppTab::Planes {
            assert!(text.contains("Mode S · local decoder"), "{text}");
            assert!(text.contains("0 aircraft"), "{text}");
            assert!(text.contains("© OpenStreetMap contributors"), "{text}");
        }
    }
}

#[test]
fn scheduled_tune_waits_for_actual_capture_and_uses_one_enable_state() {
    use ez_proto::{HardwareStatus, ServerEvent};
    let mut app = offline_test_app();
    let (client, _commands, events) = crate::daemon_client::DaemonClient::test_connection();
    // A fake adapter exposes a real control request before its authoritative
    // hardware acknowledgement; the scheduler must not report it as completed.
    {
        let mut state = app.shared.lock().unwrap();
        state.source.source_mode = crate::source_manager::SourceMode::Daemon;
        state.source.set_test_daemon_client(client);
        state.source.frequency_hz = 100_000_000;
        state.source.center_frequency_hz = Some(100_000_000);
        state
            .scheduler
            .update_from_passes(&[crate::tle_engine::PassInfo {
                satellite: "ISS".into(),
                aos: "AOS".into(),
                los: "LOS".into(),
                frequency_hz: 145_800_000,
                aos_dt: 100.0,
                los_dt: 200.0,
                max_elevation: 45.0,
            }]);
        state.source.status = crate::source_manager::SourceStatus::Running;
    }
    app.tick_satellite_jobs(150.0, Instant::now());
    assert!(app.last_auto_tuned_job.is_none());
    assert!(app
        .satellite_panel
        .auto_tune_status
        .as_ref()
        .unwrap()
        .contains("acknowledgement"));
    events
        .send(ServerEvent::Hardware(HardwareStatus {
            connected: true,
            frequency_hz: 145_800_000,
            sample_rate_hz: 2_048_000,
            gain_db: 20.0,
            requested_gain_db: None,
            gain_mode: None,
            source_kind: "fake".into(),
            error: None,
        }))
        .unwrap();
    {
        let mut state = app.shared.lock().unwrap();
        let _ = state.source.recv_daemon_event();
    }
    app.tick_satellite_jobs(151.0, Instant::now());
    assert!(app.last_auto_tuned_job.is_some());
    {
        let mut state = app.shared.lock().unwrap();
        state.scheduler.auto_tune_enabled = false;
    }
    app.tick_satellite_jobs(152.0, Instant::now());
    assert!(app.last_auto_tuned_job.is_none());
    assert!(app.satellite_panel.auto_tune_status.is_none());
    // The actual satellite UI binds its checkbox to Scheduler, not a shadow.
    let ctx = egui::Context::default();
    let mut texts = String::new();
    let mut checkbox = None;
    for _ in 0..2 {
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            crate::satellite_tab::render_satellite_tab(
                ui,
                &app.shared,
                &mut app.satellite_panel,
                &mut app.satellite_subtab,
                &mut app.constellation,
                &mut app.decoding_panel,
            )
        });
        for shape in output.shapes {
            if let egui::Shape::Text(t) = shape.shape {
                if t.galley.text() == "Auto-tune enabled" {
                    checkbox = Some(t.pos + t.galley.size() / 2.0);
                }
                texts.push_str(t.galley.text());
            }
        }
    }
    assert!(texts.contains("Auto-tune enabled"));
    assert!(!app.shared.lock().unwrap().scheduler.auto_tune_enabled);
    let position = checkbox.expect("checkbox label was not painted");
    for pressed in [true, false] {
        let _ = ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::default(),
                    },
                ],
                ..Default::default()
            },
            |ui| {
                crate::satellite_tab::render_satellite_tab(
                    ui,
                    &app.shared,
                    &mut app.satellite_panel,
                    &mut app.satellite_subtab,
                    &mut app.constellation,
                    &mut app.decoding_panel,
                )
            },
        );
    }
    assert!(
        app.shared.lock().unwrap().scheduler.auto_tune_enabled,
        "sidebar toggle did not update scheduler"
    );
    app.shared.lock().unwrap().source.status =
        crate::source_manager::SourceStatus::Error("hardware rejected tuning".into());
    app.tick_satellite_jobs(153.0, Instant::now());
    assert!(app.last_auto_tuned_job.is_none());
    assert!(app
        .satellite_panel
        .auto_tune_status
        .as_ref()
        .unwrap()
        .contains("failed"));
    {
        let mut state = app.shared.lock().unwrap();
        state.source.source_mode = crate::source_manager::SourceMode::Replay;
        state.source.status = crate::source_manager::SourceStatus::Idle;
        state.source.center_frequency_hz = Some(100_000_000);
    }
    app.tick_satellite_jobs(154.0, Instant::now());
    assert!(
        app.last_auto_tuned_job.is_none(),
        "unavailable replay frequency was claimed as applied"
    );
    assert!(app
        .satellite_panel
        .auto_tune_status
        .as_ref()
        .unwrap()
        .contains("outside"));
}

#[path = "app_runtime_action_tests.rs"]
mod runtime_action_tests;

#[test]
fn hidden_local_capture_epoch_retires_state_but_forward_gap_keeps_fresh_trails() {
    use crate::adsb_decoder::capture_tests::position_messages;
    use ez_daemon::hardware::adsb::encode_uc8;
    let mut app = offline_test_app();
    app.current_tab = AppTab::Meteor;
    let [odd, even] = position_messages(0x3c4b12);
    let raw = encode_uc8(&[odd, even]);
    let single = encode_uc8(&[odd]);
    let clock = ez_proto::CaptureClock::new(401, 0, 2_400_000, None);
    app.feed_local_adsb_capture(&raw, clock, dump1090::demod::InputFormat::Uc8);
    app.adsb_panel.tick_model();
    assert_eq!(app.adsb_panel.trail_len(0x3c4b12), 1);
    app.seen_aircraft.insert(0x3c4b12);
    app.notified_aircraft.insert(0x3c4b12);
    let mut rewind = clock;
    rewind.epoch = 402;
    rewind.discontinuous = true;
    app.feed_local_adsb_capture(&single, rewind, dump1090::demod::InputFormat::Uc8);
    app.adsb_panel.tick_model();
    assert!(!app.adsb_panel.aircraft[0].has_position);
    assert_eq!(app.adsb_panel.trail_len(0x3c4b12), 0);
    assert!(app.seen_aircraft.is_empty());
    assert!(app.notified_aircraft.is_empty());
    assert_eq!(app.adsb_decoder.total_messages, 1);
    let mut next = rewind.offset_samples(single.len() as u64 / 2).unwrap();
    next.discontinuous = false;
    app.feed_local_adsb_capture(&raw, next, dump1090::demod::InputFormat::Uc8);
    app.adsb_panel.tick_model();
    assert_eq!(app.adsb_panel.trail_len(0x3c4b12), 1);
    let mut gap = next.offset_samples(raw.len() as u64 / 2 + 1).unwrap();
    gap.discontinuous = true;
    app.feed_local_adsb_capture(&single, gap, dump1090::demod::InputFormat::Uc8);
    app.adsb_panel.tick_model();
    assert!(app.adsb_panel.aircraft[0].has_position);
    assert!(app.adsb_panel.trail_len(0x3c4b12) > 0);
}

#[test]
fn continuous_adsb_keeps_up_with_60_hz_app_and_a_held_shared_state_lock() {
    use crate::source_manager::SourceMode;
    let mut app = offline_test_app();
    app.current_tab = AppTab::Meteor;
    let block = crate::local_adsb::tests::crossing_block();
    let stream = {
        let mut state = app.shared.lock().unwrap();
        state.adsb_running = true;
        state.source.source_mode = SourceMode::Hardware;
        state.source.frequency_hz = 1_090_000_000;
        state.source.sample_rate_hz = 2_400_000;
        state
            .source
            .test_set_hardware_open(move |_| Ok(block.clone()));
        state.source.capture_stream()
    };
    app.sync_adsb_capture();
    app.shared.lock().unwrap().source.start();
    let deadline = Instant::now() + Duration::from_secs(5);
    while stream.progress().samples == 0 {
        assert!(Instant::now() < deadline, "capture source did not start");
        std::thread::sleep(Duration::from_millis(1));
    }
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    let started = Instant::now();
    for tick in 1..=60 {
        app.logic(&ctx, &mut frame);
        let next = started + Duration::from_secs_f64(f64::from(tick) / 60.0);
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
    }
    let before = app.local_adsb.snapshot().unwrap().received_samples;
    {
        let mut state = app.shared.lock().unwrap();
        // The capture actor never takes this UI/model mutex.
        std::thread::sleep(Duration::from_millis(200));
        assert!(app.local_adsb.snapshot().unwrap().received_samples > before + 300_000);
        state.source.stop();
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    let snapshot = loop {
        if let Some(snapshot) = app
            .local_adsb
            .snapshot()
            .filter(|snapshot| snapshot.complete)
        {
            break snapshot;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    };
    let progress = stream.progress();
    assert!(progress.samples >= 2_000_000);
    assert_eq!(snapshot.received_samples, progress.samples);
    assert_eq!(snapshot.lost_samples, 0);
    assert_eq!(snapshot.dropped_blocks, 0);
    assert_eq!(snapshot.receiver.total_messages, progress.blocks - 1);
    app.logic(&ctx, &mut frame);
    assert_eq!(
        app.adsb_panel.total_messages,
        snapshot.receiver.total_messages
    );
    assert_eq!(app.adsb_panel.aircraft.len(), 1);
    println!(
        "A09 real app at 60 Hz plus 200 ms shared-lock stall: {} samples, {} cross-block messages, zero lost samples",
        progress.samples, snapshot.receiver.total_messages,
    );
}

#[test]
fn native_adsb_coordinates_reach_map_after_callback_loss_and_weak_frames() {
    use crate::adsb_decoder::capture_tests::position_messages;
    use crate::source_manager::{CaptureBlock, SourceMode};
    use ez_daemon::hardware::adsb::{encode_uc8, EXAMPLE_DF17_CALLSIGN};
    use ez_proto::CaptureClock;

    let mut app = offline_test_app();
    let stream = {
        let mut state = app.shared.lock().unwrap();
        state.adsb_running = true;
        state.source.source_mode = SourceMode::Hardware;
        state.source.frequency_hz = 1_090_000_000;
        state.source.sample_rate_hz = 2_400_000;
        state.config.observer_lat = 52.2572;
        state.config.observer_lon = 8.6676;
        // Allocate a real source generation, then inject acquisition-clocked
        // blocks through its native bus without using a physical SDR.
        state
            .source
            .test_set_hardware_open(|_| Err("manual ADS-B capture fixture".into()));
        state.source.start();
        state.source.wait_for_shutdown();
        state.source.capture_stream()
    };
    app.sync_adsb_capture();
    let generation = app.shared.lock().unwrap().source.stream_generation();
    let publish = |samples: Vec<u8>, clock: CaptureClock, stream_sample: u64| {
        let count = samples.len() as u64 / 2;
        let mut block = CaptureBlock::fixture(samples, clock, generation, stream_sample);
        block.session = Some(stream.session());
        stream.publish(&block);
        count
    };
    let wait = |app: &CentralApp, received: u64| {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(snapshot) = app
                .local_adsb
                .snapshot()
                .filter(|s| s.received_samples >= received)
            {
                return snapshot;
            }
            assert!(Instant::now() < deadline, "native ADS-B snapshot stalled");
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    let [odd, even] = position_messages(0x4840d6);
    let first = encode_uc8(&[EXAMPLE_DF17_CALLSIGN, odd, even]);
    let clock = CaptureClock::new(701, 0, 2_400_000, None);
    let mut delivered = publish(first, clock, 0);
    wait(&app, delivered);
    app.apply_local_adsb_snapshot();
    app.adsb_panel.tick_model();
    assert!(app.adsb_panel.aircraft[0].position_valid());
    let original = (
        app.adsb_panel.aircraft[0].lat,
        app.adsb_panel.aircraft[0].lon,
    );
    app.adsb_panel.selected_icao = Some(0x4840d6);
    app.seen_aircraft.insert(0x4840d6);
    app.notified_aircraft.insert(0x4840d6);

    // A native USB/callback queue drop advances the acquisition clock, but
    // the delivered stream's indices are still contiguous. This previously
    // erased the coordinates and also cleared the GUI's selection/trails.
    let mut after_loss = clock.offset_samples(delivered + 16_384).unwrap();
    after_loss.discontinuous = true;
    delivered += publish(encode_uc8(&[EXAMPLE_DF17_CALLSIGN]), after_loss, delivered);
    let snapshot = wait(&app, delivered);
    assert_eq!(snapshot.total_lost_samples, 0); // counts native bus loss, not USB loss
    app.apply_local_adsb_snapshot();
    app.adsb_panel.tick_model();
    assert!(app.adsb_panel.aircraft[0].position_valid());
    assert_eq!(
        (
            app.adsb_panel.aircraft[0].lat,
            app.adsb_panel.aircraft[0].lon
        ),
        original
    );
    assert_eq!(app.adsb_panel.selected_icao, Some(0x4840d6));
    assert_eq!(app.adsb_panel.trail_len(0x4840d6), 1);
    assert!(app.seen_aircraft.contains(&0x4840d6));
    assert!(app.notified_aircraft.contains(&0x4840d6));
    assert!(app
        .status_bar
        .current()
        .unwrap()
        .text
        .contains("ADS-B capture gap"));

    let ctx = egui::Context::default();
    app.adsb_panel.observer_lat = 52.2572;
    app.adsb_panel.observer_lon = 8.6676;
    app.adsb_panel.test_offline_map(&ctx);
    let mut renderer = software_renderer::SoftwareRenderer::default();
    let mut render_map = |app: &mut CentralApp, expected_marker: bool, name: &str| {
        for pass in 0..3 {
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 700.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| app.adsb_panel.ui_map(ui));
                },
            );
            if pass == 2 {
                let marker_label = output.shapes.iter().any(|shape| {
                    matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "KLM 63EW")
                });
                assert_eq!(
                    marker_label, expected_marker,
                    "incorrect map visibility in {name}"
                );
                if expected_marker {
                    assert!(output.shapes.iter().any(|shape| {
                        matches!(&shape.shape, egui::Shape::Mesh(mesh)
                            if mesh.vertices.len() == 4 && mesh.calc_bounds().width() < 40.0
                            && mesh.calc_bounds().width() > 10.0 && shape.clip_rect.intersects(mesh.calc_bounds()))
                    }), "callsign had no visible aircraft mesh");
                }
            }
            if let Ok(folder) = std::env::var("EZ_SDR_ADSB_AUDIT_DIR") {
                let pixels = renderer.render(&ctx, output, 1000, 700);
                if pass == 2 {
                    pixels.save(format!("{folder}/{name}.png")).unwrap();
                }
            }
        }
    };
    render_map(&mut app, true, "position-after-callback-loss");

    // A new epoch cannot borrow the previous fix; a fresh weak pair must
    // pass CRC, semantic merging, native snapshot publication and rendering.
    let fresh_clock = CaptureClock::new(702, 0, 2_400_000, None);
    let mut weak_pair = encode_uc8(&[odd, even]);
    for iq in weak_pair.chunks_exact_mut(2) {
        if iq[0] > 127 {
            iq[0] = 132;
        }
    }
    let mut fresh = encode_uc8(&[EXAMPLE_DF17_CALLSIGN]);
    fresh.extend(weak_pair);
    delivered += publish(fresh, fresh_clock, delivered);
    wait(&app, delivered);
    app.apply_local_adsb_snapshot();
    assert!(app.adsb_panel.aircraft[0].position_valid());
    assert!(app.adsb_panel.selected_icao.is_none());
    assert!(app.notified_aircraft.is_empty());
    render_map(&mut app, true, "position-from-weak-frames");

    let expired_clock = fresh_clock.offset_samples(61 * 2_400_000).unwrap();
    delivered += publish(
        encode_uc8(&[EXAMPLE_DF17_CALLSIGN]),
        expired_clock,
        delivered,
    );
    wait(&app, delivered);
    app.apply_local_adsb_snapshot();
    assert_eq!(app.adsb_panel.aircraft[0].callsign, "KLM 63EW");
    assert!(!app.adsb_panel.aircraft[0].position_valid());
    render_map(&mut app, false, "expired-position-hidden");
}

#[test]
fn native_iq_recorders_preserve_every_sample_at_60_hz_and_during_a_gui_stall() {
    use crate::source_manager::SourceMode;
    let mut app = offline_test_app();
    app.current_tab = AppTab::Meteor;
    let dir = std::env::temp_dir().join(format!(
        "ez-a09-recording-{}-{}",
        std::process::id(),
        ez_proto::new_capture_epoch()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let period: Vec<u8> = (0..16_384)
        .flat_map(|sample| [(sample % 256) as u8, (255 - sample % 256) as u8])
        .collect();
    let stream = {
        let mut state = app.shared.lock().unwrap();
        state.source.source_mode = SourceMode::Hardware;
        state.source.frequency_hz = 118_000_000;
        state.source.center_frequency_hz = Some(118_000_000);
        state.source.sample_rate_hz = 2_400_000;
        let input = period.clone();
        state
            .source
            .test_set_hardware_open(move |_| Ok(input.clone()));
        state.source.capture_stream()
    };
    app.recorder_panel.output_dir = dir.display().to_string();
    app.recorder_panel.filename_template = "native".into();
    app.satellite_panel.cf32_output_dir = dir.display().to_string();
    // Arm both independent subscriptions before the first acquisition block.
    app.recorder_panel.start_recording();
    app.satellite_panel.start_cf32_recording();
    assert!(
        app.recorder_panel.recording,
        "{}",
        app.recorder_panel.last_error
    );
    assert!(
        app.satellite_panel.cf32_recording,
        "{:?}",
        app.satellite_panel.pending_status
    );
    let cf32_path = app
        .satellite_panel
        .cf32_writer
        .as_ref()
        .unwrap()
        .path()
        .to_path_buf();
    app.shared.lock().unwrap().source.start();
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    let started = Instant::now();
    for tick in 1..=60 {
        app.logic(&ctx, &mut frame);
        std::thread::sleep(
            (started + Duration::from_secs_f64(f64::from(tick) / 60.0))
                .saturating_duration_since(Instant::now()),
        );
    }
    {
        let mut state = app.shared.lock().unwrap();
        std::thread::sleep(Duration::from_millis(300));
        state.source.stop();
        state.source.wait_for_shutdown();
    }
    let source = stream.progress();
    app.recorder_panel.stop_and_wait();
    app.satellite_panel.stop_cf32_and_wait();
    assert!(source.samples >= 2_000_000);
    let expected = period.repeat(source.blocks as usize);
    let raw = std::fs::read(dir.join("native.iq")).unwrap();
    let cf32 = std::fs::read(&cf32_path).unwrap();
    assert_eq!(raw.len() as u64 / 2, source.samples);
    assert!(raw == expected, "raw recording differs from native input");
    let converted = lrpt_decode::uc8_bytes_to_cf32_le(&raw);
    assert_eq!(cf32.len(), converted.len(), "CF32 native sample loss");
    assert!(
        cf32 == converted,
        "CF32 conversion differs from native input"
    );
    let raw_meta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("native.json")).unwrap()).unwrap();
    let cf32_meta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(cf32_path.with_extension("json")).unwrap()).unwrap();
    assert_eq!(raw_meta["complete"], true);
    assert_eq!(cf32_meta["complete"], true);
    assert_eq!(raw_meta["capture"]["samples"], source.samples);
    assert_eq!(raw_meta["capture"], cf32_meta["capture"]);
    assert_eq!(raw_meta["capture_input_dropped_blocks"], 0);
    assert_eq!(cf32_meta["capture_input_dropped_blocks"], 0);
    assert_eq!(raw_meta["lost_samples"], 0);
    assert_eq!(cf32_meta["lost_samples"], 0);
    println!(
        "A09 native recording at 60 Hz + 300 ms GUI/shared-lock stall: {} samples; raw {} bytes, CF32 {} bytes, zero loss",
        source.samples,
        raw.len(),
        cf32.len()
    );
    drop(app);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn native_wav_recording_matches_continuous_dsp_at_60_hz_and_during_a_gui_stall() {
    use crate::source_manager::SourceMode;
    let mut app = offline_test_app();
    app.current_tab = AppTab::Meteor;
    let dir = std::env::temp_dir().join(format!(
        "ez-a09-wave-{}-{}",
        std::process::id(),
        ez_proto::new_capture_epoch()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let period: Vec<u8> = (0..16_384)
        .flat_map(|n| {
            let t = n as f64 / 2_400_000.0;
            let amplitude = 0.45 * (1.0 + 0.4 * (std::f64::consts::TAU * 1_000.0 * t).sin());
            let angle = std::f64::consts::TAU * 100_000.0 * t;
            [angle.cos(), angle.sin()].map(|value| {
                (value * amplitude * 128.0 + 127.4)
                    .round()
                    .clamp(0.0, 255.0) as u8
            })
        })
        .collect();
    let (release_source, source_gate) = crossbeam_channel::bounded(1);
    let stream = {
        let mut state = app.shared.lock().unwrap();
        state.source.source_mode = SourceMode::Hardware;
        state.source.frequency_hz = 118_100_000;
        state.source.center_frequency_hz = Some(118_000_000);
        state.source.sample_rate_hz = 2_400_000;
        crate::radio_ui::select_demod(&mut state, crate::sdr_panel::DemodMode::Am);
        state.config.advanced.rf_decim = 8;
        state.config.advanced.rf_dc_remove = true;
        state.volume = 0.6;
        state.audio_running = false; // WAV capture runs without opening a speaker.
        let input = period.clone();
        state.source.test_set_hardware_open(move |_| {
            source_gate.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(input.clone())
        });
        state.source.capture_stream()
    };
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    app.logic(&ctx, &mut frame);
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.demod_worker.applied_version() < app.demod_worker.requested_version() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    let (advanced, iq_config, volume, cutoff, bandwidth) = {
        let state = app.shared.lock().unwrap();
        (
            state.config.advanced.clone(),
            crate::radio_ui::radio_iq_config(&state),
            state.volume,
            state.lpf_cutoff,
            crate::radio_ui::channel_bandwidth(&state),
        )
    };
    app.recorder_panel.output_dir = dir.display().to_string();
    app.recorder_panel.filename_template = "native-wave".into();
    app.recorder_panel.record_iq = false;
    app.recorder_panel.record_audio = true;
    app.recorder_panel.start_recording();
    assert!(
        app.recorder_panel.recording,
        "{}",
        app.recorder_panel.last_error
    );
    app.shared.lock().unwrap().source.start();
    // Apply the new capture generation before the device produces its first
    // block. Old requested generations must not accept the new session.
    app.logic(&ctx, &mut frame);
    while app.demod_worker.applied_version() < app.demod_worker.requested_version() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    release_source.send(()).unwrap();
    let started = Instant::now();
    for tick in 1..=60 {
        app.logic(&ctx, &mut frame);
        std::thread::sleep(
            (started + Duration::from_secs_f64(f64::from(tick) / 60.0))
                .saturating_duration_since(Instant::now()),
        );
    }
    {
        let mut state = app.shared.lock().unwrap();
        std::thread::sleep(Duration::from_millis(300));
        state.source.stop();
        state.source.wait_for_shutdown();
    }
    let source = stream.progress();
    // The byte-for-byte assertion below checks lossless draining, not a
    // real-time speed budget. Debug DSP and concurrent test workers can need
    // substantially longer than five seconds to drain the RF tail.
    let deadline = Instant::now() + Duration::from_secs(60);
    while !app.demod_worker.radio_snapshot().unwrap().complete {
        assert!(
            Instant::now() < deadline,
            "native RF tail did not finish: received {}, capture {}",
            app.demod_worker.radio_snapshot().unwrap().received_samples,
            source.samples
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    app.recorder_panel.stop_and_wait();
    let snapshot = app.demod_worker.radio_snapshot().unwrap();
    assert!(source.samples >= 2_000_000);
    assert_eq!(snapshot.received_samples, source.samples);
    assert_eq!(snapshot.demodulated_samples, source.samples);
    assert_eq!(snapshot.skipped_samples, 0);
    assert_eq!(snapshot.lost_samples, 0);
    assert_eq!(snapshot.dropped_blocks, 0);
    let mut iq = crate::radio_iq::RadioIqProcessor::new(iq_config);
    let mut mixer = crate::radio_iq::VfoMixer::new(iq.output_rate());
    mixer.configure(iq.output_rate(), 100_000.0);
    let mut demod = crate::demod::Demodulator::new();
    demod.apply_advanced(&advanced, crate::sdr_panel::DemodMode::Am);
    demod.set_sample_rates_exact(iq.output_rate(), 48_000);
    demod.set_lpf_cutoff(cutoff);
    demod.set_rf_bandwidth(bandwidth);
    demod.reset();
    let mut expected = Vec::new();
    for _ in 0..source.blocks {
        let channel = mixer.process(&iq.process(&period));
        expected.extend(
            demod
                // Use the same physical capture clock for the synchronous
                // reference. The RF decimator's causal startup interval is
                // retained by capture-clock conversion; treating this as a
                // new 300 kSPS source omits that interval and shifts the PCM.
                .demodulate_capture_complex(
                    &channel,
                    crate::sdr_panel::DemodMode::Am,
                    period.len() / 2,
                    2_400_000.0,
                )
                .iter()
                .map(|sample| ((sample * volume).clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16),
        );
    }
    let mut wav = hound::WavReader::open(dir.join("native-wave_audio.wav")).unwrap();
    assert_eq!(wav.spec().sample_rate, 48_000);
    let recorded = wav.samples::<i16>().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(recorded.len(), expected.len());
    let differences: Vec<_> = recorded
        .iter()
        .zip(&expected)
        .enumerate()
        .filter(|(_, (actual, reference))| actual != reference)
        .map(|(index, (&actual, &reference))| (index, actual, reference))
        .collect();
    let max_error = differences
        .iter()
        .map(|(_, actual, reference)| (i32::from(*actual) - i32::from(*reference)).abs())
        .max()
        .unwrap_or(0);
    assert!(
        recorded == expected,
        "WAV differs from continuous synchronous DSP: {} mismatches, max error {max_error} PCM counts, first {:?}",
        differences.len(),
        &differences[..differences.len().min(8)]
    );
    let meta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("native-wave.wav.json")).unwrap()).unwrap();
    assert_eq!(meta["complete"], true, "{}", meta["stop_reason"]);
    assert_eq!(meta["audio_capture"]["samples"], source.samples);
    assert_eq!(meta["audio_samples"], expected.len() as u64);
    assert_eq!(meta["audio_native_lost_samples"], 0);
    assert_eq!(meta["audio_input_dropped_blocks"], 0);
    println!(
        "A09 native app WAV at 60 Hz + 300 ms GUI/shared-lock stall: {} RF samples, {} PCM samples identical to synchronous DSP, zero loss",
        source.samples,
        recorded.len()
    );
    drop(app);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn hidden_app_recorders_finalize_each_real_worker_epoch_before_retuned_samples() {
    use crate::source_manager::{SourceMode, SourceStatus};
    let mut app = offline_test_app();
    app.current_tab = AppTab::Meteor;
    app.satellite_panel.selected_sat_index = None;
    let dir = std::env::temp_dir().join(format!(
        "ez-r05-app-{}-{}",
        std::process::id(),
        ez_proto::new_capture_epoch()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    app.recorder_panel.output_dir = dir.display().to_string();
    app.recorder_panel.filename_template = "first".into();
    {
        let mut state = app.shared.lock().unwrap();
        state.source.source_mode = SourceMode::Hardware;
        state.source.frequency_hz = 118_000_000;
        state.source.center_frequency_hz = Some(118_000_000);
        state.source.sample_rate_hz = 2_400_000;
        state
            .source
            .test_set_hardware_open(|settings: crate::source_manager::RtlOpenSettings| {
                let pair = if settings.frequency_hz == 118_000_000 {
                    [80, 90]
                } else {
                    [200, 210]
                };
                Ok(pair.repeat(16_384))
            });
        state.source.start();
    }
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !app.shared.lock().unwrap().source.capture_tuning_applied() {
        app.logic(&ctx, &mut frame);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    app.recorder_panel.start_recording();
    assert!(
        app.recorder_panel.recording,
        "{}",
        app.recorder_panel.last_error
    );
    let satellite = crate::satellite::recorder::Cf32StreamWriter::start(
        dir.to_str().unwrap(),
        None,
        2_400_000,
        118_000_000,
        0.0,
        0.0,
    )
    .unwrap();
    let satellite_path = satellite.path().to_path_buf();
    let stream = app.shared.lock().unwrap().source.capture_stream();
    app.satellite_panel.cf32_writer = Some(satellite.with_capture(stream));
    app.satellite_panel.cf32_recording = true;
    while app.recorder_panel.bytes_written == 0 {
        app.logic(&ctx, &mut frame);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(app.satellite_panel.cf32_recording);
    app.shared
        .lock()
        .unwrap()
        .source
        .tune_and_restart(119_000_000, 2_000_000);
    while app.recorder_panel.recording || app.satellite_panel.cf32_recording {
        app.logic(&ctx, &mut frame);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    let first = std::fs::read(dir.join("first.iq")).unwrap();
    assert!(!first.is_empty());
    assert!(first.chunks_exact(2).all(|pair| pair == [80, 90]));
    let satellite_bytes = std::fs::read(&satellite_path).unwrap();
    assert!(!satellite_bytes.is_empty());
    assert_eq!(
        satellite_bytes,
        lrpt_decode::uc8_bytes_to_cf32_le(&[80, 90].repeat(satellite_bytes.len() / 8))
    );
    let first_metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("first.json")).unwrap()).unwrap();
    let satellite_metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(satellite_path.with_extension("json")).unwrap())
            .unwrap();
    assert_eq!(first_metadata["frequency_hz"], 118_000_000);
    assert_eq!(first_metadata["sample_rate_hz"], 2_400_000);
    assert_eq!(first_metadata["capture"]["samples"], first.len() as u64 / 2);
    assert_eq!(
        satellite_metadata["capture"]["samples"],
        satellite_bytes.len() as u64 / 8
    );
    assert_eq!(
        first_metadata["capture"]["first_clock"]["epoch"],
        satellite_metadata["capture"]["first_clock"]["epoch"]
    );
    assert_eq!(satellite_metadata["frequency_hz"], 118_000_000);
    assert_eq!(satellite_metadata["sample_rate_hz"], 2_400_000);
    assert!(app.recorder_panel.last_error.is_empty());
    assert_eq!(first_metadata["stop_reason"], "capture session ended");
    assert!(!app.shared.lock().unwrap().recording);
    // A clean source-end prefix can now be offered for offline decoding.
    // Any handoff must still describe the old file's immutable geometry.
    if let Some(request) = &app.satellite_panel.pending_decode_request {
        assert_eq!(std::path::Path::new(&request.file_path), satellite_path);
        assert_eq!(request.sample_rate, 2_400_000);
    }
    while !app.shared.lock().unwrap().source.capture_tuning_applied() {
        app.logic(&ctx, &mut frame);
        assert!(Instant::now() < deadline);
    }
    assert_eq!(
        app.shared.lock().unwrap().source.status,
        SourceStatus::Running
    );
    app.recorder_panel.filename_template = "second".into();
    app.recorder_panel.start_recording();
    while app.recorder_panel.bytes_written == 0 {
        app.logic(&ctx, &mut frame);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    app.recorder_panel.stop_and_wait();
    let second = std::fs::read(dir.join("second.iq")).unwrap();
    assert!(second.chunks_exact(2).all(|pair| pair == [200, 210]));
    let second_metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("second.json")).unwrap()).unwrap();
    assert_eq!(second_metadata["frequency_hz"], 119_000_000);
    assert_eq!(second_metadata["sample_rate_hz"], 2_000_000);
    assert_eq!(second_metadata["complete"], true);
    assert_ne!(
        second_metadata["capture"]["first_clock"]["epoch"],
        first_metadata["capture"]["first_clock"]["epoch"]
    );
    app.shared.lock().unwrap().source.stop();
    drop(app);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_recording_epoch_stop_reason_reaches_hidden_app_and_allows_a_new_file() {
    use crate::daemon_client::tests::TestDaemon;
    use crate::source_manager::SourceMode;
    let daemon = TestDaemon::spawn_with_rate("r05-recording", 2_000_000).await;
    let mut app = tokio::task::block_in_place(offline_test_app);
    app.current_tab = AppTab::Meteor;
    {
        let mut state = app.shared.lock().unwrap();
        state.source.source_mode = SourceMode::Daemon;
        state.source.daemon_addr = daemon.addr.to_string();
        state.source.frequency_hz = 100_000_000;
        state.source.center_frequency_hz = Some(100_000_000);
        state.source.sample_rate_hz = 2_000_000;
        state.source.start();
    }
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        app.drain_daemon_events();
        if app.shared.lock().unwrap().source.capture_tuning_applied() {
            break;
        }
        assert!(Instant::now() < end, "daemon acquisition did not apply");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    app.recorder_panel.start();
    loop {
        app.drain_daemon_events();
        if app.recorder_panel.recording && app.recorder_panel.bytes_written > 0 {
            break;
        }
        assert!(
            Instant::now() < end,
            "daemon record did not start: {}",
            app.recorder_panel.last_error
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let first = std::path::PathBuf::from(&app.recorder_panel.last_filename);
    app.shared
        .lock()
        .unwrap()
        .source
        .tune_and_restart(101_000_000, 2_400_000);
    loop {
        app.drain_daemon_events();
        if !app.recorder_panel.recording
            && app.recorder_panel.last_error.contains("Recording stopped")
        {
            break;
        }
        assert!(
            Instant::now() < end,
            "epoch stop was not delivered: {}",
            app.recorder_panel.last_error
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(!app.shared.lock().unwrap().recording);
    let first_metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(format!("{}.json", first.display())).unwrap())
            .unwrap();
    assert_eq!(first_metadata["frequency_hz"], 100_000_000);
    assert_eq!(first_metadata["sample_rate_hz"], 2_000_000);
    assert_eq!(first_metadata["complete"], false);
    assert_eq!(
        std::fs::metadata(&first).unwrap().len(),
        first_metadata["capture"]["samples"].as_u64().unwrap() * 8
    );
    while !app.shared.lock().unwrap().source.capture_tuning_applied() {
        app.drain_daemon_events();
        assert!(Instant::now() < end);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    app.recorder_panel.start();
    loop {
        app.drain_daemon_events();
        if app.recorder_panel.recording && app.recorder_panel.bytes_written > 0 {
            break;
        }
        assert!(
            Instant::now() < end,
            "new file did not start: {}",
            app.recorder_panel.last_error
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let second = std::path::PathBuf::from(&app.recorder_panel.last_filename);
    assert_ne!(first, second);
    app.recorder_panel.stop();
    loop {
        app.drain_daemon_events();
        if !app.recorder_panel.recording {
            break;
        }
        assert!(Instant::now() < end);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let second_metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(format!("{}.json", second.display())).unwrap())
            .unwrap();
    assert_eq!(second_metadata["frequency_hz"], 101_000_000);
    assert_eq!(second_metadata["sample_rate_hz"], 2_400_000);
    assert_eq!(second_metadata["complete"], true);
    assert_ne!(
        first_metadata["capture"]["first_clock"]["epoch"],
        second_metadata["capture"]["first_clock"]["epoch"]
    );
    assert_eq!(
        std::fs::metadata(&second).unwrap().len(),
        second_metadata["capture"]["samples"].as_u64().unwrap() * 8
    );
    let directory = first.parent().unwrap().to_path_buf();
    app.shared.lock().unwrap().source.stop();
    drop(app);
    daemon.shutdown().await;
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn scan_real_local_worker_has_identical_hits_visible_hidden_and_frozen_then_waits_on_stop() {
    use crate::source_manager::SourceMode;
    let iq: Vec<u8> = (0..16_384)
        .flat_map(|index| {
            let sample = [(0.0, 0.15), (25_000.0, 0.25), (50_000.0, 0.35)]
                .into_iter()
                .map(|(frequency, amplitude)| {
                    num_complex::Complex32::from_polar(
                        amplitude,
                        (std::f64::consts::TAU * frequency * index as f64 / 2_048_000.0) as f32,
                    )
                })
                .sum::<num_complex::Complex32>();
            [sample.re, sample.im]
                .map(|value| (127.4 + value * 128.0).round().clamp(0.0, 255.0) as u8)
        })
        .collect();
    let mut reference = None;
    for (tab, frozen) in [
        (AppTab::Listen, false),
        (AppTab::Meteor, false),
        (AppTab::Planes, true),
    ] {
        let mut app = offline_test_app();
        app.current_tab = tab;
        app.satellite_panel.selected_sat_index = None;
        {
            let mut state = app.shared.lock().unwrap();
            state.config.advanced.rf_decim = 1;
            state.config.advanced.rf_dc_remove = false;
            state.source.source_mode = SourceMode::Hardware;
            state.source.frequency_hz = 100_000_000;
            state.source.center_frequency_hz = Some(100_000_000);
            state.source.sample_rate_hz = 2_048_000;
            state.spectrum.frozen = frozen;
            crate::radio_ui::select_demod(&mut state, crate::sdr_panel::DemodMode::Fm);
            let iq = iq.clone();
            state.source.test_set_hardware_open(move |_| Ok(iq.clone()));
            state.source.start();
        }
        app.scanner.start_hz = 100_000_000;
        app.scanner.stop_hz = 100_050_000;
        app.scanner.step_hz = 25_000;
        app.scanner.dwell_ms = 20;
        app.scanner.threshold_db = -35.0;
        app.scanner.start();
        app.apply_scanner_tune();
        let ctx = egui::Context::default();
        let mut frame = eframe::Frame::_new_kittest();
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.scanner.hits.len() < 3 {
            app.logic(&ctx, &mut frame);
            assert!(
                Instant::now() < deadline,
                "{tab:?}, frozen={frozen}: {} / {:?}",
                app.scanner.status_text,
                app.scanner.hits
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        let hits: Vec<_> = app
            .scanner
            .hits
            .iter()
            .map(|hit| (hit.freq_hz, (hit.strength_db * 10.0).round() as i32))
            .collect();
        assert_eq!(
            hits.iter().map(|hit| hit.0).collect::<Vec<_>>(),
            [100_000_000, 100_025_000, 100_050_000]
        );
        if let Some(reference) = &reference {
            assert_eq!(&hits, reference);
        } else {
            reference = Some(hits);
        }
        if frozen {
            assert_eq!(app.shared.lock().unwrap().spectrum.peak_level(), -100.0);
        }
        app.shared.lock().unwrap().source.stop();
        let frequency = app.scanner.current_freq_hz;
        let counts: Vec<_> = app.scanner.hits.iter().map(|hit| hit.hit_count).collect();
        for _ in 0..20 {
            app.logic(&ctx, &mut frame);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(app.scanner.current_freq_hz, frequency);
        assert_eq!(
            app.scanner
                .hits
                .iter()
                .map(|hit| hit.hit_count)
                .collect::<Vec<_>>(),
            counts
        );
        assert!(app.scanner.status_text.contains("Waiting for the receiver"));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scan_real_daemon_frames_work_hidden_and_frozen_then_wait_on_stop() {
    use crate::daemon_client::tests::TestDaemon;
    use crate::source_manager::SourceMode;
    let daemon = TestDaemon::spawn_with_rate("g02-scanner", 2_000_000).await;
    for (tab, frozen) in [
        (AppTab::Listen, false),
        (AppTab::Meteor, false),
        (AppTab::Planes, true),
    ] {
        let mut app = tokio::task::block_in_place(offline_test_app);
        app.current_tab = tab;
        {
            let mut state = app.shared.lock().unwrap();
            state.source.source_mode = SourceMode::Daemon;
            state.source.daemon_addr = daemon.addr.to_string();
            state.source.frequency_hz = 100_250_000;
            state.source.center_frequency_hz = Some(100_000_000);
            state.source.sample_rate_hz = 2_000_000;
            state.spectrum.frozen = frozen;
            crate::radio_ui::select_demod(&mut state, crate::sdr_panel::DemodMode::Fm);
            state.source.start();
        }
        app.scanner.start_hz = 100_250_000;
        app.scanner.stop_hz = 100_300_000;
        app.scanner.step_hz = 50_000;
        app.scanner.dwell_ms = 20;
        app.scanner.threshold_db = -30.0;
        app.scanner.start();
        app.apply_scanner_tune();
        let ctx = egui::Context::default();
        let mut frame = eframe::Frame::_new_kittest();
        let deadline = Instant::now() + Duration::from_secs(8);
        while app.scanner.hits.is_empty() || app.scanner.current_freq_hz != 100_250_000 {
            app.logic(&ctx, &mut frame);
            assert!(
                Instant::now() < deadline,
                "{tab:?}, frozen={frozen}: {} / {:?}",
                app.scanner.status_text,
                app.scanner.hits
            );
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        assert_eq!(app.scanner.hits.len(), 1);
        assert_eq!(app.scanner.hits[0].freq_hz, 100_250_000);
        assert!(app.scanner.hits[0].strength_db > -20.0);
        if frozen {
            assert_eq!(app.shared.lock().unwrap().spectrum.peak_level(), -100.0);
        }
        app.shared.lock().unwrap().source.stop();
        let counts: Vec<_> = app.scanner.hits.iter().map(|hit| hit.hit_count).collect();
        for _ in 0..20 {
            app.logic(&ctx, &mut frame);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(app.scanner.current_freq_hz, 100_250_000);
        assert_eq!(
            app.scanner
                .hits
                .iter()
                .map(|hit| hit.hit_count)
                .collect::<Vec<_>>(),
            counts
        );
        assert!(app.scanner.status_text.contains("Waiting for the receiver"));
    }
    daemon.shutdown().await;
}

#[test]
fn desktop_live_meteor_controls_start_and_stop_the_owned_capture_worker() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx);
    let mut config = AppConfig::default();
    config.mqtt_broker.clear();
    let mut app = CentralApp::new_with_config(&cc, config);
    {
        let mut state = app.shared.lock().unwrap();
        state.source.source_mode = crate::source_manager::SourceMode::Simulated;
        state.source.sample_rate_hz = 288_000;
    }
    app.meteor_decoder.live_start_requested = true;
    app.tick_meteor_workflows();
    assert!(
        app.meteor_decoder.live_active,
        "{:?}",
        app.meteor_decoder.error
    );
    app.meteor_decoder.live_stop_requested = true;
    app.tick_meteor_workflows();
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.meteor_decoder.live_active && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
        app.tick_meteor_workflows();
    }
    assert!(!app.meteor_decoder.live_active);
    app.shared.lock().unwrap().source.stop();
}

#[test]
fn desktop_live_meteor_does_not_wait_forever_after_capture_open_failure() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx);
    let mut config = AppConfig::default();
    config.mqtt_broker.clear();
    let mut app = CentralApp::new_with_config(&cc, config);
    app.shared.lock().unwrap().source.sample_rate_hz = 0;
    app.meteor_decoder.live_start_requested = true;
    app.tick_meteor_workflows();
    assert!(!app.meteor_decoder.live_active);
    assert!(app
        .meteor_decoder
        .error
        .as_deref()
        .is_some_and(|e| e.contains("Receiver could not start")));
}

#[test]
fn daemon_live_stop_timeout_is_visible_and_never_claims_complete() {
    let mut app = offline_test_app();
    app.meteor_decoder
        .start_daemon_live(ez_proto::MeteorProfile::default(), Some("NOAA-19".into()));
    app.meteor_decoder.request_daemon_live_stop();
    app.shared
        .lock()
        .unwrap()
        .source
        .set_test_daemon_channel_error("LRPT product finalization timed out after 15 seconds");

    app.tick_meteor_workflows();

    assert!(!app.meteor_decoder.live_active);
    assert!(app.meteor_decoder.result.is_none());
    let message = app.meteor_decoder.error.as_deref().unwrap_or_default();
    assert!(message.contains("15 seconds"), "{message}");
    assert!(
        message.contains("use Stop live before disconnecting"),
        "{message}"
    );
}
