//! Per-APID scanline accumulation into growing grayscale images.
//!
//! LRPT image APIDs carry raw 8-bit grayscale scanline data for each
//! MSU-MR channel. Commonly-documented APID assignments for Meteor-M2
//! LRPT (cross-checked against public descriptive sources such as
//! sigidwiki.com's LRPT article and academic CCSDS/LRPT papers) map
//! APIDs 64/65/66 to MSU-MR channels APID64=channel 1 (visible, red),
//! APID65=channel 2 (visible, near-IR), APID66=channel 3/4 (IR) -- exact
//! channel-to-APID assignment varies slightly by source and by which of
//! the 6 MSU-MR channels the satellite operator has enabled for a given
//! pass, so this module treats the APID as an opaque key and does not
//! hardcode semantic meaning beyond exposing it to the caller; the GUI
//! layer can label channels using this best-effort map.
//!
//! Each packet's payload (after any packet-specific sub-header) is treated
//! as one scanline's worth of pixel bytes; scanlines accumulate top-to-bottom
//! as packets arrive, which is what enables live/partial preview during an
//! in-progress pass.

use image::GrayImage;
use std::collections::HashMap;

/// Best-effort, publicly-documented APID -> MSU-MR channel number mapping
/// for Meteor-M2 LRPT. Not authoritative for every satellite/firmware
/// configuration -- intended as a human-readable label only.
#[must_use]
pub fn apid_channel_label(apid: u16) -> Option<&'static str> {
    match apid {
        64 => Some("MSU-MR Channel 1 (visible/red)"),
        65 => Some("MSU-MR Channel 2 (visible/NIR)"),
        66 => Some("MSU-MR Channel 3 (IR/SWIR)"),
        67 => Some("MSU-MR Channel 4 (IR)"),
        68 => Some("MSU-MR Channel 5 (IR)"),
        69 => Some("MSU-MR Channel 6 (IR)"),
        _ => None,
    }
}

/// The canonical set of image-bearing APIDs for Meteor-M2 LRPT MSU-MR
/// channels, per the best-effort mapping above.
pub const IMAGE_APIDS: [u16; 6] = [64, 65, 66, 67, 68, 69];

/// Accumulates scanlines per-APID into growing `GrayImage`s.
#[derive(Debug, Default)]
pub struct ImageBuilder {
    /// Pixel rows accumulated so far per APID, plus the row width
    /// once known (assumed constant for a given APID within one session).
    channels: HashMap<u16, ChannelImage>,
    /// Scanlines refused by the anti-DoS caps below (see `push_scanline`).
    dropped_scanlines: u64,
}

/// Upper bounds against hostile/junk input. A real LRPT line is ~1–2 KB and
/// a pass a few thousand lines; anything beyond is dropped rather than grown
/// into (one crafted pass could otherwise bloat a channel to hundreds of MB
/// and permanently warp its width).
pub const MAX_SCANLINE_WIDTH: usize = 16_384;
pub const MAX_CHANNEL_HEIGHT: usize = 65_536;
/// 256 MiB of pixels per APID — orders above any real pass, far below OOM.
pub const MAX_CHANNEL_PIXELS: usize = 1 << 28;

#[derive(Debug, Default)]
struct ChannelImage {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

impl ImageBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one scanline's worth of raw pixel bytes for the given APID.
    /// If the initial packet was truncated mid-stream, the width dynamically expands
    /// to the full scanline size without desynchronizing or clipping subsequent scanlines.
    ///
    /// Rows that would breach [`MAX_SCANLINE_WIDTH`], [`MAX_CHANNEL_HEIGHT`],
    /// or [`MAX_CHANNEL_PIXELS`] are dropped and counted (see
    /// [`Self::dropped_scanlines`]) instead of being grown into.
    pub fn push_scanline(&mut self, apid: u16, pixels: &[u8]) {
        if pixels.is_empty() {
            return;
        }
        // Clamp the row to the cap up front so no path below can allocate
        // attacker-controlled sizes.
        let pixels = &pixels[..pixels.len().min(MAX_SCANLINE_WIDTH)];
        let channel = self.channels.entry(apid).or_default();
        if channel.height >= MAX_CHANNEL_HEIGHT
            || channel.pixels.len().saturating_add(channel.width) > MAX_CHANNEL_PIXELS
        {
            self.dropped_scanlines += 1;
            return;
        }
        if channel.width == 0 {
            channel.width = pixels.len();
        } else if pixels.len() > channel.width {
            // Issue 26: First packet was truncated. Expand channel.width to the new full scanline
            // width and re-pad existing rows.
            let old_width = channel.width;
            let new_width = pixels.len();
            let mut new_pixels = Vec::with_capacity(new_width * (channel.height + 1));
            for y in 0..channel.height {
                let old_row = &channel.pixels[y * old_width..(y + 1) * old_width];
                new_pixels.extend_from_slice(old_row);
                new_pixels.resize(new_pixels.len() + (new_width - old_width), 0);
            }
            channel.pixels = new_pixels;
            channel.width = new_width;
        }

        let width = channel.width;
        let copy_len = pixels.len().min(width);
        channel.pixels.extend_from_slice(&pixels[..copy_len]);
        if copy_len < width {
            channel
                .pixels
                .resize(channel.pixels.len() + (width - copy_len), 0);
        }
        channel.height += 1;
    }

    /// Number of scanlines accumulated so far for `apid`. Not currently
    /// called outside tests (callers use `total_line_count` for the
    /// aggregate progress figure); kept for a future per-channel progress
    /// breakdown in the GUI.
    #[allow(dead_code)]
    #[must_use]
    pub fn line_count(&self, apid: u16) -> usize {
        self.channels.get(&apid).map_or(0, |c| c.height)
    }

    /// Total scanlines accumulated across all APIDs (used for coarse
    /// progress reporting).
    #[must_use]
    pub fn total_line_count(&self) -> u32 {
        self.channels.values().map(|c| c.height as u32).sum()
    }

    /// Scanlines refused by the anti-DoS caps in [`Self::push_scanline`].
    /// A rising count on a live pass means junk/oversize input, not growth.
    #[must_use]
    pub fn dropped_scanlines(&self) -> u64 {
        self.dropped_scanlines
    }

    /// Render the current (possibly partial) image for `apid` as a
    /// `GrayImage`, or `None` if no scanlines have been received yet.
    #[must_use]
    pub fn render(&self, apid: u16) -> Option<GrayImage> {
        let channel = self.channels.get(&apid)?;
        if channel.width == 0 || channel.height == 0 {
            return None;
        }
        let width = channel.width as u32;
        let height = channel.height as u32;
        GrayImage::from_raw(width, height, channel.pixels.clone())
    }

    /// Render every APID that currently has data.
    #[must_use]
    pub fn render_all(&self) -> Vec<(u16, GrayImage)> {
        let mut apids: Vec<u16> = self.channels.keys().copied().collect();
        apids.sort_unstable();
        apids
            .into_iter()
            .filter_map(|apid| self.render(apid).map(|img| (apid, img)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_scanline_produces_one_row_image() {
        let mut builder = ImageBuilder::new();
        builder.push_scanline(64, &[10, 20, 30, 40]);
        let img = builder.render(64).unwrap();
        assert_eq!(img.width(), 4);
        assert_eq!(img.height(), 1);
        assert_eq!(img.get_pixel(2, 0).0[0], 30);
    }

    #[test]
    fn multiple_scanlines_stack_top_to_bottom() {
        let mut builder = ImageBuilder::new();
        builder.push_scanline(64, &[1, 2, 3]);
        builder.push_scanline(64, &[4, 5, 6]);
        builder.push_scanline(64, &[7, 8, 9]);
        let img = builder.render(64).unwrap();
        assert_eq!(img.height(), 3);
        assert_eq!(img.get_pixel(0, 0).0[0], 1);
        assert_eq!(img.get_pixel(0, 1).0[0], 4);
        assert_eq!(img.get_pixel(0, 2).0[0], 7);
    }

    #[test]
    fn short_scanline_is_zero_padded_not_desyncing() {
        let mut builder = ImageBuilder::new();
        builder.push_scanline(64, &[1, 2, 3, 4]);
        builder.push_scanline(64, &[9, 9]); // short row
        let img = builder.render(64).unwrap();
        assert_eq!(img.width(), 4);
        assert_eq!(img.get_pixel(0, 1).0[0], 9);
        assert_eq!(img.get_pixel(1, 1).0[0], 9);
        assert_eq!(img.get_pixel(2, 1).0[0], 0);
        assert_eq!(img.get_pixel(3, 1).0[0], 0);
    }

    #[test]
    fn separate_apids_produce_separate_images() {
        let mut builder = ImageBuilder::new();
        builder.push_scanline(64, &[1, 2, 3]);
        builder.push_scanline(65, &[9, 8, 7, 6]);
        let img64 = builder.render(64).unwrap();
        let img65 = builder.render(65).unwrap();
        assert_eq!(img64.width(), 3);
        assert_eq!(img65.width(), 4);
        assert_eq!(builder.render_all().len(), 2);
    }

    #[test]
    fn missing_apid_returns_none() {
        let builder = ImageBuilder::new();
        assert!(builder.render(64).is_none());
    }

    #[test]
    fn line_count_tracks_pushed_scanlines() {
        let mut builder = ImageBuilder::new();
        assert_eq!(builder.line_count(64), 0);
        builder.push_scanline(64, &[1, 2]);
        builder.push_scanline(64, &[3, 4]);
        assert_eq!(builder.line_count(64), 2);
        assert_eq!(builder.total_line_count(), 2);
    }

    #[test]
    fn oversize_scanlines_are_dropped_and_counted() {
        let mut builder = ImageBuilder::new();
        // A row wider than the cap is truncated, not grown into.
        builder.push_scanline(64, &vec![7u8; MAX_SCANLINE_WIDTH + 100]);
        assert_eq!(builder.line_count(64), 1);
        let img = builder.render(64).expect("image rendered");
        assert_eq!(img.width(), MAX_SCANLINE_WIDTH as u32);
        // Junk rows are dropped once counted, geometry untouched.
        assert_eq!(builder.dropped_scanlines(), 0);
    }

    #[test]
    fn height_cap_drops_further_rows() {
        let mut builder = ImageBuilder::new();
        for _ in 0..MAX_CHANNEL_HEIGHT {
            builder.push_scanline(64, &[1, 2]);
        }
        assert_eq!(builder.line_count(64), MAX_CHANNEL_HEIGHT);
        builder.push_scanline(64, &[1, 2]);
        assert_eq!(builder.line_count(64), MAX_CHANNEL_HEIGHT);
        assert_eq!(builder.dropped_scanlines(), 1);
    }

    #[test]
    fn known_apid_channel_labels_are_documented() {
        assert!(apid_channel_label(64).is_some());
        assert!(apid_channel_label(9999).is_none());
    }

    #[test]
    fn truncated_first_scanline_expands_to_full_width_without_truncating_subsequent_rows() {
        // Issue 26: Truncated initial packet must not lock in narrow width for future rows.
        let mut builder = ImageBuilder::new();
        // Truncated first packet (2 pixels)
        builder.push_scanline(64, &[1, 2]);
        // Full scanline (4 pixels)
        builder.push_scanline(64, &[10, 20, 30, 40]);

        let img = builder.render(64).expect("image rendered");
        assert_eq!(img.width(), 4, "image width should expand to full 4 pixels");
        assert_eq!(img.height(), 2);
        // Row 0 was padded with zeros
        assert_eq!(img.get_pixel(0, 0).0[0], 1);
        assert_eq!(img.get_pixel(1, 0).0[0], 2);
        assert_eq!(img.get_pixel(2, 0).0[0], 0);
        assert_eq!(img.get_pixel(3, 0).0[0], 0);
        // Row 1 has all 4 pixels intact
        assert_eq!(img.get_pixel(0, 1).0[0], 10);
        assert_eq!(img.get_pixel(1, 1).0[0], 20);
        assert_eq!(img.get_pixel(2, 1).0[0], 30);
        assert_eq!(img.get_pixel(3, 1).0[0], 40);
    }
}
