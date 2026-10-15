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
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::convert::IqFormat;
use crate::cpr::{CprDecoder, CprFrame, CprTimestamp, CprType};
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
    #[arg(
        long,
        default_value = "2400000",
        help = "Sample rate in Hz (Demod2400 requires 2400000)"
    )]
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
    #[arg(
        long,
        default_value = "0.0.0.0",
        help = "Net output bind address (use 127.0.0.1 for local only)"
    )]
    net_bind_address: String,
}

/// Thin wrapper around `NetIo` exposing the API expected by main.
struct NetOutput {
    inner: net_io::NetIo,
}

impl NetOutput {
    fn new(
        bind_address: &str,
        beast_port: u16,
        sbs_port: u16,
        raw_port: u16,
    ) -> anyhow::Result<Self> {
        let inner = net_io::NetIo::new();
        inner.start_on(bind_address, beast_port, sbs_port, raw_port)?;
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

    fn broadcast_raw_and_sbs(&self, mm: &ModesMessage, cpr: &mut CprDecoder) {
        let msg_len = mm.msgbits / 8;
        if !matches!(msg_len, 2 | 7 | 14) {
            return;
        }

        let msg = &mm.msg[..msg_len];
        // No AVR timestamp is available in the raw serializer, so use the
        // untimestamped `*` prefix for both Mode S and Mode A/C frames.
        self.inner.send_raw(msg);
        if msg_len == 2 {
            return;
        }

        if let Some(mut aircraft) = mode_s::decode_validated_mode_s_message(msg) {
            // BaseStation's six-hex-digit aircraft field cannot represent the
            // namespaced DF18 addresses tracked internally with a high byte.
            if aircraft.icao > 0x00FF_FFFF {
                return;
            }
            let type_code = msg.get(4).map_or(0, |byte| byte >> 3);
            if matches!(aircraft.df, 17 | 18)
                && ((5..=18).contains(&type_code) || (20..=22).contains(&type_code))
            {
                let frame = CprFrame {
                    cpr_type: if type_code <= 8 {
                        CprType::Surface
                    } else {
                        CprType::Airborne
                    },
                    odd: msg[6] & 0x04 != 0,
                    lat: (u32::from(msg[6] & 0x03) << 15)
                        | (u32::from(msg[7]) << 7)
                        | (u32::from(msg[8]) >> 1),
                    lon: (u32::from(msg[8] & 0x01) << 16)
                        | (u32::from(msg[9]) << 8)
                        | u32::from(msg[10]),
                };
                aircraft.position = cpr.submit_with_timestamp(
                    aircraft.icao,
                    frame,
                    cpr_capture_timestamp(mm.timestamp_msg),
                );
            }
            let Some(message_type) = sbs_message_type(aircraft.df, type_code) else {
                return;
            };
            let (date, time) = sbs_timestamp(SystemTime::now());
            let icao = format!("{:06X}", aircraft.icao & 0x00FF_FFFF);
            self.inner.send_sbs(&format!(
                "MSG,{message_type},,,{icao},,{date},{time},{date},{time},{callsign},{altitude},{ground_speed},{track},{latitude},{longitude},{vertical_rate},{squawk},,,,{on_ground}",
                callsign = aircraft.callsign.as_deref().unwrap_or(""),
                altitude = aircraft.altitude.map_or_else(String::new, |v| v.to_string()),
                ground_speed = aircraft
                    .velocity
                    .map(|v| v.0)
                    .or(aircraft.surface_movement)
                    .map_or_else(String::new, |v| format!("{v:.1}")),
                track = aircraft.velocity.map_or_else(String::new, |v| format!("{:.1}", v.1)),
                latitude = aircraft.position.map_or_else(String::new, |v| format!("{:.5}", v.0)),
                longitude = aircraft.position.map_or_else(String::new, |v| format!("{:.5}", v.1)),
                vertical_rate = aircraft.vertical_rate.map_or_else(String::new, |v| v.to_string()),
                squawk = aircraft.squawk.map_or_else(String::new, |v| format!("{v:04}")),
                on_ground = aircraft
                    .on_ground
                    .map_or_else(String::new, |v| if v { "-1" } else { "0" }.to_owned()),
            ));
        }
    }
}

fn cpr_capture_timestamp(ticks: u64) -> CprTimestamp {
    let seconds = ticks / TICKS_PER_SECOND;
    let remaining_ticks = ticks % TICKS_PER_SECOND;
    let nanoseconds = remaining_ticks * 1_000_000_000 / TICKS_PER_SECOND;
    CprTimestamp::Capture {
        epoch: 0,
        elapsed: Duration::new(seconds, nanoseconds as u32),
    }
}

fn sbs_message_type(df: u8, type_code: u8) -> Option<u8> {
    Some(match df {
        17 | 18 => match type_code {
            1..=4 => 1,
            5..=8 => 2,
            9..=18 | 20..=22 => 3,
            19 => 4,
            _ => return None,
        },
        0 | 16 => 7,
        11 => 8,
        4 | 20 => 5,
        5 | 21 => 6,
        _ => return None,
    })
}

fn sbs_timestamp(now: SystemTime) -> (String, String) {
    let millis = now
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let seconds = (millis / 1000) as i64;
    let days = seconds.div_euclid(86_400);
    let day_seconds = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let date = format!("{year:04}/{month:02}/{day:02}");
    let time = format!(
        "{:02}:{:02}:{:02}.{:03}",
        day_seconds / 3600,
        (day_seconds / 60) % 60,
        day_seconds % 60,
        millis % 1000
    );
    (date, time)
}

fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
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
    #[must_use]
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

    // The demodulator correlators are designed for 2.4 Msps. Anything else
    // still runs, but detection performance degrades; 0 would divide by zero
    // below, so it is rejected outright.
    if args.sample_rate == 0 {
        anyhow::bail!("--sample-rate must be greater than 0");
    }
    if args.sample_rate != 2_400_000 {
        eprintln!(
            "dump1090: warning: demodulator is tuned for 2400000 Hz, got {} Hz; detection may degrade",
            args.sample_rate
        );
    }

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
            &args.net_bind_address,
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
    // Overlap carried between blocks so messages straddling a 131072-sample
    // boundary are still detected. 320 samples cover a full 112-bit message
    // (~299 samples worst case incl. preamble) at 2.4 Msps for ~0.24% extra work.
    const OVERLAP_SAMPLES: usize = 320;
    // Reused across iterations to avoid a 256 KB allocator round-trip per block.
    let mut mag_data: Vec<u16> = Vec::with_capacity(buf_size + OVERLAP_SAMPLES);
    let mut carry: Vec<u16> = Vec::with_capacity(OVERLAP_SAMPLES);
    let start_time = Instant::now();
    // Count of new (non-overlap) samples consumed so far; block timestamps
    // are derived from it so overlap samples keep their original positions.
    let mut next_sample_idx: u64 = 0;
    let ticks_per_sample: u64 = TICKS_PER_SECOND / u64::from(args.sample_rate);

    let mut demod_stats = DemodStats::default();
    let mut demod = Demodulator::new();
    let mut cpr_decoder = CprDecoder::new();

    let mut last_stats_print = Instant::now();
    let mut interval_start_ms: u64 = 0;
    let mut interval_reader_cpu = Duration::ZERO;
    let mut interval_demod_cpu = Duration::ZERO;
    let mut interval_samples: u64 = 0;

    while running.load(Ordering::SeqCst) {
        let reader_start = Instant::now();
        let n = match source.read_samples(&mut buf) {
            Ok(0) => {
                if source.is_live() {
                    // Transient underrun (USB timeout, momentary stall), not
                    // end-of-stream: back off briefly instead of exiting.
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                }
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

        // Assemble block = previous tail (overlap) + new samples, reusing the
        // magnitude buffer allocation across iterations.
        mag_data.clear();
        mag_data.extend_from_slice(&carry);
        mag_data.extend_from_slice(&buf[..n]);
        let overlap = carry.len();
        let valid_length = mag_data.len();
        let block_start_idx = next_sample_idx.saturating_sub(overlap as u64);
        next_sample_idx += n as u64;

        let magnitudes = convert::to_magnitude(&mag_data);

        let now_ms = start_time.elapsed().as_millis() as u64;

        let (mean_level, mean_power) = compute_magbuf_stats(magnitudes);

        let mag_buf = MagBuf {
            data: std::mem::take(&mut mag_data).into(),
            total_length: valid_length,
            valid_length,
            overlap,
            sample_timestamp: block_start_idx * ticks_per_sample,
            sys_timestamp: now_ms,
            flags: MagBufFlags(0),
            mean_level,
            mean_power,
            dropped: 0,
        };

        // New samples consumed this block (overlap excluded from accounting).
        interval_samples += n as u64;

        let mut messages: Vec<ModesMessage> = Vec::new();
        let demod_start = Instant::now();
        demod.process(&mag_buf, &mut demod_stats, &mut |mm: &mut ModesMessage| {
            messages.push(mm.clone());
        });
        let demod_elapsed = demod_start.elapsed();
        interval_reader_cpu += reader_elapsed;
        interval_demod_cpu += demod_elapsed;

        // Reclaim the magnitude buffer allocation and retain the tail as the
        // next block's overlap.
        mag_data = mag_buf.data.into_owned();
        let tail = valid_length.saturating_sub(OVERLAP_SAMPLES);
        carry.clear();
        carry.extend_from_slice(&mag_data[tail..valid_length]);

        for mm in &messages {
            // 7. for each frame, call mode_s::decode_mode_s()
            if let Some(_aircraft_msg) = mode_s::decode_mode_s(mm) {
                // 7. if decoded, tracker.update_from_message()
                tracker.update_from_message(mm);
            }

            // Send each accepted frame to all requested network feed formats.
            if let Some(ref net) = net_output {
                net.broadcast_beast(mm);
                net.broadcast_raw_and_sbs(mm, &mut cpr_decoder);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpr_pair_timeout_uses_capture_time_for_fast_file_replay() {
        let mut decoder = CprDecoder::new();
        let even = CprFrame {
            cpr_type: CprType::Airborne,
            odd: false,
            lat: 93_000,
            lon: 51_372,
        };
        let odd = CprFrame {
            cpr_type: CprType::Airborne,
            odd: true,
            lat: 74_158,
            lon: 50_194,
        };

        assert!(decoder
            .submit_with_timestamp(0x40621D, even, cpr_capture_timestamp(0))
            .is_none());
        // The CPU can process both file frames in milliseconds, but their
        // source timestamps are over ten seconds apart and cannot form a pair.
        assert!(decoder
            .submit_with_timestamp(
                0x40621D,
                odd,
                cpr_capture_timestamp(10 * TICKS_PER_SECOND + 1),
            )
            .is_none());
    }
}
