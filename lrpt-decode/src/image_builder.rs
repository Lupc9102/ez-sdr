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
    /// Pixel rows accumulated so far per APID, plus the fixed row width
    /// once known (assumed constant for a given APID within one session).
    channels: HashMap<u16, ChannelImage>,
}

#[derive(Debug, Default)]
struct ChannelImage {
    width: usize,
    rows: Vec<Vec<u8>>,
}

impl ImageBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one scanline's worth of raw pixel bytes for the given APID.
    /// The width of the image for that APID is fixed to the length of the
    /// first scanline received; subsequent scanlines are truncated or
    /// zero-padded to match (a corrupted/short packet must not desync the
    /// whole image).
    pub fn push_scanline(&mut self, apid: u16, pixels: &[u8]) {
        if pixels.is_empty() {
            return;
        }
        let channel = self.channels.entry(apid).or_insert_with(|| ChannelImage {
            width: pixels.len(),
            rows: Vec::new(),
        });
        let width = channel.width;
        let mut row = vec![0u8; width];
        let copy_len = pixels.len().min(width);
        row[..copy_len].copy_from_slice(&pixels[..copy_len]);
        channel.rows.push(row);
    }

    /// Number of scanlines accumulated so far for `apid`. Not currently
    /// called outside tests (callers use `total_line_count` for the
    /// aggregate progress figure); kept for a future per-channel progress
    /// breakdown in the GUI.
    #[allow(dead_code)]
    #[must_use]
    pub fn line_count(&self, apid: u16) -> usize {
        self.channels.get(&apid).map_or(0, |c| c.rows.len())
    }

    /// Total scanlines accumulated across all APIDs (used for coarse
    /// progress reporting).
    #[must_use]
    pub fn total_line_count(&self) -> u32 {
        self.channels.values().map(|c| c.rows.len() as u32).sum()
    }

    /// Render the current (possibly partial) image for `apid` as a
    /// `GrayImage`, or `None` if no scanlines have been received yet.
    #[must_use]
    pub fn render(&self, apid: u16) -> Option<GrayImage> {
        let channel = self.channels.get(&apid)?;
        if channel.width == 0 || channel.rows.is_empty() {
            return None;
        }
        let width = channel.width as u32;
        let height = channel.rows.len() as u32;
        let mut img = GrayImage::new(width, height);
        for (y, row) in channel.rows.iter().enumerate() {
            for (x, &px) in row.iter().enumerate() {
                img.put_pixel(x as u32, y as u32, image::Luma([px]));
            }
        }
        Some(img)
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
    fn known_apid_channel_labels_are_documented() {
        assert!(apid_channel_label(64).is_some());
        assert!(apid_channel_label(9999).is_none());
    }
}
