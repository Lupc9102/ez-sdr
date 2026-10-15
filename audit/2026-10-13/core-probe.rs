use ez_daemon::{
    bus::{OverflowPolicy, SampleBlock, SampleBus},
    channelizer::Channelizer,
    hardware::{IqSource, ReadPending},
    state::DaemonState,
};
use ez_proto::{ChannelSpec, PipelineKind};
use num_complex::Complex32;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::time::Duration;

fn spec(id: u32, kind: PipelineKind, hz: u32) -> ChannelSpec {
    ChannelSpec {
        id,
        kind,
        channel_sample_rate_hz: hz,
        center_offset_hz: 0,
        demod_mode: None,
        audio: None,
        meteor_profile: None,
    }
}
struct GatedSource {
    hz: u32,
    begun: mpsc::Sender<()>,
    finish: mpsc::Receiver<()>,
}
impl IqSource for GatedSource {
    fn start(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn stop(&mut self) {}
    fn set_frequency(&mut self, _: u64) -> anyhow::Result<()> {
        Ok(())
    }
    fn set_sample_rate(&mut self, hz: u32) -> anyhow::Result<()> {
        self.begun.send(()).unwrap();
        self.finish.recv().unwrap();
        self.hz = hz;
        Ok(())
    }
    fn set_gain(&mut self, _: f64) -> anyhow::Result<()> {
        Ok(())
    }
    fn read_iq(&mut self, _: &mut [Complex32]) -> anyhow::Result<usize> {
        std::thread::sleep(Duration::from_millis(1));
        Err(ReadPending::Timeout.into())
    }
    fn frequency_hz(&self) -> u64 {
        100_000_000
    }
    fn sample_rate_hz(&self) -> u32 {
        self.hz
    }
    fn gain_db(&self) -> f64 {
        0.0
    }
    fn kind(&self) -> &'static str {
        "gated-audit"
    }
}
fn run_rate_race() {
    let (begun_tx, begun_rx) = mpsc::channel();
    let (finish_tx, finish_rx) = mpsc::channel();
    let bus = SampleBus::new();
    let running = Arc::new(AtomicBool::new(true));
    let (hw, thread) = ez_daemon::ingest::spawn(
        Box::new(GatedSource {
            hz: 2_400_000,
            begun: begun_tx,
            finish: finish_rx,
        }),
        bus.clone(),
        running.clone(),
    )
    .unwrap();
    let state = Arc::new(DaemonState::new(
        bus,
        hw,
        100_000_000,
        2_400_000,
        std::env::temp_dir(),
    ));
    let worker_state = state.clone();
    let worker = std::thread::spawn(move || worker_state.set_sample_rate(1_200_000));
    begun_rx.recv().unwrap();
    state
        .subscribe(spec(1, PipelineKind::Audio, 48_000))
        .unwrap();
    finish_tx.send(()).unwrap();
    println!("RATE_RACE setter={:?}; hardware={} Hz; new 2.4-MHz spectrum accepted={} (stale state geometry)",worker.join().unwrap(),state.hardware_status().sample_rate_hz,state.subscribe(spec(2,PipelineKind::Spectrum,2_400_000)).is_ok());
    running.store(false, Ordering::Relaxed);
    thread.join().unwrap();
    drop(state);
}
fn run_adsb_rate() {
    let bus = SampleBus::new();
    let input = bus.subscribe(2, OverflowPolicy::DropOldest);
    let mut channelizer = Channelizer::new(input, 1_090_000_000, 3_200_000);
    let (_, output) = channelizer.add_channel(1_090_000_000, 2_400_000).unwrap();
    bus.publish(SampleBlock {
        product_session: 1,
        observed_at: None,
        capture: None,
        start_sample: 0,
        sample_rate_hz: 3_200_000,
        center_freq_hz: 1_090_000_000,
        samples: Arc::from(vec![Complex32::new(1.0, 0.0); 8192]),
    });
    channelizer.tick(Duration::ZERO);
    let block = output.try_recv().unwrap();
    println!(
        "ADS_B_TAP requested=2400000 Hz; actual={} Hz; samples={}",
        block.sample_rate_hz,
        block.samples.len()
    );
    let mut src = ez_daemon::hardware::synthetic::SyntheticSource::default();
    src.set_sample_rate(3_200_000).unwrap();
    src.set_frequency(1_090_000_000).unwrap();
    let running = Arc::new(AtomicBool::new(true));
    let (hw, thread) =
        ez_daemon::ingest::spawn(Box::new(src), bus.clone(), running.clone()).unwrap();
    let state = DaemonState::new(bus, hw, 1_090_000_000, 3_200_000, std::env::temp_dir());
    println!(
        "ADS_B_STATE accepted={}",
        state
            .subscribe(spec(3, PipelineKind::AdsbPackets, 2_400_000))
            .is_ok()
    );
    running.store(false, Ordering::Relaxed);
    thread.join().unwrap();
    drop(state);
}
fn ingest_threads() -> usize {
    std::fs::read_dir("/proc/self/task")
        .unwrap()
        .filter_map(Result::ok)
        .filter(|p| {
            std::fs::read_to_string(p.path().join("comm"))
                .is_ok_and(|n| n.trim().starts_with("ez-daemon-inges"))
        })
        .count()
}
fn run_bind_leak() {
    let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let before = ingest_threads();
    let running = Arc::new(AtomicBool::new(true));
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let result = rt.block_on(ez_daemon::run(
        ez_daemon::DaemonConfig {
            listen_addr: held.local_addr().unwrap(),
            web_listen_addr: "127.0.0.1:0".parse().unwrap(),
            initial_freq_hz: 100_000_000,
            initial_sample_rate_hz: 2_000_000,
            recording_dir: std::env::temp_dir(),
            web_static_dir: std::env::temp_dir(),
            auth_token: None,
            source: ez_daemon::SourceConfig::Synthetic,
            receiver_position: None,
        },
        running.clone(),
    ));
    std::thread::sleep(Duration::from_millis(150));
    println!(
        "BIND_FAILURE result={:?}; running={}; ingest threads before={}, after={}",
        result,
        running.load(Ordering::Relaxed),
        before,
        ingest_threads()
    );
    running.store(false, Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(150));
}
fn main() {
    run_rate_race();
    run_adsb_rate();
    run_bind_leak();
}
