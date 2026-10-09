//! Clean-room Meteor-M2 LRPT (Low Rate Picture Transmission) decoder.
//!
//! Pipeline: IQ -> profile-specific QPSK/OQPSK -> optional 80k marker removal/
//! convolutional deinterleaving -> Viterbi -> optional NRZ-M -> CADU sync ->
//! derandomize -> Reed-Solomon(255,223) correction -> VCDU/M-PDU
//! Space Packet reassembly -> per-APID scanline image reconstruction.
//!
//! File helpers with only a symbol-rate argument retain the uninterleaved
//! experimental coding primitive. Mission receivers select `for_mission`.
//! Implemented from public CCSDS specifications and Meteor transport parameters.
//! Scope: Meteor-M2-3 and Meteor-M2-4 LRPT downlinks. No APT/NOAA support.

mod ccsds;
#[cfg(test)]
mod diff_decode;
mod frame_sync;
mod image_builder;
mod interleaver;
mod iq_source;
mod qpsk;
mod randomizer;
mod reed_solomon;
mod viterbi;

use crossbeam_channel::Sender;
use num_complex::Complex32;
use std::path::Path;

pub use ez_proto::MeteorProfile;
pub use image_builder::apid_channel_label;
pub use iq_source::{
    cf32_amplitude_for_path, cf32_le_bytes_to_complex, cf32_le_bytes_to_normalized,
    cf32_le_bytes_to_uc8, cs8_bytes_to_complex, iq_bytes_to_complex, normalized_component_to_uc8,
    uc8_bytes_to_cf32_le, uc8_bytes_to_normalized, Cf32Amplitude,
};

#[derive(Debug, thiserror::Error)]
pub enum LrptError {
    #[error("failed to read IQ file: {0}")]
    Io(#[from] std::io::Error),
    #[error("no CADU frame sync ever achieved on this recording")]
    NoSync,
    #[error("invalid CF32 metadata: {0}")]
    InvalidMetadata(String),
    #[error("invalid Meteor configuration: {0}")]
    InvalidConfiguration(String),
}

/// Supported mission receiver envelope. Four input samples per symbol keep
/// staggered-branch timing and sample interpolation inside the independently
/// verified carrier operating range; the mission RRC occupies 1.5 * Rs.
pub fn validate_mission_sampling(
    sample_rate: u32,
    profile: MeteorProfile,
) -> Result<(), LrptError> {
    let minimum = profile.symbol_rate_hz() * 4;
    if sample_rate < minimum {
        return Err(LrptError::InvalidConfiguration(format!(
            "{} requires at least {minimum} samples/s (four samples per symbol); got {sample_rate}",
            profile.label()
        )));
    }
    Ok(())
}

/// Snapshot of decode progress, sent periodically over `progress_tx` so the
/// GUI can render a live meter/preview without blocking on the full decode.
#[derive(Debug, Clone, Default)]
pub struct DecodeProgress {
    /// Complete scanlines, requiring all fourteen horizontal image segments.
    pub lines_decoded: u32,
    /// Unique decoded image segments, including partially received scanlines.
    pub segments_decoded: u32,
    pub rs_ok: u32,
    pub rs_failed: u32,
    pub costas_locked: bool,
    /// Whether the CADU frame synchronizer currently has a locked sync
    /// marker (distinct from `costas_locked`, which reflects the QPSK
    /// carrier loop rather than frame-level lock).
    pub frame_locked: bool,
    /// Segments refused by the image builder (legacy field name). Rising on a
    /// live pass means invalid/oversize input, not image growth.
    pub dropped_scanlines: u64,
    /// Downsampled/latest preview of the image(s) being built, keyed by APID.
    pub preview: Vec<(u16, image::GrayImage)>,
}

/// Final output of a completed decode: one grayscale image per image APID
/// (MSU-MR channel).
#[derive(Debug, Clone, Default)]
pub struct DecodeResult {
    pub images: Vec<(u16, image::GrayImage)>,
    pub rs_ok: u32,
    pub rs_failed: u32,
}

/// Decode an entire `.iq` recording file (used for offline import).
///
/// The file is streamed in 1 MiB chunks rather than read whole: a 1 GiB
/// recording would otherwise need ~8 GiB of transient allocations (file
/// bytes + Complex32 expansion + symbol vectors).
///
/// # Errors
/// Returns `LrptError::Io` if the file cannot be read, or `LrptError::NoSync`
/// if CADU frame sync is never achieved anywhere in the recording.
pub fn decode_file(
    path: &Path,
    sample_rate: u32,
    symbol_rate: u32,
    progress_tx: Sender<DecodeProgress>,
) -> Result<DecodeResult, LrptError> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let mut reader = std::io::BufReader::with_capacity(1 << 20, file);
    let mut decoder = LrptDecoder::new(sample_rate, symbol_rate, progress_tx);
    let mut chunk = vec![0u8; 1 << 20];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => decoder.push_samples(&chunk[..n]),
            Err(e) => return Err(LrptError::Io(e)),
        }
    }
    decoder.finish().ok_or(LrptError::NoSync)
}

/// Decode a streamed little-endian `Complex32` recording. The decoder retains at most one
/// trailing partial sample between reads, so arbitrarily-sized buffered reads cannot break
/// the 8-byte I/Q sample alignment used by `.cf32` recordings.
pub fn decode_cf32_file(
    path: &Path,
    sample_rate: u32,
    symbol_rate: u32,
    progress_tx: Sender<DecodeProgress>,
) -> Result<DecodeResult, LrptError> {
    decode_cf32_file_with_amplitude(path, sample_rate, symbol_rate, progress_tx, None)
}

pub fn decode_cf32_file_with_amplitude(
    path: &Path,
    sample_rate: u32,
    symbol_rate: u32,
    progress_tx: Sender<DecodeProgress>,
    explicit: Option<Cf32Amplitude>,
) -> Result<DecodeResult, LrptError> {
    let amplitude = cf32_amplitude_for_path(path, explicit).map_err(LrptError::InvalidMetadata)?;
    let file = std::fs::File::open(path)?;
    let mut reader = std::io::BufReader::with_capacity(1 << 20, file);
    decode_cf32_reader_with_amplitude(
        &mut reader,
        sample_rate,
        symbol_rate,
        progress_tx,
        amplitude,
    )
}

/// Decode a Meteor LRPT recording stored as interleaved signed 8-bit I/Q (`.cs8`).
pub fn decode_cs8_file(
    path: &Path,
    sample_rate: u32,
    symbol_rate: u32,
    progress_tx: Sender<DecodeProgress>,
) -> Result<DecodeResult, LrptError> {
    let file = std::fs::File::open(path)?;
    let mut reader = std::io::BufReader::with_capacity(1 << 20, file);
    decode_cs8_reader(&mut reader, sample_rate, symbol_rate, progress_tx)
}

fn decode_cs8_reader(
    reader: &mut impl std::io::Read,
    sample_rate: u32,
    symbol_rate: u32,
    progress_tx: Sender<DecodeProgress>,
) -> Result<DecodeResult, LrptError> {
    let mut decoder = LrptDecoder::new(sample_rate, symbol_rate, progress_tx);
    let mut chunk = vec![0u8; 1 << 20];
    let mut carry = Vec::with_capacity(1);
    loop {
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        if carry.is_empty() {
            let aligned_len = n / 2 * 2;
            decoder.push_samples_cs8(&chunk[..aligned_len]);
            carry.extend_from_slice(&chunk[aligned_len..n]);
        } else {
            decoder.push_samples_cs8(&[carry[0], chunk[0]]);
            carry.clear();
            let remaining = &chunk[1..n];
            let aligned_len = remaining.len() / 2 * 2;
            decoder.push_samples_cs8(&remaining[..aligned_len]);
            carry.extend_from_slice(&remaining[aligned_len..]);
        }
    }
    decoder.finish().ok_or(LrptError::NoSync)
}

#[cfg(test)]
fn decode_cf32_reader(
    reader: &mut impl std::io::Read,
    sample_rate: u32,
    symbol_rate: u32,
    progress_tx: Sender<DecodeProgress>,
) -> Result<DecodeResult, LrptError> {
    decode_cf32_reader_with_amplitude(
        reader,
        sample_rate,
        symbol_rate,
        progress_tx,
        Cf32Amplitude::Normalized,
    )
}

fn decode_cf32_reader_with_amplitude(
    reader: &mut impl std::io::Read,
    sample_rate: u32,
    symbol_rate: u32,
    progress_tx: Sender<DecodeProgress>,
    amplitude: Cf32Amplitude,
) -> Result<DecodeResult, LrptError> {
    let mut decoder = LrptDecoder::new(sample_rate, symbol_rate, progress_tx);
    let mut chunk = vec![0u8; 1 << 20];
    let mut carry = Vec::with_capacity(7);

    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if carry.is_empty() {
                    let aligned_len = n / 8 * 8;
                    decoder.push_samples_cf32_with_amplitude(&chunk[..aligned_len], amplitude);
                    carry.extend_from_slice(&chunk[aligned_len..n]);
                    continue;
                }

                let mut prefix = [0u8; 8];
                let needed = 8 - carry.len();
                if n < needed {
                    carry.extend_from_slice(&chunk[..n]);
                    continue;
                }
                prefix[..carry.len()].copy_from_slice(&carry);
                prefix[carry.len()..].copy_from_slice(&chunk[..needed]);
                decoder.push_samples_cf32_with_amplitude(&prefix, amplitude);
                carry.clear();

                let remaining = &chunk[needed..n];
                let aligned_len = remaining.len() / 8 * 8;
                if aligned_len > 0 {
                    decoder.push_samples_cf32_with_amplitude(&remaining[..aligned_len], amplitude);
                }
                carry.extend_from_slice(&remaining[aligned_len..]);
            }
            Err(e) => return Err(LrptError::Io(e)),
        }
    }

    decoder.finish().ok_or(LrptError::NoSync)
}

// Two missing CADUs are sufficient to withdraw current frame lock. This is
// measured in recovered symbols, so silence and different caller chunk sizes
// cannot indefinitely preserve a previously selected hypothesis.
const FRAME_LOCK_MAX_AGE: u64 = 2 * frame_sync::CADU_LEN_BYTES as u64 * 8;
const VITERBI_OUTPUT_DELAY: u64 = 63;

/// One phase/axis hypothesis for the QPSK-to-convolutional-code path.
/// Independent I/Q inversion and axis exchange cover eight phase/conjugation
/// ambiguities for each of the two OQPSK branch stagger directions. NRZ-M
/// decoding remains after the Viterbi decoder.
struct DecodeLane {
    viterbi: viterbi::ViterbiDecoder,
    plain_framer: frame_sync::FrameSync,
    nrzm_framer: frame_sync::FrameSync,
    nrzm_previous: Option<u8>,
    profile: Option<MeteorProfile>,
    interleaver: Option<interleaver::MeteorDeinterleaver>,
    transport_generation: u64,
    transport_bits_received: u64,
    transport_erasures: std::collections::VecDeque<u64>,
}

impl DecodeLane {
    fn new() -> Self {
        Self {
            viterbi: viterbi::ViterbiDecoder::new(),
            plain_framer: frame_sync::FrameSync::new(),
            nrzm_framer: frame_sync::FrameSync::new(),
            nrzm_previous: None,
            profile: None,
            interleaver: None,
            transport_generation: 0,
            transport_bits_received: 0,
            transport_erasures: std::collections::VecDeque::new(),
        }
    }

    fn for_profile(profile: Option<MeteorProfile>) -> Self {
        let mut lane = Self::new();
        lane.profile = profile;
        lane.interleaver = profile
            .filter(|profile| profile.interleaved())
            .map(|_| interleaver::MeteorDeinterleaver::new());
        lane
    }

    fn push_symbols(
        &mut self,
        symbols: &[qpsk::RecoveredSymbol],
        hypothesis: usize,
    ) -> Vec<frame_sync::LocatedFrame> {
        let mut coded = Vec::with_capacity(symbols.len() * 2);
        for symbol in symbols {
            let value = if hypothesis & 8 != 0 {
                symbol.alternate_value.unwrap_or(symbol.value)
            } else {
                symbol.value
            };
            let mut first = u8::from(value.re < 0.0);
            let mut second = u8::from(value.im < 0.0);
            if hypothesis & 0b100 != 0 {
                std::mem::swap(&mut first, &mut second);
            }
            first ^= (hypothesis & 0b001 != 0) as u8;
            second ^= (hypothesis & 0b010 != 0) as u8;
            coded.extend_from_slice(&[first, second]);
        }
        if let Some(stage) = &mut self.interleaver {
            let bits: Vec<_> = coded
                .iter()
                .enumerate()
                .map(|(index, &value)| {
                    let symbol = symbols[index / 2];
                    let observed = if hypothesis & 8 != 0 {
                        symbol.alternate_value.unwrap_or(symbol.value)
                    } else {
                        symbol.value
                    };
                    interleaver::ChannelBit {
                        value,
                        valid: observed.norm_sqr().is_finite() && observed.norm_sqr() > 1e-12,
                    }
                })
                .collect();
            let blocks = stage.push_bits(&bits);
            return self.push_transport_blocks(blocks);
        }
        self.push_coded_bits(&coded)
    }

    fn push_coded_bits(&mut self, coded: &[u8]) -> Vec<frame_sync::LocatedFrame> {
        let recovered = self.viterbi.push_bits(coded);
        let validate = |cadu: &[u8]| {
            let transport = randomizer::derandomize(&cadu[4..]);
            let result = reed_solomon::decode_interleaved(&transport);
            result.codewords_failed == 0 && result.codewords_ok == 4
        };
        let mut frames = if self.profile.is_none_or(|profile| !profile.nrzm()) {
            self.plain_framer.push_bits_located(&recovered, validate)
        } else {
            Vec::new()
        };
        if self.profile.is_some_and(|profile| !profile.nrzm()) {
            return frames;
        }

        let mut nrzm = Vec::with_capacity(recovered.len());
        for bit in recovered {
            if let Some(previous) = self.nrzm_previous {
                nrzm.push(previous ^ bit);
            }
            self.nrzm_previous = Some(bit);
        }
        let mut nrzm_frames = self.nrzm_framer.push_bits_located(&nrzm, validate);
        // NRZ-M drops the first information bit while establishing its initial
        // level. Align both framers to the same recovered-information clock.
        for frame in &mut nrzm_frames {
            frame.end_bit += 1;
        }
        frames.extend(nrzm_frames);
        frames
    }

    fn push_transport_blocks(
        &mut self,
        blocks: Vec<interleaver::TransportBlock>,
    ) -> Vec<frame_sync::LocatedFrame> {
        let mut frames = Vec::new();
        for block in blocks {
            if block.generation != self.transport_generation {
                self.transport_generation = block.generation;
                self.viterbi = viterbi::ViterbiDecoder::new();
                self.plain_framer = frame_sync::FrameSync::new();
                self.nrzm_framer = frame_sync::FrameSync::new();
                self.nrzm_previous = None;
                self.transport_bits_received = 0;
                self.transport_erasures.clear();
            }
            for (index, bit) in block.bits.iter().enumerate() {
                if !bit.valid {
                    self.transport_erasures
                        .push_back((self.transport_bits_received + index as u64) / 2);
                }
            }
            self.transport_bits_received += block.bits.len() as u64;
            let bits = block.bits.map(|bit| bit.value);
            let recovered = self.push_coded_bits(&bits);
            for mut frame in recovered {
                let start = frame
                    .end_bit
                    .saturating_sub(frame_sync::CADU_LEN_BYTES as u64 * 8);
                if self
                    .transport_erasures
                    .iter()
                    .any(|&index| index >= start && index < frame.end_bit)
                {
                    continue;
                }
                frame.end_bit = interleaver::MeteorDeinterleaver::observed_symbol_end(
                    block.first_wire_bit,
                    frame.end_bit + VITERBI_OUTPUT_DELAY,
                );
                frames.push(frame);
            }
            let oldest = (self.transport_bits_received / 2)
                .saturating_sub(VITERBI_OUTPUT_DELAY + frame_sync::CADU_LEN_BYTES as u64 * 8);
            while self
                .transport_erasures
                .front()
                .is_some_and(|&index| index < oldest)
            {
                self.transport_erasures.pop_front();
            }
        }
        frames
    }
}

/// Streaming decoder driven incrementally with IQ byte chunks, for live
/// decode fed from the same sample loop that feeds the recorder/spectrum.
pub struct LrptDecoder {
    progress_tx: Sender<DecodeProgress>,

    demod: qpsk::QpskDemod,
    profile: Option<MeteorProfile>,
    lanes: [DecodeLane; 16],
    selected_lane: Option<usize>,
    symbols_received: u64,
    last_valid_frame_end: Option<u64>,
    low_energy_symbols: [std::collections::VecDeque<u64>; 2],
    reassembler: ccsds::PacketReassembler,
    images: image_builder::ImageBuilder,

    ever_locked: bool,
    rs_ok: u32,
    rs_failed: u32,
    progress_interval_symbols: u64,
    symbols_at_progress: u64,
    last_progress_lock: Option<(bool, bool)>,
    #[cfg(test)]
    validated_payloads: Vec<Vec<u8>>,
}

impl LrptDecoder {
    #[must_use]
    pub fn new(sample_rate: u32, symbol_rate: u32, progress_tx: Sender<DecodeProgress>) -> Self {
        assert!(sample_rate > 0, "sample_rate must be greater than 0");
        assert!(symbol_rate > 0, "symbol_rate must be greater than 0");
        let config = qpsk::QpskConfig {
            sample_rate: sample_rate as f32,
            symbol_rate: symbol_rate as f32,
            oqpsk: true,
            ..qpsk::QpskConfig::default()
        };
        Self {
            progress_tx,
            demod: qpsk::QpskDemod::new(config),
            profile: None,
            lanes: std::array::from_fn(|_| DecodeLane::new()),
            selected_lane: None,
            symbols_received: 0,
            last_valid_frame_end: None,
            low_energy_symbols: std::array::from_fn(|_| std::collections::VecDeque::new()),
            reassembler: ccsds::PacketReassembler::new(),
            images: image_builder::ImageBuilder::new(),
            ever_locked: false,
            rs_ok: 0,
            rs_failed: 0,
            progress_interval_symbols: u64::from(symbol_rate).div_ceil(4),
            symbols_at_progress: 0,
            last_progress_lock: None,
            #[cfg(test)]
            validated_payloads: Vec::new(),
        }
    }

    /// Construct a real mission transport. `new` remains an uninterleaved
    /// coding primitive for experiments; an arbitrary symbol rate is not a
    /// substitute for selecting the actual mission's interleaver/NRZ-M chain.
    pub fn for_mission(
        sample_rate: u32,
        profile: MeteorProfile,
        progress_tx: Sender<DecodeProgress>,
    ) -> Result<Self, LrptError> {
        validate_mission_sampling(sample_rate, profile)?;
        let mut decoder = Self::new(sample_rate, profile.symbol_rate_hz(), progress_tx);
        decoder.profile = Some(profile);
        decoder.reset_stream(sample_rate, profile.symbol_rate_hz());
        Ok(decoder)
    }

    /// A capture gap resets partial transport state but retains this product.
    pub fn reset_mission_stream(&mut self, sample_rate: u32) -> Result<(), LrptError> {
        let profile = self.profile.ok_or_else(|| {
            LrptError::InvalidConfiguration("mission reset requires a mission profile".into())
        })?;
        validate_mission_sampling(sample_rate, profile)?;
        self.reset_stream(sample_rate, profile.symbol_rate_hz());
        Ok(())
    }

    /// A different pass/source/tuning begins a fresh product and fresh counts.
    pub fn start_new_product(
        &mut self,
        sample_rate: u32,
        profile: MeteorProfile,
    ) -> Result<(), LrptError> {
        *self = Self::for_mission(sample_rate, profile, self.progress_tx.clone())?;
        Ok(())
    }

    /// Discard partial RF, convolutional, CADU and packet state after missing
    /// samples. Completed image data and lifetime RS counts remain available.
    pub fn reset_stream(&mut self, sample_rate: u32, symbol_rate: u32) {
        assert!(sample_rate > 0 && symbol_rate > 0);
        assert!(self
            .profile
            .is_none_or(|profile| profile.symbol_rate_hz() == symbol_rate));
        self.demod = qpsk::QpskDemod::new(qpsk::QpskConfig {
            sample_rate: sample_rate as f32,
            symbol_rate: symbol_rate as f32,
            oqpsk: self.profile.is_none_or(MeteorProfile::oqpsk),
            ..qpsk::QpskConfig::default()
        });
        self.lanes = std::array::from_fn(|_| DecodeLane::for_profile(self.profile));
        self.selected_lane = None;
        self.symbols_received = 0;
        self.last_valid_frame_end = None;
        for observations in &mut self.low_energy_symbols {
            observations.clear();
        }
        self.reassembler = ccsds::PacketReassembler::new();
        self.progress_interval_symbols = u64::from(symbol_rate).div_ceil(4);
        self.symbols_at_progress = 0;
        self.last_progress_lock = None;
    }

    /// Feed a chunk of raw interleaved u8 IQ bytes (RTL-SDR `Uc8` convention).
    pub fn push_samples(&mut self, bytes: &[u8]) {
        let complex = iq_bytes_to_complex(bytes);
        self.push_complex(&complex);
    }

    /// Feed signed interleaved 8-bit IQ samples (`.cs8`).
    pub fn push_samples_cs8(&mut self, bytes: &[u8]) {
        let complex = cs8_bytes_to_complex(bytes);
        self.push_complex(&complex);
    }

    /// Feed a chunk of LE-float32-interleaved I/Q bytes (current `.cf32`
    /// recorder format — see
    /// `ez-gui/src/satellite/recorder.rs::raw_iq_bytes_to_cf32_le`).
    pub fn push_samples_cf32(&mut self, bytes: &[u8]) {
        self.push_samples_cf32_with_amplitude(bytes, Cf32Amplitude::Normalized);
    }

    pub fn push_samples_cf32_with_amplitude(&mut self, bytes: &[u8], amplitude: Cf32Amplitude) {
        let complex = cf32_le_bytes_to_normalized(bytes, amplitude);
        self.push_complex(&complex);
    }

    /// Feed already-parsed complex baseband samples directly. Intended for callers that
    /// already hold `Complex32` samples (e.g. a daemon pipeline consuming a
    /// [`crate`]-external sample bus) and would otherwise have to re-serialize them to
    /// bytes just for this decoder to re-parse — `push_samples`/`push_samples_cf32` are
    /// thin wrappers around this for callers that only have raw bytes on hand.
    pub fn push_complex(&mut self, complex: &[Complex32]) {
        let symbols = self.demod.process(complex);
        if symbols.is_empty() {
            return;
        }

        self.push_recovered_symbols(&symbols);
        self.maybe_send_progress();
    }

    fn push_recovered_symbols(&mut self, symbols: &[qpsk::RecoveredSymbol]) {
        for chunk in symbols.chunks(1024) {
            self.push_symbol_batch(chunk);
        }
    }

    fn push_symbol_batch(&mut self, symbols: &[qpsk::RecoveredSymbol]) {
        for (offset, symbol) in symbols.iter().enumerate() {
            let pairings = [symbol.value, symbol.alternate_value.unwrap_or(symbol.value)];
            for (observations, value) in self.low_energy_symbols.iter_mut().zip(pairings) {
                if !value.norm_sqr().is_finite() || value.norm_sqr() <= 1e-12 {
                    observations.push_back(self.symbols_received + offset as u64);
                }
            }
        }
        self.symbols_received += symbols.len() as u64;
        // Keep every hypothesis current through fades and mapping changes.
        // A selected lane is a diagnostic hint, never a reason to discard
        // independently validated frames from another hypothesis.
        let mut frames = Vec::new();
        for (hypothesis, lane) in self.lanes.iter_mut().enumerate() {
            frames.extend(
                lane.push_symbols(symbols, hypothesis)
                    .into_iter()
                    .map(|frame| (hypothesis, frame)),
            );
        }
        // Merge in stream order rather than lane order, including when one
        // input call covers multiple frames and a polarity change.
        frames.sort_by_key(|(lane, frame)| (frame.end_bit, Some(*lane) != self.selected_lane));
        for (hypothesis, frame) in frames {
            let start = frame
                .end_bit
                .saturating_sub(frame_sync::CADU_LEN_BYTES as u64 * 8);
            if !self.profile.is_some_and(MeteorProfile::interleaved)
                && self.low_energy_symbols[hypothesis / 8]
                    .iter()
                    .any(|&position| position >= start && position < frame.end_bit)
            {
                // Missing-energy decisions are erasures, not evidence of a
                // received frame. In particular, constant hard decisions can
                // produce algebraically valid RS payloads during silence.
                continue;
            }
            // Equivalent convolutional hypotheses can produce the same CADU.
            // Suppress duplicate observations by position, not payload bytes:
            // successive identical transmitted frames remain distinct.
            // Marker alignment can correct a one-coded-bit offset between
            // opposite stagger/axis hypotheses. Their frame observations
            // can therefore differ by one recovered symbol. Real successive
            // CADUs are thousands of symbols apart, including identical ones.
            let position_tolerance =
                u64::from(self.profile.is_some_and(MeteorProfile::interleaved));
            if self
                .last_valid_frame_end
                .is_some_and(|end| frame.end_bit <= end.saturating_add(position_tolerance))
            {
                continue;
            }
            if self.process_cadu_frame(&frame.bytes) {
                self.last_valid_frame_end = Some(frame.end_bit);
                self.selected_lane = Some(hypothesis);
            }
        }
        let recovered_end = if self.profile.is_some_and(MeteorProfile::interleaved) {
            self.symbols_received
        } else {
            self.symbols_received.saturating_sub(VITERBI_OUTPUT_DELAY)
        };
        let oldest_candidate = recovered_end.saturating_sub(frame_sync::CADU_LEN_BYTES as u64 * 8);
        for observations in &mut self.low_energy_symbols {
            while observations
                .front()
                .is_some_and(|&position| position < oldest_candidate)
            {
                observations.pop_front();
            }
        }
        let max_frame_age = if self.profile.is_some_and(MeteorProfile::interleaved) {
            (FRAME_LOCK_MAX_AGE * 10).div_ceil(9) // 8 marker bits per 72 data bits
        } else {
            FRAME_LOCK_MAX_AGE
        };
        if self
            .last_valid_frame_end
            .is_none_or(|end| recovered_end.saturating_sub(end) >= max_frame_age)
        {
            self.selected_lane = None;
        }
    }

    /// Process one fully-extracted CADU frame: derandomize, RS-decode,
    /// hand VCDU/M-PDU payload to the packet reassembler, and feed
    /// completed image-APID packets to the image builder.
    fn process_cadu_frame(&mut self, cadu: &[u8]) -> bool {
        // First 4 bytes are the sync marker; the rest is the transport
        // frame (interleaved RS codewords, randomized).
        if cadu.len() < 4 + reed_solomon::INTERLEAVED_FRAME_LEN {
            return false;
        }
        let transport = &cadu[4..4 + reed_solomon::INTERLEAVED_FRAME_LEN];
        let derandomized = randomizer::derandomize(transport);
        let rs_result = reed_solomon::decode_interleaved(&derandomized);

        self.rs_ok += rs_result.codewords_ok;
        self.rs_failed += rs_result.codewords_failed;

        // Issue 28: If any interleaved RS codeword failed to decode, the frame contains
        // uncorrectable corruption. Because the 4 codewords are byte-interleaved across
        // the frame, even a single failure corrupts every 4th byte across the entire payload.
        // Drop the frame rather than poisoning the packet reassembler and image builder.
        if rs_result.codewords_failed > 0 || rs_result.codewords_ok == 0 {
            return false;
        }
        self.ever_locked = true;

        let payload = &rs_result.data;
        if payload.len() < ccsds::MPDU_HEADER_OFFSET + ccsds::MPDU_HEADER_LEN {
            return false;
        }

        let Some(vcdu_header) = ccsds::parse_vcdu_header(payload) else {
            return false;
        };
        let mpdu_start = ccsds::MPDU_HEADER_OFFSET;
        let Some(mpdu_header) =
            ccsds::parse_mpdu_header(&payload[mpdu_start..mpdu_start + ccsds::MPDU_HEADER_LEN])
        else {
            return false;
        };
        let mpdu_payload = &payload[mpdu_start + ccsds::MPDU_HEADER_LEN..];

        #[cfg(test)]
        self.validated_payloads.push(payload.clone());

        self.reassembler.push_mpdu_payload(
            mpdu_payload,
            mpdu_header.first_header_pointer,
            vcdu_header.virtual_channel_id,
            vcdu_header.frame_counter,
        );

        for packet in self.reassembler.drain_completed() {
            if image_builder::IMAGE_APIDS.contains(&packet.apid) {
                self.images
                    .push_packet(packet.apid, packet.header.sequence_count, &packet.data);
            }
        }
        true
    }

    fn maybe_send_progress(&mut self) {
        let lock = (self.demod.is_locked(), self.selected_lane.is_some());
        // Quarter-second observation cadence, independent of image extent and
        // input chunk count. Lock transitions bypass throttling. Publication
        // occurs at the next caller chunk boundary; a full queue never blocks.
        if self.last_progress_lock == Some(lock)
            && self
                .symbols_received
                .saturating_sub(self.symbols_at_progress)
                < self.progress_interval_symbols
        {
            return;
        }
        if self.progress_tx.is_full() || self.progress_tx.len() >= 2 {
            return;
        }

        let progress = DecodeProgress {
            lines_decoded: self.images.total_line_count(),
            segments_decoded: self.images.total_segment_count(),
            rs_ok: self.rs_ok,
            rs_failed: self.rs_failed,
            costas_locked: self.demod.is_locked(),
            frame_locked: self.selected_lane.is_some(),
            dropped_scanlines: self.images.dropped_scanlines(),
            preview: self.images.render_previews(1_024),
        };
        if self.progress_tx.try_send(progress).is_ok() {
            self.symbols_at_progress = self.symbols_received;
            self.last_progress_lock = Some(lock);
        }
    }

    /// Finalize the decode and return whatever images were reconstructed.
    /// Returns `None` if CADU sync was never achieved.
    #[must_use]
    pub fn finish(self) -> Option<DecodeResult> {
        if !self.ever_locked {
            return None;
        }
        Some(DecodeResult {
            images: self.images.render_all(),
            rs_ok: self.rs_ok,
            rs_failed: self.rs_failed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use num_complex::Complex32;

    /// Build a synthetic, noiseless QPSK IQ byte stream encoding a known
    /// CADU frame (sync marker + RS-encoded, randomized transport data),
    /// for full end-to-end pipeline validation without any real RF
    /// capture. This validates: QPSK demod -> differential decode (with
    /// the dibit mapping under test) -> frame sync -> derandomize ->
    /// RS-decode, all chained together.
    ///
    /// Returns raw interleaved u8 IQ bytes at `sample_rate`.
    fn build_synthetic_iq_stream(
        payload: &[u8],
        mapping: diff_decode::DibitMapping,
        sample_rate: f32,
        symbol_rate: f32,
    ) -> Vec<u8> {
        // 1. RS-encode the (unrandomized) payload into an interleaved
        //    transport frame, then randomize the *whole* coded frame
        //    (data + parity together), per CCSDS 131.0-B: the
        //    pseudo-randomizer scrambles the fully-coded transfer frame for
        //    the link, so RS parity is computed over the real data first
        //    and the randomizer is the outermost transmit-side step (and
        //    thus the first receive-side step, before RS decoding). Then
        //    prepend the sync marker -> full CADU bytes.
        let transport = reed_solomon::encode_interleaved(payload);
        let randomized_transport = randomizer::derandomize(&transport); // XOR is its own inverse: this "randomizes"
        let mut cadu_bytes = Vec::with_capacity(4 + randomized_transport.len());
        cadu_bytes.extend_from_slice(&frame_sync::SYNC_MARKER.to_be_bytes());
        cadu_bytes.extend_from_slice(&randomized_transport);

        // 2. Bytes -> bits (MSB first).
        let bits = diff_decode::bytes_to_bits(&cadu_bytes);

        // 3. Bits -> dibits, inverting `dibit_to_bits` for the mapping
        //    under test (brute-force: find the dibit whose expansion
        //    matches each 2-bit group).
        let dibits: Vec<u8> = bits
            .chunks_exact(2)
            .map(|pair| {
                (0..4u8)
                    .find(|&d| diff_decode::dibit_to_bits(d, mapping) == [pair[0], pair[1]])
                    .expect("mapping must be a bijection over 0..4")
            })
            .collect();

        // 4. Differentially *encode* dibits into a quadrant sequence
        //    (inverse of the receiver's differential_decode step).
        let quadrants = diff_decode::differential_encode(&dibits, 0);

        // 5. Quadrants -> ideal QPSK constellation symbols (I,Q = +-1).
        let symbol_for_quadrant = |q: u8| -> Complex32 {
            match q {
                0 => Complex32::new(1.0, 1.0),
                1 => Complex32::new(-1.0, 1.0),
                2 => Complex32::new(-1.0, -1.0),
                _ => Complex32::new(1.0, -1.0),
            }
        };

        // 6. Upsample each symbol to `sps` samples (rectangular pulse --
        //    good enough given the receiver's own RRC+lowpass filtering
        //    smooths it, and Gardner/Costas only need a clean symbol
        //    clock, not a bandlimited transmit pulse, for this synthetic
        //    round-trip check). Prepend/append a repeated-symbol
        //    preamble/postamble so the receiver's own pipeline warm-up
        //    (AGC convergence, Costas/Gardner acquisition, filter group
        //    delay, interpolator startup) has settled before the real
        //    frame begins and doesn't starve the tail end of the frame --
        //    equivalent to the natural pre/post-roll present in any real
        //    continuous downlink, just made explicit here since this is a
        //    single isolated synthetic frame rather than part of a
        //    continuous carrier.
        let sps = (sample_rate / symbol_rate).round() as usize;
        let amplitude = 40.0f32; // typical ADC-scale amplitude
        const PREAMBLE_SYMBOLS: usize = 200;
        const POSTAMBLE_SYMBOLS: usize = 40;

        let mut full_quadrants =
            Vec::with_capacity(quadrants.len() + PREAMBLE_SYMBOLS + POSTAMBLE_SYMBOLS);
        let first_q = *quadrants.first().unwrap_or(&0);
        let last_q = *quadrants.last().unwrap_or(&0);
        full_quadrants.extend(std::iter::repeat_n(first_q, PREAMBLE_SYMBOLS));
        full_quadrants.extend(quadrants);
        full_quadrants.extend(std::iter::repeat_n(last_q, POSTAMBLE_SYMBOLS));

        let mut iq_bytes = Vec::with_capacity(full_quadrants.len() * sps * 2);
        for q in full_quadrants {
            let sym = symbol_for_quadrant(q) * amplitude;
            for _ in 0..sps {
                let i_byte = (sym.re + 127.4).round().clamp(0.0, 255.0) as u8;
                let q_byte = (sym.im + 127.4).round().clamp(0.0, 255.0) as u8;
                iq_bytes.push(i_byte);
                iq_bytes.push(q_byte);
            }
        }
        iq_bytes
    }

    /// Build the actual Meteor channel-coding order used on air:
    /// CADU bits -> optional NRZ-M -> CCSDS K=7 rate-1/2 convolutional
    /// coding -> OQPSK with Q delayed by half a symbol.
    fn build_coded_oqpsk_iq_stream(
        payload: &[u8],
        hypothesis: usize,
        nrzm: bool,
        sample_rate: f32,
        symbol_rate: f32,
    ) -> Vec<u8> {
        build_coded_oqpsk_iq_frames(payload, hypothesis, nrzm, sample_rate, symbol_rate, 1)
    }

    fn build_coded_oqpsk_iq_frames(
        payload: &[u8],
        hypothesis: usize,
        nrzm: bool,
        sample_rate: f32,
        symbol_rate: f32,
        frame_count: usize,
    ) -> Vec<u8> {
        let transport = reed_solomon::encode_interleaved(payload);
        let randomized_transport = randomizer::derandomize(&transport);
        let mut cadu = frame_sync::SYNC_MARKER.to_be_bytes().to_vec();
        cadu.extend_from_slice(&randomized_transport);

        let mut information = vec![0u8; 256];
        for _ in 0..frame_count {
            information.extend(diff_decode::bytes_to_bits(&cadu));
        }
        information.extend(std::iter::repeat_n(0, 256));
        if nrzm {
            let mut level = 0u8;
            for bit in &mut information {
                if *bit != 0 {
                    level ^= 1;
                }
                *bit = level;
            }
        }

        let mut state = 0u8;
        let mut symbols = Vec::with_capacity(information.len());
        for bit in information {
            let register = ((bit << 6) | state) & 0x7f;
            let wanted = [
                ((register & 0o171).count_ones() as u8 & 1) ^ 1,
                ((register & 0o133).count_ones() as u8 & 1) ^ 1,
            ];
            state = (register >> 1) & 0x3f;

            let base = (0..4u8)
                .map(|value| [value >> 1, value & 1])
                .find(|bits| {
                    let mut first = bits[0];
                    let mut second = bits[1];
                    if hypothesis & 0b100 != 0 {
                        std::mem::swap(&mut first, &mut second);
                    }
                    first ^= (hypothesis & 0b001 != 0) as u8;
                    second ^= (hypothesis & 0b010 != 0) as u8;
                    [first, second] == wanted
                })
                .expect("phase hypothesis is bijective");
            symbols.push(Complex32::new(
                if base[0] == 0 { 1.0 } else { -1.0 },
                if base[1] == 0 { 1.0 } else { -1.0 },
            ));
        }

        let sps = (sample_rate / symbol_rate).round() as usize;
        assert!(sps >= 2 && sps.is_multiple_of(2));
        let half = sps / 2;
        let sample_count = symbols.len() * sps + half;
        let amplitude = 40.0;
        let mut iq = Vec::with_capacity(sample_count * 2);
        for sample in 0..sample_count {
            let i_index = (sample / sps).min(symbols.len() - 1);
            let q_index = if sample < half {
                0
            } else {
                ((sample - half) / sps).min(symbols.len() - 1)
            };
            let i = symbols[i_index].re * amplitude;
            let q = symbols[q_index].im * amplitude;
            iq.push((i + 127.4).round().clamp(0.0, 255.0) as u8);
            iq.push((q + 127.4).round().clamp(0.0, 255.0) as u8);
        }
        iq
    }

    /// Build a synthetic VCDU+M-PDU payload (before RS encoding) carrying
    /// one small CCSDS Space Packet for a given APID, padded to exactly
    /// `reed_solomon::RS_DATA_LEN * reed_solomon::INTERLEAVE` bytes (the
    /// RS-encoder's expected payload size).
    fn build_synthetic_vcdu_payload(apid: u16, packet_data: &[u8]) -> Vec<u8> {
        let mut payload = Vec::new();
        // VCDU header (6 bytes): version=0, spacecraft_id=1, vcid=5,
        // frame_counter=0.
        payload.push(0b00_000000); // version(2)=0, sc_id_hi(6)=0
        payload.push(0b01_000101); // sc_id_lo(2)=01, vcid(6)=000101
        payload.extend_from_slice(&[0, 0, 0]); // frame counter
        payload.push(0); // spare/cycle byte

        // Meteor AOS VCDU insert zone (two bytes) is present even when the
        // mission does not carry an auxiliary insert payload.
        payload.extend_from_slice(&[0, 0]);

        // M-PDU header (2 bytes): first_header_pointer = 0 (new packet
        // starts immediately after this header).
        payload.push(0x00);
        payload.push(0x00);

        // CCSDS Space Packet primary header (6 bytes) + data.
        let word0 = apid & 0x07FF; // version=0, type=0, sec_hdr=0
        payload.push((word0 >> 8) as u8);
        payload.push((word0 & 0xFF) as u8);
        let word1 = 0b11u16 << 14; // sequence flags = unsegmented
        payload.push((word1 >> 8) as u8);
        payload.push((word1 & 0xFF) as u8);
        let len_field = (packet_data.len() - 1) as u16;
        payload.push((len_field >> 8) as u8);
        payload.push((len_field & 0xFF) as u8);
        payload.extend_from_slice(packet_data);

        // Pad the remainder of the VCDU with a single valid CCSDS idle/fill
        // packet (APID 0x7FF, the standard CCSDS "fill" APID) rather than
        // raw zero bytes: a real downlink continuously fills VCDUs with
        // real or idle packets, and zero-padding would otherwise look like
        // a string of bogus zero-length packet headers to the reassembler,
        // which has no other way to know where meaningful data ends within
        // a frame.
        let target_len = reed_solomon::RS_DATA_LEN * reed_solomon::INTERLEAVE;
        const IDLE_APID: u16 = 0x07FF;
        const IDLE_HEADER_LEN: usize = ccsds::SPACE_PACKET_HEADER_LEN;
        if payload.len() + IDLE_HEADER_LEN <= target_len {
            let idle_data_len = target_len - payload.len() - IDLE_HEADER_LEN;
            let idle_word0 = IDLE_APID & 0x07FF;
            payload.push((idle_word0 >> 8) as u8);
            payload.push((idle_word0 & 0xFF) as u8);
            let idle_word1 = 0b11u16 << 14; // sequence flags = unsegmented
            payload.push((idle_word1 >> 8) as u8);
            payload.push((idle_word1 & 0xFF) as u8);
            let idle_len_field = (idle_data_len - 1) as u16;
            payload.push((idle_len_field >> 8) as u8);
            payload.push((idle_len_field & 0xFF) as u8);
            payload.resize(target_len, 0);
        } else {
            payload.resize(target_len, 0);
        }
        payload
    }

    #[test]
    fn full_pipeline_round_trip_recovers_original_payload_natural_mapping() {
        run_full_pipeline_round_trip(diff_decode::DibitMapping::Natural);
    }

    #[test]
    fn full_pipeline_round_trip_recovers_original_payload_gray_mapping() {
        run_full_pipeline_round_trip(diff_decode::DibitMapping::Gray);
    }

    fn run_full_pipeline_round_trip(mapping: diff_decode::DibitMapping) {
        let packet_data = b"LRPT-TEST-IMAGE-SCANLINE-DATA-0123456789ABCDEF".to_vec();
        let apid = 64u16;
        let payload = build_synthetic_vcdu_payload(apid, &packet_data);

        let sample_rate = 8000.0f32;
        let symbol_rate = 1000.0f32;
        let iq_bytes = build_synthetic_iq_stream(&payload, mapping, sample_rate, symbol_rate);

        // Feed the synthetic stream through the *actual* receive chain:
        // QPSK demod -> differential decode (using the mapping under
        // test) -> frame sync -> derandomize -> RS decode.
        let mut demod = qpsk::QpskDemod::new(qpsk::QpskConfig {
            sample_rate,
            symbol_rate,
            ..qpsk::QpskConfig::default()
        });
        let complex = iq_bytes_to_complex(&iq_bytes);
        let symbols = demod.process(&complex);
        assert!(!symbols.is_empty(), "demod produced no symbols");

        let quadrants: Vec<u8> = symbols
            .iter()
            .map(|s| diff_decode::symbol_to_quadrant(s.value))
            .collect();
        let dibits = diff_decode::differential_decode(&quadrants);
        let bits = diff_decode::dibits_to_bits(&dibits, mapping);

        let mut framer = frame_sync::FrameSync::new();
        let frames = framer.push_bits(&bits);
        assert!(
            !frames.is_empty(),
            "frame sync never locked (mapping={mapping:?})"
        );

        let cadu = &frames[0];
        assert_eq!(cadu.len(), frame_sync::CADU_LEN_BYTES);

        let transport = &cadu[4..4 + reed_solomon::INTERLEAVED_FRAME_LEN];
        let derandomized = randomizer::derandomize(transport);
        let rs_result = reed_solomon::decode_interleaved(&derandomized);
        assert_eq!(
            rs_result.codewords_failed, 0,
            "RS decode should succeed with clean synthetic signal"
        );
        assert_eq!(rs_result.codewords_ok, 4);

        assert_eq!(
            &rs_result.data[..payload.len()],
            &payload[..],
            "recovered payload must match original exactly"
        );

        // Also confirm the packet layer reassembles the original bytes.
        let vcdu_payload = &rs_result.data;
        let mpdu_header = ccsds::parse_mpdu_header(
            &vcdu_payload
                [ccsds::MPDU_HEADER_OFFSET..ccsds::MPDU_HEADER_OFFSET + ccsds::MPDU_HEADER_LEN],
        )
        .unwrap();
        let mpdu_payload = &vcdu_payload[ccsds::MPDU_HEADER_OFFSET + ccsds::MPDU_HEADER_LEN..];
        let mut reassembler = ccsds::PacketReassembler::new();
        let vcdu_header = ccsds::parse_vcdu_header(vcdu_payload).unwrap();
        reassembler.push_mpdu_payload(
            mpdu_payload,
            mpdu_header.first_header_pointer,
            vcdu_header.virtual_channel_id,
            vcdu_header.frame_counter,
        );
        let completed = reassembler.drain_completed();
        // The VCDU also contains one trailing CCSDS idle/fill packet
        // (APID 0x7FF) used to pad the frame to its fixed RS payload size;
        // only the real-APID packet's content matters for this assertion.
        let real: Vec<_> = completed.iter().filter(|p| p.apid == apid).collect();
        assert_eq!(
            real.len(),
            1,
            "expected exactly one packet on the real APID"
        );
        assert_eq!(real[0].data, packet_data);
    }

    #[test]
    fn lrpt_decoder_streaming_api_recovers_image_from_synthetic_signal() {
        let packet_data = vec![42u8; 50];
        let apid = 65u16;
        let payload = build_synthetic_vcdu_payload(apid, &packet_data);

        let sample_rate = 8000u32;
        let symbol_rate = 1000u32;
        let iq_bytes =
            build_coded_oqpsk_iq_stream(&payload, 0, true, sample_rate as f32, symbol_rate as f32);

        let (tx, rx) = crossbeam_channel::unbounded();
        let mut decoder = LrptDecoder::new(sample_rate, symbol_rate, tx);

        // Feed in chunks to exercise the streaming/incremental API.
        for chunk in iq_bytes.chunks(4096) {
            decoder.push_samples(chunk);
        }
        // Drain any progress messages sent (just confirm no panics / the
        // channel API works; content isn't asserted here).
        while rx.try_recv().is_ok() {}

        let result = decoder.finish();
        assert!(
            result.is_some(),
            "decoder should have achieved CADU sync on a clean synthetic signal"
        );
        let result = result.unwrap();
        assert!(
            result.rs_ok > 0,
            "expected at least one successfully RS-decoded codeword"
        );
    }

    #[test]
    fn decode_file_reports_no_sync_on_pure_noise() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("lrpt_nosync_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("noise.iq");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            // Constant DC (no signal at all) -- should never lock.
            let buf = vec![127u8; 4096];
            f.write_all(&buf).unwrap();
        }

        let (tx, _rx) = crossbeam_channel::unbounded();
        let result = decode_file(&path, 8000, 1000, tx);
        assert!(matches!(result, Err(LrptError::NoSync)));

        let _ = std::fs::remove_dir_all(&dir);
    }

    struct ThreeByteReader {
        bytes: std::io::Cursor<Vec<u8>>,
    }

    impl std::io::Read for ThreeByteReader {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            let limit = output.len().min(3);
            self.bytes.read(&mut output[..limit])
        }
    }

    #[test]
    fn decode_cf32_reader_preserves_samples_split_across_reads() {
        let mut bytes = Vec::new();
        for sample in [
            Complex32::new(0.25, -0.5),
            Complex32::new(-0.75, 1.0),
            Complex32::new(0.125, 0.375),
        ] {
            bytes.extend_from_slice(&sample.re.to_le_bytes());
            bytes.extend_from_slice(&sample.im.to_le_bytes());
        }
        let mut reader = ThreeByteReader {
            bytes: std::io::Cursor::new(bytes),
        };
        let (tx, _rx) = crossbeam_channel::unbounded();

        let result = decode_cf32_reader(&mut reader, 8_000, 1_000, tx);

        assert!(matches!(result, Err(LrptError::NoSync)));
        assert_eq!(reader.bytes.position(), 24);
    }

    #[test]
    fn decode_cs8_reader_preserves_iq_pairs_split_across_reads() {
        let mut reader = ThreeByteReader {
            bytes: std::io::Cursor::new(vec![0x80, 0x7f, 0xff, 0x00, 0x01, 0xfe]),
        };
        let (tx, _rx) = crossbeam_channel::unbounded();
        let result = decode_cs8_reader(&mut reader, 8_000, 1_000, tx);
        assert!(matches!(result, Err(LrptError::NoSync)));
        assert_eq!(reader.bytes.position(), 6);
    }

    #[test]
    fn all_phase_and_axis_hypotheses_can_acquire_coded_oqpsk() {
        let packet_data = vec![7u8; 40];
        let apid = 66u16;
        let payload = build_synthetic_vcdu_payload(apid, &packet_data);
        let sample_rate = 8000u32;
        let symbol_rate = 1000u32;
        for hypothesis in 0..8 {
            let iq = build_coded_oqpsk_iq_stream(
                &payload,
                hypothesis,
                true,
                sample_rate as f32,
                symbol_rate as f32,
            );
            let (tx, rx) = crossbeam_channel::unbounded();
            let mut decoder = LrptDecoder::new(sample_rate, symbol_rate, tx);
            for chunk in iq.chunks(4096) {
                decoder.push_samples(chunk);
            }
            while rx.try_recv().is_ok() {}
            let result = decoder
                .finish()
                .unwrap_or_else(|| panic!("hypothesis {hypothesis} failed"));
            assert!(result.rs_ok >= 4, "hypothesis {hypothesis}");
        }
    }

    #[test]
    fn normalized_and_legacy_cf32_recover_the_same_coded_lrpt_frames() {
        let payload = build_synthetic_vcdu_payload(66, &[7; 40]);
        let iq = build_coded_oqpsk_iq_frames(&payload, 0, true, 8_000.0, 1_000.0, 3);
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut original = LrptDecoder::new(8_000, 1_000, tx);
        original.push_samples(&iq);
        let expected = original.finish().expect("CU8 recovers LRPT");
        assert_eq!(expected.rs_ok, 12);
        for amplitude in [Cf32Amplitude::Normalized, Cf32Amplitude::AdcCounts127_4] {
            let bytes = match amplitude {
                Cf32Amplitude::Normalized => uc8_bytes_to_cf32_le(&iq),
                Cf32Amplitude::AdcCounts127_4 => iq
                    .iter()
                    .flat_map(|value| (f32::from(*value) - 127.4).to_le_bytes())
                    .collect(),
            };
            let mut reader = std::io::Cursor::new(bytes);
            let (tx, _rx) = crossbeam_channel::unbounded();
            let actual =
                decode_cf32_reader_with_amplitude(&mut reader, 8_000, 1_000, tx, amplitude)
                    .expect("CF32 recovers LRPT");
            assert_eq!(actual.rs_ok, expected.rs_ok, "{amplitude:?}");
            assert_eq!(actual.rs_failed, expected.rs_failed, "{amplitude:?}");
            assert_eq!(actual.images, expected.images, "{amplitude:?}");
        }
    }

    #[test]
    fn capture_gap_rejects_a_split_cadu_and_preserves_completed_images() {
        let payload = build_synthetic_vcdu_payload(66, &[7; 40]);
        let iq = build_coded_oqpsk_iq_stream(&payload, 0, true, 8_000.0, 1_000.0);
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut decoder = LrptDecoder::new(8_000, 1_000, tx);
        decoder.push_samples(&iq);
        assert_eq!(decoder.rs_ok, 4);
        let images = decoder.images.render_all();
        decoder.reset_stream(8_000, 1_000);
        let cut = iq.len() / 4 * 2;
        decoder.push_samples(&iq[..cut]);
        decoder.reset_stream(8_000, 1_000);
        decoder.push_samples(&iq[cut..]);
        assert_eq!(decoder.rs_ok, 4, "partial CADU crossed a capture gap");
        assert_eq!(decoder.images.render_all(), images);
        decoder.reset_stream(8_000, 1_000);
        decoder.push_samples(&iq);
        assert_eq!(decoder.rs_ok, 8, "decoder must reacquire after the gap");
    }

    #[test]
    fn large_input_blocks_preserve_all_frames_like_small_chunks() {
        let payload = build_synthetic_vcdu_payload(66, &[7; 40]);
        let iq = build_coded_oqpsk_iq_frames(&payload, 0, true, 8_000.0, 1_000.0, 6);
        let (whole_tx, _whole_rx) = crossbeam_channel::unbounded();
        let (chunk_tx, _chunk_rx) = crossbeam_channel::unbounded();
        let mut whole = LrptDecoder::new(8_000, 1_000, whole_tx);
        let mut chunked = LrptDecoder::new(8_000, 1_000, chunk_tx);
        whole.push_samples(&iq);
        for chunk in iq.chunks(4_096) {
            chunked.push_samples(chunk);
        }
        let expected = chunked.finish().expect("small chunks recover frames");
        assert_eq!(expected.rs_ok, 24);
        let actual = whole.finish().expect("large chunk recovers frames");
        assert_eq!(
            actual.rs_ok, expected.rs_ok,
            "large blocks must not discard CADUs"
        );
        assert_eq!(actual.images, expected.images);
    }

    // Build symbols without the RF demodulator so mapping reacquisition is
    // tested independently of the carrier-recovery defect tracked as S02.
    fn coded_symbol_frames(frame_count: usize, nrzm: bool) -> Vec<qpsk::RecoveredSymbol> {
        let mut information = vec![0; 256];
        for counter in 0..frame_count {
            let mut payload = build_synthetic_vcdu_payload(66, &[counter as u8; 40]);
            payload[4] = counter as u8;
            let mut cadu = frame_sync::SYNC_MARKER.to_be_bytes().to_vec();
            cadu.extend(randomizer::derandomize(&reed_solomon::encode_interleaved(
                &payload,
            )));
            information.extend(diff_decode::bytes_to_bits(&cadu));
        }
        information.extend([0; 256]);
        let mut level = 0;
        let mut state = 0;
        information
            .into_iter()
            .map(|bit| {
                let input = if nrzm {
                    level ^= bit;
                    level
                } else {
                    bit
                };
                let register: u8 = (input << 6) | state;
                state = register >> 1;
                let values = [0o171, 0o133].map(|polynomial| {
                    if (register & polynomial).count_ones().is_multiple_of(2) {
                        -1.0
                    } else {
                        1.0
                    }
                });
                qpsk::RecoveredSymbol {
                    value: Complex32::new(values[0], values[1]),
                    alternate_value: None,
                    costas_locked: true,
                }
            })
            .collect()
    }

    #[test]
    fn missing_energy_is_owned_by_the_observed_stagger_pairing() {
        for alternate in [false, true] {
            let mut symbols = coded_symbol_frames(3, true);
            for symbol in &mut symbols {
                if alternate {
                    symbol.alternate_value = Some(symbol.value);
                    symbol.value = Complex32::new(0.0, 0.0);
                } else {
                    symbol.alternate_value = Some(Complex32::new(0.0, 0.0));
                }
            }
            let (tx, _rx) = crossbeam_channel::bounded(2);
            let mut decoder = LrptDecoder::new(8000, 1000, tx);
            decoder.push_recovered_symbols(&symbols);
            assert_eq!(decoder.rs_ok, 12);
            assert_eq!(decoder.validated_payloads.len(), 3);
            assert_eq!(decoder.selected_lane.unwrap() >= 8, alternate);
        }
    }

    #[test]
    fn hypotheses_reacquire_polarity_conjugation_and_axis_changes_in_stream_order() {
        for nrzm in [false, true] {
            let mut symbols = coded_symbol_frames(16, nrzm);
            // Change mappings between frames, including inversion of both
            // axes (a 180-degree slip), conjugation and exchange of branches.
            for (index, symbol) in symbols.iter_mut().enumerate() {
                let frame = index.saturating_sub(256) / 8192;
                let mapping = [0, 3, 2, 4][(frame / 4).min(3)];
                if mapping & 4 != 0 {
                    symbol.value = Complex32::new(symbol.value.im, symbol.value.re);
                }
                if mapping & 1 != 0 {
                    symbol.value.re = -symbol.value.re;
                }
                if mapping & 2 != 0 {
                    symbol.value.im = -symbol.value.im;
                }
            }
            let mut expected = None;
            for chunk_size in [97, 4096, symbols.len()] {
                let (tx, _rx) = crossbeam_channel::unbounded();
                let mut decoder = LrptDecoder::new(8000, 1000, tx);
                for chunk in symbols.chunks(chunk_size) {
                    decoder.push_recovered_symbols(chunk);
                }
                assert!(decoder.rs_ok >= 13 * 4, "failed to recover later mappings");
                assert!(decoder.selected_lane.is_some());
                let mut previous = None;
                for payload in &decoder.validated_payloads {
                    let counter = payload[4];
                    let mut wanted = build_synthetic_vcdu_payload(66, &[counter; 40]);
                    wanted[4] = counter;
                    assert_eq!(payload, &wanted, "reacquired payload changed");
                    assert!(previous.is_none_or(|old| counter > old));
                    previous = Some(counter);
                }
                assert_eq!(previous, Some(15), "final mapping failed to reacquire");
                let observed = (
                    decoder.rs_ok,
                    decoder.last_valid_frame_end,
                    decoder.validated_payloads,
                );
                if let Some(expected) = &expected {
                    assert_eq!(&observed, expected, "NRZ-M={nrzm}, chunk={chunk_size}");
                } else {
                    expected = Some(observed);
                }
            }
        }
    }

    #[test]
    fn frame_lock_expires_during_fade_and_later_frames_reacquire_without_reset() {
        let symbols = coded_symbol_frames(8, true);
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut decoder = LrptDecoder::new(8000, 1000, tx);
        let cut = 256 + 3 * 8192 + 128;
        decoder.push_recovered_symbols(&symbols[..cut]);
        assert_eq!(decoder.rs_ok, 12);
        assert!(decoder.selected_lane.is_some());
        let fade = vec![
            qpsk::RecoveredSymbol {
                value: Complex32::new(0.0, 0.0),
                alternate_value: None,
                costas_locked: false,
            };
            FRAME_LOCK_MAX_AGE as usize
        ];
        decoder.push_recovered_symbols(&fade);
        assert!(
            decoder.selected_lane.is_none(),
            "symbols={} last_valid={:?} rs_ok={}",
            decoder.symbols_received,
            decoder.last_valid_frame_end,
            decoder.rs_ok
        );
        decoder.maybe_send_progress();
        assert!(!rx.recv().unwrap().frame_locked);
        decoder.push_recovered_symbols(&symbols[cut..]);
        assert!(decoder.rs_ok >= 7 * 4);
        assert!(decoder.selected_lane.is_some());
        assert!(
            decoder.finish().is_some(),
            "historical result remains available"
        );
    }

    #[test]
    fn repeated_identical_frames_are_distinct_but_equivalent_lanes_are_not() {
        let payload = build_synthetic_vcdu_payload(66, &[7; 40]);
        let iq = build_coded_oqpsk_iq_frames(&payload, 0, true, 8000.0, 1000.0, 6);
        for chunk_size in [4096, iq.len()] {
            let (tx, _rx) = crossbeam_channel::unbounded();
            let mut decoder = LrptDecoder::new(8000, 1000, tx);
            for chunk in iq.chunks(chunk_size) {
                decoder.push_samples(chunk);
            }
            assert_eq!(decoder.rs_ok, 24);
        }
    }

    #[test]
    fn full_progress_queue_does_not_block_decoding() {
        let (progress_tx, progress_rx) = crossbeam_channel::bounded(1);
        progress_tx.send(DecodeProgress::default()).unwrap();
        let (done_tx, done_rx) = crossbeam_channel::bounded(1);
        let worker = std::thread::spawn(move || {
            let mut decoder = LrptDecoder::new(8_000, 1_000, progress_tx);
            for _ in 0..6 {
                decoder.push_complex(&[Complex32::new(0.0, 0.0); 512]);
            }
            done_tx.send(()).unwrap();
        });
        let finished = done_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .is_ok();
        // Always release an old blocking implementation before asserting.
        drop(progress_rx);
        worker.join().unwrap();
        assert!(finished, "a full progress queue blocked decoding");
    }

    #[test]
    fn undrained_unbounded_progress_channel_retains_at_most_two_snapshots() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut decoder = LrptDecoder::new(8_000, 1_000, tx);
        for _ in 0..40 {
            decoder.push_complex(&[Complex32::new(0.0, 0.0); 64]);
        }
        assert_eq!(rx.len(), 2);
    }

    #[test]
    fn sparse_progress_counts_segments_and_obeys_observation_cadence() {
        for chunk_size in [1, 97] {
            let (tx, rx) = crossbeam_channel::unbounded();
            let mut decoder = LrptDecoder::new(8000, 1000, tx);
            decoder.images.push_packet(
                64,
                16_300,
                include_bytes!("../../audit/2026-10-07/fixtures/nonflat-q50.segment"),
            );
            let samples = vec![Complex32::new(0.0, 0.0); 8000];
            let mut observed_at = Vec::new();
            for chunk in samples.chunks(chunk_size) {
                decoder.push_complex(chunk);
                for progress in rx.try_iter() {
                    assert_eq!(progress.lines_decoded, 0);
                    assert_eq!(progress.segments_decoded, 1);
                    assert!(!progress.costas_locked && !progress.frame_locked);
                    assert_eq!(progress.preview[0].1.height(), 4); // bounded 1568->784 width
                    observed_at.push(decoder.symbols_received);
                }
            }
            assert!(observed_at.len() >= 3, "progress stalled: {observed_at:?}");
            for pair in observed_at.windows(2) {
                assert!(
                    (250..=263).contains(&(pair[1] - pair[0])),
                    "cadence={pair:?}"
                );
            }
        }
    }

    #[test]
    fn interleaved_mission_recovers_independent_payloads_and_observation_positions() {
        let expected: Vec<_> =
            include_bytes!("../../audit/2026-10-07/fixtures/meteor-80k-payloads.bin")
                .chunks_exact(892)
                .map(<[u8]>::to_vec)
                .collect();
        let bytes = include_bytes!("../../audit/2026-10-07/fixtures/meteor-80k-wire.bits");
        let bits: Vec<_> = bytes
            .iter()
            .flat_map(|&byte| (0..8).map(move |bit| (byte >> (7 - bit)) & 1))
            .collect();
        let symbols: Vec<_> = bits
            .chunks_exact(2)
            .map(|pair| qpsk::RecoveredSymbol {
                value: Complex32::new(1.0 - 2.0 * pair[0] as f32, 1.0 - 2.0 * pair[1] as f32),
                alternate_value: None,
                costas_locked: true,
            })
            .collect();
        let mut wanted_end = None;
        for chunk_size in [997, 16_384] {
            let (tx, _rx) = crossbeam_channel::bounded(2);
            let mut decoder =
                LrptDecoder::for_mission(320_000, MeteorProfile::Oqpsk80Interleaved, tx).unwrap();
            for chunk in symbols.chunks(chunk_size) {
                decoder.push_recovered_symbols(chunk);
            }
            assert_eq!(decoder.validated_payloads, expected);
            assert_eq!(decoder.rs_ok, 12);
            assert!(decoder.last_valid_frame_end.unwrap() <= decoder.symbols_received);
            if let Some(end) = wanted_end {
                assert_eq!(decoder.last_valid_frame_end, end);
            }
            wanted_end = Some(decoder.last_valid_frame_end);
        }
    }

    #[test]
    #[ignore = "requires independent interleaved/RRC IQ generated by generate_interleaver_fixture.py --iq"]
    fn independent_interleaved_iq_recovers_both_staggers_and_identical_payloads() {
        let directory = std::path::PathBuf::from(std::env::var("EZ_INTERLEAVER_VECTORS").unwrap());
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(directory.join("meteor-80k-fixture.json")).unwrap(),
        )
        .unwrap();
        let bytes = std::fs::read(directory.join("meteor-80k-payloads.bin")).unwrap();
        let expected: Vec<_> = bytes.chunks_exact(892).map(<[u8]>::to_vec).collect();
        for case in manifest["iq_cases"].as_array().unwrap() {
            let iq = cf32_le_bytes_to_complex(
                &std::fs::read(directory.join(case["file"].as_str().unwrap())).unwrap(),
            );
            for chunk_size in [997, 16_384] {
                let (tx, _rx) = crossbeam_channel::bounded(2);
                let mut decoder =
                    LrptDecoder::for_mission(320_000, MeteorProfile::Oqpsk80Interleaved, tx)
                        .unwrap();
                for chunk in iq.chunks(chunk_size) {
                    decoder.push_complex(chunk);
                }
                println!(
                    "interleaved {} chunk={chunk_size} frames={} rs_ok={}",
                    case["file"],
                    decoder.validated_payloads.len(),
                    decoder.rs_ok
                );
                assert!(
                    decoder.validated_payloads == expected,
                    "interleaved payload mismatch"
                );
                assert_eq!(decoder.rs_ok, 12);
            }
        }
    }

    #[test]
    fn mission_gap_preserves_product_but_new_product_retires_images_and_counts() {
        let (tx, _rx) = crossbeam_channel::bounded(2);
        let mut decoder = LrptDecoder::for_mission(288_000, MeteorProfile::Oqpsk72, tx).unwrap();
        let segment = include_bytes!("../../audit/2026-10-07/fixtures/nonflat-q50.segment");
        let payload = build_synthetic_vcdu_payload(64, segment);
        let mut cadu = frame_sync::SYNC_MARKER.to_be_bytes().to_vec();
        cadu.extend(randomizer::derandomize(&reed_solomon::encode_interleaved(
            &payload,
        )));
        assert!(decoder.process_cadu_frame(&cadu));
        let rs_ok = decoder.rs_ok;
        assert_eq!(decoder.images.total_segment_count(), 1);
        let received_image = decoder.images.render(64).unwrap();
        decoder.reset_mission_stream(576_000).unwrap();
        assert_eq!(decoder.rs_ok, rs_ok);
        assert_eq!(decoder.images.total_segment_count(), 1);
        assert_eq!(decoder.images.render(64).unwrap(), received_image);
        assert!(decoder.ever_locked);
        decoder
            .start_new_product(320_000, MeteorProfile::Oqpsk80Interleaved)
            .unwrap();
        assert_eq!(decoder.rs_ok, 0);
        assert!(!decoder.ever_locked);
        assert_eq!(decoder.images.total_line_count(), 0);
        assert_eq!(decoder.images.total_segment_count(), 0);
    }

    #[test]
    fn interleaved_profile_never_accepts_a_plain_uninterleaved_transport() {
        let symbols = coded_symbol_frames(3, true);
        let (tx, _rx) = crossbeam_channel::bounded(2);
        let mut decoder =
            LrptDecoder::for_mission(320_000, MeteorProfile::Oqpsk80Interleaved, tx).unwrap();
        decoder.push_recovered_symbols(&symbols);
        assert_eq!(decoder.rs_ok, 0);
        assert!(decoder.validated_payloads.is_empty());
    }

    #[test]
    fn mission_profile_rejects_unsupported_rates_and_preserves_explicit_transport_on_gap() {
        for profile in MeteorProfile::ALL {
            let (tx, _rx) = crossbeam_channel::bounded(2);
            assert!(LrptDecoder::for_mission(
                profile.symbol_rate_hz() * 4 - 1,
                profile,
                tx.clone()
            )
            .is_err());
            let mut decoder =
                LrptDecoder::for_mission(profile.symbol_rate_hz() * 4, profile, tx).unwrap();
            decoder.reset_mission_stream(2_048_000).unwrap();
            assert_eq!(decoder.profile, Some(profile));
            assert!(decoder
                .lanes
                .iter()
                .all(|lane| lane.profile == Some(profile)
                    && lane.interleaver.is_some() == profile.interleaved()));
        }
    }

    #[test]
    fn independent_oqpsk_fixtures_recover_both_staggers_with_frequency_and_clock_offsets() {
        let expected: Vec<_> =
            include_bytes!("../../audit/2026-10-07/fixtures/oqpsk-expected-payloads.bin")
                .chunks_exact(892)
                .map(<[u8]>::to_vec)
                .collect();
        // Independent analytical RRC/libfec waveforms: opposite staggers,
        // 123/287-degree phase, +/-1 kHz carrier and -/+150 ppm clock drift.
        for bytes in [
            include_bytes!("../../audit/2026-10-07/fixtures/oqpsk-case-55.cf32").as_slice(),
            include_bytes!("../../audit/2026-10-07/fixtures/oqpsk-case-59.cf32").as_slice(),
        ] {
            let iq = cf32_le_bytes_to_complex(bytes);
            for chunk_size in [997, 16_384] {
                let (tx, _rx) = crossbeam_channel::bounded(2);
                let mut decoder =
                    LrptDecoder::for_mission(288_000, MeteorProfile::Oqpsk72, tx).unwrap();
                for chunk in iq.chunks(chunk_size) {
                    decoder.push_complex(chunk);
                }
                assert_eq!(decoder.validated_payloads, expected);
                assert_eq!(decoder.rs_ok, 12);
            }
        }
    }

    #[test]
    #[ignore = "requires the independent libfec/RRC corpus generated by generate_oqpsk_vectors.py"]
    fn independent_oqpsk_carrier_corpus_recovers_identical_payloads() {
        let directory =
            std::path::PathBuf::from(std::env::var("EZ_OQPSK_VECTORS").expect("EZ_OQPSK_VECTORS"));
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(directory.join("manifest.json")).unwrap())
                .unwrap();
        let expected = std::fs::read(directory.join("expected-payloads.bin")).unwrap();
        let expected: Vec<_> = expected.chunks_exact(892).map(<[u8]>::to_vec).collect();
        let mut failures = Vec::new();
        for case in manifest["cases"].as_array().unwrap() {
            let name = case["file"].as_str().unwrap();
            let bytes = std::fs::read(directory.join(name)).unwrap();
            let iq = cf32_le_bytes_to_complex(&bytes);
            for chunk_size in [997, 16_384] {
                let (tx, _rx) = crossbeam_channel::bounded(2);
                let mut decoder = LrptDecoder::new(
                    case["sample_rate"].as_u64().unwrap() as u32,
                    case["symbol_rate"].as_u64().unwrap() as u32,
                    tx,
                );
                for chunk in iq.chunks(chunk_size) {
                    decoder.push_complex(chunk);
                }
                let good = decoder.validated_payloads == expected && decoder.rs_ok == 12;
                println!(
                    "{name} chunk={chunk_size} rs_ok={} payload_equal={good}",
                    decoder.rs_ok
                );
                if !good {
                    failures.push(format!(
                        "{case} chunk={chunk_size} rs_ok={} frames={}",
                        decoder.rs_ok,
                        decoder.validated_payloads.len()
                    ));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "independent OQPSK failures: {failures:#?}"
        );
    }

    #[test]
    #[ignore = "requires the downloaded independent public Meteor IQ and corrected CADUs"]
    fn public_off_air_carrier_recovers_reference_payload_bytes() {
        use std::io::{Read, Seek, SeekFrom};
        let iq_path = std::env::var("EZ_METEOR_CS8").expect("EZ_METEOR_CS8");
        let reference =
            std::fs::read(std::env::var("EZ_METEOR_CADU").expect("EZ_METEOR_CADU")).unwrap();
        let mut expected = std::collections::BTreeMap::new();
        for cadu in reference.chunks_exact(1024) {
            assert_eq!(&cadu[..4], &[0x1a, 0xcf, 0xfc, 0x1d]);
            let payload = &cadu[4..896];
            let header = ccsds::parse_vcdu_header(payload).unwrap();
            expected.insert(
                (header.virtual_channel_id, header.frame_counter),
                payload.to_vec(),
            );
        }
        let start: u64 = std::env::var("EZ_METEOR_START_SECONDS")
            .unwrap_or_else(|_| "180".into())
            .parse()
            .unwrap();
        let seconds: u64 = std::env::var("EZ_METEOR_SECONDS")
            .unwrap_or_else(|_| "20".into())
            .parse()
            .unwrap();
        let mut input = std::fs::File::open(iq_path).unwrap();
        input.seek(SeekFrom::Start(start * 288_000 * 2)).unwrap();
        let mut input = input.take(seconds * 288_000 * 2);
        let (tx, _rx) = crossbeam_channel::bounded(2);
        let mut decoder = LrptDecoder::for_mission(288_000, MeteorProfile::Oqpsk72, tx).unwrap();
        let mut bytes = [0; 8192];
        loop {
            let count = input.read(&mut bytes).unwrap();
            if count == 0 {
                break;
            }
            assert_eq!(count % 2, 0);
            decoder.push_samples_cs8(&bytes[..count]);
        }
        let mut matched = 0;
        let mut additional = 0;
        for payload in &decoder.validated_payloads {
            let header = ccsds::parse_vcdu_header(payload).unwrap();
            if let Some(wanted) = expected.get(&(header.virtual_channel_id, header.frame_counter)) {
                assert_eq!(
                    payload, wanted,
                    "VCID {} counter {}",
                    header.virtual_channel_id, header.frame_counter
                );
                matched += 1;
            } else {
                additional += 1;
            }
        }
        println!("public off-air start={start}s duration={seconds}s accepted={} matched={matched} additional={additional} rs_ok={}",decoder.validated_payloads.len(),decoder.rs_ok);
        assert!(matched >= 100, "insufficient independent off-air matches");
        assert_eq!(
            additional, 0,
            "extra CADUs require an independent decoder comparison"
        );
    }

    #[test]
    fn corrupted_rs_frame_is_dropped_and_does_not_pollute_reassembler() {
        // Issue 28: CADU frames with uncorrectable RS errors must be dropped.
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut decoder = LrptDecoder::new(48000, 72000, tx);

        let payload = vec![0u8; reed_solomon::RS_DATA_LEN * reed_solomon::INTERLEAVE];
        let mut transport = randomizer::derandomize(&reed_solomon::encode_interleaved(&payload));
        for error in 0..17 {
            transport[error * reed_solomon::INTERLEAVE] ^= 0x5a;
        }
        let mut cadu = frame_sync::SYNC_MARKER.to_be_bytes().to_vec();
        cadu.extend_from_slice(&transport);

        decoder.process_cadu_frame(&cadu);

        // RS failed count must increment, but no packets should be accepted
        assert!(decoder.rs_failed > 0);
        assert_eq!(decoder.images.total_line_count(), 0);
        assert!(decoder.reassembler.drain_completed().is_empty());
    }
}
