//! Sample format conversion - translated from convert.c

/// Identity pass-through for buffers that are already magnitude samples.
/// SdrSource::read_samples returns u16 magnitudes, so no conversion is needed.
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
        let fi = src[i * 2] as f32 - 127.4;
        let fq = src[i * 2 + 1] as f32 - 127.4;
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
        let i_val = i16::from_le_bytes(i_bytes).abs() as f32;
        let q_val = i16::from_le_bytes(q_bytes).abs() as f32;
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
        let i_val = i16::from_le_bytes(i_bytes).abs() as f32;
        let q_val = i16::from_le_bytes(q_bytes).abs() as f32;
        let mag = ((i_val * i_val + q_val * q_val).sqrt() * 32.0).min(65535.0);
        dst[i] = mag as u16;
    }
}
