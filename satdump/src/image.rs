//! Image processing utilities translated from satdump legacy C++ core.
//!
//! Provides compositing, histogram equalization, LUT application, contrast
//! stretching, brightness/contrast correction and saving via the `image` crate.

use std::path::Path;

/// Errors that can occur during image operations.
#[derive(Debug)]
pub enum ImageError {
    Io(std::io::Error),
    ImageCrate(image::ImageError),
    InvalidDimensions(String),
    InvalidChannels(String),
    UnsupportedDepth(String),
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageError::Io(e) => write!(f, "IO error: {e}"),
            ImageError::ImageCrate(e) => write!(f, "Image crate error: {e}"),
            ImageError::InvalidDimensions(s) => write!(f, "Invalid dimensions: {s}"),
            ImageError::InvalidChannels(s) => write!(f, "Invalid channels: {s}"),
            ImageError::UnsupportedDepth(s) => write!(f, "Unsupported depth: {s}"),
        }
    }
}

impl std::error::Error for ImageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ImageError::Io(e) => Some(e),
            ImageError::ImageCrate(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for ImageError {
    fn from(e: std::io::Error) -> Self {
        ImageError::Io(e)
    }
}

impl From<image::ImageError> for ImageError {
    fn from(e: image::ImageError) -> Self {
        ImageError::ImageCrate(e)
    }
}

/// Trait for pixel types supported by our image container.
///
/// Implemented for `u8` and `u16`, matching the legacy 8-bit and 16-bit modes.
pub trait PixelType: Copy + Clone + Default + Send + Sync + PartialOrd + 'static {
    /// Maximum value the pixel can hold (255 for u8, 65535 for u16).
    fn max_value() -> u32;
    /// Convert to `u32` for indexing.
    fn to_u32(self) -> u32;
    /// Construct from `u32`, clamped to the valid range.
    fn from_u32(v: u32) -> Self;
    /// Convert to `f64` for linear calculations.
    fn to_f64(self) -> f64;
    /// Construct from `f64`, clamped to the valid range (truncates like legacy C++).
    fn from_f64(v: f64) -> Self;
}

impl PixelType for u8 {
    fn max_value() -> u32 {
        255
    }
    fn to_u32(self) -> u32 {
        self as u32
    }
    fn from_u32(v: u32) -> Self {
        (v.clamp(0, 255) & 0xFF) as u8
    }
    fn to_f64(self) -> f64 {
        self as f64
    }
    fn from_f64(v: f64) -> Self {
        (v.clamp(0.0, 255.0) as f64) as u8
    }
}

impl PixelType for u16 {
    fn max_value() -> u32 {
        65535
    }
    fn to_u32(self) -> u32 {
        self as u32
    }
    fn from_u32(v: u32) -> Self {
        v.clamp(0, 65535) as u16
    }
    fn to_f64(self) -> f64 {
        self as f64
    }
    fn from_f64(v: f64) -> Self {
        (v.clamp(0.0, 65535.0) as f64) as u16
    }
}

/// Generic image container.
///
/// Data is stored in planar format: all pixels for channel 0, then channel 1,
/// etc. This matches the legacy `satdump::image::Image` layout.
#[derive(Clone, Debug)]
pub struct Image<T: PixelType> {
    width: usize,
    height: usize,
    channels: usize,
    data: Vec<T>,
}

impl<T: PixelType> Image<T> {
    /// Create a new black image.
    pub fn new(width: usize, height: usize, channels: usize) -> Self {
        assert!(width > 0 && height > 0 && channels > 0);
        let len = width * height * channels;
        Self {
            width,
            height,
            channels,
            data: vec![T::default(); len],
        }
    }

    /// Create an image from an existing buffer in planar order.
    pub fn from_data(
        width: usize,
        height: usize,
        channels: usize,
        data: Vec<T>,
    ) -> Result<Self, ImageError> {
        let expected = width * height * channels;
        if data.len() != expected {
            return Err(ImageError::InvalidDimensions(format!(
                "expected {expected} elements, got {}",
                data.len()
            )));
        }
        Ok(Self {
            width,
            height,
            channels,
            data,
        })
    }

    pub fn width(&self) -> usize {
        self.width
    }
    pub fn height(&self) -> usize {
        self.height
    }
    pub fn channels(&self) -> usize {
        self.channels
    }
    pub fn maxval(&self) -> u32 {
        T::max_value()
    }

    /// Total number of elements in the buffer.
    pub fn len(&self) -> usize {
        self.data.len()
    }
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Mutable access to the underlying planar buffer.
    pub fn raw_data(&mut self) -> &mut [T] {
        &mut self.data
    }

    /// Immutable access to the underlying planar buffer.
    pub fn raw_data_ref(&self) -> &[T] {
        &self.data
    }

    /// Linear index for a given channel and pixel offset.
    #[inline]
    fn idx(&self, channel: usize, px: usize) -> usize {
        channel * self.width * self.height + px
    }

    #[inline]
    fn idx_xy(&self, channel: usize, x: usize, y: usize) -> usize {
        channel * self.width * self.height + y * self.width + x
    }

    pub fn get(&self, channel: usize, x: usize, y: usize) -> T {
        self.data[self.idx_xy(channel, x, y)]
    }

    pub fn set(&mut self, channel: usize, x: usize, y: usize, value: T) {
        let i = self.idx_xy(channel, x, y);
        self.data[i] = value;
    }

    pub fn get_linear(&self, channel: usize, p: usize) -> T {
        self.data[self.idx(channel, p)]
    }

    pub fn set_linear(&mut self, channel: usize, p: usize, value: T) {
        let i = self.idx(channel, p);
        self.data[i] = value;
    }

    /// Clamp a value to the valid pixel range.
    pub fn clamp<V: Into<f64>>(&self, v: V) -> T {
        T::from_f64(v.into())
    }

    /// Fill all channels with a single value.
    pub fn fill(&mut self, value: T) {
        self.data.fill(value);
    }

    /// Convert a single-channel image to RGB by replicating the channel.
    pub fn to_rgb(&self) -> Image<T> {
        match self.channels {
            1 => {
                let mut out = Image::new(self.width, self.height, 3);
                let px = self.width * self.height;
                for p in 0..px {
                    let v = self.get_linear(0, p);
                    out.set_linear(0, p, v);
                    out.set_linear(1, p, v);
                    out.set_linear(2, p, v);
                }
                out
            }
            4 => {
                let mut out = Image::new(self.width, self.height, 3);
                let px = self.width * self.height;
                for p in 0..px {
                    out.set_linear(0, p, self.get_linear(0, p));
                    out.set_linear(1, p, self.get_linear(1, p));
                    out.set_linear(2, p, self.get_linear(2, p));
                }
                out
            }
            _ => self.clone(),
        }
    }

    /// Convert an image to RGBA.
    pub fn to_rgba(&self) -> Image<T> {
        match self.channels {
            1 => {
                let mut out = Image::new(self.width, self.height, 4);
                let px = self.width * self.height;
                let max = T::from_u32(T::max_value());
                for p in 0..px {
                    let v = self.get_linear(0, p);
                    out.set_linear(0, p, v);
                    out.set_linear(1, p, v);
                    out.set_linear(2, p, v);
                    out.set_linear(3, p, max);
                }
                out
            }
            2 => {
                let mut out = Image::new(self.width, self.height, 4);
                let px = self.width * self.height;
                for p in 0..px {
                    let v = self.get_linear(0, p);
                    out.set_linear(0, p, v);
                    out.set_linear(1, p, v);
                    out.set_linear(2, p, v);
                    out.set_linear(3, p, self.get_linear(1, p));
                }
                out
            }
            3 => {
                let mut out = Image::new(self.width, self.height, 4);
                let px = self.width * self.height;
                let max = T::from_u32(T::max_value());
                for p in 0..px {
                    out.set_linear(0, p, self.get_linear(0, p));
                    out.set_linear(1, p, self.get_linear(1, p));
                    out.set_linear(2, p, self.get_linear(2, p));
                    out.set_linear(3, p, max);
                }
                out
            }
            _ => self.clone(),
        }
    }

    /// Copy a single-channel image into a specific channel of this image at an offset.
    ///
    /// If `channel == 0` and `src` has the same number of channels as `self`,
    /// all channels are copied (legacy behaviour).
    pub fn draw_image(
        &mut self,
        channel: usize,
        src: &Image<T>,
        x0: isize,
        y0: isize,
    ) -> Result<(), ImageError> {
        if src.channels != 1 && src.channels != self.channels {
            return Err(ImageError::InvalidChannels(format!(
                "src channels {} incompatible with dst channels {}",
                src.channels, self.channels
            )));
        }

        let width = (self.width as isize).min(x0 + src.width as isize) - x0;
        let height = (self.height as isize).min(y0 + src.height as isize) - y0;

        if width <= 0 || height <= 0 {
            return Ok(());
        }

        let copy_all_channels = channel == 0 && src.channels == self.channels;

        if copy_all_channels {
            for c in 0..self.channels {
                for y in 0..height as usize {
                    let dy = y0 + y as isize;
                    if dy < 0 {
                        continue;
                    }
                    let dy = dy as usize;
                    for x in 0..width as usize {
                        let dx = x0 + x as isize;
                        if dx < 0 {
                            continue;
                        }
                        let dx = dx as usize;
                        let v = src.get(c, x, y);
                        self.set(c, dx, dy, v);
                    }
                }
            }
        } else {
            for y in 0..height as usize {
                let dy = y0 + y as isize;
                if dy < 0 {
                    continue;
                }
                let dy = dy as usize;
                for x in 0..width as usize {
                    let dx = x0 + x as isize;
                    if dx < 0 {
                        continue;
                    }
                    let dx = dx as usize;
                    let v = src.get(0, x, y);
                    self.set(channel, dx, dy, v);
                }
            }
        }

        Ok(())
    }

    /// Alpha-blend `src` onto this image. `src` must have 2 channels (Luma+Alpha)
    /// or 4 channels (RGBA). `self` must have compatible channels.
    pub fn draw_image_alpha(
        &mut self,
        src: &Image<T>,
        x0: isize,
        y0: isize,
    ) -> Result<(), ImageError> {
        let width = (self.width as isize).min(x0 + src.width as isize) - x0;
        let height = (self.height as isize).min(y0 + src.height as isize) - y0;

        if width <= 0 || height <= 0 {
            return Ok(());
        }

        let maxv_f = T::max_value() as f64;

        match (src.channels, self.channels) {
            (2, 1) | (2, 2) => {
                for y in 0..height as usize {
                    let dy = y0 + y as isize;
                    if dy < 0 {
                        continue;
                    }
                    let dy = dy as usize;
                    for x in 0..width as usize {
                        let dx = x0 + x as isize;
                        if dx < 0 {
                            continue;
                        }
                        let dx = dx as usize;
                        let a = src.get(1, x, y).to_f64() / maxv_f;
                        let src_v = src.get(0, x, y).to_f64();
                        let dst_v = self.get(0, dx, dy).to_f64();
                        let blended = src_v * a + (1.0 - a) * dst_v;
                        self.set(0, dx, dy, T::from_f64(blended));
                    }
                }
            }
            (4, 3) | (4, 4) => {
                for y in 0..height as usize {
                    let dy = y0 + y as isize;
                    if dy < 0 {
                        continue;
                    }
                    let dy = dy as usize;
                    for x in 0..width as usize {
                        let dx = x0 + x as isize;
                        if dx < 0 {
                            continue;
                        }
                        let dx = dx as usize;
                        let a = src.get(3, x, y).to_f64() / maxv_f;
                        for c in 0..3 {
                            let src_v = src.get(c, x, y).to_f64();
                            let dst_v = self.get(c, dx, dy).to_f64();
                            let blended = src_v * a + (1.0 - a) * dst_v;
                            self.set(c, dx, dy, T::from_f64(blended));
                        }
                        if self.channels == 4 {
                            let src_a = src.get(3, x, y).to_f64();
                            let dst_a = self.get(3, dx, dy).to_f64();
                            if src_a > dst_a {
                                self.set(3, dx, dy, src.get(3, x, y));
                            }
                        }
                    }
                }
            }
            _ => {
                return Err(ImageError::InvalidChannels(
                    "draw_image_alpha requires src channels 2 or 4".into(),
                ));
            }
        }

        Ok(())
    }

    /// Bilinear resize to the requested dimensions.
    pub fn resize_bilinear(&self, width: usize, height: usize) -> Image<T> {
        if width == self.width && height == self.height {
            return self.clone();
        }
        let mut out = Image::new(width, height, self.channels);
        let x_scale = (self.width as f64 - 1.0) / width.max(1) as f64;
        let y_scale = (self.height as f64 - 1.0) / height.max(1) as f64;
        let max_index = self.width * self.height;

        for c in 0..self.channels {
            for i in 0..height {
                for j in 0..width {
                    let x = (x_scale * j as f64) as usize;
                    let y = (y_scale * i as f64) as usize;
                    let x_diff = x_scale * j as f64 - x as f64;
                    let y_diff = y_scale * i as f64 - y as f64;
                    let index = y * self.width + x;

                    let a = self.get_linear(c, index.min(max_index - 1));
                    let b = if index + 1 < max_index {
                        self.get_linear(c, index + 1)
                    } else {
                        a
                    };
                    let c0 = if index + self.width < max_index {
                        self.get_linear(c, index + self.width)
                    } else {
                        a
                    };
                    let d = if index + self.width + 1 < max_index {
                        self.get_linear(c, index + self.width + 1)
                    } else {
                        a
                    };

                    let val = a.to_f64() * (1.0 - x_diff) * (1.0 - y_diff)
                        + b.to_f64() * x_diff * (1.0 - y_diff)
                        + c0.to_f64() * y_diff * (1.0 - x_diff)
                        + d.to_f64() * x_diff * y_diff;

                    out.set_linear(c, i * width + j, T::from_f64(val));
                }
            }
        }

        out
    }

    /// Histogram equalization.
    ///
    /// If `per_channel` is `false` the histogram is computed over all channels
    /// combined (treating the buffer as linear), matching the legacy behaviour.
    pub fn equalize(&mut self, per_channel: bool) {
        let nlevels = (T::max_value() + 1) as usize;
        let channel_pixels = self.width * self.height;

        let channels_to_process = if per_channel { self.channels } else { 1 };

        for c in 0..channels_to_process {
            if per_channel && c == 3 && self.channels == 4 {
                // Do not equalize alpha individually
                break;
            }

            let size = if per_channel {
                channel_pixels
            } else {
                channel_pixels * self.channels
            };

            let mut histogram = vec![0usize; nlevels];
            if per_channel {
                for p in 0..channel_pixels {
                    let v = self.get_linear(c, p).to_u32() as usize;
                    histogram[v] += 1;
                }
            } else {
                for v in self.data.iter() {
                    histogram[v.to_u32() as usize] += 1;
                }
            }

            let mut cum_hist = vec![0usize; nlevels];
            cum_hist[0] = histogram[0];
            for i in 1..nlevels {
                cum_hist[i] = histogram[i] + cum_hist[i - 1];
            }

            let scale_factor = (nlevels - 1) as f64 / size as f64;
            let mut scaling = vec![0u32; nlevels];
            for i in 0..nlevels {
                scaling[i] = (cum_hist[i] as f64 * scale_factor).round() as u32;
            }

            if per_channel {
                for p in 0..channel_pixels {
                    let v = self.get_linear(c, p).to_u32() as usize;
                    self.set_linear(c, p, T::from_u32(scaling[v]));
                }
            } else {
                for v in self.data.iter_mut() {
                    let idx = v.to_u32() as usize;
                    *v = T::from_u32(scaling[idx]);
                }
            }
        }
    }

    /// Contrast stretch: map current min/max to the full dynamic range.
    pub fn normalize(&mut self) {
        if self.data.is_empty() {
            return;
        }

        let mut min = self.data[0].to_u32();
        let mut max = self.data[0].to_u32();

        for v in self.data.iter() {
            let val = v.to_u32();
            if val < min {
                min = val;
            }
            if val > max {
                max = val;
            }
        }

        if max == min {
            return;
        }

        let maxval = T::max_value() as f64;
        let factor = maxval / (max - min) as f64;

        for v in self.data.iter_mut() {
            let val = v.to_u32();
            let new_val = (val - min) as f64 * factor;
            *v = T::from_f64(new_val);
        }
    }

    /// Brightness / contrast adjustment similar to GIMP.
    ///
    /// `brightness` and `contrast` are expected in the legacy range.
    pub fn brightness_contrast(&mut self, brightness: f32, contrast: f32) {
        let scale = T::max_value() as f64 - 1.0;
        let brightness_v = brightness as f64 / 2.0;
        let slant = ((contrast as f64 + 1.0) * std::f64::consts::FRAC_PI_4).tan();

        let channel_count = if self.channels == 4 { 3 } else { self.channels };

        for c in 0..channel_count {
            for y in 0..self.height {
                for x in 0..self.width {
                    let v = self.get(c, x, y).to_f64() / scale;

                    let v = if brightness_v < 0.0 {
                        v * (1.0 + brightness_v)
                    } else {
                        v + ((1.0 - v) * brightness_v)
                    };

                    let v = (v - 0.5) * slant + 0.5;
                    self.set(c, x, y, T::from_f64(v * scale));
                }
            }
        }
    }

    /// Percentile-based white balance.
    pub fn white_balance(&mut self, percentile_value: f32) {
        let max_val = T::max_value() as f64;
        let total_pixels = self.width * self.height;

        for c in 0..self.channels {
            let mut sorted: Vec<u32> = (0..total_pixels)
                .map(|p| self.get_linear(c, p).to_u32())
                .collect();
            sorted.sort_unstable();

            let p1 = percentile(&sorted, percentile_value);
            let p2 = percentile(&sorted, 100.0 - percentile_value);

            if p1 == p2 {
                continue;
            }

            for p in 0..total_pixels {
                let v = self.get_linear(c, p).to_f64();
                let balanced = (v - p1 as f64) * max_val / (p2 as f64 - p1 as f64);
                self.set_linear(c, p, T::from_f64(balanced));
            }
        }
    }

    /// Apply a 1-D lookup table to a single-channel image.
    ///
    /// `lut` must be a single-row image. Its channel count determines the output
    /// channel count. The source pixel value is used as an index (scaled to lut
    /// width) to pick a color.
    pub fn apply_lut(&self, lut: &Image<T>) -> Result<Image<T>, ImageError> {
        if self.channels != 1 {
            return Err(ImageError::InvalidChannels(
                "LUT application requires a single-channel source".into(),
            ));
        }
        if lut.height != 1 {
            return Err(ImageError::InvalidDimensions(
                "LUT must be a single-row image".into(),
            ));
        }
        let out_channels = lut.channels;
        let mut out = Image::new(self.width, self.height, out_channels);
        let maxval = T::max_value() as f64;
        let lut_w = lut.width as f64;

        for y in 0..self.height {
            for x in 0..self.width {
                let v = self.get(0, x, y).to_f64() / maxval;
                let lut_x = (v * (lut_w - 1.0)).round() as usize;
                let lut_x = lut_x.min(lut.width - 1);
                for c in 0..out_channels {
                    out.set(c, x, y, lut.get(c, lut_x, 0));
                }
            }
        }

        Ok(out)
    }

    /// Invert all pixel values linearly.
    pub fn linear_invert(&mut self) {
        let max = T::max_value();
        for v in self.data.iter_mut() {
            *v = T::from_u32(max - v.to_u32());
        }
    }

    /// Very basic despeckle algorithm.
    pub fn simple_despeckle(&mut self, threshold: u32) {
        let w = self.width;
        let h = self.height;
        for c in 0..self.channels {
            for y in 0..h {
                for x in 0..w {
                    let current = self.get(c, x, y).to_u32();
                    let below = if y + 1 < h {
                        self.get(c, x, y + 1).to_u32()
                    } else {
                        0
                    };
                    let left = if x > 0 {
                        self.get(c, x - 1, y).to_u32()
                    } else {
                        0
                    };
                    let right = if x + 1 < w {
                        self.get(c, x + 1, y).to_u32()
                    } else {
                        0
                    };

                    if (current > left + threshold && current > right + threshold)
                        || (current > below + threshold && current > right + threshold)
                    {
                        self.set(c, x, y, T::from_u32((left + right) / 2));
                    }
                }
            }
        }
    }

    /// Save the image to a file using the `image` crate.
    ///
    /// The format is inferred from the extension. Data is converted from planar
    /// to interleaved layout as required by the crate.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), ImageError> {
        let width = self.width as u32;
        let height = self.height as u32;

        match (self.channels, T::max_value()) {
            (1, 255) => {
                let buf = self.to_interleaved_u8();
                let img = image::GrayImage::from_raw(width, height, buf).ok_or_else(|| {
                    ImageError::InvalidDimensions("failed to create GrayImage".into())
                })?;
                img.save(path)?;
            }
            (1, 65535) => {
                let buf = self.to_interleaved_u16();
                let img =
                    image::ImageBuffer::<image::Luma<u16>, Vec<u16>>::from_raw(width, height, buf)
                        .ok_or_else(|| {
                            ImageError::InvalidDimensions("failed to create Gray16Image".into())
                        })?;
                image::DynamicImage::ImageLuma16(img).save(path)?;
            }
            (2, 255) => {
                let buf = self.to_interleaved_u8();
                let img = image::GrayAlphaImage::from_raw(width, height, buf).ok_or_else(|| {
                    ImageError::InvalidDimensions("failed to create GrayAlphaImage".into())
                })?;
                img.save(path)?;
            }
            (2, 65535) => {
                let buf = self.to_interleaved_u16();
                let img = image::ImageBuffer::<image::LumaA<u16>, Vec<u16>>::from_raw(
                    width, height, buf,
                )
                .ok_or_else(|| {
                    ImageError::InvalidDimensions("failed to create GrayAlpha16Image".into())
                })?;
                image::DynamicImage::ImageLumaA16(img).save(path)?;
            }
            (3, 255) => {
                let buf = self.to_interleaved_u8();
                let img = image::RgbImage::from_raw(width, height, buf).ok_or_else(|| {
                    ImageError::InvalidDimensions("failed to create RgbImage".into())
                })?;
                img.save(path)?;
            }
            (3, 65535) => {
                let buf = self.to_interleaved_u16();
                let img =
                    image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_raw(width, height, buf)
                        .ok_or_else(|| {
                            ImageError::InvalidDimensions("failed to create Rgb16Image".into())
                        })?;
                image::DynamicImage::ImageRgb16(img).save(path)?;
            }
            (4, 255) => {
                let buf = self.to_interleaved_u8();
                let img = image::RgbaImage::from_raw(width, height, buf).ok_or_else(|| {
                    ImageError::InvalidDimensions("failed to create RgbaImage".into())
                })?;
                img.save(path)?;
            }
            (4, 65535) => {
                let buf = self.to_interleaved_u16();
                let img =
                    image::ImageBuffer::<image::Rgba<u16>, Vec<u16>>::from_raw(width, height, buf)
                        .ok_or_else(|| {
                            ImageError::InvalidDimensions("failed to create Rgba16Image".into())
                        })?;
                image::DynamicImage::ImageRgba16(img).save(path)?;
            }
            _ => {
                return Err(ImageError::UnsupportedDepth(format!(
                    "unsupported channel/depth combination: {}ch / {}",
                    self.channels,
                    T::max_value()
                )));
            }
        }
        Ok(())
    }

    fn to_interleaved_u8(&self) -> Vec<u8> {
        let pixels = self.width * self.height;
        let mut buf = vec![0u8; pixels * self.channels];
        for p in 0..pixels {
            for c in 0..self.channels {
                buf[p * self.channels + c] = self.get_linear(c, p).to_u32() as u8;
            }
        }
        buf
    }

    fn to_interleaved_u16(&self) -> Vec<u16> {
        let pixels = self.width * self.height;
        let mut buf = vec![0u16; pixels * self.channels];
        for p in 0..pixels {
            for c in 0..self.channels {
                buf[p * self.channels + c] = self.get_linear(c, p).to_u32() as u16;
            }
        }
        buf
    }
}

/// Compose several single-channel images into one multi-channel image.
pub fn composite_channels<T: PixelType>(inputs: &[&Image<T>]) -> Result<Image<T>, ImageError> {
    if inputs.is_empty() {
        return Err(ImageError::InvalidChannels(
            "no input images for compositing".into(),
        ));
    }
    let width = inputs[0].width;
    let height = inputs[0].height;
    for (i, img) in inputs.iter().enumerate() {
        if img.channels != 1 {
            return Err(ImageError::InvalidChannels(format!(
                "image {i} is not single-channel"
            )));
        }
        if img.width != width || img.height != height {
            return Err(ImageError::InvalidDimensions(format!(
                "image {i} size mismatch"
            )));
        }
    }
    let mut out = Image::new(width, height, inputs.len());
    let px = width * height;
    for (c, img) in inputs.iter().enumerate() {
        for p in 0..px {
            out.set_linear(c, p, img.get_linear(0, p));
        }
    }
    Ok(out)
}

/// Blend several images together. For RGBA inputs alpha-weighted blending is
/// used. For other inputs zero-valued pixels are skipped (legacy behaviour).
pub fn blend_images<T: PixelType>(images: &[&Image<T>]) -> Result<Image<T>, ImageError> {
    if images.is_empty() {
        return Err(ImageError::InvalidChannels("no images to blend".into()));
    }
    let width = images.iter().map(|i| i.width).min().unwrap();
    let height = images.iter().map(|i| i.height).min().unwrap();
    let channels = images[0].channels;
    let are_rgba = channels == 4;

    for img in images.iter() {
        if img.channels != channels {
            return Err(ImageError::InvalidChannels(
                "channel mismatch in blend_images".into(),
            ));
        }
    }

    let mut out = Image::new(width, height, channels);
    let pixels = width * height;

    if are_rgba {
        for c in 0..3 {
            for p in 0..pixels {
                let mut final_val = 0.0;
                let mut num_layers = 0.0;
                for img in images.iter() {
                    let alpha = img.get_linear(3, p).to_f64() / T::max_value() as f64;
                    final_val += img.get_linear(c, p).to_f64() * alpha;
                    num_layers += alpha;
                }
                let v = if num_layers > 0.0 {
                    final_val / num_layers
                } else {
                    0.0
                };
                out.set_linear(c, p, T::from_f64(v));
            }
        }
        for p in 0..pixels {
            let mut final_alpha = 0.0f64;
            for img in images.iter() {
                final_alpha = final_alpha.max(img.get_linear(3, p).to_f64());
            }
            out.set_linear(3, p, T::from_f64(final_alpha));
        }
    } else {
        for c in 0..channels {
            for p in 0..pixels {
                let mut final_val = 0.0;
                let mut num_layers = images.len();
                for img in images.iter() {
                    let layer_val = img.get_linear(c, p).to_f64();
                    if layer_val == 0.0 {
                        num_layers -= 1;
                    } else {
                        final_val += layer_val;
                    }
                }
                let v = if num_layers > 0 {
                    final_val / num_layers as f64
                } else {
                    0.0
                };
                out.set_linear(c, p, T::from_f64(v));
            }
        }
    }

    Ok(out)
}

/// Merge two images with a given opacity factor (legacy alpha compositing).
///
/// Both images are expected to be 16-bit in the original, but this generic
/// implementation works for any supported depth.
pub fn merge_opacity<T: PixelType>(
    img1: &Image<T>,
    img2: &Image<T>,
    opacity: f32,
) -> Result<Image<T>, ImageError> {
    let width = img1.width.min(img2.width);
    let height = img1.height.min(img2.height);
    let channels_1 = img1.channels;
    let channels_2 = img2.channels;
    let color_channels = channels_1.min(3);
    let mut ret = Image::new(width, height, channels_1);
    let pixels = width * height;
    let maxv_f = T::max_value() as f64;

    for p in 0..pixels {
        let alpha_1 = if channels_1 == 4 {
            img1.get_linear(3, p).to_f64() / maxv_f
        } else {
            1.0
        };
        let alpha_2 = if channels_2 == 4 {
            img2.get_linear(3, p).to_f64() / maxv_f
        } else {
            1.0
        } * opacity as f64;
        let ret_alpha = alpha_2 + alpha_1 * (1.0 - alpha_2);

        for j in 0..color_channels {
            let v1 = img1.get_linear(j, p).to_f64() / maxv_f;
            let v2 = img2.get_linear(j, p).to_f64() / maxv_f;
            let blended = (alpha_2 * v2 + alpha_1 * v1 * (1.0 - alpha_2)) / ret_alpha;
            ret.set_linear(j, p, T::from_f64(blended * maxv_f));
        }

        if channels_1 == 4 {
            ret.set_linear(3, p, T::from_f64(ret_alpha * maxv_f));
        } else {
            for j in 0..color_channels {
                let v = ret.get_linear(j, p).to_f64() * ret_alpha;
                ret.set_linear(j, p, T::from_f64(v));
            }
        }
    }

    Ok(ret)
}

/// Generate the legacy "jet" colormap as a single-row RGB LUT (256 entries).
pub fn lut_jet<T: PixelType>() -> Image<T> {
    let max = T::from_u32(T::max_value());
    let zero = T::from_u32(0);
    // 4 control points from legacy code:
    // (0,0,max), (max,0,max), (max,0,max), (max,0,0)
    let mut img = Image::new(4, 1, 3);
    img.set(0, 0, 0, zero);
    img.set(1, 0, 0, zero);
    img.set(2, 0, 0, max);

    img.set(0, 1, 0, max);
    img.set(1, 1, 0, zero);
    img.set(2, 1, 0, max);

    img.set(0, 2, 0, max);
    img.set(1, 2, 0, zero);
    img.set(2, 2, 0, max);

    img.set(0, 3, 0, max);
    img.set(1, 3, 0, zero);
    img.set(2, 3, 0, zero);

    img.resize_bilinear(256, 1)
}

fn percentile(sorted: &[u32], perc: f32) -> u32 {
    let size = sorted.len();
    if size == 0 {
        return 0;
    }
    let number_percent = (size as f32 + 1.0) * perc / 100.0;
    if number_percent <= 1.0 {
        sorted[0]
    } else if number_percent >= size as f32 {
        sorted[size - 1]
    } else {
        let lower = number_percent.floor() as usize - 1;
        let upper = lower + 1;
        let frac = number_percent - lower as f32 - 1.0;
        let diff = if upper < size {
            sorted[upper] as f32 - sorted[lower] as f32
        } else {
            0.0
        };
        (sorted[lower] as f32 + frac * diff).round() as u32
    }
}
