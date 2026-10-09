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

use lrpt_decode::{validate_mission_sampling, DecodeProgress, LrptDecoder, LrptError};

use ez_proto::{ChannelId, MeteorProfile, TelemetryFrame};

use crate::broadcast::{Broadcaster, BroadcasterHandle, OverflowPolicy};
use crate::bus::{SampleBlock, SampleBusHandle, SampleContinuity};

#[derive(Debug, Clone, Copy)]
pub struct TelemetryConfig {
    pub channel_id: ChannelId,
    pub sample_rate_hz: u32,
    pub profile: MeteorProfile,
    pub center_freq_hz: u64,
}

pub struct TelemetryPipeline {
    input: SampleBusHandle,
    channel_id: ChannelId,
    decoder: LrptDecoder,
    profile: MeteorProfile,
    sample_rate_hz: u32,
    center_freq_hz: u64,
    source_product: Option<u64>,
    product_id: u64,
    continuity: SampleContinuity,
    progress_rx: crossbeam_channel::Receiver<DecodeProgress>,
    output: Broadcaster<TelemetryFrame>,
}

impl TelemetryPipeline {
    pub fn new(input: SampleBusHandle, config: TelemetryConfig) -> Result<Self, LrptError> {
        let (progress_tx, progress_rx) = crossbeam_channel::unbounded();
        Ok(Self {
            input,
            channel_id: config.channel_id,
            decoder: LrptDecoder::for_mission(config.sample_rate_hz, config.profile, progress_tx)?,
            profile: config.profile,
            sample_rate_hz: config.sample_rate_hz,
            center_freq_hz: config.center_freq_hz,
            source_product: None,
            product_id: ez_proto::new_capture_epoch(),
            continuity: SampleContinuity::default(),
            progress_rx,
            output: Broadcaster::new(),
        })
    }

    /// Explicit same-frequency pass/product boundary. Retire queued old IQ.
    pub fn start_new_product(&mut self) -> Result<(), LrptError> {
        for _ in 0..self.input.queued_len() {
            let _ = self.input.try_recv();
        }
        self.retire_product()
    }

    fn retire_product(&mut self) -> Result<(), LrptError> {
        self.decoder
            .start_new_product(self.sample_rate_hz, self.profile)?;
        self.continuity = SampleContinuity::default();
        while self.progress_rx.try_recv().is_ok() {}
        self.product_id = ez_proto::new_capture_epoch();
        self.output.publish(TelemetryFrame {
            product_id: self.product_id,
            channel_id: self.channel_id,
            apid: 0,
            width: 0,
            height: 0,
            pixels: Vec::new(),
            rs_ok: 0,
            rs_failed: 0,
            costas_locked: false,
            frame_locked: false,
        });
        Ok(())
    }

    /// Retuning to another RF channel starts another product. A rate-only
    /// adjustment discards partial transport and preserves the same image.
    pub fn retune(&mut self, sample_rate_hz: u32, center_freq_hz: u64) -> Result<(), LrptError> {
        validate_mission_sampling(sample_rate_hz, self.profile)?;
        self.sample_rate_hz = sample_rate_hz;
        if self.center_freq_hz != center_freq_hz {
            self.center_freq_hz = center_freq_hz;
            self.source_product = None;
            self.start_new_product()?;
        } else {
            self.decoder.reset_mission_stream(sample_rate_hz)?;
            self.continuity = SampleContinuity::default();
            while self.progress_rx.try_recv().is_ok() {}
        }
        Ok(())
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
        // A queued tail from before a channel retune cannot populate the new product.
        if block.center_freq_hz != self.center_freq_hz {
            return;
        }
        if validate_mission_sampling(block.sample_rate_hz, self.profile).is_err() {
            self.decoder
                .reset_mission_stream(self.sample_rate_hz)
                .expect("validated configuration");
            self.continuity = SampleContinuity::default();
            while self.progress_rx.try_recv().is_ok() {}
            return;
        }
        self.sample_rate_hz = block.sample_rate_hz;
        if self
            .source_product
            .is_some_and(|session| session != block.product_session)
        {
            // Source-session identity survives channelizer/gap epoch changes.
            self.retire_product().expect("validated configuration");
        }
        self.source_product = Some(block.product_session);
        if !self.continuity.observe(block) {
            self.decoder
                .reset_mission_stream(block.sample_rate_hz)
                .expect("validated configuration");
            while self.progress_rx.try_recv().is_ok() {}
        }
        self.decoder.push_complex(&block.samples);
        while let Ok(progress) = self.progress_rx.try_recv() {
            self.publish_progress(&progress);
        }
    }

    fn publish_progress(&mut self, progress: &DecodeProgress) {
        for (apid, image) in &progress.preview {
            self.output.publish(TelemetryFrame {
                product_id: self.product_id,
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
            profile: MeteorProfile::Oqpsk72,
            center_freq_hz: 137_100_000,
        };
        (bus, TelemetryPipeline::new(handle, config).unwrap())
    }

    fn silence_block(len: usize) -> SampleBlock {
        SampleBlock {
            product_session: 0,
            observed_at: None,
            capture: None,
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

    fn seeded_product() -> (SampleBus, TelemetryPipeline) {
        let (bus, mut pipeline) = test_pipeline();
        let bytes = include_bytes!("../../../audit/2026-10-07/fixtures/meteor-product-72k.cf32");
        let samples = lrpt_decode::cf32_le_bytes_to_complex(bytes);
        let mut at = 0;
        for chunk in samples.chunks(4096) {
            let mut block = silence_block(chunk.len());
            block.product_session = 42;
            block.start_sample = at;
            block.capture = Some(ez_proto::CaptureClock::new(7, at, 288_000, None));
            block.samples = Arc::from(chunk);
            pipeline.process_block(&block);
            at += chunk.len() as u64;
        }
        let decoded = pipeline.decoder.progress_snapshot();
        assert_eq!(decoded.rs_ok, 12);
        assert_eq!(decoded.segments_decoded, 3);
        assert_eq!(decoded.preview.len(), 1);
        (bus, pipeline)
    }

    #[test]
    fn received_images_survive_gap_epochs_but_retire_on_real_product_boundaries() {
        for boundary in 0..4 {
            let (bus, mut pipeline) = seeded_product();
            let output = pipeline.subscribe(16);
            let before = pipeline.decoder.progress_snapshot();
            let product = pipeline.product_id;
            let mut block = silence_block(32);
            block.product_session = 42;
            block.start_sample = 800_000;
            block.capture = Some(ez_proto::CaptureClock::new(999, 800_000, 288_000, None));
            block.capture.as_mut().unwrap().discontinuous = true;
            match boundary {
                0 => pipeline.process_block(&block),
                1 => {
                    block.product_session = 43;
                    bus.publish(block.clone());
                    pipeline.process_block(&block);
                    assert_eq!(
                        pipeline.input.queued_len(),
                        1,
                        "new-source queued IQ was discarded"
                    );
                }
                2 => {
                    pipeline.retune(288_000, 137_900_000).unwrap();
                    // The old channel tail must never populate the new product.
                    pipeline.process_block(&block);
                }
                _ => {
                    bus.publish(block);
                    pipeline.start_new_product().unwrap();
                    assert_eq!(pipeline.input.queued_len(), 0);
                }
            }
            let after = pipeline.decoder.progress_snapshot();
            if boundary == 0 {
                assert_eq!(pipeline.product_id, product);
                assert_eq!(after.rs_ok, before.rs_ok);
                assert_eq!(after.segments_decoded, before.segments_decoded);
                assert_eq!(after.preview, before.preview);
            } else {
                assert_ne!(pipeline.product_id, product);
                assert_eq!(after.rs_ok, 0);
                assert_eq!(after.segments_decoded, 0);
                assert!(after.preview.is_empty());
                let reset = output.try_recv().unwrap();
                assert_eq!(reset.product_id, pipeline.product_id);
                assert_eq!((reset.apid, reset.width, reset.height), (0, 0, 0));
                assert!(reset.pixels.is_empty());
            }
        }
    }

    #[test]
    fn invalid_dynamic_sampling_cannot_consume_or_complete_a_frame() {
        let (_, mut pipeline) = seeded_product();
        let before = pipeline.decoder.progress_snapshot();
        let mut block = silence_block(4096);
        block.sample_rate_hz = 100_000;
        pipeline.process_block(&block);
        let after = pipeline.decoder.progress_snapshot();
        assert_eq!(before.rs_ok, after.rs_ok);
        assert_eq!(before.preview, after.preview);
        assert!(!after.frame_locked);
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
