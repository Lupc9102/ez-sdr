//! Streaming two-sample/bit UAT FSK synchronization and demodulation.

use std::collections::VecDeque;

use num_complex::Complex32;

use crate::fec::{correct_downlink, correct_uplink};
use crate::message::{decode_adsb, MessageKind, UatMessage};

pub const SAMPLE_RATE_HZ: u32 = 2_083_333;
const SYNC_BITS: usize = 36;
const DOWNLINK_SYNC: u64 = 0xEACD_DA4E2;
const UPLINK_SYNC: u64 = 0x1532_25B1D;
const MAX_FRAME_BITS: usize = SYNC_BITS + 4416;
const MAX_FRAME_SAMPLES: usize = MAX_FRAME_BITS * 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigError {
    UnsupportedSampleRate(u32),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedSampleRate(rate) => write!(
                f,
                "UAT requires {SAMPLE_RATE_HZ} samples/s (got {rate}); resample IQ before decoding"
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SampleGap {
    pub expected_sample: u64,
    pub observed_sample: u64,
}

/// Incremental UAT receiver. It accepts finite, arbitrary-sized sample chunks.
#[derive(Debug)]
pub struct UatDecoder {
    samples: VecDeque<Complex32>,
    base_sample: u64,
    next_sample: u64,
    sample_rate_hz: u32,
}

impl UatDecoder {
    pub fn new(sample_rate_hz: u32) -> Result<Self, ConfigError> {
        if sample_rate_hz != SAMPLE_RATE_HZ {
            return Err(ConfigError::UnsupportedSampleRate(sample_rate_hz));
        }
        Ok(Self {
            samples: VecDeque::with_capacity(MAX_FRAME_SAMPLES * 2),
            base_sample: 0,
            next_sample: 0,
            sample_rate_hz,
        })
    }

    pub fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz
    }

    /// Append complex baseband samples and return every complete corrected frame.
    /// If samples were dropped upstream, call `reset` before appending the next chunk.
    pub fn push(&mut self, input: &[Complex32]) -> Vec<UatMessage> {
        self.push_at(self.next_sample, input).0
    }

    /// Append samples with their absolute stream position. A discontinuity clears
    /// partial FSK/FEC state and is returned as metadata with the decoded frames.
    pub fn push_at(
        &mut self,
        first_sample: u64,
        input: &[Complex32],
    ) -> (Vec<UatMessage>, Option<SampleGap>) {
        let gap = (first_sample != self.next_sample).then_some(SampleGap {
            expected_sample: self.next_sample,
            observed_sample: first_sample,
        });
        if gap.is_some() {
            self.samples.clear();
            self.base_sample = first_sample;
            self.next_sample = first_sample;
        }
        if input.is_empty() {
            return (Vec::new(), gap);
        }
        self.samples.extend(input.iter().copied());
        self.next_sample = self.next_sample.saturating_add(input.len() as u64);
        (self.decode_ready(), gap)
    }

    /// Discard partial sync/frame state after a source restart or capture gap.
    /// The absolute sample counter is retained so emitted indices remain monotonic.
    pub fn reset(&mut self) {
        self.samples.clear();
        self.base_sample = self.next_sample;
    }

    fn decode_ready(&mut self) -> Vec<UatMessage> {
        let mut output = Vec::new();
        loop {
            if self.samples.len() < SYNC_BITS * 2 {
                break;
            }
            let Some((start, kind, threshold)) = find_sync(&self.samples) else {
                // Keep enough samples to cover a sync word that straddles calls.
                let keep = SYNC_BITS * 2 - 1;
                let drop_count = self.samples.len().saturating_sub(keep);
                self.discard_front(drop_count);
                break;
            };
            if start > 0 {
                self.discard_front(start);
            }
            let kind = if kind == MessageKind::DownlinkShort {
                if self.samples.len() < SYNC_BITS * 2 + 16 {
                    break;
                }
                let header = demod_bytes(&self.samples, SYNC_BITS * 2, 1, threshold)[0];
                if header >> 3 == 0 {
                    MessageKind::DownlinkShort
                } else {
                    MessageKind::DownlinkLong
                }
            } else {
                kind
            };
            let (payload_bytes, frame_bits) = match kind {
                MessageKind::DownlinkShort => (30, 240),
                MessageKind::DownlinkLong => (48, 384),
                MessageKind::Uplink => (552, 4416),
            };
            let required = (SYNC_BITS + frame_bits) * 2;
            if self.samples.len() < required {
                break;
            }
            let payload_start = SYNC_BITS * 2;
            let raw = demod_bytes(&self.samples, payload_start, payload_bytes, threshold);
            let corrected = match kind {
                MessageKind::DownlinkShort | MessageKind::DownlinkLong => correct_downlink(&raw),
                MessageKind::Uplink => correct_uplink(&raw),
            };
            let Some((payload, corrected_symbols)) = corrected else {
                // A plausible sync may precede noise. Advance one sample so a real
                // overlapping/nearby frame is still considered.
                self.discard_front(1);
                continue;
            };
            let frame_start = self.base_sample;
            let power = self
                .samples
                .iter()
                .take(required)
                .map(|sample| sample.norm_sqr() as f64)
                .sum::<f64>()
                / required as f64;
            let rssi_dbfs = if power > 0.0 {
                (10.0 * power.log10()) as f32
            } else {
                -120.0
            };
            let actual_kind = if kind == MessageKind::DownlinkLong && payload.len() == 18 {
                MessageKind::DownlinkShort
            } else {
                kind
            };
            let adsb = match actual_kind {
                MessageKind::DownlinkShort | MessageKind::DownlinkLong => decode_adsb(&payload),
                MessageKind::Uplink => None,
            };
            output.push(UatMessage {
                kind: actual_kind,
                payload,
                corrected_symbols,
                first_sample: frame_start,
                rssi_dbfs,
                adsb,
            });
            self.discard_front(required);
        }
        output
    }

    fn discard_front(&mut self, count: usize) {
        for _ in 0..count.min(self.samples.len()) {
            self.samples.pop_front();
            self.base_sample = self.base_sample.saturating_add(1);
        }
    }
}

fn phase_delta(a: Complex32, b: Complex32) -> Option<f32> {
    if a.norm_sqr() < 1e-12 || b.norm_sqr() < 1e-12 {
        return None;
    }
    Some((a.conj() * b).arg())
}

fn bit_delta(samples: &VecDeque<Complex32>, start: usize, bit: usize) -> Option<f32> {
    let a = *samples.get(start + bit * 2)?;
    let b = *samples.get(start + bit * 2 + 1)?;
    phase_delta(a, b)
}

fn bit(word: u64, index: usize) -> bool {
    word & (1u64 << (SYNC_BITS - index - 1)) != 0
}

fn match_sync(samples: &VecDeque<Complex32>, start: usize, word: u64) -> Option<(usize, f32)> {
    let mut zeros = 0.0f32;
    let mut ones = 0.0f32;
    let mut nzeros = 0usize;
    let mut nones = 0usize;
    let mut deltas = [0.0f32; SYNC_BITS];
    for (i, slot) in deltas.iter_mut().enumerate() {
        let delta = bit_delta(samples, start, i)?;
        *slot = delta;
        if bit(word, i) {
            ones += delta;
            nones += 1;
        } else {
            zeros += delta;
            nzeros += 1;
        }
    }
    if nzeros == 0 || nones == 0 {
        return None;
    }
    let threshold = (zeros / nzeros as f32 + ones / nones as f32) / 2.0;
    let errors = deltas
        .iter()
        .enumerate()
        .filter(|(i, delta)| {
            if bit(word, *i) {
                **delta < threshold
            } else {
                **delta > threshold
            }
        })
        .count();
    (errors <= 4).then_some((errors, threshold))
}

fn find_sync(samples: &VecDeque<Complex32>) -> Option<(usize, MessageKind, f32)> {
    let last_start = samples.len().saturating_sub(SYNC_BITS * 2);
    for start in 0..=last_start {
        for (word, kind) in [
            (DOWNLINK_SYNC, MessageKind::DownlinkLong),
            (UPLINK_SYNC, MessageKind::Uplink),
        ] {
            if let Some((_, threshold)) = match_sync(samples, start, word) {
                if kind == MessageKind::Uplink {
                    return Some((start, kind, threshold));
                }
                return Some((start, MessageKind::DownlinkShort, threshold));
            }
        }
    }
    None
}

fn demod_bytes(
    samples: &VecDeque<Complex32>,
    start: usize,
    count: usize,
    threshold: f32,
) -> Vec<u8> {
    (0..count)
        .map(|byte_index| {
            let mut byte = 0u8;
            for bit_index in 0..8 {
                let bit = byte_index * 8 + bit_index;
                let delta = bit_delta(samples, start, bit).unwrap_or(threshold - 1.0);
                byte = (byte << 1) | u8::from(delta > threshold);
            }
            byte
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use num_complex::Complex32;

    #[test]
    fn validates_sample_clock() {
        assert_eq!(
            UatDecoder::new(2_000_000).unwrap_err(),
            ConfigError::UnsupportedSampleRate(2_000_000)
        );
        assert!(UatDecoder::new(SAMPLE_RATE_HZ).is_ok());
    }

    #[test]
    fn decodes_official_sample_payload_from_partitioned_synthetic_iq() {
        // The payload is the first 34-byte downlink in FlightAware's published
        // sample-data.txt.gz. The parity symbols were generated independently
        // with dump978's upstream encoder; this test exercises the actual IQ path.
        let codeword = hex("08a66ef1353e2d525fd4050911882aa038101d06b85d440be2a4c2a0000590000000d0e3c7ccb1fed50a5afd9d6aa963");
        let iq = make_iq(DOWNLINK_SYNC, &codeword);
        let samples = iq.iter().copied().collect::<VecDeque<_>>();
        let (start, kind, threshold) = find_sync(&samples).expect("synthetic sync");
        assert_eq!(start, 0);
        assert_eq!(kind, MessageKind::DownlinkShort);
        assert_eq!(
            demod_bytes(&samples, SYNC_BITS * 2, 48, threshold),
            codeword
        );
        let mut decoder = UatDecoder::new(SAMPLE_RATE_HZ).unwrap();
        let mut messages = Vec::new();
        let mut offset = 0;
        for size in [1usize, 17, 31, 2, 113, 5, 97, 251, 89, 1000] {
            if offset == iq.len() {
                break;
            }
            let end = (offset + size).min(iq.len());
            let (mut batch, gap) = decoder.push_at(50 + offset as u64, &iq[offset..end]);
            if offset == 0 {
                assert_eq!(
                    gap,
                    Some(SampleGap {
                        expected_sample: 0,
                        observed_sample: 50
                    })
                );
            } else {
                assert_eq!(gap, None);
            }
            messages.append(&mut batch);
            offset = end;
        }
        if offset < iq.len() {
            let (mut batch, gap) = decoder.push_at(50 + offset as u64, &iq[offset..]);
            assert_eq!(gap, None);
            messages.append(&mut batch);
        }
        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.kind, MessageKind::DownlinkLong);
        assert_eq!(
            message.payload,
            hex("08a66ef1353e2d525fd4050911882aa038101d06b85d440be2a4c2a0000590000000")
        );
        assert_eq!(message.first_sample, 50);
        let report = message.adsb.as_ref().unwrap();
        assert_eq!(report.address, 0x00a66ef1);
        assert_eq!(report.address_qualifier, 0);
        assert_eq!(report.callsign.as_deref(), Some("N5130E"));
        assert_eq!(report.position, Some((37.43639, 118.95972)));
        assert_eq!(report.pressure_altitude_ft, Some(975));
        assert_eq!(report.north_velocity_kt, Some(-97));
        assert_eq!(report.east_velocity_kt, Some(84));
        assert_eq!(report.vertical_velocity_barometric_fpm, Some(-128));
        assert_eq!(
            report.air_ground_state,
            Some(crate::AirGroundState::Airborne)
        );
        assert!((report.ground_speed_kt.unwrap() - 128.3).abs() < 0.1);
        assert!((report.true_track_deg.unwrap() - 139.1).abs() < 0.1);
        assert_eq!(decoder.push(&[]), Vec::<UatMessage>::new());
    }

    #[test]
    fn decodes_six_interleaved_uplink_blocks_from_iq() {
        let raw = include_bytes!("../tests/fixtures/uplink_encoded.bin");
        let iq = make_iq(UPLINK_SYNC, raw);
        assert_eq!(iq.len(), (SYNC_BITS + 4416) * 2);
        let mut decoder = UatDecoder::new(SAMPLE_RATE_HZ).unwrap();
        let mut messages = Vec::new();
        let mut offset = 0usize;
        for size in [1usize, 87, 3001, 17, 511, 1200, 2033, 5000] {
            if offset == iq.len() {
                break;
            }
            let end = (offset + size).min(iq.len());
            messages.extend(decoder.push(&iq[offset..end]));
            offset = end;
        }
        if offset < iq.len() {
            messages.extend(decoder.push(&iq[offset..]));
        }
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].kind, MessageKind::Uplink);
        assert_eq!(
            messages[0].payload,
            (0..432).map(|byte| byte as u8).collect::<Vec<_>>()
        );
        assert!(messages[0].adsb.is_none());
    }

    fn make_iq(sync: u64, payload: &[u8]) -> Vec<Complex32> {
        let mut bits = (0..SYNC_BITS).map(|i| bit(sync, i)).collect::<Vec<_>>();
        bits.extend(
            payload
                .iter()
                .flat_map(|byte| (0..8).map(move |i| byte & (1 << (7 - i)) != 0)),
        );
        let mut phase = 0.0f32;
        let mut samples = Vec::with_capacity(bits.len() * 2);
        for value in bits {
            for _ in 0..2 {
                samples.push(Complex32::from_polar(0.8, phase));
                phase += if value { 0.2 } else { -0.2 };
            }
        }
        samples
    }

    fn hex(text: &str) -> Vec<u8> {
        text.as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }
}
