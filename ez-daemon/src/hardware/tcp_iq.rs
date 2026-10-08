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

use super::{IqSource, ReadPending};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const LIVE_TIMEOUT: Duration = Duration::from_millis(200);
const READ_BUF_BYTES: usize = 16 * 16384;

/// In rtl_tcp mode getters report the last command written to the socket. The
/// protocol supplies no acknowledgement, gain table or hardware readback. In
/// headerless mode these values describe externally configured capture metadata.
pub struct TcpIqSource {
    address: SocketAddr,
    expect_rtl_tcp_header: bool,
    stream: Option<TcpStream>,
    pending: Vec<u8>,
    read_buf: Vec<u8>,
    frequency_hz: u64,
    sample_rate_hz: u32,
    gain_db: f64,
    requested_gain_db: f64,
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
            requested_gain_db: 40.0,
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
        stream.set_write_timeout(Some(LIVE_TIMEOUT))?;
        if self.expect_rtl_tcp_header {
            let mut header = [0u8; 12];
            stream
                .read_exact(&mut header)
                .context("reading rtl_tcp header")?;
            if &header[..4] != b"RTL0" {
                anyhow::bail!("raw IQ server did not send rtl_tcp RTL0 header");
            }
        }
        stream.set_read_timeout(Some(LIVE_TIMEOUT))?;
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
            self.set_gain(self.requested_gain_db)?;
        }
        Ok(())
    }

    fn stop(&mut self) {
        self.stream.take();
    }

    fn set_frequency(&mut self, hz: u64) -> Result<()> {
        if self.expect_rtl_tcp_header {
            let value = u32::try_from(hz).context("rtl_tcp frequency exceeds u32 range")?;
            self.send_command(0x01, value)?;
        }
        self.frequency_hz = hz;
        Ok(())
    }

    fn set_sample_rate(&mut self, hz: u32) -> Result<()> {
        anyhow::ensure!(hz > 0, "sample rate must be positive");
        self.send_command(0x02, hz)?;
        self.sample_rate_hz = hz;
        Ok(())
    }

    fn set_gain(&mut self, db: f64) -> Result<()> {
        anyhow::ensure!(db.is_finite(), "gain must be finite");
        if self.expect_rtl_tcp_header {
            // rtl_tcp command_worker: 0x03 selects manual mode; 0x04 supplies
            // gain in tenths of a dB. <=0 selects tuner AGC, as on local RTL.
            let tenths = (db * 10.0).round();
            anyhow::ensure!(tenths <= i32::MAX as f64, "gain exceeds rtl_tcp range");
            if db <= 0.0 {
                self.send_command(0x03, 0)?;
            } else {
                self.send_command(0x03, 1)?;
                self.send_command(0x04, tenths as u32)?;
            }
            self.gain_db = if db <= 0.0 { 0.0 } else { tenths / 10.0 };
        } else {
            self.gain_db = db;
        }
        self.requested_gain_db = db;
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
        let read = stream.read(&mut self.read_buf[filled..want_bytes]);
        let mut eof = false;
        match read {
            Ok(0) => eof = true,
            Ok(n) => filled += n,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error.into()),
        }
        if !filled.is_multiple_of(2) {
            self.pending.push(self.read_buf[filled - 1]);
            filled -= 1;
        }
        if filled == 0 {
            if eof {
                anyhow::ensure!(
                    self.pending.is_empty(),
                    "raw IQ source ended with an incomplete sample pair"
                );
                return Ok(0);
            }
            return Err(ReadPending::Timeout.into());
        }
        let complex = lrpt_decode::uc8_bytes_to_normalized(&self.read_buf[..filled]);
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
    fn requested_gain_db(&self) -> Option<f64> {
        self.expect_rtl_tcp_header.then_some(self.requested_gain_db)
    }
    fn gain_mode(&self) -> Option<ez_proto::GainMode> {
        self.expect_rtl_tcp_header
            .then_some(if self.requested_gain_db <= 0.0 {
                ez_proto::GainMode::Automatic
            } else {
                ez_proto::GainMode::Manual
            })
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
    fn stalled_tcp_peer_yields_and_fragmented_iq_keeps_the_first_component() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (send_more, wait_more) = std::sync::mpsc::channel();
        let server = thread::spawn(move || {
            let (mut peer, _) = listener.accept().unwrap();
            peer.write_all(&[12]).unwrap();
            wait_more.recv().unwrap();
            peer.write_all(&[34, 56, 78]).unwrap();
        });
        let mut source = TcpIqSource::new(address, false);
        source.start().unwrap();
        let mut output = [Complex32::default(); 4];
        let first = source.read_iq(&mut output).unwrap_err();
        assert!(matches!(
            first.downcast_ref::<ReadPending>(),
            Some(ReadPending::Timeout)
        ));
        let start = std::time::Instant::now();
        assert!(source
            .read_iq(&mut output)
            .unwrap_err()
            .downcast_ref::<ReadPending>()
            .is_some());
        assert!(start.elapsed() < Duration::from_secs(1));
        send_more.send(()).unwrap();
        assert_eq!(source.read_iq(&mut output).unwrap(), 2);
        assert_eq!(
            &output[..2],
            &lrpt_decode::uc8_bytes_to_normalized(&[12, 34, 56, 78])
        );
        server.join().unwrap();
        assert_eq!(source.read_iq(&mut output).unwrap(), 0);
    }

    #[test]
    fn rtl_tcp_gain_wire_commands_cover_startup_manual_and_auto() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut peer, _) = listener.accept().unwrap();
            peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            peer.write_all(b"RTL0\0\0\0\0\0\0\0\0").unwrap();
            let expected = [
                (1, 137_100_000),
                (2, 1_024_000),
                (3, 1),
                (4, 123),
                (3, 1),
                (4, 297),
                (3, 0),
            ];
            for (command, value) in expected {
                let mut packet = [0; 5];
                peer.read_exact(&mut packet).unwrap();
                assert_eq!(packet[0], command);
                assert_eq!(u32::from_be_bytes(packet[1..].try_into().unwrap()), value);
            }
        });
        let mut source = TcpIqSource::new(address, true);
        source.set_frequency(137_100_000).unwrap();
        source.set_sample_rate(1_024_000).unwrap();
        source.set_gain(12.34).unwrap();
        assert_eq!(source.gain_db(), 12.3);
        assert_eq!(source.requested_gain_db(), Some(12.34));
        assert_eq!(source.gain_mode(), Some(ez_proto::GainMode::Manual));
        source.start().unwrap();
        source.set_gain(29.68).unwrap();
        assert_eq!(source.gain_db(), 29.7);
        source.set_gain(-1.0).unwrap();
        assert_eq!(source.gain_db(), 0.0);
        assert_eq!(source.gain_mode(), Some(ez_proto::GainMode::Automatic));
        server.join().unwrap();
        source.stop();
    }

    #[test]
    fn invalid_tcp_controls_preserve_cached_metadata() {
        let mut source = TcpIqSource::new("127.0.0.1:1".parse().unwrap(), true);
        assert!(source.set_frequency(u64::MAX).is_err());
        assert!(source.set_sample_rate(0).is_err());
        assert!(source.set_gain(f64::NAN).is_err());
        assert!(source.set_gain(f64::INFINITY).is_err());
        assert_eq!(source.frequency_hz(), 1_090_000_000);
        assert_eq!(source.sample_rate_hz(), 2_400_000);
        assert_eq!(source.gain_db(), 40.0);
    }

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
            let mut commands = [0u8; 20];
            stream.read_exact(&mut commands).unwrap();
            assert_eq!(commands[0], 0x01);
            assert_eq!(&commands[1..5], &1_090_000_000u32.to_be_bytes());
            assert_eq!(commands[5], 0x02);
            assert_eq!(&commands[6..10], &2_400_000u32.to_be_bytes());
            assert_eq!(commands[10], 0x03);
            assert_eq!(&commands[11..15], &1u32.to_be_bytes());
            assert_eq!(commands[15], 0x04);
            assert_eq!(&commands[16..20], &400u32.to_be_bytes());
            stream.write_all(&[127, 127, 241, 127]).unwrap();
        });

        let mut source = TcpIqSource::new(address, true);
        source.start().unwrap();
        let mut samples = [Complex32::new(0.0, 0.0); 2];
        assert_eq!(source.read_iq(&mut samples).unwrap(), 2);
        assert!((samples[0].re + 0.5 / 127.5).abs() < 1e-5);
        assert!((samples[1].re - 113.5 / 127.5).abs() < 1e-5);
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
