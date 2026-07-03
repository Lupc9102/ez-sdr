//! Sample format conversion - translated from convert.c

/// Identity pass-through for buffers that are already magnitude samples.
/// `SdrSource::read_samples` returns u16 magnitudes, so no conversion is needed.
#[must_use]
pub fn to_magnitude(samples: &[u16]) -> &[u16] {
    samples
}

/// Supported IQ sample formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IqFormat {
    /// Two unsigned bytes per sample: I, Q (RTL-SDR default)
    Uc8,
    /// Two signed 16-bit little-endian values per sample
    Sc16,
    /// Two signed 16-bit little-endian values per sample, Q11 normalised
    Sc16Q11,
}

/// Convert UC8 IQ bytes to magnitude u16 samples.
/// `src` must have at least `samples * 2` bytes.
/// `dst` must have at least `samples` elements.
pub fn convert_uc8_to_mag(src: &[u8], dst: &mut [u16]) {
    let samples = (src.len() / 2).min(dst.len());
    for i in 0..samples {
        let fi = f32::from(src[i * 2]) - 127.4;
        let fq = f32::from(src[i * 2 + 1]) - 127.4;
        let mag = ((fi * fi + fq * fq).sqrt() * 512.0).min(65535.0);
        dst[i] = mag as u16;
    }
}

/// Convert SC16 IQ bytes to magnitude u16 samples.
pub fn convert_sc16_to_mag(src: &[u8], dst: &mut [u16]) {
    let samples = (src.len() / 4).min(dst.len());
    for i in 0..samples {
        let i_bytes = [src[i * 4], src[i * 4 + 1]];
        let q_bytes = [src[i * 4 + 2], src[i * 4 + 3]];
        let i_val = f32::from(i16::from_le_bytes(i_bytes).abs());
        let q_val = f32::from(i16::from_le_bytes(q_bytes).abs());
        let mag = ((i_val * i_val + q_val * q_val).sqrt() * 2.0).min(65535.0);
        dst[i] = mag as u16;
    }
}

/// Convert SC16Q11 IQ bytes to magnitude u16 samples.
pub fn convert_sc16q11_to_mag(src: &[u8], dst: &mut [u16]) {
    let samples = (src.len() / 4).min(dst.len());
    for i in 0..samples {
        let i_bytes = [src[i * 4], src[i * 4 + 1]];
        let q_bytes = [src[i * 4 + 2], src[i * 4 + 3]];
        let i_val = f32::from(i16::from_le_bytes(i_bytes).abs());
        let q_val = f32::from(i16::from_le_bytes(q_bytes).abs());
        let mag = ((i_val * i_val + q_val * q_val).sqrt() * 32.0).min(65535.0);
        dst[i] = mag as u16;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_magnitude_passthrough() {
        let data = [100u16, 200, 300];
        assert_eq!(to_magnitude(&data), &data as &[u16]);
    }

    #[test]
    fn convert_uc8_to_mag_zero_input() {
        let src = [128u8, 128, 128, 128];
        let mut dst = [0u16; 2];
        convert_uc8_to_mag(&src, &mut dst);
        assert!(dst[0] > 0);
        assert_eq!(dst[0], dst[1]);
    }

    #[test]
    fn convert_uc8_to_mag_max_input() {
        let src = [255u8, 255];
        let mut dst = [0u16; 1];
        convert_uc8_to_mag(&src, &mut dst);
        assert_eq!(dst[0], 65535);
    }

    #[test]
    fn convert_uc8_to_mag_shorter_dst() {
        let src = [100u8, 150, 200, 250];
        let mut dst = [0u16; 1];
        convert_uc8_to_mag(&src, &mut dst);
        assert_ne!(dst[0], 0);
    }

    #[test]
    fn convert_sc16_to_mag_known() {
        let src = [100u8, 0, 0, 0, 0, 0, 0, 0];
        let mut dst = [0u16; 2];
        convert_sc16_to_mag(&src, &mut dst);
        assert_eq!(dst[0], 200);
        assert_eq!(dst[1], 0);
    }

    #[test]
    fn convert_sc16_to_mag_negative_iq() {
        let src = [0xFFu8, 0xFF, 0xFF, 0xFF];
        let mut dst = [0u16; 1];
        convert_sc16_to_mag(&src, &mut dst);
        assert_eq!(dst[0], 2);
    }

    #[test]
    fn convert_sc16q11_to_mag_known() {
        let src = [100u8, 0, 0, 0];
        let mut dst = [0u16; 1];
        convert_sc16q11_to_mag(&src, &mut dst);
        assert_eq!(dst[0], 3200);
    }

    #[test]
    fn convert_sc16q11_to_mag_clamp() {
        let src = [0xFFu8, 0x7F, 0, 0];
        let mut dst = [0u16; 1];
        convert_sc16q11_to_mag(&src, &mut dst);
        assert_eq!(dst[0], 65535);
    }

    // ── Additional edge-case tests ──

    #[test]
    fn to_magnitude_empty() {
        let data: &[u16] = &[];
        assert!(to_magnitude(data).is_empty());
    }

    #[test]
    fn iq_format_debug_and_eq() {
        assert_eq!(format!("{:?}", IqFormat::Uc8), "Uc8");
        assert_eq!(format!("{:?}", IqFormat::Sc16), "Sc16");
        assert_eq!(format!("{:?}", IqFormat::Sc16Q11), "Sc16Q11");
        assert_eq!(IqFormat::Uc8, IqFormat::Uc8);
        assert_ne!(IqFormat::Uc8, IqFormat::Sc16);
    }

    #[test]
    fn convert_uc8_to_mag_empty_src() {
        let mut dst = [0u16; 4];
        convert_uc8_to_mag(&[], &mut dst);
        assert_eq!(dst, [0, 0, 0, 0]);
    }

    #[test]
    fn convert_uc8_to_mag_empty_dst() {
        let src = [128u8, 128, 128, 128];
        convert_uc8_to_mag(&src, &mut []);
    }

    #[test]
    fn convert_uc8_to_mag_asymmetric_iq() {
        let src = [200u8, 100];
        let mut dst = [0u16; 1];
        convert_uc8_to_mag(&src, &mut dst);
        assert!(dst[0] > 0);
    }

    #[test]
    fn convert_uc8_to_mag_shorter_src() {
        let src = [255u8, 0];
        let mut dst = [0u16; 10];
        convert_uc8_to_mag(&src, &mut dst);
        assert_eq!(dst[0], 65535);
        assert_eq!(dst[1], 0);
    }

    #[test]
    fn convert_sc16_to_mag_empty_src() {
        let mut dst = [0u16; 4];
        convert_sc16_to_mag(&[], &mut dst);
        assert_eq!(dst, [0, 0, 0, 0]);
    }

    #[test]
    fn convert_sc16_to_mag_empty_dst() {
        let src = [100u8, 0, 0, 0, 200, 0, 0, 0];
        convert_sc16_to_mag(&src, &mut []);
    }

    #[test]
    fn convert_sc16_to_mag_all_zero() {
        let src = [0u8; 8];
        let mut dst = [0u16; 2];
        convert_sc16_to_mag(&src, &mut dst);
        assert_eq!(dst, [0, 0]);
    }

    #[test]
    fn convert_sc16_to_mag_shorter_dst() {
        let src = [100u8, 0, 0, 0, 200, 0, 0, 0];
        let mut dst = [0u16; 1];
        convert_sc16_to_mag(&src, &mut dst);
        assert_eq!(dst[0], 200);
    }

    #[test]
    fn convert_sc16_to_mag_max_iq() {
        // i16::MAX = 32767
        let src = [0xFFu8, 0x7F, 0xFF, 0x7F];
        let mut dst = [0u16; 1];
        convert_sc16_to_mag(&src, &mut dst);
        // sqrt(32767^2 + 32767^2) * 2 ≈ 92681, clamped to 65535
        assert_eq!(dst[0], 65535);
    }

    #[test]
    fn convert_sc16_to_mag_both_negative() {
        // i = -200, q = -200 → abs both 200
        let src = [0x38u8, 0xFF, 0x38, 0xFF]; // -200 in LE
        let mut dst = [0u16; 1];
        convert_sc16_to_mag(&src, &mut dst);
        // sqrt(200^2 + 200^2) * 2 ≈ 565
        assert_eq!(dst[0], 565);
    }

    #[test]
    fn convert_sc16q11_to_mag_empty_src() {
        let mut dst = [0u16; 4];
        convert_sc16q11_to_mag(&[], &mut dst);
        assert_eq!(dst, [0, 0, 0, 0]);
    }

    #[test]
    fn convert_sc16q11_to_mag_empty_dst() {
        let src = [100u8, 0, 0, 0, 200, 0, 0, 0];
        convert_sc16q11_to_mag(&src, &mut []);
    }

    #[test]
    fn convert_sc16q11_to_mag_all_zero() {
        let src = [0u8; 8];
        let mut dst = [0u16; 2];
        convert_sc16q11_to_mag(&src, &mut dst);
        assert_eq!(dst, [0, 0]);
    }

    #[test]
    fn convert_sc16q11_to_mag_negative_iq() {
        // i = -1, q = -1
        let src = [0xFFu8, 0xFF, 0xFF, 0xFF];
        let mut dst = [0u16; 1];
        convert_sc16q11_to_mag(&src, &mut dst);
        // sqrt(1^2 + 1^2) * 32 ≈ 45
        assert_eq!(dst[0], 45);
    }

    #[test]
    fn convert_sc16q11_to_mag_shorter_dst() {
        let src = [100u8, 0, 0, 0, 200, 0, 0, 0];
        let mut dst = [0u16; 1];
        convert_sc16q11_to_mag(&src, &mut dst);
        assert_eq!(dst[0], 3200);
    }

    #[test]
    fn convert_sc16q11_to_mag_no_clamp_required() {
        // i=1024, q=1024 → sqrt(1024^2 + 1024^2) * 32 ≈ 46340
        let src = [0x00u8, 0x04, 0x00, 0x04];
        let mut dst = [0u16; 1];
        convert_sc16q11_to_mag(&src, &mut dst);
        assert_eq!(dst[0], 46340);
    }
}
