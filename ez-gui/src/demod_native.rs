//! Continuous local radio DSP. Capture, RF state, demodulation and playback
//! never wait for the GUI or its SharedState mutex.

use super::*;
use crate::capture_stream::{CaptureSession, CaptureStream};
use crate::radio_iq::{RadioIqConfig, RadioIqProcessor, VfoMixer};
use crate::radio_power::PowerMeter;
#[cfg(test)]
use crate::radio_power::RfPassband;
use crate::radio_rds::{RdsDecoder, RdsRegion, RdsSnapshot};
use crate::radio_squelch::{CtcssSquelch, SquelchMode};
use crate::source_manager::CaptureBlock;
use arc_swap::ArcSwap;
use ez_daemon::broadcast::{Broadcaster, BroadcasterHandle, OverflowPolicy};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RadioConfig {
    pub enabled: bool,
    pub preview: bool,
    pub measure_power: bool,
    pub frequency_hz: u64,
    pub center_hz: u64,
    pub generation: u64,
    pub iq: RadioIqConfig,
    pub audio_running: bool,
    pub volume: f32,
    pub squelch_db: f32,
    pub squelch_mode: SquelchMode,
    pub ctcss_tone_hz: Option<f32>,
    pub rds_enabled: bool,
    pub rds_incremental: bool,
    pub rds_region: RdsRegion,
    pub sink: Option<crate::audio_output::AudioSink>,
}

#[derive(Clone, Default)]
pub(crate) struct RadioSnapshot {
    pub control_version: u64,
    pub generation: u64,
    pub epoch: Option<u64>,
    pub discontinuity_revision: u64,
    pub received_samples: u64,
    pub demodulated_samples: u64,
    pub audio_samples: u64,
    pub lost_samples: u64,
    pub skipped_samples: u64,
    pub dropped_blocks: u64,
    pub complete: bool,
    pub received_tone: Option<f32>,
    pub rds: Option<RdsSnapshot>,
    pub stereo_locked: bool,
    pub fm_deviation_hz: f32,
    pub audio_peak: f32,
    pub signal_db: f32,
    pub waveform: Vec<f32>,
}

pub(crate) struct RadioPreview {
    pub samples: Vec<Complex32>,
    pub clock: ez_proto::CaptureClock,
    pub observed_at: Instant,
    pub input_samples: usize,
    pub center_hz: u64,
    pub rate: f64,
    pub control_version: u64,
}

#[derive(Clone)]
pub(crate) struct RadioAudio {
    pub samples: Vec<f32>,
    pub rate: u32,
    pub clock: ez_proto::CaptureClock,
    pub center_hz: u64,
    pub input_samples: u64,
    pub observed_at: Instant,
    pub sequence: u64,
    pub error: Option<String>,
}

#[derive(Clone)]
pub(crate) struct RadioAudioStream {
    pub bus: Broadcaster<Arc<RadioAudio>>,
    pub capture: CaptureStream,
    pub handled_samples: Arc<AtomicU64>,
    pub lost_samples: Arc<AtomicU64>,
    pub running: Arc<AtomicBool>,
    pub produced_samples: Arc<AtomicU64>,
}

pub(super) struct RadioState {
    output: Arc<ArcSwap<RadioSnapshot>>,
    previews: BroadcasterHandle<Arc<RadioPreview>>,
    audio: RadioAudioStream,
    #[cfg(test)]
    timing: Arc<(AtomicBool, Mutex<Vec<u64>>)>,
}

struct Processor {
    iq: RadioIqProcessor,
    mixer: VfoMixer,
    demod: Demodulator,
    ctcss: CtcssSquelch,
    rds: RdsDecoder,
    power: PowerMeter,
    config: Option<(RadioConfig, DemodConfig)>,
    expected: Option<(u64, u64, ez_proto::CaptureClock)>,
    session: Option<Arc<CaptureSession>>,
    next_stream: u64,
    snapshot: RadioSnapshot,
    advanced_version: u64,
    reset_version: u64,
    sequence: u64,
    rf_active: bool,
    demod_active: bool,
}

impl Processor {
    fn new() -> Self {
        Self {
            iq: RadioIqProcessor::default(),
            mixer: VfoMixer::new(2_048_000.0),
            demod: Demodulator::new(),
            ctcss: CtcssSquelch::new(),
            rds: RdsDecoder::new(),
            power: PowerMeter::new(),
            config: None,
            expected: None,
            session: None,
            next_stream: 0,
            snapshot: RadioSnapshot::default(),
            advanced_version: 0,
            reset_version: 0,
            sequence: 0,
            rf_active: false,
            demod_active: false,
        }
    }

    fn reset(&mut self) {
        self.iq.reset();
        self.mixer.reset();
        self.demod.reset();
        self.ctcss.reset();
        self.rds.reset();
        self.power.reset();
        self.snapshot.waveform.clear();
        self.snapshot.rds = None;
        self.snapshot.received_tone = None;
        self.snapshot.stereo_locked = false;
        self.snapshot.fm_deviation_hz = 0.0;
        self.snapshot.audio_peak = 0.0;
        self.snapshot.signal_db = -120.0;
        self.snapshot.discontinuity_revision += 1;
        self.rf_active = false;
        self.demod_active = false;
    }

    fn configure(&mut self, desired: &WorkerControl) {
        if desired.advanced_version != self.advanced_version {
            if let Some((advanced, mode)) = &desired.advanced {
                self.demod.apply_advanced(advanced, *mode);
            }
            self.advanced_version = desired.advanced_version;
        }
        if let Some(radio) = &desired.radio {
            let changed = self.config.as_ref().is_none_or(|(old, old_demod)| {
                old.enabled != radio.enabled
                    || old.frequency_hz != radio.frequency_hz
                    || old.center_hz != radio.center_hz
                    || old.iq != radio.iq
                    || *old_demod != desired.config
                    || old.audio_running != radio.audio_running
                    || old.squelch_mode != radio.squelch_mode
                    || old.ctcss_tone_hz != radio.ctcss_tone_hz
                    || old.rds_enabled != radio.rds_enabled
            });
            if changed || desired.reset_version != self.reset_version {
                self.reset();
                self.expected = None;
            }
            self.iq.configure(radio.iq);
            self.mixer.configure(
                self.iq.output_rate(),
                (i128::from(radio.frequency_hz) - i128::from(radio.center_hz)) as f64,
            );
            self.rds.set_incremental(radio.rds_incremental);
            self.rds.set_region(radio.rds_region);
            self.config = Some((radio.clone(), desired.config));
        }
        self.reset_version = desired.reset_version;
        self.demod
            .set_sample_rates_exact(desired.config.input_rate, desired.config.audio_rate);
        self.demod.set_lpf_cutoff(desired.config.lpf_cutoff);
        self.demod.set_rf_bandwidth(desired.config.rf_bandwidth);
        self.snapshot.control_version = desired.version;
    }

    fn process(
        &mut self,
        block: &CaptureBlock,
        desired: &WorkerControl,
        previews: &Broadcaster<Arc<RadioPreview>>,
        audio: &RadioAudioStream,
    ) {
        let samples = (block.samples.len() / 2) as u64;
        if self.snapshot.generation != block.source_generation {
            self.account_tail();
            self.reset();
            self.expected = None;
            self.snapshot.epoch = None;
            self.next_stream = 0;
            self.session.clone_from(&block.session);
            self.snapshot.generation = block.source_generation;
        }
        let missing = block.stream_sample_index.saturating_sub(self.next_stream);
        self.snapshot.lost_samples += missing;
        self.snapshot.received_samples += samples;
        self.next_stream = block.stream_sample_index + samples;
        self.snapshot.complete = false;
        if desired
            .radio
            .as_ref()
            .is_some_and(|radio| block.source_generation != radio.generation)
        {
            self.snapshot.skipped_samples += samples;
            self.expected = None;
            // A stale queue tail must not publish an error into a newly opened
            // recorder or reset/demodulate under that session's controls.
            return;
        }
        let continuous = self
            .expected
            .as_ref()
            .is_some_and(|(generation, center, clock)| {
                *generation == block.source_generation
                    && *center == block.center_hz
                    && *clock == block.clock
            });
        if !continuous || missing > 0 {
            self.reset();
            self.mixer.seek_capture(block.clock);
        }
        self.snapshot.epoch = Some(block.clock.epoch);
        self.expected = block.clock.offset_samples(samples).map(|mut next| {
            next.discontinuous = false;
            (block.source_generation, block.center_hz, next)
        });
        let Some(radio) = &desired.radio else {
            self.snapshot.skipped_samples += samples;
            self.expected = None;
            self.reject_audio(block, desired, audio);
            return;
        };
        if !radio.enabled
            || block.center_hz != radio.center_hz
            || block.clock.exact_sample_rate_hz() != f64::from(radio.iq.input_rate)
        {
            self.snapshot.skipped_samples += samples;
            self.expected = None;
            self.reject_audio(block, desired, audio);
            return;
        }
        let active = radio.audio_running
            || audio.bus.subscriber_count() > 0
            || (radio.rds_enabled && desired.config.mode == DemodMode::Wfm)
            || (radio.squelch_mode.uses_ctcss() && desired.config.mode == DemodMode::Fm);
        if !active && !radio.preview && !radio.measure_power {
            self.rf_active = false;
            self.demod_active = false;
            return;
        }
        let rf_resumed = !self.rf_active;
        if rf_resumed {
            self.reset();
            self.mixer.seek_capture(block.clock);
            self.rf_active = true;
        }
        let wideband = if let Some(samples) = &block.float_samples {
            self.iq.process_complex(samples)
        } else {
            self.iq.process(&block.samples)
        };
        let offset = (i128::from(radio.frequency_hz) - i128::from(block.center_hz)) as f64;
        let signal = if radio.measure_power || radio.squelch_mode == SquelchMode::Power {
            self.power.process(
                &wideband,
                self.iq.output_rate(),
                offset,
                self.demod.rf_passband(desired.config.mode),
            )
        } else {
            -120.0
        };
        self.snapshot.signal_db = signal;
        let resumed = active && !self.demod_active;
        if resumed {
            self.demod.reset();
            self.ctcss.reset();
            self.rds.reset();
            self.mixer.seek_capture(block.clock);
            self.snapshot.waveform.clear();
        }
        self.demod_active = active;
        if active {
            let channel = self.mixer.process(&wideband);
            let mut playback = if desired.config.stereo {
                self.demod
                    .demodulate_stereo_complex(&channel, desired.config.mode)
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
            } else {
                self.demod.demodulate_complex(&channel, desired.config.mode)
            };
            self.snapshot.demodulated_samples += samples;
            self.snapshot.fm_deviation_hz = self.demod.last_fm_deviation_hz;
            self.snapshot.audio_peak = self.demod.last_audio_peak;
            self.snapshot.stereo_locked = desired.config.stereo && self.demod.last_stereo_locked;
            let subaudible = self.demod.take_nfm_subaudible_audio();
            if !desired.config.stereo
                && desired.config.mode == DemodMode::Fm
                && radio.squelch_mode.uses_ctcss()
            {
                let gains =
                    self.ctcss
                        .process(&subaudible, desired.config.audio_rate, radio.ctcss_tone_hz);
                self.snapshot.received_tone = self.ctcss.status().detected_tone_hz;
                if radio.squelch_mode == SquelchMode::CtcssMute {
                    for (index, sample) in playback.iter_mut().enumerate() {
                        *sample *= gains.get(index).copied().unwrap_or(0.0);
                    }
                }
            }
            let (multiplex, multiplex_rate) = self.demod.take_wfm_multiplex();
            if radio.rds_enabled && desired.config.mode == DemodMode::Wfm {
                self.rds.process_multiplex(&multiplex, multiplex_rate);
                self.snapshot.rds = Some(self.rds.snapshot());
            }
            let gate = if crate::radio_ui::mode_has_squelch(desired.config.mode)
                && radio.squelch_mode == SquelchMode::Power
                && signal <= radio.squelch_db
            {
                0.0
            } else {
                1.0
            };
            for sample in &mut playback {
                *sample *= radio.volume * gate;
            }
            let mono = if desired.config.stereo {
                playback
                    .chunks_exact(2)
                    .map(|pair| (pair[0] + pair[1]) * 0.5)
                    .collect::<Vec<_>>()
            } else {
                playback.clone()
            };
            self.snapshot.audio_samples += mono.len() as u64;
            self.snapshot.waveform.extend_from_slice(&mono);
            let excess = self.snapshot.waveform.len().saturating_sub(2048);
            self.snapshot.waveform.drain(..excess);
            self.sequence += 1;
            if audio.bus.subscriber_count() > 0 {
                let mut clock = block.clock;
                clock.discontinuous |= !continuous || missing > 0 || resumed;
                audio.produced_samples.fetch_add(samples, Ordering::Release);
                audio.bus.publish(Arc::new(RadioAudio {
                    samples: mono,
                    rate: desired.config.audio_rate,
                    clock,
                    center_hz: block.center_hz,
                    input_samples: samples,
                    observed_at: block.observed_at,
                    sequence: self.sequence,
                    error: None,
                }));
            }
            if radio.audio_running {
                if let Some(sink) = &radio.sink {
                    sink.push(playback);
                }
            }
        }
        let mut preview_clock = block.clock;
        preview_clock.discontinuous |= !continuous || missing > 0 || rf_resumed;
        if radio.preview {
            previews.publish(Arc::new(RadioPreview {
                samples: wideband,
                clock: preview_clock,
                observed_at: block.observed_at,
                input_samples: samples as usize,
                center_hz: block.center_hz,
                rate: self.iq.output_rate(),
                control_version: desired.version,
            }));
        }
    }

    fn reject_audio(
        &mut self,
        block: &CaptureBlock,
        desired: &WorkerControl,
        audio: &RadioAudioStream,
    ) {
        if audio.bus.subscriber_count() == 0 {
            return;
        }
        self.sequence += 1;
        audio.bus.publish(Arc::new(RadioAudio {
            samples: Vec::new(),
            rate: desired.config.audio_rate,
            clock: block.clock,
            center_hz: block.center_hz,
            input_samples: 0,
            observed_at: block.observed_at,
            sequence: self.sequence,
            error: Some("Recording stopped: radio settings no longer match the capture".into()),
        }));
    }

    fn account_tail(&mut self) {
        if let Some(session) = &self.session {
            let progress = session.progress();
            if !progress.live {
                self.snapshot.lost_samples += progress.samples.saturating_sub(self.next_stream);
                self.next_stream = progress.samples;
            }
        }
    }
}

impl DemodWorker {
    pub(crate) fn new_capture(stream: CaptureStream) -> Self {
        let input = stream.subscribe(128);
        let control = Arc::new(Mutex::new(WorkerControl::default()));
        let (control_wake_tx, control_wake_rx) = crossbeam_channel::bounded(1);
        let applied_version = Arc::new(AtomicU64::new(0));
        let running = Arc::new(AtomicBool::new(true));
        let dropped_iq = Arc::new(AtomicU64::new(0));
        let output = Arc::new(ArcSwap::from_pointee(RadioSnapshot::default()));
        let previews = Broadcaster::new();
        let preview_rx = previews.subscribe(4, OverflowPolicy::DropOldest);
        let audio = RadioAudioStream {
            bus: Broadcaster::new(),
            capture: stream.clone(),
            handled_samples: Arc::new(AtomicU64::new(0)),
            lost_samples: Arc::new(AtomicU64::new(0)),
            running: Arc::clone(&running),
            produced_samples: Arc::new(AtomicU64::new(0)),
        };
        let worker_control = Arc::clone(&control);
        let worker_running = Arc::clone(&running);
        let worker_applied = Arc::clone(&applied_version);
        let worker_drops = Arc::clone(&dropped_iq);
        let worker_output = Arc::clone(&output);
        let worker_audio = audio.clone();
        #[cfg(test)]
        let timing = Arc::new((AtomicBool::new(false), Mutex::new(Vec::new())));
        #[cfg(test)]
        let worker_timing = Arc::clone(&timing);
        let handle = std::thread::spawn(move || {
            struct RunningFlag(Arc<AtomicBool>);
            impl Drop for RunningFlag {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::Release);
                }
            }
            let _running_flag = RunningFlag(Arc::clone(&worker_running));
            let mut processor = Processor::new();
            let mut published = Instant::now();
            while worker_running.load(Ordering::Acquire) {
                let block = input.recv_timeout(Duration::from_millis(10));
                let desired = worker_control
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clone();
                while control_wake_rx.try_recv().is_ok() {}
                processor.configure(&desired);
                worker_applied.store(desired.version, Ordering::Release);
                if let Some(block) = block {
                    #[cfg(test)]
                    let started = worker_timing.0.load(Ordering::Relaxed).then(Instant::now);
                    processor.process(&block, &desired, &previews, &worker_audio);
                    #[cfg(test)]
                    if let Some(started) = started {
                        let elapsed = started.elapsed().as_micros() as u64;
                        let mut timings = worker_timing.1.lock().unwrap();
                        if timings.len() < 8192 {
                            timings.push(elapsed);
                        }
                    }
                }
                let current = stream.progress();
                let finished = current.generation > 0 && !current.live && input.queued_len() == 0;
                if finished {
                    processor.account_tail();
                    processor.snapshot.lost_samples = stream
                        .total_samples()
                        .saturating_sub(processor.snapshot.received_samples);
                    processor.snapshot.complete = true;
                    if current.generation != processor.snapshot.generation {
                        processor.reset();
                        processor.snapshot.generation = current.generation;
                        processor.snapshot.epoch = None;
                        processor.session = None;
                    }
                }
                processor.snapshot.dropped_blocks = input.dropped_count();
                worker_audio.handled_samples.store(
                    processor.snapshot.received_samples + processor.snapshot.lost_samples,
                    Ordering::Release,
                );
                worker_audio
                    .lost_samples
                    .store(processor.snapshot.lost_samples, Ordering::Release);
                worker_drops.store(input.dropped_count(), Ordering::Relaxed);
                if finished
                    || published.elapsed() >= Duration::from_millis(16)
                    || processor.snapshot.control_version != worker_output.load().control_version
                {
                    worker_output.store(Arc::new(processor.snapshot.clone()));
                    published = Instant::now();
                }
            }
        });
        let (_, audio_rx) = crossbeam_channel::bounded(1);
        Self {
            native: Some(RadioState {
                output,
                previews: preview_rx,
                audio,
                #[cfg(test)]
                timing,
            }),
            iq_tx: None,
            control,
            control_wake_tx,
            applied_version,
            running,
            audio_rx,
            handle: Some(handle),
            dropped_iq,
            dropped_frames: Arc::new(AtomicU64::new(0)),
        }
    }

    pub(crate) fn is_native(&self) -> bool {
        self.native.is_some()
    }

    pub(crate) fn configure_radio(&self, config: RadioConfig, demod: DemodConfig) -> bool {
        self.update_control(|desired| {
            if desired.radio.as_ref() == Some(&config) && desired.config == demod {
                return false;
            }
            desired.radio = Some(config);
            desired.config = demod;
            true
        })
    }

    pub(crate) fn radio_snapshot(&self) -> Option<Arc<RadioSnapshot>> {
        self.native.as_ref().map(|native| native.output.load_full())
    }

    #[cfg(test)]
    pub(crate) fn test_start_native_timing(&self) {
        self.native
            .as_ref()
            .unwrap()
            .timing
            .0
            .store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn test_take_native_timing(&self) -> Vec<u64> {
        let timing = &self.native.as_ref().unwrap().timing;
        timing.0.store(false, Ordering::Release);
        std::mem::take(&mut *timing.1.lock().unwrap())
    }

    pub(crate) fn radio_previews(&self) -> Vec<Arc<RadioPreview>> {
        let Some(native) = &self.native else {
            return Vec::new();
        };
        (0..4).filter_map(|_| native.previews.try_recv()).collect()
    }

    pub(crate) fn audio_stream(&self) -> Option<RadioAudioStream> {
        self.native.as_ref().map(|native| native.audio.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    include!("demod_native_audit.rs");

    fn config(sink: Option<crate::audio_output::AudioSink>) -> (RadioConfig, DemodConfig) {
        (
            RadioConfig {
                enabled: true,
                preview: true,
                measure_power: false,
                frequency_hz: 100_100_000,
                center_hz: 100_000_000,
                generation: 1,
                iq: RadioIqConfig {
                    input_rate: 2_400_000,
                    decimation: 8,
                    dc_remove: true,
                    invert: false,
                },
                audio_running: true,
                volume: 0.6,
                squelch_db: -50.0,
                squelch_mode: SquelchMode::Off,
                ctcss_tone_hz: None,
                rds_enabled: false,
                rds_incremental: false,
                rds_region: RdsRegion::Europe,
                sink,
            },
            DemodConfig {
                mode: DemodMode::Am,
                input_rate: 300_000.0,
                audio_rate: 48_000,
                lpf_cutoff: 5_000.0,
                rf_bandwidth: 12_500.0,
                stereo: false,
            },
        )
    }

    fn input(count: usize, start: usize) -> Vec<u8> {
        (start..start + count)
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
            .collect()
    }

    #[test]
    fn native_wav_rejects_capture_geometry_before_radio_controls_catch_up() {
        for changed_rate in [false, true] {
            let stream = CaptureStream::default();
            stream.start(1);
            let worker = DemodWorker::new_capture(stream.clone());
            let (radio, demod) = config(None);
            worker.configure_radio(radio, demod);
            let deadline = Instant::now() + Duration::from_secs(10);
            while worker.applied_version() < worker.requested_version() {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
            let dir = std::env::temp_dir().join(format!(
                "ez-a09-radio-wav-{}-{}",
                std::process::id(),
                ez_proto::new_capture_epoch()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let writer = hound::WavWriter::create(
                dir.join("audio.wav"),
                hound::WavSpec {
                    channels: 1,
                    sample_rate: 48_000,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                },
            )
            .unwrap();
            let mut recording = crate::audio_recording::AudioRecorder::start(
                worker.audio_stream().unwrap(),
                writer,
                dir.join("audio.json"),
                serde_json::json!({"frequency_hz":100_000_000, "sample_rate_hz":2_400_000}),
                None,
            );
            let mut block = CaptureBlock::fixture(
                input(16_384, 0),
                ez_proto::CaptureClock::new(10, 0, 2_400_000, None),
                1,
                0,
            );
            block.center_hz = 100_000_000;
            block.session = Some(stream.session());
            stream.publish(&block);
            let deadline = Instant::now() + Duration::from_secs(10);
            while recording.progress().bytes_written == 0 {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
            let prefix_bytes = recording.progress().bytes_written;
            block.clock = ez_proto::CaptureClock::new(
                11,
                0,
                if changed_rate { 2_000_000 } else { 2_400_000 },
                None,
            );
            block.stream_sample_index = 16_384;
            if !changed_rate {
                block.center_hz = 101_000_000;
            }
            stream.publish(&block);
            let deadline = Instant::now() + Duration::from_secs(10);
            while !recording.progress().stopped {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
            let result = recording.finish().unwrap();
            assert_eq!(result.bytes, prefix_bytes);
            assert_eq!(result.capture.as_ref().unwrap().samples, 16_384);
            assert_eq!(result.capture.as_ref().unwrap().first_clock.epoch, 10);
            assert!(result.error.unwrap().contains("no longer match"));
            let sidecar: serde_json::Value =
                serde_json::from_slice(&std::fs::read(dir.join("audio.json")).unwrap()).unwrap();
            assert_eq!(sidecar["complete"], false);
            assert!(sidecar["stop_reason"]
                .as_str()
                .unwrap()
                .contains("no longer match"));
            let reader = hound::WavReader::open(dir.join("audio.wav")).unwrap();
            assert_eq!(u64::from(reader.duration()), prefix_bytes / 2);
            stream.session().finish();
            drop(recording);
            drop(worker);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn native_radio_delivers_all_rf_and_playback_without_gui_reads() {
        let stream = CaptureStream::default();
        stream.start(1);
        let worker = DemodWorker::new_capture(stream.clone());
        let (sink, playback) = crate::audio_output::AudioSink::fixture(256);
        let (radio, demod) = config(Some(sink));
        worker.configure_radio(radio, demod);
        let audio_stream = worker.audio_stream().unwrap();
        let audio = audio_stream
            .bus
            .subscribe(256, OverflowPolicy::DropIncoming);
        let deadline = Instant::now() + Duration::from_secs(15);
        while worker.applied_version() < worker.requested_version() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        let period = input(16_384, 0);
        let started = Instant::now();
        for n in 0..120 {
            let mut block = CaptureBlock::fixture(
                period.clone(),
                ez_proto::CaptureClock::new(10, n * 16_384, 2_400_000, None),
                1,
                n * 16_384,
            );
            block.session = Some(stream.session());
            block.center_hz = 100_000_000;
            stream.publish(&block);
            std::thread::sleep(
                (started + Duration::from_secs_f64((n + 1) as f64 * 16_384.0 / 2_400_000.0))
                    .saturating_duration_since(Instant::now()),
            );
        }
        stream.session().finish();
        let deadline = Instant::now() + Duration::from_secs(15);
        while !worker.radio_snapshot().unwrap().complete {
            let snapshot = worker.radio_snapshot().unwrap();
            assert!(
                Instant::now() < deadline,
                "native radio did not finish: received={}, demodulated={}, lost={}, drops={}",
                snapshot.received_samples,
                snapshot.demodulated_samples,
                snapshot.lost_samples,
                snapshot.dropped_blocks
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        let snapshot = worker.radio_snapshot().unwrap();
        assert_eq!(snapshot.received_samples, 120 * 16_384);
        assert_eq!(snapshot.demodulated_samples, snapshot.received_samples);
        assert_eq!(snapshot.lost_samples, 0);
        assert_eq!(snapshot.skipped_samples, 0);
        assert_eq!(snapshot.dropped_blocks, 0);
        let received: Vec<_> = (0..120).map(|_| audio.try_recv().unwrap()).collect();
        let heard: Vec<_> = playback.try_iter().collect();
        assert_eq!(heard.len(), 120);
        for (heard, frame) in heard.iter().zip(&received) {
            assert_eq!(*heard, frame.samples);
        }
        assert_eq!(
            heard.iter().map(|frame| frame.len() as u64).sum::<u64>(),
            snapshot.audio_samples
        );
        assert!(heard.iter().flatten().any(|sample| sample.abs() > 0.001));
        assert_eq!(audio.dropped_count(), 0);
        assert_eq!(worker.radio_previews().len(), 4);
        println!(
            "A09 native radio/playback: {} RF samples, {} audio samples, no GUI drains, zero loss",
            snapshot.received_samples, snapshot.audio_samples
        );
    }

    #[test]
    fn native_float_replay_reaches_rf_preview_and_audio_at_sub_byte_amplitudes() {
        let stream = CaptureStream::default();
        stream.start(1);
        let (radio, demod_config) = config(None);
        let desired = WorkerControl {
            radio: Some(radio.clone()),
            config: demod_config,
            ..Default::default()
        };
        let previews = Broadcaster::new();
        let preview = previews.subscribe(4, OverflowPolicy::DropIncoming);
        let audio = RadioAudioStream {
            bus: Broadcaster::new(),
            capture: stream.clone(),
            handled_samples: Arc::new(AtomicU64::new(0)),
            lost_samples: Arc::new(AtomicU64::new(0)),
            running: Arc::new(AtomicBool::new(true)),
            produced_samples: Arc::new(AtomicU64::new(0)),
        };
        let received_audio = audio.bus.subscribe(4, OverflowPolicy::DropIncoming);
        let samples: Vec<_> = (0..131_072)
            .map(|n| {
                let time = n as f64 / 2_400_000.0;
                Complex32::from_polar(
                    (0.0001 * (1.0 + 0.4 * (std::f64::consts::TAU * 1000.0 * time).sin())) as f32,
                    (std::f64::consts::TAU * 100_000.0 * time) as f32,
                )
            })
            .collect();
        let mut block = CaptureBlock::fixture(
            vec![128; samples.len() * 2],
            ez_proto::CaptureClock::new(9, 0, 2_400_000, None),
            1,
            0,
        );
        block.center_hz = radio.center_hz;
        block.session = Some(stream.session());
        block.float_samples = Some(Arc::from(samples.clone()));
        let mut processor = Processor::new();
        processor.configure(&desired);
        processor.process(&block, &desired, &previews, &audio);
        let rf = preview.try_recv().unwrap();
        let mut iq = RadioIqProcessor::new(radio.iq);
        let expected_rf = iq.process_complex(&samples);
        assert_eq!(rf.samples, expected_rf);
        assert!(rf.samples[1024..]
            .iter()
            .any(|sample| sample.norm() > 0.00005));
        let mut mixer = VfoMixer::new(iq.output_rate());
        mixer.configure(iq.output_rate(), 100_000.0);
        let channel = mixer.process(&expected_rf);
        let mut demod = Demodulator::new();
        demod.set_sample_rates_exact(demod_config.input_rate, demod_config.audio_rate);
        demod.set_lpf_cutoff(demod_config.lpf_cutoff);
        demod.set_rf_bandwidth(demod_config.rf_bandwidth);
        let expected: Vec<_> = demod
            .demodulate_complex(&channel, demod_config.mode)
            .into_iter()
            .map(|sample| sample * radio.volume)
            .collect();
        assert_eq!(received_audio.try_recv().unwrap().samples, expected);
        assert!(expected.iter().any(|sample| sample.abs() > 0.00001));
    }

    #[test]
    fn native_radio_resets_every_state_at_capture_boundaries() {
        for boundary in 0..7 {
            let stream = CaptureStream::default();
            stream.start(1);
            let (radio, demod) = config(None);
            let mut desired = WorkerControl {
                radio: Some(radio),
                config: demod,
                ..Default::default()
            };
            let previews = Broadcaster::new();
            let audio = RadioAudioStream {
                bus: Broadcaster::new(),
                capture: stream.clone(),
                handled_samples: Arc::new(AtomicU64::new(0)),
                lost_samples: Arc::new(AtomicU64::new(0)),
                running: Arc::new(AtomicBool::new(true)),
                produced_samples: Arc::new(AtomicU64::new(0)),
            };
            let rx = audio.bus.subscribe(16, OverflowPolicy::DropIncoming);
            let mut processor = Processor::new();
            processor.configure(&desired);
            let mut block = CaptureBlock::fixture(
                input(16_384, 0),
                ez_proto::CaptureClock::new(10, 0, 2_400_000, None),
                1,
                0,
            );
            block.session = Some(stream.session());
            block.center_hz = 100_000_000;
            processor.process(&block, &desired, &previews, &audio);
            rx.try_recv().unwrap();
            block.clock.source_sample_index = 16_384;
            block.stream_sample_index = 16_384;
            match boundary {
                0 => block.clock.epoch += 1,
                1 => block.clock.discontinuous = true,
                2 => {
                    block.clock.source_sample_index += 8;
                    block.stream_sample_index += 8;
                }
                3 => block.clock.origin_unix_ns = Some(123),
                4 => {
                    block.source_generation += 1;
                    desired.radio.as_mut().unwrap().generation = block.source_generation;
                    processor.configure(&desired);
                }
                5 => {
                    processor.expected.as_mut().unwrap().1 += 1;
                }
                _ => {
                    processor.expected.as_mut().unwrap().2.source_sample_rate_hz += 1;
                }
            }
            let mut fresh = Processor::new();
            fresh.configure(&desired);
            let reference_audio = RadioAudioStream {
                bus: Broadcaster::new(),
                ..audio.clone()
            };
            let reference = reference_audio
                .bus
                .subscribe(16, OverflowPolicy::DropIncoming);
            for _ in 0..3 {
                processor.process(&block, &desired, &previews, &audio);
                fresh.process(&block, &desired, &previews, &reference_audio);
                let actual = rx.try_recv().unwrap();
                let expected = reference.try_recv().unwrap();
                assert_eq!(actual.samples, expected.samples, "boundary {boundary}");
                block.clock = block.clock.offset_samples(16_384).unwrap();
                block.clock.discontinuous = false;
                block.stream_sample_index += 16_384;
            }
        }
    }

    #[test]
    fn native_power_squelch_uses_selected_rf_without_display_updates() {
        let mut meter = PowerMeter::new();
        let rate = 2_400_000.0;
        let tone = |frequency: f64, amplitude: f32| {
            (0..240_000)
                .map(|n| {
                    let phase = std::f64::consts::TAU * frequency * n as f64 / rate;
                    Complex32::new(
                        phase.cos() as f32 * amplitude,
                        phase.sin() as f32 * amplitude,
                    )
                })
                .collect::<Vec<_>>()
        };
        let off = tone(rate / 4096.0 * 341.0, 0.5);
        assert!(
            meter.process(
                &off,
                rate,
                0.0,
                RfPassband::for_mode(DemodMode::Fm, 12500.0, DsbSideband::Both)
            ) < -80.0
        );
        let selected = tone(0.0, 0.1);
        assert!(
            (meter.process(
                &selected,
                rate,
                0.0,
                RfPassband::for_mode(DemodMode::Fm, 12500.0, DsbSideband::Both)
            ) + 20.0)
                .abs()
                < 0.01
        );
        assert_eq!(
            meter.process(
                &vec![Complex32::default(); 240_000],
                rate,
                0.0,
                RfPassband::for_mode(DemodMode::Fm, 12500.0, DsbSideband::Both)
            ),
            -120.0
        );
    }

    #[test]
    fn native_ssb_gate_and_recording_monitor_follow_wanted_sideband_at_vfo_offsets() {
        let mut cases = 0;
        for mode in [DemodMode::Usb, DemodMode::Lsb] {
            for offset in [-25000_i64, 0, 25000] {
                for wanted in [false, true] {
                    let stream = CaptureStream::default();
                    stream.start(1);
                    let (mut radio, mut demod) = config(None);
                    radio.center_hz = 100_000_000;
                    radio.frequency_hz = (100_000_000_i64 + offset) as u64;
                    radio.iq = RadioIqConfig {
                        input_rate: 384000,
                        decimation: 1,
                        dc_remove: false,
                        invert: false,
                    };
                    radio.preview = false;
                    radio.measure_power = true;
                    radio.squelch_mode = SquelchMode::Power;
                    radio.squelch_db = -45.0;
                    radio.volume = 1.0;
                    demod.mode = mode;
                    demod.input_rate = 384000.0;
                    demod.rf_bandwidth = 3000.0;
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
                    let rx = audio.bus.subscribe(32, OverflowPolicy::DropIncoming);
                    let mut processor = Processor::new();
                    processor.configure(&desired);
                    processor.demod.set_agc_enabled(false);
                    let sign = if mode == DemodMode::Usb { 1.0 } else { -1.0 };
                    for n in 0..12 {
                        let samples: Vec<_> = (n * 4096..(n + 1) * 4096)
                            .map(|index| {
                                let time = index as f64 / 384000.0;
                                let opposite = std::f64::consts::TAU
                                    * (offset as f64 - sign * 2197.137)
                                    * time;
                                let selected = std::f64::consts::TAU
                                    * (offset as f64 + sign * 2197.137)
                                    * time;
                                Complex32::new(
                                    (0.7 * opposite.cos()
                                        + if wanted { 0.05 * selected.cos() } else { 0.0 })
                                        as f32,
                                    (0.7 * opposite.sin()
                                        + if wanted { 0.05 * selected.sin() } else { 0.0 })
                                        as f32,
                                )
                            })
                            .collect();
                        let mut block = CaptureBlock::fixture(
                            Vec::new(),
                            ez_proto::CaptureClock::new(123, n as u64 * 4096, 384000, None),
                            1,
                            n as u64 * 4096,
                        );
                        block.float_samples = Some(Arc::from(samples));
                        block.center_hz = 100_000_000;
                        block.clock.discontinuous = n == 0;
                        processor.process(&block, &desired, &previews, &audio);
                    }
                    let mut frames = Vec::new();
                    while let Some(frame) = rx.try_recv() {
                        frames.push(frame);
                    }
                    assert_eq!(frames.len(), 12);
                    let last = frames.last().unwrap();
                    if wanted {
                        assert!((processor.snapshot.signal_db + 26.0206).abs() < 0.15);
                        assert!(last.samples.iter().any(|x| x.abs() > 0.02));
                    } else {
                        assert!(processor.snapshot.signal_db < -80.0);
                        assert!(last.samples.iter().all(|x| *x == 0.0));
                    }
                    let mut recorder = crate::recorder_panel::RecorderPanel::new(
                        crate::test_helpers::make_shared_state(),
                    );
                    recorder.signal_monitor = true;
                    recorder.tick_squelch_record(
                        processor.snapshot.signal_db,
                        -45.0,
                        desired.radio.as_ref().unwrap().frequency_hz,
                        mode.label(),
                    );
                    assert_eq!(
                        recorder.signal_log.len(),
                        usize::from(wanted),
                        "{mode:?} offset={offset}"
                    );
                    cases += 1;
                }
            }
        }
        println!("power_native_consumers cases={cases} display_preview=false gate_and_recording_monitor=passed");
    }

    #[test]
    fn skipped_new_generation_retires_previous_epoch_and_radio_snapshot() {
        let stream = CaptureStream::default();
        stream.start(1);
        let (radio, demod) = config(None);
        let desired = WorkerControl {
            radio: Some(radio),
            config: demod,
            ..Default::default()
        };
        let audio = RadioAudioStream {
            bus: Broadcaster::new(),
            capture: stream.clone(),
            handled_samples: Arc::new(AtomicU64::new(0)),
            lost_samples: Arc::new(AtomicU64::new(0)),
            running: Arc::new(AtomicBool::new(true)),
            produced_samples: Arc::new(AtomicU64::new(0)),
        };
        let mut processor = Processor::new();
        processor.configure(&desired);
        let previews = Broadcaster::new();
        let mut block = CaptureBlock::fixture(
            input(16_384, 0),
            ez_proto::CaptureClock::new(10, 0, 2_400_000, None),
            1,
            0,
        );
        block.center_hz = 100_000_000;
        block.session = Some(stream.session());
        processor.process(&block, &desired, &previews, &audio);
        assert_eq!(processor.snapshot.epoch, Some(10));
        assert!(!processor.snapshot.waveform.is_empty());
        stream.session().finish();
        stream.start(2);
        block.source_generation = 2;
        block.clock.epoch = 11;
        block.session = Some(stream.session());
        // The control plane still refers to generation 1. This block is
        // counted but skipped; no epoch or radio state can survive from 1.
        processor.process(&block, &desired, &previews, &audio);
        assert_eq!(processor.snapshot.generation, 2);
        assert_eq!(processor.snapshot.epoch, None);
        assert!(processor.snapshot.waveform.is_empty());
        assert_eq!(processor.snapshot.signal_db, -120.0);
        assert!(processor.expected.is_none());
        assert_eq!(processor.snapshot.skipped_samples, 16_384);
    }

    #[test]
    fn native_radio_accounts_for_bounded_overflow_missing_tail_and_lost_generations() {
        let stream = CaptureStream::default();
        stream.start(1);
        let worker = DemodWorker::new_capture(stream.clone());
        let (radio, demod) = config(None);
        worker.configure_radio(radio, demod);
        let deadline = Instant::now() + Duration::from_secs(5);
        while worker.applied_version() < worker.requested_version() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        let controls = worker.control.lock().unwrap();
        for n in 0..300 {
            let mut block = CaptureBlock::fixture(
                vec![80, 90],
                ez_proto::CaptureClock::new(10, n, 2_400_000, None),
                1,
                n,
            );
            block.center_hz = 100_000_000;
            block.session = Some(stream.session());
            stream.publish(&block);
        }
        stream.session().finish();
        stream.start(2);
        for n in 0..20 {
            let mut block = CaptureBlock::fixture(
                vec![80, 90],
                ez_proto::CaptureClock::new(11, n, 2_400_000, None),
                2,
                n,
            );
            block.center_hz = 100_000_000;
            block.session = Some(stream.session());
            stream.publish(&block);
        }
        stream.session().finish();
        drop(controls);
        loop {
            let snapshot = worker.radio_snapshot().unwrap();
            if snapshot.complete {
                assert!(snapshot.dropped_blocks >= 191);
                assert_eq!(snapshot.received_samples + snapshot.lost_samples, 320);
                assert_eq!(snapshot.lost_samples, snapshot.dropped_blocks);
                assert_eq!(snapshot.generation, 2);
                assert_eq!(snapshot.epoch, None);
                assert_eq!(
                    worker
                        .audio_stream()
                        .unwrap()
                        .handled_samples
                        .load(Ordering::Acquire),
                    320
                );
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn native_radio_coalesces_controls_and_acknowledges_without_iq() {
        let stream = CaptureStream::default();
        stream.start(1);
        let worker = DemodWorker::new_capture(stream.clone());
        let (sink, playback) = crate::audio_output::AudioSink::fixture(8);
        let (mut radio, demod) = config(Some(sink));
        for n in 0..100 {
            radio.volume = if n == 99 { 0.0 } else { 0.6 };
            radio.frequency_hz = if n == 99 {
                100_100_000
            } else {
                100_000_000 + n * 100
            };
            assert!(worker.configure_radio(radio.clone(), demod));
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while worker.applied_version() < worker.requested_version() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        let mut block = CaptureBlock::fixture(
            input(16_384, 0),
            ez_proto::CaptureClock::new(10, 0, 2_400_000, None),
            1,
            0,
        );
        block.center_hz = 100_000_000;
        block.session = Some(stream.session());
        stream.publish(&block);
        let heard = playback.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(!heard.is_empty());
        assert!(heard.iter().all(|sample| *sample == 0.0));
    }
}
