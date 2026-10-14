use super::*;
use crate::source_manager::{RtlOpenSettings, SourceMode, SourceStatus};
use std::sync::atomic::{AtomicBool, Ordering};

fn frame(app: &mut CentralApp, ctx: &egui::Context) {
    let mut frame = eframe::Frame::_new_kittest();
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
        app.logic(ui.ctx(), &mut frame)
    });
}

fn wait(app: &mut CentralApp, ctx: &egui::Context, ready: impl Fn(&CentralApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(4);
    while !ready(app) {
        frame(app, ctx);
        assert!(
            Instant::now() < deadline,
            "action did not finish: {:?}; {}",
            app.shared.lock().unwrap().source.status,
            app.ai_panel
                .messages
                .last()
                .map_or("", |message| message.content.as_str())
        );
        std::thread::sleep(Duration::from_millis(3));
    }
}

fn approved(
    app: &mut CentralApp,
    ctx: &egui::Context,
    tool: &str,
    args: serde_json::Value,
) -> String {
    app.ai_panel.test_propose_tool(tool, args);
    let requested = app.ai_panel.test_approve_tool();
    if !requested.starts_with("Error:") {
        assert!(requested.starts_with("Requested"), "{requested}");
        frame(app, ctx);
        wait(app, ctx, |app| {
            app.runtime_action.is_none() && app.shared.lock().unwrap().runtime_actions.is_empty()
        });
        // AI results are consumed in the following app cycle, with no stream
        // or AI window required.
        frame(app, ctx);
    }
    app.ai_panel.messages.last().unwrap().content.clone()
}

fn fake_local(app: &mut CentralApp, fail: Arc<AtomicBool>) -> Arc<Mutex<Vec<RtlOpenSettings>>> {
    let opens = Arc::new(Mutex::new(Vec::new()));
    let opened = Arc::clone(&opens);
    let mut state = app.shared.lock().unwrap();
    state.source.source_mode = SourceMode::Hardware;
    state.source.frequency_hz = 100_000_000;
    state.source.center_frequency_hz = Some(100_000_000);
    state.source.sample_rate_hz = 2_000_000;
    state.source.frequency_offset_hz = 0;
    state.source.test_set_hardware_open(move |settings| {
        opened.lock().unwrap().push(settings);
        if fail.load(Ordering::Relaxed) {
            Err("injected device-open rejection".into())
        } else {
            Ok(vec![127; 16384])
        }
    });
    state.audio_running = false;
    opens
}

fn private_dir(label: &str) -> std::path::PathBuf {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ez-ai-runtime-{label}-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn approved_ai_local_tools_restart_live_capture_and_record_real_files_while_hidden() {
    let mut app = offline_test_app();
    app.current_tab = AppTab::Meteor;
    let opens = fake_local(&mut app, Arc::new(AtomicBool::new(false)));
    let ctx = egui::Context::default();
    app.shared.lock().unwrap().source.start();
    assert_eq!(
        app.shared.lock().unwrap().source.status,
        SourceStatus::Opening
    );
    wait(&mut app, &ctx, |app| {
        app.shared.lock().unwrap().source.capture_tuning_applied()
    });

    // The current AI tool approval widget must enqueue, with no mutation
    // while the untrusted provider proposal is only displayed.
    app.ai_panel
        .test_propose_tool("set_gain", serde_json::json!({"db": 20.0}));
    let generation = app.shared.lock().unwrap().source.stream_generation();
    let before = opens.lock().unwrap().len();
    frame(&mut app, &ctx);
    assert_eq!(opens.lock().unwrap().len(), before);
    assert_eq!(
        app.shared.lock().unwrap().source.stream_generation(),
        generation
    );
    let mut button = None;
    for _ in 0..2 {
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
                )),
                ..Default::default()
            },
            |ui| app.ai_panel.ui(ui),
        );
        for shape in output.shapes {
            if let egui::Shape::Text(text) = shape.shape {
                if text.galley.text() == "✔ Approve" {
                    button = Some(text.pos + text.galley.size() / 2.0);
                }
            }
        }
    }
    let position = button.expect("AI approval control was not painted");
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
            |ui| app.ai_panel.ui(ui),
        );
    }
    assert!(app
        .ai_panel
        .messages
        .last()
        .unwrap()
        .content
        .contains("Requested gain"));
    assert_eq!(
        opens.lock().unwrap().len(),
        before,
        "approval bypassed the app lifecycle queue"
    );
    frame(&mut app, &ctx);
    wait(&mut app, &ctx, |app| app.runtime_action.is_none());
    frame(&mut app, &ctx);
    assert!(app
        .ai_panel
        .messages
        .last()
        .unwrap()
        .content
        .contains("Receiver acknowledged gain"));
    assert_eq!(
        app.shared.lock().unwrap().source.stream_generation(),
        generation + 1
    );
    assert_eq!(opens.lock().unwrap().last().unwrap().gain_db, 20.0);

    for (tool, args) in [
        ("tune_frequency", serde_json::json!({"hz": 120_000_000})),
        ("set_sample_rate", serde_json::json!({"rate": 2_400_000})),
        ("toggle_bias_tee", serde_json::json!({"on": true})),
        ("set_ppm", serde_json::json!({"ppm": 17})),
    ] {
        let generation = app.shared.lock().unwrap().source.stream_generation();
        let result = approved(&mut app, &ctx, tool, args.clone());
        assert!(result.contains("Receiver acknowledged"), "{result}");
        assert_eq!(
            app.shared.lock().unwrap().source.stream_generation(),
            generation + 1
        );
        let result = approved(&mut app, &ctx, tool, args);
        assert!(result.contains("Receiver acknowledged"), "{result}");
        assert_eq!(
            app.shared.lock().unwrap().source.stream_generation(),
            generation + 1,
            "unchanged control restarted the receiver"
        );
    }
    let actual = opens.lock().unwrap().last().unwrap().clone();
    assert_eq!(
        (actual.sample_rate_hz, actual.ppm, actual.bias_tee),
        (2_400_000, 17, true)
    );

    let dir = private_dir("local");
    app.recorder_panel.output_dir = dir.to_string_lossy().into();
    app.recorder_panel.filename_template = "approved_capture".into();
    let result = approved(&mut app, &ctx, "start_recording", serde_json::json!({}));
    assert!(result.contains("Recorder opened"), "{result}");
    let path = dir.join(&app.recorder_panel.last_filename);
    assert!(path.is_file());
    wait(&mut app, &ctx, |app| app.recorder_panel.bytes_written > 0);
    let result = approved(&mut app, &ctx, "stop_recording", serde_json::json!({}));
    assert!(
        result.contains("Recorder stopped and finalized"),
        "{result}"
    );
    assert!(app.recorder_panel.bytes_written > 0);
    assert_eq!(
        std::fs::metadata(path).unwrap().len(),
        app.recorder_panel.bytes_written
    );
    assert!(!app.shared.lock().unwrap().recording);

    let generation = app.shared.lock().unwrap().source.stream_generation();
    let result = approved(&mut app, &ctx, "start_adsb", serde_json::json!({}));
    assert!(
        result.contains("Receiver acknowledged start ADS-B"),
        "{result}"
    );
    assert_eq!(
        app.shared.lock().unwrap().source.stream_generation(),
        generation + 1
    );
    let actual = opens.lock().unwrap().last().unwrap().clone();
    assert_eq!(
        (actual.frequency_hz, actual.sample_rate_hz),
        (1_090_000_000, 2_400_000)
    );
    assert!(app.shared.lock().unwrap().adsb_running);
    let result = approved(&mut app, &ctx, "stop_adsb", serde_json::json!({}));
    assert!(
        result.contains("Receiver acknowledged stop ADS-B"),
        "{result}"
    );
    assert_eq!(app.shared.lock().unwrap().source.status, SourceStatus::Idle);
    assert!(!app.shared.lock().unwrap().adsb_running);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn approved_ai_local_failures_report_device_and_recording_errors_without_success() {
    let ctx = egui::Context::default();
    for (tool, args) in [
        ("set_gain", serde_json::json!({"db": 20.0})),
        ("set_sample_rate", serde_json::json!({"rate": 2_400_000})),
        ("toggle_bias_tee", serde_json::json!({"on": true})),
        ("set_ppm", serde_json::json!({"ppm": 15})),
        ("start_adsb", serde_json::json!({})),
    ] {
        let mut app = offline_test_app();
        let fail = Arc::new(AtomicBool::new(false));
        let opens = fake_local(&mut app, Arc::clone(&fail));
        app.shared.lock().unwrap().source.start();
        wait(&mut app, &ctx, |app| {
            app.shared.lock().unwrap().source.capture_tuning_applied()
        });
        let count = opens.lock().unwrap().len();
        fail.store(true, Ordering::Relaxed);
        let result = approved(&mut app, &ctx, tool, args);
        assert!(
            result.contains("Error: injected device-open rejection"),
            "{tool}: {result}"
        );
        assert!(!result.contains("Receiver acknowledged"));
        assert_eq!(opens.lock().unwrap().len(), count + 1);
        if tool == "start_adsb" {
            assert!(!app.shared.lock().unwrap().adsb_running);
        }
        app.shared.lock().unwrap().source.stop();
    }
    let mut app = offline_test_app();
    fake_local(&mut app, Arc::new(AtomicBool::new(false)));
    let result = approved(&mut app, &ctx, "start_recording", serde_json::json!({}));
    assert!(
        result.contains("Error: wait for an acknowledged running receiver"),
        "{result}"
    );
    assert!(!app.shared.lock().unwrap().recording);
    app.shared.lock().unwrap().source.start();
    wait(&mut app, &ctx, |app| {
        app.shared.lock().unwrap().source.capture_tuning_applied()
    });
    let dir = private_dir("failure");
    let blocked = dir.join("ordinary-file");
    std::fs::write(&blocked, b"preserve").unwrap();
    app.recorder_panel.output_dir = blocked.to_string_lossy().into();
    let result = approved(&mut app, &ctx, "start_recording", serde_json::json!({}));
    assert!(result.contains("Error:"), "{result}");
    assert!(!result.contains("Recorder opened"));
    assert!(!app.recorder_panel.recording);
    assert!(!app.shared.lock().unwrap().recording);
    assert_eq!(std::fs::read(blocked).unwrap(), b"preserve");
    app.shared.lock().unwrap().source.stop();
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approved_ai_daemon_tools_acknowledge_capture_files_and_adsb_from_initial_two_msps() {
    let daemon = crate::daemon_client::tests::TestDaemon::spawn("ai-actions").await;
    let mut app = tokio::task::block_in_place(offline_test_app);
    app.current_tab = AppTab::Meteor;
    let ctx = egui::Context::default();
    {
        let mut state = app.shared.lock().unwrap();
        state.source.source_mode = SourceMode::Daemon;
        state.source.daemon_addr = daemon.addr.to_string();
        state.source.center_frequency_hz = Some(100_000_000);
        state.source.start();
        state.audio_running = false;
    }
    wait(&mut app, &ctx, |app| {
        app.shared.lock().unwrap().source.capture_tuning_applied()
    });
    let generation = app.shared.lock().unwrap().source.stream_generation();
    for (tool, args) in [
        ("tune_frequency", serde_json::json!({"hz": 101_000_000})),
        ("set_gain", serde_json::json!({"db": 20.0})),
        ("set_sample_rate", serde_json::json!({"rate": 2_200_000})),
    ] {
        let result = approved(&mut app, &ctx, tool, args);
        assert!(result.contains("Receiver acknowledged"), "{result}");
        assert!(!result.contains("Error:"), "{result}");
        assert_eq!(
            app.shared.lock().unwrap().source.stream_generation(),
            generation,
            "daemon setting reconnected the source"
        );
    }
    for (tool, args) in [
        ("toggle_bias_tee", serde_json::json!({"on": true})),
        ("set_ppm", serde_json::json!({"ppm": 17})),
        ("set_demod", serde_json::json!({"mode": "CW"})),
    ] {
        let result = approved(&mut app, &ctx, tool, args);
        assert!(result.contains("Error:"), "{result}");
    }
    app.recorder_panel.record_audio = true;
    let result = approved(&mut app, &ctx, "start_recording", serde_json::json!({}));
    assert!(
        result.contains("Error:") && result.contains("WAV"),
        "{result}"
    );
    app.recorder_panel.record_audio = false;
    let result = approved(&mut app, &ctx, "start_recording", serde_json::json!({}));
    assert!(result.contains("Recorder opened"), "{result}");
    let path = std::path::PathBuf::from(&app.recorder_panel.last_filename);
    assert!(path.is_file());
    wait(&mut app, &ctx, |app| app.recorder_panel.bytes_written > 0);
    let result = approved(&mut app, &ctx, "stop_recording", serde_json::json!({}));
    assert!(
        result.contains("Recorder stopped and finalized"),
        "{result}"
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().len(),
        app.recorder_panel.bytes_written
    );
    assert!(!app.shared.lock().unwrap().recording);

    // Restore the initial 2.0 MSps and exercise the actual audio-to-ADS-B
    // handoff. Its old virtual channel must retire before SetSampleRate.
    let result = approved(
        &mut app,
        &ctx,
        "set_sample_rate",
        serde_json::json!({"rate": 2_000_000}),
    );
    assert!(result.contains("Receiver acknowledged"), "{result}");
    app.shared.lock().unwrap().audio_running = true;
    for _ in 0..20 {
        frame(&mut app, &ctx);
        std::thread::sleep(Duration::from_millis(10));
    }
    let result = approved(&mut app, &ctx, "start_adsb", serde_json::json!({}));
    assert!(
        result.contains("Receiver acknowledged start ADS-B"),
        "{result}"
    );
    {
        let state = app.shared.lock().unwrap();
        assert_eq!(
            (
                state.source.capture_center_frequency_hz(),
                state.source.sample_rate_hz
            ),
            (1_090_000_000, 2_400_000)
        );
        assert!(state.source.daemon_adsb_ready());
    }
    let result = approved(&mut app, &ctx, "stop_adsb", serde_json::json!({}));
    assert!(
        result.contains("Receiver acknowledged stop ADS-B"),
        "{result}"
    );
    assert_eq!(app.shared.lock().unwrap().source.status, SourceStatus::Idle);
    let folder = path.parent().unwrap().to_path_buf();
    daemon.shutdown().await;
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn approved_ai_waits_for_daemon_acknowledgement_and_reports_rejection_or_timeout() {
    use ez_proto::{HardwareStatus, ServerEvent};
    let ctx = egui::Context::default();
    for timeout in [false, true] {
        let mut app = offline_test_app();
        let (client, mut commands, events) = crate::daemon_client::DaemonClient::test_connection();
        {
            let mut state = app.shared.lock().unwrap();
            state.source.source_mode = SourceMode::Daemon;
            state.source.center_frequency_hz = Some(100_000_000);
            state.source.set_test_daemon_client(client);
            state.audio_running = false;
        }
        events
            .send(ServerEvent::Hardware(HardwareStatus {
                connected: true,
                frequency_hz: 100_000_000,
                sample_rate_hz: 2_000_000,
                gain_db: 20.0,
                requested_gain_db: None,
                gain_mode: None,
                source_kind: "fake".into(),
                error: None,
            }))
            .unwrap();
        frame(&mut app, &ctx);
        app.ai_panel
            .test_propose_tool("set_sample_rate", serde_json::json!({"rate": 2_400_000}));
        assert!(app.ai_panel.test_approve_tool().starts_with("Requested"));
        frame(&mut app, &ctx);
        assert!(app.runtime_action.is_some());
        assert!(!app
            .ai_panel
            .messages
            .last()
            .unwrap()
            .content
            .contains("Receiver acknowledged"));
        assert!(matches!(
            commands.try_recv().unwrap(),
            ez_proto::ClientCommand::SetSampleRate { hz: 2_400_000 }
        ));
        // An unrelated old hardware snapshot cannot erase the requested rate
        // or fabricate acknowledgement. The operation remains pending.
        events
            .send(ServerEvent::Hardware(HardwareStatus {
                connected: true,
                frequency_hz: 100_000_000,
                sample_rate_hz: 2_000_000,
                gain_db: 20.0,
                requested_gain_db: None,
                gain_mode: None,
                source_kind: "fake".into(),
                error: None,
            }))
            .unwrap();
        frame(&mut app, &ctx);
        assert_eq!(app.shared.lock().unwrap().source.sample_rate_hz, 2_400_000);
        assert!(app.runtime_action.is_some());
        if timeout {
            app.test_expire_runtime_action();
        } else {
            events
                .send(ServerEvent::Error {
                    message: "injected active-channel rejection".into(),
                })
                .unwrap();
        }
        frame(&mut app, &ctx);
        frame(&mut app, &ctx);
        frame(&mut app, &ctx);
        let result = &app.ai_panel.messages.last().unwrap().content;
        assert!(
            result.contains(if timeout {
                "Error: timed out awaiting sample rate"
            } else {
                "Error: daemon reported an error"
            }),
            "{result}"
        );
        assert!(!result.contains("Receiver acknowledged"));
        assert!(app.runtime_action.is_none());
        assert_eq!(
            app.shared.lock().unwrap().source.sample_rate_hz,
            2_000_000,
            "failed request did not restore authoritative capture geometry"
        );
        app.shared.lock().unwrap().source.stop();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approved_ai_adsb_request_survives_initial_daemon_handshake() {
    let daemon = crate::daemon_client::tests::TestDaemon::spawn("ai-adsb-initial").await;
    let mut app = tokio::task::block_in_place(offline_test_app);
    app.current_tab = AppTab::Meteor;
    {
        let mut state = app.shared.lock().unwrap();
        state.source.source_mode = SourceMode::Daemon;
        state.source.daemon_addr = daemon.addr.to_string();
        state.audio_running = false;
    }
    let ctx = egui::Context::default();
    let result = approved(&mut app, &ctx, "start_adsb", serde_json::json!({}));
    assert!(
        result.contains("Receiver acknowledged start ADS-B"),
        "{result}"
    );
    assert!(app.shared.lock().unwrap().source.daemon_adsb_ready());
    let result = approved(&mut app, &ctx, "stop_adsb", serde_json::json!({}));
    assert!(
        result.contains("Receiver acknowledged stop ADS-B"),
        "{result}"
    );
    daemon.shutdown().await;
}

#[test]
fn approved_ai_recording_stop_reports_finalization_failure_and_preferences_remain_truthful() {
    let mut app = offline_test_app();
    let ctx = egui::Context::default();
    let dir = private_dir("finalization");
    fake_local(&mut app, Arc::new(AtomicBool::new(false)));
    app.shared.lock().unwrap().source.start();
    wait(&mut app, &ctx, |app| {
        app.shared.lock().unwrap().source.capture_tuning_applied()
    });
    app.recorder_panel.output_dir = dir.to_string_lossy().into();
    let result = approved(&mut app, &ctx, "start_recording", serde_json::json!({}));
    assert!(result.contains("Recorder opened"), "{result}");
    // A real read-only file handle lets WAV creation buffer its header but
    // rejects the flush/seek performed by the production stop lifecycle.
    let path = dir.join("read-only-handle");
    std::fs::write(&path, b"preserved").unwrap();
    app.recorder_panel.wav_writer = Some(
        hound::WavWriter::new(
            std::io::BufWriter::new(std::fs::File::open(&path).unwrap()),
            hound::WavSpec {
                channels: 1,
                sample_rate: 48_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap(),
    );
    let result = approved(&mut app, &ctx, "stop_recording", serde_json::json!({}));
    assert!(result.contains("Error: Failed to finalize WAV"), "{result}");
    assert!(!result.contains("Recorder stopped and finalized"));
    assert!(!app.shared.lock().unwrap().recording);
    assert_eq!(std::fs::read(path).unwrap(), b"preserved");
    app.shared.lock().unwrap().source.stop();
    let result = approved(&mut app, &ctx, "set_gain", serde_json::json!({"db": 25.0}));
    assert!(
        result.contains("preference retained while receiver is stopped"),
        "{result}"
    );
    assert!(!result.contains("Receiver acknowledged"));
    let result = approved(
        &mut app,
        &ctx,
        "set_demod",
        serde_json::json!({"mode": "nfm"}),
    );
    assert!(
        result.contains("Demodulation preference set to FM"),
        "{result}"
    );
    let result = approved(
        &mut app,
        &ctx,
        "set_squelch",
        serde_json::json!({"db": -50.0}),
    );
    assert!(result.contains("preference retained"), "{result}");
    let result = approved(
        &mut app,
        &ctx,
        "select_satellite",
        serde_json::json!({"name": "ISS"}),
    );
    assert!(result.contains("preference retained"), "{result}");
    assert!(app
        .shared
        .lock()
        .unwrap()
        .selected_satellite
        .as_ref()
        .unwrap()
        .starts_with("ISS"));
    app.shared.lock().unwrap().source.source_mode = SourceMode::Replay;
    for (tool, args) in [
        ("set_gain", serde_json::json!({"db": 20.0})),
        ("toggle_bias_tee", serde_json::json!({"on": true})),
        ("set_ppm", serde_json::json!({"ppm": 17})),
        ("start_adsb", serde_json::json!({})),
    ] {
        let result = approved(&mut app, &ctx, tool, args);
        assert!(result.contains("Error:"), "{result}");
        assert!(!result.contains("Receiver acknowledged"));
    }
    std::fs::remove_dir_all(dir).unwrap();
}
