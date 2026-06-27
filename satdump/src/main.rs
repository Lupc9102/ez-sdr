pub mod dsp;
pub mod pipeline;
pub mod products;
pub mod image;
pub mod projection;
pub mod common;
pub mod instrument;
pub mod source;
pub mod cli;

use anyhow::{Context, Result};
use clap::{Arg, ArgMatches, Command};
use std::path::{Path, PathBuf};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// CLI command handler for the `run` subcommand
// ---------------------------------------------------------------------------

struct RunCmd;

impl cli::CmdHandler for RunCmd {
    fn cmd(&self) -> &'static str {
        "run"
    }

    fn reg(&self, app: &mut Command) {
        let sub = Command::new("run")
            .about("Run a processing pipeline")
            .arg(Arg::new("input").required(true).help("Input WAV file"))
            .arg(
                Arg::new("pipeline")
                    .long("pipeline")
                    .required(true)
                    .help("Pipeline JSON file"),
            )
            .arg(
                Arg::new("output")
                    .long("output")
                    .required(true)
                    .help("Output directory"),
            );
        *app = app.clone().subcommand(sub);
    }

    fn run(&self, matches: &ArgMatches, _is_gui: bool) -> Result<()> {
        let input = matches
            .get_one::<String>("input")
            .map(PathBuf::from)
            .context("Missing input argument")?;
        let pipeline_path = matches
            .get_one::<String>("pipeline")
            .map(PathBuf::from)
            .context("Missing pipeline argument")?;
        let output = matches
            .get_one::<String>("output")
            .map(PathBuf::from)
            .context("Missing output argument")?;

        run_pipeline(&input, &pipeline_path, &output)
    }
}

// ---------------------------------------------------------------------------
// Pipeline runner (source -> DSP -> instrument -> products -> image saving)
// ---------------------------------------------------------------------------

pub fn run_pipeline(input: &Path, pipeline_path: &Path, output: &Path) -> Result<()> {
    if !input.exists() {
        anyhow::bail!("Input file {} does not exist", input.display());
    }
    if !pipeline_path.exists() {
        anyhow::bail!("Pipeline file {} does not exist", pipeline_path.display());
    }

    // Load pipeline configuration
    let pipeline_json = std::fs::read_to_string(pipeline_path)
        .with_context(|| format!("Failed to read pipeline {}", pipeline_path.display()))?;
    let pipeline: pipeline::Pipeline = serde_json::from_str(&pipeline_json)
        .with_context(|| format!("Failed to parse pipeline {}", pipeline_path.display()))?;

    println!("Loaded pipeline: {} ({})", pipeline.id, pipeline.name);
    for step in &pipeline.steps {
        println!("  step: {} -> {}", step.level, step.module);
    }

    std::fs::create_dir_all(output)
        .with_context(|| format!("Failed to create output directory {}", output.display()))?;

    // For this audited revision we support NOAA APT audio pipelines.
    if pipeline.id == "noaa_apt" || pipeline.id.contains("apt") {
        run_apt_pipeline(input, output, &pipeline)
    } else {
        println!(
            "Pipeline {} is not fully implemented yet; performing passthrough.",
            pipeline.id
        );
        std::fs::copy(input, output.join("output.bin"))?;
        Ok(())
    }
}

fn run_apt_pipeline(input: &Path, output: &Path, _pipeline: &pipeline::Pipeline) -> Result<()> {
    // ------------------------------------------------------------------
    // 1. Source : read WAV audio
    // ------------------------------------------------------------------
    println!("Opening source: {}", input.display());
    let mut reader = hound::WavReader::open(input)
        .with_context(|| format!("Failed to open WAV: {}", input.display()))?;
    let spec = reader.spec();
    if spec.sample_format != hound::SampleFormat::Int || spec.bits_per_sample != 16 {
        anyhow::bail!("Only 16-bit integer WAV files are supported");
    }
    let samples: Vec<f32> = reader
        .samples::<i16>()
        .map(|s| s.map(|v| v as f32 / 32767.0))
        .collect::<Result<Vec<_>, _>>()?;
    let mono = if spec.channels == 2 {
        samples.iter().step_by(2).copied().collect::<Vec<f32>>()
    } else {
        samples
    };
    println!("Read {} audio samples @ {} Hz", mono.len(), spec.sample_rate);

    // ------------------------------------------------------------------
    // 2. DSP : simple low-pass FIR filter (demonstrates DSP integration)
    // ------------------------------------------------------------------
    println!("Running DSP stage...");
    let taps = dsp::fft::low_pass(1.0, spec.sample_rate as f64, 3000.0, 500.0);
    let mut fir = dsp::filter::FirBlock::new(taps);
    let mut filtered = vec![0.0f32; mono.len()];
    let produced = fir.process(&mono, &mut filtered);
    filtered.truncate(produced);
    println!("DSP produced {} samples", filtered.len());

    // ------------------------------------------------------------------
    // 3. Instrument decoder : NOAA APT
    // ------------------------------------------------------------------
    println!("Running APT instrument decoder...");
    let decoder = instrument::AptDecoder::new(spec.sample_rate);
    let result = decoder
        .decode_samples(&filtered)
        .context("APT decoding failed")?;

    // ------------------------------------------------------------------
    // 4. Products : build ImageProduct / DataSet and save
    // ------------------------------------------------------------------
    println!("Building products...");
    let mut dataset = products::DataSet::new();
    dataset.satellite_name = "NOAA".to_string();
    dataset.timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs_f64();

    // Helper to convert AptImage -> crate::image::Image<u16>
    let apt_to_image =
        |apt: &instrument::AptImage| -> anyhow::Result<crate::image::Image<u16>> {
            crate::image::Image::from_data(apt.width, apt.height, 1, apt.data.clone())
                .map_err(|e| anyhow::anyhow!("Image conversion error: {e}"))
        };

    if !result.raw_sync.data.is_empty() {
        let img = apt_to_image(&result.raw_sync)?;
        let path = output.join("raw_sync.png");
        img.save(&path)
            .map_err(|e| anyhow::anyhow!("Failed to save raw_sync: {e}"))?;
    }

    // Channel A
    if let Some(ref cha) = result.channel_a {
        let img = apt_to_image(cha)?;
        let filename = "channel_a.png".to_string();
        let path = output.join(&filename);
        img.save(&path)
            .map_err(|e| anyhow::anyhow!("Failed to save channel_a: {e}"))?;

        let mut ip = products::ImageProduct::new();
        ip.images.push(products::ImageHolder {
            abs_index: 0,
            filename: filename.clone(),
            channel_name: format!("CH-A-{}", result.metadata.channel_a),
            image: img,
            bit_depth: 16,
            wavenumber: -1.0,
            bandwidth: -1.0,
            calibration_type: String::new(),
            polarization: products::ChannelPolarization::None,
        });
        let product_dir = output.join("product_a");
        std::fs::create_dir_all(&product_dir)?;
        ip.save(&product_dir)?;
        dataset.products_list.push("product_a".to_string());
    }

    // Channel B
    if let Some(ref chb) = result.channel_b {
        let img = apt_to_image(chb)?;
        let filename = "channel_b.png".to_string();
        let path = output.join(&filename);
        img.save(&path)
            .map_err(|e| anyhow::anyhow!("Failed to save channel_b: {e}"))?;

        let mut ip = products::ImageProduct::new();
        ip.images.push(products::ImageHolder {
            abs_index: 1,
            filename: filename.clone(),
            channel_name: format!("CH-B-{}", result.metadata.channel_b),
            image: img,
            bit_depth: 16,
            wavenumber: -1.0,
            bandwidth: -1.0,
            calibration_type: String::new(),
            polarization: products::ChannelPolarization::None,
        });
        let product_dir = output.join("product_b");
        std::fs::create_dir_all(&product_dir)?;
        ip.save(&product_dir)?;
        dataset.products_list.push("product_b".to_string());
    }

    dataset.save(output)?;
    println!("Pipeline complete. Products saved to {}", output.display());
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut handler = cli::CommandHandler::new(false);
    handler.add_handler(Arc::new(RunCmd));
    let matches = handler.parse(&args)?;
    handler.run(&matches)?;
    Ok(())
}
