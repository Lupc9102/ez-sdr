//! Raw IQ byte <-> `Complex32` conversion.
//!
//! Two source formats are supported:
//! - Raw interleaved unsigned-8-bit I/Q byte pairs (RTL-SDR "Uc8"
//!   convention), decoded by [`iq_bytes_to_complex`] using the exact same
//!   `f32::from(byte) - 127.4` offset convention as
//!   `ez-gui/src/spectrum.rs::push_iq_samples`.
//! - Interleaved little-endian float32 I/Q pairs (the current `.cf32`
//!   recorder format — see
//!   `ez-gui/src/satellite/recorder.rs::raw_iq_bytes_to_cf32_le`), decoded
//!   by [`cf32_le_bytes_to_complex`].

use num_complex::Complex32;

/// CF32 component units. New recordings use normalized full-scale units;
/// `AdcCounts127_4` explicitly imports legacy byte-minus-127.4 captures.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cf32Amplitude {
    #[default]
    Normalized,
    #[serde(rename = "adc_counts_127_4")]
    AdcCounts127_4,
}

impl Cf32Amplitude {
    #[must_use]
    pub fn to_normalized(self, value: f32) -> f32 {
        if !value.is_finite() {
            return 0.0;
        }
        match self {
            Self::Normalized => value,
            Self::AdcCounts127_4 => (value - 0.1) / 127.5,
        }
    }
}

/// Resolve units from bounded recording metadata or an explicit user choice.
/// An unannotated foreign CF32 file defaults to normalized full-scale units.
/// Only the known legacy satellite-recorder schema implies raw ADC counts;
/// amplitude is never guessed from sample peaks.
pub fn cf32_amplitude_for_path(
    path: &std::path::Path,
    explicit: Option<Cf32Amplitude>,
) -> Result<Cf32Amplitude, String> {
    if let Some(amplitude) = explicit {
        return Ok(amplitude);
    }
    let appended = std::path::PathBuf::from(format!("{}.json", path.display()));
    for sidecar in [appended, path.with_extension("json")] {
        let metadata = match std::fs::metadata(&sidecar) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("Could not read {}: {error}", sidecar.display())),
        };
        if !metadata.is_file() || metadata.len() > 65_536 {
            return Err(format!(
                "CF32 metadata must be a regular file under 64 KiB: {}",
                sidecar.display()
            ));
        }
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(&sidecar)
            .map_err(|e| e.to_string())?
            .take(65_537)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 65_536 {
            return Err("CF32 metadata exceeds 64 KiB".into());
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|e| format!("Invalid CF32 metadata {}: {e}", sidecar.display()))?;
        if let Some(amplitude) = value.get("amplitude") {
            return serde_json::from_value(amplitude.clone())
                .map_err(|e| format!("Unsupported CF32 amplitude: {e}"));
        }
        if value["format"] == "cf32_le"
            && [
                "frequency_hz",
                "sample_rate_hz",
                "start_utc",
                "satellite",
                "observer_lat",
                "observer_lon",
                "bytes_total",
                "duration_sec",
            ]
            .iter()
            .all(|field| value.get(*field).is_some())
        {
            return Ok(Cf32Amplitude::AdcCounts127_4);
        }
        return Ok(Cf32Amplitude::Normalized);
    }
    Ok(Cf32Amplitude::Normalized)
}

/// Symmetric full-scale CU8 conversion used by daemon ingestion and CF32 files.
#[must_use]
pub fn uc8_bytes_to_normalized(bytes: &[u8]) -> Vec<Complex32> {
    bytes
        .chunks_exact(2)
        .map(|pair| {
            Complex32::new(
                (f32::from(pair[0]) - 127.5) / 127.5,
                (f32::from(pair[1]) - 127.5) / 127.5,
            )
        })
        .collect()
}

#[must_use]
pub fn normalized_component_to_uc8(component: f32) -> u8 {
    if !component.is_finite() {
        return 128;
    }
    (component.clamp(-1.0, 1.0) * 127.5 + 127.5).round() as u8
}

#[must_use]
pub fn uc8_bytes_to_cf32_le(bytes: &[u8]) -> Vec<u8> {
    uc8_bytes_to_normalized(bytes)
        .iter()
        .flat_map(|sample| {
            sample
                .re
                .to_le_bytes()
                .into_iter()
                .chain(sample.im.to_le_bytes())
        })
        .collect()
}

#[must_use]
pub fn cf32_le_bytes_to_normalized(bytes: &[u8], amplitude: Cf32Amplitude) -> Vec<Complex32> {
    cf32_le_bytes_to_complex(bytes)
        .into_iter()
        .map(|sample| {
            Complex32::new(
                amplitude.to_normalized(sample.re),
                amplitude.to_normalized(sample.im),
            )
        })
        .collect()
}

#[must_use]
pub fn cf32_le_bytes_to_uc8(bytes: &[u8], amplitude: Cf32Amplitude) -> Vec<u8> {
    cf32_le_bytes_to_normalized(bytes, amplitude)
        .iter()
        .flat_map(|sample| {
            [
                normalized_component_to_uc8(sample.re),
                normalized_component_to_uc8(sample.im),
            ]
        })
        .collect()
}

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

/// Convert interleaved signed 8-bit I/Q samples (`.cs8`) to complex samples.
/// Unlike RTL-SDR `.cu8`, signed recordings are already centered around zero.
#[must_use]
pub fn cs8_bytes_to_complex(bytes: &[u8]) -> Vec<Complex32> {
    let n = bytes.len() / 2;
    (0..n)
        .map(|i| Complex32::new(bytes[i * 2] as i8 as f32, bytes[i * 2 + 1] as i8 as f32))
        .collect()
}

/// Parse LE-float32-interleaved I/Q bytes without converting component units.
/// Importers should use `cf32_le_bytes_to_normalized` with explicit units. 8 bytes per sample pair (4-byte LE f32 I, 4-byte LE f32 Q).
/// A trailing partial pair is ignored.
#[must_use]
pub fn cf32_le_bytes_to_complex(bytes: &[u8]) -> Vec<Complex32> {
    let n = bytes.len() / 8;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let off = i * 8;
        let re = f32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
        let im = f32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap());
        out.push(Complex32::new(re, im));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cf32_units_round_trip_every_adc_value_and_complex_tones() {
        let raw: Vec<u8> = (0..=255).flat_map(|i| [i, 255 - i]).collect();
        let normalized = uc8_bytes_to_cf32_le(&raw);
        assert_eq!(
            cf32_le_bytes_to_uc8(&normalized, Cf32Amplitude::Normalized),
            raw
        );
        let legacy: Vec<u8> = raw
            .iter()
            .flat_map(|v| (f32::from(*v) - 127.4).to_le_bytes())
            .collect();
        assert_eq!(
            cf32_le_bytes_to_uc8(&legacy, Cf32Amplitude::AdcCounts127_4),
            raw
        );
        for amplitude in [0.001, 0.05, 0.5, 1.0] {
            let original: Vec<_> = (0..1024)
                .map(|n| Complex32::from_polar(amplitude, n as f32 * 0.13))
                .collect();
            let bytes: Vec<u8> = original
                .iter()
                .flat_map(|s| {
                    [
                        normalized_component_to_uc8(s.re),
                        normalized_component_to_uc8(s.im),
                    ]
                })
                .collect();
            let decoded = uc8_bytes_to_normalized(&cf32_le_bytes_to_uc8(
                &uc8_bytes_to_cf32_le(&bytes),
                Cf32Amplitude::Normalized,
            ));
            for (a, b) in original.iter().zip(decoded) {
                assert!((a.re - b.re).abs() <= 1.0 / 255.0 + 1e-6);
                assert!((a.im - b.im).abs() <= 1.0 / 255.0 + 1e-6);
            }
        }
    }

    #[test]
    fn cf32_metadata_selects_only_explicit_or_recognized_legacy_units() {
        let dir = std::env::temp_dir().join(format!("ez-cf32-units-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("input.cf32");
        let sidecar = path.with_extension("json");
        assert_eq!(
            cf32_amplitude_for_path(&path, None).unwrap(),
            Cf32Amplitude::Normalized
        );
        std::fs::write(&sidecar, br#"{"format":"cf32_le","frequency_hz":1,"sample_rate_hz":2400000,"start_utc":"old","satellite":null,"observer_lat":0,"observer_lon":0,"bytes_total":16,"duration_sec":1}"#).unwrap();
        assert_eq!(
            cf32_amplitude_for_path(&path, None).unwrap(),
            Cf32Amplitude::AdcCounts127_4
        );
        assert_eq!(
            cf32_amplitude_for_path(&path, Some(Cf32Amplitude::Normalized)).unwrap(),
            Cf32Amplitude::Normalized
        );
        std::fs::write(&sidecar, br#"{"amplitude":"normalized"}"#).unwrap();
        assert_eq!(
            cf32_amplitude_for_path(&path, None).unwrap(),
            Cf32Amplitude::Normalized
        );
        std::fs::write(&sidecar, br#"{"amplitude":"unknown"}"#).unwrap();
        assert!(cf32_amplitude_for_path(&path, None).is_err());
        std::fs::write(&sidecar, vec![b' '; 65537]).unwrap();
        assert!(cf32_amplitude_for_path(&path, None).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

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
    fn cs8_samples_are_signed_and_centered() {
        let out = cs8_bytes_to_complex(&[0x80, 0x7f, 0xff, 0x00]);
        assert_eq!(
            out,
            vec![Complex32::new(-128.0, 127.0), Complex32::new(-1.0, 0.0)]
        );
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
    fn cf32_le_zero_bytes_map_to_zero() {
        let out = cf32_le_bytes_to_complex(&[0u8; 8]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].re, 0.0);
        assert_eq!(out[0].im, 0.0);
    }

    #[test]
    fn cf32_le_known_value_round_trip() {
        let re: f32 = 0.25;
        let im: f32 = -0.75;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&re.to_le_bytes());
        bytes.extend_from_slice(&im.to_le_bytes());
        let out = cf32_le_bytes_to_complex(&bytes);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].re, re);
        assert_eq!(out[0].im, im);
    }

    #[test]
    fn cf32_le_trailing_partial_pair_ignored() {
        let re: f32 = 1.0;
        let im: f32 = 2.0;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&re.to_le_bytes());
        bytes.extend_from_slice(&im.to_le_bytes());
        bytes.extend_from_slice(&[1, 2, 3]); // trailing partial pair
        let out = cf32_le_bytes_to_complex(&bytes);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].re, re);
        assert_eq!(out[0].im, im);
    }
}
