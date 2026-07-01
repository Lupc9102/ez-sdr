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
}

pub const HACKRF_TRUE: u8 = 1;

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
}

unsafe extern "C" fn rx_callback(transfer: *mut HackrfTransfer) -> c_int {
    if EXIT.load(Ordering::Relaxed) || (*transfer).valid_length <= 0 {
        return -1;
    }
    let ctx = &*((*transfer).ctx as *mut HackRfCtx);
    let slice = std::slice::from_raw_parts((*transfer).buffer, (*transfer).valid_length as usize);
    let mut data = slice.to_vec();
    for b in data.iter_mut() {
        *b ^= 0x80;
    }
    let _ = ctx.tx.try_send(data);
    0
}

pub struct HackRf {
    device: *mut HackrfDevice,
    config: HackRfConfig,
    ctx: *mut HackRfCtx,
    rx: Option<Receiver<Vec<u8>>>,
    freq: u64,
    sample_rate: u32,
    gain: f64,
}

unsafe impl Send for HackRf {}

impl HackRf {
    pub fn new(config: HackRfConfig) -> Self {
        let freq = config.freq;
        let sample_rate = config.rate;
        Self {
            device: ptr::null_mut(),
            config,
            ctx: ptr::null_mut(),
            rx: None,
            freq,
            sample_rate,
            gain: 0.0,
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
        if !self.device.is_null() {
            // Stop RX before close so the callback cannot fire into freed memory.
            unsafe { hackrf_stop_rx(self.device) };
            unsafe { hackrf_close(self.device) };
            unsafe { hackrf_exit() };
            self.device = ptr::null_mut();
        }
        if !self.ctx.is_null() {
            unsafe {
                let _ = Box::from_raw(self.ctx);
            }
            self.ctx = ptr::null_mut();
        }
    }
}

impl SdrSource for HackRf {
    fn start(&mut self) -> anyhow::Result<()> {
        if !self.device.is_null() {
            return Ok(());
        }

        let mut rate = self.config.rate as f64;
        let mut freq = self.config.freq as f64;
        if self.config.ppm != 0 {
            rate = rate * (1_000_000.0 - self.config.ppm as f64) / 1_000_000.0;
            freq = freq * (1_000_000.0 - self.config.ppm as f64) / 1_000_000.0;
        }

        Self::check(unsafe { hackrf_init() }, "hackrf_init")?;
        if let Err(e) = Self::check(unsafe { hackrf_open(&mut self.device) }, "hackrf_open") {
            unsafe { hackrf_exit() };
            return Err(e);
        }

        let dev = self.device;
        let res = Self::check(unsafe { hackrf_set_freq(dev, freq as u64) }, "hackrf_set_freq")
            .and_then(|_| Self::check(unsafe { hackrf_set_sample_rate(dev, rate) }, "hackrf_set_sample_rate"))
            .and_then(|_| Self::check(unsafe { hackrf_set_amp_enable(dev, self.config.enable_amp as u8) }, "hackrf_set_amp_enable"))
            .and_then(|_| Self::check(unsafe { hackrf_set_lna_gain(dev, self.config.lna_gain) }, "hackrf_set_lna_gain"))
            .and_then(|_| Self::check(unsafe { hackrf_set_vga_gain(dev, self.config.vga_gain) }, "hackrf_set_vga_gain"))
            .and_then(|_| Self::check(unsafe { hackrf_set_antenna_enable(dev, self.config.enable_ant_pwr as u8) }, "hackrf_set_antenna_enable"));

        if let Err(e) = res {
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
        if !self.device.is_null() {
            unsafe { hackrf_stop_rx(self.device) };
            unsafe { hackrf_close(self.device) };
            unsafe { hackrf_exit() };
            self.device = ptr::null_mut();
        }
        if !self.ctx.is_null() {
            unsafe {
                let _ = Box::from_raw(self.ctx);
            }
            self.ctx = ptr::null_mut();
        }
        self.rx = None;
    }

    fn set_frequency(&mut self, freq: u64) -> anyhow::Result<()> {
        self.freq = freq;
        if !self.device.is_null() {
            Self::check(unsafe { hackrf_set_freq(self.device, freq) }, "hackrf_set_freq")?;
        }
        Ok(())
    }

    fn set_sample_rate(&mut self, rate: u32) -> anyhow::Result<()> {
        self.sample_rate = rate;
        if !self.device.is_null() {
            Self::check(unsafe { hackrf_set_sample_rate(self.device, rate as f64) }, "hackrf_set_sample_rate")?;
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
            let ctx = Box::into_raw(Box::new(HackRfCtx { tx }));
            self.ctx = ctx;
            Self::check(
                unsafe { hackrf_start_rx(self.device, Some(rx_callback), ctx as *mut c_void) },
                "hackrf_start_rx",
            )?;
        }

        let rx = self.rx.as_ref().expect("rx channel always set above");
        let need_bytes = buf.len() * 2;
        let mut raw: Vec<u8> = Vec::with_capacity(need_bytes);

        while raw.len() < need_bytes {
            if EXIT.load(Ordering::Relaxed) {
                break;
            }
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(chunk) => raw.extend_from_slice(&chunk),
                Err(RecvTimeoutError::Timeout) => {
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
        }
        Ok(samples)
    }
}
