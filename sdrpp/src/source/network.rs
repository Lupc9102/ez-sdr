//! Network IQ source implementing the rtl_tcp protocol.
//! Translated from `source_modules/rtl_tcp_source`.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{bail, Result};
use crossbeam_channel::{bounded, Receiver, Sender, TryRecvError};
use num_complex::Complex;

/// Number of sample blocks to buffer in the channel.
const CHANNEL_DEPTH: usize = 16;

/// Default block size in samples (matches original `2400000 / 200`).
const DEFAULT_BLOCK_SAMPLES: usize = 12_000;

/// IQ sample type used by the DSP pipeline.
pub type Sample = Complex<f32>;

/// rtl_tcp command codes.
#[repr(u8)]
enum Command {
    SetFrequency = 1,
    SetSampleRate = 2,
    SetGainMode = 3,
    SetGain = 4,
    SetPpm = 5,
    SetAgcMode = 8,
    SetDirectSampling = 9,
    SetOffsetTuning = 10,
    SetGainIndex = 13,
    SetBiasTee = 14,
}

/// Network IQ source implementing the rtl_tcp protocol.
///
/// Connects to an rtl_tcp server, sends configuration commands,
/// and streams received IQ samples through a [`crossbeam_channel::Receiver`].
pub struct RtlTcpSource {
    stream: TcpStream,
    running: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    tx: Sender<Vec<Sample>>,
    rx: Receiver<Vec<Sample>>,
    frequency: f64,
    sample_rate: f64,
    block_samples: usize,
}

impl RtlTcpSource {
    /// Connect to an rtl_tcp server.
    ///
    /// `addr` may be any string accepted by [`TcpStream::connect`]
    /// (e.g. `"localhost:1234"`).
    pub fn connect(addr: &str) -> Result<Self> {
        let stream = TcpStream::connect(addr)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_nodelay(true)?;

        let (tx, rx) = bounded(CHANNEL_DEPTH);

        Ok(Self {
            stream,
            running: Arc::new(AtomicBool::new(false)),
            worker: None,
            tx,
            rx,
            frequency: 0.0,
            sample_rate: 0.0,
            block_samples: DEFAULT_BLOCK_SAMPLES,
        })
    }

    /// Start the background receiver thread.
    pub fn start(&mut self) -> Result<()> {
        if self.worker.is_some() {
            bail!("source already running");
        }

        self.running.store(true, Ordering::SeqCst);

        let mut stream = self.stream.try_clone()?;
        let tx = self.tx.clone();
        let running = Arc::clone(&self.running);
        let block = self.block_samples;

        let handle = thread::Builder::new()
            .name("rtl_tcp".into())
            .spawn(move || {
                if let Err(e) = worker_main(&mut stream, &tx, &running, block) {
                    eprintln!("[RtlTcpSource] worker error: {}", e);
                }
            })?;

        self.worker = Some(handle);
        Ok(())
    }

    /// Shut down the receiver thread and close the TCP connection.
    pub fn close(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        let _ = self.stream.shutdown(Shutdown::Both);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }

    /// Return `true` if the receiver thread is running.
    pub fn is_running(&self) -> bool {
        self.worker.is_some()
    }

    /// Receive the next block of samples (blocking).
    pub fn recv(&self) -> Option<Vec<Sample>> {
        self.rx.recv().ok()
    }

    /// Try to receive the next block of samples without blocking.
    pub fn try_recv(&self) -> Result<Vec<Sample>, TryRecvError> {
        self.rx.try_recv()
    }

    /// Return a reference to the internal sample receiver.
    pub fn receiver(&self) -> &Receiver<Vec<Sample>> {
        &self.rx
    }

    // -- Tuning & control commands ----------------------------------------

    /// Set center frequency in Hz.
    pub fn set_frequency(&mut self, hz: u32) -> io::Result<()> {
        self.frequency = f64::from(hz);
        self.send_command(Command::SetFrequency, hz)
    }

    /// Set sample rate in samples per second.
    ///
    /// This also updates the internal block size used by the
    /// receiver thread (`sr / 200.0`).
    pub fn set_sample_rate(&mut self, sps: u32) -> io::Result<()> {
        self.sample_rate = f64::from(sps);
        self.block_samples = (self.sample_rate / 200.0).max(1.0) as usize;
        self.send_command(Command::SetSampleRate, sps)
    }

    /// Set gain mode (0 = auto, 1 = manual).
    pub fn set_gain_mode(&mut self, mode: u32) -> io::Result<()> {
        self.send_command(Command::SetGainMode, mode)
    }

    /// Set tuner gain in tenths of a dB.
    pub fn set_gain(&mut self, gain: u32) -> io::Result<()> {
        self.send_command(Command::SetGain, gain)
    }

    /// Set frequency correction in parts-per-million.
    pub fn set_ppm(&mut self, ppm: u32) -> io::Result<()> {
        self.send_command(Command::SetPpm, ppm)
    }

    /// Set RTL AGC mode (0 = off, 1 = on).
    pub fn set_agc_mode(&mut self, mode: u32) -> io::Result<()> {
        self.send_command(Command::SetAgcMode, mode)
    }

    /// Set direct sampling mode (0 = disabled, 1 = I, 2 = Q).
    pub fn set_direct_sampling(&mut self, mode: u32) -> io::Result<()> {
        self.send_command(Command::SetDirectSampling, mode)
    }

    /// Enable or disable offset tuning.
    pub fn set_offset_tuning(&mut self, enabled: bool) -> io::Result<()> {
        self.send_command(Command::SetOffsetTuning, enabled as u32)
    }

    /// Set gain by table index.
    pub fn set_gain_index(&mut self, index: u32) -> io::Result<()> {
        self.send_command(Command::SetGainIndex, index)
    }

    /// Enable or disable the bias tee.
    pub fn set_bias_tee(&mut self, enabled: bool) -> io::Result<()> {
        self.send_command(Command::SetBiasTee, enabled as u32)
    }

    // -- Helpers ---------------------------------------------------------

    fn send_command(&mut self, cmd: Command, param: u32) -> io::Result<()> {
        let mut buf = [0u8; 5];
        buf[0] = cmd as u8;
        buf[1..5].copy_from_slice(&param.to_be_bytes());
        self.stream.write_all(&buf)
    }
}

impl Drop for RtlTcpSource {
    fn drop(&mut self) {
        self.close();
    }
}

/// Background worker: reads raw bytes from the TCP stream,
/// converts them to [`Sample`] blocks, and publishes them
/// over the supplied channel.
fn worker_main(
    stream: &mut TcpStream,
    tx: &Sender<Vec<Sample>>,
    running: &AtomicBool,
    block_samples: usize,
) -> io::Result<()> {
    let mut raw = vec![0u8; block_samples * 2];

    while running.load(Ordering::SeqCst) {
        if let Err(e) = stream.read_exact(&mut raw) {
            if !running.load(Ordering::SeqCst) {
                return Ok(());
            }
            return Err(e);
        }

        let samples = convert_u8iq(&raw);
        if tx.send(samples).is_err() {
            break; // receiver dropped
        }
    }

    Ok(())
}

/// Convert interleaved unsigned IQ bytes to normalized complex floats.
///
/// Each byte pair `(I, Q)` is converted to:
/// `((I - 128.0) / 128.0, (Q - 128.0) / 128.0)`
fn convert_u8iq(data: &[u8]) -> Vec<Sample> {
    let count = data.len() / 2;
    let mut out = Vec::with_capacity(count);

    for i in 0..count {
        let re = (data[i * 2] as f32 - 128.0) / 128.0;
        let im = (data[i * 2 + 1] as f32 - 128.0) / 128.0;
        out.push(Sample::new(re, im));
    }

    out
}
