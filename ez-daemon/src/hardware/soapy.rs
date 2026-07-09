#![deny(unsafe_op_in_unsafe_fn)]
#![allow(unsafe_code)]

//! SoapySDR [`IqSource`] backend — a vendor-neutral hardware abstraction layer covering
//! many SDR families (LimeSDR, PlutoSDR, BladeRF, Airspy, ...) through one C ABI. Mirrors
//! `dump1090::sdr::soapy`'s proven FFI bindings and CS16 stream format, but decodes the
//! stream into phase-preserving `Complex32` samples via a local CS16->complex converter
//! instead of discarding phase into a magnitude table.

use std::ffi::{c_int, c_void, CStr, CString};
use std::ptr;

use anyhow::Result;
use num_complex::Complex32;

use super::IqSource;

const SOAPY_SDR_RX: i32 = 2;
const SOAPY_SDR_CS16: &CStr = c"CS16";

#[derive(Debug, thiserror::Error)]
pub enum SoapyError {
    #[error("no SoapySDR device found matching {0:?}")]
    NoDevice(Option<String>),
    #[error("multiple SoapySDR devices matched {0:?}; specify a more precise device string")]
    AmbiguousDevice(Option<String>),
    #[error("{0} failed: {1}")]
    CallFailed(&'static str, String),
}

#[allow(dead_code)]
mod ffi {
    use std::ffi::{c_char, c_int, c_long, c_void};

    #[repr(C)]
    pub struct SoapySDRDevice {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct SoapySDRStream {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct SoapySDRKwargs {
        pub size: usize,
        pub keys: *mut *mut c_char,
        pub vals: *mut *mut c_char,
    }

    #[link(name = "SoapySDR")]
    extern "C" {
        pub fn SoapySDRDevice_enumerateStrArgs(
            args: *const c_char,
            length: *mut usize,
        ) -> *mut SoapySDRKwargs;
        pub fn SoapySDRKwargsList_clear(args: *mut SoapySDRKwargs, length: usize);
        pub fn SoapySDRDevice_makeStrArgs(args: *const c_char) -> *mut SoapySDRDevice;
        pub fn SoapySDRDevice_unmake(dev: *mut SoapySDRDevice);
        pub fn SoapySDRDevice_lastError() -> *const c_char;
        pub fn SoapySDRDevice_setSampleRate(
            dev: *mut SoapySDRDevice,
            direction: i32,
            channel: usize,
            rate: f64,
        ) -> c_int;
        pub fn SoapySDRDevice_setFrequency(
            dev: *mut SoapySDRDevice,
            direction: i32,
            channel: usize,
            freq: f64,
            args: *const SoapySDRKwargs,
        ) -> c_int;
        pub fn SoapySDRDevice_hasGainMode(
            dev: *mut SoapySDRDevice,
            direction: i32,
            channel: usize,
        ) -> bool;
        pub fn SoapySDRDevice_setGainMode(
            dev: *mut SoapySDRDevice,
            direction: i32,
            channel: usize,
            automatic: bool,
        ) -> c_int;
        pub fn SoapySDRDevice_setGain(
            dev: *mut SoapySDRDevice,
            direction: i32,
            channel: usize,
            gain: f64,
        ) -> c_int;
        pub fn SoapySDRDevice_setupStream(
            dev: *mut SoapySDRDevice,
            direction: i32,
            format: *const c_char,
            channels: *const usize,
            num_chans: usize,
            args: *const SoapySDRKwargs,
        ) -> *mut SoapySDRStream;
        pub fn SoapySDRDevice_activateStream(
            dev: *mut SoapySDRDevice,
            stream: *mut SoapySDRStream,
            flags: c_int,
            time_ns: i64,
            num_elems: usize,
        ) -> c_int;
        pub fn SoapySDRDevice_readStream(
            dev: *mut SoapySDRDevice,
            stream: *mut SoapySDRStream,
            buffs: *mut *mut c_void,
            num_elems: usize,
            flags: *mut c_int,
            time_ns: *mut i64,
            timeout_us: c_long,
        ) -> c_int;
        pub fn SoapySDRDevice_closeStream(dev: *mut SoapySDRDevice, stream: *mut SoapySDRStream);
    }
}

fn last_err() -> String {
    // SAFETY: `SoapySDRDevice_lastError` returns either null (no error) or a pointer to a
    // static, library-owned C string that we only read, never free or mutate.
    unsafe {
        let ptr = ffi::SoapySDRDevice_lastError();
        if ptr.is_null() {
            "unknown error".to_string()
        } else {
            // SAFETY: `ptr` is non-null, so it points to a valid NUL-terminated C string
            // owned by the SoapySDR runtime for the duration of this call.
            CStr::from_ptr(ptr).to_string_lossy().into_owned()
        }
    }
}

fn soapy_check(op: &'static str, code: c_int) -> Result<(), SoapyError> {
    if code == 0 {
        Ok(())
    } else {
        Err(SoapyError::CallFailed(op, last_err()))
    }
}

/// Converts interleaved little-endian signed-16-bit I/Q samples (SoapySDR's `"CS16"` stream
/// format) into phase-preserving, full-scale-normalized `Complex32`.
fn cs16_le_bytes_to_complex(bytes: &[u8]) -> Vec<Complex32> {
    bytes
        .chunks_exact(4)
        .map(|c| {
            let i = i16::from_le_bytes([c[0], c[1]]);
            let q = i16::from_le_bytes([c[2], c[3]]);
            Complex32::new(f32::from(i) / 32768.0, f32::from(q) / 32768.0)
        })
        .collect()
}

pub struct SoapySource {
    dev: *mut ffi::SoapySDRDevice,
    stream: *mut ffi::SoapySDRStream,
    dev_name: Option<String>,
    frequency_hz: u64,
    sample_rate_hz: u32,
    gain_db: f64,
    byte_buf: Vec<u8>,
}

// SAFETY: `SoapySource` only touches its raw device/stream pointers through FFI calls made
// under `&mut self` (the `IqSource` trait is exclusively `&mut self`), so no concurrent
// access from safe code can occur; SoapySDR itself synchronises internally.
unsafe impl Send for SoapySource {}

impl SoapySource {
    #[must_use]
    pub fn new(device: Option<String>) -> Self {
        Self {
            dev: ptr::null_mut(),
            stream: ptr::null_mut(),
            dev_name: device,
            frequency_hz: 100_000_000,
            sample_rate_hz: 2_048_000,
            gain_db: 0.0, // <= 0.0 selects AGC, matching RtlSdrSource's convention
            byte_buf: Vec::new(),
        }
    }

    fn apply_gain(&mut self) -> Result<()> {
        if self.dev.is_null() {
            return Ok(());
        }
        let dev = self.dev;
        // SAFETY: `dev` checked non-null above.
        let has_gain_mode = unsafe { ffi::SoapySDRDevice_hasGainMode(dev, SOAPY_SDR_RX, 0) };
        if self.gain_db <= 0.0 {
            if has_gain_mode {
                // SAFETY: `dev` checked non-null above.
                soapy_check("setGainMode", unsafe {
                    ffi::SoapySDRDevice_setGainMode(dev, SOAPY_SDR_RX, 0, true)
                })?;
            }
        } else {
            if has_gain_mode {
                // SAFETY: `dev` checked non-null above.
                soapy_check("setGainMode", unsafe {
                    ffi::SoapySDRDevice_setGainMode(dev, SOAPY_SDR_RX, 0, false)
                })?;
            }
            // SAFETY: `dev` checked non-null above.
            soapy_check("setGain", unsafe {
                ffi::SoapySDRDevice_setGain(dev, SOAPY_SDR_RX, 0, self.gain_db)
            })?;
        }
        Ok(())
    }
}

impl IqSource for SoapySource {
    fn start(&mut self) -> Result<()> {
        if !self.dev.is_null() {
            return Ok(());
        }
        let args = CString::new(self.dev_name.as_deref().unwrap_or(""))?;
        let mut length: usize = 0;
        // SAFETY: `args` is a valid NUL-terminated CString; `length` is a stack-local
        // out-parameter for the FFI to write into.
        let results = unsafe { ffi::SoapySDRDevice_enumerateStrArgs(args.as_ptr(), &mut length) };
        // SAFETY: `results`/`length` come directly from the `enumerateStrArgs` call above;
        // this cleanup call is required regardless of how many results were found.
        unsafe { ffi::SoapySDRKwargsList_clear(results, length) };
        if length == 0 {
            return Err(SoapyError::NoDevice(self.dev_name.clone()).into());
        }
        if length > 1 {
            return Err(SoapyError::AmbiguousDevice(self.dev_name.clone()).into());
        }

        // SAFETY: `args` is a valid NUL-terminated CString; the returned handle is later
        // destroyed via `SoapySDRDevice_unmake` in `stop`.
        let dev = unsafe { ffi::SoapySDRDevice_makeStrArgs(args.as_ptr()) };
        if dev.is_null() {
            return Err(SoapyError::CallFailed("makeStrArgs", last_err()).into());
        }
        self.dev = dev;

        // SAFETY: `dev` was just successfully created above.
        if let Err(e) = soapy_check("setSampleRate", unsafe {
            ffi::SoapySDRDevice_setSampleRate(dev, SOAPY_SDR_RX, 0, self.sample_rate_hz as f64)
        }) {
            self.stop();
            return Err(e.into());
        }
        // SAFETY: `dev` is valid.
        if let Err(e) = soapy_check("setFrequency", unsafe {
            ffi::SoapySDRDevice_setFrequency(
                dev,
                SOAPY_SDR_RX,
                0,
                self.frequency_hz as f64,
                ptr::null(),
            )
        }) {
            self.stop();
            return Err(e.into());
        }
        if let Err(e) = self.apply_gain() {
            self.stop();
            return Err(e);
        }

        let channels: [usize; 1] = [0];
        let stream_args = ffi::SoapySDRKwargs {
            size: 0,
            keys: ptr::null_mut(),
            vals: ptr::null_mut(),
        };
        // SAFETY: `dev` is valid; `SOAPY_SDR_CS16` is a valid NUL-terminated format string;
        // `channels` is a live local array; `stream_args` is a zero-length kwargs struct
        // (no extra args), matching the SoapySDR API's documented "no args" convention.
        let stream = unsafe {
            ffi::SoapySDRDevice_setupStream(
                dev,
                SOAPY_SDR_RX,
                SOAPY_SDR_CS16.as_ptr(),
                channels.as_ptr(),
                1,
                &stream_args,
            )
        };
        if stream.is_null() {
            let err = SoapyError::CallFailed("setupStream", last_err());
            self.stop();
            return Err(err.into());
        }
        self.stream = stream;

        // SAFETY: `dev` is valid and `stream` was just returned by `setupStream` above.
        if let Err(e) = soapy_check("activateStream", unsafe {
            ffi::SoapySDRDevice_activateStream(dev, stream, 0, 0, 0)
        }) {
            self.stop();
            return Err(e.into());
        }

        Ok(())
    }

    fn stop(&mut self) {
        if !self.stream.is_null() {
            // SAFETY: `self.stream` is non-null and was returned by a successful
            // `setupStream`; `self.dev` is its matching, still-valid device handle.
            unsafe { ffi::SoapySDRDevice_closeStream(self.dev, self.stream) };
            self.stream = ptr::null_mut();
        }
        if !self.dev.is_null() {
            // SAFETY: `self.dev` is non-null and was returned by a successful
            // `makeStrArgs`.
            unsafe { ffi::SoapySDRDevice_unmake(self.dev) };
            self.dev = ptr::null_mut();
        }
    }

    fn set_frequency(&mut self, hz: u64) -> Result<()> {
        self.frequency_hz = hz;
        if !self.dev.is_null() {
            // SAFETY: `self.dev` checked non-null above.
            soapy_check("setFrequency", unsafe {
                ffi::SoapySDRDevice_setFrequency(self.dev, SOAPY_SDR_RX, 0, hz as f64, ptr::null())
            })?;
        }
        Ok(())
    }

    fn set_sample_rate(&mut self, hz: u32) -> Result<()> {
        self.sample_rate_hz = hz;
        if !self.dev.is_null() {
            // SAFETY: `self.dev` checked non-null above.
            soapy_check("setSampleRate", unsafe {
                ffi::SoapySDRDevice_setSampleRate(self.dev, SOAPY_SDR_RX, 0, hz as f64)
            })?;
        }
        Ok(())
    }

    fn set_gain(&mut self, db: f64) -> Result<()> {
        self.gain_db = db;
        self.apply_gain()
    }

    fn read_iq(&mut self, buf: &mut [Complex32]) -> Result<usize> {
        if self.dev.is_null() || self.stream.is_null() {
            return Ok(0);
        }
        let want_samples = buf.len();
        self.byte_buf.resize(want_samples * 4, 0);
        let mut buf_ptr = self.byte_buf.as_mut_ptr().cast::<c_void>();
        let mut flags: c_int = 0;
        let mut time_ns: i64 = 0;

        // SAFETY: `self.dev`/`self.stream` are checked non-null above. `buf_ptr` points
        // into `self.byte_buf`, freshly sized to `want_samples * 4` bytes (CS16 = 4
        // bytes/sample); `SoapySDRDevice_readStream` writes at most `want_samples` elements
        // through it. `flags`/`time_ns` are stack locals valid for the call.
        let samples_read = unsafe {
            ffi::SoapySDRDevice_readStream(
                self.dev,
                self.stream,
                &mut buf_ptr,
                want_samples,
                &mut flags,
                &mut time_ns,
                500_000,
            )
        };
        if samples_read <= 0 {
            return Ok(0);
        }

        let n_bytes = samples_read as usize * 4;
        let complex = cs16_le_bytes_to_complex(&self.byte_buf[..n_bytes]);
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
        "soapy"
    }
}

impl Drop for SoapySource {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_source_has_null_device_and_sane_defaults() {
        let src = SoapySource::new(None);
        assert!(src.dev.is_null());
        assert_eq!(src.frequency_hz(), 100_000_000);
        assert_eq!(src.sample_rate_hz(), 2_048_000);
        assert_eq!(src.kind(), "soapy");
    }

    #[test]
    fn read_iq_on_unstarted_source_returns_zero_without_touching_hardware() {
        let mut src = SoapySource::new(None);
        let mut buf = vec![Complex32::new(0.0, 0.0); 16];
        assert_eq!(src.read_iq(&mut buf).unwrap(), 0);
    }

    #[test]
    fn drop_on_never_started_source_does_not_panic() {
        let src = SoapySource::new(Some("driver=rtlsdr".into()));
        drop(src);
    }

    #[test]
    fn is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<SoapySource>();
    }

    #[test]
    fn cs16_conversion_is_symmetric_and_normalized() {
        let bytes = [0x00, 0x40, 0x00, 0xC0]; // i=0x4000=16384, q=0xC000=-16384 (LE i16)
        let c = cs16_le_bytes_to_complex(&bytes);
        assert_eq!(c.len(), 1);
        assert!((c[0].re - 0.5).abs() < 1e-6);
        assert!((c[0].im + 0.5).abs() < 1e-6);
    }
}
