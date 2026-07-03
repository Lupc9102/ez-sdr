pub mod adaptive;
pub mod convert;
pub mod cpr;
pub mod crc;
pub mod demod;
pub mod fifo;
pub mod icao_filter;
pub mod mode_ac;
pub mod mode_s;
pub mod net_io;
pub mod sdr;
pub mod stats;
pub mod track;
pub mod util;

use clap::Parser;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::convert::IqFormat;
use crate::demod::{DemodStats, MagBuf, MagBufFlags, ModesMessage};
use crate::sdr::ifile::IFileSdr;
#[cfg(feature = "rtlsdr")]
use crate::sdr::rtlsdr::RtlSdr;
use crate::sdr::SdrSource;
use crate::stats::{Stats, MAX_BITERRORS};
use crate::track::Tracker;

/// 12 MHz clock ticks used for timestamp interpolation.
const TICKS_PER_SECOND: u64 = 12_000_000;

#[derive(Parser, Debug)]
#[command(name = "dump1090", version, about = "ADS-B Mode S decoder in Rust")]
struct Args {
    #[arg(long, help = "RTL-SDR device index or serial")]
    device_index: Option<String>,
    #[arg(long, help = "Gain in dB (0 = auto)")]
    gain: Option<f64>,
    #[arg(long, default_value = "1090000000", help = "Center frequency in Hz")]
    freq: u64,
    #[arg(long, default_value = "2000000", help = "Sample rate in Hz")]
    sample_rate: u32,
    #[arg(long, help = "Read from file instead of SDR")]
    ifile: Option<String>,
    #[arg(long, help = "Enable net output (Beast / SBS / raw)")]
    net: bool,
    #[arg(long, default_value = "30005", help = "Beast output port")]
    net_beast_port: u16,
    #[arg(long, default_value = "30003", help = "SBS output port")]
    net_sbs_port: u16,
    #[arg(long, default_value = "30002", help = "Raw output port")]
    net_raw_port: u16,
}

/// Thin wrapper around `NetIo` exposing the API expected by main.
struct NetOutput {
    inner: net_io::NetIo,
}

impl NetOutput {
    fn new(beast_port: u16, sbs_port: u16, raw_port: u16) -> anyhow::Result<Self> {
        let inner = net_io::NetIo::new();
        inner.start(beast_port, sbs_port, raw_port)?;
        Ok(NetOutput { inner })
    }

    fn broadcast_beast(&self, mm: &ModesMessage) {
        let signal = (mm.signal_level * 255.0).min(255.0) as u8;
        let msg_len = mm.msgbits / 8;
        if msg_len > 0 {
            self.inner
                .send_beast(mm.timestamp_msg, signal, &mm.msg[..msg_len]);
        }
    }
}

/// Wrapper around `Demod2400` that exposes a `process` method matching the
/// architecture expected by main.
pub struct Demodulator {
    inner: demod::Demod2400,
}

impl Default for Demodulator {
    fn default() -> Self {
        Self::new()
    }
}

impl Demodulator {
    pub fn new() -> Self {
        Demodulator {
            inner: demod::Demod2400::new(),
        }
    }

    pub fn process(
        &mut self,
        mag: &MagBuf,
        stats: &mut DemodStats,
        on_message: &mut dyn FnMut(&mut ModesMessage),
    ) {
        self.inner.demodulate(mag, stats, on_message);
        self.inner.demodulate_ac(mag, stats, on_message);
    }
}

fn compute_magbuf_stats(data: &[u16]) -> (f64, f64) {
    let n = data.len() as f64;
    if n == 0.0 {
        return (0.0, 0.0);
    }
    let sum_level: f64 = data.iter().map(|&v| v as f64 / 65535.0).sum();
    let mean_level = sum_level / n;
    let sum_power: f64 = data
        .iter()
        .map(|&v| {
            let normalized = v as f64 / 65535.0;
            normalized * normalized
        })
        .sum();
    let mean_power = sum_power / n;
    (mean_level, mean_power)
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // 1. Ctrl-C handling
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::Relaxed);
    })?;

    // 2. Initialize SdrSource
    let mut source: Box<dyn SdrSource> = if let Some(ref path) = args.ifile {
        let format = IqFormat::Uc8;
        let loop_file = false;
        Box::new(IFileSdr::new(path.as_str(), format, loop_file))
    } else {
        #[cfg(feature = "rtlsdr")]
        {
            let gain = args.gain.unwrap_or(0.0);
            Box::new(RtlSdr::new(
                args.device_index.clone(),
                args.freq,
                args.sample_rate,
                gain,
                0,     // ppm
                0,     // direct_sampling
                false, // digital_agc
            ))
        }
        #[cfg(not(feature = "rtlsdr"))]
        {
            anyhow::bail!(
                "no SDR backend compiled in; rebuild with --features rtlsdr (or pass --ifile)"
            )
        }
    };

    // 3. Set frequency, sample rate, gain
    source.set_frequency(args.freq)?;
    source.set_sample_rate(args.sample_rate)?;
    source.set_gain(args.gain.unwrap_or(0.0))?;

    // 4. Start source
    source.start().inspect_err(|_e| {
        eprintln!(
            "dump1090: failed to start source. If using RTL-SDR, ensure a device is connected."
        );
    })?;

    // 5. If --net, start NetOutput
    let net_output: Option<NetOutput> = if args.net {
        Some(NetOutput::new(
            args.net_beast_port,
            args.net_sbs_port,
            args.net_raw_port,
        )?)
    } else {
        None
    };

    // 6. Create Tracker
    let mut tracker = Tracker::new();

    // 7. Run loop
    let buf_size: usize = 128 * 1024; // 131072 samples
    let mut buf = vec![0u16; buf_size];
    let start_time = Instant::now();
    let mut sample_timestamp: u64 = 0;
    let ticks_per_sample: u64 = TICKS_PER_SECOND / args.sample_rate as u64;

    let mut demod_stats = DemodStats::default();
    let mut demod = Demodulator::new();

    let mut last_stats_print = Instant::now();
    let mut interval_start_ms: u64 = 0;
    let mut interval_reader_cpu = Duration::ZERO;
    let mut interval_demod_cpu = Duration::ZERO;
    let mut interval_samples: u64 = 0;

    while running.load(Ordering::SeqCst) {
        let reader_start = Instant::now();
        let n = match source.read_samples(&mut buf) {
            Ok(0) => {
                // EOF for file input
                break;
            }
            Ok(n) => n,
            Err(e) => {
                eprintln!("dump1090: read_samples error: {}", e);
                break;
            }
        };
        let reader_elapsed = reader_start.elapsed();

        let magnitudes = convert::to_magnitude(&buf[..n]);

        let now_ms = start_time.elapsed().as_millis() as u64;

        let (mean_level, mean_power) = compute_magbuf_stats(magnitudes);

        let total_length = magnitudes.len();
        let mag_buf = MagBuf {
            data: magnitudes.to_vec(),
            total_length,
            valid_length: magnitudes.len(),
            overlap: 0,
            sample_timestamp,
            sys_timestamp: now_ms,
            flags: MagBufFlags(0),
            mean_level,
            mean_power,
            dropped: 0,
        };

        // Advance timestamp for next block
        sample_timestamp += (n as u64) * ticks_per_sample;
        interval_samples += n as u64;

        let mut messages: Vec<ModesMessage> = Vec::new();
        let demod_start = Instant::now();
        demod.process(&mag_buf, &mut demod_stats, &mut |mm: &mut ModesMessage| {
            messages.push(mm.clone());
        });
        let demod_elapsed = demod_start.elapsed();
        interval_reader_cpu += reader_elapsed;
        interval_demod_cpu += demod_elapsed;

        for mm in &messages {
            // 7. for each frame, call mode_s::decode_mode_s()
            if let Some(_aircraft_msg) = mode_s::decode_mode_s(mm) {
                // 7. if decoded, tracker.update_from_message()
                tracker.update_from_message(mm);
            }

            // 7. if --net, net_output.broadcast_beast(&frame.raw_bytes)
            if let Some(ref net) = net_output {
                net.broadcast_beast(mm);
            }
        }

        // 8. Print stats periodic summary every second
        if last_stats_print.elapsed() >= Duration::from_secs(1) {
            last_stats_print = Instant::now();

            let mut interval_stats = Stats {
                start_ms: interval_start_ms,
                end_ms: now_ms,
                ..Default::default()
            };
            interval_stats.demod_preambles = demod_stats.demod_preambles as u32;
            interval_stats.demod_rejected_bad = demod_stats.demod_rejected_bad as u32;
            interval_stats.demod_rejected_unknown_icao =
                demod_stats.demod_rejected_unknown_icao as u32;
            for i in 0..=MAX_BITERRORS {
                interval_stats.demod_accepted[i] = demod_stats.demod_accepted[i] as u32;
            }
            interval_stats.demod_modeac = demod_stats.demod_modeac as u32;
            interval_stats.noise_power_sum = demod_stats.noise_power_sum;
            interval_stats.noise_power_count = demod_stats.noise_power_count;
            interval_stats.signal_power_sum = demod_stats.signal_power_sum;
            interval_stats.signal_power_count = demod_stats.signal_power_count;
            interval_stats.peak_signal_power = demod_stats.peak_signal_power;
            interval_stats.strong_signal_count = demod_stats.strong_signal_count as u32;
            // Prune aircraft that haven't been seen in the last 60 s. Without
            // this the tracker HashMap grew unbounded over multi-hour sessions
            // (the original track.c pruned at the same 60 s idle threshold via
            // expireAircraft). Done once per stats interval (1 s) so the cost
            // is amortised.
            tracker.prune_older_than(now_ms.saturating_sub(60_000));
            interval_stats.unique_aircraft = tracker.len() as u32;
            interval_stats.reader_cpu = interval_reader_cpu;
            interval_stats.demod_cpu = interval_demod_cpu;
            interval_stats.samples_processed = interval_samples;
            interval_stats.tick_second(now_ms);

            println!("{}", interval_stats);

            // Reset interval accumulators
            demod_stats = DemodStats::default();
            interval_start_ms = now_ms;
            interval_reader_cpu = Duration::ZERO;
            interval_demod_cpu = Duration::ZERO;
            interval_samples = 0;
        }
    }

    // 9. Handle Ctrl-C gracefully (stop source, flush tracker)
    println!("dump1090: shutting down...");
    source.stop();
    println!("dump1090: tracked {} unique aircraft", tracker.len());

    Ok(())
}
