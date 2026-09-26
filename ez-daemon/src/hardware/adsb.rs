//! Deterministic 1090ES ADS-B waveform generator.
//!
//! RTL-SDR devices expose unsigned interleaved 8-bit I/Q (`Uc8`).  This module
//! generates the same sample convention at 2.4 MSPS: a Mode-S preamble followed
//! by 112 PPM bits.  It is intentionally noiseless at the carrier envelope so
//! decoder tests are stable, while still passing through the exact Uc8 converter
//! used by [`RtlSdrSource`].

use num_complex::Complex32;

use super::IqSource;

pub const ADSB_SAMPLE_RATE_HZ: u32 = 2_400_000;
const U8_MID: f32 = 127.4;
const IDLE_SAMPLES: usize = 95;
const TRAILING_SAMPLES: usize = 128;
const HIGH_LEVEL: f32 = 0.90;

/// A small, valid DF17 identification message used by the built-in emulator.
pub const EXAMPLE_DF17_CALLSIGN: [u8; 14] = [
    0x8D, 0x48, 0x40, 0xD6, 0x20, 0x2C, 0xC3, 0x7C, 0xDB, 0x31, 0x57, 0x9F, 0xE8, 0x02,
];

/// Encode one or more 112-bit Mode-S messages as raw RTL-SDR Uc8 IQ.
#[must_use]
pub fn encode_uc8(messages: &[[u8; 14]]) -> Vec<u8> {
    let mut out = Vec::new();
    for message in messages {
        out.extend(std::iter::repeat_n(127u8, IDLE_SAMPLES * 2));

        // The 8 us Mode-S preamble at 2.4 MSPS.  This phase matches one of the
        // demodulator's accepted preamble templates (high at samples 1, 3, 9, 12).
        for level in [
            0.0, HIGH_LEVEL, 0.0, HIGH_LEVEL, 0.0, 0.0, 0.0, 0.0, 0.0, HIGH_LEVEL, 0.0, 0.0,
            HIGH_LEVEL, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        ] {
            push_sample(&mut out, level);
        }

        for bit_index in 0..112 {
            let bit = (message[bit_index / 8] >> (7 - (bit_index % 8))) & 1;
            let start = ((bit_index as f64) * 2.4).round() as usize;
            let end = (((bit_index + 1) as f64) * 2.4).round() as usize;
            let mid = ((bit_index as f64) * 2.4 + 1.2).round() as usize;
            for sample in start..end {
                let one = bit != 0 && sample < mid;
                let zero = bit == 0 && sample >= mid;
                push_sample(&mut out, if one || zero { HIGH_LEVEL } else { 0.0 });
            }
        }

        out.extend(std::iter::repeat_n(127u8, TRAILING_SAMPLES * 2));
    }
    out
}

fn push_sample(out: &mut Vec<u8>, level: f32) {
    let i = (U8_MID + level * 127.0).round().clamp(0.0, 255.0) as u8;
    // Keep Q at the ADC midpoint.  This is a valid real-envelope test signal;
    // the production path computes magnitude from both I and Q exactly as rtl-sdr.
    out.extend_from_slice(&[i, 127]);
}

/// In-memory source that behaves like a live RTL-SDR stream.
pub struct AdsbEmulatorSource {
    iq: Vec<Complex32>,
    cursor: usize,
    looping: bool,
    frequency_hz: u64,
    gain_db: f64,
    running: bool,
}

impl AdsbEmulatorSource {
    #[must_use]
    pub fn new(messages: &[[u8; 14]], looping: bool) -> Self {
        let raw = encode_uc8(messages);
        Self {
            iq: lrpt_decode::iq_bytes_to_complex(&raw),
            cursor: 0,
            looping,
            frequency_hz: 1_090_000_000,
            gain_db: 40.0,
            running: false,
        }
    }

    #[must_use]
    pub fn example(looping: bool) -> Self {
        Self::new(&[EXAMPLE_DF17_CALLSIGN], looping)
    }
}

impl IqSource for AdsbEmulatorSource {
    fn start(&mut self) -> anyhow::Result<()> {
        self.running = true;
        self.cursor = 0;
        Ok(())
    }

    fn stop(&mut self) {
        self.running = false;
    }

    fn set_frequency(&mut self, hz: u64) -> anyhow::Result<()> {
        self.frequency_hz = hz;
        Ok(())
    }

    fn set_sample_rate(&mut self, hz: u32) -> anyhow::Result<()> {
        if hz != ADSB_SAMPLE_RATE_HZ {
            anyhow::bail!("ADS-B emulator requires {ADSB_SAMPLE_RATE_HZ} Hz");
        }
        Ok(())
    }

    fn set_gain(&mut self, db: f64) -> anyhow::Result<()> {
        self.gain_db = db;
        Ok(())
    }

    fn read_iq(&mut self, buf: &mut [Complex32]) -> anyhow::Result<usize> {
        if !self.running || self.iq.is_empty() {
            return Ok(0);
        }
        let mut written = 0;
        while written < buf.len() {
            if self.cursor == self.iq.len() {
                if !self.looping {
                    break;
                }
                self.cursor = 0;
            }
            let n = (buf.len() - written).min(self.iq.len() - self.cursor);
            buf[written..written + n].copy_from_slice(&self.iq[self.cursor..self.cursor + n]);
            self.cursor += n;
            written += n;
        }
        Ok(written)
    }

    fn frequency_hz(&self) -> u64 {
        self.frequency_hz
    }
    fn sample_rate_hz(&self) -> u32 {
        ADSB_SAMPLE_RATE_HZ
    }
    fn gain_db(&self) -> f64 {
        self.gain_db
    }
    fn kind(&self) -> &'static str {
        "adsb-emulator"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dump1090::demod::{Demod2400, DemodStats, MagBuf, MagBufFlags};

    #[test]
    fn generated_uc8_decodes_as_the_example_aircraft() {
        let raw = encode_uc8(&[EXAMPLE_DF17_CALLSIGN]);
        let complex = lrpt_decode::iq_bytes_to_complex(&raw);
        let data: Vec<u16> = complex
            .iter()
            .map(|sample| ((sample.norm() / 128.0) * 65535.0).min(65535.0) as u16)
            .collect();
        let mut demod = Demod2400::new();
        let mut stats = DemodStats::default();
        let mut decoded = Vec::new();
        let len = data.len();
        demod.demodulate(
            &MagBuf {
                total_length: len,
                valid_length: len,
                overlap: 0,
                data,
                sample_timestamp: 0,
                sys_timestamp: 0,
                flags: MagBufFlags(0),
                mean_level: 0.1,
                mean_power: 0.1,
                dropped: 0,
            },
            &mut stats,
            &mut |message| decoded.push(message.msg),
        );
        assert!(
            decoded.contains(&EXAMPLE_DF17_CALLSIGN),
            "decoded {} messages, preambles {} accepted {:?}",
            decoded.len(),
            stats.demod_preambles,
            stats.demod_accepted
        );
    }
}
