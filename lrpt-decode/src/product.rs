//! Post-processing and export helpers for decoded Meteor channel rasters.
//!
//! These operations deliberately keep the original decoded channels intact: callers
//! create composed/adjusted products as separate images so an export never destroys
//! the raw decoder output.

use image::{GrayImage, ImageBuffer, Luma, Rgb, RgbImage};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageAdjustments {
    /// Additive offset in 8-bit pixel units, clamped to 0..=255.
    pub brightness: f32,
    /// Multiplicative contrast around the midpoint (1.0 preserves contrast).
    pub contrast: f32,
    /// Gamma correction (1.0 preserves values; must be finite and positive).
    pub gamma: f32,
    /// Clockwise quarter-turns, 0 through 3.
    pub rotate_quarter_turns: u8,
}

impl Default for ImageAdjustments {
    fn default() -> Self {
        Self {
            brightness: 0.0,
            contrast: 1.0,
            gamma: 1.0,
            rotate_quarter_turns: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CropRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProductError {
    #[error("image dimensions must be nonzero")]
    EmptyImage,
    #[error("crop rectangle is empty or outside the image")]
    InvalidCrop,
    #[error("image adjustments must be finite, gamma positive, and rotation 0..=3")]
    InvalidAdjustments,
    #[error("selected APID {0} is missing")]
    MissingChannel(u16),
    #[error("channel images must have matching dimensions")]
    MismatchedChannels,
}

/// Build an RGB image from the requested APID channels, preserving source pixels.
/// The APID mapping is explicit because LRPT APID order is not RGB order.
pub fn compose_rgb(
    channels: &[(u16, GrayImage)],
    apids: [u16; 3],
) -> Result<RgbImage, ProductError> {
    let find = |apid| {
        channels
            .iter()
            .find(|(id, _)| *id == apid)
            .map(|(_, image)| image)
    };
    let red = find(apids[0]).ok_or(ProductError::MissingChannel(apids[0]))?;
    let green = find(apids[1]).ok_or(ProductError::MissingChannel(apids[1]))?;
    let blue = find(apids[2]).ok_or(ProductError::MissingChannel(apids[2]))?;
    if red.dimensions() != green.dimensions() || red.dimensions() != blue.dimensions() {
        return Err(ProductError::MismatchedChannels);
    }
    if red.width() == 0 || red.height() == 0 {
        return Err(ProductError::EmptyImage);
    }
    Ok(ImageBuffer::from_fn(red.width(), red.height(), |x, y| {
        Rgb([
            red.get_pixel(x, y)[0],
            green.get_pixel(x, y)[0],
            blue.get_pixel(x, y)[0],
        ])
    }))
}

/// Apply brightness, contrast, gamma and clockwise quarter-turn rotation.
pub fn adjust_image(
    image: &GrayImage,
    settings: ImageAdjustments,
) -> Result<GrayImage, ProductError> {
    if image.width() == 0 || image.height() == 0 {
        return Err(ProductError::EmptyImage);
    }
    if !settings.brightness.is_finite()
        || !settings.contrast.is_finite()
        || !settings.gamma.is_finite()
        || settings.gamma <= 0.0
        || settings.rotate_quarter_turns > 3
    {
        return Err(ProductError::InvalidAdjustments);
    }
    let adjusted = ImageBuffer::from_fn(image.width(), image.height(), |x, y| {
        let normalized = (f32::from(image.get_pixel(x, y)[0]) / 255.0 - 0.5)
            .mul_add(settings.contrast, 0.5)
            + settings.brightness / 255.0;
        let corrected = normalized.clamp(0.0, 1.0).powf(1.0 / settings.gamma);
        Luma([(corrected * 255.0).round() as u8])
    });
    Ok(match settings.rotate_quarter_turns {
        0 => adjusted,
        1 => image::imageops::rotate90(&adjusted),
        2 => image::imageops::rotate180(&adjusted),
        _ => image::imageops::rotate270(&adjusted),
    })
}

/// Crop using image pixel coordinates. Overflowing rectangles are rejected.
pub fn crop_image(image: &GrayImage, crop: CropRect) -> Result<GrayImage, ProductError> {
    let x_end = crop
        .x
        .checked_add(crop.width)
        .ok_or(ProductError::InvalidCrop)?;
    let y_end = crop
        .y
        .checked_add(crop.height)
        .ok_or(ProductError::InvalidCrop)?;
    if crop.width == 0 || crop.height == 0 || x_end > image.width() || y_end > image.height() {
        return Err(ProductError::InvalidCrop);
    }
    Ok(image::imageops::crop_imm(image, crop.x, crop.y, crop.width, crop.height).to_image())
}

/// A 256-bin intensity histogram.
pub fn histogram(image: &GrayImage) -> [u64; 256] {
    let mut bins = [0_u64; 256];
    for pixel in image.pixels() {
        bins[usize::from(pixel[0])] += 1;
    }
    bins
}

/// Histogram equalization with deterministic handling of constant images.
pub fn equalize_histogram(image: &GrayImage) -> Result<GrayImage, ProductError> {
    if image.width() == 0 || image.height() == 0 {
        return Err(ProductError::EmptyImage);
    }
    let bins = histogram(image);
    let total = u64::from(image.width()) * u64::from(image.height());
    let first = bins.iter().position(|count| *count != 0).unwrap_or(0);
    let base = bins[first];
    let denominator = total.saturating_sub(base);
    if denominator == 0 {
        return Ok(image.clone());
    }
    let mut cumulative = 0_u64;
    let mut lut = [0_u8; 256];
    for (value, count) in bins.into_iter().enumerate() {
        cumulative += count;
        lut[value] =
            (((cumulative.saturating_sub(base)) as f64 / denominator as f64) * 255.0).round() as u8;
    }
    Ok(ImageBuffer::from_fn(
        image.width(),
        image.height(),
        |x, y| Luma([lut[usize::from(image.get_pixel(x, y)[0])]]),
    ))
}

/// Return the available channels by APID without changing their order.
pub fn channel_map(images: &[(u16, GrayImage)]) -> BTreeMap<u16, GrayImage> {
    images.iter().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_mapping_uses_explicit_apid_assignment() {
        let channels = vec![
            (65, GrayImage::from_pixel(2, 1, Luma([20]))),
            (64, GrayImage::from_pixel(2, 1, Luma([10]))),
            (66, GrayImage::from_pixel(2, 1, Luma([30]))),
        ];
        let rgb = compose_rgb(&channels, [64, 65, 66]).unwrap();
        assert_eq!(rgb.get_pixel(1, 0).0, [10, 20, 30]);
    }

    #[test]
    fn crop_adjust_rotation_histogram_and_equalization_are_real_pixel_operations() {
        let image = GrayImage::from_raw(3, 2, vec![0, 64, 128, 192, 224, 255]).unwrap();
        assert_eq!(histogram(&image).iter().sum::<u64>(), 6);
        let cropped = crop_image(
            &image,
            CropRect {
                x: 1,
                y: 0,
                width: 2,
                height: 2,
            },
        )
        .unwrap();
        assert_eq!(cropped.dimensions(), (2, 2));
        let rotated = adjust_image(
            &cropped,
            ImageAdjustments {
                brightness: 16.0,
                rotate_quarter_turns: 1,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(rotated.dimensions(), (2, 2));
        assert!(rotated.get_pixel(0, 0)[0] > cropped.get_pixel(1, 0)[0]);
        let equalized = equalize_histogram(&image).unwrap();
        assert_eq!(equalized.get_pixel(0, 0)[0], 0);
        assert_eq!(equalized.get_pixel(2, 1)[0], 255);
    }

    #[test]
    fn invalid_crop_and_mismatched_channels_are_rejected() {
        let image = GrayImage::from_pixel(1, 1, Luma([0]));
        assert_eq!(
            crop_image(
                &image,
                CropRect {
                    x: 1,
                    y: 0,
                    width: 1,
                    height: 1
                }
            ),
            Err(ProductError::InvalidCrop)
        );
        let images = vec![(64, image.clone()), (65, GrayImage::new(2, 1)), (66, image)];
        assert_eq!(
            compose_rgb(&images, [64, 65, 66]),
            Err(ProductError::MismatchedChannels)
        );
    }
}
