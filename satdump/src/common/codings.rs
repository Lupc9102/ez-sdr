//! Codings - translated from src-core/common/codings/

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Phase0,
    Phase90,
    Phase180,
    Phase270,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Constellation {
    Bpsk,
    Qpsk,
    Oqpsk,
    Psk8,
}

fn rotate_64(word: u64, p: Phase) -> u64 {
    let i = word & 0xaaaaaaaaaaaaaaaa;
    let q = word & 0x5555555555555555;
    let mut w = word;
    match p {
        Phase::Phase0 => {}
        Phase::Phase90 => {
            w = ((i ^ 0xaaaaaaaaaaaaaaaa) >> 1) | (q << 1);
        }
        Phase::Phase180 => {
            w = word ^ 0xffffffffffffffff;
        }
        Phase::Phase270 => {
            w = (i >> 1) | ((q ^ 0x5555555555555555) << 1);
        }
    }
    ((w & 0x5555555555555555) << 1) | ((w & 0xAAAAAAAAAAAAAAAA) >> 1)
}

fn corr_64(v1: u64, v2: u64) -> i32 {
    let mut cor = 0;
    let mut diff = v1 ^ v2;
    while diff != 0 {
        diff &= diff - 1;
        cor += 1;
    }
    64 - cor
}

fn swap_iq(in_: u64) -> u64 {
    let i = in_ & 0xaaaaaaaaaaaaaaaa;
    let q = in_ & 0x5555555555555555;
    (i >> 1) | (q << 1)
}

pub struct Correlator {
    modulation: Constellation,
    syncwords: [u64; 8],
    hard_buf: Vec<u8>,
}

impl Correlator {
    pub fn new(modulation: Constellation, syncword: u64) -> Self {
        let mut syncwords = [0u64; 8];
        match modulation {
            Constellation::Bpsk => {
                syncwords[0] = syncword;
                syncwords[1] = syncword ^ 0xFFFFFFFFFFFFFFFF;
            }
            Constellation::Qpsk => {
                for i in 0..4 {
                    let phase = match i {
                        0 => Phase::Phase0,
                        1 => Phase::Phase90,
                        2 => Phase::Phase180,
                        3 => Phase::Phase270,
                        _ => Phase::Phase0,
                    };
                    syncwords[i] = rotate_64(syncword, phase);
                }
                for i in 4..8 {
                    let phase = match i - 4 {
                        0 => Phase::Phase0,
                        1 => Phase::Phase90,
                        2 => Phase::Phase180,
                        3 => Phase::Phase270,
                        _ => Phase::Phase0,
                    };
                    syncwords[i] = rotate_64(swap_iq(syncword) ^ 0xFFFFFFFFFFFFFFFF, phase);
                }
            }
            _ => {}
        }
        Self {
            modulation,
            syncwords,
            hard_buf: vec![0u8; 8192 * 20],
        }
    }

    pub fn correlate(&mut self, soft_input: &[i8]) -> (usize, Phase, bool, i32) {
        let length = soft_input.len();
        let mut bits = 0usize;
        let mut bytes = 0usize;
        let mut shifter: u8 = 0;
        for &s in soft_input.iter() {
            shifter = (shifter << 1) | if s > 0 { 1 } else { 0 };
            bits += 1;
            if bits == 8 {
                if bytes < self.hard_buf.len() {
                    self.hard_buf[bytes] = shifter;
                }
                bits = 0;
                bytes += 1;
            }
        }
        let mut current = ((self.hard_buf.get(0).copied().unwrap_or(0) as u64) << 56)
            | ((self.hard_buf.get(1).copied().unwrap_or(0) as u64) << 48)
            | ((self.hard_buf.get(2).copied().unwrap_or(0) as u64) << 40)
            | ((self.hard_buf.get(3).copied().unwrap_or(0) as u64) << 32)
            | ((self.hard_buf.get(4).copied().unwrap_or(0) as u64) << 24)
            | ((self.hard_buf.get(5).copied().unwrap_or(0) as u64) << 16)
            | ((self.hard_buf.get(6).copied().unwrap_or(0) as u64) << 8)
            | ((self.hard_buf.get(7).copied().unwrap_or(0) as u64) << 0);
        let mut correlation = 0i32;
        let mut offset = 0usize;
        let mut phase_out = Phase::Phase0;
        let mut swap_out = false;
        match self.modulation {
            Constellation::Bpsk => {
                let mut pos = 8usize;
                for p in 0..2 {
                    let corr = corr_64(self.syncwords[p], current);
                    if corr > 45 {
                        return (0, if p == 1 { Phase::Phase180 } else { Phase::Phase0 }, false, corr);
                    }
                }
                for i in 0..length.saturating_sub(8) {
                    for ii in 0..8 {
                        for p in 0..2 {
                            let corr = corr_64(self.syncwords[p], current);
                            if corr > correlation {
                                correlation = corr;
                                offset = i * 8 + ii;
                                phase_out = if p == 1 { Phase::Phase180 } else { Phase::Phase0 };
                                swap_out = false;
                            }
                        }
                        let bit = if pos < self.hard_buf.len() {
                            ((self.hard_buf[pos] >> (7 - ii)) & 0b1) as u64
                        } else {
                            0
                        };
                        current = (current << 1) | bit;
                    }
                    pos += 1;
                }
            }
            Constellation::Qpsk => {
                let mut pos = 8usize;
                for p in 0..8 {
                    let corr = corr_64(self.syncwords[p], current);
                    if corr > 45 {
                        let phase = match p % 4 {
                            0 => Phase::Phase0,
                            1 => Phase::Phase90,
                            2 => Phase::Phase180,
                            3 => Phase::Phase270,
                            _ => Phase::Phase0,
                        };
                        return (0, phase, (p / 4) == 0, corr);
                    }
                }
                for i in 0..(length / 8).saturating_sub(8) {
                    for ii in 0..4 {
                        let step = ii * 2;
                        for p in 0..8 {
                            let corr = corr_64(self.syncwords[p], current);
                            if corr > correlation {
                                correlation = corr;
                                offset = i * 8 + step;
                                phase_out = match p % 4 {
                                    0 => Phase::Phase0,
                                    1 => Phase::Phase90,
                                    2 => Phase::Phase180,
                                    3 => Phase::Phase270,
                                    _ => Phase::Phase0,
                                };
                                swap_out = (p / 4) == 0;
                            }
                        }
                        let bits = if pos < self.hard_buf.len() {
                            ((self.hard_buf[pos] >> (6 - step)) & 0b11) as u64
                        } else {
                            0
                        };
                        current = (current << 2) | bits;
                    }
                    pos += 1;
                }
            }
            _ => {}
        }
        (offset, phase_out, swap_out, correlation)
    }
}

pub fn manchester_decode(part_one: u8, part_two: u8) -> u8 {
    let mut data: u8 = 0x00;
    let mut temp: u8 = 0x00;
    let mut bit_count: u8 = 0;
    let mut bit_one_count: u8 = 0;
    let mut bit_two_count: u8 = 0;
    let mut bit_by_one_count: u8 = 1;
    let mut bit_by_two_count: u8 = 1;
    let mut first_half_count: u8 = 0;
    while bit_count != 8 {
        if first_half_count <= 6 {
            temp |= (part_one >> (bit_one_count + bit_by_one_count)) & 0x01;
            data |= temp << bit_count;
            bit_count += 1;
            bit_one_count += 1;
            bit_by_one_count += 1;
        } else {
            temp |= (part_two >> (bit_two_count + bit_by_two_count)) & 0x01;
            data |= temp << bit_count;
            bit_count += 1;
            bit_two_count += 1;
            bit_by_two_count += 1;
        }
        first_half_count += 2;
        temp = 0x00;
    }
    data
}

pub fn manchester_decoder(in_: &[u8], out: &mut [u8]) -> usize {
    let length = in_.len();
    let mut produced = 0usize;
    for i in (0..length).step_by(2) {
        if i + 1 < length && produced < out.len() {
            out[produced] = manchester_decode(in_[i + 1], in_[i]);
            produced += 1;
        }
    }
    produced
}

static CCSDS_PN: [u8; 255] = [
    0xff, 0x48, 0x0e, 0xc0, 0x9a, 0x0d, 0x70, 0xbc,
    0x8e, 0x2c, 0x93, 0xad, 0xa7, 0xb7, 0x46, 0xce,
    0x5a, 0x97, 0x7d, 0xcc, 0x32, 0xa2, 0xbf, 0x3e,
    0x0a, 0x10, 0xf1, 0x88, 0x94, 0xcd, 0xea, 0xb1,
    0xfe, 0x90, 0x1d, 0x81, 0x34, 0x1a, 0xe1, 0x79,
    0x1c, 0x59, 0x27, 0x5b, 0x4f, 0x6e, 0x8d, 0x9c,
    0xb5, 0x2e, 0xfb, 0x98, 0x65, 0x45, 0x7e, 0x7c,
    0x14, 0x21, 0xe3, 0x11, 0x29, 0x9b, 0xd5, 0x63,
    0xfd, 0x20, 0x3b, 0x02, 0x68, 0x35, 0xc2, 0xf2,
    0x38, 0xb2, 0x4e, 0xb6, 0x9e, 0xdd, 0x1b, 0x39,
    0x6a, 0x5d, 0xf7, 0x30, 0xca, 0x8a, 0xfc, 0xf8,
    0x28, 0x43, 0xc6, 0x22, 0x53, 0x37, 0xaa, 0xc7,
    0xfa, 0x40, 0x76, 0x04, 0xd0, 0x6b, 0x85, 0xe4,
    0x71, 0x64, 0x9d, 0x6d, 0x3d, 0xba, 0x36, 0x72,
    0xd4, 0xbb, 0xee, 0x61, 0x95, 0x15, 0xf9, 0xf0,
    0x50, 0x87, 0x8c, 0x44, 0xa6, 0x6f, 0x55, 0x8f,
    0xf4, 0x80, 0xec, 0x09, 0xa0, 0xd7, 0x0b, 0xc8,
    0xe2, 0xc9, 0x3a, 0xda, 0x7b, 0x74, 0x6c, 0xe5,
    0xa9, 0x77, 0xdc, 0xc3, 0x2a, 0x2b, 0xf3, 0xe0,
    0xa1, 0x0f, 0x18, 0x89, 0x4c, 0xde, 0xab, 0x1f,
    0xe9, 0x01, 0xd8, 0x13, 0x41, 0xae, 0x17, 0x91,
    0xc5, 0x92, 0x75, 0xb4, 0xf6, 0xe8, 0xd9, 0xcb,
    0x52, 0xef, 0xb9, 0x86, 0x54, 0x57, 0xe7, 0xc1,
    0x42, 0x1e, 0x31, 0x12, 0x99, 0xbd, 0x56, 0x3f,
    0xd2, 0x03, 0xb0, 0x26, 0x83, 0x5c, 0x2f, 0x23,
    0x8b, 0x24, 0xeb, 0x69, 0xed, 0xd1, 0xb3, 0x96,
    0xa5, 0xdf, 0x73, 0x0c, 0xa8, 0xaf, 0xcf, 0x82,
    0x84, 0x3c, 0x62, 0x25, 0x33, 0x7a, 0xac, 0x7f,
    0xa4, 0x07, 0x60, 0x4d, 0x06, 0xb8, 0x5e, 0x47,
    0x16, 0x49, 0xd6, 0xd3, 0xdb, 0xa3, 0x67, 0x2d,
    0x4b, 0xbe, 0xe6, 0x19, 0x51, 0x5f, 0x9f, 0x05,
    0x08, 0x78, 0xc4, 0x4a, 0x66, 0xf5, 0x58,
];

static CCSDS_SOFT_PN: [bool; 255] = [
    true, true, true, true, true, true, true, true,
    false, true, false, false, true, false, false, false,
    false, false, false, false, true, true, true, false,
    true, true, false, false, false, false, false, false,
    true, false, false, true, true, false, true, false,
    false, false, false, false, true, true, false, true,
    false, true, true, true, false, false, false, false,
    true, false, true, true, true, true, false, false,
    true, false, false, false, true, true, true, false,
    false, false, true, false, true, true, false, false,
    true, false, false, true, false, false, true, true,
    true, false, true, false, true, true, false, true,
    true, false, true, false, false, true, true, true,
    true, false, true, true, false, true, true, true,
    false, true, false, false, false, true, true, false,
    true, true, false, false, true, true, true, false,
    false, true, false, true, true, false, true, false,
    true, false, false, true, false, true, true, true,
    false, true, true, true, true, true, false, true,
    true, true, false, false, true, true, false, false,
    false, false, true, true, false, false, true, false,
    true, false, true, false, false, false, true, false,
    true, false, true, true, true, true, true, true,
    false, false, true, true, true, true, true, false,
    false, false, false, false, true, false, true, false,
    false, false, false, true, false, false, false, false,
    true, true, true, true, false, false, false, true,
    true, false, false, false, true, false, false, false,
    true, false, false, true, false, true, false, false,
    true, true, false, false, true, true, false, true,
    true, true, false, true, false, true, false, true,
    true, false, true, true, false, false, false,
];

pub fn derand_ccsds(data: &mut [u8]) {
    for (i, b) in data.iter_mut().enumerate() {
        *b ^= CCSDS_PN[i % 255];
    }
}

pub fn derand_ccsds_soft(data: &mut [i8]) {
    for (i, b) in data.iter_mut().enumerate() {
        if CCSDS_SOFT_PN[i % 255] {
            *b = -*b;
        }
    }
}

pub fn derand_ccsds_bits(data: &mut [u8]) {
    for (i, b) in data.iter_mut().enumerate() {
        if CCSDS_SOFT_PN[i % 255] {
            *b = if *b != 0 { 0 } else { 1 };
        }
    }
}
