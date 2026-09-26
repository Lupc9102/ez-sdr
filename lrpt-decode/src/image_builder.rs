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

#[derive(Debug, Default)]
struct ChannelImage {
    pixels: Vec<u8>,
    height: usize,
    offset: Option<u32>,
    rollover: u32,
    last_sequence: u16,
}

#[derive(Debug, Default)]
pub struct ImageBuilder {
    channels: BTreeMap<u16, ChannelImage>,
    dropped_segments: u64,
}

impl ImageBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Decode one complete image Space Packet and place its 14 JPEG MCUs in
    /// the 1568-pixel MSU-MR channel raster. The packet sequence counter and
    /// MCUN field jointly identify the segment's line and horizontal slot.
    pub fn push_packet(&mut self, apid: u16, sequence: u16, payload: &[u8]) {
        if !IMAGE_APIDS.contains(&apid) {
            return;
        }
        let Some(segment) = decode_segment(payload) else {
            self.dropped_segments = self.dropped_segments.saturating_add(1);
            return;
        };
        let channel = self.channels.entry(apid).or_default();
        if channel.last_sequence > sequence && channel.last_sequence > 13_926 && sequence < 2_458 {
            channel.rollover = channel.rollover.saturating_add(16_384);
        }
        let mcu_count = u32::from(segment.mcu_number) / BLOCKS_PER_SEGMENT as u32;
        if mcu_count >= SEGMENTS_PER_LINE as u32 {
            self.dropped_segments = self.dropped_segments.saturating_add(1);
            return;
        }
        let offset = *channel.offset.get_or_insert_with(|| {
            let sequence = u32::from(sequence);
            let adjusted = sequence + u32::from(mcu_count > sequence) * 16_384;
            adjusted.saturating_sub(mcu_count) % 43
        });
        let absolute_sequence = u32::from(sequence) + channel.rollover;
        if absolute_sequence < offset {
            self.dropped_segments = self.dropped_segments.saturating_add(1);
            return;
        }
        let segment_id = ((absolute_sequence - offset) / 43) * SEGMENTS_PER_LINE as u32 + mcu_count;
        let line = segment_id as usize / SEGMENTS_PER_LINE;
        let column = segment_id as usize % SEGMENTS_PER_LINE;
        let required_height = (line + 1) * SEGMENT_HEIGHT;
        if required_height > MAX_CHANNEL_HEIGHT
            || required_height
                .checked_mul(IMAGE_WIDTH)
                .is_none_or(|pixels| pixels > MAX_CHANNEL_PIXELS)
        {
            self.dropped_segments = self.dropped_segments.saturating_add(1);
            return;
        }
        if required_height > channel.height {
            channel.pixels.resize(required_height * IMAGE_WIDTH, 0);
            channel.height = required_height;
        }
        let x = column * SEGMENT_WIDTH;
        let y = line * SEGMENT_HEIGHT;
        for row in 0..SEGMENT_HEIGHT {
            let dst = (y + row) * IMAGE_WIDTH + x;
            channel.pixels[dst..dst + SEGMENT_WIDTH]
                .copy_from_slice(&segment.pixels[row * SEGMENT_WIDTH..(row + 1) * SEGMENT_WIDTH]);
        }
        channel.last_sequence = sequence;
    }

    #[must_use]
    pub fn render(&self, apid: u16) -> Option<GrayImage> {
        let channel = self.channels.get(&apid)?;
        if channel.height == 0 {
            return None;
        }
        GrayImage::from_raw(
            IMAGE_WIDTH as u32,
            channel.height as u32,
            channel.pixels.clone(),
        )
    }

    #[must_use]
    pub fn render_all(&self) -> Vec<(u16, GrayImage)> {
        self.channels
            .iter()
            .filter_map(|(&apid, _)| self.render(apid).map(|image| (apid, image)))
            .collect()
    }

    /// Bounded display snapshots without cloning the growing full-pass rasters.
    /// Full-resolution pixels stay available through `render_all` at completion.
    #[must_use]
    pub fn render_previews(&self, max_side: usize) -> Vec<(u16, GrayImage)> {
        self.channels
            .iter()
            .filter(|(_, channel)| channel.height > 0)
            .map(|(&apid, channel)| {
                let step = IMAGE_WIDTH.max(channel.height).div_ceil(max_side.max(1));
                let width = IMAGE_WIDTH.div_ceil(step);
                let height = channel.height.div_ceil(step);
                let preview = GrayImage::from_fn(width as u32, height as u32, |x, y| {
                    let source_x = (x as usize * step).min(IMAGE_WIDTH - 1);
                    let source_y = (y as usize * step).min(channel.height - 1);
                    image::Luma([channel.pixels[source_y * IMAGE_WIDTH + source_x]])
                });
                (apid, preview)
            })
            .collect()
    }

    #[must_use]
    pub fn total_line_count(&self) -> u32 {
        self.channels
            .values()
            .map(|channel| channel.height as u32)
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
    std::array::from_fn(|i| ((scale / 100.0 * QTABLE[i]) + 0.5).max(1.0))
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
        let full = builder.render(64).unwrap();
        assert!(full.height() > 1_024);
        let preview = builder.render_previews(1_024);
        assert_eq!(preview.len(), 1);
        assert!(preview[0].1.width() <= 1_024 && preview[0].1.height() <= 1_024);
        assert_eq!(builder.render(64).unwrap(), full);
    }

    #[test]
    fn known_apid_labels_are_documented() {
        assert_eq!(apid_channel_label(64), Some("MSU-MR Channel 1"));
        assert!(apid_channel_label(9999).is_none());
    }
}
