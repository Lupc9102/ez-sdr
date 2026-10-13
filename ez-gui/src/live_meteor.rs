//! Independent, bounded live Meteor capture consumer.
//!
//! This worker has its own CaptureStream subscription. It never reads the GUI's
//! display queue, and its progress channel is deliberately latest-value bounded.

use crate::capture_stream::CaptureStream;
use crate::source_manager::CaptureBlock;
use crossbeam_channel::{Receiver, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const SUBSCRIPTION_CAPACITY: usize = 32;
type LiveWorker = (
    Arc<AtomicBool>,
    Receiver<LiveMeteorEvent>,
    std::thread::JoinHandle<()>,
);

#[derive(Debug, Clone)]
pub(crate) enum LiveMeteorEvent {
    Progress(lrpt_decode::DecodeProgress),
    Status(String),
    Finished(Option<lrpt_decode::DecodeResult>),
    Error(String),
}

/// Starts a worker that owns one independent subscriber and no more than one
/// progress event in transit to the GUI.
pub(crate) fn start(
    stream: CaptureStream,
    sample_rate_hz: u32,
    profile: ez_proto::MeteorProfile,
    satellite_name: Option<String>,
    cds_epoch: lrpt_decode::CdsEpoch,
) -> Result<LiveWorker, String> {
    lrpt_decode::validate_mission_sampling(sample_rate_hz, profile)
        .map_err(|error| error.to_string())?;
    let subscription = stream.subscribe(SUBSCRIPTION_CAPACITY);
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let (event_tx, event_rx) = crossbeam_channel::bounded(1);
    let worker_event_rx = event_rx.clone();
    let handle = std::thread::Builder::new()
        .name("meteor-live-decoder".into())
        .spawn(move || {
            run(
                subscription,
                sample_rate_hz,
                profile,
                satellite_name,
                cds_epoch,
                worker_stop,
                event_tx,
                worker_event_rx,
            )
        })
        .map_err(|error| format!("Could not start live Meteor decoder: {error}"))?;
    Ok((stop, event_rx, handle))
}

fn send_latest(
    tx: &Sender<LiveMeteorEvent>,
    rx: &Receiver<LiveMeteorEvent>,
    event: LiveMeteorEvent,
) {
    // This one-slot queue caps memory if the UI stops repainting. Replace a
    // stale snapshot so the eventual final image cannot be lost behind it.
    if let Err(crossbeam_channel::TrySendError::Full(event)) = tx.try_send(event) {
        while rx.try_recv().is_ok() {}
        let _ = tx.try_send(event);
    }
}

#[allow(clippy::too_many_arguments)]
fn run(
    subscription: ez_daemon::broadcast::BroadcasterHandle<Arc<CaptureBlock>>,
    initial_rate: u32,
    profile: ez_proto::MeteorProfile,
    satellite_name: Option<String>,
    cds_epoch: lrpt_decode::CdsEpoch,
    stop: Arc<AtomicBool>,
    events: Sender<LiveMeteorEvent>,
    event_rx: Receiver<LiveMeteorEvent>,
) {
    // Surface startup immediately. Constructing the decoder can allocate and
    // initialize DSP state, which should not leave the panel showing its
    // generic "Starting" message while the worker is already listening.
    send_latest(
        &events,
        &event_rx,
        LiveMeteorEvent::Status(format!(
            "Live LRPT listening{}; waiting for capture samples.",
            satellite_name
                .as_deref()
                .map(|name| format!(" for {name}"))
                .unwrap_or_default()
        )),
    );

    let (decoder_tx, decoder_rx) = crossbeam_channel::bounded(1);
    let mut decoder =
        match lrpt_decode::LrptDecoder::for_mission(initial_rate, profile, decoder_tx.clone()) {
            Ok(decoder) => decoder,
            Err(error) => {
                send_latest(
                    &events,
                    &event_rx,
                    LiveMeteorEvent::Error(error.to_string()),
                );
                return;
            }
        };
    decoder.set_cds_epoch(cds_epoch);

    let mut last_epoch = None;
    let mut last_generation = None;
    let mut last_center = None;
    let mut last_rate = initial_rate;
    let mut expected_source_index = None;
    let mut observed_drops = 0;

    while !stop.load(Ordering::Acquire) {
        let Some(block) = subscription.recv_timeout(Duration::from_millis(100)) else {
            if let Some(progress) = decoder_rx.try_iter().last() {
                send_latest(&events, &event_rx, LiveMeteorEvent::Progress(progress));
            }
            continue;
        };

        let received_drops = subscription.dropped_count();
        let capture_rate = block.clock.exact_sample_rate_hz().round() as u32;
        let capture_rate = if capture_rate == 0 {
            initial_rate
        } else {
            capture_rate
        };
        let new_product = last_epoch.is_some_and(|epoch| epoch != block.clock.epoch)
            || last_generation.is_some_and(|generation| generation != block.source_generation)
            || last_center.is_some_and(|center| center != block.center_hz)
            || last_rate != capture_rate;
        let dropped = received_drops != observed_drops;
        let expected_index_gap = expected_source_index
            .is_some_and(|expected| expected != block.clock.source_sample_index);

        if new_product {
            if let Err(error) = decoder.start_new_product(capture_rate, profile) {
                send_latest(
                    &events,
                    &event_rx,
                    LiveMeteorEvent::Error(error.to_string()),
                );
                break;
            }
            decoder.set_cds_epoch(cds_epoch);
            send_latest(
                &events,
                &event_rx,
                LiveMeteorEvent::Status(format!(
                    "New live product: {} at {:.3} MHz.",
                    satellite_name.as_deref().unwrap_or("Meteor LRPT"),
                    block.center_hz as f64 / 1_000_000.0
                )),
            );
        } else if dropped || expected_index_gap || block.clock.discontinuous {
            if let Err(error) = decoder.reset_mission_stream(capture_rate) {
                send_latest(
                    &events,
                    &event_rx,
                    LiveMeteorEvent::Error(error.to_string()),
                );
                break;
            }
            send_latest(
                &events,
                &event_rx,
                LiveMeteorEvent::Status(format!(
                    "Capture gap detected; decoder synchronization reset ({} dropped block(s)).",
                    received_drops.saturating_sub(observed_drops)
                )),
            );
        }

        let sample_count = if let Some(samples) = &block.float_samples {
            decoder.push_complex(samples);
            samples.len() as u64
        } else {
            let complete = block.samples.len() & !1;
            decoder.push_samples(&block.samples[..complete]);
            (complete / 2) as u64
        };
        expected_source_index = block
            .clock
            .source_sample_index
            .checked_add(sample_count.saturating_mul(u64::from(block.clock.sample_stride.max(1))));
        last_epoch = Some(block.clock.epoch);
        last_generation = Some(block.source_generation);
        last_center = Some(block.center_hz);
        last_rate = capture_rate;
        observed_drops = received_drops;

        if let Some(progress) = decoder_rx.try_iter().last() {
            send_latest(&events, &event_rx, LiveMeteorEvent::Progress(progress));
        }
    }

    // Progress is also sampled on stop so the last partial image remains visible.
    let final_progress = decoder.progress_snapshot();
    if final_progress.lines_decoded > 0 || final_progress.segments_decoded > 0 {
        send_latest(
            &events,
            &event_rx,
            LiveMeteorEvent::Progress(final_progress),
        );
    }
    let result = decoder.finish();
    send_latest(&events, &event_rx, LiveMeteorEvent::Finished(result));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture_stream::CaptureStream;
    use crate::source_manager::CaptureBlock;

    #[test]
    fn live_worker_uses_an_independent_bounded_subscription_and_stops_without_repaint() {
        let stream = CaptureStream::default();
        let (stop, events, handle) = start(
            stream.clone(),
            2_048_000,
            ez_proto::MeteorProfile::default(),
            Some("Meteor-M2-4".into()),
            lrpt_decode::CdsEpoch::MeteorM2x,
        )
        .unwrap();
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(1)),
            Ok(LiveMeteorEvent::Status(status)) if status.contains("Live LRPT listening")
        ));
        stream.start(4);
        let session = stream.session();
        let mut block = CaptureBlock::fixture(
            vec![128; 4096],
            ez_proto::CaptureClock::new(7, 0, 2_048_000, None),
            4,
            0,
        );
        block.center_hz = 137_100_000;
        block.session = Some(session);
        stream.publish(&block);
        stop.store(true, Ordering::Release);
        handle.join().unwrap();
        assert!(events
            .try_iter()
            .any(|event| matches!(event, LiveMeteorEvent::Finished(_))));
    }

    #[test]
    fn empty_live_worker_always_emits_a_terminal_event() {
        let stream = CaptureStream::default();
        let (stop, events, handle) = start(
            stream,
            2_048_000,
            ez_proto::MeteorProfile::default(),
            None,
            lrpt_decode::CdsEpoch::MeteorM2x,
        )
        .unwrap();
        stop.store(true, Ordering::Release);
        handle.join().unwrap();
        assert!(events
            .try_iter()
            .any(|event| matches!(event, LiveMeteorEvent::Finished(None))));
    }
}
