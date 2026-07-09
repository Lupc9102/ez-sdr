use clap::Parser;

/// `ez-sdr` headless daemon: owns hardware ingestion, wideband channelization, and
/// multi-tenant DSP pipeline routing. GUI and CLI clients attach over the network on-demand
/// and never block or interrupt daemon-side processing by attaching or detaching.
#[derive(Debug, Parser)]
#[command(name = "ez-daemon", version)]
struct Args {
    /// TCP address to accept client connections on.
    #[arg(long, default_value = "127.0.0.1:7890")]
    listen: String,

    /// Initial center frequency in Hz.
    #[arg(long, default_value_t = 433_000_000)]
    freq: u64,

    /// Initial sample rate in Hz.
    #[arg(long, default_value_t = 2_048_000)]
    sample_rate: u32,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    tracing::info!(
        listen = %args.listen,
        freq = args.freq,
        sample_rate = args.sample_rate,
        "ez-daemon starting"
    );

    // TODO(task #12): wire up SampleBus + channelizer + pipelines + TCP server once those
    // modules land; this entry point becomes `ez_daemon::run(args.into())`.
    Ok(())
}
