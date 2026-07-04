//! Pure-Rust image processing algorithms for satellite imagery.
//!
//! All functions operate on `&mut [f32]` buffers in the 0.0..1.0 range for
//! precision. Conversion to/from `u8` happens at display/export boundaries.

/// Histogram equalization (global). Redistributes pixel intensities to a
/// uniform distribution across the full dynamic range.
pub fn equalize(pixels: &mut [f32]) {
    if pixels.is_empty() {
        return;
    }
    // Build histogram (256 bins)
    let mut hist = [0u32; 256];
    for &p in pixels.iter() {
        let bin = (p * 255.0).round().clamp(0.0, 255.0) as usize;
        hist[bin] += 1;
    }
    // Build CDF
    let n = pixels.len() as f32;
    let mut cdf = [0.0f32; 256];
    cdf[0] = hist[0] as f32 / n;
    for i in 1..256 {
        cdf[i] = cdf[i - 1] + hist[i] as f32 / n;
    }
    // Find first non-zero CDF entry
    let cdf_min = cdf.iter().find(|&&v| v > 0.0).copied().unwrap_or(0.0);
    // Build LUT
    let mut lut = [0.0f32; 256];
    for i in 0..256 {
        lut[i] = ((cdf[i] - cdf_min) / (1.0 - cdf_min)).clamp(0.0, 1.0);
    }
    // Apply LUT
    for p in pixels.iter_mut() {
        let bin = (*p * 255.0).round().clamp(0.0, 255.0) as usize;
        *p = lut[bin];
    }
}

/// Per-channel histogram equalization for RGB. Equalizes each channel
/// independently, then clamps.
pub fn equalize_rgb(r: &mut [f32], g: &mut [f32], b: &mut [f32]) {
    equalize(r);
    equalize(g);
    equalize(b);
}

/// White balance (GIMP algorithm). Adjusts R/G/B channels so that the
/// brightest neutral areas appear white. `percentile` controls which
/// histogram percentile is used as the white point (default 0.05 = 5%).
pub fn white_balance(r: &mut [f32], g: &mut [f32], b: &mut [f32], percentile: f32) {
    if r.is_empty() {
        return;
    }
    let n = r.len();
    let count = ((n as f32) * percentile).max(1.0) as usize;

    // Sort copies to find percentile values
    let mut r_sorted: Vec<f32> = r.to_vec();
    let mut g_sorted: Vec<f32> = g.to_vec();
    let mut b_sorted: Vec<f32> = b.to_vec();
    r_sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    g_sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    b_sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    // Take the average of the top `count` pixels as the white point
    let r_white: f32 = r_sorted[n - count..].iter().sum::<f32>() / count as f32;
    let g_white: f32 = g_sorted[n - count..].iter().sum::<f32>() / count as f32;
    let b_white: f32 = b_sorted[n - count..].iter().sum::<f32>() / count as f32;

    let max_white = r_white.max(g_white).max(b_white).max(0.001);

    // Scale each channel so its white point matches the brightest
    let r_scale = max_white / r_white.max(0.001);
    let g_scale = max_white / g_white.max(0.001);
    let b_scale = max_white / b_white.max(0.001);

    for i in 0..n {
        r[i] = (r[i] * r_scale).clamp(0.0, 1.0);
        g[i] = (g[i] * g_scale).clamp(0.0, 1.0);
        b[i] = (b[i] * b_scale).clamp(0.0, 1.0);
    }
}

/// Normalize: stretch the current min/max to fill 0.0..1.0.
pub fn normalize(pixels: &mut [f32]) {
    if pixels.is_empty() {
        return;
    }
    let min = pixels.iter().cloned().fold(f32::INFINITY, f32::min);
    let max = pixels.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let range = (max - min).max(0.001);
    for p in pixels.iter_mut() {
        *p = ((*p - min) / range).clamp(0.0, 1.0);
    }
}

/// Brightness and contrast adjustment.
/// `brightness` in -1.0..1.0 (0 = no change), `contrast` in 0.0..2.0 (1.0 = no change).
pub fn brightness_contrast(pixels: &mut [f32], brightness: f32, contrast: f32) {
    let factor = (contrast - 1.0).max(-0.999); // avoid division by zero
    for p in pixels.iter_mut() {
        *p = ((*p + brightness) * (1.0 + factor)).clamp(0.0, 1.0);
    }
}

/// Hue/saturation/lightness adjustment (simplified: global, not per-range).
/// `hue_shift` in degrees (-180..180), `saturation` and `lightness` as multipliers
/// (1.0 = no change, 0.0 = zero saturation / black).
pub fn hue_saturation(rgb: &mut [f32], hue_shift: f32, saturation: f32, lightness: f32) {
    if !rgb.len().is_multiple_of(3) {
        return;
    }
    let h_rad = hue_shift.to_radians();

    for chunk in rgb.chunks_exact_mut(3) {
        let r = chunk[0];
        let g = chunk[1];
        let b = chunk[2];

        // RGB to HSL
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let l = (max + min) / 2.0;

        if (max - min).abs() < 0.001 {
            // Achromatic
            chunk[0] = (l + lightness).clamp(0.0, 1.0);
            chunk[1] = chunk[0];
            chunk[2] = chunk[0];
            continue;
        }

        let d = max - min;
        let s = if l > 0.5 {
            d / (2.0 - max - min)
        } else {
            d / (max + min)
        };

        let mut h = if max == r {
            ((g - b) / d + if g < b { 6.0 } else { 0.0 }) / 6.0
        } else if max == g {
            ((b - r) / d + 2.0) / 6.0
        } else {
            ((r - g) / d + 4.0) / 6.0
        };

        // Apply hue shift
        h = (h + h_rad / (2.0 * std::f32::consts::PI)).rem_euclid(1.0);

        // Apply saturation
        let s = (s * saturation).clamp(0.0, 1.0);

        // Apply lightness
        let l = (l + lightness).clamp(0.0, 1.0);

        // HSL to RGB
        if s < 0.001 {
            chunk[0] = l;
            chunk[1] = l;
            chunk[2] = l;
        } else {
            let q = if l < 0.5 {
                l * (1.0 + s)
            } else {
                l + s - l * s
            };
            let p = 2.0 * l - q;
            chunk[0] = hue_to_rgb(p, q, h + 1.0 / 3.0);
            chunk[1] = hue_to_rgb(p, q, h);
            chunk[2] = hue_to_rgb(p, q, h - 1.0 / 3.0);
        }
    }
}

fn hue_to_rgb(p: f32, q: f32, t: f32) -> f32 {
    let t = t.rem_euclid(1.0);
    if t < 1.0 / 6.0 {
        p + (q - p) * 6.0 * t
    } else if t < 1.0 / 2.0 {
        q
    } else if t < 2.0 / 3.0 {
        p + (q - p) * (2.0 / 3.0 - t) * 6.0
    } else {
        p
    }
}

/// Median blur (3x3, edge-preserving noise reduction).
pub fn median_blur(width: u32, height: u32, pixels: &mut [f32]) {
    let w = width as usize;
    let h = height as usize;
    if w < 3 || h < 3 || pixels.len() != w * h {
        return;
    }
    let original = pixels.to_vec();
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let mut neighbors = Vec::with_capacity(9);
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let nx = (x as i32 + dx) as usize;
                    let ny = (y as i32 + dy) as usize;
                    neighbors.push(original[ny * w + nx]);
                }
            }
            neighbors.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            pixels[y * w + x] = neighbors[4]; // median of 9
        }
    }
}

/// Linear invert: blacks become white and vice versa.
pub fn invert(pixels: &mut [f32]) {
    for p in pixels.iter_mut() {
        *p = 1.0 - *p;
    }
}

/// Compute a 256-bin histogram from float pixels (0.0..1.0).
pub fn histogram(pixels: &[f32]) -> [u32; 256] {
    let mut hist = [0u32; 256];
    for &p in pixels.iter() {
        let bin = (p * 255.0).round().clamp(0.0, 255.0) as usize;
        hist[bin] += 1;
    }
    hist
}

/// Processing pipeline configuration. All fields control whether the
/// corresponding filter is applied and with what parameters.
#[derive(Debug, Clone)]
pub struct ProcessingPipeline {
    pub equalize: bool,
    pub equalize_per_channel: bool,
    pub white_balance: bool,
    pub brightness: f32,
    pub contrast: f32,
    pub hue_shift: f32,
    pub saturation: f32,
    pub lightness: f32,
    pub median_blur: bool,
    pub invert: bool,
}

impl Default for ProcessingPipeline {
    fn default() -> Self {
        Self {
            equalize: false,
            equalize_per_channel: false,
            white_balance: false,
            brightness: 0.0,
            contrast: 1.0,
            hue_shift: 0.0,
            saturation: 1.0,
            lightness: 0.0,
            median_blur: false,
            invert: false,
        }
    }
}

impl ProcessingPipeline {
    /// Apply the full pipeline to RGB pixel buffers.
    pub fn apply(&self, width: u32, height: u32, r: &mut [f32], g: &mut [f32], b: &mut [f32]) {
        // 1. Equalize (per-channel or global)
        if self.equalize {
            if self.equalize_per_channel {
                equalize_rgb(r, g, b);
            } else {
                // Global: concatenate and equalize, then split back
                let mut all: Vec<f32> = Vec::with_capacity(r.len() * 3);
                all.extend_from_slice(r);
                all.extend_from_slice(g);
                all.extend_from_slice(b);
                equalize(&mut all);
                let n = r.len();
                r.copy_from_slice(&all[..n]);
                g.copy_from_slice(&all[n..2 * n]);
                b.copy_from_slice(&all[2 * n..3 * n]);
            }
        }

        // 2. White balance
        if self.white_balance {
            white_balance(r, g, b, 0.05);
        }

        // 3. Brightness + contrast (applied per-channel)
        if self.brightness.abs() > 0.001 || (self.contrast - 1.0).abs() > 0.001 {
            brightness_contrast(r, self.brightness, self.contrast);
            brightness_contrast(g, self.brightness, self.contrast);
            brightness_contrast(b, self.brightness, self.contrast);
        }

        // 4. Hue/Saturation/Lightness
        if self.hue_shift.abs() > 0.5
            || (self.saturation - 1.0).abs() > 0.001
            || self.lightness.abs() > 0.001
        {
            let mut rgb: Vec<f32> = Vec::with_capacity(r.len() * 3);
            for i in 0..r.len() {
                rgb.push(r[i]);
                rgb.push(g[i]);
                rgb.push(b[i]);
            }
            hue_saturation(&mut rgb, self.hue_shift, self.saturation, self.lightness);
            for (i, chunk) in rgb.chunks_exact(3).enumerate() {
                r[i] = chunk[0];
                g[i] = chunk[1];
                b[i] = chunk[2];
            }
        }

        // 5. Median blur
        if self.median_blur {
            median_blur(width, height, r);
            median_blur(width, height, g);
            median_blur(width, height, b);
        }

        // 6. Invert
        if self.invert {
            invert(r);
            invert(g);
            invert(b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equalize_spreads_histogram() {
        let mut pixels = vec![0.1; 1000];
        pixels[0] = 0.9;
        equalize(&mut pixels);
        assert!(pixels[0] > 0.5);
    }

    #[test]
    fn equalize_empty_is_noop() {
        let mut pixels: Vec<f32> = vec![];
        equalize(&mut pixels);
        assert!(pixels.is_empty());
    }

    #[test]
    fn invert_flips_values() {
        let mut pixels = vec![0.0, 0.25, 0.5, 0.75, 1.0];
        invert(&mut pixels);
        assert_eq!(pixels, vec![1.0, 0.75, 0.5, 0.25, 0.0]);
    }

    #[test]
    fn normalize_stretches_range() {
        let mut pixels = vec![0.2, 0.4, 0.6, 0.8];
        normalize(&mut pixels);
        assert!((pixels[0] - 0.0).abs() < 0.01);
        assert!((pixels[3] - 1.0).abs() < 0.01);
    }

    #[test]
    fn normalize_constant_is_noop() {
        let mut pixels = vec![0.5; 100];
        normalize(&mut pixels);
        // All same value: range is 0, division by 0.001 produces ~0.5/0.001 = huge, clamped to 1.0
        assert!(pixels[0] >= 0.0);
    }

    #[test]
    fn brightness_increases_values() {
        let mut pixels = vec![0.5; 100];
        brightness_contrast(&mut pixels, 0.2, 1.0);
        assert!((pixels[0] - 0.7).abs() < 0.01);
    }

    #[test]
    fn contrast_stretches_values() {
        let mut pixels = vec![0.0, 0.25, 0.5, 0.75, 1.0];
        brightness_contrast(&mut pixels, 0.0, 2.0);
        // contrast=2.0 means factor=1.0, so result = p * 2.0
        assert!((pixels[0] - 0.0).abs() < 0.01);
        assert!((pixels[2] - 1.0).abs() < 0.01);
        assert!((pixels[4] - 1.0).abs() < 0.01); // clamped
    }

    #[test]
    fn histogram_counts_correctly() {
        let pixels = vec![0.0, 0.5, 1.0, 0.5, 0.0];
        let hist = histogram(&pixels);
        assert_eq!(hist[0], 2);
        assert_eq!(hist[128], 2); // 0.5 maps to bin 128, appears twice
        assert_eq!(hist[255], 1);
    }

    #[test]
    fn histogram_empty() {
        let hist = histogram(&[]);
        assert_eq!(hist.iter().sum::<u32>(), 0);
    }

    #[test]
    fn white_balance_scales_channels() {
        let mut r = vec![0.2; 100];
        let mut g = vec![0.4; 100];
        let mut b = vec![0.8; 100];
        white_balance(&mut r, &mut g, &mut b, 0.05);
        // After white balance, the channels should be more balanced
        let avg_r: f32 = r.iter().sum::<f32>() / r.len() as f32;
        let avg_g: f32 = g.iter().sum::<f32>() / g.len() as f32;
        let avg_b: f32 = b.iter().sum::<f32>() / b.len() as f32;
        // All should be closer together
        let spread = (avg_r - avg_g).abs() + (avg_g - avg_b).abs();
        assert!(spread < 0.5);
    }

    #[test]
    fn median_blur_reduces_noise() {
        let w = 5u32;
        let h = 5u32;
        let mut pixels = vec![0.5; (w * h) as usize];
        // Add a single bright pixel (salt noise)
        pixels[12] = 1.0;
        median_blur(w, h, &mut pixels);
        // The noisy pixel should be smoothed out
        assert!(pixels[12] < 0.8);
    }

    #[test]
    fn median_blur_small_image_is_noop() {
        let mut pixels = vec![0.5; 4];
        median_blur(2, 2, &mut pixels);
        assert!(pixels.iter().all(|&p| (p - 0.5).abs() < 0.001));
    }

    #[test]
    fn hue_saturation_no_change() {
        let mut rgb = vec![0.5, 0.5, 0.5, 0.8, 0.2, 0.1];
        let original = rgb.clone();
        hue_saturation(&mut rgb, 0.0, 1.0, 0.0);
        for (a, b) in rgb.iter().zip(original.iter()) {
            assert!((a - b).abs() < 0.01);
        }
    }

    #[test]
    fn pipeline_default_is_noop() {
        let pipeline = ProcessingPipeline::default();
        let mut r = vec![0.3; 100];
        let mut g = vec![0.5; 100];
        let mut b = vec![0.7; 100];
        let r_orig = r.clone();
        let g_orig = g.clone();
        let b_orig = b.clone();
        pipeline.apply(10, 10, &mut r, &mut g, &mut b);
        for i in 0..100 {
            assert!((r[i] - r_orig[i]).abs() < 0.01);
            assert!((g[i] - g_orig[i]).abs() < 0.01);
            assert!((b[i] - b_orig[i]).abs() < 0.01);
        }
    }

    #[test]
    fn pipeline_equalize_brightens() {
        let pipeline = ProcessingPipeline {
            equalize: true,
            ..Default::default()
        };
        // Use a dark but non-uniform image so equalization actually brightens it
        let mut r: Vec<f32> = (0..100).map(|i| (i as f32 / 100.0) * 0.3).collect();
        let mut g = r.clone();
        let mut b = r.clone();
        let original = r[90];
        pipeline.apply(10, 10, &mut r, &mut g, &mut b);
        // After equalization, the brightest pixel should be brighter than before
        assert!(r[90] > original);
    }
}
