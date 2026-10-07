// Deliberately ignored acceptance probes for the 2026-10-07 audit.
// Run explicitly with `cargo test -p ez-gui --no-default-features --lib
// audit_ -- --ignored --nocapture`. They assert the desired invariant and
// currently fail; they are not evidence that these defects have been fixed.

fn audit_tone(rate: f64, frequency: f64) -> Vec<Complex32> {
    (0..4096)
        .map(|n| {
            Complex32::from_polar(
                0.5,
                (std::f64::consts::TAU * frequency * n as f64 / rate) as f32,
            )
        })
        .collect()
}

#[test]
#[ignore = "R01: CS8 file source preserves signed bytes but declares CU8"]
fn audit_signed_file_replay() {
    let path = std::env::temp_dir().join(format!("ez-audit-signed-{}.cs8", std::process::id()));
    let signed = [0i8, 1, -1, 64, -64, 127, -128, 0];
    std::fs::write(&path, signed.map(|x| x as u8)).unwrap();
    let mut source = crate::source_manager::SourceManager::new();
    source.source_mode = crate::source_manager::SourceMode::Replay;
    source.replay_file = Some(path.to_string_lossy().into_owned());
    source.start();
    let deadline = Instant::now() + Duration::from_secs(5);
    let replayed = loop {
        if let Some(bytes) = source.recv_samples() {
            break bytes;
        }
        assert!(
            Instant::now() < deadline,
            "replay timeout: {:?}",
            source.status
        );
        std::thread::sleep(Duration::from_millis(2));
    };
    source.stop();
    source.wait_for_shutdown();
    std::fs::remove_file(path).unwrap();
    let correct = signed.map(|x| (i16::from(x) + 128) as u8);
    println!("CS8 signed={signed:?} replayed_as_CU8={replayed:?} correct_CU8={correct:?}");
    assert_eq!(
        replayed, correct,
        "the signed input must be converted before entering the CU8 data plane"
    );
}

#[test]
#[ignore = "R06: power meter does not match the SSB passband"]
fn audit_ssb_power_meter_passband() {
    let rate = 300_000.0;
    let mut meter = PowerMeter::new();
    let wanted = meter.process(&audit_tone(rate, 30.0 * rate / 4096.0), rate, 0.0, 3_000.0);
    let opposite = meter.process(&audit_tone(rate, -15.0 * rate / 4096.0), rate, 0.0, 3_000.0);
    println!(
        "USB bandwidth=3000 wanted_2197Hz_db={wanted:.3} opposite_minus1099Hz_db={opposite:.3}"
    );
    assert!(
        wanted > -10.0 && opposite < -60.0,
        "USB squelch must measure its selected positive-frequency passband"
    );
}

#[test]
#[ignore = "R07: an allowed 50 Hz CW passband can contain no FFT bins"]
fn audit_narrow_cw_power_meter() {
    let rate = 300_000.0;
    let offset = rate / 4096.0 / 2.0;
    let level = PowerMeter::new().process(&audit_tone(rate, offset), rate, offset, 50.0);
    println!("CW bandwidth=50 rate=300000 offset={offset:.6} in_band_carrier_db={level}");
    assert!(
        level > -10.0,
        "a strong carrier centered on the VFO must be measurable"
    );
}

#[test]
#[ignore = "R08: old capture generations are processed under new controls"]
fn audit_reject_old_capture_generation() {
    let stream = CaptureStream::default();
    stream.start(2);
    let (mut radio, demod) = config(None);
    radio.generation = 2;
    let desired = WorkerControl {
        radio: Some(radio),
        config: demod,
        ..Default::default()
    };
    let previews = Broadcaster::new();
    let audio = RadioAudioStream {
        bus: Broadcaster::new(),
        capture: stream,
        handled_samples: Arc::new(AtomicU64::new(0)),
        lost_samples: Arc::new(AtomicU64::new(0)),
        running: Arc::new(AtomicBool::new(true)),
        produced_samples: Arc::new(AtomicU64::new(0)),
    };
    let rx = audio.bus.subscribe(4, OverflowPolicy::DropIncoming);
    let mut processor = Processor::new();
    processor.configure(&desired);
    let mut block = CaptureBlock::fixture(
        input(16_384, 0),
        ez_proto::CaptureClock::new(10, 0, 2_400_000, None),
        1,
        0,
    );
    block.center_hz = 100_000_000;
    processor.process(&block, &desired, &previews, &audio);
    let accepted = rx.try_recv().map_or(0, |frame| frame.samples.len());
    println!("requested_generation=2 block_generation=1 accepted_audio_samples={accepted} skipped_rf_samples={}", processor.snapshot.skipped_samples);
    assert_eq!(
        processor.snapshot.skipped_samples, 16_384,
        "queued samples from the previous receiver session must be rejected"
    );
}
