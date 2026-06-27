use crate::dsp::{Stream, UntypedStream};
use num_complex::Complex32;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

#[repr(C)]
struct WavHeader {
    signature: [u8; 4],
    file_size: u32,
    file_type: [u8; 4],
    format_marker: [u8; 4],
    format_header_length: u32,
    sample_type: u16,
    channel_count: u16,
    sample_rate: u32,
    bytes_per_second: u32,
    bytes_per_sample: u16,
    bit_depth: u16,
    data_marker: [u8; 4],
    data_size: u32,
}

pub struct WavReader {
    file: File,
    header: WavHeader,
    valid: bool,
    bytes_read: u64,
}

impl WavReader {
    pub fn open(path: &str) -> std::io::Result<Self> {
        let mut file = File::open(path)?;
        let mut hdr_buf = [0u8; 44];
        file.read_exact(&mut hdr_buf)?;
        let header = unsafe { std::ptr::read(hdr_buf.as_ptr() as *const WavHeader) };
        let valid = &header.signature == b"RIFF" && &header.file_type == b"WAVE";
        Ok(Self {
            file,
            header,
            valid,
            bytes_read: 0,
        })
    }

    pub fn is_valid(&self) -> bool {
        self.valid
    }

    pub fn sample_rate(&self) -> u32 {
        self.header.sample_rate
    }

    pub fn channel_count(&self) -> u16 {
        self.header.channel_count
    }

    pub fn bit_depth(&self) -> u16 {
        self.header.bit_depth
    }

    pub fn read_samples(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let n = self.file.read(out)?;
        if n < out.len() {
            self.file.seek(SeekFrom::Start(44))?;
            let rem = self.file.read(&mut out[n..])?;
            self.bytes_read = (n + rem) as u64;
            Ok(n + rem)
        } else {
            self.bytes_read += n as u64;
            Ok(n)
        }
    }

    pub fn rewind(&mut self) -> std::io::Result<()> {
        self.file.seek(SeekFrom::Start(44))?;
        self.bytes_read = 0;
        Ok(())
    }
}

pub struct FileSource {
    stream: Arc<Stream<Complex32>>,
    reader: Arc<std::sync::Mutex<Option<WavReader>>>,
    running: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    float32_mode: bool,
    sample_rate: f32,
    center_freq: f64,
}

impl FileSource {
    pub fn new() -> Self {
        Self {
            stream: Arc::new(Stream::new()),
            reader: Arc::new(std::sync::Mutex::new(None)),
            running: Arc::new(AtomicBool::new(false)),
            worker: None,
            float32_mode: false,
            sample_rate: 1_000_000.0,
            center_freq: 100_000_000.0,
        }
    }

    pub fn stream(&self) -> Arc<Stream<Complex32>> {
        Arc::clone(&self.stream)
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    pub fn center_frequency(&self) -> f64 {
        self.center_freq
    }

    pub fn set_float32_mode(&mut self, enabled: bool) {
        self.float32_mode = enabled;
    }

    pub fn float32_mode(&self) -> bool {
        self.float32_mode
    }

    pub fn open(&mut self, path: &str) -> std::io::Result<()> {
        let reader = WavReader::open(path)?;
        if !reader.is_valid() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid wav"));
        }
        let sr = reader.sample_rate();
        if sr == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "sample rate zero"));
        }
        self.sample_rate = sr as f32;
        self.center_freq = Self::extract_frequency(path);
        let mut guard = self.reader.lock().unwrap();
        *guard = Some(reader);
        Ok(())
    }

    pub fn start(&mut self) {
        if self.running.load(Ordering::SeqCst) {
            return;
        }
        if self.reader.lock().unwrap().is_none() {
            return;
        }
        self.running.store(true, Ordering::SeqCst);
        let reader = Arc::clone(&self.reader);
        let stream = Arc::clone(&self.stream);
        let running = Arc::clone(&self.running);
        let float32 = self.float32_mode;
        let sr = self.sample_rate;
        self.worker = Some(thread::spawn(move || {
            worker_loop(reader, stream, running, float32, sr);
        }));
    }

    pub fn stop(&mut self) {
        if !self.running.load(Ordering::SeqCst) {
            return;
        }
        self.running.store(false, Ordering::SeqCst);
        self.stream.stop_writer();
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
        self.stream.clear_writer_stop();
        if let Ok(mut guard) = self.reader.lock() {
            if let Some(ref mut r) = *guard {
                let _ = r.rewind();
            }
        }
    }

    fn extract_frequency(path: &str) -> f64 {
        let bytes = path.as_bytes();
        let mut start = None;
        let mut end = None;
        for (i, &b) in bytes.iter().enumerate() {
            if b.is_ascii_digit() {
                if start.is_none() {
                    start = Some(i);
                }
                end = Some(i + 1);
            } else if start.is_some() {
                if i + 2 <= bytes.len() && &bytes[i..i + 2] == b"Hz" {
                    end = Some(i + 2);
                    break;
                }
                start = None;
                end = None;
            }
        }
        if let (Some(s), Some(e)) = (start, end) {
            if e >= s + 2 && &bytes[e - 2..e] == b"Hz" {
                let num_str = std::str::from_utf8(&bytes[s..e - 2]).unwrap_or("");
                num_str.parse().unwrap_or(0.0)
            } else {
                0.0
            }
        } else {
            0.0
        }
    }
}

impl Default for FileSource {
    fn default() -> Self {
        Self::new()
    }
}

fn worker_loop(
    reader: Arc<std::sync::Mutex<Option<WavReader>>>,
    stream: Arc<Stream<Complex32>>,
    running: Arc<AtomicBool>,
    float32: bool,
    sample_rate: f32,
) {
    let block_size = ((sample_rate / 200.0) as usize).max(1).min(1_000_000);
    if float32 {
        let mut buf = vec![0u8; block_size * std::mem::size_of::<Complex32>()];
        while running.load(Ordering::SeqCst) {
            let n = {
                let mut guard = reader.lock().unwrap();
                let r = match guard.as_mut() {
                    Some(r) => r,
                    None => break,
                };
                match r.read_samples(&mut buf) {
                    Ok(n) => n,
                    Err(_) => break,
                }
            };
            if n == 0 {
                continue;
            }
            let samples = n / std::mem::size_of::<Complex32>();
            let mut write = stream.write_buf();
            let src = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const Complex32, samples) };
            write[..samples].copy_from_slice(src);
            drop(write);
            if !stream.swap(samples) {
                break;
            }
        }
    } else {
        let mut buf = vec![0u8; block_size * 2 * std::mem::size_of::<i16>()];
        while running.load(Ordering::SeqCst) {
            let n = {
                let mut guard = reader.lock().unwrap();
                let r = match guard.as_mut() {
                    Some(r) => r,
                    None => break,
                };
                match r.read_samples(&mut buf) {
                    Ok(n) => n,
                    Err(_) => break,
                }
            };
            if n == 0 {
                continue;
            }
            let sample_pairs = n / (2 * std::mem::size_of::<i16>());
            let mut write = stream.write_buf();
            let src = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const i16, sample_pairs * 2) };
            for i in 0..sample_pairs {
                let re = src[i * 2] as f32 / 32768.0;
                let im = src[i * 2 + 1] as f32 / 32768.0;
                write[i] = Complex32::new(re, im);
            }
            drop(write);
            if !stream.swap(sample_pairs) {
                break;
            }
        }
    }
}
