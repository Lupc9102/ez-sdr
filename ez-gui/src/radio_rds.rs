//! Streaming RDS reception from the raw WFM multiplex (before audio filtering).
//!
//! A complex 57 kHz mixer, fourth-order anti-alias filter and 19 kHz resampler
//! feed sixteen staggered biphase matched filters. Complex differential symbols
//! remove arbitrary carrier phase; independent CRC synchronizers acquire bit
//! timing without assuming the input block boundaries. Metadata is committed
//! only after all four 26-bit blocks validate, including the C/C' distinction.
//! No input-sized buffers are retained. The constellation contains at most 64
//! normalized matched-filter symbols from the selected timing phase.

use serde::{Deserialize, Serialize};

const SYMBOL_RATE: f64 = 1_187.5;
const BASEBAND_RATE: f64 = SYMBOL_RATE * 16.0;
const OFFSETS: [u16; 5] = [0x0fc, 0x198, 0x168, 0x1b4, 0x350];
const LOCK_TIMEOUT: u64 = 38_000; // Two seconds at the internal 19 kHz clock.

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RdsRegion {
    #[default]
    Europe,
    NorthAmerica,
}

impl RdsRegion {
    pub fn label(self) -> &'static str {
        match self {
            Self::Europe => "Europe",
            Self::NorthAmerica => "North America",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RdsSnapshot {
    pub synchronized: bool,
    pub pi: Option<u16>,
    pub pty: Option<u8>,
    pub tp: Option<bool>,
    pub ta: Option<bool>,
    pub music: Option<bool>,
    pub program_service: Option<String>,
    pub radio_text: Option<String>,
    pub valid_groups: u64,
    pub crc_errors: u64,
    pub region: RdsRegion,
    /// At most 64 unit-length complex matched-filter symbols, oldest first.
    /// These are received symbols, not a fabricated ideal constellation.
    pub recent_symbols: Vec<[f32; 2]>,
}

impl RdsSnapshot {
    pub fn country_code(&self) -> Option<u8> {
        self.pi
            .filter(|_| self.region == RdsRegion::Europe)
            .map(|pi| (pi >> 12) as u8)
    }

    pub fn program_coverage(&self) -> Option<u8> {
        self.pi
            .filter(|_| self.region == RdsRegion::Europe)
            .map(|pi| ((pi >> 8) & 15) as u8)
    }

    pub fn reference_number(&self) -> Option<u8> {
        self.pi
            .filter(|_| self.region == RdsRegion::Europe)
            .map(|pi| pi as u8)
    }

    pub fn pty_label(&self) -> Option<&'static str> {
        self.pty.map(|pty| match self.region {
            RdsRegion::Europe => PTY_EU[usize::from(pty.min(31))],
            RdsRegion::NorthAmerica => PTY_NA[usize::from(pty.min(31))],
        })
    }
}

const PTY_EU: [&str; 32] = [
    "None",
    "News",
    "Current Affairs",
    "Information",
    "Sport",
    "Education",
    "Drama",
    "Culture",
    "Science",
    "Varied",
    "Pop Music",
    "Rock Music",
    "Easy Listening",
    "Light Classical",
    "Serious Classical",
    "Other Music",
    "Weather",
    "Finance",
    "Children's Programmes",
    "Social Affairs",
    "Religion",
    "Phone In",
    "Travel",
    "Leisure",
    "Jazz Music",
    "Country Music",
    "National Music",
    "Oldies Music",
    "Folk Music",
    "Documentary",
    "Alarm Test",
    "Alarm",
];
const PTY_NA: [&str; 32] = [
    "None",
    "News",
    "Information",
    "Sports",
    "Talk",
    "Rock",
    "Classic Rock",
    "Adult Hits",
    "Soft Rock",
    "Top 40",
    "Country",
    "Oldies",
    "Soft",
    "Nostalgia",
    "Jazz",
    "Classical",
    "Rhythm and Blues",
    "Soft Rhythm and Blues",
    "Foreign Language",
    "Religious Music",
    "Religious Talk",
    "Personality",
    "Public",
    "College",
    "Spanish Talk",
    "Spanish Music",
    "Hip Hop",
    "Unassigned",
    "Unassigned",
    "Weather",
    "Emergency Test",
    "Emergency",
];

#[derive(Clone, Copy, Default)]
struct Iq {
    re: f64,
    im: f64,
}

impl Iq {
    fn energy(self) -> f64 {
        self.re * self.re + self.im * self.im
    }
}

#[derive(Default)]
struct Lowpass {
    b: [f64; 3],
    a: [f64; 2],
    state: [Iq; 2],
}

impl Lowpass {
    fn configure(&mut self, rate: f64, q: f64) {
        let (sin, cos) = (std::f64::consts::TAU * 3_000.0 / rate).sin_cos();
        let alpha = sin / (2.0 * q);
        let a0 = 1.0 + alpha;
        self.b = [
            (1.0 - cos) / (2.0 * a0),
            (1.0 - cos) / a0,
            (1.0 - cos) / (2.0 * a0),
        ];
        self.a = [-2.0 * cos / a0, (1.0 - alpha) / a0];
        self.state = [Iq::default(); 2];
    }

    fn process(&mut self, x: Iq) -> Iq {
        let y = Iq {
            re: self.b[0] * x.re + self.state[0].re,
            im: self.b[0] * x.im + self.state[0].im,
        };
        self.state[0] = Iq {
            re: self.b[1] * x.re - self.a[0] * y.re + self.state[1].re,
            im: self.b[1] * x.im - self.a[0] * y.im + self.state[1].im,
        };
        self.state[1] = Iq {
            re: self.b[2] * x.re - self.a[1] * y.re,
            im: self.b[2] * x.im - self.a[1] * y.im,
        };
        y
    }
}

/// RDS's degree-ten shortened cyclic code: x^10+x^8+x^7+x^5+x^4+x^3+1.
fn syndrome(mut word: u32) -> u16 {
    for bit in (10..26).rev() {
        if word & (1 << bit) != 0 {
            word ^= 0x5b9 << (bit - 10);
        }
    }
    word as u16
}

#[derive(Default)]
struct BlockSync {
    shift: u32,
    bits: u8,
    until_block: u8,
    next_block: usize,
    group: [u16; 4],
    acquired: bool,
    crc_errors: u64,
}

impl BlockSync {
    fn push(&mut self, bit: bool) -> Option<[u16; 4]> {
        self.shift = ((self.shift << 1) | u32::from(bit)) & 0x03ff_ffff;
        self.bits = self.bits.saturating_add(1).min(26);
        if self.bits < 26 {
            return None;
        }
        // Once a group has synchronized, A is expected exactly 26 bits after
        // D. Scanning every intermediate payload window would occasionally
        // mistake a coincidental A syndrome for a new boundary and repeatedly
        // discard particular valid groups with the same transmitted payload.
        if self.next_block != 0 || self.acquired {
            self.until_block -= 1;
            if self.until_block != 0 {
                return None;
            }
            let offset = if self.next_block == 2 && self.group[1] & 0x0800 != 0 {
                OFFSETS[4]
            } else {
                OFFSETS[self.next_block]
            };
            let payload = (self.shift >> 10) as u16;
            let repeated_pi_ok =
                self.next_block != 2 || self.group[1] & 0x0800 == 0 || payload == self.group[0];
            if syndrome(self.shift) == offset && repeated_pi_ok {
                self.group[self.next_block] = payload;
                self.next_block += 1;
                self.until_block = 26;
                if self.next_block == 4 {
                    self.next_block = 0;
                    self.acquired = true;
                    return Some(self.group);
                }
                return None;
            }
            if self.acquired {
                self.crc_errors = self.crc_errors.saturating_add(1);
            }
            self.next_block = 0;
            self.acquired = false;
        }
        if syndrome(self.shift) == OFFSETS[0] {
            self.group[0] = (self.shift >> 10) as u16;
            self.next_block = 1;
            self.until_block = 26;
        }
        None
    }
}

struct Metadata {
    snapshot: RdsSnapshot,
    ps: [u8; 8],
    ps_mask: u8,
    rt: [u8; 64],
    rt_mask: u16,
    rt_ab: Option<bool>,
    rt_version_b: bool,
    incremental: bool,
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            snapshot: RdsSnapshot::default(),
            ps: [b' '; 8],
            ps_mask: 0,
            rt: [b' '; 64],
            rt_mask: 0,
            rt_ab: None,
            rt_version_b: false,
            incremental: true,
        }
    }
}

fn text(bytes: &[u8]) -> String {
    // The protocol's ASCII subset is unambiguous. Unsupported extended RDS
    // glyphs remain visible as replacement characters rather than guessed text.
    bytes
        .iter()
        .map(|&b| match b {
            0x20..=0x7e => b as char,
            0..=0x1f => ' ',
            _ => '\u{fffd}',
        })
        .collect::<String>()
        .trim_end()
        .to_owned()
}

impl Metadata {
    fn accept(&mut self, words: [u16; 4]) {
        let [pi, b, c, d] = words;
        if self.snapshot.pi != Some(pi) {
            let region = self.snapshot.region;
            let groups = self.snapshot.valid_groups;
            let errors = self.snapshot.crc_errors;
            let incremental = self.incremental;
            *self = Self::default();
            self.snapshot.region = region;
            self.snapshot.valid_groups = groups;
            self.snapshot.crc_errors = errors;
            self.incremental = incremental;
        }
        self.snapshot.pi = Some(pi);
        self.snapshot.pty = Some(((b >> 5) & 31) as u8);
        self.snapshot.tp = Some(b & 0x0400 != 0);
        self.snapshot.valid_groups = self.snapshot.valid_groups.saturating_add(1);
        self.snapshot.synchronized = true;
        match b >> 12 {
            0 => {
                self.snapshot.ta = Some(b & 0x10 != 0);
                self.snapshot.music = Some(b & 0x08 != 0);
                let segment = usize::from(b & 3);
                let pair = d.to_be_bytes();
                if self.ps_mask & (1 << segment) != 0
                    && self.ps[2 * segment..2 * segment + 2] != pair
                {
                    self.ps = [b' '; 8];
                    self.ps_mask = 0;
                    self.snapshot.program_service = None;
                }
                self.ps[2 * segment..2 * segment + 2].copy_from_slice(&pair);
                self.ps_mask |= 1 << segment;
                if self.incremental || self.ps_mask == 15 {
                    self.snapshot.program_service = Some(text(&self.ps));
                }
            }
            2 => {
                let ab = b & 0x10 != 0;
                let version_b = b & 0x0800 != 0;
                if self.rt_ab != Some(ab) || version_b != self.rt_version_b {
                    self.rt = [b' '; 64];
                    self.rt_mask = 0;
                    self.snapshot.radio_text = None;
                    self.rt_ab = Some(ab);
                    self.rt_version_b = version_b;
                }
                let segment = usize::from(b & 15);
                let width = if version_b { 2 } else { 4 };
                let start = segment * width;
                if !version_b {
                    self.rt[start..start + 2].copy_from_slice(&c.to_be_bytes());
                }
                self.rt[start + width - 2..start + width].copy_from_slice(&d.to_be_bytes());
                self.rt_mask |= 1 << segment;
                let maximum = width * 16;
                let end = self.rt[..maximum]
                    .iter()
                    .position(|&v| v == b'\r')
                    .unwrap_or(maximum);
                let required = if end == maximum { 16 } else { end / width + 1 };
                let required_mask = ((1_u32 << required) - 1) as u16;
                if self.incremental || self.rt_mask & required_mask == required_mask {
                    self.snapshot.radio_text = Some(text(&self.rt[..end]));
                }
            }
            _ => {}
        }
    }
}

/// A bounded, CPU-local decoder. Call `reset` on retuning, restarting the source,
/// disabling RDS, or changing demodulation mode; a sample-rate change also resets.
pub struct RdsDecoder {
    rate: f64,
    oscillator: Iq,
    rotation: Iq,
    oscillator_count: u16,
    filters: [Lowpass; 2],
    resample_phase: f64,
    previous_filtered: Iq,
    history: [Iq; 16],
    write: usize,
    filled: usize,
    previous_symbols: [Iq; 16],
    previous_valid: [bool; 16],
    synchronizers: [BlockSync; 16],
    clock: u64,
    selected_phase: Option<usize>,
    last_group: u64,
    metadata: Metadata,
    symbols: [[f32; 2]; 64],
    symbol_write: usize,
    symbol_count: usize,
}

impl Default for RdsDecoder {
    fn default() -> Self {
        Self {
            rate: 0.0,
            oscillator: Iq { re: 1.0, im: 0.0 },
            rotation: Iq::default(),
            oscillator_count: 0,
            filters: std::array::from_fn(|_| Lowpass::default()),
            resample_phase: 0.0,
            previous_filtered: Iq::default(),
            history: [Iq::default(); 16],
            write: 0,
            filled: 0,
            previous_symbols: [Iq::default(); 16],
            previous_valid: [false; 16],
            synchronizers: std::array::from_fn(|_| BlockSync::default()),
            clock: 0,
            selected_phase: None,
            last_group: 0,
            metadata: Metadata::default(),
            symbols: [[0.0; 2]; 64],
            symbol_write: 0,
            symbol_count: 0,
        }
    }
}

impl RdsDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        let incremental = self.metadata.incremental;
        let region = self.metadata.snapshot.region;
        *self = Self::default();
        self.set_incremental(incremental);
        self.set_region(region);
    }

    pub fn set_incremental(&mut self, incremental: bool) {
        self.metadata.incremental = incremental;
        if !incremental {
            if self.metadata.ps_mask != 15 {
                self.metadata.snapshot.program_service = None;
            }
            // Never leave a formerly incremental incomplete text on screen.
            let width = if self.metadata.rt_version_b { 2 } else { 4 };
            let maximum = width * 16;
            let end = self.metadata.rt[..maximum]
                .iter()
                .position(|&v| v == b'\r')
                .unwrap_or(maximum);
            let segments = if end == maximum { 16 } else { end / width + 1 };
            let mask = ((1_u32 << segments) - 1) as u16;
            if self.metadata.rt_mask & mask != mask {
                self.metadata.snapshot.radio_text = None;
            }
        }
    }

    pub fn set_region(&mut self, region: RdsRegion) {
        self.metadata.snapshot.region = region;
    }

    pub fn snapshot(&self) -> RdsSnapshot {
        let mut snapshot = self.metadata.snapshot.clone();
        snapshot.synchronized = self.selected_phase.is_some()
            && self.clock.saturating_sub(self.last_group) < LOCK_TIMEOUT;
        snapshot.recent_symbols = (0..self.symbol_count)
            .map(|i| self.symbols[(self.symbol_write + 64 - self.symbol_count + i) % 64])
            .collect();
        snapshot
    }

    /// Input units may be arbitrary finite multiplex amplitude. Rates below
    /// 120 kHz cannot contain the complete 57 kHz RDS channel and are rejected.
    pub fn process_multiplex(&mut self, samples: &[f32], sample_rate: f64) {
        if !sample_rate.is_finite() || !(120_000.0..=20_000_000.0).contains(&sample_rate) {
            self.reset();
            return;
        }
        if (sample_rate - self.rate).abs() > sample_rate * 1e-9 {
            self.reset();
            self.rate = sample_rate;
            let (sin, cos) = (-std::f64::consts::TAU * 57_000.0 / sample_rate).sin_cos();
            self.rotation = Iq { re: cos, im: sin };
            self.filters[0].configure(sample_rate, 0.541_196_100_146_197);
            self.filters[1].configure(sample_rate, 1.306_562_964_876_377);
        }
        for &sample in samples {
            // Non-finite source glitches must not poison persistent IIR state.
            let x = if sample.is_finite() {
                f64::from(sample)
            } else {
                0.0
            };
            let mixed = Iq {
                re: x * self.oscillator.re,
                im: x * self.oscillator.im,
            };
            self.oscillator = Iq {
                re: self.oscillator.re * self.rotation.re - self.oscillator.im * self.rotation.im,
                im: self.oscillator.re * self.rotation.im + self.oscillator.im * self.rotation.re,
            };
            self.oscillator_count += 1;
            if self.oscillator_count == 2_048 {
                self.oscillator_count = 0;
                let norm = self.oscillator.energy().sqrt();
                self.oscillator.re /= norm;
                self.oscillator.im /= norm;
            }
            let first = self.filters[0].process(mixed);
            let filtered = self.filters[1].process(first);
            self.resample_phase += BASEBAND_RATE;
            if self.resample_phase >= self.rate {
                self.resample_phase -= self.rate;
                let fraction = 1.0 - self.resample_phase / BASEBAND_RATE;
                self.push_baseband(Iq {
                    re: self.previous_filtered.re
                        + fraction * (filtered.re - self.previous_filtered.re),
                    im: self.previous_filtered.im
                        + fraction * (filtered.im - self.previous_filtered.im),
                });
            }
            self.previous_filtered = filtered;
        }
    }

    fn push_baseband(&mut self, sample: Iq) {
        self.history[self.write] = sample;
        self.write = (self.write + 1) % 16;
        self.filled = (self.filled + 1).min(16);
        self.clock = self.clock.saturating_add(1);
        if self.filled != 16 {
            return;
        }
        let mut symbol = Iq::default();
        for index in 0..16 {
            let x = self.history[(self.write + index) % 16];
            let sign = if index < 8 { 1.0 } else { -1.0 };
            symbol.re += sign * x.re;
            symbol.im += sign * x.im;
        }
        let phase = self.write;
        let previous = self.previous_symbols[phase];
        let valid = self.previous_valid[phase];
        self.previous_symbols[phase] = symbol;
        self.previous_valid[phase] = symbol.energy() > 1e-20;
        if !valid || !self.previous_valid[phase] {
            return;
        }
        if self.selected_phase == Some(phase) {
            let norm = symbol.energy().sqrt();
            self.symbols[self.symbol_write] =
                [(symbol.re / norm) as f32, (symbol.im / norm) as f32];
            self.symbol_write = (self.symbol_write + 1) % 64;
            self.symbol_count = (self.symbol_count + 1).min(64);
        }
        let bit = symbol.re * previous.re + symbol.im * previous.im < 0.0;
        let old_errors = self.synchronizers[phase].crc_errors;
        let group = self.synchronizers[phase].push(bit);
        if self.selected_phase == Some(phase) {
            self.metadata.snapshot.crc_errors = self
                .metadata
                .snapshot
                .crc_errors
                .saturating_add(self.synchronizers[phase].crc_errors - old_errors);
        }
        if let Some(words) = group {
            let may_switch =
                self.selected_phase.is_none() || self.clock.saturating_sub(self.last_group) > 3_800;
            if self.selected_phase == Some(phase) || may_switch {
                self.selected_phase = Some(phase);
                self.last_group = self.clock;
                self.metadata.accept(words);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_block(data: u16, offset: u16) -> u32 {
        // Serial shift-register encoder, independent of the decoder's division.
        let mut register = 0_u16;
        for bit in (0..16).rev() {
            let feedback = ((register >> 9) ^ (data >> bit)) & 1;
            register = (register << 1) & 0x3ff;
            if feedback != 0 {
                register ^= 0x1b9;
            }
        }
        (u32::from(data) << 10) | u32::from(register ^ offset)
    }

    fn bits(groups: &[[u16; 4]]) -> Vec<bool> {
        let mut output = Vec::new();
        for group in groups {
            for (i, &word) in group.iter().enumerate() {
                let offset = if i == 2 && group[1] & 0x800 != 0 {
                    OFFSETS[4]
                } else {
                    OFFSETS[i]
                };
                let encoded = encode_block(word, offset);
                output.extend((0..26).rev().map(|bit| encoded & (1 << bit) != 0));
            }
        }
        output
    }

    fn ps_groups(pi: u16, name: &[u8; 8]) -> Vec<[u16; 4]> {
        (0..4)
            .map(|segment| {
                [
                    pi,
                    0x0400 | (10 << 5) | 0x18 | segment as u16,
                    0,
                    u16::from_be_bytes([name[segment * 2], name[segment * 2 + 1]]),
                ]
            })
            .collect()
    }

    fn rt_groups(pi: u16, text: &[u8], ab: bool, version_b: bool) -> Vec<[u16; 4]> {
        let width = if version_b { 2 } else { 4 };
        let mut message = text.to_vec();
        message.push(b'\r');
        while message.len() % width != 0 {
            message.push(b' ');
        }
        message
            .chunks(width)
            .enumerate()
            .map(|(segment, bytes)| {
                [
                    pi,
                    0x2000
                        | (10 << 5)
                        | ((ab as u16) << 4)
                        | ((version_b as u16) << 11)
                        | segment as u16,
                    if version_b {
                        pi
                    } else {
                        u16::from_be_bytes([bytes[0], bytes[1]])
                    },
                    u16::from_be_bytes([bytes[width - 2], bytes[width - 1]]),
                ]
            })
            .collect()
    }

    #[test]
    fn crc_offsets_and_single_bit_corruption() {
        for offset in OFFSETS {
            for payload in [0, 1, 0x1234, 0xabcd, 0xffff] {
                let word = encode_block(payload, offset);
                assert_eq!(syndrome(word), offset);
                for bit in 0..26 {
                    assert_ne!(syndrome(word ^ (1 << bit)), offset);
                }
            }
        }
    }

    #[test]
    fn protocol_sync_rejects_corrupt_groups_and_recovers() {
        let groups = ps_groups(0x1234, b"CODEX FM");
        let mut stream = vec![true, false, true, true, false];
        let mut corrupted = bits(&groups[..1]);
        corrupted[60] ^= true;
        stream.extend(corrupted);
        stream.extend(bits(&groups));
        let mut sync = BlockSync::default();
        let decoded: Vec<_> = stream
            .into_iter()
            .filter_map(|bit| sync.push(bit))
            .collect();
        assert_eq!(decoded, groups);
    }

    #[test]
    fn version_b_requires_c_prime_and_repeated_pi() {
        let group = rt_groups(0xbeef, b"B text", true, true)[0];
        let mut sync = BlockSync::default();
        assert_eq!(
            bits(&[group])
                .into_iter()
                .filter_map(|b| sync.push(b))
                .last(),
            Some(group)
        );
        let mut wrong_pi = group;
        wrong_pi[2] ^= 1;
        assert!(bits(&[wrong_pi])
            .into_iter()
            .filter_map(|b| sync.push(b))
            .next()
            .is_none());
        let mut wrong_offset = bits(&[group]);
        let encoded = encode_block(group[2], OFFSETS[2]);
        for (i, bit) in (0..26).rev().enumerate() {
            wrong_offset[52 + i] = encoded & (1 << bit) != 0;
        }
        assert!(wrong_offset
            .into_iter()
            .filter_map(|b| sync.push(b))
            .next()
            .is_none());
    }

    #[test]
    fn metadata_incremental_complete_text_ab_and_station_change() {
        let mut decoder = RdsDecoder::new();
        decoder.set_incremental(false);
        decoder.set_region(RdsRegion::NorthAmerica);
        let groups = ps_groups(0x1234, b"CODEX FM");
        decoder.metadata.accept(groups[0]);
        assert_eq!(decoder.snapshot().program_service, None);
        for &g in &groups[1..] {
            decoder.metadata.accept(g);
        }
        assert_eq!(
            decoder.snapshot().program_service.as_deref(),
            Some("CODEX FM")
        );
        for g in rt_groups(0x1234, b"Actual signal, 57 kHz!", false, false) {
            decoder.metadata.accept(g);
        }
        assert_eq!(
            decoder.snapshot().radio_text.as_deref(),
            Some("Actual signal, 57 kHz!")
        );
        assert_eq!(decoder.snapshot().pty_label(), Some("Country"));
        let changed = rt_groups(0x1234, b"New text after AB flag", true, false);
        decoder.metadata.accept(changed[0]);
        assert_eq!(decoder.snapshot().radio_text, None);
        for &g in &changed[1..] {
            decoder.metadata.accept(g);
        }
        assert_eq!(
            decoder.snapshot().radio_text.as_deref(),
            Some("New text after AB flag")
        );
        decoder.metadata.accept(ps_groups(0x5678, b"NEXT  FM")[0]);
        let s = decoder.snapshot();
        assert_eq!(s.pi, Some(0x5678));
        assert_eq!(s.radio_text, None);
        assert_eq!(s.program_service, None);
        decoder.reset();
        assert_eq!(decoder.snapshot().pi, None);
        assert_eq!(decoder.snapshot().region, RdsRegion::NorthAmerica);
        assert!(!decoder.metadata.incremental);
    }

    #[test]
    fn rbds_region_does_not_mislabel_callsign_pi_as_european_fields() {
        let mut decoder = RdsDecoder::new();
        decoder.metadata.accept(ps_groups(0x54a7, b"ASTRA FM")[0]);
        let eu = decoder.snapshot();
        assert_eq!(eu.country_code(), Some(5));
        assert_eq!(eu.program_coverage(), Some(4));
        assert_eq!(eu.reference_number(), Some(0xa7));
        assert_eq!(eu.pty_label(), Some("Pop Music"));
        decoder.set_region(RdsRegion::NorthAmerica);
        let rbds = decoder.snapshot();
        assert_eq!(rbds.pi, Some(0x54a7));
        assert_eq!(rbds.pty_label(), Some("Country"));
        assert_eq!(rbds.country_code(), None);
        assert_eq!(rbds.program_coverage(), None);
        assert_eq!(rbds.reference_number(), None);
    }

    // Independent transmitter: bit-domain differential encoding, biphase chips,
    // arbitrary symbol epoch/carrier phase, stereo pilot and audio interferers.
    fn waveform(bits: &[bool], rate: f64, offset: f64, phase: f64, noise: f64) -> Vec<f32> {
        let mut sign = 1.0;
        let symbols: Vec<f64> = bits
            .iter()
            .map(|&bit| {
                if bit {
                    sign = -sign;
                }
                sign
            })
            .collect();
        let samples = ((bits.len() as f64 + 0.47) * rate / SYMBOL_RATE).ceil() as usize;
        let mut rng = 0x8765_4321_u32;
        (0..samples)
            .map(|n| {
                let t = n as f64 / rate;
                let position = t * SYMBOL_RATE - 0.47;
                let biphase = if position < 0.0 || position as usize >= symbols.len() {
                    0.0
                } else {
                    symbols[position as usize] * if position.fract() < 0.5 { 1.0 } else { -1.0 }
                };
                rng ^= rng << 13;
                rng ^= rng >> 17;
                rng ^= rng << 5;
                let white = f64::from(rng) / f64::from(u32::MAX) * 2.0 - 1.0;
                (0.065 * biphase * (std::f64::consts::TAU * (57_000.0 + offset) * t + phase).cos()
                    + 0.1 * (std::f64::consts::TAU * 19_000.0 * t + 0.19).sin()
                    + 0.3 * (std::f64::consts::TAU * 1_370.0 * t).sin()
                    + 0.15
                        * (std::f64::consts::TAU * 38_000.0 * t).cos()
                        * (std::f64::consts::TAU * 2_300.0 * t).sin()
                    + white * noise) as f32
            })
            .collect()
    }

    #[test]
    fn full_multiplex_phase_noise_carrier_offset_and_streaming_chunks() {
        for (rate, offset, phase) in [
            (250_000.0, -3.0, 1.17),
            (240_000.5, 3.0, 2.68),
            (2_400_000.0, 0.0, -0.74),
        ] {
            let mut groups = ps_groups(0x54a7, b"ASTRA FM");
            groups.extend(rt_groups(
                0x54a7,
                b"CRC checked radio text 2026",
                false,
                false,
            ));
            let mut transmitted = vec![false; 128];
            for _ in 0..3 {
                transmitted.extend(bits(&groups));
            }
            let audio = waveform(&transmitted, rate, offset, phase, 0.035);
            let mut decoder = RdsDecoder::new();
            decoder.set_incremental(false);
            for chunk in audio.chunks(997) {
                decoder.process_multiplex(chunk, rate);
            }
            let status = decoder.snapshot();
            assert_eq!(
                status.pi,
                Some(0x54a7),
                "rate={rate} offset={offset} groups={}",
                status.valid_groups
            );
            assert_eq!(
                status.program_service.as_deref(),
                Some("ASTRA FM"),
                "rate={rate}"
            );
            assert_eq!(
                status.radio_text.as_deref(),
                Some("CRC checked radio text 2026"),
                "rate={rate}"
            );
            assert!(status.synchronized);
            assert!(status.valid_groups >= 20);
            assert_eq!(status.recent_symbols.len(), 64);
            assert!(status
                .recent_symbols
                .iter()
                .flatten()
                .all(|x| x.is_finite()));
        }
    }

    #[test]
    fn noise_alone_never_publishes_metadata_and_invalid_rate_resets() {
        let empty = vec![false; 4_750];
        let mut audio = waveform(&empty, 250_000.0, 0.0, 0.0, 0.15);
        // Remove the encoded subcarrier; retain pilot/audio and random noise.
        for (n, value) in audio.iter_mut().enumerate() {
            let position = n as f64 / 250_000.0 * SYMBOL_RATE - 0.47;
            if position >= 0.0 && (position as usize) < empty.len() {
                let biphase = if position.fract() < 0.5 { 1.0 } else { -1.0 };
                *value -= (0.065
                    * biphase
                    * (std::f64::consts::TAU * 57_000.0 * n as f64 / 250_000.0).cos())
                    as f32;
            }
        }
        let mut decoder = RdsDecoder::new();
        decoder.process_multiplex(&audio, 250_000.0);
        assert_eq!(decoder.snapshot().pi, None);
        assert!(!decoder.snapshot().synchronized);
        decoder.metadata.accept(ps_groups(0x1234, b"CODEX FM")[0]);
        decoder.process_multiplex(&[0.0], 48_000.0);
        assert_eq!(decoder.snapshot().pi, None);
    }

    fn shaped_waveform(bits: &[bool], rate: f64, carrier_offset: f64, clock_ppm: f64) -> Vec<f32> {
        // Transmitter root-raised-cosine pulse shaping, rolloff=1, at the
        // 2375 chip/s biphase clock. This is deliberately different from the
        // receiver's rectangular matched filter and IIR anti-alias filter.
        let mut shaped = vec![0.0; bits.len() * 16 + 128];
        let mut sign = 1.0;
        for (index, &bit) in bits.iter().enumerate() {
            if bit {
                sign = -sign;
            }
            for half in 0..2 {
                for tap in 0..97 {
                    let u = (tap as f64 - 48.0) / 8.0;
                    let pulse = if (u.abs() - 0.25).abs() < 1e-9 {
                        1.0
                    } else {
                        4.0 * (std::f64::consts::TAU * u).cos()
                            / (std::f64::consts::PI * (1.0 - 16.0 * u * u))
                    };
                    shaped[index * 16 + half * 8 + tap] +=
                        sign * if half == 0 { pulse } else { -pulse };
                }
            }
        }
        let clock_scale = 1.0 + clock_ppm * 1e-6;
        let len = (shaped.len() as f64 * rate / (BASEBAND_RATE * clock_scale)).ceil() as usize;
        let mut rng = 0xc0de_1234_u32;
        (0..len)
            .map(|n| {
                let t = n as f64 / rate;
                let position = t * BASEBAND_RATE * clock_scale;
                let index = position as usize;
                let baseband = if index + 1 < shaped.len() {
                    shaped[index] + position.fract() * (shaped[index + 1] - shaped[index])
                } else {
                    0.0
                };
                rng ^= rng << 13;
                rng ^= rng >> 17;
                rng ^= rng << 5;
                let noise = (f64::from(rng) / f64::from(u32::MAX) * 2.0 - 1.0) * 0.02;
                (baseband
                    * 0.065
                    * (std::f64::consts::TAU * (57_000.0 + carrier_offset) * t + 2.19).cos()
                    + 0.1 * (std::f64::consts::TAU * 19_000.0 * t).sin()
                    + 0.3 * (std::f64::consts::TAU * 1_737.0 * t).cos()
                    + noise) as f32
            })
            .collect()
    }

    #[test]
    fn shaped_biphase_clock_drift_and_version_b_text() {
        let mut groups = ps_groups(0xc512, b"PULSE FM");
        groups.extend(rt_groups(0xc512, b"Shaped RDS signal", true, true));
        let mut transmitted = vec![false; 128];
        for _ in 0..6 {
            transmitted.extend(bits(&groups));
        }
        let audio = shaped_waveform(&transmitted, 250_000.0, -12.0, 100.0);
        let mut decoder = RdsDecoder::new();
        decoder.set_incremental(false);
        for chunk in audio.chunks(3337) {
            decoder.process_multiplex(chunk, 250_000.0);
        }
        let status = decoder.snapshot();
        assert_eq!(status.pi, Some(0xc512));
        assert_eq!(status.program_service.as_deref(), Some("PULSE FM"));
        assert_eq!(status.radio_text.as_deref(), Some("Shaped RDS signal"));
        assert!(status.synchronized);
    }

    #[test]
    fn arbitrary_partitions_match_and_signal_loss_expires_lock() {
        let groups = ps_groups(0x8b91, b"CHUNK FM");
        let mut transmitted = vec![false; 64];
        for _ in 0..3 {
            transmitted.extend(bits(&groups));
        }
        let audio = waveform(&transmitted, 250_000.0, 1.5, 1.8, 0.02);
        let mut whole = RdsDecoder::new();
        let mut split = RdsDecoder::new();
        whole.process_multiplex(&audio, 250_000.0);
        let mut start = 0;
        let sizes = [1, 17, 4096, 83, 997];
        let mut iteration = 0;
        while start < audio.len() {
            let end = (start + sizes[iteration % sizes.len()]).min(audio.len());
            split.process_multiplex(&audio[start..end], 250_000.0);
            start = end;
            iteration += 1;
        }
        assert_eq!(whole.snapshot(), split.snapshot());
        assert!(split.snapshot().synchronized);
        split.process_multiplex(&vec![0.0; 525_000], 250_000.0);
        assert!(!split.snapshot().synchronized);
        // Retuning/restarting must explicitly clear previous-station labels.
        split.reset();
        assert_eq!(split.snapshot().pi, None);
        assert!(split.snapshot().recent_symbols.is_empty());
        split.process_multiplex(&[f32::NAN, f32::INFINITY, 0.0], 250_000.0);
        split.process_multiplex(&audio, 250_000.0);
        assert_eq!(
            split.snapshot().program_service.as_deref(),
            Some("CHUNK FM")
        );
        split.process_multiplex(&[0.0], 240_000.0);
        assert_eq!(split.snapshot().pi, None);
    }
}
