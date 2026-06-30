//! File input source - translated from sdr_ifile.c

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
            Source::Stdin => Err(io::Error::new(io::ErrorKind::Other, "cannot seek stdin")),
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
            IqFormat::Sc16Q11 => convert::convert_sc16q11_to_mag(&self.read_buf, &mut out[..samples]),
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
        while total < buf.len() {
            let remaining = buf.len() - total;
            let (samples, eof) = self.read_raw(remaining)?;
            self.convert(samples, &mut buf[total..total + samples]);
            total += samples;

            if eof {
                if self.loop_file && self.path != "-" {
                    if let Some(ref mut s) = self.source {
                        let _ = s.seek(SeekFrom::Start(0));
                        continue;
                    }
                }
                break;
            }
        }
        Ok(total)
    }
}
