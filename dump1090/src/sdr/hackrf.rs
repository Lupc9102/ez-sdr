#![deny(unsafe_op_in_unsafe_fn)]

//! HackRF source - translated from sdr_hackrf.c

use std::ffi::{c_int, c_void};
use std::ptr;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError};
use std::time::Duration;

use crate::convert;
use crate::sdr::SdrSource;
use crate::util::EXIT;

#[repr(C)]
pub struct HackrfDevice {
    _private: [u8; 0],
}

#[repr(C)]
pub struct HackrfTransfer {
    pub device: *mut HackrfDevice,
    pub buffer: *mut u8,
    pub buffer_length: c_int,
    pub valid_length: c_int,
    pub ctx: *mut c_void,
    pub tx_ctx: *mut c_void,
}

pub const HACKRF_TRUE: u8 = 1;

#[link(name = "hackrf")]
extern "C" {
    fn hackrf_init() -> c_int;
    fn hackrf_open(device: *mut *mut HackrfDevice) -> c_int;
    fn hackrf_close(device: *mut HackrfDevice) -> c_int;
    fn hackrf_exit() -> c_int;
    fn hackrf_stop_rx(device: *mut HackrfDevice) -> c_int;
    fn hackrf_set_freq(device: *mut HackrfDevice, freq_hz: u64) -> c_int;
    fn hackrf_set_sample_rate(device: *mut HackrfDevice, freq_hz: f64) -> c_int;
    fn hackrf_set_amp_enable(device: *mut HackrfDevice, value: u8) -> c_int;
    fn hackrf_set_lna_gain(device: *mut HackrfDevice, value: u32) -> c_int;
    fn hackrf_set_vga_gain(device: *mut HackrfDevice, value: u32) -> c_int;
    fn hackrf_set_antenna_enable(device: *mut HackrfDevice, value: u8) -> c_int;
    fn hackrf_start_rx(
        device: *mut HackrfDevice,
        callback: Option<unsafe extern "C" fn(*mut HackrfTransfer) -> c_int>,
        ctx: *mut c_void,
    ) -> c_int;
    fn hackrf_is_streaming(device: *mut HackrfDevice) -> u8;
}

#[derive(Clone, Debug)]
pub struct HackRfConfig {
    pub freq: u64,
    pub enable_amp: bool,
    pub enable_ant_pwr: bool,
    pub lna_gain: u32,
    pub vga_gain: u32,
    pub rate: u32,
    pub ppm: i32,
}

impl Default for HackRfConfig {
    fn default() -> Self {
        Self {
            freq: 1_090_000_000,
            enable_amp: false,
            enable_ant_pwr: false,
            lna_gain: 32,
            vga_gain: 50,
            rate: 2_400_000,
            ppm: 0,
        }
    }
}

struct HackRfCtx {
    tx: std::sync::mpsc::SyncSender<Vec<u8>>,
    active: std::sync::atomic::AtomicBool,
}

// SAFETY: This is an `extern "C"` callback registered with `hackrf_start_rx`.
// The `HackrfTransfer` pointer and its fields (`buffer`, `valid_length`, `ctx`)
// are provided by the HackRF library.
unsafe extern "C" fn rx_callback(transfer: *mut HackrfTransfer) -> c_int {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if transfer.is_null() {
            return -1;
        }
        let t = unsafe { &*transfer };
        if EXIT.load(Ordering::Relaxed)
            || t.valid_length <= 0
            || t.ctx.is_null()
            || t.buffer.is_null()
        {
            return -1;
        }

        // SAFETY: `t.ctx` was created via `Arc::into_raw` and is non-null.
        // We increment strong count for the duration of this callback invocation
        // so that in-flight callbacks remain safe even if teardown starts concurrently.
        let raw = t.ctx as *const HackRfCtx;
        unsafe { std::sync::Arc::increment_strong_count(raw) };
        let ctx = unsafe { std::sync::Arc::from_raw(raw) };

        if !ctx.active.load(Ordering::Relaxed) {
            return -1;
        }

        let (buf_ptr, buf_len) = (t.buffer, t.valid_length as usize);
        let slice = unsafe { std::slice::from_raw_parts(buf_ptr, buf_len) };
        let mut data = slice.to_vec();
        for b in data.iter_mut() {
            *b ^= 0x80;
        }
        let _ = ctx.tx.try_send(data);
        0
    }));

    result.unwrap_or(-1)
}

pub struct HackRf {
    device: *mut HackrfDevice,
    config: HackRfConfig,
    ctx: Option<std::sync::Arc<HackRfCtx>>,
    raw_ctx: Option<*mut c_void>,
    rx: Option<Receiver<Vec<u8>>>,
    freq: u64,
    sample_rate: u32,
    gain: f64,
    surplus: Vec<u8>,
}

// SAFETY: `HackRf` contains raw device/ctx pointers that are accessed only
// through FFI calls under `&mut self` (via `SdrSource`), so no concurrent
// mutable access occurs. The HackRF library handles its own synchronisation.
unsafe impl Send for HackRf {}

impl HackRf {
    #[must_use]
    pub fn new(config: HackRfConfig) -> Self {
        let freq = config.freq;
        let sample_rate = config.rate;
        Self {
            device: ptr::null_mut(),
            config,
            ctx: None,
            raw_ctx: None,
            rx: None,
            freq,
            sample_rate,
            gain: 0.0,
            surplus: Vec::new(),
        }
    }

    fn check(code: c_int, msg: &str) -> anyhow::Result<()> {
        if code != 0 {
            Err(anyhow::anyhow!("HackRF: {} failed with code {}", msg, code))
        } else {
            Ok(())
        }
    }
}

impl Drop for HackRf {
    fn drop(&mut self) {
        self.stop();
    }
}

impl SdrSource for HackRf {
    fn is_live(&self) -> bool {
        true
    }

    fn start(&mut self) -> anyhow::Result<()> {
        if !self.device.is_null() {
            return Ok(());
        }

        let rate = self.config.rate as f64;
        let mut freq = self.config.freq as f64;
        if self.config.ppm != 0 {
            freq = freq * (1_000_000.0 - self.config.ppm as f64) / 1_000_000.0;
        }

        // SAFETY: `hackrf_init` has no preconditions per the libhackrf API.
        Self::check(unsafe { hackrf_init() }, "hackrf_init")?;
        // SAFETY: `&mut self.device` is a valid out-parameter for the device
        // handle; the pointer is a field on `self` that lives for the duration.
        if let Err(e) = Self::check(unsafe { hackrf_open(&mut self.device) }, "hackrf_open") {
            // SAFETY: `hackrf_exit` cleans up after a failed init/open.
            unsafe { hackrf_exit() };
            return Err(e);
        }

        let dev = self.device;
        // SAFETY: `dev` is the handle returned by `hackrf_open` above. Each
        // FFI call requires a valid open device handle per the libhackrf API.
        let res = Self::check(
            unsafe { hackrf_set_freq(dev, freq as u64) },
            "hackrf_set_freq",
        )
        .and_then(|_| {
            Self::check(
                unsafe { hackrf_set_sample_rate(dev, rate) },
                "hackrf_set_sample_rate",
            )
        })
        .and_then(|_| {
            Self::check(
                unsafe { hackrf_set_amp_enable(dev, self.config.enable_amp as u8) },
                "hackrf_set_amp_enable",
            )
        })
        .and_then(|_| {
            Self::check(
                unsafe { hackrf_set_lna_gain(dev, self.config.lna_gain) },
                "hackrf_set_lna_gain",
            )
        })
        .and_then(|_| {
            Self::check(
                unsafe { hackrf_set_vga_gain(dev, self.config.vga_gain) },
                "hackrf_set_vga_gain",
            )
        })
        .and_then(|_| {
            Self::check(
                unsafe { hackrf_set_antenna_enable(dev, self.config.enable_ant_pwr as u8) },
                "hackrf_set_antenna_enable",
            )
        });

        if let Err(e) = res {
            // SAFETY: `dev` is still valid; closing and exiting to clean up.
            unsafe { hackrf_close(dev) };
            unsafe { hackrf_exit() };
            self.device = ptr::null_mut();
            return Err(e);
        }

        self.freq = freq as u64;
        self.sample_rate = rate as u32;
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(ref ctx) = self.ctx {
            ctx.active.store(false, Ordering::SeqCst);
        }
        if !self.device.is_null() {
            // SAFETY: `self.device` is checked non-null and was returned by
            // `hackrf_open`. Standard shutdown sequence per libhackrf API.
            unsafe { hackrf_stop_rx(self.device) };
            let mut retries = 0;
            while unsafe { hackrf_is_streaming(self.device) } == HACKRF_TRUE && retries < 100 {
                std::thread::sleep(Duration::from_millis(5));
                retries += 1;
            }
            unsafe { hackrf_close(self.device) };
            unsafe { hackrf_exit() };
            self.device = ptr::null_mut();
        }
        if let Some(raw_ctx) = self.raw_ctx.take() {
            // SAFETY: `raw_ctx` was allocated via Arc::into_raw in read_samples.
            unsafe {
                let _ = std::sync::Arc::from_raw(raw_ctx as *const HackRfCtx);
            }
        }
        self.ctx = None;
        self.rx = None;
    }

    fn set_frequency(&mut self, freq: u64) -> anyhow::Result<()> {
        self.freq = freq;
        if !self.device.is_null() {
            // SAFETY: `self.device` is checked non-null above.
            Self::check(
                unsafe { hackrf_set_freq(self.device, freq) },
                "hackrf_set_freq",
            )?;
        }
        Ok(())
    }

    fn set_sample_rate(&mut self, rate: u32) -> anyhow::Result<()> {
        self.sample_rate = rate;
        if !self.device.is_null() {
            // SAFETY: `self.device` is checked non-null above.
            Self::check(
                unsafe { hackrf_set_sample_rate(self.device, rate as f64) },
                "hackrf_set_sample_rate",
            )?;
        }
        Ok(())
    }

    fn set_gain(&mut self, gain: f64) -> anyhow::Result<()> {
        self.gain = gain;
        Ok(())
    }

    fn read_samples(&mut self, buf: &mut [u16]) -> anyhow::Result<usize> {
        if self.device.is_null() {
            return Ok(0);
        }

        if self.rx.is_none() {
            let (tx, rx) = sync_channel(16);
            self.rx = Some(rx);
            let ctx = std::sync::Arc::new(HackRfCtx {
                tx,
                active: std::sync::atomic::AtomicBool::new(true),
            });
            let raw_ctx = std::sync::Arc::into_raw(std::sync::Arc::clone(&ctx)) as *mut c_void;
            self.ctx = Some(ctx);
            self.raw_ctx = Some(raw_ctx);
            // SAFETY: `self.device` is non-null (valid open handle).
            // `rx_callback` is a valid `extern "C"` function pointer.
            // `raw_ctx` is an `Arc::into_raw` allocation that stays alive while
            // the stream is active.
            Self::check(
                unsafe { hackrf_start_rx(self.device, Some(rx_callback), raw_ctx) },
                "hackrf_start_rx",
            )?;
        }

        let rx = self.rx.as_ref().expect("rx channel always set above");
        let need_bytes = buf.len() * 2;
        let mut raw: Vec<u8> = Vec::with_capacity(need_bytes);

        if !self.surplus.is_empty() {
            raw.append(&mut self.surplus);
        }

        while raw.len() < need_bytes {
            if EXIT.load(Ordering::Relaxed) {
                break;
            }
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(chunk) => raw.extend_from_slice(&chunk),
                Err(RecvTimeoutError::Timeout) => {
                    // SAFETY: `self.device` is non-null (checked earlier).
                    if unsafe { hackrf_is_streaming(self.device) } != HACKRF_TRUE {
                        break;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }

        let samples = (raw.len() / 2).min(buf.len());
        if samples > 0 {
            convert::convert_uc8_to_mag(&raw[..samples * 2], &mut buf[..samples]);
            if raw.len() > samples * 2 {
                self.surplus.extend_from_slice(&raw[samples * 2..]);
            }
        }
        Ok(samples)
    }
}
