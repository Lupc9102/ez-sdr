//! Raw IQ byte <-> `Complex32` conversion, plus recording sidecar parsing.
//!
//! Recordings are raw interleaved unsigned-8-bit I/Q byte pairs (RTL-SDR
//! "Uc8" convention). Uses the exact same `f32::from(byte) - 127.4` offset
//! convention as `ez-gui/src/spectrum.rs::push_iq_samples` for consistency
//! across the codebase.

use num_complex::Complex32;
use serde::Deserialize;
use std::path::Path;

/// Convert raw interleaved u8 IQ bytes to `Complex32` samples.
///
/// Any trailing unpaired byte (odd-length input) is ignored.
#[must_use]
pub fn iq_bytes_to_complex(bytes: &[u8]) -> Vec<Complex32> {
    let n = bytes.len() / 2;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let i_val = f32::from(bytes[2 * i]) - 127.4;
        let q_val = f32::from(bytes[2 * i + 1]) - 127.4;
        out.push(Complex32::new(i_val, q_val));
    }
    out
}

/// Metadata sidecar written by `RecorderPanel` alongside a `.iq` recording
/// (see `ez-gui/src/recorder_panel.rs`). Only the fields useful for
/// decoding are parsed; unknown fields are ignored by `serde`.
#[derive(Debug, Clone, Deserialize)]
pub struct RecordingSidecar {
    pub frequency_hz: u64,
    #[serde(default)]
    pub frequency_mhz: f64,
    pub sample_rate_hz: u32,
    #[serde(default)]
    pub demod_mode: String,
    #[serde(default)]
    pub gain_db: f64,
    #[serde(default)]
    pub ppm_correction: i32,
    #[serde(default)]
    pub timestamp_utc: String,
    #[serde(default)]
    pub files: Vec<String>,
}

/// Look for a sibling `.json` sidecar next to `iq_path` (e.g. `foo.iq` ->
/// `foo.json`) and parse it. Returns `None` if no sidecar exists or it
/// fails to parse.
#[must_use]
pub fn load_sidecar(iq_path: &Path) -> Option<RecordingSidecar> {
    let sidecar_path = iq_path.with_extension("json");
    let text = std::fs::read_to_string(sidecar_path).ok()?;
    serde_json::from_str(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_byte_maps_to_negative_offset() {
        let out = iq_bytes_to_complex(&[0, 0]);
        assert_eq!(out.len(), 1);
        assert!((out[0].re - (-127.4)).abs() < 1e-6);
        assert!((out[0].im - (-127.4)).abs() < 1e-6);
    }

    #[test]
    fn mid_value_maps_near_zero() {
        // 127 -> -0.4, 128 -> 0.6
        let out = iq_bytes_to_complex(&[127, 128]);
        assert!((out[0].re - (-0.4)).abs() < 1e-5);
        assert!((out[0].im - 0.6).abs() < 1e-5);
    }

    #[test]
    fn odd_trailing_byte_is_ignored() {
        let out = iq_bytes_to_complex(&[10, 20, 30]);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn multiple_pairs_convert_in_order() {
        let out = iq_bytes_to_complex(&[0, 255, 255, 0]);
        assert_eq!(out.len(), 2);
        assert!((out[0].re - (-127.4)).abs() < 1e-5);
        assert!((out[0].im - 127.6).abs() < 1e-5);
        assert!((out[1].re - 127.6).abs() < 1e-5);
        assert!((out[1].im - (-127.4)).abs() < 1e-5);
    }

    #[test]
    fn load_sidecar_parses_recorder_panel_shape() {
        let dir = std::env::temp_dir().join(format!("lrpt_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let iq_path = dir.join("rec.iq");
        let json_path = dir.join("rec.json");
        std::fs::write(
            &json_path,
            r#"{
  "frequency_hz": 137100000,
  "frequency_mhz": 137.100000,
  "sample_rate_hz": 2048000,
  "demod_mode": "RAW",
  "gain_db": 30.0,
  "ppm_correction": 0,
  "timestamp_utc": "2026-01-01T00:00:00Z",
  "files": ["rec.iq"]
}
"#,
        )
        .unwrap();
        let sidecar = load_sidecar(&iq_path).expect("sidecar should parse");
        assert_eq!(sidecar.frequency_hz, 137_100_000);
        assert_eq!(sidecar.sample_rate_hz, 2_048_000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_sidecar_returns_none_when_missing() {
        let path = Path::new("/nonexistent/path/does_not_exist.iq");
        assert!(load_sidecar(path).is_none());
    }
}
