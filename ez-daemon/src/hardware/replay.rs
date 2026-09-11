//! Replays a previously recorded IQ file as if it were a live source. Understands the
//! two on-disk formats already used elsewhere in this workspace: interleaved raw
//! unsigned-8-bit I/Q (`.iq`) and interleaved little-endian f32 I/Q (`.cf32`). Reuses
//! `lrpt_decode`'s byte->`Complex32` converters so all three crates agree on the exact
//! same DC-offset/scale conventions.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use num_complex::Complex32;

use super::IqSource;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayFormat {
    RawU8,
    Cf32Le,
}

impl ReplayFormat {
    fn bytes_per_sample(self) -> usize {
        match self {
            ReplayFormat::RawU8 => 2,
            ReplayFormat::Cf32Le => 8,
        }
    }

    fn convert(self, bytes: &[u8]) -> Vec<Complex32> {
        match self {
            ReplayFormat::RawU8 => lrpt_decode::iq_bytes_to_complex(bytes),
            ReplayFormat::Cf32Le => lrpt_decode::cf32_le_bytes_to_complex(bytes),
        }
    }
}

pub struct FileReplaySource {
    reader: BufReader<File>,
    format: ReplayFormat,
    frequency_hz: u64,
    sample_rate_hz: u32,
    gain_db: f64,
    looping: bool,
    speed: f32,
    running: bool,
    started_at: Option<Instant>,
    samples_emitted: u64,
    byte_buf: Vec<u8>,
}

impl FileReplaySource {
    pub fn open(
        path: &str,
        format: ReplayFormat,
        sample_rate_hz: u32,
        looping: bool,
        speed: f32,
    ) -> Result<Self> {
        if sample_rate_hz == 0 {
            anyhow::bail!("replay sample rate must be non-zero");
        }
        let file = File::open(path).with_context(|| format!("opening replay file {path}"))?;
        Ok(Self {
            reader: BufReader::new(file),
            format,
            frequency_hz: 0,
            sample_rate_hz,
            gain_db: 0.0,
            looping,
            speed: speed.max(0.01),
            running: false,
            started_at: None,
            samples_emitted: 0,
            byte_buf: Vec::new(),
        })
    }

    /// Blocks until wall-clock time has "caught up" to how many samples we've emitted so
    /// far at the configured playback speed, so a replay behaves like a live capture
    /// instead of dumping the whole file as fast as disk I/O allows.
    fn pace(&mut self) {
        // Defensive: rate/speed are validated on set, but a zero here (e.g. a
        // source constructed with rate 0 before validation existed) must pace
        // at full speed, never `Duration::from_secs_f64(inf)`-panic.
        let rate = (self.sample_rate_hz as f64).max(1.0);
        let speed = (self.speed as f64).max(0.01);
        let started_at = *self.started_at.get_or_insert_with(Instant::now);
        let expected_elapsed = Duration::from_secs_f64(self.samples_emitted as f64 / rate / speed);
        let actual_elapsed = started_at.elapsed();
        if expected_elapsed > actual_elapsed {
            std::thread::sleep(expected_elapsed - actual_elapsed);
        }
    }
}

impl IqSource for FileReplaySource {
    fn start(&mut self) -> Result<()> {
        self.running = true;
        self.started_at = Some(Instant::now());
        self.samples_emitted = 0;
        Ok(())
    }

    fn stop(&mut self) {
        self.running = false;
    }

    fn set_frequency(&mut self, hz: u64) -> Result<()> {
        self.frequency_hz = hz;
        Ok(())
    }

    fn set_sample_rate(&mut self, hz: u32) -> Result<()> {
        if hz == 0 {
            anyhow::bail!("replay sample rate must be non-zero");
        }
        self.sample_rate_hz = hz;
        Ok(())
    }

    fn set_gain(&mut self, db: f64) -> Result<()> {
        self.gain_db = db;
        Ok(())
    }

    fn read_iq(&mut self, buf: &mut [Complex32]) -> Result<usize> {
        let want_bytes = buf.len() * self.format.bytes_per_sample();
        self.byte_buf.resize(want_bytes, 0);

        let mut filled = 0;
        // Tracks a rewind that produced no data: looping an empty file would
        // otherwise seek+read 0 bytes forever at 100% CPU.
        let mut stalled = false;
        while filled < want_bytes {
            match self.reader.read(&mut self.byte_buf[filled..])? {
                0 => {
                    if self.looping {
                        if stalled {
                            break;
                        }
                        self.reader.seek(SeekFrom::Start(0))?;
                        stalled = true;
                        continue;
                    }
                    break;
                }
                n => {
                    filled += n;
                    stalled = false;
                }
            }
        }
        if filled == 0 {
            return Ok(0);
        }

        let complex = self.format.convert(&self.byte_buf[..filled]);
        let n = complex.len().min(buf.len());
        buf[..n].copy_from_slice(&complex[..n]);
        self.samples_emitted += n as u64;
        self.pace();
        Ok(n)
    }

    fn frequency_hz(&self) -> u64 {
        self.frequency_hz
    }

    fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz
    }

    fn gain_db(&self) -> f64 {
        self.gain_db
    }

    fn kind(&self) -> &'static str {
        "replay"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn replays_cf32_file_and_loops_on_eof() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "ez_daemon_replay_test_{:x}.cf32",
            std::process::id()
        ));
        {
            let mut f = File::create(&path).unwrap();
            for i in 0..16 {
                let iq = (i as f32, -(i as f32));
                f.write_all(&iq.0.to_le_bytes()).unwrap();
                f.write_all(&iq.1.to_le_bytes()).unwrap();
            }
        }

        let mut src = FileReplaySource::open(
            path.to_str().unwrap(),
            ReplayFormat::Cf32Le,
            1_000_000,
            true,
            1000.0,
        )
        .unwrap();
        src.start().unwrap();
        let mut buf = vec![Complex32::new(0.0, 0.0); 16];
        let n = src.read_iq(&mut buf).unwrap();
        assert_eq!(n, 16);
        assert_eq!(buf[0], Complex32::new(0.0, 0.0));
        assert_eq!(buf[1], Complex32::new(1.0, -1.0));

        // Second read must loop back to the start rather than returning 0.
        let n2 = src.read_iq(&mut buf).unwrap();
        assert_eq!(n2, 16);
        assert_eq!(buf[0], Complex32::new(0.0, 0.0));

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn looping_empty_file_reports_eof_without_spinning() {
        // Looping an empty file must report EOF, not seek+read 0 bytes
        // forever at 100% CPU (a spin would hang the test suite).
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "ez_daemon_replay_empty_{:x}.cf32",
            std::process::id()
        ));
        File::create(&path).unwrap();
        let mut src = FileReplaySource::open(
            path.to_str().unwrap(),
            ReplayFormat::Cf32Le,
            1_000_000,
            true,
            1000.0,
        )
        .unwrap();
        src.start().unwrap();
        let mut buf = vec![Complex32::new(0.0, 0.0); 4];
        assert_eq!(src.read_iq(&mut buf).unwrap(), 0);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn zero_sample_rate_is_rejected() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "ez_daemon_replay_rate_{:x}.cf32",
            std::process::id()
        ));
        File::create(&path).unwrap();
        assert!(FileReplaySource::open(
            path.to_str().unwrap(),
            ReplayFormat::Cf32Le,
            0,
            false,
            1.0
        )
        .is_err());
        let mut src = FileReplaySource::open(
            path.to_str().unwrap(),
            ReplayFormat::Cf32Le,
            1_000_000,
            false,
            1.0,
        )
        .unwrap();
        assert!(src.set_sample_rate(0).is_err());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn non_looping_replay_reports_eof() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "ez_daemon_replay_eof_{:x}.cf32",
            std::process::id()
        ));
        {
            let mut f = File::create(&path).unwrap();
            f.write_all(&0f32.to_le_bytes()).unwrap();
            f.write_all(&0f32.to_le_bytes()).unwrap();
        }

        let mut src = FileReplaySource::open(
            path.to_str().unwrap(),
            ReplayFormat::Cf32Le,
            1_000_000,
            false,
            1000.0,
        )
        .unwrap();
        src.start().unwrap();
        let mut buf = vec![Complex32::new(0.0, 0.0); 4];
        let n = src.read_iq(&mut buf).unwrap();
        assert_eq!(n, 1);
        let n2 = src.read_iq(&mut buf).unwrap();
        assert_eq!(n2, 0);

        std::fs::remove_file(&path).ok();
    }
}
