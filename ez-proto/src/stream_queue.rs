//! Bounded live stream buffering shared by network writers and GUI receivers.
//! Controls use a separate queue. Snapshots replace older snapshots, while
//! audio keeps a short consecutive tail. Every skipped update is reported.

use crate::{ChannelId, ServerEvent};
use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;

pub const STREAM_AUDIO_CAPACITY: usize = 8;
pub const STREAM_CHANNEL_CAPACITY: usize = 32;
/// Final LRPT products can contain six 1568x16384 rasters plus per-row timestamps.
/// The envelope is still finite; exceeding it disconnects the stream instead of silently
/// truncating a completed product.
pub const STREAM_BYTE_CAPACITY: usize = 160 * 1024 * 1024;
const TELEMETRY_FINAL_CAPACITY: usize = 8;

fn is_critical_telemetry(event: &ServerEvent) -> bool {
    matches!(
        event,
        ServerEvent::Telemetry(frame)
            if frame.final_product
                || frame.product_complete
                || (frame.width == 0 && frame.height == 0 && frame.pixels.is_empty())
    )
}

#[derive(Default)]
struct Slot {
    frames: VecDeque<(ServerEvent, Instant, usize)>,
    skipped: u64,
    observed_at: Option<Instant>,
    // Preserve the queued front after emitting a gap so a fast producer cannot starve it.
    protect_front: bool,
}

#[derive(Default)]
pub struct StreamQueue {
    channels: BTreeMap<ChannelId, Slot>,
    last_channel: Option<ChannelId>,
    bytes: usize,
    skipped: u64,
}

impl StreamQueue {
    pub fn is_stream(event: &ServerEvent) -> bool {
        matches!(
            event,
            ServerEvent::Spectrum(_)
                | ServerEvent::Audio(_)
                | ServerEvent::Aircraft(_)
                | ServerEvent::Telemetry(_)
                | ServerEvent::StreamGap { .. }
        )
    }

    /// False requires an explicit channel-limit error from the caller.
    pub fn push(&mut self, event: ServerEvent, observed_at: Instant) -> bool {
        let (id, capacity) = match &event {
            ServerEvent::Spectrum(frame) => (frame.channel_id, 1),
            ServerEvent::Audio(frame) => (frame.channel_id, STREAM_AUDIO_CAPACITY),
            ServerEvent::Aircraft(frame) => (frame.channel_id, 1),
            ServerEvent::Telemetry(frame) => (frame.channel_id, TELEMETRY_FINAL_CAPACITY),
            ServerEvent::StreamGap {
                channel_id,
                dropped_frames,
            } => {
                return self.note_gap(*channel_id, *dropped_frames, observed_at);
            }
            _ => return false,
        };
        if !self.channels.contains_key(&id) && self.channels.len() >= STREAM_CHANNEL_CAPACITY {
            self.skipped = self.skipped.saturating_add(1);
            return false;
        }
        let weight = weight(&event);
        let slot = self.channels.entry(id).or_default();
        slot.observed_at = Some(observed_at);
        let final_telemetry = is_critical_telemetry(&event);
        if final_telemetry && slot.frames.len() >= capacity {
            return false;
        }
        if let ServerEvent::Telemetry(frame) = &event {
            if !frame.final_product && !frame.product_complete {
                let protected = if slot.protect_front { 1 } else { 0 };
                if let Some(index) = slot
                    .frames
                    .iter()
                    .enumerate()
                    .skip(protected)
                    .filter_map(|(index, (queued, _, _))| {
                        matches!(queued, ServerEvent::Telemetry(queued)
                            if !queued.final_product
                                && !queued.product_complete
                                && (queued.width != 0 || queued.height != 0 || !queued.pixels.is_empty()))
                        .then_some(index)
                    })
                    .next_back()
                {
                    let (_, _, old_weight) = slot.frames.remove(index).unwrap();
                    self.bytes -= old_weight;
                    slot.skipped = slot.skipped.saturating_add(1);
                    self.skipped = self.skipped.saturating_add(1);
                }
            }
        }
        while slot.frames.len() >= capacity {
            let protected = if slot.protect_front { 1 } else { 0 };
            let Some(index) = slot.frames.iter().enumerate().skip(protected).find_map(
                |(index, (queued, _, _))| (!is_critical_telemetry(queued)).then_some(index),
            ) else {
                if !final_telemetry {
                    slot.skipped = slot.skipped.saturating_add(1);
                    self.skipped = self.skipped.saturating_add(1);
                    return true;
                }
                return false;
            };
            let (_, _, old_weight) = slot.frames.remove(index).unwrap();
            self.bytes -= old_weight;
            slot.skipped = slot.skipped.saturating_add(1);
            self.skipped = self.skipped.saturating_add(1);
        }
        slot.frames.push_back((event, observed_at, weight));
        self.bytes += weight;
        while self.bytes > STREAM_BYTE_CAPACITY {
            let id = self
                .channels
                .iter()
                .filter_map(|(&id, slot)| {
                    let protected = if slot.protect_front { 1 } else { 0 };
                    slot.frames.iter().enumerate().skip(protected).find_map(
                        |(_, (event, at, _))| (!is_critical_telemetry(event)).then_some((id, *at)),
                    )
                })
                .min_by_key(|(_, at)| *at)
                .map(|(id, _)| id);
            let Some(id) = id else {
                return false;
            };
            let slot = self.channels.get_mut(&id).unwrap();
            let protected = if slot.protect_front { 1 } else { 0 };
            let index = slot
                .frames
                .iter()
                .enumerate()
                .skip(protected)
                .find_map(|(index, (event, _, _))| (!is_critical_telemetry(event)).then_some(index))
                .unwrap();
            self.bytes -= slot.frames.remove(index).unwrap().2;
            slot.skipped = slot.skipped.saturating_add(1);
            self.skipped = self.skipped.saturating_add(1);
        }
        true
    }

    pub fn note_gap(&mut self, id: ChannelId, frames: u64, observed_at: Instant) -> bool {
        if frames == 0 {
            return true;
        }
        if !self.channels.contains_key(&id) && self.channels.len() >= STREAM_CHANNEL_CAPACITY {
            return false;
        }
        let slot = self.channels.entry(id).or_default();
        slot.skipped = slot.skipped.saturating_add(frames);
        slot.observed_at = Some(observed_at);
        self.skipped = self.skipped.saturating_add(frames);
        true
    }

    /// Fair across channels, with the loss notice preceding retained data.
    /// Protect the next retained frame after a gap so a continuous producer cannot keep
    /// replacing it between polls and make the consumer observe gaps forever.
    pub fn pop(&mut self) -> Option<(ServerEvent, Instant)> {
        let id = self
            .channels
            .keys()
            .copied()
            .find(|id| self.last_channel.is_none_or(|last| *id > last))
            .or_else(|| self.channels.keys().next().copied())?;
        self.last_channel = Some(id);
        let slot = self.channels.get_mut(&id).unwrap();
        let event = if slot.skipped > 0 && !slot.protect_front {
            slot.protect_front = !slot.frames.is_empty();
            Some((
                ServerEvent::StreamGap {
                    channel_id: id,
                    dropped_frames: std::mem::take(&mut slot.skipped),
                },
                slot.observed_at.unwrap(),
            ))
        } else {
            let next = slot.frames.pop_front().map(|(event, at, weight)| {
                self.bytes -= weight;
                (event, at)
            });
            if next.is_some() {
                slot.protect_front = false;
            }
            next
        };
        if slot.frames.is_empty() && slot.skipped == 0 {
            self.channels.remove(&id);
        }
        event
    }

    pub fn clear(&mut self, id: ChannelId) {
        if let Some(slot) = self.channels.remove(&id) {
            self.bytes -= slot.frames.iter().map(|frame| frame.2).sum::<usize>();
        }
    }

    pub fn queued_frames(&self) -> usize {
        self.channels.values().map(|slot| slot.frames.len()).sum()
    }

    pub fn buffered_bytes(&self) -> usize {
        self.bytes
    }
    pub fn skipped_frames(&self) -> u64 {
        self.skipped
    }
}

fn weight(event: &ServerEvent) -> usize {
    std::mem::size_of::<ServerEvent>()
        + match event {
            ServerEvent::Audio(frame) => frame.samples.len() * 4,
            ServerEvent::Spectrum(frame) => {
                (frame.bins.len() + frame.measurement_bins.as_ref().map_or(0, Vec::len)) * 4
            }
            ServerEvent::Telemetry(frame) => {
                frame.pixels.len() + frame.line_times_unix_seconds.len() * 8
            }
            ServerEvent::Aircraft(frame) => frame
                .aircraft
                .iter()
                .map(|aircraft| {
                    std::mem::size_of_val(aircraft)
                        + aircraft.address_type.len()
                        + aircraft.source.len()
                        + aircraft.callsign.as_ref().map_or(0, String::len)
                        + aircraft.position_source.as_ref().map_or(0, String::len)
                })
                .sum(),
            _ => 0,
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio(n: u64) -> ServerEvent {
        ServerEvent::Audio(crate::AudioFrame {
            audio_clock: crate::CaptureClock::new(7, n * 16, 48_000, None),
            clock: crate::CaptureClock::new(7, n * 16, 48_000, None),
            sequence: n + 1,
            channel_id: 1,
            sample_rate_hz: 48_000,
            channels: 1,
            samples: vec![0.25; 16],
            squelch: crate::SquelchState {
                db: -120.0,
                revision: 0,
            },
        })
    }

    fn spectrum(timestamp_ms: u64) -> ServerEvent {
        ServerEvent::Spectrum(crate::SpectrumFrame {
            channel_id: 2,
            center_hz: 100_000_000,
            sample_rate_hz: 2_000_000,
            bins: vec![timestamp_ms as f32; 16],
            timestamp_ms,
            observation_revision: 0,
            measurement_bins: None,
        })
    }

    #[test]
    fn mixed_stream_queue_retains_fresh_snapshots_and_exact_audio_tail_and_gaps() {
        let mut queue = StreamQueue::default();
        let observed = Instant::now() - std::time::Duration::from_secs(2);
        for n in 0..1000 {
            queue.push(audio(n), observed);
            queue.push(
                ServerEvent::Spectrum(crate::SpectrumFrame {
                    channel_id: 2,
                    center_hz: 100_000_000,
                    sample_rate_hz: 2_000_000,
                    bins: vec![n as f32; 16],
                    timestamp_ms: n,
                    observation_revision: 42,
                    measurement_bins: None,
                }),
                observed,
            );
            queue.push(
                ServerEvent::Aircraft(crate::AircraftSnapshot {
                    channel_id: 3,
                    receiver: crate::ReceiverSnapshot {
                        total_messages: n,
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                observed,
            );
        }
        assert_eq!(queue.queued_frames(), 10);
        assert_eq!(queue.skipped_frames(), 2990);
        let mut retained = Vec::new();
        let mut losses = BTreeMap::new();
        let mut snapshots = 0;
        while let Some((event, at)) = queue.pop() {
            assert_eq!(
                at, observed,
                "queue delay must not manufacture observation freshness"
            );
            match event {
                ServerEvent::StreamGap {
                    channel_id,
                    dropped_frames,
                } => {
                    losses.insert(channel_id, dropped_frames);
                }
                ServerEvent::Audio(frame) => retained.push(frame.sequence),
                ServerEvent::Spectrum(frame) => {
                    assert_eq!(frame.timestamp_ms, 999);
                    assert_eq!(frame.bins, vec![999.0; 16]);
                    snapshots += 1;
                }
                ServerEvent::Aircraft(frame) => {
                    assert_eq!(frame.receiver.total_messages, 999);
                    snapshots += 1;
                }
                _ => unreachable!(),
            }
        }
        assert_eq!(retained, (993..=1000).collect::<Vec<_>>());
        assert_eq!(losses, BTreeMap::from([(1, 992), (2, 999), (3, 999)]));
        assert_eq!(snapshots, 2);
        assert_eq!(queue.buffered_bytes(), 0);
    }

    #[test]
    fn continuous_snapshot_overflow_reports_gaps_without_starving_frames() {
        let mut queue = StreamQueue::default();
        let now = Instant::now();
        queue.push(spectrum(1), now);
        queue.push(spectrum(2), now);

        assert!(matches!(
            queue.pop().map(|(event, _)| event),
            Some(ServerEvent::StreamGap {
                channel_id: 2,
                dropped_frames: 1
            })
        ));

        // Keep the frame announced by the gap until the consumer can read it, even
        // when a newer snapshot arrives in between polls.
        queue.push(spectrum(3), Instant::now());
        assert!(matches!(
            queue.pop().map(|(event, _)| event),
            Some(ServerEvent::Spectrum(frame)) if frame.timestamp_ms == 2
        ));

        queue.push(spectrum(4), Instant::now());
        assert!(matches!(
            queue.pop().map(|(event, _)| event),
            Some(ServerEvent::StreamGap {
                channel_id: 2,
                dropped_frames: 1
            })
        ));
        assert!(matches!(
            queue.pop().map(|(event, _)| event),
            Some(ServerEvent::Spectrum(frame)) if frame.timestamp_ms == 4
        ));
        assert_eq!(queue.skipped_frames(), 2);
        assert_eq!(queue.queued_frames(), 0);
    }

    #[test]
    fn mixed_stream_payload_budget_and_channel_retirement_stay_bounded() {
        let mut queue = StreamQueue::default();
        for id in 0..32 {
            queue.push(
                ServerEvent::Telemetry(crate::TelemetryFrame {
                    product_id: 1,
                    channel_id: id,
                    apid: 0,
                    width: 1024,
                    height: 1024,
                    pixels: vec![0; 1024 * 1024],
                    line_times_unix_seconds: Vec::new(),
                    final_product: false,
                    product_complete: false,
                    rs_ok: 0,
                    rs_failed: 0,
                    costas_locked: false,
                    frame_locked: false,
                }),
                Instant::now(),
            );
            assert!(queue.buffered_bytes() <= STREAM_BYTE_CAPACITY);
        }
        assert_eq!(queue.skipped_frames(), 0);
        let mut extra = audio(0);
        if let ServerEvent::Audio(frame) = &mut extra {
            frame.channel_id = 32;
        }
        assert!(
            !queue.push(extra, Instant::now()),
            "channel limit must be reported to the transport"
        );
        queue.clear(31);
        let mut notices = 0;
        while let Some((event, _)) = queue.pop() {
            match event {
                ServerEvent::StreamGap { channel_id, .. } => {
                    assert_ne!(channel_id, 31);
                    notices += 1;
                }
                ServerEvent::Telemetry(frame) => assert_ne!(frame.channel_id, 31),
                _ => unreachable!(),
            }
        }
        assert_eq!(notices, 0);
        assert_eq!(queue.buffered_bytes(), 0);
    }

    #[test]
    fn final_lrpt_batch_keeps_all_max_height_apids_before_completion() {
        const WIDTH: u32 = 1568;
        const HEIGHT: u32 = 16_384;
        let mut queue = StreamQueue::default();
        for apid in 64..70 {
            let frame = crate::TelemetryFrame {
                product_id: 77,
                channel_id: 4,
                apid,
                width: WIDTH,
                height: HEIGHT,
                pixels: vec![apid as u8; (WIDTH * HEIGHT) as usize],
                line_times_unix_seconds: vec![None; HEIGHT as usize],
                final_product: true,
                product_complete: false,
                rs_ok: 12,
                rs_failed: 0,
                costas_locked: true,
                frame_locked: true,
            };
            assert!(queue.push(ServerEvent::Telemetry(frame), Instant::now()));
        }
        assert!(queue.push(
            ServerEvent::Telemetry(crate::TelemetryFrame {
                product_id: 77,
                channel_id: 4,
                apid: 0,
                width: 0,
                height: 0,
                pixels: Vec::new(),
                line_times_unix_seconds: Vec::new(),
                final_product: false,
                product_complete: true,
                rs_ok: 12,
                rs_failed: 0,
                costas_locked: true,
                frame_locked: true,
            }),
            Instant::now(),
        ));
        assert_eq!(queue.queued_frames(), 7);
        assert!(queue.buffered_bytes() <= STREAM_BYTE_CAPACITY);

        for expected_apid in 64..70 {
            let (event, _) = queue.pop().expect("final channel frame retained");
            assert!(matches!(event, ServerEvent::Telemetry(frame)
                if frame.apid == expected_apid && frame.final_product));
        }
        let (event, _) = queue.pop().expect("completion sentinel retained");
        assert!(matches!(event, ServerEvent::Telemetry(frame) if frame.product_complete));
        assert_eq!(queue.queued_frames(), 0);
    }
}
