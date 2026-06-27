//! RTL-SDR source - translated from sdr_rtlsdr.c

use std::ffi::{c_char, c_int, CStr};
use std::ptr;

use crate::convert;
use crate::sdr::SdrSource;

const MODES_RTL_BUF_SIZE: usize = 16 * 16384; // 256 kB

#[derive(Debug, thiserror::Error)]
pub enum RtlSdrError {
    #[error("no RTL-SDR devices found")]
    NoDevices,
    #[error("device not found: {0}")]
    DeviceNotFound(String),
    #[error("rtlsdr_open failed: {0}")]
    OpenFailed(String),
    #[error("gain control not supported")]
    GainControlNotSupported,
    #[error("failed to set gain")]
    SetGainFailed,
    #[error("read sync failed")]
    ReadSyncFailed,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

mod ffi {
    use std::ffi::{c_char, c_int, c_void};

    #[repr(C)]
    pub struct rtlsdr_dev {
        _private: [u8; 0],
    }

    extern "C" {
        pub fn rtlsdr_get_device_count() -> u32;
        pub fn rtlsdr_get_device_name(index: u32) -> *const c_char;
        pub fn rtlsdr_get_device_usb_strings(
            index: u32,
            manufacturer: *mut c_char,
            product: *mut c_char,
            serial: *mut c_char,
        ) -> c_int;
        pub fn rtlsdr_open(dev: *mut *mut rtlsdr_dev, index: u32) -> c_int;
        pub fn rtlsdr_close(dev: *mut rtlsdr_dev) -> c_int;
        pub fn rtlsdr_set_center_freq(dev: *mut rtlsdr_dev, freq: u32) -> c_int;
        pub fn rtlsdr_set_sample_rate(dev: *mut rtlsdr_dev, rate: u32) -> c_int;
        pub fn rtlsdr_set_tuner_gain(dev: *mut rtlsdr_dev, gain: c_int) -> c_int;
        pub fn rtlsdr_set_tuner_gain_mode(dev: *mut rtlsdr_dev, manual: c_int) -> c_int;
        pub fn rtlsdr_set_agc_mode(dev: *mut rtlsdr_dev, on: c_int) -> c_int;
        pub fn rtlsdr_set_freq_correction(dev: *mut rtlsdr_dev, ppm: c_int) -> c_int;
        pub fn rtlsdr_set_direct_sampling(dev: *mut rtlsdr_dev, on: c_int) -> c_int;
        pub fn rtlsdr_get_tuner_gains(dev: *mut rtlsdr_dev, gains: *mut c_int) -> c_int;
        pub fn rtlsdr_reset_buffer(dev: *mut rtlsdr_dev) -> c_int;
        pub fn rtlsdr_read_sync(
            dev: *mut rtlsdr_dev,
            buf: *mut c_void,
            len: c_int,
            n_read: *mut c_int,
        ) -> c_int;
    }
}

fn show_devices() {
    let count = unsafe { ffi::rtlsdr_get_device_count() };
    eprintln!("rtlsdr: found {count} device(s):");
    for i in 0..count {
        let mut mfg = [0u8; 256];
        let mut prod = [0u8; 256];
        let mut serial = [0u8; 256];
        let ret = unsafe {
            ffi::rtlsdr_get_device_usb_strings(
                i,
                mfg.as_mut_ptr() as *mut c_char,
                prod.as_mut_ptr() as *mut c_char,
                serial.as_mut_ptr() as *mut c_char,
            )
        };
        if ret != 0 {
            eprintln!("  {i}:  unable to read device details");
        } else {
            let mfg_s = unsafe { CStr::from_ptr(mfg.as_ptr().cast()) }.to_string_lossy();
            let prod_s = unsafe { CStr::from_ptr(prod.as_ptr().cast()) }.to_string_lossy();
            let serial_s = unsafe { CStr::from_ptr(serial.as_ptr().cast()) }.to_string_lossy();
            eprintln!("  {i}:  {mfg_s}, {prod_s}, SN: {serial_s}");
        }
    }
}

fn find_device_index(name: &str) -> Result<u32, RtlSdrError> {
    let count = unsafe { ffi::rtlsdr_get_device_count() };
    if count == 0 {
        return Err(RtlSdrError::NoDevices);
    }
    if name == "0" {
        return Ok(0);
    }
    if !name.starts_with('0') {
        if let Ok(device) = name.parse::<u32>() {
            if device < count {
                return Ok(device);
            }
        }
    }
    for i in 0..count {
        let mut serial = [0u8; 256];
        let ret = unsafe {
            ffi::rtlsdr_get_device_usb_strings(i, ptr::null_mut(), ptr::null_mut(), serial.as_mut_ptr().cast())
        };
        if ret == 0 {
            let s = unsafe { CStr::from_ptr(serial.as_ptr().cast()) }.to_string_lossy();
            if s == name {
                return Ok(i);
            }
        }
    }
    for i in 0..count {
        let mut serial = [0u8; 256];
        let ret = unsafe {
            ffi::rtlsdr_get_device_usb_strings(i, ptr::null_mut(), ptr::null_mut(), serial.as_mut_ptr().cast())
        };
        if ret == 0 {
            let s = unsafe { CStr::from_ptr(serial.as_ptr().cast()) }.to_string_lossy();
            if s.starts_with(name) {
                return Ok(i);
            }
        }
    }
    for i in 0..count {
        let mut serial = [0u8; 256];
        let ret = unsafe {
            ffi::rtlsdr_get_device_usb_strings(i, ptr::null_mut(), ptr::null_mut(), serial.as_mut_ptr().cast())
        };
        if ret == 0 {
            let s = unsafe { CStr::from_ptr(serial.as_ptr().cast()) }.to_string_lossy();
            if s.ends_with(name) && name.len() < s.len() {
                return Ok(i);
            }
        }
    }
    Err(RtlSdrError::DeviceNotFound(name.to_string()))
}

pub struct RtlSdr {
    dev: *mut ffi::rtlsdr_dev,
    dev_index: u32,
    dev_name: Option<String>,
    freq: u64,
    sample_rate: u32,
    gain: f64,
    ppm: i32,
    direct_sampling: i32,
    digital_agc: bool,
    gains: Vec<i32>,
}

unsafe impl Send for RtlSdr {}

impl RtlSdr {
    pub fn new(
        dev_name: Option<String>,
        freq: u64,
        sample_rate: u32,
        gain: f64,
        ppm: i32,
        direct_sampling: i32,
        digital_agc: bool,
    ) -> Self {
        Self {
            dev: ptr::null_mut(),
            dev_index: 0,
            dev_name,
            freq,
            sample_rate,
            gain,
            ppm,
            direct_sampling,
            digital_agc,
            gains: Vec::new(),
        }
    }

    pub fn list_devices() {
        show_devices();
    }
}

impl SdrSource for RtlSdr {
    fn start(&mut self) -> anyhow::Result<()> {
        if unsafe { ffi::rtlsdr_get_device_count() } == 0 {
            return Err(RtlSdrError::NoDevices.into());
        }

        let dev_index = if let Some(ref name) = self.dev_name {
            match find_device_index(name) {
                Ok(i) => i,
                Err(e) => {
                    show_devices();
                    return Err(e.into());
                }
            }
        } else {
            0
        };
        self.dev_index = dev_index;

        let mut dev = ptr::null_mut();
        if unsafe { ffi::rtlsdr_open(&mut dev, dev_index) } < 0 {
            return Err(RtlSdrError::OpenFailed(format!(
                "error opening the RTLSDR device: {}",
                std::io::Error::last_os_error()
            )).into());
        }

        if self.direct_sampling != 0 {
            eprintln!("rtlsdr: direct sampling from input {}", self.direct_sampling);
            unsafe { ffi::rtlsdr_set_direct_sampling(dev, self.direct_sampling); }
        } else {
            let numgains = unsafe { ffi::rtlsdr_get_tuner_gains(dev, ptr::null_mut()) };
            if numgains > 0 {
                self.gains.resize((numgains + 1) as usize, 0);
                let ret = unsafe { ffi::rtlsdr_get_tuner_gains(dev, self.gains.as_mut_ptr()) };
                if ret == numgains {
                    self.gains.truncate(numgains as usize);
                    self.gains.sort_unstable();
                    let last = self.gains.last().copied().unwrap_or(0);
                    self.gains.push(last + 90);
                }
            }

            if !self.gains.is_empty() {
                let gain_tenths = (self.gain * 10.0).round() as i32;
                let mut best_step = 0i32;
                let mut best_diff = i32::MAX;
                for (i, &g) in self.gains.iter().enumerate().take(self.gains.len() - 1) {
                    let diff = (g - gain_tenths).abs();
                    if diff < best_diff {
                        best_diff = diff;
                        best_step = i as i32;
                    }
                }
                let selected = if self.gain <= 0.0 {
                    self.gains.len() as i32 - 1
                } else {
                    best_step
                };

                if selected as usize >= self.gains.len() - 1 {
                    unsafe { ffi::rtlsdr_set_tuner_gain_mode(dev, 0); }
                    eprintln!("rtlsdr: tuner AGC enabled");
                } else {
                    unsafe { ffi::rtlsdr_set_tuner_gain_mode(dev, 1); }
                    unsafe { ffi::rtlsdr_set_tuner_gain(dev, self.gains[selected as usize]); }
                    eprintln!(
                        "rtlsdr: tuner gain set to {:.1} dB",
                        self.gains[selected as usize] as f64 / 10.0
                    );
                }
            }
        }

        if self.digital_agc {
            eprintln!("rtlsdr: enabling digital AGC");
            unsafe { ffi::rtlsdr_set_agc_mode(dev, 1); }
        }

        if unsafe { ffi::rtlsdr_set_freq_correction(dev, self.ppm) } < 0 {
            eprintln!("rtlsdr: warning: failed to set frequency correction");
        }

        if unsafe { ffi::rtlsdr_set_center_freq(dev, self.freq as u32) } < 0 {
            unsafe { ffi::rtlsdr_close(dev); }
            return Err(RtlSdrError::OpenFailed("failed to set center frequency".into()).into());
        }

        if unsafe { ffi::rtlsdr_set_sample_rate(dev, self.sample_rate) } < 0 {
            unsafe { ffi::rtlsdr_close(dev); }
            return Err(RtlSdrError::OpenFailed("failed to set sample rate".into()).into());
        }

        if unsafe { ffi::rtlsdr_reset_buffer(dev) } < 0 {
            eprintln!("rtlsdr: warning: failed to reset buffer");
        }

        self.dev = dev;
        Ok(())
    }

    fn stop(&mut self) {
        if !self.dev.is_null() {
            unsafe { ffi::rtlsdr_close(self.dev); }
            self.dev = ptr::null_mut();
        }
    }

    fn set_frequency(&mut self, freq: u64) -> anyhow::Result<()> {
        self.freq = freq;
        if !self.dev.is_null() {
            if unsafe { ffi::rtlsdr_set_center_freq(self.dev, freq as u32) } < 0 {
                return Err(RtlSdrError::OpenFailed("failed to set center frequency".into()).into());
            }
        }
        Ok(())
    }

    fn set_sample_rate(&mut self, rate: u32) -> anyhow::Result<()> {
        self.sample_rate = rate;
        if !self.dev.is_null() {
            if unsafe { ffi::rtlsdr_set_sample_rate(self.dev, rate) } < 0 {
                return Err(RtlSdrError::OpenFailed("failed to set sample rate".into()).into());
            }
        }
        Ok(())
    }

    fn set_gain(&mut self, gain: f64) -> anyhow::Result<()> {
        self.gain = gain;
        if !self.dev.is_null() && !self.gains.is_empty() {
            let gain_tenths = (gain * 10.0).round() as i32;
            let mut best_step = 0i32;
            let mut best_diff = i32::MAX;
            for (i, &g) in self.gains.iter().enumerate().take(self.gains.len() - 1) {
                let diff = (g - gain_tenths).abs();
                if diff < best_diff {
                    best_diff = diff;
                    best_step = i as i32;
                }
            }
            let selected = if gain <= 0.0 {
                self.gains.len() as i32 - 1
            } else {
                best_step
            };
            if selected as usize >= self.gains.len() - 1 {
                unsafe { ffi::rtlsdr_set_tuner_gain_mode(self.dev, 0); }
            } else {
                unsafe { ffi::rtlsdr_set_tuner_gain_mode(self.dev, 1); }
                unsafe { ffi::rtlsdr_set_tuner_gain(self.dev, self.gains[selected as usize]); }
            }
        }
        Ok(())
    }

    fn read_samples(&mut self, buf: &mut [u16]) -> anyhow::Result<usize> {
        if self.dev.is_null() {
            return Ok(0);
        }

        let want_bytes = (buf.len() * 2).min(MODES_RTL_BUF_SIZE);
        let mut read_buf = vec![0u8; want_bytes];
        let mut n_read: c_int = 0;

        let ret = unsafe {
            ffi::rtlsdr_read_sync(
                self.dev,
                read_buf.as_mut_ptr().cast(),
                want_bytes as c_int,
                &mut n_read,
            )
        };

        if ret < 0 {
            return Err(RtlSdrError::ReadSyncFailed.into());
        }

        let bytes_read = n_read as usize;
        let samples_read = bytes_read / 2;

        if samples_read > 0 {
            convert::convert_uc8_to_mag(&read_buf[..bytes_read], &mut buf[..samples_read]);
        }

        Ok(samples_read)
    }
}

impl Drop for RtlSdr {
    fn drop(&mut self) {
        self.stop();
    }
}
