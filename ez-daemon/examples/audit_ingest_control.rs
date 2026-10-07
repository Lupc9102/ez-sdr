//! Models the real RTL/HackRF/Soapy setters' cache-before-error behavior without hardware.
//! Run: cargo run -p ez-daemon --example audit_ingest_control
use ez_daemon::{
    bus::{OverflowPolicy, SampleBus},
    hardware::IqSource,
    ingest,
};
use num_complex::Complex32;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

struct RejectedTune {
    cached_frequency: u64,
}
impl IqSource for RejectedTune {
    fn start(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn stop(&mut self) {}
    fn set_frequency(&mut self, hz: u64) -> anyhow::Result<()> {
        self.cached_frequency = hz; // Matches the real backends before their fallible FFI call.
        anyhow::bail!("injected hardware rejection")
    }
    fn set_sample_rate(&mut self, _: u32) -> anyhow::Result<()> {
        Ok(())
    }
    fn set_gain(&mut self, _: f64) -> anyhow::Result<()> {
        Ok(())
    }
    fn read_iq(&mut self, buf: &mut [Complex32]) -> anyhow::Result<usize> {
        std::thread::sleep(Duration::from_millis(2));
        buf.fill(Complex32::new(0.5, 0.0));
        Ok(buf.len())
    }
    fn frequency_hz(&self) -> u64 {
        self.cached_frequency
    }
    fn sample_rate_hz(&self) -> u32 {
        2_000_000
    }
    fn gain_db(&self) -> f64 {
        0.0
    }
    fn kind(&self) -> &'static str {
        "audit-injected-rejection"
    }
}
fn main() {
    let bus = SampleBus::new();
    let rx = bus.subscribe(16, OverflowPolicy::DropOldest);
    let running = Arc::new(AtomicBool::new(true));
    let (handle, thread) = ingest::spawn(
        Box::new(RejectedTune {
            cached_frequency: 100_000_000,
        }),
        bus,
        Arc::clone(&running),
    )
    .unwrap();
    let before = rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let result = handle.set_frequency(101_000_000);
    let status = handle.status();
    let after = loop {
        let block = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        if block.center_freq_hz == 101_000_000 {
            break block;
        }
    };
    running.store(false, Ordering::Relaxed);
    thread.join().unwrap();
    println!("rejected_tune_error={} reported_frequency={} next_block_frequency={} capture_epoch_changed={} next_block_discontinuous={}",result.is_err(),status.frequency_hz,after.center_freq_hz,before.capture_clock().epoch!=after.capture_clock().epoch,after.capture_clock().discontinuous);
}
