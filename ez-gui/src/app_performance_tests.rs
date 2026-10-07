//! Repeatable CPU/frame evidence. This does not measure GPU presentation.
use super::*;
use eframe::App;

use super::software_renderer;

fn distribution(mut values: Vec<u64>) -> serde_json::Value {
    assert!(!values.is_empty());
    values.sort_unstable();
    let percentile = |percent: usize| values[(values.len() * percent).div_ceil(100) - 1];
    serde_json::json!({"samples":values.len(),"unit":"microseconds",
        "p50":percentile(50),"p95":percentile(95),"p99":percentile(99),
        "max":values.last().unwrap()})
}

/// Run alone with EZ_SDR_PERFORMANCE_DIR pointing to a private output folder.
#[test]
#[ignore = "writes CPU benchmark and visual evidence; run in isolation"]
fn profile_frames_with_continuous_native_recording() {
    let folder = std::path::PathBuf::from(std::env::var("EZ_SDR_PERFORMANCE_DIR").unwrap());
    std::fs::create_dir_all(&folder).unwrap();
    let recordings = folder.join("temporary-recordings");
    std::fs::create_dir_all(&recordings).unwrap();
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut config = AppConfig::default();
    config.mqtt_broker.clear();
    config.skip_antenna_checklists = true;
    config.observer_lat = 51.5;
    config.observer_lon = -0.1;
    let mut app = CentralApp::new_with_config(&cc, config);
    let mut frame = eframe::Frame::_new_kittest();
    app.theme_applied = true;
    app.adsb_panel.test_offline_map(&ctx);
    app.adsb_panel.max_age_secs = 600;
    let aircraft: Vec<_> = (0..2000)
        .map(|n| crate::adsb_panel::AircraftEntry {
            icao: 0xA00000 + n,
            callsign: format!("AUD{n:04}"),
            lat: 51.5 + f64::from(n % 50) * 0.016 - 0.4,
            lon: -0.1 + f64::from(n / 50) * 0.026 - 0.5,
            altitude: Some(18_000 + (n % 80) as i32 * 100),
            speed: Some(220),
            heading: Some(n % 360),
            has_position: true,
            has_heading: true,
            seen: Instant::now(),
            position_seen: Some(Instant::now()),
            reliable: true,
            score: 1.0,
            quality_known: true,
            ..Default::default()
        })
        .collect();
    let period: Vec<u8> = (0..16_384)
        .flat_map(|n| {
            let t = f64::from(n) / 2_400_000.0;
            let amplitude = 0.45 * (1.0 + 0.4 * (std::f64::consts::TAU * 1000.0 * t).sin());
            let phase = std::f64::consts::TAU * 100_000.0 * t;
            [phase.cos(), phase.sin()]
                .map(|v| (v * amplitude * 128.0 + 127.4).round().clamp(0.0, 255.0) as u8)
        })
        .collect();
    let stream = {
        let mut state = app.shared.lock().unwrap();
        state.source.source_mode = crate::source_manager::SourceMode::Hardware;
        state.source.frequency_hz = 118_100_000;
        state.source.center_frequency_hz = Some(118_000_000);
        state.source.sample_rate_hz = 2_400_000;
        state.config.advanced.rf_decim = 8;
        state.config.advanced.rf_dc_remove = true;
        state.audio_running = false; // No physical audio device is opened.
        state.scheduler.auto_tune_enabled = false;
        state.scheduler.custom_tasks.clear();
        crate::radio_ui::select_demod(&mut state, crate::sdr_panel::DemodMode::Am);
        let input = period.clone();
        state
            .source
            .test_set_hardware_open(move |_| Ok(input.clone()));
        state.source.capture_stream()
    };
    app.logic(&ctx, &mut frame);
    app.recorder_panel.output_dir = recordings.display().to_string();
    app.recorder_panel.filename_template = "profile".into();
    app.recorder_panel.record_audio = true;
    app.satellite_panel.cf32_output_dir = recordings.display().to_string();
    app.recorder_panel.start_recording();
    app.satellite_panel.start_cf32_recording();
    assert!(
        app.recorder_panel.recording,
        "{}",
        app.recorder_panel.last_error
    );
    assert!(app.satellite_panel.cf32_recording);
    let cf32_path = app
        .satellite_panel
        .cf32_writer
        .as_ref()
        .unwrap()
        .path()
        .to_path_buf();
    app.demod_worker.test_start_native_timing();
    app.shared.lock().unwrap().source.start();
    let mut results = Vec::new();
    let mut previews = Vec::new();
    for (name, tab, width, height, drag) in [
        ("spectrum-wide", AppTab::Listen, 1920, 1012, false),
        ("spectrum-small", AppTab::Listen, 1000, 700, false),
        ("dense-aircraft", AppTab::Planes, 1920, 1012, false),
        ("dense-map-drag", AppTab::Planes, 1920, 1012, true),
        ("satellite-map", AppTab::Satellites, 1400, 900, false),
    ] {
        app.current_tab = tab;
        let camera_before = app.adsb_panel.test_map_camera();
        let mut logic_times = Vec::new();
        let mut ui_times = Vec::new();
        let mut mesh_times = Vec::new();
        let mut total_times = Vec::new();
        let mut updates = egui::TexturesDelta::default();
        let started = Instant::now();
        let mut last = None;
        for n in 0..130 {
            let pos = egui::pos2(950.0 + n as f32 * 0.75, 520.0 + n as f32 * 0.25);
            let mut events = Vec::new();
            if drag {
                events.push(egui::Event::PointerMoved(pos));
                if n == 0 || n == 129 {
                    events.push(egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: n == 0,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
            }
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width as f32, height as f32),
                )),
                events,
                ..Default::default()
            };
            let start = Instant::now();
            let mut logic = None;
            let mut ui_time = 0;
            let mut fixture_time = 0;
            let mut output = ctx.run_ui(input, |ui| {
                if logic.is_none() {
                    let logic_start = Instant::now();
                    app.logic(&ctx, &mut frame);
                    logic = Some(logic_start.elapsed().as_micros() as u64);
                    // Replay display fixtures after source-transition cleanup.
                    // Fixture injection is excluded from measured frame CPU.
                    let fixture_start = Instant::now();
                    if app.adsb_panel.aircraft.len() != aircraft.len() {
                        app.adsb_panel.aircraft.clone_from(&aircraft);
                    }
                    fixture_time += fixture_start.elapsed().as_micros() as u64;
                }
                let ui_start = Instant::now();
                app.ui(ui, &mut frame);
                ui_time += ui_start.elapsed().as_micros() as u64;
            });
            {
                let state = app.shared.lock().unwrap();
                assert_eq!(
                    state.source.capture_center_frequency_hz(),
                    118_000_000,
                    "capture center changed while rendering {name}, frame {n}"
                );
                assert_eq!(
                    state.source.sample_rate_hz, 2_400_000,
                    "capture rate changed while rendering {name}, frame {n}"
                );
            }
            let mesh_start = Instant::now();
            let meshes = ctx.tessellate(output.shapes.clone(), output.pixels_per_point);
            let mesh_time = mesh_start.elapsed().as_micros() as u64;
            let total = (start.elapsed().as_micros() as u64).saturating_sub(fixture_time);
            assert!(!meshes.is_empty());
            assert_eq!(app.adsb_panel.aircraft.len(), 2000);
            updates.append(std::mem::take(&mut output.textures_delta));
            if n >= 10 {
                logic_times.push(logic.unwrap());
                ui_times.push(ui_time);
                mesh_times.push(mesh_time);
                total_times.push(total);
            }
            last = Some(output);
            std::thread::sleep(
                (started + Duration::from_secs_f64(f64::from(n + 1) / 60.0))
                    .saturating_duration_since(Instant::now()),
            );
        }
        if drag {
            assert_ne!(
                camera_before,
                app.adsb_panel.test_map_camera(),
                "map did not pan"
            );
        }
        let mut output = last.unwrap();
        output.textures_delta = updates;
        previews.push((name, output, width, height));
        results.push(
            serde_json::json!({"case":name,"screen":[width,height],"warmup_frames":10,
            "logic":distribution(logic_times),"ui_draw":distribution(ui_times),
            "tessellation":distribution(mesh_times),"frame_cpu":distribution(total_times)}),
        );
    }
    {
        let mut state = app.shared.lock().unwrap();
        state.source.stop();
        state.source.wait_for_shutdown();
    }
    app.recorder_panel.stop_and_wait();
    app.satellite_panel.stop_cf32_and_wait();
    assert!(
        app.recorder_panel.last_error.is_empty(),
        "{}",
        app.recorder_panel.last_error
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while !app.demod_worker.radio_snapshot().unwrap().complete {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    let radio = app.demod_worker.radio_snapshot().unwrap();
    let source = stream.progress();
    assert_eq!(radio.received_samples, source.samples);
    assert_eq!(radio.lost_samples, 0);
    let raw = std::fs::read(recordings.join("profile.iq")).unwrap();
    assert_eq!(raw.len() as u64, source.samples * 2);
    assert!(raw.chunks_exact(period.len()).all(|block| block == period));
    let converted = std::fs::read(&cf32_path).unwrap();
    assert_eq!(converted.len() as u64, source.samples * 8);
    assert!(converted == lrpt_decode::uc8_bytes_to_cf32_le(&raw));
    let raw_meta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(recordings.join("profile.json")).unwrap()).unwrap();
    let cf_meta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(cf32_path.with_extension("json")).unwrap()).unwrap();
    let audio_meta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(recordings.join("profile.wav.json")).unwrap())
            .unwrap();
    assert_eq!(raw_meta["complete"], true);
    assert_eq!(cf_meta["complete"], true);
    assert_eq!(audio_meta["complete"], true);
    assert_eq!(raw_meta["capture"]["samples"], source.samples);
    assert_eq!(cf_meta["capture"]["samples"], source.samples);
    assert_eq!(audio_meta["audio_capture"]["samples"], source.samples);
    assert_eq!(audio_meta["audio_samples"], radio.audio_samples);
    let native_timings = app.demod_worker.test_take_native_timing();
    assert_eq!(native_timings.len(), (source.blocks as usize).min(8192));
    let evidence = serde_json::json!({"mode":"debug CPU/headless; no GPU or physical devices",
        "percentiles":"nearest rank, 120 measured frames per case, 10 warmup frames excluded",
        "rf_processing_scope":"native Processor::process per 16384-sample block; excludes receive/configure/wait and timing-storage overhead",
        "cases":results,"native_rf_processing":distribution(native_timings),
        "native_timing_cap":8192,"native_timing_truncated":source.blocks>8192,
        "native_samples":source.samples,"native_blocks":source.blocks,"radio_lost_samples":radio.lost_samples,
        "raw_bytes":raw.len(),"cf32_bytes":converted.len(),"wav_samples":radio.audio_samples,
        "all_recordings_complete":true,"map_drag_verified":true,"network":"disconnected map fixture; no tile/geolocation requests"});
    std::fs::write(
        folder.join("performance.json"),
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
    println!("{}", serde_json::to_string_pretty(&evidence).unwrap());
    // Visual QA occurs after capture stops and is excluded from all timings.
    let mut renderer = software_renderer::SoftwareRenderer::default();
    for (name, output, width, height) in previews {
        renderer
            .render(&ctx, output, width, height)
            .save(folder.join(format!("{name}.png")))
            .unwrap();
    }
    std::fs::remove_dir_all(recordings).unwrap();
}
