#![recursion_limit = "256"]

pub mod dsp;
pub mod signal_path;
pub mod source;
pub mod sink;
pub mod decoder;
pub mod module;
pub mod config;
pub mod server;
pub mod core;

use clap::Parser;
use num_complex::Complex32;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[cfg(feature = "rtlsdr")]
use crate::source::rtl_sdr::Source;

#[derive(Parser, Debug)]
#[command(name = "sdrpp", about = "SDR++ core in Rust")]
struct Args {
    #[arg(long, help = "Server mode")]
    server: bool,
    #[arg(long, help = "Config file path")]
    config: Option<String>,
}

struct NullFftProvider;

impl signal_path::FftBufferProvider for NullFftProvider {
    fn acquire(&mut self) -> Option<&mut [f32]> {
        None
    }
    fn release(&mut self) {}
}

enum SourceWrapper {
    #[cfg(feature = "rtlsdr")]
    Rtl(source::rtl_sdr::RtlSdrSource),
    File(source::file::FileSource),
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // 1. Initialize Core
    let mut core = core::Core::new();
    if let Some(ref config_path) = args.config {
        core.config.set_path(config_path);
    }
    core.server_mode = args.server;
    core.init().map_err(|e| anyhow::anyhow!(e.to_string()))?;

    // 2. Server mode
    if args.server {
        let mut srv = server::Server::new();
        srv.start("0.0.0.0", 5259)?;
        println!("SDR++ server listening on 0.0.0.0:5259");

        let running = Arc::new(AtomicBool::new(true));
        let r = running.clone();
        ctrlc::set_handler(move || {
            println!("Received Ctrl-C, shutting down server...");
            r.store(false, Ordering::SeqCst);
        })?;

        while running.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        srv.stop();
        core.shutdown();
        println!("Server shut down.");
        return Ok(());
    }

    // 3. Audio Sink
    let mut audio_sink = sink::audio::AudioSink::new("Radio")?;
    audio_sink.start()?;
    let audio_rate = audio_sink.current_sample_rate() as f64;

    // 4. Load source (RTL-SDR first, then file fallback)
    let (mut source, source_rate) = load_source(&args)?;
    println!("Source sample rate: {} Hz", source_rate);

    // 5. Signal path
    let decim_ratio = (source_rate / audio_rate).max(1.0).round() as usize;
    let effective_rate = source_rate / decim_ratio.max(1) as f64;
    println!("Decimation: {}, effective rate: {} Hz", decim_ratio, effective_rate);

    let mut iq_frontend = signal_path::IqFrontend::new(
        source_rate,
        true,                                // buffering
        decim_ratio,
        false,                               // dc_blocking
        2048,                                // fft_size
        20.0,                                // fft_rate
        signal_path::FftWindow::Blackman,
        Box::new(NullFftProvider),
    );
    iq_frontend.start();

    let mut vfo_manager = signal_path::VfoManager::new();
    let vfo_name = "Radio";
    vfo_manager.create_vfo(vfo_name, 0, 0.0, 12500.0, effective_rate, 100.0, 500000.0, false);
    let (_vfo_dsp, vfo_rx) = iq_frontend
        .add_vfo(vfo_name, effective_rate, 12500.0, 0.0)
        .expect("Failed to add VFO");

    // 6. DSP chain
    let lpf_taps = dsp::filter::lowpass_taps(6000.0, 2000.0, effective_rate as f32);
    let mut fir_filter = dsp::filter::FirFilter::<Complex32, f32>::new(&lpf_taps);

    let mut fft_block = dsp::fft::FftBlock::new(2048);
    fft_block.set_window(dsp::fft::WindowType::Blackman, true);

    // 7. Decoder
    let mut decoder = decoder::radio::Driver::new(
        decoder::radio::Mode::Nfm,
        effective_rate as f32,
        12500.0,
    );

    // Clone file stream if using file source so we can read from it later
    let file_stream = match &source {
        SourceWrapper::File(src) => Some(src.stream()),
        #[cfg(feature = "rtlsdr")]
        _ => None,
    };

    // Start file source worker now that everything is wired
    if let SourceWrapper::File(ref mut src) = source {
        src.start();
    }

    // 8. Ctrl-C handler
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        println!("Received Ctrl-C, shutting down...");
        r.store(false, Ordering::SeqCst);
    })?;

    // 9. Process loop
    println!("SDR++ running. Press Ctrl-C to stop.");

    match &mut source {
        #[cfg(feature = "rtlsdr")]
        SourceWrapper::Rtl(src) => {
            let mut block = vec![Complex32::new(0.0, 0.0); 8192];
            while running.load(Ordering::SeqCst) {
                match src.read_samples(&mut block) {
                    Ok(n) if n > 0 => {
                        iq_frontend.process_block(&block[..n]);
                        process_vfo_chain(
                            &vfo_rx,
                            &mut fir_filter,
                            &mut fft_block,
                            &mut decoder,
                            &audio_sink,
                        );
                    }
                    Ok(_) => break,
                    Err(e) => {
                        eprintln!("RTL-SDR read error: {}", e);
                        break;
                    }
                }
            }
        }
        SourceWrapper::File(_) => {
            let stream = file_stream.expect("file stream missing");
            while running.load(Ordering::SeqCst) {
                match stream.read() {
                    Some(count) => {
                        let buf = stream.read_buf();
                        iq_frontend.process_block(&buf[..count]);
                        drop(buf);
                        stream.flush();
                        process_vfo_chain(
                            &vfo_rx,
                            &mut fir_filter,
                            &mut fft_block,
                            &mut decoder,
                            &audio_sink,
                        );
                    }
                    None => break,
                }
            }
        }
    }

    // 10. Graceful shutdown
    println!("Shutting down...");
    iq_frontend.stop();
    audio_sink.stop()?;

    match source {
        #[cfg(feature = "rtlsdr")]
        SourceWrapper::Rtl(mut src) => {
            let _ = src.stop();
        }
        SourceWrapper::File(mut src) => {
            src.stop();
        }
    }

    core.shutdown();
    println!("SDR++ shut down.");
    Ok(())
}

fn load_source(args: &Args) -> anyhow::Result<(SourceWrapper, f64)> {
    #[cfg(feature = "rtlsdr")]
    {
        let mut src = source::rtl_sdr::RtlSdrSource::new();
        if src.open(0).is_ok() {
            let target_rate = 1_920_000;
            if src.set_sample_rate(target_rate).is_ok() && src.start().is_ok() {
                println!("RTL-SDR source started at {} Hz", target_rate);
                return Ok((SourceWrapper::Rtl(src), target_rate as f64));
            }
        }
    }

    let mut file_src = source::file::FileSource::new();
    let path = args.config.as_deref().unwrap_or("test.wav");
    file_src.open(path)?;
    let rate = file_src.sample_rate() as f64;
    println!("File source opened: {} at {} Hz", path, rate);
    Ok((SourceWrapper::File(file_src), rate))
}

fn process_vfo_chain(
    vfo_rx: &crossbeam_channel::Receiver<Arc<Vec<Complex32>>>,
    fir_filter: &mut dsp::filter::FirFilter<Complex32, f32>,
    fft_block: &mut dsp::fft::FftBlock,
    decoder: &mut decoder::radio::Driver,
    audio_sink: &sink::audio::AudioSink,
) {
    while let Ok(block) = vfo_rx.try_recv() {
        // FIR lowpass
        let mut filtered = vec![Complex32::new(0.0, 0.0); block.len()];
        fir_filter.process(&block, &mut filtered);

        // FFT waterfall analysis (diagnostic)
        if filtered.len() >= fft_block.fft_size() {
            let _spectrum = fft_block.process_windowed(&filtered[..fft_block.fft_size()]);
        }

        // Demodulate
        let audio = decoder.process(&filtered);

        // Convert to audio sink format
        let stereo: Vec<sink::audio::StereoSample> = audio
            .iter()
            .map(|s| sink::audio::StereoSample::new(s.l, s.r))
            .collect();

        audio_sink.push_samples(&stereo);
    }
}
