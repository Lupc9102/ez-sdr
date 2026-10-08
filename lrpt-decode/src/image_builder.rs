//! Meteor MSU-MR LRPT JPEG-segment decoding and channel-image assembly.

use std::collections::BTreeMap;

use image::GrayImage;

pub const IMAGE_APIDS: &[u16] = &[64, 65, 66, 67, 68, 69];
pub const IMAGE_WIDTH: usize = 1568;
const SEGMENTS_PER_LINE: usize = 14;
const BLOCKS_PER_SEGMENT: usize = 14;
const BLOCK_WIDTH: usize = 8;
const SEGMENT_WIDTH: usize = BLOCKS_PER_SEGMENT * BLOCK_WIDTH;
const SEGMENT_HEIGHT: usize = 8;
pub const MAX_CHANNEL_HEIGHT: usize = 16_384;
pub const MAX_CHANNEL_PIXELS: usize = IMAGE_WIDTH * MAX_CHANNEL_HEIGHT;

#[must_use]
pub fn apid_channel_label(apid: u16) -> Option<&'static str> {
    match apid {
        64 => Some("MSU-MR Channel 1"),
        65 => Some("MSU-MR Channel 2"),
        66 => Some("MSU-MR Channel 3"),
        67 => Some("MSU-MR Channel 4"),
        68 => Some("MSU-MR Channel 5"),
        69 => Some("MSU-MR Channel 6"),
        _ => None,
    }
}

// MSU-MR transmits fourteen segments for each of three channels, then
// telemetry, in a shared 43-packet cycle. Sequence counters wrap at 16384.
const PACKETS_PER_GROUP: i64 = 43;
const SEQUENCE_MODULUS: i64 = 16_384;
const DAY_US: i64 = 86_400_000_000;
// Nominal eight-scanline interval, corroborated by the public M2-4 capture.
// Adjacent observed groups refine this estimate for long-gap unwrapping.
const GROUP_PERIOD_US: f64 = 1_231_142.0;

#[derive(Debug)]
struct ScanGroup {
    segments: [Option<Vec<u8>>; SEGMENTS_PER_LINE],
}

impl Default for ScanGroup {
    fn default() -> Self {
        Self {
            segments: std::array::from_fn(|_| None),
        }
    }
}

#[derive(Debug, Default)]
struct ChannelImage {
    groups: BTreeMap<i64, ScanGroup>,
    phase: i64,
    latest_timed_base: Option<(i64, i64)>,
}

#[derive(Debug)]
struct SequenceClock {
    highest: Option<i64>,
    latest_time: Option<i64>,
    timed_anchors: BTreeMap<i64, i64>,
    group_period_us: f64,
}

impl Default for SequenceClock {
    fn default() -> Self {
        Self {
            highest: None,
            latest_time: None,
            timed_anchors: BTreeMap::new(),
            group_period_us: GROUP_PERIOD_US,
        }
    }
}

fn nearest_sequence(sequence: u16, reference: i64) -> i64 {
    reference
        + (i64::from(sequence) - reference + SEQUENCE_MODULUS / 2).rem_euclid(SEQUENCE_MODULUS)
        - SEQUENCE_MODULUS / 2
}

impl SequenceClock {
    fn timestamp(&self, payload: &[u8]) -> Option<i64> {
        let days = u16::from_be_bytes([payload[0], payload[1]]);
        let millis = u32::from_be_bytes(payload[2..6].try_into().ok()?);
        let micros = u16::from_be_bytes([payload[6], payload[7]]);
        if millis >= 86_400_000 || micros >= 1000 || (days == 0 && millis == 0 && micros == 0) {
            return None;
        }
        let raw = i64::from(days) * DAY_US + i64::from(millis) * 1000 + i64::from(micros);
        if days == 0 {
            // Older Meteor timestamps contain time-of-day only. Reordered
            // packets on either side of midnight must retain their own day.
            if let Some(latest) = self.latest_time {
                return Some(latest + (raw - latest + DAY_US / 2).rem_euclid(DAY_US) - DAY_US / 2);
            }
        }
        Some(raw)
    }

    fn locate(&self, sequence: u16, timestamp: Option<i64>) -> i64 {
        if let Some(time) = timestamp {
            let anchor = self
                .timed_anchors
                .range(..=time)
                .next_back()
                .or_else(|| self.timed_anchors.range(time..).next());
            if let Some((&anchor_time, &anchor_sequence)) = anchor {
                let elapsed_groups =
                    ((time - anchor_time) as f64 / self.group_period_us).round() as i64;
                return nearest_sequence(
                    sequence,
                    anchor_sequence + elapsed_groups * PACKETS_PER_GROUP,
                );
            }
        }
        nearest_sequence(sequence, self.highest.unwrap_or(i64::from(sequence)))
    }

    fn accept(&mut self, absolute: i64, time: Option<i64>) {
        self.highest = Some(self.highest.map_or(absolute, |old| old.max(absolute)));
        if let Some(time) = time {
            self.latest_time = Some(self.latest_time.map_or(time, |old| old.max(time)));
            self.timed_anchors.entry(time).or_insert(absolute);
        }
    }
}

#[derive(Debug, Default)]
pub struct ImageBuilder {
    channels: BTreeMap<u16, ChannelImage>,
    clock: SequenceClock,
    dropped_segments: u64,
}

impl ImageBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Decode a segment and retain its absolute shared sequence/time identity.
    /// Missing segments remain absent; output is cropped to received pass bounds.
    pub fn push_packet(&mut self, apid: u16, sequence: u16, payload: &[u8]) {
        if !IMAGE_APIDS.contains(&apid) {
            return;
        }
        let Some(segment) = decode_segment(payload) else {
            self.dropped_segments = self.dropped_segments.saturating_add(1);
            return;
        };
        let column = usize::from(segment.mcu_number) / BLOCKS_PER_SEGMENT;
        if sequence >= SEQUENCE_MODULUS as u16
            || !usize::from(segment.mcu_number).is_multiple_of(BLOCKS_PER_SEGMENT)
            || column >= SEGMENTS_PER_LINE
        {
            self.dropped_segments = self.dropped_segments.saturating_add(1);
            return;
        }
        let time = self.clock.timestamp(payload);
        let absolute = self.clock.locate(sequence, time);
        let base = absolute - column as i64;
        let phase = base.rem_euclid(PACKETS_PER_GROUP);
        if self
            .channels
            .get(&apid)
            .is_some_and(|channel| channel.phase != phase)
        {
            self.dropped_segments = self.dropped_segments.saturating_add(1);
            return;
        }
        let raw_group = base.div_euclid(PACKETS_PER_GROUP);
        // The lowest transmitted APID begins each instrument channel cycle.
        // Channels whose modulo phase wraps before it belong to the preceding
        // numerical sequence group. Recompute shifts when an earlier APID arrives.
        let first_phase = self
            .channels
            .first_key_value()
            .filter(|(first_apid, _)| **first_apid < apid)
            .map_or(phase, |(_, channel)| channel.phase);
        let candidate_group = raw_group - i64::from(phase < first_phase);
        let mut first = candidate_group;
        let mut last = candidate_group;
        for channel in self.channels.values() {
            let shift = i64::from(channel.phase < first_phase);
            if let Some((&group, _)) = channel.groups.first_key_value() {
                first = first.min(group - shift);
            }
            if let Some((&group, _)) = channel.groups.last_key_value() {
                last = last.max(group - shift);
            }
        }
        let span_height = (last - first + 1) as usize * SEGMENT_HEIGHT;
        if span_height > MAX_CHANNEL_HEIGHT
            || span_height
                .checked_mul(IMAGE_WIDTH)
                .is_none_or(|pixels| pixels > MAX_CHANNEL_PIXELS)
        {
            self.dropped_segments = self.dropped_segments.saturating_add(1);
            return;
        }
        let channel = self.channels.entry(apid).or_insert_with(|| ChannelImage {
            phase,
            ..ChannelImage::default()
        });
        let group = channel.groups.entry(raw_group).or_default();
        if group.segments[column].is_some() {
            return;
        }
        group.segments[column] = Some(segment.pixels);
        if let Some(time) = time {
            if let Some((old_time, old_base)) = channel.latest_timed_base {
                let cycles = (base - old_base) / PACKETS_PER_GROUP;
                if cycles > 0 && time > old_time {
                    let observed = (time - old_time) as f64 / cycles as f64;
                    if (1_000_000.0..1_500_000.0).contains(&observed) {
                        self.clock.group_period_us = observed;
                    }
                }
            }
            if channel
                .latest_timed_base
                .is_none_or(|(old_time, _)| time > old_time)
            {
                channel.latest_timed_base = Some((time, base));
            }
        }
        self.clock.accept(absolute, time);
    }

    fn bounds(&self) -> Option<(i64, i64, i64)> {
        let first_phase = self.channels.first_key_value()?.1.phase;
        let mut first = i64::MAX;
        let mut last = i64::MIN;
        for channel in self.channels.values() {
            let shift = i64::from(channel.phase < first_phase);
            first = first.min(*channel.groups.first_key_value()?.0 - shift);
            last = last.max(*channel.groups.last_key_value()?.0 - shift);
        }
        Some((first, last, first_phase))
    }

    fn pixel(channel: &ChannelImage, group: i64, x: usize, row: usize) -> u8 {
        channel
            .groups
            .get(&group)
            .and_then(|group| group.segments[x / SEGMENT_WIDTH].as_ref())
            .map_or(0, |segment| {
                segment[row * SEGMENT_WIDTH + x % SEGMENT_WIDTH]
            })
    }

    #[must_use]
    pub fn render(&self, apid: u16) -> Option<GrayImage> {
        let channel = self.channels.get(&apid)?;
        let (first, last, first_phase) = self.bounds()?;
        let height = (last - first + 1) as usize * SEGMENT_HEIGHT;
        let shift = i64::from(channel.phase < first_phase);
        Some(GrayImage::from_fn(
            IMAGE_WIDTH as u32,
            height as u32,
            |x, y| {
                image::Luma([Self::pixel(
                    channel,
                    first + i64::from(y / SEGMENT_HEIGHT as u32) + shift,
                    x as usize,
                    y as usize % SEGMENT_HEIGHT,
                )])
            },
        ))
    }

    #[must_use]
    pub fn render_all(&self) -> Vec<(u16, GrayImage)> {
        self.channels
            .keys()
            .filter_map(|&apid| self.render(apid).map(|image| (apid, image)))
            .collect()
    }

    /// Sample sparse groups directly, without allocating full-pass rasters.
    #[must_use]
    pub fn render_previews(&self, max_side: usize) -> Vec<(u16, GrayImage)> {
        let Some((first, last, first_phase)) = self.bounds() else {
            return Vec::new();
        };
        let full_height = (last - first + 1) as usize * SEGMENT_HEIGHT;
        let step = IMAGE_WIDTH.max(full_height).div_ceil(max_side.max(1));
        let width = IMAGE_WIDTH.div_ceil(step);
        let height = full_height.div_ceil(step);
        self.channels
            .iter()
            .map(|(&apid, channel)| {
                let shift = i64::from(channel.phase < first_phase);
                let preview = GrayImage::from_fn(width as u32, height as u32, |x, y| {
                    let source_y = y as usize * step;
                    image::Luma([Self::pixel(
                        channel,
                        first + (source_y / SEGMENT_HEIGHT) as i64 + shift,
                        x as usize * step,
                        source_y % SEGMENT_HEIGHT,
                    )])
                });
                (apid, preview)
            })
            .collect()
    }

    /// Number of complete decoded scanlines. A group contributes eight only
    /// after all fourteen horizontal segments are present; holes never count.
    #[must_use]
    pub fn total_line_count(&self) -> u32 {
        self.channels
            .values()
            .flat_map(|channel| channel.groups.values())
            .filter(|group| group.segments.iter().all(Option::is_some))
            .count() as u32
            * SEGMENT_HEIGHT as u32
    }

    #[must_use]
    pub fn total_segment_count(&self) -> u32 {
        self.channels
            .values()
            .flat_map(|channel| channel.groups.values())
            .map(|group| {
                group
                    .segments
                    .iter()
                    .filter(|segment| segment.is_some())
                    .count() as u32
            })
            .sum()
    }

    #[must_use]
    pub fn dropped_scanlines(&self) -> u64 {
        self.dropped_segments
    }
}

#[derive(Debug)]
struct DecodedSegment {
    mcu_number: u8,
    pixels: Vec<u8>,
}

const QTABLE: [f64; 64] = [
    16., 11., 10., 16., 24., 40., 51., 61., 12., 12., 14., 19., 26., 58., 60., 55., 14., 13., 16.,
    24., 40., 57., 69., 56., 14., 17., 22., 29., 51., 87., 80., 62., 18., 22., 37., 56., 68., 109.,
    103., 77., 24., 35., 55., 64., 81., 104., 113., 92., 49., 64., 78., 87., 103., 121., 120.,
    101., 72., 92., 95., 98., 112., 100., 103., 99.,
];
const ZIGZAG: [usize; 64] = [
    0, 1, 5, 6, 14, 15, 27, 28, 2, 4, 7, 13, 16, 26, 29, 42, 3, 8, 12, 17, 25, 30, 41, 43, 9, 11,
    18, 24, 31, 40, 44, 53, 10, 19, 23, 32, 39, 45, 52, 54, 20, 22, 33, 38, 46, 51, 55, 60, 21, 34,
    37, 47, 50, 56, 59, 61, 35, 36, 48, 49, 57, 58, 62, 63,
];
const DC_COUNTS: [u8; 16] = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
const AC_COUNTS: [u8; 16] = [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 125];
const AC_VALUES: [u8; 162] = [
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07,
    0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xa1, 0x08, 0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0,
    0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x25, 0x26, 0x27, 0x28,
    0x29, 0x2a, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49,
    0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
    0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89,
    0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7,
    0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5,
    0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2,
    0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8,
    0xf9, 0xfa,
];

struct BitReader<'a> {
    bytes: &'a [u8],
    bit: usize,
}
impl<'a> BitReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, bit: 0 }
    }
    fn read(&mut self, count: usize) -> Option<u32> {
        if self.bit + count > self.bytes.len() * 8 {
            return None;
        }
        let mut value = 0;
        for _ in 0..count {
            value = (value << 1) | u32::from((self.bytes[self.bit / 8] >> (7 - self.bit % 8)) & 1);
            self.bit += 1;
        }
        Some(value)
    }
}

fn huffman_symbol(reader: &mut BitReader<'_>, counts: &[u8; 16], values: &[u8]) -> Option<u8> {
    let mut code = 0u32;
    let mut first = 0u32;
    let mut offset = 0usize;
    for length in 1..=16 {
        code = (code << 1) | reader.read(1)?;
        let count = u32::from(counts[length - 1]);
        if code >= first && code < first + count {
            return values.get(offset + (code - first) as usize).copied();
        }
        offset += count as usize;
        first = (first + count) << 1;
    }
    None
}

fn receive_extend(reader: &mut BitReader<'_>, size: usize) -> Option<i32> {
    if size == 0 {
        return Some(0);
    }
    let value = reader.read(size)? as i32;
    let threshold = 1i32 << (size - 1);
    Some(if value < threshold {
        value - ((1i32 << size) - 1)
    } else {
        value
    })
}

fn quantization_table(quality: u8) -> [f64; 64] {
    let quality = f64::from(quality);
    let scale = if (20.0..50.0).contains(&quality) {
        5000.0 / quality
    } else {
        200.0 - 2.0 * quality
    };
    std::array::from_fn(|i| ((scale / 100.0 * QTABLE[i]) + 0.5).floor().max(1.0))
}

fn idct(coefficients: &[f64; 64]) -> [u8; 64] {
    let mut out = [0u8; 64];
    for y in 0..8 {
        for x in 0..8 {
            let mut sum = 0.0;
            for v in 0..8 {
                for u in 0..8 {
                    let cu = if u == 0 {
                        std::f64::consts::FRAC_1_SQRT_2
                    } else {
                        1.0
                    };
                    let cv = if v == 0 {
                        std::f64::consts::FRAC_1_SQRT_2
                    } else {
                        1.0
                    };
                    sum += cu
                        * cv
                        * coefficients[v * 8 + u]
                        * (((2 * x + 1) * u) as f64 * std::f64::consts::PI / 16.0).cos()
                        * (((2 * y + 1) * v) as f64 * std::f64::consts::PI / 16.0).cos();
                }
            }
            out[y * 8 + x] = (sum / 4.0 + 128.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

fn decode_segment(payload: &[u8]) -> Option<DecodedSegment> {
    if payload.len() <= 14
        || payload[9] != 0
        || payload[10] != 0
        || u16::from_be_bytes([payload[11], payload[12]]) != 0xfff0
    {
        return None;
    }
    let mut reader = BitReader::new(&payload[14..]);
    let qtable = quantization_table(payload[13]);
    let mut pixels = vec![0u8; SEGMENT_WIDTH * SEGMENT_HEIGHT];
    let mut last_dc = 0i32;
    for block_number in 0..BLOCKS_PER_SEGMENT {
        let dc_size = usize::from(huffman_symbol(
            &mut reader,
            &DC_COUNTS,
            &(0u8..12).collect::<Vec<_>>(),
        )?);
        last_dc += receive_extend(&mut reader, dc_size)?;
        let mut zigzag = [0i32; 64];
        zigzag[0] = last_dc;
        let mut index = 1usize;
        while index < 64 {
            let symbol = huffman_symbol(&mut reader, &AC_COUNTS, &AC_VALUES)?;
            if symbol == 0 {
                break;
            }
            if symbol == 0xf0 {
                index += 16;
                continue;
            }
            index += usize::from(symbol >> 4);
            if index >= 64 {
                return None;
            }
            let size = usize::from(symbol & 0x0f);
            zigzag[index] = receive_extend(&mut reader, size)?;
            index += 1;
        }
        let mut natural = [0f64; 64];
        for i in 0..64 {
            natural[i] = f64::from(zigzag[ZIGZAG[i]]) * qtable[i];
        }
        let block = idct(&natural);
        for row in 0..8 {
            let dst = row * SEGMENT_WIDTH + block_number * BLOCK_WIDTH;
            pixels[dst..dst + BLOCK_WIDTH].copy_from_slice(&block[row * 8..row * 8 + 8]);
        }
    }
    Some(DecodedSegment {
        mcu_number: payload[8],
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zero_segment(mcu_number: u8) -> Vec<u8> {
        let mut payload = vec![0u8; 14];
        payload[8] = mcu_number;
        payload[11] = 0xff;
        payload[12] = 0xf0;
        payload[13] = 50;
        let mut bits = Vec::new();
        for _ in 0..14 {
            bits.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
        }
        for chunk in bits.chunks(8) {
            let mut byte = 0u8;
            for (i, &bit) in chunk.iter().enumerate() {
                byte |= bit << (7 - i);
            }
            payload.push(byte);
        }
        payload
    }

    #[test]
    fn mission_quantizers_are_integers_on_both_quality_branches() {
        assert_eq!(quantization_table(50), QTABLE);
        assert_eq!(quantization_table(20)[0], 40.0);
        assert_eq!(quantization_table(80)[0], 6.0);
        assert_eq!(quantization_table(100), [1.0; 64]);
        for quality in 0..=255 {
            assert!(quantization_table(quality)
                .iter()
                .all(|value| *value >= 1.0 && value.fract() == 0.0));
        }
    }

    #[test]
    fn nonflat_independent_libjpeg_pixels_match_with_one_level_idct_tolerance() {
        let payload = include_bytes!("../../audit/2026-10-07/fixtures/nonflat-q50.segment");
        let expected = include_bytes!("../../audit/2026-10-07/fixtures/nonflat-q50.pixels");
        let decoded = decode_segment(payload).unwrap();
        assert_eq!(decoded.pixels.len(), expected.len());
        let max_error = decoded
            .pixels
            .iter()
            .zip(expected)
            .map(|(actual, expected)| actual.abs_diff(*expected))
            .max()
            .unwrap();
        assert!(max_error <= 1, "independent JPEG max error: {max_error}");
    }

    #[test]
    fn nonzero_dc_exact_vector_decodes_uniform_pixel_160() {
        let mut payload = vec![0; 14];
        payload[11..14].copy_from_slice(&[0xff, 0xf0, 50]);
        // First MCU: category-5 DC +16 then EOB. Remaining MCUs: DC delta
        // zero + EOB. This is a standard Huffman vector, not our encoder.
        let mut bits = vec![1, 1, 0, 1, 0, 0, 0, 0, 1, 0, 1, 0];
        for _ in 1..14 {
            bits.extend([0, 0, 1, 0, 1, 0]);
        }
        for chunk in bits.chunks(8) {
            payload.push(
                chunk
                    .iter()
                    .enumerate()
                    .fold(0, |byte, (index, bit)| byte | (bit << (7 - index))),
            );
        }
        let decoded = decode_segment(&payload).unwrap();
        assert!(decoded.pixels.iter().all(|pixel| *pixel == 160));
    }

    #[test]
    fn decodes_huffman_quantization_and_idct() {
        let segment = decode_segment(&zero_segment(0)).expect("valid segment");
        assert_eq!(segment.pixels.len(), 112 * 8);
        assert!(segment.pixels.iter().all(|&pixel| pixel == 128));
    }

    #[test]
    fn assembles_fourteen_segments_into_one_full_line() {
        let mut builder = ImageBuilder::new();
        for segment in 0..14u16 {
            builder.push_packet(64, segment, &zero_segment((segment * 14) as u8));
        }
        let image = builder.render(64).expect("image");
        assert_eq!(image.width(), 1568);
        assert_eq!(image.height(), 8);
        assert!(image.pixels().all(|pixel| pixel == &image::Luma([128])));
    }

    #[test]
    fn rejects_invalid_instrument_header() {
        let mut builder = ImageBuilder::new();
        builder.push_packet(64, 0, &[0u8; 20]);
        assert!(builder.render(64).is_none());
        assert_eq!(builder.dropped_scanlines(), 1);
    }

    #[test]
    fn preview_is_bounded_without_changing_full_resolution_image() {
        let mut builder = ImageBuilder::new();
        builder.push_packet(64, 16_000, &zero_segment(0));
        builder.push_packet(64, ((16_000 + 43 * 140) % 16_384) as u16, &zero_segment(0));
        let full = builder.render(64).unwrap();
        assert!(full.height() > 1_024);
        let preview = builder.render_previews(1_024);
        assert_eq!(preview.len(), 1);
        assert!(preview[0].1.width() <= 1_024 && preview[0].1.height() <= 1_024);
        assert_eq!(builder.render(64).unwrap(), full);
    }

    #[test]
    fn late_start_sparse_groups_and_duplicates_count_only_received_segments() {
        let mut builder = ImageBuilder::new();
        builder.push_packet(64, 16_300, &zero_segment(0));
        assert_eq!(builder.render(64).unwrap().height(), 8);
        assert_eq!(builder.total_line_count(), 0);
        assert_eq!(builder.total_segment_count(), 1);
        builder.push_packet(64, 16_300, &zero_segment(0));
        assert_eq!(builder.total_segment_count(), 1);
        // Two entirely missing groups remain black; complete received groups
        // count sixteen scanlines even though their output spans thirty-two.
        for column in 0..14 {
            for base in [16_300, 16_300 + 3 * 43] {
                builder.push_packet(
                    64,
                    ((base + column) % 16_384) as u16,
                    &zero_segment((column * 14) as u8),
                );
            }
        }
        let image = builder.render(64).unwrap();
        assert_eq!(image.height(), 32);
        assert_eq!(builder.total_line_count(), 16);
        assert_eq!(builder.total_segment_count(), 28);
        assert!(image.as_raw()[8 * IMAGE_WIDTH..24 * IMAGE_WIDTH]
            .iter()
            .all(|&pixel| pixel == 0));
        assert_eq!(builder.dropped_scanlines(), 0);
    }

    #[test]
    fn shared_epoch_aligns_channels_straddling_zero_and_reordered_old_packets() {
        let mut builder = ImageBuilder::new();
        // One scan: channel 1 then 2 then 3. Channel 2 wraps mid-row.
        for (apid, base) in [(65, 16_372), (66, 16_386), (64, 16_358)] {
            for column in 0..14 {
                builder.push_packet(
                    apid,
                    ((base + column) % 16_384) as u16,
                    &zero_segment((column * 14) as u8),
                );
            }
        }
        let images = builder.render_all();
        assert_eq!(images.len(), 3);
        assert!(images
            .iter()
            .all(|(_, image)| image.height() == 8
                && image.as_raw().iter().all(|&pixel| pixel == 128)));
        assert_eq!(builder.total_segment_count(), 42);
        assert_eq!(builder.total_line_count(), 24);
        // A delayed pre-wrap duplicate must not increment the epoch again.
        builder.push_packet(65, 16_383, &zero_segment(154));
        builder.push_packet(65, 0, &zero_segment(168));
        assert_eq!(builder.render_all(), images);
        assert_eq!(builder.total_segment_count(), 42);
    }

    fn timed_segment(mcu: u8, time_us: i64, days: u16) -> Vec<u8> {
        let mut payload = zero_segment(mcu);
        payload[..2].copy_from_slice(&days.to_be_bytes());
        payload[2..6].copy_from_slice(&((time_us / 1000) as u32).to_be_bytes());
        payload[6..8].copy_from_slice(&((time_us % 1000) as u16).to_be_bytes());
        payload
    }

    #[test]
    fn timestamps_keep_large_gaps_multiple_rollovers_and_old_reordering_in_one_pass() {
        let mut builder = ImageBuilder::new();
        let first_base = 16_000;
        let time = 10_000_000;
        for group in [0, 1, 400, 800, 1200] {
            let sequence = ((first_base + group * 43) % 16_384) as u16;
            let timestamp = time + (GROUP_PERIOD_US * f64::from(group)).round() as i64;
            builder.push_packet(64, sequence, &timed_segment(0, timestamp, 9000));
        }
        assert_eq!(builder.render(64).unwrap().height(), 1201 * 8);
        assert_eq!(builder.total_segment_count(), 5);
        // Reorder a previously missing segment of the first group after three
        // wraps. Timestamp association prevents moving it to the latest epoch.
        builder.push_packet(64, 16_001, &timed_segment(14, time, 9000));
        let image = builder.render(64).unwrap();
        assert_eq!(image.height(), 1201 * 8);
        assert_eq!(image.get_pixel(112, 0).0, [128]);
        assert_eq!(image.get_pixel(112, 1200 * 8).0, [0]);
        assert_eq!(builder.total_segment_count(), 6);
    }

    #[test]
    fn daily_timestamps_cross_midnight_and_reordering_without_false_epoch_growth() {
        let mut builder = ImageBuilder::new();
        let before = DAY_US - 1_000_000;
        let after = (before + GROUP_PERIOD_US.round() as i64) % DAY_US;
        builder.push_packet(64, 16_380, &timed_segment(0, before, 0));
        builder.push_packet(
            64,
            ((16_380 + 43) % 16_384) as u16,
            &timed_segment(0, after, 0),
        );
        builder.push_packet(64, 16_381, &timed_segment(14, before, 0));
        assert_eq!(builder.render(64).unwrap().height(), 16);
        assert_eq!(builder.total_segment_count(), 3);
        assert_eq!(builder.dropped_scanlines(), 0);
    }

    #[test]
    fn pass_cap_is_relative_and_rejects_growth_without_poisoning_the_clock() {
        let mut builder = ImageBuilder::new();
        let time = 1_000_000;
        builder.push_packet(64, 16_300, &timed_segment(0, time, 9000));
        let too_far = MAX_CHANNEL_HEIGHT as i64 / 8;
        builder.push_packet(
            64,
            ((16_300 + too_far * 43) % 16_384) as u16,
            &timed_segment(
                0,
                time + (GROUP_PERIOD_US * too_far as f64).round() as i64,
                9000,
            ),
        );
        assert_eq!(builder.dropped_scanlines(), 1);
        assert_eq!(builder.total_segment_count(), 1);
        builder.push_packet(
            64,
            16_343,
            &timed_segment(0, time + GROUP_PERIOD_US.round() as i64, 9000),
        );
        assert_eq!(builder.render(64).unwrap().height(), 16);
        assert_eq!(builder.total_segment_count(), 2);
    }

    #[test]
    fn off_air_segments_align_across_channels_rollovers_and_reverse_order() {
        let fixture = include_bytes!("../../audit/2026-10-07/fixtures/public-meteor-segments.bin");
        let mut records = Vec::new();
        let mut cursor = 0;
        while cursor < fixture.len() {
            let read = |offset| {
                u16::from_le_bytes([fixture[cursor + offset], fixture[cursor + offset + 1]])
            };
            let (apid, sequence, group, length) = (read(0), read(2), read(4), usize::from(read(6)));
            cursor += 8;
            let payload = &fixture[cursor..cursor + length];
            cursor += length;
            let expected = &fixture[cursor..cursor + SEGMENT_WIDTH * SEGMENT_HEIGHT];
            cursor += expected.len();
            records.push((apid, sequence, group, payload, expected));
        }
        let mut first_render = None;
        for reverse in [false, true] {
            let mut builder = ImageBuilder::new();
            for index in 0..records.len() {
                let index = if reverse {
                    records.len() - 1 - index
                } else {
                    index
                };
                let (apid, sequence, _, payload, _) = records[index];
                builder.push_packet(apid, sequence, payload);
            }
            assert_eq!(builder.total_segment_count(), 6);
            assert_eq!(builder.total_line_count(), 0);
            assert_eq!(builder.dropped_scanlines(), 0);
            let images = builder.render_all();
            assert_eq!(images.len(), 3);
            assert!(images.iter().all(|(_, image)| image.height() == 5504));
            for (apid, _, group, payload, expected) in &records {
                let image = &images
                    .iter()
                    .find(|(channel, _)| channel == apid)
                    .unwrap()
                    .1;
                let x = usize::from(payload[8] / 14) * SEGMENT_WIDTH;
                let y = usize::from(*group) * SEGMENT_HEIGHT;
                for row in 0..SEGMENT_HEIGHT {
                    for column in 0..SEGMENT_WIDTH {
                        let actual = image.get_pixel((x + column) as u32, (y + row) as u32).0[0];
                        assert!(
                            actual.abs_diff(expected[row * SEGMENT_WIDTH + column]) <= 1,
                            "APID={apid} group={group} pixel=({column},{row})"
                        );
                    }
                }
            }
            if let Some(first) = &first_render {
                assert_eq!(&images, first);
            } else {
                first_render = Some(images);
            }
        }
    }

    #[test]
    fn known_apid_labels_are_documented() {
        assert_eq!(apid_channel_label(64), Some("MSU-MR Channel 1"));
        assert!(apid_channel_label(9999).is_none());
    }
}
