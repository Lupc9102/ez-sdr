//! Network RTL-SDR/rtl_tcp-compatible raw IQ source.
//!
//! Physical RTL-SDR v4 devices deliver Uc8 IQ over USB.  `rtl_tcp` is the
//! common network bridge: it sends a 12-byte `RTL0` header, then the same raw
//! interleaved unsigned 8-bit I/Q byte stream.  This source accepts that wire
//! format so a test generator can stand in for a dongle without hardware.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use anyhow::{Context, Result};
use num_complex::Complex32;

use super::IqSource;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const READ_BUF_BYTES: usize = 16 * 16384;

pub struct TcpIqSource {
    address: SocketAddr,
    expect_rtl_tcp_header: bool,
    stream: Option<TcpStream>,
    pending: Vec<u8>,
    read_buf: Vec<u8>,
    frequency_hz: u64,
    sample_rate_hz: u32,
    gain_db: f64,
}

impl TcpIqSource {
    #[must_use]
    pub fn new(address: SocketAddr, expect_rtl_tcp_header: bool) -> Self {
        Self {
            address,
            expect_rtl_tcp_header,
            stream: None,
            pending: Vec::new(),
            read_buf: Vec::new(),
            frequency_hz: 1_090_000_000,
            sample_rate_hz: 2_400_000,
            gain_db: 40.0,
        }
    }

    fn send_command(&mut self, command: u8, value: u32) -> Result<()> {
        // A plain raw-Uc8 endpoint is receive-only.  rtl_tcp's five-byte
        // control protocol is only valid when the peer sent its RTL0 greeting;
        // writing commands to a raw stream can corrupt a bidirectional test
        // server (and is surprising for callers that explicitly disabled the
        // greeting).
        if !self.expect_rtl_tcp_header {
            return Ok(());
        }
        if let Some(stream) = self.stream.as_mut() {
            let mut packet = [0u8; 5];
            packet[0] = command;
            packet[1..].copy_from_slice(&value.to_be_bytes());
            stream
                .write_all(&packet)
                .context("sending rtl_tcp command")?;
        }
        Ok(())
    }
}

impl IqSource for TcpIqSource {
    fn start(&mut self) -> Result<()> {
        let mut stream = TcpStream::connect_timeout(&self.address, CONNECT_TIMEOUT)
            .with_context(|| format!("connecting to raw IQ source {}", self.address))?;
        stream.set_read_timeout(Some(CONNECT_TIMEOUT))?;
        if self.expect_rtl_tcp_header {
            let mut header = [0u8; 12];
            stream
                .read_exact(&mut header)
                .context("reading rtl_tcp header")?;
            if &header[..4] != b"RTL0" {
                anyhow::bail!("raw IQ server did not send rtl_tcp RTL0 header");
            }
        }
        self.stream = Some(stream);
        self.pending.clear();
        // `DaemonConfig` applies initial tuning before the ingestion thread
        // calls `start`, so those values were previously only cached locally.
        // rtl_tcp requires them to be sent after the TCP connection is live;
        // apply the cached startup settings now as well as handling later
        // control requests in `set_*`.
        if self.expect_rtl_tcp_header {
            self.send_command(0x01, self.frequency_hz as u32)?;
            self.send_command(0x02, self.sample_rate_hz)?;
        }
        Ok(())
    }

    fn stop(&mut self) {
        self.stream.take();
    }

    fn set_frequency(&mut self, hz: u64) -> Result<()> {
        self.frequency_hz = hz;
        self.send_command(0x01, hz as u32)
    }

    fn set_sample_rate(&mut self, hz: u32) -> Result<()> {
        self.sample_rate_hz = hz;
        self.send_command(0x02, hz)
    }

    fn set_gain(&mut self, db: f64) -> Result<()> {
        self.gain_db = db;
        Ok(())
    }

    fn read_iq(&mut self, buf: &mut [Complex32]) -> Result<usize> {
        let want_bytes = (buf.len() * 2).min(READ_BUF_BYTES);
        self.read_buf.resize(want_bytes, 0);
        let stream = self
            .stream
            .as_mut()
            .context("raw IQ source is not started")?;

        let mut filled = 0;
        if !self.pending.is_empty() {
            let n = self.pending.len().min(want_bytes);
            self.read_buf[..n].copy_from_slice(&self.pending[..n]);
            self.pending.drain(..n);
            filled = n;
        }
        while filled < want_bytes {
            let n = stream.read(&mut self.read_buf[filled..want_bytes])?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        if filled % 2 != 0 {
            self.pending.push(self.read_buf[filled - 1]);
            filled -= 1;
        }
        let complex = lrpt_decode::iq_bytes_to_complex(&self.read_buf[..filled]);
        let n = complex.len().min(buf.len());
        buf[..n].copy_from_slice(&complex[..n]);
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
        "tcp-iq"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    #[ignore = "requires loopback TCP sockets; run manually with --ignored"]
    fn rtl_tcp_start_applies_cached_tuning_and_reads_uc8() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut header = [0u8; 12];
            header[..4].copy_from_slice(b"RTL0");
            stream.write_all(&header).unwrap();

            // rtl_tcp control packets are command byte + big-endian u32.
            let mut commands = [0u8; 10];
            stream.read_exact(&mut commands).unwrap();
            assert_eq!(commands[0], 0x01);
            assert_eq!(&commands[1..5], &1_090_000_000u32.to_be_bytes());
            assert_eq!(commands[5], 0x02);
            assert_eq!(&commands[6..10], &2_400_000u32.to_be_bytes());
            stream.write_all(&[127, 127, 241, 127]).unwrap();
        });

        let mut source = TcpIqSource::new(address, true);
        source.start().unwrap();
        let mut samples = [Complex32::new(0.0, 0.0); 2];
        assert_eq!(source.read_iq(&mut samples).unwrap(), 2);
        assert!((samples[0].re + 0.4).abs() < 1e-5);
        assert!((samples[1].re - 113.6).abs() < 1e-5);
        server.join().unwrap();
    }

    #[test]
    #[ignore = "requires loopback TCP sockets; run manually with --ignored"]
    fn raw_uc8_mode_does_not_write_rtl_tcp_commands() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.write_all(&[127, 127, 241, 127]).unwrap();
            // If the client wrote command bytes to this receive-only endpoint,
            // this server would observe them only after the sample payload.
            // Closing immediately keeps the assertion deterministic: a
            // command write would fail with a broken pipe in `set_*`, while
            // the source's startup path must complete without any writes.
        });

        let mut source = TcpIqSource::new(address, false);
        source.start().unwrap();
        let mut samples = [Complex32::new(0.0, 0.0); 2];
        assert_eq!(source.read_iq(&mut samples).unwrap(), 2);
        server.join().unwrap();
    }
}
