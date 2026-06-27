//! Satellite instrument decoders - translated from plugins/
//! NOAA APT analog decoder (audio -> imagery)

use anyhow::{Context, Result};
use image::{ImageBuffer, Luma};
use num_complex::Complex32;
use std::f32::consts::PI;
use std::path::Path;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const APT_IMG_WIDTH: usize = 2080;
pub const APT_IMG_OVERS: usize = 4;
pub const FILTER_SRATE: f32 = (APT_IMG_WIDTH * 2 * APT_IMG_OVERS) as f32; // 16640.0

const MAX_STDDEV_VALID: f64 = 2100.0;

/// APT sync-A pattern (39 elements, upsampled 4x)
const SYNC_A: [u8; 39] = [
    0, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0,
    255, 255, 0, 0, 255, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// Wedge sync pattern (9 elements, upsampled 8x)
const WEDGE_SYNC: [u8; 9] = [31, 63, 95, 127, 159, 191, 224, 255, 0];

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AptWedge {
    pub start_line: usize,
    pub std_dev: [f64; 16],

    pub ref1: u16,
    pub ref2: u16,
    pub ref3: u16,
    pub ref4: u16,
    pub ref5: u16,
    pub ref6: u16,
    pub ref7: u16,
    pub ref8: u16,
    pub zero_mod_ref: u16,
    pub therm_temp1: u16,
    pub therm_temp2: u16,
    pub therm_temp3: u16,
    pub therm_temp4: u16,
    pub patch_temp: u16,
    pub back_scan: u16,
    pub channel: u16,

    /// Resolved channel: 0..5 -> AVHRR 1,2,3a,3b,4,5 or -1 if unknown
    pub rchannel: i32,
}

impl Default for AptWedge {
    fn default() -> Self {
        Self {
            start_line: 0,
            std_dev: [0.0; 16],
            ref1: 0,
            ref2: 0,
            ref3: 0,
            ref4: 0,
            ref5: 0,
            ref6: 0,
            ref7: 0,
            ref8: 0,
            zero_mod_ref: 0,
            therm_temp1: 0,
            therm_temp2: 0,
            therm_temp3: 0,
            therm_temp4: 0,
            patch_temp: 0,
            back_scan: 0,
            channel: 0,
            rchannel: -1,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AptImage {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u16>,
}

impl AptImage {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            data: vec![0u16; width * height],
        }
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize) -> u16 {
        self.data[y * self.width + x]
    }

    #[inline]
    pub fn set(&mut self, x: usize, y: usize, val: u16) {
        self.data[y * self.width + x] = val;
    }

    /// Vertical crop: keep all columns between [x0, x1)
    pub fn crop_to(&self, x0: usize, x1: usize) -> Self {
        let w = x1.saturating_sub(x0);
        let h = self.height;
        let mut data = vec![0u16; w * h];
        for y in 0..h {
            for x in 0..w {
                data[y * w + x] = self.get(x0 + x, y);
            }
        }
        Self {
            width: w,
            height: h,
            data,
        }
    }

    /// In-place crop of a sub-rectangle
    pub fn crop(&mut self, x0: usize, y0: usize, w: usize, h: usize) {
        let mut data = vec![0u16; w * h];
        for y in 0..h {
            for x in 0..w {
                data[y * w + x] = self.get(x0 + x, y0 + y);
            }
        }
        self.width = w;
        self.height = h;
        self.data = data;
    }
}

#[derive(Debug, Default)]
pub struct AptMetadata {
    pub lines: usize,
    pub channel_a: i32,
    pub channel_a1: i32,
    pub channel_b: i32,
    pub calibrated: bool,
    pub new_white: u16,
    pub new_black: u16,
    pub timing_lines: Vec<usize>,
}

#[derive(Debug)]
pub struct AptDecodeResult {
    pub raw_sync: AptImage,
    pub raw_unsynced: Option<AptImage>,
    pub channel_a: Option<AptImage>,
    pub channel_b: Option<AptImage>,
    pub metadata: AptMetadata,
}

// ---------------------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------------------

pub struct AptDecoder {
    pub audio_samplerate: u32,
    pub autocrop_wedges: bool,
    pub max_crop_stddev: f64,
    pub save_unsynced: bool,
    pub align_timestamps: bool,
}

impl Default for AptDecoder {
    fn default() -> Self {
        Self {
            audio_samplerate: 11025,
            autocrop_wedges: false,
            max_crop_stddev: 3500.0,
            save_unsynced: true,
            align_timestamps: true,
        }
    }
}

impl AptDecoder {
    pub fn new(audio_samplerate: u32) -> Self {
        Self {
            audio_samplerate,
            ..Default::default()
        }
    }

    /// Decode a 16-bit mono WAV file.
    pub fn decode_wav<P: AsRef<Path>>(&self, path: P) -> Result<AptDecodeResult> {
        let mut reader = hound::WavReader::open(path.as_ref())
            .with_context(|| format!("Failed to open WAV: {:?}", path.as_ref()))?;
        let spec = reader.spec();
        if spec.sample_format != hound::SampleFormat::Int || spec.bits_per_sample != 16 {
            anyhow::bail!("Only 16-bit integer WAV files are supported");
        }
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.map(|v| v as f32 / 32767.0))
            .collect::<Result<Vec<_>, _>>()?;
        let mono = if spec.channels == 2 {
            samples.iter().step_by(2).copied().collect()
        } else {
            samples
        };
        self.decode_samples(&mono)
    }

    /// Main processing pipeline: audio samples -> decoded imagery.
    pub fn decode_samples(&self, samples: &[f32]) -> Result<AptDecodeResult> {
        // --- DSP chain ---
        // 1. Real -> Complex (implicit)
        let complex_input: Vec<Complex32> = samples.iter().map(|&s| Complex32::new(s, 0.0)).collect();

        // 2. Frequency shift by -2.4 kHz
        let shifted = freq_shift(&complex_input, -2400.0, self.audio_samplerate as f32);

        // 3. Rational resample to FILTER_SRATE (16640 Hz)
        let resampled = rational_resample_complex(
            &shifted,
            FILTER_SRATE,
            self.audio_samplerate as f32,
        );

        // 4. Low-pass FIR at 1040 Hz
        let lpf = design_lowpass_fir(1040.0, FILTER_SRATE, 129);
        let filtered = apply_fir_complex(&resampled, &lpf);

        // 5. Complex to magnitude
        let mag = complex_to_mag(&filtered);

        // --- Buffer to image ---
        let overs = APT_IMG_OVERS;
        let width = APT_IMG_WIDTH;
        let total_pixels = mag.len().min(mag.len() / (width * overs) * (width * overs));
        let line_cnt = total_pixels / (width * overs);

        let mut imagebuf = vec![0u16; line_cnt * width * overs];
        for (i, buf_val) in imagebuf.iter_mut().enumerate().take(total_pixels) {
            let mut v = mag[i] * 2.0 * 65535.0;
            if v > 65535.0 {
                v = 65535.0;
            }
            *buf_val = v as u16;
        }

        let mut wip_unsynced = AptImage {
            width: width * overs,
            height: line_cnt,
            data: imagebuf,
        };

        // White balance (unsynced)
        white_balance(&mut wip_unsynced.data);

        let raw_unsynced = if self.save_unsynced {
            let mut sized = AptImage::new(width, line_cnt);
            for line in 0..line_cnt.saturating_sub(1) {
                for i in 0..width {
                    let src = wip_unsynced.get(i * overs, line);
                    sized.set(i, line, src);
                }
            }
            Some(sized)
        } else {
            None
        };

        // --- Synchronize ---
        let mut wip_synced = synchronize(&wip_unsynced);
        white_balance(&mut wip_synced.data);

        // --- Parse wedges and spaces ---
        let mut wedge_1 = wip_synced.crop_to(997, 997 + 41);
        let mut wedge_2 = wip_synced.crop_to(2037, 2037 + 41);
        let space_a = wip_synced.crop_to(42, 42 + 43);
        let space_b = wip_synced.crop_to(1081, 1081 + 43);

        let wedges1 = parse_wedge_full(&wedge_1);
        let wedges2 = parse_wedge_full(&wedge_2);

        let (new_white, new_black) = get_calib_values_wedge(&wedges1);
        let (new_white1, new_black1) = get_calib_values_wedge(&wedges2);

        let mut meta = AptMetadata {
            lines: line_cnt,
            ..Default::default()
        };

        let mut switchy = -1i32;
        let mut space_av = 0u32;
        let mut space_av1 = 0u32;
        let mut space_bv = 0u32;
        let mut bb_a = 0u32;
        let mut bb_a1 = 0u32;
        let mut calib_wedge_ch1 = AptWedge::default();
        let mut calib_wedge_ch2 = AptWedge::default();
        let mut prt_counts = [0u16; 4];
        let mut timing_lines: Vec<usize> = Vec::new();

        if new_white != 0 && new_black != 0 && new_white1 != 0 && new_black1 != 0 {
            let cal_white = ((new_white as u32 + new_white1 as u32) / 2) as u16;
            let cal_black = ((new_black as u32 + new_black1 as u32) / 2) as u16;
            meta.new_white = cal_white;
            meta.new_black = cal_black;
            meta.calibrated = true;

            // Calibrate entire synchronized image
            for v in wip_synced.data.iter_mut() {
                *v = scale_val(*v, cal_black, cal_white);
            }

            // --- Process wedge 1 ---
            let mut valid_temp1 = 0usize;
            let mut valid_temp2 = 0usize;
            let mut valid_temp3 = 0usize;
            let mut valid_temp4 = 0usize;
            let mut valid_patch = 0usize;
            let mut validn1_0 = 0usize;
            let mut validn1_1 = 0usize;

            for wed in &wedges1 {
                if meta.channel_a == -1 {
                    meta.channel_a = wed.rchannel;
                } else if meta.channel_a1 == -1 && meta.channel_a != wed.rchannel {
                    meta.channel_a1 = wed.rchannel;
                    switchy = wed.start_line as i32;
                }

                if wed.std_dev[9] < MAX_STDDEV_VALID {
                    calib_wedge_ch1.therm_temp1 = calib_wedge_ch1.therm_temp1.saturating_add(scale_val(wed.therm_temp1, cal_black, cal_white));
                    valid_temp1 += 1;
                }
                if wed.std_dev[10] < MAX_STDDEV_VALID {
                    calib_wedge_ch1.therm_temp2 = calib_wedge_ch1.therm_temp2.saturating_add(scale_val(wed.therm_temp2, cal_black, cal_white));
                    valid_temp2 += 1;
                }
                if wed.std_dev[11] < MAX_STDDEV_VALID {
                    calib_wedge_ch1.therm_temp3 = calib_wedge_ch1.therm_temp3.saturating_add(scale_val(wed.therm_temp3, cal_black, cal_white));
                    valid_temp3 += 1;
                }
                if wed.std_dev[12] < MAX_STDDEV_VALID {
                    calib_wedge_ch1.therm_temp4 = calib_wedge_ch1.therm_temp4.saturating_add(scale_val(wed.therm_temp4, cal_black, cal_white));
                    valid_temp4 += 1;
                }
                if wed.std_dev[13] < MAX_STDDEV_VALID {
                    calib_wedge_ch1.patch_temp = calib_wedge_ch1.patch_temp.saturating_add(scale_val(wed.patch_temp, cal_black, cal_white));
                    valid_patch += 1;
                }
                if wed.std_dev[14] < MAX_STDDEV_VALID {
                    if meta.channel_a1 == -1 {
                        bb_a += scale_val(wed.back_scan, cal_black, cal_white) as u32;
                        validn1_0 += 1;
                    } else {
                        bb_a1 += scale_val(wed.back_scan, cal_black, cal_white) as u32;
                        validn1_1 += 1;
                    }
                }
            }

            if valid_temp1 > 0 && valid_temp2 > 0 && valid_temp3 > 0 && valid_temp4 > 0 && valid_patch > 0 {
                calib_wedge_ch1.therm_temp1 = ((calib_wedge_ch1.therm_temp1 as u32 / valid_temp1 as u32) >> 6) as u16;
                calib_wedge_ch1.therm_temp2 = ((calib_wedge_ch1.therm_temp2 as u32 / valid_temp2 as u32) >> 6) as u16;
                calib_wedge_ch1.therm_temp3 = ((calib_wedge_ch1.therm_temp3 as u32 / valid_temp3 as u32) >> 6) as u16;
                calib_wedge_ch1.therm_temp4 = ((calib_wedge_ch1.therm_temp4 as u32 / valid_temp4 as u32) >> 6) as u16;
                calib_wedge_ch1.patch_temp = ((calib_wedge_ch1.patch_temp as u32 / valid_patch as u32) >> 6) as u16;
                bb_a = if validn1_0 == 0 {
                    0
                } else {
                    (bb_a / validn1_0 as u32) >> 6
                };
                if validn1_1 != 0 {
                    bb_a1 = (bb_a1 / validn1_1 as u32) >> 6;
                }
            }

            // Adjust channel IDs
            for ch in [&mut meta.channel_a, &mut meta.channel_a1] {
                if *ch >= 1 {
                    if *ch > 2 {
                        *ch += 1;
                    }
                    *ch -= 1;
                }
            }

            // --- Process wedge 2 ---
            let mut valid_backscan = 0usize;
            valid_temp1 = 0;
            valid_temp2 = 0;
            valid_temp3 = 0;
            valid_temp4 = 0;
            valid_patch = 0;

            for wed in &wedges2 {
                if meta.channel_b == -1 {
                    meta.channel_b = wed.rchannel;
                }

                if wed.std_dev[9] < MAX_STDDEV_VALID {
                    calib_wedge_ch2.therm_temp1 = calib_wedge_ch2.therm_temp1.saturating_add(scale_val(wed.therm_temp1, cal_black, cal_white));
                    valid_temp1 += 1;
                }
                if wed.std_dev[10] < MAX_STDDEV_VALID {
                    calib_wedge_ch2.therm_temp2 = calib_wedge_ch2.therm_temp2.saturating_add(scale_val(wed.therm_temp2, cal_black, cal_white));
                    valid_temp2 += 1;
                }
                if wed.std_dev[11] < MAX_STDDEV_VALID {
                    calib_wedge_ch2.therm_temp3 = calib_wedge_ch2.therm_temp3.saturating_add(scale_val(wed.therm_temp3, cal_black, cal_white));
                    valid_temp3 += 1;
                }
                if wed.std_dev[12] < MAX_STDDEV_VALID {
                    calib_wedge_ch2.therm_temp4 = calib_wedge_ch2.therm_temp4.saturating_add(scale_val(wed.therm_temp4, cal_black, cal_white));
                    valid_temp4 += 1;
                }
                if wed.std_dev[13] < MAX_STDDEV_VALID {
                    calib_wedge_ch2.patch_temp = calib_wedge_ch2.patch_temp.saturating_add(scale_val(wed.patch_temp, cal_black, cal_white));
                    valid_patch += 1;
                }
                if wed.std_dev[14] < MAX_STDDEV_VALID {
                    calib_wedge_ch2.back_scan = calib_wedge_ch2.back_scan.saturating_add(scale_val(wed.back_scan, cal_black, cal_white));
                    valid_backscan += 1;
                }
            }

            if valid_temp1 > 0 && valid_temp2 > 0 && valid_temp3 > 0 && valid_temp4 > 0 && valid_patch > 0 && valid_backscan > 0 {
                calib_wedge_ch2.therm_temp1 = ((calib_wedge_ch2.therm_temp1 as u32 / valid_temp1 as u32) >> 6) as u16;
                calib_wedge_ch2.therm_temp2 = ((calib_wedge_ch2.therm_temp2 as u32 / valid_temp2 as u32) >> 6) as u16;
                calib_wedge_ch2.therm_temp3 = ((calib_wedge_ch2.therm_temp3 as u32 / valid_temp3 as u32) >> 6) as u16;
                calib_wedge_ch2.therm_temp4 = ((calib_wedge_ch2.therm_temp4 as u32 / valid_temp4 as u32) >> 6) as u16;
                calib_wedge_ch2.patch_temp = ((calib_wedge_ch2.patch_temp as u32 / valid_patch as u32) >> 6) as u16;
                calib_wedge_ch2.back_scan = ((calib_wedge_ch2.back_scan as u32 / valid_backscan as u32) >> 6) as u16;
            }

            if meta.channel_b >= 1 {
                if meta.channel_b > 2 {
                    meta.channel_b += 1;
                }
                meta.channel_b -= 1;
            }

            prt_counts[0] = ((calib_wedge_ch1.therm_temp1 as u32 + calib_wedge_ch2.therm_temp1 as u32) / 2) as u16;
            prt_counts[1] = ((calib_wedge_ch1.therm_temp2 as u32 + calib_wedge_ch2.therm_temp2 as u32) / 2) as u16;
            prt_counts[2] = ((calib_wedge_ch1.therm_temp3 as u32 + calib_wedge_ch2.therm_temp3 as u32) / 2) as u16;
            prt_counts[3] = ((calib_wedge_ch1.therm_temp4 as u32 + calib_wedge_ch2.therm_temp4 as u32) / 2) as u16;

            // --- Space A analysis ---
            let mut validl1 = 0usize;
            let mut validl1_1 = 0usize;

            if meta.channel_a1 != -1 {
                let limit = (switchy as usize).min(space_a.height).saturating_sub(1);
                for y in 0..limit {
                    let (avg, stddev) = line_avg_stddev(&space_a, y);
                    let avg_scaled = scale_val(avg as u16, cal_black, cal_white);
                    if stddev < MAX_STDDEV_VALID
                        && avg_scaled as f64 > stddev
                        && (avg_scaled as f64) < 65535.0 - stddev
                    {
                        space_av1 += avg_scaled as u32;
                        validl1_1 += 1;
                    }
                }
                if validl1_1 > 0 {
                    space_av1 /= validl1_1 as u32;
                }
            }

            let limit_a = if meta.channel_a1 != -1 {
                switchy as usize
            } else {
                space_a.height.saturating_sub(1)
            };
            for y in 0..limit_a {
                let (avg, stddev) = line_avg_stddev(&space_a, y);
                let avg_scaled = scale_val(avg as u16, cal_black, cal_white);
                if stddev < MAX_STDDEV_VALID
                    && avg_scaled as f64 > stddev
                    && (avg_scaled as f64) < 65535.0 - stddev
                {
                    space_av += avg_scaled as u32;
                    validl1 += 1;
                }
            }
            if validl1 > 0 {
                space_av /= validl1 as u32;
            }

            // --- Space B analysis + timing marks ---
            let mut validl2 = 0usize;
            let mut wip_timing_line = 0usize;

            for y in 0..space_b.height {
                let (avg, stddev) = line_avg_stddev(&space_b, y);
                let avg_scaled = scale_val(avg as u16, cal_black, cal_white);

                if stddev < MAX_STDDEV_VALID {
                    if avg_scaled as f64 <= stddev {
                        if wip_timing_line == 0 {
                            wip_timing_line = y;
                        } else if y - wip_timing_line > 1 {
                            wip_timing_line = 0;
                        }
                    } else if avg_scaled as f64 >= 65535.0 - stddev && wip_timing_line != 0 {
                        if y - wip_timing_line > 3 || y - wip_timing_line < 2 {
                            wip_timing_line = 0;
                        } else if y - wip_timing_line == 3 {
                            timing_lines.push(wip_timing_line);
                            wip_timing_line = 0;
                        }
                    } else {
                        wip_timing_line = 0;
                        space_bv += avg_scaled as u32;
                        validl2 += 1;
                    }
                }
            }
            if validl2 > 0 {
                space_bv /= validl2 as u32;
            }
        }

        meta.timing_lines = timing_lines;

        // --- Autocrop ---
        let (first_valid_line, last_valid_line) = if self.autocrop_wedges {
            let (f1, l1) = find_valid_wedge_bounds(&wedge_1, self.max_crop_stddev);
            let (f2, l2) = find_valid_wedge_bounds(&wedge_2, self.max_crop_stddev);
            let first = f1.min(f2);
            let last = l1.max(l2);
            if last > first + 100 && first != usize::MAX && last != 0 {
                (first, last)
            } else {
                (0, wip_synced.height)
            }
        } else {
            (0, wip_synced.height)
        };

        if self.autocrop_wedges && last_valid_line > first_valid_line + 100 {
            wip_synced.crop(0, first_valid_line, APT_IMG_WIDTH, last_valid_line - first_valid_line);
        }

        // --- Extract image channels ---
        let mut cha = wip_synced.crop_to(86, 86 + 909);
        let mut chb = wip_synced.crop_to(1126, 1126 + 909);

        // Fixup bleed on right edge
        for y in 0..cha.height {
            let v = (cha.get(908, y) as f32 * 0.25 + cha.get(907, y) as f32 * 0.75) as u16;
            cha.set(908, y, v);
            let v = (chb.get(908, y) as f32 * 0.25 + chb.get(907, y) as f32 * 0.75) as u16;
            chb.set(908, y, v);
        }

        Ok(AptDecodeResult {
            raw_sync: wip_synced,
            raw_unsynced,
            channel_a: Some(cha),
            channel_b: Some(chb),
            metadata: meta,
        })
    }

    /// Save a 16-bit grayscale PNG.
    pub fn save_png<P: AsRef<Path>>(&self, image: &AptImage, path: P) -> Result<()> {
        save_gray16(path.as_ref(), image.width as u32, image.height as u32, &image.data)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn scale_val(val: u16, new_black: u16, new_white: u16) -> u16 {
    let init = val as f32 - new_black as f32;
    let mut v = init / (new_white as f32 - new_black as f32);
    v *= 65535.0;
    v = v.clamp(0.0, 65535.0);
    v as u16
}

fn white_balance(data: &mut [u16]) {
    let (min, max) = data.iter().fold((u16::MAX, u16::MIN), |(mn, mx), &v| {
        (mn.min(v), mx.max(v))
    });
    if max > min {
        let range = (max - min) as f32;
        for v in data.iter_mut() {
            let scaled = ((*v - min) as f32 / range) * 65535.0;
            *v = scaled as u16;
        }
    }
}

fn line_avg_stddev(img: &AptImage, y: usize) -> (f64, f64) {
    let mut avg = 0.0f64;
    for x in 0..img.width {
        avg += img.get(x, y) as f64;
    }
    avg /= img.width as f64;
    let mut var = 0.0f64;
    for x in 0..img.width {
        let d = img.get(x, y) as f64 - avg;
        var += d * d;
    }
    let stddev = (var / img.width as f64).sqrt();
    (avg, stddev)
}

fn find_valid_wedge_bounds(wedge: &AptImage, max_stddev: f64) -> (usize, usize) {
    let h = wedge.height;
    let mut first = usize::MAX;
    let mut last = 0usize;

    for line in 0..h {
        let (_, stddev) = line_avg_stddev(wedge, line);
        if stddev < max_stddev {
            first = line;
            break;
        }
    }

    for line in (0..h.saturating_sub(1)).rev() {
        let (_, stddev) = line_avg_stddev(wedge, line);
        if stddev < max_stddev {
            last = line;
            break;
        }
    }

    (first, last)
}

fn save_gray16(path: &Path, width: u32, height: u32, data: &[u16]) -> Result<()> {
    let img = ImageBuffer::<Luma<u16>, Vec<u16>>::from_raw(width, height, data.to_vec())
        .context("invalid image dimensions for buffer")?;
    img.save(path)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// DSP helpers
// ---------------------------------------------------------------------------

fn freq_shift(input: &[Complex32], freq: f32, fs: f32) -> Vec<Complex32> {
    let phase_inc = -2.0 * PI * freq / fs;
    input
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            let phase = phase_inc * i as f32;
            let rot = Complex32::new(phase.cos(), phase.sin());
            s * rot
        })
        .collect()
}

/// Design a windowed-sinc low-pass FIR (Hamming window).
fn design_lowpass_fir(cutoff_hz: f32, fs: f32, ntaps: usize) -> Vec<f32> {
    assert!(ntaps % 2 == 1, "tap count must be odd");
    let fc = cutoff_hz / fs;
    let mut taps = vec![0.0f32; ntaps];
    let center = (ntaps - 1) as f32 / 2.0;
    for i in 0..ntaps {
        let x = i as f32 - center;
        let sinc = if x.abs() < 1e-9 {
            2.0 * fc
        } else {
            (2.0 * PI * fc * x).sin() / (PI * x)
        };
        let window = 0.54 - 0.46 * (2.0 * PI * i as f32 / (ntaps - 1) as f32).cos();
        taps[i] = sinc * window;
    }
    taps
}

fn apply_fir_complex(input: &[Complex32], taps: &[f32]) -> Vec<Complex32> {
    let n = input.len();
    let m = taps.len();
    let mut out = vec![Complex32::new(0.0, 0.0); n];
    for i in 0..n {
        let mut acc = Complex32::new(0.0, 0.0);
        for (j, &h) in taps.iter().enumerate() {
            let idx = i as isize - j as isize + (m as isize - 1) / 2;
            if idx >= 0 && idx < n as isize {
                acc += input[idx as usize] * h;
            }
        }
        out[i] = acc;
    }
    out
}

/// Rational resample: output rate = out_fs, input rate = in_fs.
/// Implemented as zero-stuff -> FIR -> decimate.
fn rational_resample_complex(input: &[Complex32], out_fs: f32, in_fs: f32) -> Vec<Complex32> {
    // Determine L/M
    let ratio = out_fs / in_fs;
    let l = (ratio * 1000.0).round() as usize; // e.g. 16640/11025 ~ 1.5093
    let m = 1000usize;
    // Find exact integer ratio using continued fraction / brute force for simple rates
    // For practicality, work out exact ratio via fractions:
    let (l, m) = rational_approximation(out_fs, in_fs);

    let cutoff = in_fs.min(out_fs) / 2.0;
    let taps = design_lowpass_fir(cutoff, in_fs * l as f32, 64.max(l * 4).max(m * 4) | 1);

    let interp_len = input.len() * l + taps.len();
    let mut filtered = vec![Complex32::new(0.0, 0.0); interp_len];

    // Convolution with interpolated sequence
    for (i, &sample) in input.iter().enumerate() {
        for (j, &h) in taps.iter().enumerate() {
            filtered[i * l + j] += sample * h;
        }
    }

    // Decimate by M
    let out_len = (input.len() * l) / m;
    let mut out = Vec::with_capacity(out_len);
    for i in (0..input.len() * l).step_by(m) {
        if i < filtered.len() {
            out.push(filtered[i] * l as f32);
        }
    }
    out
}

/// Helper to compute integer L/M approximating out_fs/in_fs.
fn rational_approximation(out_fs: f32, in_fs: f32) -> (usize, usize) {
    // Common NOAA APT audio rates: 11025, 22050, 44100, 48000, 96000
    // Target 16640 Hz.
    let pairs = [
        ((16640.0, 11025.0), (3328, 2205)),
        ((16640.0, 22050.0), (1664, 2205)),
        ((16640.0, 44100.0), (832, 2205)),
        ((16640.0, 48000.0), (26, 75)),
        ((16640.0, 96000.0), (13, 75)),
    ];
    for &(ref_rates, lm) in &pairs {
        if (out_fs - ref_rates.0).abs() < 1.0 && (in_fs - ref_rates.1).abs() < 1.0 {
            return lm;
        }
    }
    // fallback: 1:1 or compute generic
    let ratio = out_fs / in_fs;
    if (ratio - 1.0).abs() < 0.001 {
        return (1, 1);
    }
    // Generic fallback using 1000 base
    let l = (ratio * 1000.0).round() as usize;
    (l, 1000)
}

fn complex_to_mag(input: &[Complex32]) -> Vec<f32> {
    input.iter().map(|c| c.norm()).collect()
}

// ---------------------------------------------------------------------------
// Synchronization
// ---------------------------------------------------------------------------

fn synchronize(unsynced: &AptImage) -> AptImage {
    let overs = APT_IMG_OVERS;
    let width = APT_IMG_WIDTH;
    let line_cnt = unsynced.height;

    let mut final_sync_a = vec![0i32; SYNC_A.len() * overs];
    for (i, &v) in SYNC_A.iter().enumerate() {
        for f in 0..overs {
            final_sync_a[i * overs + f] = v as i32;
        }
    }

    let mut synced = AptImage::new(width, line_cnt);
    let sync_len = final_sync_a.len();
    let search_limit = (width * overs).saturating_sub(sync_len);

    for line in 0..line_cnt.saturating_sub(1) {
        let mut best_cor = i64::MAX;
        let mut best_pos = 0usize;

        for pos in 0..=search_limit {
            let mut cor = 0i64;
            for (i, &sync_val) in final_sync_a.iter().enumerate() {
                let sample = (unsynced.get(pos + i, line) >> 8) as i64;
                cor += (sample - sync_val as i64).abs();
            }
            if cor < best_cor {
                best_cor = cor;
                best_pos = pos;
            }
        }

        for i in 0..width {
            let src_x = best_pos + i * overs;
            if src_x < unsynced.width {
                synced.set(i, line, unsynced.get(src_x, line));
            }
        }
    }

    synced
}

// ---------------------------------------------------------------------------
// Wedge parsing
// ---------------------------------------------------------------------------

fn parse_wedge_full(wedge: &AptImage) -> Vec<AptWedge> {
    if wedge.height < 128 {
        return Vec::new();
    }

    let sync_len = WEDGE_SYNC.len() * 8;
    let mut final_sync_wedge = vec![0i32; sync_len];
    for (i, &v) in WEDGE_SYNC.iter().enumerate() {
        for f in 0..8 {
            final_sync_wedge[i * 8 + f] = v as i32;
        }
    }

    // Average each horizontal line to 1 sample
    let mut wedge_a = vec![0u16; wedge.height];
    for line in 0..wedge.height {
        let mut sum = 0u32;
        for x in 0..wedge.width {
            sum += wedge.get(x, line) as u32;
        }
        wedge_a[line] = (sum / wedge.width as u32) as u16;
    }

    let block = 16 * 8;
    let max_line = (wedge_a.len().saturating_sub(sync_len)) / block;
    let mut wedges = Vec::new();

    for line in 0..max_line {
        let start_idx = line * block;
        let mut best_cor = i64::MAX;
        let mut best_pos = 0usize;

        for pos in 0..block {
            let mut cor = 0i64;
            for (i, &sync_val) in final_sync_wedge.iter().enumerate() {
                let sample = (wedge_a[start_idx + pos + i] >> 8) as i64;
                cor += (sample - sync_val as i64).abs();
            }
            if cor < best_cor {
                best_cor = cor;
                best_pos = pos;
            }
        }

        if start_idx + best_pos + 15 * 8 + 7 >= wedge_a.len() {
            break;
        }

        let mut final_wedge = [0u16; 16];
        for i in 0..16 {
            let mut sum = 0u32;
            for v in 0..8 {
                sum += wedge_a[start_idx + best_pos + i * 8 + v] as u32;
            }
            final_wedge[i] = (sum / 8) as u16;
        }

        let mut wed = AptWedge {
            start_line: start_idx + best_pos,
            ref1: final_wedge[0],
            ref2: final_wedge[1],
            ref3: final_wedge[2],
            ref4: final_wedge[3],
            ref5: final_wedge[4],
            ref6: final_wedge[5],
            ref7: final_wedge[6],
            ref8: final_wedge[7],
            zero_mod_ref: final_wedge[8],
            therm_temp1: final_wedge[9],
            therm_temp2: final_wedge[10],
            therm_temp3: final_wedge[11],
            therm_temp4: final_wedge[12],
            patch_temp: final_wedge[13],
            back_scan: final_wedge[14],
            channel: final_wedge[15],
            ..Default::default()
        };

        // Standard deviation of each wedge segment
        for c in 0..16 {
            let mut vals: Vec<f64> = Vec::with_capacity(41 * 8);
            for y in 0..8 {
                let sy = wed.start_line + c * 8 + y;
                if sy < wedge.height {
                    for x in 0..wedge.width {
                        vals.push(wedge.get(x, sy) as f64);
                    }
                }
            }
            if !vals.is_empty() {
                let mean = vals.iter().sum::<f64>() / vals.len() as f64;
                let variance = vals.iter().map(|&v| (v - mean) * (v - mean)).sum::<f64>()
                    / (vals.len() as f64);
                wed.std_dev[c] = variance.sqrt();
            }
        }

        // Identify channel from wedge reference bars
        let mut min_diff = MAX_STDDEV_VALID;
        let mut best_wedge = 0i32;
        if wed.std_dev[15] <= MAX_STDDEV_VALID {
            for i in 0..8 {
                if wed.std_dev[i] > MAX_STDDEV_VALID {
                    continue;
                }
                let diff = (final_wedge[i] as i32 - wed.channel as i32).abs();
                if (diff as f64) < min_diff {
                    best_wedge = (i + 1) as i32;
                    min_diff = diff as f64;
                }
            }
        }
        if (1..=5).contains(&best_wedge) {
            wed.rchannel = best_wedge;
        }

        wedges.push(wed);
    }

    wedges
}

fn get_calib_values_wedge(wedges: &[AptWedge]) -> (u16, u16) {
    let mut whites = Vec::new();
    let mut blacks = Vec::new();
    for w in wedges {
        if w.std_dev[7] < MAX_STDDEV_VALID {
            whites.push(w.ref8);
        }
        if w.std_dev[8] < MAX_STDDEV_VALID {
            blacks.push(w.zero_mod_ref);
        }
    }
    let white = if !whites.is_empty() {
        (whites.iter().map(|&v| v as u64).sum::<u64>() / whites.len() as u64) as u16
    } else {
        0
    };
    let black = if !blacks.is_empty() {
        (blacks.iter().map(|&v| v as u64).sum::<u64>() / blacks.len() as u64) as u16
    } else {
        0
    };
    (white, black)
}
