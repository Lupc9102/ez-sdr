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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use num_complex::Complex32;

use super::IqSource;

const HACKRF_TRUE: c_int = 1;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HackRfGainPlan {
    amp_enabled: bool,
    lna_db: u32,
    vga_db: u32,
}

fn gain_plan(requested_db: f64) -> HackRfGainPlan {
    let requested = requested_db.clamp(0.0, 116.0).round() as u32;
    let amp_enabled = requested > 40;
    let remaining = requested.saturating_sub(if amp_enabled { 14 } else { 0 });
    let lna_db = ((remaining.min(40) / 8) * 8).min(40);
    let vga_db = (((remaining.saturating_sub(lna_db)).min(62) / 2) * 2).min(62);
    HackRfGainPlan {
        amp_enabled,
        lna_db,
        vga_db,
    }
}

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
    #[repr(C)]
    pub struct hackrf_transfer {
        pub device: *mut hackrf_device,
        pub buffer: *mut u8,
        pub buffer_length: c_int,
        pub valid_length: c_int,
        pub ctx: *mut c_void,
        pub tx_ctx: *mut c_void,
    }

    pub type HackrfTransfer = hackrf_transfer;

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

pub use ffi::HackrfTransfer;

struct RxCtx {
    tx: SyncSender<Vec<u8>>,
    active: AtomicBool,
}

// SAFETY: this is an `extern "C"` callback registered with `hackrf_start_rx`; the
// `hackrf_transfer` pointer and its `buffer`/`valid_length`/`ctx` fields are supplied by
// libhackrf. All pointers are validated for non-null before dereferencing, and the body
// is wrapped in `catch_unwind` to prevent panics across the FFI boundary.
unsafe extern "C" fn rx_callback(transfer: *mut ffi::hackrf_transfer) -> c_int {
    std::panic::catch_unwind(|| {
        if transfer.is_null() {
            return -1;
        }
        // SAFETY: `transfer` was checked non-null above.
        let (ctx_ptr, buf_ptr, valid_length) = unsafe {
            (
                (*transfer).ctx,
                (*transfer).buffer,
                (*transfer).valid_length,
            )
        };
        if ctx_ptr.is_null() || buf_ptr.is_null() {
            return -1;
        }
        if valid_length <= 0 {
            return 0;
        }
        // SAFETY: `ctx_ptr` points to an `RxCtx` managed via `Arc<RxCtx>` in `HackRfSource`.
        let ctx = unsafe { &*(ctx_ptr as *const RxCtx) };
        if !ctx.active.load(Ordering::Acquire) {
            return -1;
        }
        // SAFETY: `buf_ptr` is checked non-null and valid for `valid_length` bytes for the
        // duration of this callback, per the libhackrf API contract.
        let bytes = unsafe { std::slice::from_raw_parts(buf_ptr, valid_length as usize) };
        let mut data = bytes.to_vec();
        for b in &mut data {
            *b ^= 0x80;
        }
        let _ = ctx.tx.try_send(data);
        0
    })
    .unwrap_or(-1)
}

pub struct HackRfSource {
    dev: *mut ffi::hackrf_device,
    ctx: Option<Arc<RxCtx>>,
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
            ctx: None,
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

    fn apply_gain(&mut self) -> Result<()> {
        if self.dev.is_null() {
            return Ok(());
        }
        let plan = gain_plan(self.gain_db);
        // SAFETY: `self.dev` checked non-null above.
        Self::check("hackrf_set_amp_enable", unsafe {
            ffi::hackrf_set_amp_enable(self.dev, u8::from(plan.amp_enabled))
        })?;
        // SAFETY: `self.dev` checked non-null above.
        Self::check("hackrf_set_lna_gain", unsafe {
            ffi::hackrf_set_lna_gain(self.dev, plan.lna_db)
        })?;
        // SAFETY: `self.dev` checked non-null above.
        Self::check("hackrf_set_vga_gain", unsafe {
            ffi::hackrf_set_vga_gain(self.dev, plan.vga_db)
        })?;
        Ok(())
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
        if let Err(error) = self.apply_gain() {
            self.stop();
            return Err(error);
        }

        let (tx, rx) = sync_channel(64);
        self.rx = Some(rx);
        let ctx = Arc::new(RxCtx {
            tx,
            active: AtomicBool::new(true),
        });
        let raw_ctx = Arc::into_raw(Arc::clone(&ctx)) as *mut c_void;
        self.ctx = Some(ctx);

        // SAFETY: `dev` is valid and just configured above; `rx_callback` is a valid
        // `extern "C"` function pointer with the signature libhackrf expects; `raw_ctx`
        // is an Arc clone that stays alive until `stop` reclaims it after calling
        // `hackrf_stop_rx`.
        if let Err(e) = Self::check("hackrf_start_rx", unsafe {
            ffi::hackrf_start_rx(dev, Some(rx_callback), raw_ctx)
        }) {
            self.stop();
            return Err(e.into());
        }

        Ok(())
    }

    fn stop(&mut self) {
        if let Some(ctx) = self.ctx.as_ref() {
            ctx.active.store(false, Ordering::Release);
        }
        if !self.dev.is_null() {
            // SAFETY: `self.dev` is non-null and was returned by a successful
            // `hackrf_open`; this is libhackrf's documented shutdown sequence, stopping RX
            // before close/exit so no callback can fire after this point.
            unsafe { ffi::hackrf_stop_rx(self.dev) };
            unsafe { ffi::hackrf_close(self.dev) };
            unsafe { ffi::hackrf_exit() };
            self.dev = ptr::null_mut();
        }
        if let Some(ctx) = self.ctx.take() {
            // SAFETY: `raw` was created via `Arc::into_raw(Arc::clone(&ctx))` in `start`.
            // Reclaiming it with `Arc::from_raw` drops that strong reference. Because `ctx`
            // is still in scope, `RxCtx` cannot be deallocated until `ctx` is dropped,
            // preventing any UAF race with any in-flight USB transfer callback.
            let raw = Arc::as_ptr(&ctx) as *mut RxCtx;
            unsafe {
                drop(Arc::from_raw(raw));
            }
        }
        self.rx = None;
        self.byte_buf.clear();
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
        let previous = self.gain_db;
        self.gain_db = db.clamp(0.0, 116.0);
        if let Err(error) = self.apply_gain() {
            self.gain_db = previous;
            return Err(error);
        }
        Ok(())
    }

    fn read_iq(&mut self, buf: &mut [Complex32]) -> Result<usize> {
        let Some(rx) = self.rx.as_ref() else {
            return Ok(0);
        };
        let want_bytes = buf.len() * 2;

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

        let samples_available = self.byte_buf.len() / 2;
        let n = samples_available.min(buf.len());
        let bytes_consumed = n * 2;
        let complex = lrpt_decode::iq_bytes_to_complex(&self.byte_buf[..bytes_consumed]);
        buf[..n].copy_from_slice(&complex[..n]);
        self.byte_buf.drain(..bytes_consumed);
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

    #[test]
    fn total_gain_control_uses_all_hackrf_front_end_stages() {
        assert_eq!(
            gain_plan(20.0),
            HackRfGainPlan {
                amp_enabled: false,
                lna_db: 16,
                vga_db: 4,
            }
        );
        assert_eq!(
            gain_plan(54.0),
            HackRfGainPlan {
                amp_enabled: true,
                lna_db: 40,
                vga_db: 0,
            }
        );
        assert_eq!(gain_plan(116.0).vga_db, 62);
    }

    #[test]
    fn hackrf_transfer_abi_size_is_40_bytes() {
        #[cfg(target_pointer_width = "64")]
        assert_eq!(std::mem::size_of::<ffi::hackrf_transfer>(), 40);
        #[cfg(target_pointer_width = "32")]
        assert_eq!(std::mem::size_of::<ffi::hackrf_transfer>(), 24);
    }

    #[test]
    fn rx_callback_null_and_inactivity_safety() {
        // Null transfer pointer: returns -1
        assert_eq!(unsafe { rx_callback(ptr::null_mut()) }, -1);

        // Null buffer pointer: returns -1
        let (tx, rx) = sync_channel(16);
        let ctx = RxCtx {
            tx,
            active: AtomicBool::new(true),
        };
        let mut transfer = ffi::hackrf_transfer {
            device: ptr::null_mut(),
            buffer: ptr::null_mut(),
            buffer_length: 1024,
            valid_length: 1024,
            ctx: &ctx as *const RxCtx as *mut c_void,
            tx_ctx: ptr::null_mut(),
        };
        assert_eq!(unsafe { rx_callback(&mut transfer) }, -1);

        // Null ctx pointer: returns -1
        let mut dummy_buf = vec![0x80u8; 64];
        transfer.buffer = dummy_buf.as_mut_ptr();
        transfer.ctx = ptr::null_mut();
        assert_eq!(unsafe { rx_callback(&mut transfer) }, -1);

        // Valid transfer: returns 0 and transmits inverted uc8 data
        transfer.ctx = &ctx as *const RxCtx as *mut c_void;
        transfer.valid_length = 64;
        assert_eq!(unsafe { rx_callback(&mut transfer) }, 0);
        let received = rx.try_recv().expect("data sent to channel");
        assert_eq!(received.len(), 64);
        assert_eq!(received[0], 0x00); // 0x80 ^ 0x80 = 0

        // Inactive ctx (e.g. during stop teardown): returns -1
        ctx.active.store(false, Ordering::Release);
        assert_eq!(unsafe { rx_callback(&mut transfer) }, -1);
    }
}
