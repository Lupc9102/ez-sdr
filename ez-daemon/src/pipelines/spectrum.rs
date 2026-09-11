//! Headless FFT/waterfall producer: turns a stream of [`SampleBlock`]s (wideband or a tuned
//! virtual channel) into [`SpectrumFrame`]s, fanned out to any number of network clients.
//! Pure DSP — no rendering, no `egui`, nothing GUI-specific lives here or anywhere in
//! `ez-daemon`; a client turns a `SpectrumFrame`'s dB bins into pixels on its own.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

use ez_proto::SpectrumFrame;

use crate::broadcast::{Broadcaster, BroadcasterHandle, OverflowPolicy};
use crate::bus::{SampleBlock, SampleBusHandle};

#[derive(Debug, Clone, Copy)]
pub struct SpectrumConfig {
    pub fft_size: usize,
    /// Exponential smoothing factor in `[0, 1]` applied per bin across frames: `1.0` shows
    /// each frame raw with no smoothing, smaller values average more heavily against the
    /// previous frame. Matches the GUI's original `avg_alpha` display knob, just computed
    /// daemon-side now instead of client-side.
    pub avg_alpha: f32,
}

impl Default for SpectrumConfig {
    fn default() -> Self {
        Self {
            fft_size: 2048,
            avg_alpha: 0.3,
        }
    }
}

const MAX_SPECTRUM_QUEUE_FRAMES: usize = 4;

fn hann_window(len: usize) -> Vec<f32> {
    let size = len.max(1) as f32;
    (0..len)
        .map(|n| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * n as f32 / size).cos()))
        .collect()
}

/// Turns incoming sample blocks into dB-magnitude [`SpectrumFrame`]s at a fixed FFT size,
/// publishing to its own [`Broadcaster`] that any number of clients can subscribe to.
///
/// Input blocks are accumulated in a ring buffer rather than assumed to already be exactly
/// `fft_size` long: a wideband ingestion read and a decimated virtual-channel block are both
/// whatever length the upstream producer happened to hand over, never aligned to the FFT
/// window on purpose. One [`Self::tick`] call may therefore produce zero frames (not enough
/// samples accumulated yet), one, or several (a block much larger than `fft_size` arrived at
/// once) — downstream consumers are expected to be realtime visualizers that only care about
/// the newest frame, so [`Self::subscribe`] uses [`OverflowPolicy::DropOldest`].
pub struct SpectrumPipeline {
    input: SampleBusHandle,
    fft: std::sync::Arc<dyn Fft<f32>>,
    fft_size: usize,
    window: Vec<f32>,
    avg_alpha: f32,
    accum: VecDeque<Complex32>,
    scratch: Vec<Complex32>,
    smoothed_db: Vec<f32>,
    last_center_freq_hz: Option<u64>,
    output: Broadcaster<SpectrumFrame>,
}

impl SpectrumPipeline {
    #[must_use]
    pub fn new(input: SampleBusHandle, config: SpectrumConfig) -> Self {
        let fft_size = config.fft_size.max(2);
        let mut planner = FftPlanner::<f32>::new();
        Self {
            input,
            fft: planner.plan_fft_forward(fft_size),
            fft_size,
            window: hann_window(fft_size),
            avg_alpha: config.avg_alpha.clamp(0.0, 1.0),
            accum: VecDeque::with_capacity(fft_size * 2),
            scratch: vec![Complex32::new(0.0, 0.0); fft_size],
            smoothed_db: vec![-120.0; fft_size],
            last_center_freq_hz: None,
            output: Broadcaster::new(),
        }
    }

    /// Subscribes a new client to this pipeline's output frames.
    #[must_use]
    pub fn subscribe(&self, capacity: usize) -> BroadcasterHandle<SpectrumFrame> {
        self.output.subscribe(capacity, OverflowPolicy::DropOldest)
    }

    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.output.subscriber_count()
    }

    /// Waits up to `poll_timeout` for the next input block and, if one arrived, folds it into
    /// the accumulator and publishes every full FFT window's worth of frame that results.
    /// Returns whether a block was actually consumed — callers driving a shutdown-aware loop
    /// should check an external stop signal on `false` rather than treating it as "source
    /// gone" (see [`SampleBusHandle::recv_timeout`]'s doc comment for why).
    pub fn tick(&mut self, poll_timeout: Duration) -> bool {
        let Some(block) = self.input.recv_timeout(poll_timeout) else {
            return false;
        };
        self.process_block(&block);
        true
    }

    /// Drives `tick` in a loop until `running` is set to `false`.
    pub fn run(&mut self, running: &AtomicBool) {
        while running.load(Ordering::Relaxed) {
            self.tick(Duration::from_millis(250));
        }
    }

    fn process_block(&mut self, block: &SampleBlock) {
        // Issue 19: When the SDR retunes, clear the accumulator and reset smoothed dB bins
        // so pre-tune frequency energy doesn't smear into post-tune frames.
        if self.last_center_freq_hz != Some(block.center_freq_hz) {
            self.accum.clear();
            self.smoothed_db.fill(-120.0);
            self.last_center_freq_hz = Some(block.center_freq_hz);
        }

        self.accum.extend(block.samples.iter().copied());

        // Issue 18: If no clients are subscribed, avoid burning CPU on Fourier transforms.
        if self.subscriber_count() == 0 {
            if self.accum.len() > self.fft_size {
                let excess = self.accum.len() - self.fft_size;
                self.accum.drain(..excess);
            }
            return;
        }

        // Issue 18: If the backlog exceeds MAX_SPECTRUM_QUEUE_FRAMES, drop the older excess
        // so we don't execute dozens of redundant FFTs that will just be dropped by DropOldest.
        if self.accum.len() > MAX_SPECTRUM_QUEUE_FRAMES * self.fft_size {
            let excess = self.accum.len() - (MAX_SPECTRUM_QUEUE_FRAMES * self.fft_size);
            self.accum.drain(..excess);
        }

        while self.accum.len() >= self.fft_size {
            for (i, s) in self.accum.iter().take(self.fft_size).enumerate() {
                self.scratch[i] = *s * self.window[i];
            }
            self.fft.process(&mut self.scratch);
            self.accum.drain(..self.fft_size);

            let scale = 1.0 / self.fft_size as f32;
            let n = self.fft_size;
            for (i, c) in self.scratch.iter().enumerate() {
                // fftshift to ascending-frequency order: bin 0 = center - fs/2, bin n/2 =
                // center. Matches SpectrumFrame::bins' documented convention.
                let dst = (i + n / 2) % n;
                let mag = c.norm() * scale;
                let db = if mag > 1e-10 {
                    20.0 * mag.log10()
                } else {
                    -120.0
                };
                let prev = self.smoothed_db[dst];
                self.smoothed_db[dst] = self.avg_alpha * db + (1.0 - self.avg_alpha) * prev;
            }

            let timestamp_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);

            self.output.publish(SpectrumFrame {
                center_hz: block.center_freq_hz,
                sample_rate_hz: block.sample_rate_hz,
                bins: self.smoothed_db.clone(),
                timestamp_ms,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SampleBus;
    use std::sync::Arc;

    fn tone_block(n: usize, freq_hz: f64, sample_rate_hz: u32, center_hz: u64) -> SampleBlock {
        let samples: Vec<Complex32> = (0..n)
            .map(|k| {
                let phase =
                    2.0 * std::f64::consts::PI * freq_hz * (k as f64) / sample_rate_hz as f64;
                Complex32::from_polar(1.0, phase as f32)
            })
            .collect();
        SampleBlock {
            start_sample: 0,
            sample_rate_hz,
            center_freq_hz: center_hz,
            samples: Arc::from(samples),
        }
    }

    fn silence_block(n: usize, sample_rate_hz: u32, center_hz: u64) -> SampleBlock {
        SampleBlock {
            start_sample: 0,
            sample_rate_hz,
            center_freq_hz: center_hz,
            samples: Arc::from(vec![Complex32::new(0.0, 0.0); n]),
        }
    }

    fn pipeline_with(config: SpectrumConfig) -> (SampleBus, SpectrumPipeline) {
        let bus = SampleBus::new();
        let handle = bus.subscribe(8, crate::bus::OverflowPolicy::DropIncoming);
        (bus, SpectrumPipeline::new(handle, config))
    }

    #[test]
    fn hann_window_starts_and_ends_near_zero_peaks_at_center() {
        let w = hann_window(1024);
        assert!(w[0] < 0.01);
        assert!(w[1023] < 0.01);
        assert!(w[512] > 0.99);
    }

    #[test]
    fn tick_times_out_cleanly_when_idle() {
        let (_bus, mut sp) = pipeline_with(SpectrumConfig::default());
        assert!(!sp.tick(Duration::from_millis(20)));
    }

    #[test]
    fn run_exits_promptly_when_running_flag_clears() {
        let (_bus, mut sp) = pipeline_with(SpectrumConfig::default());
        let running = AtomicBool::new(false);
        sp.run(&running); // must return immediately, not hang
    }

    #[test]
    fn accumulates_partial_blocks_before_producing_a_frame() {
        let (bus, mut sp) = pipeline_with(SpectrumConfig {
            fft_size: 256,
            avg_alpha: 1.0,
        });
        let out = sp.subscribe(4);

        // Feed fewer samples than fft_size across several ticks: no frame yet.
        for _ in 0..3 {
            bus.publish(silence_block(64, 1_000_000, 100_000_000));
            sp.tick(Duration::from_millis(50));
            assert!(out.try_recv().is_none());
        }

        // The 4th block pushes total accumulated to 256 == fft_size: exactly one frame now.
        bus.publish(silence_block(64, 1_000_000, 100_000_000));
        sp.tick(Duration::from_millis(50));
        let frame = out
            .try_recv()
            .expect("expected a frame once fft_size samples accumulated");
        assert_eq!(frame.bins.len(), 256);
    }

    #[test]
    fn drains_multiple_frames_from_one_oversized_block() {
        let (bus, mut sp) = pipeline_with(SpectrumConfig {
            fft_size: 128,
            avg_alpha: 1.0,
        });
        let out = sp.subscribe(8);

        bus.publish(silence_block(128 * 3, 1_000_000, 100_000_000));
        sp.tick(Duration::from_millis(50));

        let mut frames = 0;
        while out.try_recv().is_some() {
            frames += 1;
        }
        assert_eq!(
            frames, 3,
            "one 3x-oversized block should yield exactly 3 frames"
        );
    }

    #[test]
    fn frame_metadata_matches_the_input_block() {
        let (bus, mut sp) = pipeline_with(SpectrumConfig {
            fft_size: 128,
            avg_alpha: 1.0,
        });
        let out = sp.subscribe(4);

        bus.publish(silence_block(128, 2_400_000, 433_920_000));
        sp.tick(Duration::from_millis(50));

        let frame = out.try_recv().expect("expected a frame");
        assert_eq!(frame.center_hz, 433_920_000);
        assert_eq!(frame.sample_rate_hz, 2_400_000);
        assert!(frame.timestamp_ms > 0);
    }

    #[test]
    fn pure_tone_peak_lands_at_the_expected_bin() {
        let fft_size = 1024;
        let sample_rate = 1_000_000u32;
        let (bus, mut sp) = pipeline_with(SpectrumConfig {
            fft_size,
            avg_alpha: 1.0,
        });
        let out = sp.subscribe(4);

        // +100 kHz tone: bin_hz = 1e6/1024 ~= 976.6 Hz, expected offset bins ~= 102.4,
        // landing right of center (center = fft_size/2 after the fftshift).
        bus.publish(tone_block(fft_size, 100_000.0, sample_rate, 100_000_000));
        sp.tick(Duration::from_millis(50));
        let frame = out.try_recv().expect("expected a frame");

        let (peak_bin, _) = frame
            .bins
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .expect("non-empty bins");

        let bin_hz = sample_rate as f64 / fft_size as f64;
        let expected_bin = fft_size / 2 + (100_000.0 / bin_hz).round() as usize;
        assert!(
            peak_bin.abs_diff(expected_bin) <= 1,
            "peak at bin {peak_bin}, expected near {expected_bin}"
        );
    }

    #[test]
    fn smoothing_moves_toward_new_value_without_snapping_instantly() {
        let fft_size = 256;
        let sample_rate = 1_000_000u32;
        let (bus, mut sp) = pipeline_with(SpectrumConfig {
            fft_size,
            avg_alpha: 0.2,
        });
        let out = sp.subscribe(4);

        bus.publish(silence_block(fft_size, sample_rate, 100_000_000));
        sp.tick(Duration::from_millis(50));
        let first = out.try_recv().expect("first frame");

        bus.publish(tone_block(fft_size, 0.0, sample_rate, 100_000_000));
        sp.tick(Duration::from_millis(50));
        let second = out.try_recv().expect("second frame");

        let center = fft_size / 2;
        // DC tone at full amplitude computes to ~0 dB raw; starting from -120 dB silence
        // with alpha=0.2 the smoothed value should move partway, landing well above the
        // silence floor but well below an unsmoothed instantaneous ~0 dB.
        assert!(second.bins[center] > first.bins[center] + 10.0);
        assert!(second.bins[center] < -10.0);
    }

    #[test]
    fn subscriber_count_reflects_subscriptions() {
        let (_bus, sp) = pipeline_with(SpectrumConfig::default());
        assert_eq!(sp.subscriber_count(), 0);
        let a = sp.subscribe(4);
        assert_eq!(sp.subscriber_count(), 1);
        let b = sp.subscribe(4);
        assert_eq!(sp.subscriber_count(), 2);
        drop(a);
        assert_eq!(sp.subscriber_count(), 1);
        drop(b);
        assert_eq!(sp.subscriber_count(), 0);
    }

    #[test]
    fn retune_flushes_accumulator_and_smoothed_history() {
        // Issue 19: Retuning must clear accumulator and reset smoothed bins.
        let fft_size = 256;
        let sample_rate = 1_000_000u32;
        let (bus, mut sp) = pipeline_with(SpectrumConfig {
            fft_size,
            avg_alpha: 0.5,
        });
        let out = sp.subscribe(4);

        // Put a strong DC tone at 100 MHz
        bus.publish(tone_block(fft_size, 0.0, sample_rate, 100_000_000));
        sp.tick(Duration::from_millis(50));
        let frame1 = out.try_recv().expect("frame 1");
        let center = fft_size / 2;
        assert!(frame1.bins[center] > -100.0);

        // Retune to 200 MHz with silence
        bus.publish(silence_block(fft_size, sample_rate, 200_000_000));
        sp.tick(Duration::from_millis(50));
        let frame2 = out.try_recv().expect("frame 2");
        assert_eq!(frame2.center_hz, 200_000_000);
        // After retune, history was flushed, so silence at 200 MHz shouldn't retain peak
        assert!(
            frame2.bins[center] < -110.0,
            "bins should reset on retune, got {}",
            frame2.bins[center]
        );
    }

    #[test]
    fn massive_backlog_is_capped_to_avoid_redundant_ffts() {
        // Issue 18: An oversized block with 20 frames must be capped to MAX_SPECTRUM_QUEUE_FRAMES (4).
        let fft_size = 128;
        let (bus, mut sp) = pipeline_with(SpectrumConfig {
            fft_size,
            avg_alpha: 1.0,
        });
        let out = sp.subscribe(32);

        bus.publish(silence_block(fft_size * 20, 1_000_000, 100_000_000));
        sp.tick(Duration::from_millis(50));

        let mut frames = 0;
        while out.try_recv().is_some() {
            frames += 1;
        }
        assert_eq!(
            frames, MAX_SPECTRUM_QUEUE_FRAMES,
            "backlog should be capped to 4 frames"
        );
    }
}
