#![deny(unsafe_op_in_unsafe_fn)]
#![allow(unsafe_code)]

//! RTL-SDR [`IqSource`] backend. Mirrors `dump1090::sdr::rtlsdr`'s proven FFI bindings and
//! device lifecycle (open, gain stepping, device lookup by index/serial), but yields
//! phase-preserving `Complex32` samples via [`lrpt_decode::uc8_bytes_to_normalized`] instead of
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
    requested_gain_db: f64,
    gains: Vec<i32>,
    read_buf: Vec<u8>,
}

// SAFETY: `RtlSdrSource` only holds a raw device pointer touched through FFI calls that
// take `&mut self` at the Rust level, so `IqSource`'s `&mut self`-everywhere API already
// guarantees exclusive access — no concurrent FFI calls into the same handle can occur.
unsafe impl Send for RtlSdrSource {}

/// Shared by startup and live changes; the returned dB is the supported step,
/// or zero for automatic mode. Errors leave the caller's applied cache intact.
fn apply_tuner_gain(
    gains: &[i32],
    requested: f64,
    mut set_mode: impl FnMut(i32) -> i32,
    mut set_gain: impl FnMut(i32) -> i32,
) -> Result<f64> {
    anyhow::ensure!(requested.is_finite(), "gain must be finite");
    if requested <= 0.0 {
        let code = set_mode(0);
        anyhow::ensure!(code == 0, "RTL set automatic gain mode failed: {code}");
        return Ok(0.0);
    }
    let target = requested * 10.0;
    let step = *gains
        .iter()
        .min_by(|a, b| {
            (f64::from(**a) - target)
                .abs()
                .total_cmp(&(f64::from(**b) - target).abs())
        })
        .ok_or_else(|| anyhow::anyhow!("RTL tuner has no supported manual gains"))?;
    let code = set_mode(1);
    anyhow::ensure!(code == 0, "RTL set manual gain mode failed: {code}");
    let code = set_gain(step);
    anyhow::ensure!(code == 0, "RTL set tuner gain failed: {code}");
    Ok(f64::from(step) / 10.0)
}

impl RtlSdrSource {
    #[must_use]
    pub fn new(device: Option<String>) -> Self {
        Self {
            dev: ptr::null_mut(),
            dev_name: device,
            frequency_hz: 433_000_000,
            sample_rate_hz: 2_048_000,
            requested_gain_db: 0.0,
            gain_db: 0.0, // <= 0.0 selects tuner AGC, matching dump1090's convention
            gains: Vec::new(),
            read_buf: Vec::new(),
        }
    }

    fn apply_gain(&mut self, requested: f64) -> Result<f64> {
        anyhow::ensure!(requested.is_finite(), "gain must be finite");
        if self.dev.is_null() {
            return Ok(if requested <= 0.0 { 0.0 } else { requested });
        }
        apply_tuner_gain(
            &self.gains,
            requested,
            // SAFETY: device is non-null and exclusively owned by this source.
            |manual| unsafe { ffi::rtlsdr_set_tuner_gain_mode(self.dev, manual) },
            // SAFETY: selected gain comes from this device's supported table.
            |gain| unsafe { ffi::rtlsdr_set_tuner_gain(self.dev, gain) },
        )
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
            self.gains.resize(numgains as usize, 0);
            // SAFETY: `self.gains` has at least `numgains` elements; the FFI writes
            // exactly that many `c_int`s.
            let ret = unsafe { ffi::rtlsdr_get_tuner_gains(dev, self.gains.as_mut_ptr()) };
            if ret != numgains {
                self.gains.clear();
                self.stop();
                anyhow::bail!("RTL gain table changed or read failed: {ret}");
            }
            self.gains.sort_unstable();
        }
        match self.apply_gain(self.requested_gain_db) {
            Ok(applied) => self.gain_db = applied,
            Err(error) => {
                self.stop();
                return Err(error);
            }
        }

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
        if unsafe { ffi::rtlsdr_reset_buffer(dev) } != 0 {
            self.stop();
            anyhow::bail!("RTL reset buffer failed");
        }

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
        anyhow::ensure!(hz <= u32::MAX as u64, "RTL frequency exceeds u32 range");
        if !self.dev.is_null() {
            // SAFETY: `self.dev` checked non-null above.
            if unsafe { ffi::rtlsdr_set_center_freq(self.dev, hz as u32) } < 0 {
                return Err(
                    RtlSdrError::OpenFailed("failed to set center frequency".into()).into(),
                );
            }
        }
        self.frequency_hz = hz;
        Ok(())
    }

    fn set_sample_rate(&mut self, hz: u32) -> Result<()> {
        anyhow::ensure!(hz > 0, "sample rate must be positive");
        if !self.dev.is_null() {
            // SAFETY: `self.dev` checked non-null above.
            if unsafe { ffi::rtlsdr_set_sample_rate(self.dev, hz) } < 0 {
                return Err(RtlSdrError::OpenFailed("failed to set sample rate".into()).into());
            }
        }
        self.sample_rate_hz = hz;
        Ok(())
    }

    fn set_gain(&mut self, db: f64) -> Result<()> {
        let applied = self.apply_gain(db)?;
        self.gain_db = applied;
        self.requested_gain_db = db;
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

        let bytes_read = (n_read.max(0) as usize).min(self.read_buf.len());
        let complex = lrpt_decode::uc8_bytes_to_normalized(&self.read_buf[..bytes_read]);
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

    fn requested_gain_db(&self) -> Option<f64> {
        Some(self.requested_gain_db)
    }
    fn gain_mode(&self) -> Option<ez_proto::GainMode> {
        Some(if self.requested_gain_db <= 0.0 {
            ez_proto::GainMode::Automatic
        } else {
            ez_proto::GainMode::Manual
        })
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
    fn gain_step_and_error_contract_is_shared_by_startup_and_live_control() {
        use std::cell::RefCell;
        let calls = RefCell::new(Vec::new());
        let apply = |requested, mode_error, gain_error| {
            apply_tuner_gain(
                &[0, 77, 147, 297, 496],
                requested,
                |mode| {
                    calls.borrow_mut().push(("mode", mode));
                    mode_error
                },
                |gain| {
                    calls.borrow_mut().push(("gain", gain));
                    gain_error
                },
            )
        };
        assert_eq!(apply(29.9, 0, 0).unwrap(), 29.7);
        assert_eq!(*calls.borrow(), [("mode", 1), ("gain", 297)]);
        calls.borrow_mut().clear();
        assert_eq!(apply(0.0, 0, 0).unwrap(), 0.0);
        assert_eq!(*calls.borrow(), [("mode", 0)]);
        calls.borrow_mut().clear();
        assert!(apply(15.0, -1, 0).is_err());
        assert_eq!(*calls.borrow(), [("mode", 1)]);
        assert!(apply(15.0, 0, -2).is_err());
        assert!(apply(-1.0, -3, 0).is_err());
        assert!(apply(f64::NAN, 0, 0).is_err());
        assert_eq!(
            apply_tuner_gain(&[], 0.0, |_| 0, |_| panic!("AGC must not set gain")).unwrap(),
            0.0
        );
        assert!(apply_tuner_gain(&[], 1.0, |_| panic!("no gains"), |_| 0).is_err());
    }

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

    #[test]
    fn n_read_clamping_prevents_out_of_bounds() {
        let read_buf_len = 1024usize;

        // Negative n_read clamps to 0
        let neg_n_read: c_int = -1;
        let bytes_read = (neg_n_read.max(0) as usize).min(read_buf_len);
        assert_eq!(bytes_read, 0);

        // Excess n_read clamps to read_buf.len()
        let excess_n_read: c_int = 2048;
        let bytes_read = (excess_n_read.max(0) as usize).min(read_buf_len);
        assert_eq!(bytes_read, 1024);

        // Valid n_read within bounds is preserved
        let normal_n_read: c_int = 512;
        let bytes_read = (normal_n_read.max(0) as usize).min(read_buf_len);
        assert_eq!(bytes_read, 512);
    }
}
