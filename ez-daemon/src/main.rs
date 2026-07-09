use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::anyhow;
use clap::{Parser, ValueEnum};

use ez_daemon::hardware::replay::ReplayFormat;
use ez_daemon::{DaemonConfig, SourceConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum SourceKind {
    #[value(name = "synthetic")]
    Synthetic,
    #[value(name = "replay")]
    Replay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ReplayFormatArg {
    #[value(name = "raw-u8")]
    RawU8,
    #[value(name = "cf32-le")]
    Cf32Le,
}

impl From<ReplayFormatArg> for ReplayFormat {
    fn from(value: ReplayFormatArg) -> Self {
        match value {
            ReplayFormatArg::RawU8 => ReplayFormat::RawU8,
            ReplayFormatArg::Cf32Le => ReplayFormat::Cf32Le,
        }
    }
}

/// `ez-sdr` headless daemon: owns hardware ingestion, wideband channelization, and
/// multi-tenant DSP pipeline routing. GUI and CLI clients attach over the network on-demand
/// and never block or interrupt daemon-side processing by attaching or detaching.
#[derive(Debug, Parser)]
#[command(name = "ez-daemon", version)]
struct Args {
    /// TCP address to accept client connections on.
    #[arg(long, default_value = "127.0.0.1:7890")]
    listen: SocketAddr,

    /// Initial center frequency in Hz.
    #[arg(long, default_value_t = 433_000_000)]
    freq: u64,

    /// Initial sample rate in Hz.
    #[arg(long, default_value_t = 2_048_000)]
    sample_rate: u32,

    /// Directory recorded IQ files are written to.
    #[arg(long, default_value = "./recordings")]
    recording_dir: PathBuf,

    /// IQ source to ingest from.
    #[arg(long, value_enum, default_value = "synthetic")]
    source: SourceKind,

    /// Path to a captured IQ file. Required when `--source replay`.
    #[arg(long)]
    replay_file: Option<String>,

    /// On-disk sample format of `--replay-file`.
    #[arg(long, value_enum, default_value = "cf32-le")]
    replay_format: ReplayFormatArg,

    /// Loop the replay file instead of stopping at end-of-file.
    #[arg(long)]
    replay_loop: bool,

    /// Playback speed multiplier (1.0 = real time).
    #[arg(long, default_value_t = 1.0)]
    replay_speed: f32,
}

impl Args {
    fn into_daemon_config(self) -> anyhow::Result<DaemonConfig> {
        let source = match self.source {
            SourceKind::Synthetic => SourceConfig::Synthetic,
            SourceKind::Replay => SourceConfig::Replay {
                path: self
                    .replay_file
                    .ok_or_else(|| anyhow!("--replay-file is required when --source replay"))?,
                format: self.replay_format.into(),
                looping: self.replay_loop,
                speed: self.replay_speed,
            },
        };
        Ok(DaemonConfig {
            listen_addr: self.listen,
            initial_freq_hz: self.freq,
            initial_sample_rate_hz: self.sample_rate,
            recording_dir: self.recording_dir,
            source,
        })
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    tracing::info!(?args, "ez-daemon starting");
    let config = args.into_daemon_config()?;

    let running = Arc::new(AtomicBool::new(true));
    let ctrlc_running = Arc::clone(&running);

    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    runtime.spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            tracing::info!("ctrl-c received, shutting down");
            ctrlc_running.store(false, Ordering::Relaxed);
        }
    });

    runtime.block_on(ez_daemon::run(config, running))
}
