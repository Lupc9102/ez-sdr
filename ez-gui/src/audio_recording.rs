//! WAV ingress and disk work consume continuous demodulated frames.

use crate::capture_recording::RecordingProgress;
use crate::demod::RadioAudioStream;
use arc_swap::ArcSwap;
use ez_daemon::broadcast::OverflowPolicy;
use std::sync::{atomic::Ordering, Arc};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Default)]
struct Control {
    stop: Option<(Instant, u64)>,
    deadline: Option<Instant>,
}

pub(crate) struct AudioResult {
    pub capture: Option<ez_proto::RecordingCapture>,
    pub bytes: u64,
    pub error: Option<String>,
    pub metadata: serde_json::Value,
}

pub(crate) struct AudioRecorder {
    control: Arc<ArcSwap<Control>>,
    progress: Arc<ArcSwap<RecordingProgress>>,
    stream: RadioAudioStream,
    peak: Arc<std::sync::atomic::AtomicU32>,
    thread: Option<std::thread::JoinHandle<AudioResult>>,
}

impl AudioRecorder {
    pub fn start<W: std::io::Write + std::io::Seek + Send + 'static>(
        stream: RadioAudioStream,
        mut writer: hound::WavWriter<W>,
        sidecar: std::path::PathBuf,
        mut metadata: serde_json::Value,
        deadline: Option<Instant>,
    ) -> Self {
        let input = stream.bus.subscribe(128, OverflowPolicy::DropIncoming);
        let control = Arc::new(ArcSwap::from_pointee(Control {
            deadline,
            ..Default::default()
        }));
        let progress = Arc::new(ArcSwap::from_pointee(RecordingProgress::default()));
        let worker_control = Arc::clone(&control);
        let worker_progress = Arc::clone(&progress);
        let worker_stream = stream.clone();
        let rate = writer.spec().sample_rate;
        let channels = writer.spec().channels;
        let initial_produced = stream.produced_samples.load(Ordering::Acquire);
        let peak = Arc::new(std::sync::atomic::AtomicU32::new((-120.0_f32).to_bits()));
        let worker_peak = Arc::clone(&peak);
        let started = Instant::now();
        let initial_loss = stream.lost_samples.load(Ordering::Acquire);
        let thread = std::thread::spawn(move || {
            let mut capture = None;
            let mut next_sequence = None;
            let mut bytes = 0_u64;
            let mut error = None;
            let mut partial_frame = false;
            let mut end_target = None;
            let mut published = Instant::now();
            loop {
                let block: Option<Arc<crate::demod::RadioAudio>> =
                    input.recv_timeout(Duration::from_millis(10));
                if worker_stream.lost_samples.load(Ordering::Acquire) != initial_loss {
                    error = Some("Recording stopped: radio capture samples were lost".into());
                    break;
                }
                if !worker_stream.running.load(Ordering::Acquire) {
                    error = Some("Recording stopped: radio worker stopped".into());
                    break;
                }
                let controls = *worker_control.load_full();
                let end = match (controls.stop, controls.deadline) {
                    (Some((stop, _)), Some(deadline)) => Some(stop.min(deadline)),
                    (Some((stop, _)), None) => Some(stop),
                    (None, deadline) => deadline,
                };
                if end.is_some_and(|end| Instant::now() >= end) && end_target.is_none() {
                    end_target = Some(controls.stop.map_or_else(
                        || worker_stream.capture.total_samples(),
                        |(_, target)| target,
                    ));
                }
                if input.dropped_count() > 0 {
                    error = Some("Recording stopped: WAV input queue overflow".to_string());
                    break;
                }
                if let Some(block) = block {
                    if block.observed_at < started {
                        continue;
                    }
                    if end.is_some_and(|end| block.observed_at > end) {
                        break;
                    }
                    if let Some(reason) = &block.error {
                        error = Some(reason.clone());
                        break;
                    }
                    let native_rate =
                        block.clock.source_sample_rate_hz / block.clock.sample_stride.max(1);
                    if metadata["frequency_hz"].as_u64() != Some(block.center_hz)
                        || metadata["sample_rate_hz"].as_u64() != Some(u64::from(native_rate))
                        || block.channels != channels
                        || !block
                            .samples
                            .len()
                            .is_multiple_of(usize::from(channels.max(1)))
                        || block.rate != rate
                        || next_sequence.is_some_and(|next| next != block.sequence)
                    {
                        error =
                            Some("Recording stopped: WAV clock or frame sequence changed".into());
                        break;
                    }
                    let accepted = ez_proto::RecordingCapture::accept(
                        capture.as_ref(),
                        block.clock,
                        block.center_hz,
                        native_rate,
                        block.input_samples,
                    );
                    let accepted = match accepted {
                        Ok(capture) => capture,
                        Err(reason) => {
                            error = Some(reason);
                            break;
                        }
                    };
                    capture = Some(accepted);
                    for &sample in &block.samples {
                        let sample = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
                        if let Err(reason) = writer.write_sample(sample) {
                            error = Some(format!("WAV write error: {reason}"));
                            partial_frame = true;
                            break;
                        }
                        bytes += 2;
                    }
                    let amplitude = block
                        .samples
                        .iter()
                        .map(|sample| sample.abs())
                        .fold(0.0_f32, f32::max);
                    let db = 20.0 * amplitude.max(1e-6).log10();
                    let old_peak = f32::from_bits(worker_peak.load(Ordering::Relaxed));
                    worker_peak.store(old_peak.max(db).to_bits(), Ordering::Release);
                    if error.is_some() {
                        break;
                    }
                    next_sequence = Some(block.sequence + 1);
                } else if end_target.is_some_and(|target| {
                    worker_stream.handled_samples.load(Ordering::Acquire) >= target
                }) {
                    break;
                }
                if published.elapsed() >= Duration::from_millis(16) {
                    worker_progress.store(Arc::new(RecordingProgress {
                        capture: capture.clone(),
                        bytes_written: bytes,
                        error: error.clone(),
                        stopped: false,
                    }));
                    published = Instant::now();
                }
            }
            let drops = input.dropped_count();
            drop(input);
            if let Err(reason) = writer.finalize() {
                error.get_or_insert_with(|| format!("WAV finalize error: {reason}"));
            }
            metadata["format"] = "wav16".into();
            metadata["audio_capture"] = serde_json::to_value(&capture).unwrap_or_default();
            metadata["audio_samples"] = (bytes / 2).into();
            metadata["audio_frames"] = (bytes / 2 / u64::from(channels)).into();
            metadata["audio_channels"] = channels.into();
            metadata["audio_native_partial_frame"] = partial_frame.into();
            metadata["audio_bytes_total"] = bytes.into();
            metadata["audio_sample_rate_hz"] = rate.into();
            metadata["audio_duration_sec"] =
                (bytes as f64 / 2.0 / f64::from(channels) / f64::from(rate)).into();
            metadata["audio_input_dropped_blocks"] = drops.into();
            metadata["audio_input_lost_samples"] = if drops > 0 {
                worker_stream
                    .produced_samples
                    .load(Ordering::Acquire)
                    .saturating_sub(initial_produced)
                    .saturating_sub(capture.as_ref().map_or(0, |capture| capture.samples))
            } else {
                0
            }
            .into();
            metadata["peak_level_dbfs"] =
                f32::from_bits(worker_peak.load(Ordering::Acquire)).into();
            metadata["audio_native_lost_samples"] = worker_stream
                .lost_samples
                .load(Ordering::Acquire)
                .saturating_sub(initial_loss)
                .into();
            metadata["complete"] = error.is_none().into();
            metadata["stop_reason"] = error.clone().into();
            metadata["end_utc"] = chrono::Utc::now().to_rfc3339().into();
            let saved = std::fs::File::create(&sidecar)
                .map_err(|reason| reason.to_string())
                .and_then(|file| {
                    serde_json::to_writer_pretty(file, &metadata)
                        .map_err(|reason| reason.to_string())
                });
            if let Err(reason) = saved {
                error.get_or_insert_with(|| format!("WAV metadata error: {reason}"));
            }
            worker_progress.store(Arc::new(RecordingProgress {
                capture: capture.clone(),
                bytes_written: bytes,
                error: error.clone(),
                stopped: true,
            }));
            AudioResult {
                capture,
                bytes,
                error,
                metadata,
            }
        });
        Self {
            control,
            progress,
            stream,
            peak,
            thread: Some(thread),
        }
    }

    pub fn progress(&self) -> Arc<RecordingProgress> {
        self.progress.load_full()
    }

    pub fn peak_level(&self) -> f32 {
        f32::from_bits(self.peak.load(Ordering::Acquire))
    }

    pub fn set_deadline(&self, deadline: Option<Instant>) {
        let old = *self.control.load_full();
        if old.deadline != deadline {
            self.control.store(Arc::new(Control { deadline, ..old }));
        }
    }

    pub fn finish(&mut self) -> Result<AudioResult, String> {
        self.request_stop();
        self.thread
            .take()
            .ok_or_else(|| "WAV already finalized".to_string())?
            .join()
            .map_err(|_| "WAV worker panicked".to_string())
    }

    pub fn request_stop(&self) {
        let old = *self.control.load_full();
        if old.stop.is_none() {
            self.control.store(Arc::new(Control {
                stop: Some((Instant::now(), self.stream.capture.total_samples())),
                ..old
            }));
        }
    }
}

impl Drop for AudioRecorder {
    fn drop(&mut self) {
        self.request_stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demod::{DemodWorker, RadioAudio};

    fn fixture() -> (DemodWorker, RadioAudioStream, std::path::PathBuf) {
        let worker = DemodWorker::new_capture(crate::capture_stream::CaptureStream::default());
        let stream = RadioAudioStream {
            lost_samples: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            ..worker.audio_stream().unwrap()
        };
        let dir = std::env::temp_dir().join(format!(
            "ez-a09-audio-{}-{}",
            std::process::id(),
            ez_proto::new_capture_epoch()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        (worker, stream, dir)
    }

    fn recorder(
        stream: RadioAudioStream,
        dir: &std::path::Path,
        deadline: Option<Instant>,
    ) -> AudioRecorder {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let writer = hound::WavWriter::create(dir.join("audio.wav"), spec).unwrap();
        AudioRecorder::start(
            stream,
            writer,
            dir.join("audio.json"),
            serde_json::json!({"frequency_hz":100_000_000, "sample_rate_hz":2_400_000}),
            deadline,
        )
    }

    fn block(sequence: u64) -> RadioAudio {
        RadioAudio {
            channels: 1,
            samples: vec![0.25; 16],
            rate: 48_000,
            clock: ez_proto::CaptureClock::new(10, (sequence - 1) * 1000, 2_400_000, None),
            center_hz: 100_000_000,
            input_samples: 1000,
            observed_at: Instant::now(),
            sequence,
            error: None,
        }
    }

    #[test]
    fn native_wav_boundaries_finalize_the_valid_prefix_without_gui_ticks() {
        for boundary in 0..7 {
            let (worker, stream, dir) = fixture();
            let mut recorder = recorder(stream.clone(), &dir, None);
            stream.bus.publish(Arc::new(block(1)));
            let mut next = block(2);
            match boundary {
                0 => next.clock.epoch += 1,
                1 => next.clock.source_sample_index += 1,
                2 => next.clock.discontinuous = true,
                3 => next.clock.origin_unix_ns = Some(123),
                4 => next.center_hz += 1,
                5 => next.rate = 44_100,
                _ => next.sequence += 1,
            }
            stream.bus.publish(Arc::new(next));
            let deadline = Instant::now() + Duration::from_secs(3);
            while !recorder.progress().stopped {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
            let metadata: serde_json::Value =
                serde_json::from_slice(&std::fs::read(dir.join("audio.json")).unwrap()).unwrap();
            assert_eq!(metadata["complete"], false);
            assert_eq!(metadata["audio_samples"], 16);
            assert_eq!(metadata["audio_capture"]["samples"], 1000);
            let wav = hound::WavReader::open(dir.join("audio.wav")).unwrap();
            assert_eq!(wav.duration(), 16);
            let result = recorder.finish().unwrap();
            assert!(result.error.is_some());
            assert_eq!(result.bytes, 32);
            drop(worker);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn native_wav_deadline_and_native_loss_finalize_without_gui_ticks() {
        for loss in [false, true] {
            let (worker, stream, dir) = fixture();
            let mut recorder = recorder(stream.clone(), &dir, None);
            stream.bus.publish(Arc::new(block(1)));
            let deadline = Instant::now() + Duration::from_secs(3);
            while recorder.progress().bytes_written != 32 {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
            if loss {
                stream.lost_samples.store(10, Ordering::Release);
            } else {
                recorder.set_deadline(Some(Instant::now() + Duration::from_millis(80)));
            }
            while !recorder.progress().stopped {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
            let metadata: serde_json::Value =
                serde_json::from_slice(&std::fs::read(dir.join("audio.json")).unwrap()).unwrap();
            assert_eq!(metadata["complete"], !loss);
            assert_eq!(
                metadata["audio_native_lost_samples"],
                if loss { 10 } else { 0 }
            );
            assert_eq!(
                hound::WavReader::open(dir.join("audio.wav"))
                    .unwrap()
                    .duration(),
                16
            );
            let stopped = Instant::now();
            let result = recorder.finish().unwrap();
            assert!(stopped.elapsed() < Duration::from_millis(500));
            assert_eq!(result.error.is_some(), loss);
            drop(worker);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    struct GateFile {
        file: std::fs::File,
        gate: Option<(
            Arc<std::sync::atomic::AtomicBool>,
            crossbeam_channel::Sender<()>,
            crossbeam_channel::Receiver<()>,
        )>,
        limit: Option<u64>,
        flush_error: Arc<std::sync::atomic::AtomicBool>,
    }

    impl std::io::Write for GateFile {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if let Some((armed, entered, release)) = &self.gate {
                if armed.swap(false, Ordering::AcqRel) {
                    entered.send(()).unwrap();
                    release.recv().unwrap();
                }
            }
            let position = std::io::Seek::stream_position(&mut self.file)?;
            if self.limit.is_some_and(|limit| position >= limit) {
                return Err(std::io::Error::other("injected WAV disk write failure"));
            }
            self.file.write(bytes)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            if self.flush_error.load(Ordering::Acquire) {
                Err(std::io::Error::other("injected WAV disk flush failure"))
            } else {
                self.file.flush()
            }
        }
    }

    impl std::io::Seek for GateFile {
        fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
            self.file.seek(position)
        }
    }

    fn spec() -> hound::WavSpec {
        hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        }
    }

    #[test]
    fn blocked_wav_stop_and_panel_drop_are_responsive_and_preserve_finalization() {
        for (drop_panel, fail_flush) in [(false, false), (false, true), (true, false)] {
            let (worker, stream, dir) = fixture();
            let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let (entered, entered_rx) = crossbeam_channel::bounded(1);
            let (release, release_rx) = crossbeam_channel::bounded(1);
            let flush_error = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let file = GateFile {
                file: std::fs::File::create(dir.join("audio.wav")).unwrap(),
                gate: Some((Arc::clone(&armed), entered, release_rx)),
                limit: None,
                flush_error: Arc::clone(&flush_error),
            };
            let recorder = AudioRecorder::start(
                stream.clone(),
                hound::WavWriter::new(file, spec()).unwrap(),
                dir.join("audio.json"),
                serde_json::json!({"frequency_hz":100_000_000,"sample_rate_hz":2_400_000}),
                None,
            );
            let mut panel =
                crate::recorder_panel::RecorderPanel::new(crate::test_helpers::make_shared_state());
            panel.test_set_audio_recorder(recorder);
            armed.store(true, Ordering::Release);
            stream.bus.publish(Arc::new(block(1)));
            entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            let started = Instant::now();
            panel.stop();
            let elapsed = started.elapsed();
            assert!(
                elapsed < Duration::from_millis(50),
                "WAV Stop blocked for {elapsed:?}"
            );
            assert!(panel.is_finalizing() && panel.recording);
            assert!(!dir.join("audio.json").exists());
            let mut panel = Some(panel);
            if drop_panel {
                let started = Instant::now();
                drop(panel.take());
                assert!(started.elapsed() < Duration::from_millis(50));
            }
            flush_error.store(fail_flush, Ordering::Release);
            release.send(()).unwrap();
            if let Some(panel) = &mut panel {
                panel.stop_and_wait();
                assert!(!panel.recording);
                assert_eq!(!panel.last_error.is_empty(), fail_flush);
                assert_eq!(panel.bytes_written, 32);
            }
            let deadline = Instant::now() + Duration::from_secs(3);
            let metadata = loop {
                if let Some(metadata) = std::fs::read(dir.join("audio.json"))
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                {
                    break metadata;
                }
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            };
            assert_eq!(metadata["complete"], !fail_flush);
            assert_eq!(metadata["audio_samples"], 16);
            assert_eq!(metadata["audio_capture"]["samples"], 1000);
            assert_eq!(
                hound::WavReader::open(dir.join("audio.wav"))
                    .unwrap()
                    .duration(),
                16
            );
            eprintln!("Blocked WAV Stop: {} us; drop={drop_panel}, flush_error={fail_flush}, finalized 16 PCM samples", elapsed.as_micros());
            drop(worker);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    fn publish(stream: &RadioAudioStream, sequence: u64) {
        let block = block(sequence);
        stream
            .produced_samples
            .fetch_add(block.input_samples, Ordering::Release);
        stream.bus.publish(Arc::new(block));
    }

    #[test]
    fn native_wav_overflow_keeps_a_prefix_accounts_for_loss_and_isolates_consumers() {
        let (worker, stream, dir) = fixture();
        let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (entered, entered_rx) = crossbeam_channel::bounded(1);
        let (release, release_rx) = crossbeam_channel::bounded(1);
        let file = GateFile {
            file: std::fs::File::create(dir.join("audio.wav")).unwrap(),
            gate: Some((Arc::clone(&armed), entered, release_rx)),
            limit: None,
            flush_error: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        let writer = hound::WavWriter::new(file, spec()).unwrap();
        let mut recorder = AudioRecorder::start(
            stream.clone(),
            writer,
            dir.join("audio.json"),
            serde_json::json!({"frequency_hz":100_000_000, "sample_rate_hz":2_400_000}),
            None,
        );
        let fast = stream.bus.subscribe(512, OverflowPolicy::DropIncoming);
        publish(&stream, 1);
        let deadline = Instant::now() + Duration::from_secs(3);
        while recorder.progress().bytes_written != 32 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        armed.store(true, Ordering::Release);
        publish(&stream, 2);
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        for n in 3..=302 {
            publish(&stream, n);
        }
        release.send(()).unwrap();
        while !recorder.progress().stopped {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        let result = recorder.finish().unwrap();
        assert!(result.error.unwrap().contains("input queue overflow"));
        assert_eq!(result.metadata["audio_input_dropped_blocks"], 172);
        assert_eq!(result.metadata["audio_input_lost_samples"], 300_000);
        assert_eq!(result.metadata["complete"], false);
        assert_eq!(
            hound::WavReader::open(dir.join("audio.wav"))
                .unwrap()
                .duration(),
            32
        );
        assert_eq!(fast.queued_len(), 302);
        assert_eq!(fast.dropped_count(), 0);
        publish(&stream, 303);
        assert_eq!(
            hound::WavReader::open(dir.join("audio.wav"))
                .unwrap()
                .duration(),
            32
        );
        drop(worker);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn native_wav_disk_faults_preserve_the_actual_pcm_prefix_and_stop_promptly() {
        for flush in [false, true] {
            let (worker, stream, dir) = fixture();
            let flush_error = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let file = GateFile {
                file: std::fs::File::create(dir.join("audio.wav")).unwrap(),
                gate: None,
                limit: (!flush).then_some(44 + 8),
                flush_error: Arc::clone(&flush_error),
            };
            let writer = hound::WavWriter::new(file, spec()).unwrap();
            let mut recorder = AudioRecorder::start(
                stream.clone(),
                writer,
                dir.join("audio.json"),
                serde_json::json!({"frequency_hz":100_000_000,"sample_rate_hz":2_400_000}),
                None,
            );
            publish(&stream, 1);
            let deadline = Instant::now() + Duration::from_secs(3);
            while if flush {
                recorder.progress().bytes_written != 32
            } else {
                !recorder.progress().stopped
            } {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
            flush_error.store(flush, Ordering::Release);
            let stopped = Instant::now();
            let result = recorder.finish().unwrap();
            assert!(stopped.elapsed() < Duration::from_millis(500));
            assert!(result.error.as_ref().unwrap().contains(if flush {
                "flush failure"
            } else {
                "write failure"
            }));
            assert_eq!(result.metadata["complete"], false);
            assert_eq!(result.bytes, if flush { 32 } else { 8 });
            assert_eq!(
                hound::WavReader::open(dir.join("audio.wav"))
                    .unwrap()
                    .duration(),
                if flush { 16 } else { 4 }
            );
            assert_eq!(result.metadata["audio_native_partial_frame"], !flush);
            println!(
                "A09 native WAV {} failure: stop/join {} us, {} PCM bytes preserved",
                if flush { "flush" } else { "write" },
                stopped.elapsed().as_micros(),
                result.bytes
            );
            drop(worker);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}
