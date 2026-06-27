//! Network source - translated from plugins/sdr_sources/net_source_support

use super::Source;
use anyhow::{anyhow, Result};
use num_complex::Complex32;
use std::net::UdpSocket;

/// Network source operating mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetMode {
    Udp,
    NngSub,
}

/// Network sample source implementing the common `Source` trait.
pub struct NetSource {
    mode: NetMode,
    address: String,
    port: u16,
    sample_rate: u64,
    frequency: u64,
    udp_socket: Option<UdpSocket>,
    nng_socket: Option<nng::Socket>,
}

impl NetSource {
    /// Create a new network source with defaults.
    pub fn new() -> Self {
        Self {
            mode: NetMode::Udp,
            address: "localhost".to_string(),
            port: 8877,
            sample_rate: 0,
            frequency: 0,
            udp_socket: None,
            nng_socket: None,
        }
    }

    /// Set the operating mode (UDP or NNG Subscriber).
    pub fn set_mode(&mut self, mode: NetMode) {
        self.mode = mode;
    }

    /// Set the remote address.
    pub fn set_address(&mut self, addr: String) {
        self.address = addr;
    }

    /// Set the port.
    pub fn set_port(&mut self, port: u16) {
        self.port = port;
    }
}

impl Default for NetSource {
    fn default() -> Self {
        Self::new()
    }
}

impl Source for NetSource {
    fn open(&mut self) -> Result<()> {
        match self.mode {
            NetMode::Udp => {
                let socket = UdpSocket::bind(("0.0.0.0", self.port))?;
                self.udp_socket = Some(socket);
            }
            NetMode::NngSub => {
                let socket = nng::Socket::new(nng::Protocol::Sub0)?;
                let url = format!("tcp://{}:{}", self.address, self.port);
                socket.dial(&url)?;
                self.nng_socket = Some(socket);
            }
        }
        Ok(())
    }

    fn set_freq(&mut self, freq: u64) -> Result<()> {
        self.frequency = freq;
        Ok(())
    }

    fn set_sample_rate(&mut self, sr: u32) -> Result<()> {
        self.sample_rate = sr as u64;
        Ok(())
    }

    fn read_samples(&mut self, buf: &mut [Complex32]) -> Result<usize> {
        let sample_bytes = std::mem::size_of::<Complex32>();
        match self.mode {
            NetMode::Udp => {
                let socket = self
                    .udp_socket
                    .as_ref()
                    .ok_or_else(|| anyhow!("UDP socket not open"))?;
                let byte_len = buf.len() * sample_bytes;
                let mut tmp = vec![0u8; byte_len];
                let (n, _) = socket.recv_from(&mut tmp)?;
                let nsamples = n / sample_bytes;

                for i in 0..nsamples {
                    let offset = i * sample_bytes;
                    let re = f32::from_le_bytes([
                        tmp[offset],
                        tmp[offset + 1],
                        tmp[offset + 2],
                        tmp[offset + 3],
                    ]);
                    let im = f32::from_le_bytes([
                        tmp[offset + 4],
                        tmp[offset + 5],
                        tmp[offset + 6],
                        tmp[offset + 7],
                    ]);
                    buf[i] = Complex32::new(re, im);
                }
                Ok(nsamples)
            }
            NetMode::NngSub => {
                let socket = self
                    .nng_socket
                    .as_ref()
                    .ok_or_else(|| anyhow!("NNG socket not open"))?;
                let msg = socket.recv()?;
                let nsamples = msg.len() / sample_bytes;
                let bytes: &[u8] = &msg;

                for i in 0..nsamples {
                    let offset = i * sample_bytes;
                    let re = f32::from_le_bytes([
                        bytes[offset],
                        bytes[offset + 1],
                        bytes[offset + 2],
                        bytes[offset + 3],
                    ]);
                    let im = f32::from_le_bytes([
                        bytes[offset + 4],
                        bytes[offset + 5],
                        bytes[offset + 6],
                        bytes[offset + 7],
                    ]);
                    buf[i] = Complex32::new(re, im);
                }
                Ok(nsamples)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// File source (raw interleaved f32 IQ)
// ---------------------------------------------------------------------------

pub struct FileSource {
    path: std::path::PathBuf,
    file: Option<std::fs::File>,
}

impl FileSource {
    pub fn new(path: impl AsRef<std::path::Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            file: None,
        }
    }
}

impl Default for FileSource {
    fn default() -> Self {
        Self::new("input.iq")
    }
}

impl Source for FileSource {
    fn open(&mut self) -> Result<()> {
        self.file = Some(std::fs::File::open(&self.path)?);
        Ok(())
    }

    fn set_freq(&mut self, _freq: u64) -> Result<()> {
        Ok(())
    }

    fn set_sample_rate(&mut self, _sr: u32) -> Result<()> {
        Ok(())
    }

    fn read_samples(&mut self, buf: &mut [Complex32]) -> Result<usize> {
        use std::io::Read;
        let file = self.file.as_mut().ok_or_else(|| anyhow!("File not open"))?;
        let sample_bytes = std::mem::size_of::<Complex32>();
        let byte_len = buf.len() * sample_bytes;
        let mut tmp = vec![0u8; byte_len];
        let n = file.read(&mut tmp)?;
        let nsamples = n / sample_bytes;
        for i in 0..nsamples {
            let offset = i * sample_bytes;
            let re = f32::from_le_bytes([
                tmp[offset],
                tmp[offset + 1],
                tmp[offset + 2],
                tmp[offset + 3],
            ]);
            let im = f32::from_le_bytes([
                tmp[offset + 4],
                tmp[offset + 5],
                tmp[offset + 6],
                tmp[offset + 7],
            ]);
            buf[i] = Complex32::new(re, im);
        }
        Ok(nsamples)
    }
}
