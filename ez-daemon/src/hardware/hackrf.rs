#![deny(unsafe_op_in_unsafe_fn)]
#![allow(unsafe_code)]

//! HackRF [`IqSource`] backend. Mirrors `dump1090::sdr::hackrf`'s proven FFI bindings and
//! async callback-driven RX model (HackRF has no synchronous read call — samples arrive on
//! a background thread inside libhackrf via `hackrf_start_rx`'s callback, bridged here into
//! `read_iq`'s pull model with a bounded channel), but yields phase-preserving `Complex32`
//! samples instead of discarding phase into a magnitude table.
//!
//! HackRF's native wire format is signed 8-bit I/Q; XORing each byte with `0x80` converts it
//! to the same unsigned, 127.5-centered convention `lrpt_decode::iq_bytes_to_complex` already
//! expects (the RTL-SDR "Uc8" convention), so both hardware backends share one converter.

use std::ffi::{c_int, c_void};
use std::ptr;
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::time::Duration;

use anyhow::Result;
use num_complex::Complex32;

use super::IqSource;

const HACKRF_TRUE: c_int = 1;
const DEFAULT_LNA_GAIN: u32 = 16;

#[derive(Debug, thiserror::Error)]
pub enum HackRfError {
    #[error("{0} failed with code {1}")]
    CallFailed(&'static str, i32),
    #[error("no HackRF device available")]
    OpenFailed,
}

#[allow(dead_code)]
mod ffi {
    use std::ffi::{c_int, c_void};

    #[repr(C)]
    pub struct hackrf_device {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct hackrf_transfer {
        pub device: *mut hackrf_device,
        pub buffer: *mut u8,
        pub buffer_length: c_int,
        pub valid_length: c_int,
        pub ctx: *mut c_void,
    }

    pub type SampleBlockCb = Option<unsafe extern "C" fn(*mut hackrf_transfer) -> c_int>;

    #[link(name = "hackrf")]
    extern "C" {
        pub fn hackrf_init() -> c_int;
        pub fn hackrf_exit() -> c_int;
        pub fn hackrf_open(device: *mut *mut hackrf_device) -> c_int;
        pub fn hackrf_close(device: *mut hackrf_device) -> c_int;
        pub fn hackrf_set_freq(device: *mut hackrf_device, freq_hz: u64) -> c_int;
        pub fn hackrf_set_sample_rate(device: *mut hackrf_device, freq_hz: f64) -> c_int;
        pub fn hackrf_set_lna_gain(device: *mut hackrf_device, value: u32) -> c_int;
        pub fn hackrf_set_vga_gain(device: *mut hackrf_device, value: u32) -> c_int;
        pub fn hackrf_set_amp_enable(device: *mut hackrf_device, value: u8) -> c_int;
        pub fn hackrf_start_rx(
            device: *mut hackrf_device,
            callback: SampleBlockCb,
            rx_ctx: *mut c_void,
        ) -> c_int;
        pub fn hackrf_stop_rx(device: *mut hackrf_device) -> c_int;
        pub fn hackrf_is_streaming(device: *mut hackrf_device) -> c_int;
    }
}

struct RxCtx {
    tx: SyncSender<Vec<u8>>,
}

// SAFETY: this is an `extern "C"` callback registered with `hackrf_start_rx`; the
// `hackrf_transfer` pointer and its `buffer`/`valid_length`/`ctx` fields are supplied by
// libhackrf and valid for the duration of the call, per the library's API contract.
unsafe extern "C" fn rx_callback(transfer: *mut ffi::hackrf_transfer) -> c_int {
    // SAFETY: `transfer` is non-null and valid per the libhackrf callback contract.
    let valid_length = unsafe { (*transfer).valid_length };
    if valid_length <= 0 {
        return 0;
    }
    // SAFETY: `ctx` was set to a `Box::into_raw(Box::new(RxCtx))` pointer in `start` and
    // stays alive until `stop` runs `hackrf_stop_rx` first, so no callback can still be in
    // flight when the `Box` is reclaimed.
    let ctx = unsafe { &*((*transfer).ctx as *const RxCtx) };
    // SAFETY: `buffer` is valid for `valid_length` bytes for the duration of this callback,
    // per the libhackrf API contract.
    let bytes = unsafe { std::slice::from_raw_parts((*transfer).buffer, valid_length as usize) };
    let mut data = bytes.to_vec();
    for b in &mut data {
        *b ^= 0x80;
    }
    let _ = ctx.tx.try_send(data);
    0
}

pub struct HackRfSource {
    dev: *mut ffi::hackrf_device,
    ctx: *mut RxCtx,
    rx: Option<Receiver<Vec<u8>>>,
    frequency_hz: u64,
    sample_rate_hz: u32,
    gain_db: f64,
    byte_buf: Vec<u8>,
}

// SAFETY: `HackRfSource` only touches its raw pointers through FFI calls made under
// `&mut self` (the `IqSource` trait is exclusively `&mut self`), so no concurrent access
// from safe code can occur. The one genuinely concurrent access — `rx_callback` running on
// libhackrf's internal streaming thread — only ever touches the channel `Sender`, which is
// itself `Send + Sync` and safe to share this way.
unsafe impl Send for HackRfSource {}

impl HackRfSource {
    #[must_use]
    pub fn new() -> Self {
        Self {
            dev: ptr::null_mut(),
            ctx: ptr::null_mut(),
            rx: None,
            frequency_hz: 1_090_000_000,
            sample_rate_hz: 2_400_000,
            gain_db: 20.0,
            byte_buf: Vec::new(),
        }
    }

    fn check(op: &'static str, code: c_int) -> Result<(), HackRfError> {
        if code == 0 {
            Ok(())
        } else {
            Err(HackRfError::CallFailed(op, code))
        }
    }

    fn apply_gain(&mut self) {
        if self.dev.is_null() {
            return;
        }
        let vga = (((self.gain_db.clamp(0.0, 62.0) / 2.0).round() as u32) * 2).min(62);
        // SAFETY: `self.dev` checked non-null above.
        unsafe { ffi::hackrf_set_lna_gain(self.dev, DEFAULT_LNA_GAIN) };
        // SAFETY: `self.dev` checked non-null above.
        unsafe { ffi::hackrf_set_vga_gain(self.dev, vga) };
    }
}

impl Default for HackRfSource {
    fn default() -> Self {
        Self::new()
    }
}

impl IqSource for HackRfSource {
    fn start(&mut self) -> Result<()> {
        if !self.dev.is_null() {
            return Ok(());
        }
        // SAFETY: no preconditions beyond libhackrf being linked.
        Self::check("hackrf_init", unsafe { ffi::hackrf_init() })?;

        let mut dev = ptr::null_mut();
        // SAFETY: `&mut dev` is a valid, stack-local out-parameter for the FFI to write an
        // opened device handle into.
        let ret = unsafe { ffi::hackrf_open(&mut dev) };
        if ret != 0 || dev.is_null() {
            // SAFETY: matches the successful `hackrf_init` above.
            unsafe { ffi::hackrf_exit() };
            return Err(HackRfError::OpenFailed.into());
        }
        self.dev = dev;

        // SAFETY: `dev` was just successfully opened above.
        if let Err(e) = Self::check("hackrf_set_freq", unsafe {
            ffi::hackrf_set_freq(dev, self.frequency_hz)
        }) {
            self.stop();
            return Err(e.into());
        }
        // SAFETY: `dev` is a valid, just-opened device handle.
        if let Err(e) = Self::check("hackrf_set_sample_rate", unsafe {
            ffi::hackrf_set_sample_rate(dev, self.sample_rate_hz as f64)
        }) {
            self.stop();
            return Err(e.into());
        }
        // SAFETY: `dev` is a valid, just-opened device handle.
        unsafe { ffi::hackrf_set_amp_enable(dev, 0) };
        self.apply_gain();

        let (tx, rx) = sync_channel(64);
        self.rx = Some(rx);
        let ctx = Box::into_raw(Box::new(RxCtx { tx }));
        self.ctx = ctx;

        // SAFETY: `dev` is valid and just configured above; `rx_callback` is a valid
        // `extern "C"` function pointer with the signature libhackrf expects; `ctx` is a
        // live `Box::into_raw` allocation that outlives the stream (reclaimed in `stop`,
        // which always calls `hackrf_stop_rx` before freeing it).
        if let Err(e) = Self::check("hackrf_start_rx", unsafe {
            ffi::hackrf_start_rx(dev, Some(rx_callback), ctx as *mut c_void)
        }) {
            self.stop();
            return Err(e.into());
        }

        Ok(())
    }

    fn stop(&mut self) {
        if !self.dev.is_null() {
            // SAFETY: `self.dev` is non-null and was returned by a successful
            // `hackrf_open`; this is libhackrf's documented shutdown sequence, stopping RX
            // before close/exit so no callback can fire after this point.
            unsafe { ffi::hackrf_stop_rx(self.dev) };
            unsafe { ffi::hackrf_close(self.dev) };
            unsafe { ffi::hackrf_exit() };
            self.dev = ptr::null_mut();
        }
        if !self.ctx.is_null() {
            // SAFETY: `self.ctx` was allocated with `Box::into_raw` in `start`; by this
            // point `hackrf_stop_rx` above has returned, so libhackrf's callback thread can
            // no longer invoke `rx_callback`, making it safe to reclaim.
            unsafe {
                drop(Box::from_raw(self.ctx));
            }
            self.ctx = ptr::null_mut();
        }
        self.rx = None;
    }

    fn set_frequency(&mut self, hz: u64) -> Result<()> {
        self.frequency_hz = hz;
        if !self.dev.is_null() {
            // SAFETY: `self.dev` checked non-null above.
            Self::check("hackrf_set_freq", unsafe {
                ffi::hackrf_set_freq(self.dev, hz)
            })?;
        }
        Ok(())
    }

    fn set_sample_rate(&mut self, hz: u32) -> Result<()> {
        self.sample_rate_hz = hz;
        if !self.dev.is_null() {
            // SAFETY: `self.dev` checked non-null above.
            Self::check("hackrf_set_sample_rate", unsafe {
                ffi::hackrf_set_sample_rate(self.dev, hz as f64)
            })?;
        }
        Ok(())
    }

    fn set_gain(&mut self, db: f64) -> Result<()> {
        self.gain_db = db;
        self.apply_gain();
        Ok(())
    }

    fn read_iq(&mut self, buf: &mut [Complex32]) -> Result<usize> {
        let Some(rx) = self.rx.as_ref() else {
            return Ok(0);
        };
        let want_bytes = buf.len() * 2;
        self.byte_buf.clear();

        while self.byte_buf.len() < want_bytes {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(chunk) => self.byte_buf.extend_from_slice(&chunk),
                Err(RecvTimeoutError::Timeout) => {
                    // SAFETY: `self.dev` is non-null whenever `self.rx` is `Some` — both
                    // are only ever set together in `start` and cleared together in `stop`.
                    if unsafe { ffi::hackrf_is_streaming(self.dev) } != HACKRF_TRUE {
                        break;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        if self.byte_buf.is_empty() {
            return Ok(0);
        }

        let complex = lrpt_decode::iq_bytes_to_complex(&self.byte_buf);
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
        "hackrf"
    }
}

impl Drop for HackRfSource {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_source_has_null_device_and_sane_defaults() {
        let src = HackRfSource::new();
        assert!(src.dev.is_null());
        assert_eq!(src.frequency_hz(), 1_090_000_000);
        assert_eq!(src.sample_rate_hz(), 2_400_000);
        assert_eq!(src.kind(), "hackrf");
    }

    #[test]
    fn read_iq_on_unstarted_source_returns_zero_without_touching_hardware() {
        let mut src = HackRfSource::new();
        let mut buf = vec![Complex32::new(0.0, 0.0); 16];
        assert_eq!(src.read_iq(&mut buf).unwrap(), 0);
    }

    #[test]
    fn drop_on_never_started_source_does_not_panic() {
        let src = HackRfSource::new();
        drop(src);
    }

    #[test]
    fn is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<HackRfSource>();
    }
}
