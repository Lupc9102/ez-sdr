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
}
