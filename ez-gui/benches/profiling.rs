//! Performance analysis and hot-path identification.

use std::time::Instant;

/// Profile keyboard event processing throughput
pub fn profile_keyboard_processing() {
    println!("\n=== Keyboard Processing Profile ===");

    let iterations = 100_000;
    let start = Instant::now();

    // Simulate keyboard event detection/routing
    for i in 0..iterations {
        let _ = (i % 256) as u8; // Key scanning
        let _ = i % 2; // Modifier detection
    }

    let elapsed = start.elapsed();
    let per_event = elapsed.as_nanos() as f64 / iterations as f64;

    println!("  {} keyboard events processed", iterations);
    println!("  Total time: {:.2}ms", elapsed.as_secs_f64() * 1000.0);
    println!("  Per-event time: {:.1}ns", per_event);

    if per_event > 100.0 {
        println!("  ⚠️  WARNING: High per-event latency detected");
    }
}

/// Profile lock contention on shared state
pub fn profile_lock_contention() {
    use std::sync::{Arc, Mutex};
    use std::thread;

    println!("\n=== Lock Contention Profile ===");

    let shared = Arc::new(Mutex::new(0u64));
    let mut handles = vec![];

    let start = Instant::now();
    let iterations = 10_000;

    for _ in 0..4 {
        let shared = Arc::clone(&shared);
        let handle = thread::spawn(move || {
            for _ in 0..iterations {
                if let Ok(mut val) = shared.lock() {
                    *val += 1;
                }
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    let elapsed = start.elapsed();
    println!("  4 threads × {} lock acquisitions", iterations);
    println!("  Total time: {:.2}ms", elapsed.as_secs_f64() * 1000.0);
    println!(
        "  Per-lock time: {:.2}µs",
        elapsed.as_micros() as f64 / (4 * iterations) as f64
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_and_report() {
        profile_keyboard_processing();
        profile_lock_contention();
    }
}
