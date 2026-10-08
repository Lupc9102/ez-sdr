use std::path::PathBuf;

use crate::capture_recording::{CaptureRecordingWriter, RecordingProgress};
use crate::source_manager::CaptureBlock;

pub(crate) fn spawn_disk_writer<W: std::io::Write + Send + 'static>(
    mut writer: W,
) -> (
    crossbeam_channel::Sender<Vec<u8>>,
    std::thread::JoinHandle<Result<u64, String>>,
) {
    let (tx, rx) = crossbeam_channel::bounded::<Vec<u8>>(128);
    let disk = std::thread::spawn(move || {
        let mut total = 0;
        for chunk in rx {
            writer
                .write_all(&chunk)
                .map_err(|error| format!("Write error: {error}"))?;
            total += chunk.len() as u64;
        }
        writer
            .flush()
            .map_err(|error| format!("Flush error: {error}"))?;
        Ok(total)
    });
    (tx, disk)
}

pub(crate) struct RawRecordingResult {
    pub capture: Option<ez_proto::RecordingCapture>,
    pub bytes_written: u64,
    pub metadata: serde_json::Value,
    pub error: Option<String>,
    pub dropped_chunks: u64,
}

pub(crate) struct RawCaptureWriter {
    tx: Option<crossbeam_channel::Sender<Vec<u8>>>,
    disk: Option<std::thread::JoinHandle<Result<u64, String>>>,
    path: PathBuf,
    sidecar: PathBuf,
    metadata: serde_json::Value,
    capture: Option<ez_proto::RecordingCapture>,
    bytes_written: u64,
    error: Option<String>,
    dropped_chunks: u64,
    audio_pending: bool,
}

impl RawCaptureWriter {
    pub fn new(
        tx: crossbeam_channel::Sender<Vec<u8>>,
        disk: std::thread::JoinHandle<Result<u64, String>>,
        path: PathBuf,
        sidecar: PathBuf,
        metadata: serde_json::Value,
        audio_pending: bool,
    ) -> Self {
        Self {
            tx: Some(tx),
            disk: Some(disk),
            path,
            sidecar,
            metadata,
            capture: None,
            bytes_written: 0,
            error: None,
            dropped_chunks: 0,
            audio_pending,
        }
    }
}

impl CaptureRecordingWriter for RawCaptureWriter {
    type Final = RawRecordingResult;

    fn write_capture(&mut self, block: &CaptureBlock) -> Result<(), String> {
        if block.float_samples.is_some() {
            self.metadata["input_format"] = "cf32_normalized".into();
            self.metadata["conversion"] = "cu8_compatibility_quantized_clipped".into();
        }
        if block.samples.is_empty() {
            return Ok(());
        }
        let rate = block.clock.source_sample_rate_hz / block.clock.sample_stride.max(1);
        if self.metadata["frequency_hz"].as_u64() != Some(block.center_hz)
            || self.metadata["sample_rate_hz"].as_u64() != Some(u64::from(rate))
        {
            return Err(
                "Recording stopped: capture tuning or sample rate differs from the file header"
                    .into(),
            );
        }
        if !block.samples.len().is_multiple_of(2) {
            return Err("Recording stopped: incomplete I/Q sample".into());
        }
        let capture = ez_proto::RecordingCapture::accept(
            self.capture.as_ref(),
            block.clock,
            block.center_hz,
            rate,
            (block.samples.len() / 2) as u64,
        )?;
        let tx = self
            .tx
            .as_ref()
            .ok_or_else(|| "IQ writer is stopped".to_string())?;
        match tx.try_send(block.samples.clone()) {
            Ok(()) => {
                self.bytes_written += block.samples.len() as u64;
                self.capture = Some(capture);
                Ok(())
            }
            Err(crossbeam_channel::TrySendError::Full(_)) => {
                self.dropped_chunks += 1;
                self.metadata["lost_samples"] = ((block.samples.len() / 2) as u64).into();
                Err("Recording stopped: IQ writer queue overflow".into())
            }
            Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                Err("IQ writer disconnected".into())
            }
        }
    }

    fn progress(&self) -> RecordingProgress {
        RecordingProgress {
            capture: self.capture.clone(),
            bytes_written: self.bytes_written,
            error: self.error.clone().or_else(|| {
                self.disk
                    .as_ref()
                    .filter(|disk| disk.is_finished())
                    .map(|_| "IQ disk writer stopped unexpectedly".into())
            }),
            stopped: false,
        }
    }

    fn fail(&mut self, error: String) {
        if self.error.is_none() {
            self.error = Some(error);
        }
    }

    fn input_overflow(&mut self, blocks: u64, samples: u64) {
        self.metadata["capture_input_dropped_blocks"] = blocks.into();
        self.metadata["capture_input_lost_samples"] = samples.into();
    }

    fn source_ended(&mut self) {
        self.metadata["stop_reason"] = "capture session ended".into();
    }

    fn finish(&mut self) -> RawRecordingResult {
        self.tx.take();
        if let Some(disk) = self.disk.take() {
            match disk.join() {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => {
                    if self.error.as_deref().is_some_and(|error| {
                        matches!(
                            error,
                            "IQ disk writer stopped unexpectedly" | "IQ writer disconnected"
                        )
                    }) {
                        self.error = Some(error);
                    } else {
                        self.fail(error);
                    }
                }
                Err(_) => self.fail("IQ disk writer panicked".into()),
            }
        }
        match std::fs::metadata(&self.path) {
            Ok(file) => {
                self.bytes_written = file.len();
                if let Some(capture) = &mut self.capture {
                    if capture.samples.saturating_mul(2) != self.bytes_written {
                        self.metadata["lost_samples"] = capture
                            .samples
                            .saturating_sub(self.bytes_written / 2)
                            .into();
                        self.error.get_or_insert_with(|| {
                            "IQ disk writer did not preserve the accepted prefix".into()
                        });
                        capture.samples = self.bytes_written / 2;
                        capture.next_source_sample_index = capture
                            .first_clock
                            .offset_samples(capture.samples)
                            .map_or(capture.first_clock.source_sample_index, |clock| {
                                clock.source_sample_index
                            });
                    }
                }
                if !self.bytes_written.is_multiple_of(2) {
                    self.fail("IQ disk writer left an incomplete sample".into());
                }
            }
            Err(error) => self.fail(format!("IQ file metadata failed: {error}")),
        }
        self.metadata["iq_bytes_total"] = self.bytes_written.into();
        self.metadata["capture"] =
            serde_json::to_value(&self.capture).unwrap_or(serde_json::Value::Null);
        if let Some(capture) = &self.capture {
            self.metadata["duration_sec"] = (capture.samples as f64
                * f64::from(capture.first_clock.sample_stride)
                / f64::from(capture.first_clock.source_sample_rate_hz))
            .into();
        }
        self.metadata["complete"] = (self.error.is_none() && !self.audio_pending).into();
        if let Some(error) = &self.error {
            self.metadata["stop_reason"] = error.clone().into();
        }
        self.metadata["end_utc"] = chrono::Utc::now().to_rfc3339().into();
        let sidecar = std::fs::File::create(&self.sidecar)
            .map_err(|error| error.to_string())
            .and_then(|file| {
                serde_json::to_writer_pretty(file, &self.metadata)
                    .map_err(|error| error.to_string())
            });
        if let Err(error) = sidecar {
            self.fail(format!("Failed to finalize recording metadata: {error}"));
        }
        RawRecordingResult {
            capture: self.capture.clone(),
            bytes_written: self.bytes_written,
            metadata: self.metadata.clone(),
            error: self.error.clone(),
            dropped_chunks: self.dropped_chunks,
        }
    }
}

impl Drop for RawCaptureWriter {
    fn drop(&mut self) {
        if self.disk.is_some() {
            self.fail("Capture writer interrupted during shutdown".into());
            let _ = self.finish();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture_recording::CaptureRecorder;
    use crate::capture_stream::CaptureStream;
    use std::time::Duration;

    #[test]
    fn native_raw_disk_failures_finalize_the_actual_prefix_and_stop_promptly() {
        for flush_failure in [false, true] {
            let dir = std::env::temp_dir().join(format!(
                "ez-a09-raw-fault-{}-{}",
                std::process::id(),
                ez_proto::new_capture_epoch()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("raw.iq");
            let file = std::fs::File::create(&path).unwrap();
            let disk_writer = crate::test_helpers::FaultFileWriter::new(
                file,
                (!flush_failure).then_some(8),
                flush_failure,
            );
            let (tx, disk) = spawn_disk_writer(disk_writer);
            let metadata = serde_json::json!({ "frequency_hz": 1_090_000_000u64, "sample_rate_hz": 2_400_000u32 });
            let sink = RawCaptureWriter::new(
                tx,
                disk,
                path.clone(),
                dir.join("raw.json"),
                metadata,
                false,
            );
            let stream = CaptureStream::default();
            stream.start(1);
            let mut recorder = CaptureRecorder::start(stream.clone(), sink, None);
            let input = [90, 110].repeat(10);
            let mut block = CaptureBlock::fixture(
                input.clone(),
                ez_proto::CaptureClock::new(9, 100, 2_400_000, None),
                1,
                0,
            );
            block.session = Some(stream.session());
            stream.publish(&block);
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            if flush_failure {
                while recorder.progress().capture.is_none() {
                    assert!(std::time::Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(2));
                }
            } else {
                while !recorder.progress().stopped {
                    assert!(std::time::Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
            let stopped = std::time::Instant::now();
            let result = recorder.finish().unwrap();
            let latency = stopped.elapsed();
            assert!(latency < Duration::from_millis(500));
            let expected = if flush_failure {
                input.as_slice()
            } else {
                &input[..8]
            };
            assert_eq!(std::fs::read(&path).unwrap(), expected);
            assert!(result.error.as_deref().unwrap().contains(if flush_failure {
                "flush failure"
            } else {
                "write failure"
            }));
            let meta: serde_json::Value =
                serde_json::from_slice(&std::fs::read(dir.join("raw.json")).unwrap()).unwrap();
            assert_eq!(meta["complete"], false);
            assert_eq!(meta["capture"]["samples"], expected.len() as u64 / 2);
            assert_eq!(
                meta["capture"]["next_source_sample_index"],
                100 + expected.len() as u64 / 2
            );
            assert_eq!(
                meta["lost_samples"].as_u64().unwrap_or(0),
                (input.len() - expected.len()) as u64 / 2
            );
            assert_eq!(result.bytes_written, expected.len() as u64);
            println!(
                "A09 native raw {} failure: stop/join {} us; {} preserved bytes",
                if flush_failure { "flush" } else { "write" },
                latency.as_micros(),
                result.bytes_written
            );
            drop(recorder);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    struct GateWriter<W> {
        writer: W,
        entered: crossbeam_channel::Sender<()>,
        release: crossbeam_channel::Receiver<()>,
        first: bool,
    }

    impl<W: CaptureRecordingWriter> CaptureRecordingWriter for GateWriter<W> {
        type Final = W::Final;
        fn write_capture(&mut self, block: &CaptureBlock) -> Result<(), String> {
            self.writer.write_capture(block)?;
            if self.first {
                self.first = false;
                self.entered.send(()).unwrap();
                self.release.recv_timeout(Duration::from_secs(3)).unwrap();
            }
            Ok(())
        }
        fn progress(&self) -> RecordingProgress {
            self.writer.progress()
        }
        fn fail(&mut self, error: String) {
            self.writer.fail(error);
        }
        fn input_overflow(&mut self, blocks: u64, samples: u64) {
            self.writer.input_overflow(blocks, samples);
        }
        fn source_ended(&mut self) {
            self.writer.source_ended();
        }
        fn finish(&mut self) -> Self::Final {
            self.writer.finish()
        }
    }

    #[test]
    fn iq_only_recorder_drains_eof_and_never_crosses_a_restarted_session() {
        for (empty, restart) in [(false, false), (true, false), (false, true), (true, true)] {
            let dir =
                std::env::temp_dir().join(format!("ez-iq-eof-{}", ez_proto::new_capture_epoch()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("raw.iq");
            let (tx, disk) = spawn_disk_writer(std::fs::File::create(&path).unwrap());
            let sink = RawCaptureWriter::new(
                tx,
                disk,
                path.clone(),
                dir.join("raw.json"),
                serde_json::json!({ "frequency_hz": 1_090_000_000u64, "sample_rate_hz": 2_400_000u32 }),
                false,
            );
            let stream = CaptureStream::default();
            stream.start(1);
            let owner = stream.session();
            let mut recorder = CaptureRecorder::start(stream.clone(), sink, None);
            if !empty {
                for index in 0..4 {
                    let mut block = CaptureBlock::fixture(
                        vec![90, 110],
                        ez_proto::CaptureClock::new(9, index, 2_400_000, None),
                        1,
                        index,
                    );
                    block.session = Some(owner.clone());
                    stream.publish(&block);
                }
            }
            owner.finish();
            // Restart immediately, with identical tuning. Its data is excluded.
            if restart {
                stream.start(2);
                let mut replacement = CaptureBlock::fixture(
                    vec![255, 0],
                    ez_proto::CaptureClock::new(10, 0, 2_400_000, None),
                    2,
                    0,
                );
                replacement.session = Some(stream.session());
                stream.publish(&replacement);
            }
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while !recorder.progress().stopped {
                assert!(
                    std::time::Instant::now() < deadline,
                    "EOF did not finalize IQ recording"
                );
                std::thread::sleep(Duration::from_millis(2));
            }
            let result = recorder.finish().unwrap();
            assert_eq!(
                std::fs::read(path).unwrap(),
                if empty { vec![] } else { [90, 110].repeat(4) }
            );
            assert!(result.error.is_none(), "{:?}", result.error);
            assert_eq!(result.metadata["stop_reason"], "capture session ended");
            assert_eq!(result.metadata["complete"], true);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn cf32_recording_keeps_native_float_precision_and_headroom() {
        let dir =
            std::env::temp_dir().join(format!("ez-native-cf32-{}", ez_proto::new_capture_epoch()));
        let mut writer = crate::satellite::recorder::Cf32StreamWriter::start(
            dir.to_str().unwrap(),
            None,
            2_400_000,
            1_090_000_000,
            0.0,
            0.0,
        )
        .unwrap();
        let samples = [
            num_complex::Complex32::new(0.0001, -0.0002),
            num_complex::Complex32::new(1.2, -1.3),
        ];
        let mut block = CaptureBlock::fixture(
            vec![128, 128, 255, 0],
            ez_proto::CaptureClock::new(9, 0, 2_400_000, None),
            1,
            0,
        );
        block.float_samples = Some(std::sync::Arc::from(samples));
        CaptureRecordingWriter::write_capture(&mut writer, &block).unwrap();
        let meta = writer.stop().unwrap();
        let bytes = std::fs::read(writer.path()).unwrap();
        let expected: Vec<_> = samples
            .into_iter()
            .flat_map(|sample| {
                [sample.re, sample.im]
                    .into_iter()
                    .flat_map(f32::to_le_bytes)
            })
            .collect();
        assert_eq!(bytes, expected);
        assert_eq!(meta.capture.unwrap().samples, 2);
        assert!(meta.complete);
        drop(writer);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn saturated_native_recorders_account_for_loss_finalize_prefixes_and_unsubscribe() {
        let dir = std::env::temp_dir().join(format!(
            "ez-a09-native-overflow-{}-{}",
            std::process::id(),
            ez_proto::new_capture_epoch()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("raw.iq");
        let file = std::fs::File::create(&path).unwrap();
        let (tx, disk) = spawn_disk_writer(std::io::BufWriter::new(file));
        let metadata =
            serde_json::json!({ "frequency_hz": 1_090_000_000u64, "sample_rate_hz": 2_400_000u32 });
        let raw = RawCaptureWriter::new(
            tx,
            disk,
            path.clone(),
            dir.join("raw.json"),
            metadata,
            false,
        );
        let cf32 = crate::satellite::recorder::Cf32StreamWriter::start(
            dir.to_str().unwrap(),
            None,
            2_400_000,
            1_090_000_000,
            0.0,
            0.0,
        )
        .unwrap();
        let cf_path = cf32.path().to_path_buf();
        let stream = CaptureStream::default();
        stream.start(1);
        let fast = stream.subscribe(512);
        let (entered, started) = crossbeam_channel::bounded(2);
        let (release, gate) = crossbeam_channel::bounded(2);
        let mut raw = CaptureRecorder::start(
            stream.clone(),
            GateWriter {
                writer: raw,
                entered: entered.clone(),
                release: gate.clone(),
                first: true,
            },
            None,
        );
        let mut cf32 = CaptureRecorder::start(
            stream.clone(),
            GateWriter {
                writer: cf32,
                entered,
                release: gate,
                first: true,
            },
            None,
        );
        let publish = |index: u64| {
            let mut block = CaptureBlock::fixture(
                vec![90, 110],
                ez_proto::CaptureClock::new(9, index, 2_400_000, None),
                1,
                index,
            );
            block.session = Some(stream.session());
            stream.publish(&block);
        };
        publish(0);
        started.recv_timeout(Duration::from_secs(3)).unwrap();
        started.recv_timeout(Duration::from_secs(3)).unwrap();
        for index in 1..300 {
            publish(index);
        }
        release.send(()).unwrap();
        release.send(()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while !raw.progress().stopped || !cf32.progress().stopped {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(raw
            .finish()
            .unwrap()
            .error
            .unwrap()
            .contains("capture input queue overflow"));
        let _ = cf32.finish().unwrap();
        for index in 300..320 {
            publish(index);
        }
        assert_eq!(fast.queued_len(), 320);
        assert_eq!(fast.dropped_count(), 0);
        assert_eq!(std::fs::read(&path).unwrap(), [90, 110]);
        assert_eq!(
            std::fs::read(&cf_path).unwrap(),
            lrpt_decode::uc8_bytes_to_cf32_le(&[90, 110])
        );
        for sidecar in [dir.join("raw.json"), cf_path.with_extension("json")] {
            let meta: serde_json::Value =
                serde_json::from_slice(&std::fs::read(sidecar).unwrap()).unwrap();
            assert_eq!(meta["complete"], false);
            assert_eq!(meta["capture"]["samples"], 1);
            assert_eq!(meta["capture_input_dropped_blocks"], 171);
            assert_eq!(meta["capture_input_lost_samples"], 299);
            assert!(meta["stop_reason"]
                .as_str()
                .unwrap()
                .contains("capture input queue overflow"));
        }
        drop(raw);
        drop(cf32);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
