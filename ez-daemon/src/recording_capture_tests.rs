use super::*;
use crate::broadcast::OverflowPolicy;
use crate::bus::{SampleBlock, SampleBus};
use ez_proto::CaptureClock;

fn wait(mut condition: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(3);
    while Instant::now() < end {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("recording lifecycle timed out");
}

#[test]
fn daemon_files_stop_before_epoch_tuning_rate_rewind_and_loss_boundaries() {
    for format in [RecordingFormat::Cf32, RecordingFormat::RawU8] {
        for boundary in 0..6 {
            let dir = std::env::temp_dir().join(format!(
                "ez-r05-daemon-{}-{}-{boundary}-{format:?}",
                std::process::id(),
                now_ms()
            ));
            let bus = SampleBus::new();
            let mut manager = RecordingManager::new(&dir);
            manager
                .start_recording(
                    1,
                    format,
                    1090,
                    bus.subscribe(8, OverflowPolicy::DropOldest),
                )
                .unwrap();
            let raw_a = [90, 110].repeat(64);
            let raw_b = [140, 160].repeat(32);
            let a = lrpt_decode::uc8_bytes_to_normalized(&raw_a);
            let b = lrpt_decode::uc8_bytes_to_normalized(&raw_b);
            let mut clock = CaptureClock::new(12, 700, 16_800_003, None);
            clock.sample_stride = 7;
            clock.discontinuous = true;
            let publish = |samples: Vec<Complex32>, start, capture, center, rate| {
                bus.publish(SampleBlock {
                    product_session: 0,
                    observed_at: None,
                    capture: Some(capture),
                    start_sample: start,
                    sample_rate_hz: rate,
                    center_freq_hz: center,
                    samples: Arc::from(samples),
                });
            };
            publish(a.clone(), 0, clock, 1090, 2_400_000);
            let mut next = clock.offset_samples(64).unwrap();
            next.discontinuous = false;
            publish(b.clone(), 64, next, 1090, 2_400_000);
            let sample_bytes = if format == RecordingFormat::Cf32 {
                8
            } else {
                2
            };
            wait(|| manager.status(1).unwrap().bytes_written == 96 * sample_bytes);
            let mut bad = next.offset_samples(32).unwrap();
            let mut center = 1090;
            let mut rate = 2_400_000;
            match boundary {
                0 => {
                    bad.epoch += 1;
                    bad.source_sample_index = 0;
                }
                1 => center += 100,
                2 => {
                    bad.source_sample_rate_hz = 14_000_000;
                    rate = 2_000_000;
                }
                3 => bad.source_sample_index += 7,
                4 => bad.discontinuous = true,
                _ => bad.origin_unix_ns = Some(1),
            }
            publish(vec![Complex32::new(0.99, 0.99); 17], 96, bad, center, rate);
            wait(|| !manager.status(1).unwrap().active);
            let ended = manager.status(1).unwrap();
            assert!(ended
                .stop_reason
                .as_ref()
                .unwrap()
                .contains("Recording stopped"));
            let file = PathBuf::from(ended.path.clone().unwrap());
            let expected = if format == RecordingFormat::Cf32 {
                complex_to_cf32_le_bytes(&[a.clone(), b.clone()].concat())
            } else {
                [raw_a.clone(), raw_b.clone()].concat()
            };
            assert_eq!(std::fs::read(&file).unwrap(), expected);
            let metadata: serde_json::Value =
                serde_json::from_slice(&std::fs::read(format!("{}.json", file.display())).unwrap())
                    .unwrap();
            assert_eq!(metadata["frequency_hz"], 1090);
            assert_eq!(metadata["sample_rate_hz"], 2_400_000);
            assert_eq!(metadata["capture"]["first_clock"]["epoch"], 12);
            assert_eq!(
                metadata["capture"]["first_clock"]["source_sample_index"],
                700
            );
            assert_eq!(metadata["capture"]["samples"], 96);
            assert_eq!(metadata["capture"]["next_source_sample_index"], 1372);
            assert_eq!(metadata["complete"], false);
            assert!(
                (metadata["duration_sec"].as_f64().unwrap() - 96.0 * 7.0 / 16_800_003.0).abs()
                    < 1e-12
            );
            use crate::hardware::{
                replay::{FileReplaySource, ReplayFormat},
                IqSource,
            };
            let replay_format = if format == RecordingFormat::Cf32 {
                ReplayFormat::Cf32Le
            } else {
                ReplayFormat::RawU8
            };
            let mut replay = FileReplaySource::open(
                file.to_str().unwrap(),
                replay_format,
                2_400_000,
                false,
                1000.0,
            )
            .unwrap();
            replay.start().unwrap();
            let mut replayed = vec![Complex32::default(); 96];
            assert_eq!(replay.read_iq(&mut replayed).unwrap(), 96);
            assert_eq!(replayed, [a, b].concat());
            replay.stop();
            // A finished channel can immediately begin a separate recording;
            // the previous file and its metadata remain intact.
            manager
                .start_recording(
                    1,
                    format,
                    center,
                    bus.subscribe(8, OverflowPolicy::DropOldest),
                )
                .unwrap();
            let second_path = manager.status(1).unwrap().path.unwrap();
            assert_ne!(second_path, ended.path.unwrap());
            manager.stop_recording(1).unwrap();
            assert_eq!(std::fs::read(file).unwrap(), expected);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}

#[test]
fn callback_overflow_resets_channel_dsp_and_marks_recording_prefix_incomplete() {
    use crate::channelizer::Channelizer;
    use crate::hardware::{callback_iq, IqSource};
    struct CallbackSource(callback_iq::CallbackReader);
    impl IqSource for CallbackSource {
        fn start(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
        fn stop(&mut self) {}
        fn set_frequency(&mut self, _: u64) -> anyhow::Result<()> {
            Ok(())
        }
        fn set_sample_rate(&mut self, _: u32) -> anyhow::Result<()> {
            Ok(())
        }
        fn set_gain(&mut self, _: f64) -> anyhow::Result<()> {
            Ok(())
        }
        fn read_iq(&mut self, buf: &mut [Complex32]) -> anyhow::Result<usize> {
            self.0.read(buf, 2_400_000)
        }
        fn capture_clock(&self) -> Option<CaptureClock> {
            self.0.capture_clock()
        }
        fn frequency_hz(&self) -> u64 {
            137_100_000
        }
        fn sample_rate_hz(&self) -> u32 {
            2_400_000
        }
        fn gain_db(&self) -> f64 {
            0.0
        }
        fn kind(&self) -> &'static str {
            "callback-overflow-injection"
        }
    }
    for format in [RecordingFormat::Cf32, RecordingFormat::RawU8] {
        let dir = std::env::temp_dir().join(format!(
            "ez-callback-loss-{}-{}-{format:?}",
            std::process::id(),
            now_ms()
        ));
        let bus = SampleBus::new();
        let observed = bus.subscribe(16, crate::bus::OverflowPolicy::DropOldest);
        let mut channelizer = Channelizer::new(
            bus.subscribe(16, crate::bus::OverflowPolicy::DropOldest),
            137_100_000,
            2_400_000,
        );
        let (_, channel) = channelizer.add_channel(137_100_000, 1_200_000).unwrap();
        let mut manager = RecordingManager::new(&dir);
        manager
            .start_recording(
                1,
                format,
                137_100_000,
                bus.subscribe(16, crate::bus::OverflowPolicy::DropOldest),
            )
            .unwrap();
        let (producer, reader) = callback_iq::channel(1, true);
        let prefix = [0, 16].repeat(64);
        assert!(producer.push(&prefix));
        assert!(producer.push(&[20, 30].repeat(32))); // Queue is full before ingestion starts.
        assert_eq!(producer.dropped_samples(), 32);
        let running = Arc::new(AtomicBool::new(true));
        let (_, thread) =
            crate::ingest::spawn(Box::new(CallbackSource(reader)), bus, Arc::clone(&running))
                .unwrap();
        let first = observed.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(channelizer.tick(Duration::from_secs(2)));
        let first_channel = channel.recv_timeout(Duration::from_secs(2)).unwrap();
        let sample_bytes = if format == RecordingFormat::Cf32 {
            8
        } else {
            2
        };
        wait(|| manager.status(1).unwrap().bytes_written == 64 * sample_bytes);
        assert!(producer.push(&[64, 192].repeat(32)));
        let after_gap = observed.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(after_gap.start_sample, 96);
        assert_eq!(after_gap.capture_clock().source_sample_index, 96);
        assert_eq!(after_gap.capture_clock().epoch, first.capture_clock().epoch);
        assert!(after_gap.capture_clock().discontinuous);
        assert!(channelizer.tick(Duration::from_secs(2)));
        let reset_channel = channel.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_ne!(
            reset_channel.capture_clock().epoch,
            first_channel.capture_clock().epoch
        );
        assert!(reset_channel.capture_clock().discontinuous);
        // A fresh DDC at the retained acquisition index must match after reset.
        let reference_bus = SampleBus::new();
        let mut reference = Channelizer::new(
            reference_bus.subscribe(4, crate::bus::OverflowPolicy::DropOldest),
            137_100_000,
            2_400_000,
        );
        let (_, reference_channel) = reference.add_channel(137_100_000, 1_200_000).unwrap();
        reference_bus.publish(after_gap);
        assert!(reference.tick(Duration::from_secs(2)));
        assert_eq!(
            reset_channel.samples,
            reference_channel
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .samples
        );
        wait(|| !manager.status(1).unwrap().active);
        let status = manager.status(1).unwrap();
        let file = PathBuf::from(status.path.unwrap());
        let raw = prefix.iter().map(|byte| byte ^ 0x80).collect::<Vec<_>>();
        let expected = if format == RecordingFormat::Cf32 {
            complex_to_cf32_le_bytes(&lrpt_decode::uc8_bytes_to_normalized(&raw))
        } else {
            raw
        };
        assert_eq!(std::fs::read(&file).unwrap(), expected);
        let metadata: serde_json::Value =
            serde_json::from_slice(&std::fs::read(format!("{}.json", file.display())).unwrap())
                .unwrap();
        assert_eq!(metadata["complete"], false);
        assert_eq!(metadata["capture"]["samples"], 64);
        assert_eq!(metadata["capture"]["next_source_sample_index"], 64);
        assert!(metadata["stop_reason"]
            .as_str()
            .unwrap()
            .contains("Recording stopped"));
        running.store(false, Ordering::Relaxed);
        thread.join().unwrap();
        manager.stop_recording(1).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}
