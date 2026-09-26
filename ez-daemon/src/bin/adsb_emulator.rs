//! Serve deterministic 1090ES Uc8 IQ using the rtl_tcp wire greeting.

use std::io::Write;
use std::net::{SocketAddr, TcpListener};
use std::time::{Duration, Instant};

use clap::Parser;
use ez_daemon::hardware::adsb::{encode_uc8, ADSB_SAMPLE_RATE_HZ, EXAMPLE_DF17_CALLSIGN};

#[derive(Debug, Parser)]
#[command(name = "adsb-emulator")]
struct Args {
    #[arg(long, default_value = "127.0.0.1:12345")]
    bind: SocketAddr,
    /// Number of seconds to stream each accepted connection (0 means one waveform then close).
    #[arg(long, default_value_t = 10)]
    seconds: u64,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let listener = TcpListener::bind(args.bind)?;
    eprintln!("ADS-B rtl_tcp emulator listening on {}", args.bind);
    let raw = encode_uc8(&[EXAMPLE_DF17_CALLSIGN]);
    for connection in listener.incoming() {
        let mut stream = connection?;
        // rtl_tcp greeting: magic, tuner type (R820T), gain count.
        let mut header = [0u8; 12];
        header[..4].copy_from_slice(b"RTL0");
        header[4..8].copy_from_slice(&1u32.to_be_bytes());
        header[8..12].copy_from_slice(&0u32.to_be_bytes());
        stream.write_all(&header)?;

        let start = Instant::now();
        loop {
            stream.write_all(&raw)?;
            stream.flush()?;
            if args.seconds == 0 || start.elapsed() >= Duration::from_secs(args.seconds) {
                break;
            }
            // Keep the generated stream close to 2.4 MSPS without sleeping per sample.
            let duration =
                Duration::from_secs_f64(raw.len() as f64 / 2.0 / ADSB_SAMPLE_RATE_HZ as f64);
            std::thread::sleep(duration);
        }
    }
    Ok(())
}
