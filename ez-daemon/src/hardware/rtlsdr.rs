#![deny(unsafe_op_in_unsafe_fn)]
#![allow(unsafe_code)]

//! RTL-SDR [`IqSource`] backend. Mirrors `dump1090::sdr::rtlsdr`'s proven FFI bindings and
//! device lifecycle (open, gain stepping, device lookup by index/serial), but yields
//! phase-preserving `Complex32` samples via [`lrpt_decode::iq_bytes_to_complex`] instead of
//! discarding phase into a magnitude table — the daemon's spectrum and LRPT pipelines both
//! need phase information dump1090's magnitude-only path can't provide.

use std::ffi::{c_char, c_int, CStr};
use std::ptr;

use anyhow::Result;
use num_complex::Complex32;

use super::IqSource;

const READ_BUF_BYTES: usize = 16 * 16384; // 256 kB, matches dump1090's MODES_RTL_BUF_SIZE

#[derive(Debug, thiserror::Error)]
pub enum RtlSdrError {
    #[error("no RTL-SDR devices found")]
    NoDevices,
    #[error("device not found: {0}")]
    DeviceNotFound(String),
    #[error("rtlsdr_open failed: {0}")]
    OpenFailed(String),
    #[error("read sync failed")]
    ReadSyncFailed,
}

#[allow(dead_code)]
mod ffi {
    use std::ffi::{c_char, c_int, c_void};

    #[repr(C)]
    pub struct rtlsdr_dev {
        _private: [u8; 0],
    }

    #[link(name = "rtlsdr")]
    extern "C" {
        pub fn rtlsdr_get_device_count() -> u32;
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

fn find_device_index(name: &str) -> Result<u32, RtlSdrError> {
    // SAFETY: no preconditions beyond librtlsdr being linked.
    let count = unsafe { ffi::rtlsdr_get_device_count() };
    if count == 0 {
        return Err(RtlSdrError::NoDevices);
    }
    if let Ok(device) = name.parse::<u32>() {
        if device < count {
            return Ok(device);
        }
    }
    for i in 0..count {
        let mut serial = [0u8; 256];
        // SAFETY: `serial` is a 256-byte buffer; `null_mut()` for manufacturer/product is
        // explicitly supported by the API, which writes at most 256 bytes including the
        // null terminator.
        let ret = unsafe {
            ffi::rtlsdr_get_device_usb_strings(
                i,
                ptr::null_mut(),
                ptr::null_mut(),
                serial.as_mut_ptr().cast(),
            )
        };
        if ret == 0 {
            // SAFETY: `ret == 0` guarantees the FFI wrote a valid null-terminated string.
            let s = unsafe { CStr::from_ptr(serial.as_ptr().cast::<c_char>()) }.to_string_lossy();
            if s == name {
                return Ok(i);
            }
        }
    }
    Err(RtlSdrError::DeviceNotFound(name.to_string()))
}

pub struct RtlSdrSource {
    dev: *mut ffi::rtlsdr_dev,
    dev_name: Option<String>,
    frequency_hz: u64,
    sample_rate_hz: u32,
    gain_db: f64,
    gains: Vec<i32>,
    read_buf: Vec<u8>,
}

// SAFETY: `RtlSdrSource` only holds a raw device pointer touched through FFI calls that
// take `&mut self` at the Rust level, so `IqSource`'s `&mut self`-everywhere API already
// guarantees exclusive access — no concurrent FFI calls into the same handle can occur.
unsafe impl Send for RtlSdrSource {}

impl RtlSdrSource {
    #[must_use]
    pub fn new(device: Option<String>) -> Self {
        Self {
            dev: ptr::null_mut(),
            dev_name: device,
            frequency_hz: 433_000_000,
            sample_rate_hz: 2_048_000,
            gain_db: 0.0, // <= 0.0 selects tuner AGC, matching dump1090's convention
            gains: Vec::new(),
            read_buf: Vec::new(),
        }
    }

    fn apply_gain(&mut self) {
        if self.dev.is_null() || self.gains.is_empty() {
            return;
        }
        let gain_tenths = (self.gain_db * 10.0).round() as i32;
        let mut best_step = 0i32;
        let mut best_diff = i32::MAX;
        for (i, &g) in self.gains.iter().enumerate().take(self.gains.len() - 1) {
            let diff = (g - gain_tenths).abs();
            if diff < best_diff {
                best_diff = diff;
                best_step = i as i32;
            }
        }
        let selected = if self.gain_db <= 0.0 {
            self.gains.len() as i32 - 1
        } else {
            best_step
        };
        if selected as usize >= self.gains.len() - 1 {
            // SAFETY: `self.dev` checked non-null above.
            unsafe { ffi::rtlsdr_set_tuner_gain_mode(self.dev, 0) };
        } else {
            // SAFETY: `self.dev` checked non-null above.
            unsafe { ffi::rtlsdr_set_tuner_gain_mode(self.dev, 1) };
            // SAFETY: `self.dev` is valid and `selected` is bounds-checked against
            // `self.gains`, which was populated by this same device.
            unsafe { ffi::rtlsdr_set_tuner_gain(self.dev, self.gains[selected as usize]) };
        }
    }
}

impl IqSource for RtlSdrSource {
    fn start(&mut self) -> Result<()> {
        // SAFETY: no preconditions beyond librtlsdr being linked.
        if unsafe { ffi::rtlsdr_get_device_count() } == 0 {
            return Err(RtlSdrError::NoDevices.into());
        }
        let dev_index = match &self.dev_name {
            Some(name) => find_device_index(name)?,
            None => 0,
        };

        let mut dev = ptr::null_mut();
        // SAFETY: `&mut dev` is a valid, stack-local out-parameter for the FFI to write an
        // opened device handle into.
        if unsafe { ffi::rtlsdr_open(&mut dev, dev_index) } < 0 {
            return Err(RtlSdrError::OpenFailed(format!("index {dev_index}")).into());
        }
        self.dev = dev;

        // SAFETY: `dev` was just successfully opened above. Passing `null_mut()` queries
        // the gain count without writing.
        let numgains = unsafe { ffi::rtlsdr_get_tuner_gains(dev, ptr::null_mut()) };
        if numgains > 0 {
            self.gains.resize((numgains + 1) as usize, 0);
            // SAFETY: `self.gains` has at least `numgains` elements; the FFI writes
            // exactly that many `c_int`s.
            let ret = unsafe { ffi::rtlsdr_get_tuner_gains(dev, self.gains.as_mut_ptr()) };
            if ret == numgains {
                self.gains.truncate(numgains as usize);
                self.gains.sort_unstable();
                let last = self.gains.last().copied().unwrap_or(0);
                self.gains.push(last + 90);
            }
        }
        self.apply_gain();

        // SAFETY: `dev` is a valid, just-opened device handle.
        if unsafe { ffi::rtlsdr_set_center_freq(dev, self.frequency_hz as u32) } < 0 {
            // SAFETY: `dev` is still valid; closing to clean up on error.
            unsafe { ffi::rtlsdr_close(dev) };
            self.dev = ptr::null_mut();
            return Err(RtlSdrError::OpenFailed("failed to set center frequency".into()).into());
        }
        // SAFETY: `dev` is a valid, just-opened device handle.
        if unsafe { ffi::rtlsdr_set_sample_rate(dev, self.sample_rate_hz) } < 0 {
            // SAFETY: `dev` is still valid; closing to clean up on error.
            unsafe { ffi::rtlsdr_close(dev) };
            self.dev = ptr::null_mut();
            return Err(RtlSdrError::OpenFailed("failed to set sample rate".into()).into());
        }
        // SAFETY: `dev` is a valid, just-opened device handle.
        unsafe { ffi::rtlsdr_reset_buffer(dev) };

        Ok(())
    }

    fn stop(&mut self) {
        if !self.dev.is_null() {
            // SAFETY: `self.dev` is non-null and was returned by a successful
            // `rtlsdr_open`; `rtlsdr_close` is its matching destructor.
            unsafe { ffi::rtlsdr_close(self.dev) };
            self.dev = ptr::null_mut();
        }
    }

    fn set_frequency(&mut self, hz: u64) -> Result<()> {
        self.frequency_hz = hz;
        if !self.dev.is_null() {
            // SAFETY: `self.dev` checked non-null above.
            if unsafe { ffi::rtlsdr_set_center_freq(self.dev, hz as u32) } < 0 {
                return Err(RtlSdrError::OpenFailed("failed to set center frequency".into()).into());
            }
        }
        Ok(())
    }

    fn set_sample_rate(&mut self, hz: u32) -> Result<()> {
        self.sample_rate_hz = hz;
        if !self.dev.is_null() {
            // SAFETY: `self.dev` checked non-null above.
            if unsafe { ffi::rtlsdr_set_sample_rate(self.dev, hz) } < 0 {
                return Err(RtlSdrError::OpenFailed("failed to set sample rate".into()).into());
            }
        }
        Ok(())
    }

    fn set_gain(&mut self, db: f64) -> Result<()> {
        self.gain_db = db;
        self.apply_gain();
        Ok(())
    }

    fn read_iq(&mut self, buf: &mut [Complex32]) -> Result<usize> {
        if self.dev.is_null() {
            return Ok(0);
        }
        let want_bytes = (buf.len() * 2).min(READ_BUF_BYTES);
        self.read_buf.resize(want_bytes, 0);
        let mut n_read: c_int = 0;

        // SAFETY: `self.dev` checked non-null above. `self.read_buf` is a freshly sized
        // `Vec` of `want_bytes` — its pointer is valid and aligned for the duration of the
        // call. `n_read` is a stack-local `c_int` that outlives the call.
        let ret = unsafe {
            ffi::rtlsdr_read_sync(
                self.dev,
                self.read_buf.as_mut_ptr().cast(),
                want_bytes as c_int,
                &mut n_read,
            )
        };
        if ret < 0 {
            return Err(RtlSdrError::ReadSyncFailed.into());
        }

        let bytes_read = n_read as usize;
        let complex = lrpt_decode::iq_bytes_to_complex(&self.read_buf[..bytes_read]);
        let n = complex.len().min(buf.len());
        buf[..n].copy_from_slice(&complex[..n]);
        Ok(n)
    }

    fn frequency_hz(&self) -> u64 {
        self.frequency_hz
    }

    fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz
    }

    fn gain_db(&self) -> f64 {
        self.gain_db
    }

    fn kind(&self) -> &'static str {
        "rtlsdr"
    }
}

impl Drop for RtlSdrSource {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_source_has_null_device_and_sane_defaults() {
        let src = RtlSdrSource::new(None);
        assert!(src.dev.is_null());
        assert_eq!(src.frequency_hz(), 433_000_000);
        assert_eq!(src.sample_rate_hz(), 2_048_000);
        assert_eq!(src.kind(), "rtlsdr");
    }

    #[test]
    fn read_iq_on_unstarted_source_returns_zero_without_touching_hardware() {
        let mut src = RtlSdrSource::new(None);
        let mut buf = vec![Complex32::new(0.0, 0.0); 16];
        assert_eq!(src.read_iq(&mut buf).unwrap(), 0);
    }

    #[test]
    fn drop_on_never_started_source_does_not_panic() {
        let src = RtlSdrSource::new(Some("0".into()));
        drop(src);
    }

    #[test]
    fn is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<RtlSdrSource>();
    }
}
