//! Clean-room Meteor-M2 LRPT (Low Rate Picture Transmission) decoder.
//!
//! Pipeline: raw IQ bytes -> QPSK demod -> differential decode -> CCSDS CADU
//! frame sync -> derandomize -> Reed-Solomon(255,223) correction -> VCDU/M-PDU
//! Space Packet reassembly -> per-APID scanline image reconstruction.
//!
//! Implemented from public CCSDS 131.0-B / 132.0-B specifications only.
//! Scope: Meteor-M2-3 and Meteor-M2-4 LRPT downlinks. No APT/NOAA support.

mod ccsds;
#[cfg(test)]
mod diff_decode;
mod frame_sync;
mod image_builder;
mod iq_source;
mod qpsk;
mod randomizer;
mod reed_solomon;
mod viterbi;

use crossbeam_channel::Sender;
use num_complex::Complex32;
use std::path::Path;

pub use image_builder::apid_channel_label;
pub use iq_source::{cf32_le_bytes_to_complex, cs8_bytes_to_complex, iq_bytes_to_complex};

#[derive(Debug, thiserror::Error)]
pub enum LrptError {
    #[error("failed to read IQ file: {0}")]
    Io(#[from] std::io::Error),
    #[error("no CADU frame sync ever achieved on this recording")]
    NoSync,
}

/// Snapshot of decode progress, sent periodically over `progress_tx` so the
/// GUI can render a live meter/preview without blocking on the full decode.
#[derive(Debug, Clone, Default)]
pub struct DecodeProgress {
    pub lines_decoded: u32,
    pub rs_ok: u32,
    pub rs_failed: u32,
    pub costas_locked: bool,
    /// Whether the CADU frame synchronizer currently has a locked sync
    /// marker (distinct from `costas_locked`, which reflects the QPSK
    /// carrier loop rather than frame-level lock).
    pub frame_locked: bool,
    /// Scanlines refused by the image builder's anti-DoS caps. Rising on a
    /// live pass means junk/oversize input, not image growth.
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
    let file = std::fs::File::open(path)?;
    let mut reader = std::io::BufReader::with_capacity(1 << 20, file);
    decode_cf32_reader(&mut reader, sample_rate, symbol_rate, progress_tx)
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

fn decode_cf32_reader(
    reader: &mut impl std::io::Read,
    sample_rate: u32,
    symbol_rate: u32,
    progress_tx: Sender<DecodeProgress>,
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
                    decoder.push_samples_cf32(&chunk[..aligned_len]);
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
                decoder.push_samples_cf32(&prefix);
                carry.clear();

                let remaining = &chunk[needed..n];
                let aligned_len = remaining.len() / 8 * 8;
                if aligned_len > 0 {
                    decoder.push_samples_cf32(&remaining[..aligned_len]);
                }
                carry.extend_from_slice(&remaining[aligned_len..]);
            }
            Err(e) => return Err(LrptError::Io(e)),
        }
    }

    decoder.finish().ok_or(LrptError::NoSync)
}

/// How often (in newly-produced CADU frames) to emit a `DecodeProgress`
/// update and refresh the image preview. Keeps the channel from being
/// flooded on high-rate streams while still feeling "live".
const PROGRESS_EVERY_N_FRAMES: u32 = 5;

/// One phase/axis hypothesis for the QPSK-to-convolutional-code path.
/// Independent I/Q inversion plus optional axis swap covers all eight
/// QPSK phase and conjugation ambiguities without applying differential
/// decoding at the wrong side of the Viterbi decoder.
struct DecodeLane {
    viterbi: viterbi::ViterbiDecoder,
    plain_framer: frame_sync::FrameSync,
    nrzm_framer: frame_sync::FrameSync,
    nrzm_previous: Option<u8>,
}

impl DecodeLane {
    fn new() -> Self {
        Self {
            viterbi: viterbi::ViterbiDecoder::new(),
            plain_framer: frame_sync::FrameSync::new(),
            nrzm_framer: frame_sync::FrameSync::new(),
            nrzm_previous: None,
        }
    }

    fn push_symbols(
        &mut self,
        symbols: &[qpsk::RecoveredSymbol],
        hypothesis: usize,
    ) -> Vec<Vec<u8>> {
        let mut coded = Vec::with_capacity(symbols.len() * 2);
        for symbol in symbols {
            let mut first = u8::from(symbol.value.re < 0.0);
            let mut second = u8::from(symbol.value.im < 0.0);
            if hypothesis & 0b100 != 0 {
                std::mem::swap(&mut first, &mut second);
            }
            first ^= (hypothesis & 0b001 != 0) as u8;
            second ^= (hypothesis & 0b010 != 0) as u8;
            coded.extend_from_slice(&[first, second]);
        }
        let recovered = self.viterbi.push_bits(&coded);
        let mut frames = self.plain_framer.push_bits(&recovered);

        let mut nrzm = Vec::with_capacity(recovered.len());
        for bit in recovered {
            if let Some(previous) = self.nrzm_previous {
                nrzm.push(previous ^ bit);
            }
            self.nrzm_previous = Some(bit);
        }
        frames.extend(self.nrzm_framer.push_bits(&nrzm));
        frames
    }
}

/// Streaming decoder driven incrementally with IQ byte chunks, for live
/// decode fed from the same sample loop that feeds the recorder/spectrum.
pub struct LrptDecoder {
    progress_tx: Sender<DecodeProgress>,

    demod: qpsk::QpskDemod,
    lanes: [DecodeLane; 8],
    selected_lane: Option<usize>,
    reassembler: ccsds::PacketReassembler,
    images: image_builder::ImageBuilder,

    ever_locked: bool,
    rs_ok: u32,
    rs_failed: u32,
    frames_since_progress: u32,
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
            lanes: std::array::from_fn(|_| DecodeLane::new()),
            selected_lane: None,
            reassembler: ccsds::PacketReassembler::new(),
            images: image_builder::ImageBuilder::new(),
            ever_locked: false,
            rs_ok: 0,
            rs_failed: 0,
            frames_since_progress: 0,
        }
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
        let complex = cf32_le_bytes_to_complex(bytes);
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

        let mut frames = Vec::new();
        if let Some(selected) = self.selected_lane {
            frames.extend(
                self.lanes[selected]
                    .push_symbols(&symbols, selected)
                    .into_iter()
                    .map(|frame| (selected, frame)),
            );
        } else {
            for (hypothesis, lane) in self.lanes.iter_mut().enumerate() {
                frames.extend(
                    lane.push_symbols(&symbols, hypothesis)
                        .into_iter()
                        .map(|frame| (hypothesis, frame)),
                );
            }
        }
        if frames.is_empty() {
            self.maybe_send_progress();
            return;
        }

        // NOTE: finding a sync-marker match (a drained frame) is not by
        // itself proof of a genuine lock -- at ~9.6e-6 probability per
        // 32-bit window, a long enough run of pure noise will eventually
        // produce a spurious marker match too. `ever_locked` (and thus
        // disabling the mapping-fallback search / the `NoSync` error) is
        // only set once a frame actually RS-decodes successfully, inside
        // `process_cadu_frame` -- RS(255,223) correcting real data by
        // chance on random bytes is astronomically less likely than a
        // 32-bit marker coincidence, making it a trustworthy lock signal.
        for (hypothesis, frame) in frames {
            if self
                .selected_lane
                .is_some_and(|selected| selected != hypothesis)
            {
                continue;
            }
            if self.process_cadu_frame(&frame) {
                self.selected_lane = Some(hypothesis);
            }
        }

        self.maybe_send_progress();
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
        self.frames_since_progress += 1;
        let interval = std::cmp::max(PROGRESS_EVERY_N_FRAMES, self.images.total_line_count() / 50);
        if self.frames_since_progress < interval {
            return;
        }
        self.frames_since_progress = 0;
        if self.progress_tx.is_full() || self.progress_tx.len() >= 2 {
            return;
        }

        let progress = DecodeProgress {
            lines_decoded: self.images.total_line_count(),
            rs_ok: self.rs_ok,
            rs_failed: self.rs_failed,
            costas_locked: self.demod.is_locked(),
            frame_locked: self.selected_lane.is_some(),
            dropped_scanlines: self.images.dropped_scanlines(),
            preview: self.images.render_previews(1_024),
        };
        let _ = self.progress_tx.try_send(progress);
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
        assert!(sps >= 2 && sps % 2 == 0);
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
