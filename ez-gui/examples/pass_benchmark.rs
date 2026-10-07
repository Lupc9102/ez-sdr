//! Offline timing of the supported orbit catalog and its cache boundaries.
//! Run with `cargo run -p ez-gui --example pass_benchmark` (or `--release`).
//! This measures synchronous CPU/lock occupancy, not native display latency.

use ez_gui::tle_engine::TleEngine;
use std::hint::black_box;
use std::sync::Mutex;
use std::time::Instant;

fn measure(engine: &Mutex<TleEngine>) -> u64 {
    let mut engine = engine.lock().unwrap();
    let started = Instant::now();
    black_box(engine.upcoming_passes());
    started.elapsed().as_micros() as u64
}

fn measure_tracking_refresh(engine: &Mutex<TleEngine>, satellite: &str) -> u64 {
    let mut engine = engine.lock().unwrap();
    let started = Instant::now();
    black_box(engine.upcoming_passes());
    black_box(engine.pass_trajectory(satellite, engine.observer_lat, engine.observer_lon, 6.0));
    started.elapsed().as_micros() as u64
}

fn report(case: &str, mut times: Vec<u64>) {
    times.sort_unstable();
    let percentile = |p: usize| times[((times.len() * p).div_ceil(100) - 1).min(times.len() - 1)];
    println!(
        "{case}: n={}, p50={} us, p95={} us, p99={} us, max={} us",
        times.len(),
        percentile(50),
        percentile(95),
        percentile(99),
        times.last().unwrap()
    );
}

fn main() {
    const IMPORT: &str = "ISS\n1 25544U 98067A   26263.14255447  .00007470  00000+0  14267-3 0  9991\n2 25544  51.6307 190.1401 0004820 160.6694 199.4478 15.49188396586472";
    let engine = Mutex::new(TleEngine::new());
    let catalog = engine.lock().unwrap().catalog().len();
    println!(
        "Supported catalog: {catalog} satellites; 72-hour pass horizon; optimized={}; UTC={}",
        !cfg!(debug_assertions),
        chrono::Utc::now().to_rfc3339()
    );
    assert_eq!(
        catalog, 3,
        "update the benchmark when catalog support grows"
    );
    let mut cold = Vec::new();
    let mut warm = Vec::new();
    let mut imported = Vec::new();
    let mut tracking = Vec::new();
    for _ in 0..30 {
        for ((lat, lon), satellite) in [(51.5, -0.1), (-33.8688, 151.2093), (40.7128, -74.0060)]
            .into_iter()
            .zip(["Meteor-M2-3", "Meteor-M2-4", "ISS"])
        {
            engine.lock().unwrap().set_observer(lat, lon);
            cold.push(measure(&engine));
            warm.push(measure(&engine));
            engine
                .lock()
                .unwrap()
                .update_tles_from_text(IMPORT)
                .unwrap();
            imported.push(measure(&engine));
            engine
                .lock()
                .unwrap()
                .update_tles_from_text(IMPORT)
                .unwrap();
            tracking.push(measure_tracking_refresh(&engine, satellite));
        }
    }
    report("observer invalidation (lock held)", cold);
    report("unchanged cache (lock held)", warm);
    report("catalog invalidation (lock held)", imported);
    report(
        "cold passes + selected six-hour trajectory (lock held)",
        tracking,
    );
    println!("No network, USB, speaker callbacks or user catalog changes.");
}
