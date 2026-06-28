use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

use crate::state::SharedState;

pub fn start(state: Arc<Mutex<SharedState>>, _event_tx: broadcast::Sender<String>) {
    std::thread::spawn(move || {
        let mut phase: f64 = 0.0;
        let mut buf = vec![0u8; 16384];

        loop {
            std::thread::sleep(std::time::Duration::from_millis(1));

            let (freq, _rate, running) = {
                let s = state.lock().unwrap();
                (s.source.frequency_hz, s.source.sample_rate_hz, s.source.running)
            };

            if !running {
                std::thread::sleep(std::time::Duration::from_millis(100));
                continue;
            }

            let freq_mhz = freq as f64 / 1e6;

            for i in (0..buf.len()).step_by(2) {
                let signal = (2.0 * std::f64::consts::PI * phase * freq_mhz * 0.000001).sin() * 25.0;
                let noise = ((phase * 7.13).sin() * 3.0) + ((phase * 13.7).cos() * 2.0);
                let sample = (signal + noise + 127.5).clamp(0.0, 255.0) as u8;
                buf[i] = sample;
                buf[i + 1] = sample;
                phase += 1.0;
            }

            {
                let mut s = state.lock().unwrap();
                s.push_iq(&buf);
            }
        }
    });
}
