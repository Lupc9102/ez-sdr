//! Modulation/demodulation - translated from src-core/dsp/digital/

pub fn binary_slicer(input: &[f32], output: &mut [i8]) -> usize {
    let n = input.len().min(output.len());
    for i in 0..n {
        output[i] = if input[i] >= 0.0 { 1 } else { 0 };
    }
    n
}

pub fn bit_to_float(input: &[i8], output: &mut [f32]) -> usize {
    let n = input.len().min(output.len());
    for i in 0..n {
        output[i] = if input[i] != 0 { 1.0 } else { -1.0 };
    }
    n
}

pub struct BitsRepack {
    shifter: u8,
    in_shifter: i32,
}

impl BitsRepack {
    pub fn new() -> Self {
        Self {
            shifter: 0,
            in_shifter: 0,
        }
    }

    pub fn reset(&mut self) {
        self.shifter = 0;
        self.in_shifter = 0;
    }

    pub fn process(&mut self, input: &[i8], output: &mut [i8]) -> usize {
        if input.is_empty() {
            return 0;
        }
        let mut no = 0usize;
        for &s in input {
            self.shifter = (self.shifter << 1) | ((s as u8) & 0x01);
            self.in_shifter += 1;
            if self.in_shifter >= 8 {
                if no < output.len() {
                    output[no] = self.shifter as i8;
                    no += 1;
                }
                self.in_shifter = 0;
            }
        }
        no
    }
}

pub struct UnpackBits {
    shifter: u8,
    in_shifter: i32,
}

impl UnpackBits {
    pub fn new() -> Self {
        Self {
            shifter: 0,
            in_shifter: 0,
        }
    }

    pub fn reset(&mut self) {
        self.shifter = 0;
        self.in_shifter = 0;
    }

    pub fn process(&mut self, input: &[i8], output: &mut [i8]) -> usize {
        let n = input.len();
        let out_needed = n * 8;
        let out_avail = output.len();
        let limit = out_needed.min(out_avail);
        let mut produced = 0usize;
        for &s in input {
            self.shifter = s as u8;
            for y in (0..8).rev() {
                if produced >= limit {
                    break;
                }
                output[produced] = ((self.shifter >> y) & 1) as i8;
                produced += 1;
            }
        }
        produced
    }
}

pub struct DifferentialDecoder {
    last: u8,
    mode: DiffMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffMode {
    Nrzm,
    Nrzs,
}

impl DifferentialDecoder {
    pub fn new(mode: DiffMode) -> Self {
        Self {
            last: 0,
            mode,
        }
    }

    pub fn reset(&mut self) {
        self.last = 0;
    }

    pub fn process(&mut self, data: &mut [u8]) {
        match self.mode {
            DiffMode::Nrzm => {
                for b in data.iter_mut() {
                    let current = *b & 0x01;
                    *b = current ^ self.last;
                    self.last = current;
                }
            }
            DiffMode::Nrzs => {
                for b in data.iter_mut() {
                    let current = *b & 0x01;
                    *b = (!(current ^ self.last)) & 0x01;
                    self.last = current;
                }
            }
        }
    }

    pub fn process_soft(&mut self, data: &mut [i8]) {
        match self.mode {
            DiffMode::Nrzm => {
                for b in data.iter_mut() {
                    let current = if *b > 0 { 1u8 } else { 0u8 };
                    let out = current ^ self.last;
                    *b = if out != 0 { 127 } else { -127 };
                    self.last = current;
                }
            }
            DiffMode::Nrzs => {
                for b in data.iter_mut() {
                    let current = if *b > 0 { 1u8 } else { 0u8 };
                    let out = (!(current ^ self.last)) & 0x01;
                    *b = if out != 0 { 127 } else { -127 };
                    self.last = current;
                }
            }
        }
    }
}

pub struct CaduDeframer {
    syncword: u32,
    frame_len: usize,
    state: DeframerState,
    shift_reg: u32,
    bit_count: usize,
    out_buf: Vec<u8>,
    sync_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeframerState {
    Searching,
    Synced,
}

impl CaduDeframer {
    pub fn new(syncword: u32, frame_len: usize) -> Self {
        Self {
            syncword,
            frame_len,
            state: DeframerState::Searching,
            shift_reg: 0,
            bit_count: 0,
            out_buf: vec![0u8; frame_len],
            sync_count: 0,
        }
    }

    pub fn reset(&mut self) {
        self.state = DeframerState::Searching;
        self.shift_reg = 0;
        self.bit_count = 0;
        self.sync_count = 0;
    }

    pub fn process(&mut self, input: &[u8], output: &mut [Vec<u8>]) -> usize {
        let mut produced = 0usize;
        for &bit in input {
            let b = bit & 0x01;
            self.shift_reg = ((self.shift_reg << 1) | b as u32) & 0xFFFFFFFF;
            match self.state {
                DeframerState::Searching => {
                    if self.shift_reg == self.syncword {
                        self.state = DeframerState::Synced;
                        self.bit_count = 0;
                        self.sync_count = 1;
                    }
                }
                DeframerState::Synced => {
                    if self.bit_count < self.frame_len * 8 {
                        let byte_idx = self.bit_count / 8;
                        let bit_idx = 7 - (self.bit_count % 8);
                        if byte_idx < self.out_buf.len() {
                            if b != 0 {
                                self.out_buf[byte_idx] |= 1 << bit_idx;
                            } else {
                                self.out_buf[byte_idx] &= !(1 << bit_idx);
                            }
                        }
                        self.bit_count += 1;
                    } else {
                        if self.shift_reg == self.syncword {
                            self.sync_count += 1;
                            if produced < output.len() {
                                output[produced] = self.out_buf.clone();
                                produced += 1;
                            }
                            self.bit_count = 0;
                            self.out_buf.fill(0);
                        } else {
                            self.sync_count = self.sync_count.saturating_sub(1);
                            if self.sync_count == 0 {
                                self.state = DeframerState::Searching;
                            }
                        }
                    }
                }
            }
        }
        produced
    }
}
