//! Headless LRPT satellite telemetry decode: turns a tuned virtual channel's complex
//! baseband into progressively-decoded [`TelemetryFrame`]s via `lrpt-decode`'s streaming
//! [`LrptDecoder`], fanned out to any number of network clients.
//!
//! Unlike [`crate::pipelines::spectrum::SpectrumPipeline`]/[`crate::pipelines::audio::AudioPipeline`]
//! (lightweight per-block DSP) or [`crate::pipelines::packet::PacketPipeline`] (event-driven
//! Mode-S parsing), this pipeline fronts the heaviest, most multi-stage processing of the
//! four: QPSK demod, differential decode, CADU frame sync, Reed-Solomon correction, CCSDS
//! packet reassembly, and scanline image building all happen internally inside
//! [`LrptDecoder`]. This pipeline's only job is feeding it complex samples and turning its
//! periodic [`DecodeProgress`] callbacks into published [`TelemetryFrame`]s.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use lrpt_decode::{DecodeProgress, LrptDecoder};

use ez_proto::{ChannelId, TelemetryFrame};

use crate::broadcast::{Broadcaster, BroadcasterHandle, OverflowPolicy};
use crate::bus::{SampleBlock, SampleBusHandle};

#[derive(Debug, Clone, Copy)]
pub struct TelemetryConfig {
    pub channel_id: ChannelId,
    pub sample_rate_hz: u32,
    pub symbol_rate_hz: u32,
}

pub struct TelemetryPipeline {
    input: SampleBusHandle,
    channel_id: ChannelId,
    decoder: LrptDecoder,
    progress_rx: crossbeam_channel::Receiver<DecodeProgress>,
    output: Broadcaster<TelemetryFrame>,
}

impl TelemetryPipeline {
    #[must_use]
    pub fn new(input: SampleBusHandle, config: TelemetryConfig) -> Self {
        let (progress_tx, progress_rx) = crossbeam_channel::unbounded();
        Self {
            input,
            channel_id: config.channel_id,
            decoder: LrptDecoder::new(config.sample_rate_hz, config.symbol_rate_hz, progress_tx),
            progress_rx,
            output: Broadcaster::new(),
        }
    }

    #[must_use]
    pub fn subscribe(&self, capacity: usize) -> BroadcasterHandle<TelemetryFrame> {
        self.output.subscribe(capacity, OverflowPolicy::DropOldest)
    }

    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.output.subscriber_count()
    }

    /// Polls the input bus once, processing at most one [`SampleBlock`]. Returns `true` if a
    /// block was received and processed, `false` on timeout.
    pub fn tick(&mut self, poll_timeout: Duration) -> bool {
        let Some(block) = self.input.recv_timeout(poll_timeout) else {
            return false;
        };
        self.process_block(&block);
        true
    }

    /// Drives `tick` in a loop until `running` clears, e.g. on its own dedicated thread.
    pub fn run(&mut self, running: &AtomicBool) {
        while running.load(Ordering::Relaxed) {
            self.tick(Duration::from_millis(100));
        }
    }

    fn process_block(&mut self, block: &SampleBlock) {
        self.decoder.push_complex(&block.samples);
        while let Ok(progress) = self.progress_rx.try_recv() {
            self.publish_progress(&progress);
        }
    }

    fn publish_progress(&mut self, progress: &DecodeProgress) {
        for (apid, image) in &progress.preview {
            self.output.publish(TelemetryFrame {
                channel_id: self.channel_id,
                apid: *apid,
                width: image.width(),
                height: image.height(),
                pixels: image.as_raw().clone(),
                rs_ok: progress.rs_ok,
                rs_failed: progress.rs_failed,
                costas_locked: progress.costas_locked,
                frame_locked: progress.frame_locked,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SampleBus;
    use image::GrayImage;
    use num_complex::Complex32;
    use std::sync::Arc;

    fn test_pipeline() -> (SampleBus, TelemetryPipeline) {
        let bus = SampleBus::new();
        let handle = bus.subscribe(8, OverflowPolicy::DropIncoming);
        let config = TelemetryConfig {
            channel_id: 7,
            sample_rate_hz: 288_000,
            symbol_rate_hz: 80_000,
        };
        (bus, TelemetryPipeline::new(handle, config))
    }

    fn silence_block(len: usize) -> SampleBlock {
        SampleBlock {
            start_sample: 0,
            sample_rate_hz: 288_000,
            center_freq_hz: 137_100_000,
            samples: Arc::from(vec![Complex32::new(0.0, 0.0); len]),
        }
    }

    fn test_image(width: u32, height: u32, fill: u8) -> GrayImage {
        GrayImage::from_raw(width, height, vec![fill; (width * height) as usize])
            .expect("width*height matches buffer length")
    }

    #[test]
    fn tick_times_out_cleanly_when_idle() {
        let (_bus, mut pipeline) = test_pipeline();
        assert!(!pipeline.tick(Duration::from_millis(10)));
    }

    #[test]
    fn run_exits_promptly_when_running_flag_clears() {
        let (_bus, mut pipeline) = test_pipeline();
        let running = AtomicBool::new(false);
        pipeline.run(&running);
    }

    #[test]
    fn subscriber_count_reflects_subscriptions() {
        let (_bus, pipeline) = test_pipeline();
        assert_eq!(pipeline.subscriber_count(), 0);
        let handle = pipeline.subscribe(4);
        assert_eq!(pipeline.subscriber_count(), 1);
        drop(handle);
    }

    #[test]
    fn process_block_on_silence_produces_no_telemetry_frames() {
        let (bus, mut pipeline) = test_pipeline();
        let output = pipeline.subscribe(8);
        bus.publish(silence_block(4096));
        assert!(pipeline.tick(Duration::from_millis(50)));
        assert!(output.try_recv().is_none());
    }

    #[test]
    fn publish_progress_maps_decode_progress_fields_onto_telemetry_frames() {
        let (_bus, mut pipeline) = test_pipeline();
        let output = pipeline.subscribe(8);
        let progress = DecodeProgress {
            rs_ok: 42,
            rs_failed: 3,
            costas_locked: true,
            frame_locked: true,
            preview: vec![(64, test_image(3, 2, 200))],
            ..DecodeProgress::default()
        };

        pipeline.publish_progress(&progress);

        let frame = output.try_recv().expect("frame should have been published");
        assert_eq!(frame.channel_id, 7);
        assert_eq!(frame.apid, 64);
        assert_eq!(frame.width, 3);
        assert_eq!(frame.height, 2);
        assert_eq!(frame.pixels, vec![200; 6]);
        assert_eq!(frame.rs_ok, 42);
        assert_eq!(frame.rs_failed, 3);
        assert!(frame.costas_locked);
        assert!(frame.frame_locked);
        assert!(output.try_recv().is_none());
    }

    #[test]
    fn publish_progress_emits_one_frame_per_preview_entry() {
        let (_bus, mut pipeline) = test_pipeline();
        let output = pipeline.subscribe(8);
        let progress = DecodeProgress {
            preview: vec![(64, test_image(1, 1, 10)), (65, test_image(1, 1, 20))],
            ..DecodeProgress::default()
        };

        pipeline.publish_progress(&progress);

        let first = output.try_recv().expect("first frame");
        let second = output.try_recv().expect("second frame");
        assert_eq!(first.apid, 64);
        assert_eq!(second.apid, 65);
        assert!(output.try_recv().is_none());
    }

    #[test]
    fn publish_progress_with_empty_preview_publishes_nothing() {
        let (_bus, mut pipeline) = test_pipeline();
        let output = pipeline.subscribe(8);
        pipeline.publish_progress(&DecodeProgress::default());
        assert!(output.try_recv().is_none());
    }
}
