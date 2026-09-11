//! File input source - translated from `sdr_ifile.c`

use crate::convert::{self, IqFormat};
use crate::sdr::SdrSource;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

enum Source {
    File(File),
    Stdin,
}

impl Read for Source {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Source::File(f) => f.read(buf),
            Source::Stdin => io::stdin().read(buf),
        }
    }
}

impl Seek for Source {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        match self {
            Source::File(f) => f.seek(pos),
            Source::Stdin => Err(io::Error::other("cannot seek stdin")),
        }
    }
}

pub struct IFileSdr {
    source: Option<Source>,
    path: String,
    format: IqFormat,
    bytes_per_sample: usize,
    read_buf: Vec<u8>,
    loop_file: bool,
    frequency: u64,
    sample_rate: u32,
    gain: f64,
}

impl IFileSdr {
    pub fn new(path: impl AsRef<str>, format: IqFormat, loop_file: bool) -> Self {
        let bytes_per_sample = match format {
            IqFormat::Uc8 => 2,
            IqFormat::Sc16 | IqFormat::Sc16Q11 => 4,
        };

        IFileSdr {
            source: None,
            path: path.as_ref().to_string(),
            format,
            bytes_per_sample,
            read_buf: Vec::new(),
            loop_file,
            frequency: 1_090_000_000,
            sample_rate: 2_000_000,
            gain: 0.0,
        }
    }

    pub fn set_loop(&mut self, loop_file: bool) {
        self.loop_file = loop_file;
    }

    fn open(&mut self) -> anyhow::Result<()> {
        if self.source.is_some() {
            return Ok(());
        }
        let src = if self.path == "-" {
            Source::Stdin
        } else {
            Source::File(File::open(Path::new(&self.path))?)
        };
        self.source = Some(src);
        Ok(())
    }

    fn read_raw(&mut self, want_samples: usize) -> io::Result<(usize, bool)> {
        let want_bytes = want_samples * self.bytes_per_sample;
        if self.read_buf.len() < want_bytes {
            self.read_buf.resize(want_bytes, 0);
        }

        let src = match self.source.as_mut() {
            Some(s) => s,
            None => return Ok((0, true)),
        };

        let mut total = 0usize;
        while total < want_bytes {
            let n = src.read(&mut self.read_buf[total..want_bytes])?;
            if n == 0 {
                return Ok((total / self.bytes_per_sample, true));
            }
            total += n;
        }
        Ok((want_samples, false))
    }

    fn convert(&self, samples: usize, out: &mut [u16]) {
        assert!(out.len() >= samples);
        match self.format {
            IqFormat::Uc8 => convert::convert_uc8_to_mag(&self.read_buf, &mut out[..samples]),
            IqFormat::Sc16 => convert::convert_sc16_to_mag(&self.read_buf, &mut out[..samples]),
            IqFormat::Sc16Q11 => {
                convert::convert_sc16q11_to_mag(&self.read_buf, &mut out[..samples]);
            }
        }
    }
}

impl SdrSource for IFileSdr {
    fn start(&mut self) -> anyhow::Result<()> {
        self.open()
    }

    fn stop(&mut self) {
        self.source = None;
    }

    fn set_frequency(&mut self, freq: u64) -> anyhow::Result<()> {
        self.frequency = freq;
        Ok(())
    }

    fn set_sample_rate(&mut self, rate: u32) -> anyhow::Result<()> {
        self.sample_rate = rate;
        Ok(())
    }

    fn set_gain(&mut self, gain: f64) -> anyhow::Result<()> {
        self.gain = gain;
        Ok(())
    }

    fn read_samples(&mut self, buf: &mut [u16]) -> anyhow::Result<usize> {
        self.open()?;

        let mut total = 0usize;
        // Tracks a rewind that produced no data: an empty input in loop mode
        // would otherwise re-seek and re-read `(0, eof)` forever at 100% CPU.
        let mut stalled = false;
        while total < buf.len() {
            let remaining = buf.len() - total;
            let (samples, eof) = self.read_raw(remaining)?;
            self.convert(samples, &mut buf[total..total + samples]);
            total += samples;

            if eof {
                if self.loop_file && self.path != "-" {
                    if samples == 0 && stalled {
                        // Rewound already and still no data: empty input.
                        break;
                    }
                    if let Some(ref mut s) = self.source {
                        let _ = s.seek(SeekFrom::Start(0));
                        stalled = samples == 0;
                        continue;
                    }
                }
                break;
            }
            stalled = false;
        }
        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::AtomicU32;

    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    fn create_temp_file(data: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir();
        let n = TEST_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = dir.join(format!("ifile_test_{}_{}", std::process::id(), n));
        let mut f = std::fs::File::create(&path).expect("temp file creation should succeed");
        f.write_all(data).expect("temp file write should succeed");
        path
    }

    fn path_to_str(path: &std::path::Path) -> &str {
        path.to_str().expect("temp path is valid UTF-8")
    }

    fn cleanup(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn ifile_new_with_path() {
        let path = create_temp_file(b"test");
        let sdr = IFileSdr::new(path_to_str(&path), IqFormat::Uc8, false);
        assert_eq!(sdr.path, path_to_str(&path));
        assert_eq!(sdr.format, IqFormat::Uc8);
        assert_eq!(sdr.bytes_per_sample, 2);
        assert!(!sdr.loop_file);
        assert!(sdr.source.is_none());
        assert_eq!(sdr.frequency, 1_090_000_000);
        assert_eq!(sdr.sample_rate, 2_000_000);
        cleanup(&path);
    }

    #[test]
    fn ifile_new_with_stdin() {
        let sdr = IFileSdr::new("-", IqFormat::Sc16, true);
        assert_eq!(sdr.path, "-");
        assert_eq!(sdr.format, IqFormat::Sc16);
        assert_eq!(sdr.bytes_per_sample, 4);
        assert!(sdr.loop_file);
        assert!(sdr.source.is_none());
    }

    #[test]
    fn ifile_new_sc16q11_bytes_per_sample() {
        let sdr = IFileSdr::new("dummy", IqFormat::Sc16Q11, false);
        assert_eq!(sdr.bytes_per_sample, 4);
    }

    #[test]
    fn ifile_read_uc8_samples() {
        let data: Vec<u8> = vec![
            127, 127, // sample 0: both near center -> small magnitude
            200, 150, // sample 1
            255, 128, // sample 2
        ];
        let path = create_temp_file(&data);
        let mut sdr = IFileSdr::new(path_to_str(&path), IqFormat::Uc8, false);
        let mut buf = [0u16; 4];
        let n = sdr
            .read_samples(&mut buf)
            .expect("read_samples should succeed");
        assert_eq!(n, 3);

        // Compute expected: fi=(127-127.4)=-0.4, fq=(127-127.4)=-0.4, mag~sqrt(0.32)*512~289
        let fi0 = f32::from(127u8) - 127.4f32;
        let fq0 = f32::from(127u8) - 127.4f32;
        let expected0 = ((fi0 * fi0 + fq0 * fq0).sqrt() * 512.0).min(65535.0) as u16;
        assert!(
            (buf[0] as f64 - expected0 as f64).abs() <= 1.0,
            "expected {expected0}, got {}",
            buf[0]
        );

        // Read another attempt at EOF -> 0 samples
        let n2 = sdr
            .read_samples(&mut buf)
            .expect("read_samples should succeed");
        assert_eq!(n2, 0);

        cleanup(&path);
    }

    #[test]
    fn ifile_read_sc16_samples() {
        // SC16: I=100 (0x0064), Q=0 => little endian [100, 0, 0, 0]
        // magnitude = sqrt(100^2 + 0^2) * 2 = 200
        let data: Vec<u8> = vec![100, 0, 0, 0, 0, 0, 0, 0];
        let path = create_temp_file(&data);
        let mut sdr = IFileSdr::new(path_to_str(&path), IqFormat::Sc16, false);
        let mut buf = [0u16; 4];
        let n = sdr
            .read_samples(&mut buf)
            .expect("read_samples should succeed");
        assert_eq!(n, 2);
        assert_eq!(buf[0], 200);
        assert_eq!(buf[1], 0);
        cleanup(&path);
    }

    #[test]
    fn ifile_read_sc16q11_samples() {
        // SC16Q11: I=100, Q=0 => magnitude = sqrt(100^2 + 0^2) * 32 = 3200
        let data: Vec<u8> = vec![100, 0, 0, 0];
        let path = create_temp_file(&data);
        let mut sdr = IFileSdr::new(path_to_str(&path), IqFormat::Sc16Q11, false);
        let mut buf = [0u16; 4];
        let n = sdr
            .read_samples(&mut buf)
            .expect("read_samples should succeed");
        assert_eq!(n, 1);
        assert_eq!(buf[0], 3200);
        cleanup(&path);
    }

    #[test]
    fn ifile_eof_returns_zero() {
        let data = [200u8, 200];
        let path = create_temp_file(&data);
        let mut sdr = IFileSdr::new(path_to_str(&path), IqFormat::Uc8, false);
        let mut buf = [0u16; 10];
        let n = sdr
            .read_samples(&mut buf)
            .expect("read_samples should succeed");
        assert_eq!(n, 1);
        let n2 = sdr
            .read_samples(&mut buf)
            .expect("read_samples should succeed");
        assert_eq!(n2, 0);
        cleanup(&path);
    }

    #[test]
    fn ifile_loop_rewinds() {
        let data = [200u8, 200, 210, 210];
        let path = create_temp_file(&data);
        let mut sdr = IFileSdr::new(path_to_str(&path), IqFormat::Uc8, false);
        sdr.set_loop(true);
        // Read exactly the file size (2 samples) into a small buffer
        let mut buf = [0u16; 2];
        let n1 = sdr
            .read_samples(&mut buf)
            .expect("read_samples should succeed");
        assert_eq!(n1, 2);
        let first_pass = buf.to_vec();
        // Read again — loop should rewind and yield the same 2 samples
        let n2 = sdr
            .read_samples(&mut buf)
            .expect("read_samples should succeed");
        assert_eq!(n2, 2);
        assert_eq!(buf.to_vec(), first_pass);
        cleanup(&path);
    }

    #[test]
    fn ifile_loop_empty_file_returns_eof_without_spinning() {
        // Empty input in loop mode must report EOF, not seek+read (0, eof)
        // forever at 100% CPU. Bounded by completion alone (a spin would
        // hang the test suite); the Ok(0) also keeps `main` exiting cleanly.
        let path = create_temp_file(&[]);
        let mut sdr = IFileSdr::new(path_to_str(&path), IqFormat::Uc8, true);
        let mut buf = [0u16; 8];
        let n = sdr
            .read_samples(&mut buf)
            .expect("read_samples should succeed");
        assert_eq!(n, 0);
        cleanup(&path);
    }

    #[test]
    fn ifile_set_loop_toggle() {
        let mut sdr = IFileSdr::new("dummy", IqFormat::Uc8, false);
        assert!(!sdr.loop_file);
        sdr.set_loop(true);
        assert!(sdr.loop_file);
        sdr.set_loop(false);
        assert!(!sdr.loop_file);
    }

    #[test]
    fn ifile_read_multiple_chunks() {
        // 4 samples of UC8
        let data: Vec<u8> = (0u8..8).collect();
        let path = create_temp_file(&data);
        let mut sdr = IFileSdr::new(path_to_str(&path), IqFormat::Uc8, false);
        let mut buf = [0u16; 2];
        let n1 = sdr
            .read_samples(&mut buf)
            .expect("read_samples should succeed");
        assert_eq!(n1, 2);
        let n2 = sdr
            .read_samples(&mut buf)
            .expect("read_samples should succeed");
        assert_eq!(n2, 2);
        let n3 = sdr
            .read_samples(&mut buf)
            .expect("read_samples should succeed");
        assert_eq!(n3, 0);
        cleanup(&path);
    }
}
