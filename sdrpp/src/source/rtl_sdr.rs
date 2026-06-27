//! RTL-SDR source - translated from legacy source_modules/rtl_sdr_source/src/main.cpp
#![cfg(feature = "rtlsdr")]

use crossbeam_channel::{bounded, Receiver, Sender};
use libc::{c_int, c_uchar, c_void};
use num_complex::Complex;
use std::ffi::CStr;
use std::slice;
use std::thread::{self, JoinHandle};
use thiserror::Error;

// rtlsdr_sys 1.1.2 predates the bias-tee API, so we declare it manually.
extern "C" {
    fn rtlsdr_set_bias_tee(dev: rtlsdr_sys::rtlsdr_dev_t, on: c_int) -> c_int;
}

/// Supported sample rates (Hz), matching the original C++ module.
pub const SAMPLE_RATES: [u32; 11] = [
    250_000,
    1_024_000,
    1_536_000,
    1_792_000,
    1_920_000,
    2_048_000,
    2_160_000,
    2_400_000,
    2_560_000,
    2_880_000,
    3_200_000,
];

/// Human-readable labels for the sample rates.
pub const SAMPLE_RATE_LABELS: [&str; 11] = [
    "250KHz",
    "1.024MHz",
    "1.536MHz",
    "1.792MHz",
    "1.92MHz",
    "2.048MHz",
    "2.16MHz",
    "2.4MHz",
    "2.56MHz",
    "2.88MHz",
    "3.2MHz",
];

#[derive(Error, Debug)]
pub enum RtlSdrError {
    #[error("Device error: {0}")]
    Device(String),
    #[error("Not open")]
    NotOpen,
    #[error("Already running")]
    AlreadyRunning,
    #[error("Not running")]
    NotRunning,
}

/// Generic source trait implemented by hardware backends.
pub trait Source {
    type Error: std::error::Error;
    fn open(&mut self, device_index: i32) -> Result<(), Self::Error>;
    fn set_freq(&mut self, freq_hz: u32) -> Result<(), Self::Error>;
    fn set_gain(&mut self, gain_db_tenths: i32) -> Result<(), Self::Error>;
    fn start(&mut self) -> Result<(), Self::Error>;
    fn stop(&mut self) -> Result<(), Self::Error>;
    fn read_samples(&mut self, buf: &mut [Complex<f32>]) -> Result<usize, Self::Error>;
}

/// Context object passed through the C async callback.
struct AsyncContext {
    tx: Sender<Vec<Complex<f32>>>,
}

/// C callback required by `rtlsdr_read_async`.
/// Converts raw unsigned IQ bytes to normalized `Complex<f32>` and forwards
/// them over a bounded channel.
extern "C" fn async_callback(buf: *mut c_uchar, len: u32, ctx: *mut c_void) {
    if ctx.is_null() || buf.is_null() {
        return;
    }
    let context = unsafe { &*(ctx as *const AsyncContext) };
    let raw = unsafe { slice::from_raw_parts(buf, len as usize) };
    let samp_count = raw.len() / 2;
    let mut samples = Vec::with_capacity(samp_count);
    for i in 0..samp_count {
        let re = (raw[i * 2] as f32 - 127.4) / 128.0;
        let im = (raw[i * 2 + 1] as f32 - 127.4) / 128.0;
        samples.push(Complex::new(re, im));
    }
    // Best-effort delivery; drop the buffer if the consumer can't keep up.
    let _ = context.tx.try_send(samples);
}

/// RTL-SDR source implementing the SDR++ source module logic in idiomatic Rust.
pub struct RtlSdrSource {
    dev: Option<rtlsdr_sys::rtlsdr_dev_t>,
    device_index: i32,
    sample_rate: u32,
    freq: u32,
    ppm: i32,
    gain: i32,
    gain_list: Vec<i32>,
    bias_t: bool,
    rtl_agc: bool,
    tuner_agc: bool,
    offset_tuning: bool,
    direct_sampling: i32,
    running: bool,
    worker: Option<JoinHandle<()>>,
    rx: Option<Receiver<Vec<Complex<f32>>>>,
    overflow: Vec<Complex<f32>>,
}

// SAFETY: librtlsdr device handles are not `Send` by default because they are
// raw pointers, but they may be moved to another thread as long as access is
// serialised. Our state machine guarantees only one thread touches the device
// at any time (either the worker during `read_async` or the main thread
// during `stop`).
unsafe impl Send for RtlSdrSource {}

impl RtlSdrSource {
    pub fn new() -> Self {
        RtlSdrSource {
            dev: None,
            device_index: 0,
            sample_rate: 2_400_000,
            freq: 100_000_000,
            ppm: 0,
            gain: 0,
            gain_list: Vec::new(),
            bias_t: false,
            rtl_agc: false,
            tuner_agc: false,
            offset_tuning: false,
            direct_sampling: 0,
            running: false,
            worker: None,
            rx: None,
            overflow: Vec::new(),
        }
    }

    /// Enumerate RTL-SDR devices by name.
    pub fn list_devices() -> Vec<String> {
        let mut names = Vec::new();
        unsafe {
            let count = rtlsdr_sys::rtlsdr_get_device_count();
            for i in 0..count {
                let name_ptr = rtlsdr_sys::rtlsdr_get_device_name(i);
                if !name_ptr.is_null() {
                    let name = CStr::from_ptr(name_ptr)
                        .to_string_lossy()
                        .into_owned();
                    names.push(name);
                }
            }
        }
        names
    }

    /// Set the sample rate in Hz.
    ///
    /// If the device is already open the change is applied immediately.
    pub fn set_sample_rate(&mut self, rate: u32) -> Result<(), RtlSdrError> {
        self.sample_rate = rate;
        if let Some(dev) = self.dev {
            unsafe {
                let r = rtlsdr_sys::rtlsdr_set_sample_rate(dev, rate);
                if r < 0 {
                    return Err(RtlSdrError::Device(format!(
                        "rtlsdr_set_sample_rate returned {}",
                        r
                    )));
                }
            }
        }
        Ok(())
    }

    /// Set the frequency correction in PPM.
    pub fn set_ppm(&mut self, ppm: i32) -> Result<(), RtlSdrError> {
        self.ppm = ppm;
        if let Some(dev) = self.dev {
            unsafe {
                let r = rtlsdr_sys::rtlsdr_set_freq_correction(dev, ppm);
                if r < 0 {
                    return Err(RtlSdrError::Device(format!(
                        "rtlsdr_set_freq_correction returned {}",
                        r
                    )));
                }
            }
        }
        Ok(())
    }

    /// Enable or disable the bias tee.
    pub fn set_bias_t(&mut self, enable: bool) -> Result<(), RtlSdrError> {
        self.bias_t = enable;
        if let Some(dev) = self.dev {
            unsafe {
                let r = rtlsdr_set_bias_tee(dev, if enable { 1 } else { 0 });
                if r < 0 {
                    return Err(RtlSdrError::Device(format!(
                        "rtlsdr_set_bias_tee returned {}",
                        r
                    )));
                }
            }
        }
        Ok(())
    }

    /// Enable or disable the RTL2832 internal digital AGC.
    pub fn set_rtl_agc(&mut self, enable: bool) -> Result<(), RtlSdrError> {
        self.rtl_agc = enable;
        if let Some(dev) = self.dev {
            unsafe {
                let r = rtlsdr_sys::rtlsdr_set_agc_mode(dev, if enable { 1 } else { 0 });
                if r < 0 {
                    return Err(RtlSdrError::Device(format!(
                        "rtlsdr_set_agc_mode returned {}",
                        r
                    )));
                }
            }
        }
        Ok(())
    }

    /// Enable or disable tuner AGC.
    ///
    /// When disabled the currently selected manual gain is applied.
    pub fn set_tuner_agc(&mut self, enable: bool) -> Result<(), RtlSdrError> {
        self.tuner_agc = enable;
        if let Some(dev) = self.dev {
            unsafe {
                let r =
                    rtlsdr_sys::rtlsdr_set_tuner_gain_mode(dev, if enable { 0 } else { 1 });
                if r < 0 {
                    return Err(RtlSdrError::Device(format!(
                        "rtlsdr_set_tuner_gain_mode returned {}",
                        r
                    )));
                }
                if !enable {
                    let r2 = rtlsdr_sys::rtlsdr_set_tuner_gain(dev, self.gain);
                    if r2 < 0 {
                        return Err(RtlSdrError::Device(format!(
                            "rtlsdr_set_tuner_gain returned {}",
                            r2
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    /// Enable or disable offset tuning.
    pub fn set_offset_tuning(&mut self, enable: bool) -> Result<(), RtlSdrError> {
        self.offset_tuning = enable;
        if let Some(dev) = self.dev {
            unsafe {
                let r =
                    rtlsdr_sys::rtlsdr_set_offset_tuning(dev, if enable { 1 } else { 0 });
                if r < 0 {
                    return Err(RtlSdrError::Device(format!(
                        "rtlsdr_set_offset_tuning returned {}",
                        r
                    )));
                }
            }
        }
        Ok(())
    }

    /// Set direct sampling mode.
    ///
    /// * `0` – disabled  
    /// * `1` – I branch  
    /// * `2` – Q branch  
    pub fn set_direct_sampling(&mut self, mode: i32) -> Result<(), RtlSdrError> {
        self.direct_sampling = mode.clamp(0, 2);
        if let Some(dev) = self.dev {
            unsafe {
                let r = rtlsdr_sys::rtlsdr_set_direct_sampling(dev, self.direct_sampling);
                if r < 0 {
                    return Err(RtlSdrError::Device(format!(
                        "rtlsdr_set_direct_sampling returned {}",
                        r
                    )));
                }
                // Re-apply gain settings after leaving direct sampling
                // (work-around for a librtlsdr bug, matching original C++).
                if self.direct_sampling == 0 {
                    let _ = rtlsdr_sys::rtlsdr_set_agc_mode(
                        dev,
                        if self.rtl_agc { 1 } else { 0 },
                    );
                    let _ = rtlsdr_sys::rtlsdr_set_tuner_gain_mode(
                        dev,
                        if self.tuner_agc { 0 } else { 1 },
                    );
                    if !self.tuner_agc {
                        let _ = rtlsdr_sys::rtlsdr_set_tuner_gain(dev, self.gain);
                    }
                }
            }
        }
        Ok(())
    }

    /// Return the list of valid tuner gains (tenths of a dB).
    pub fn gain_list(&self) -> &[i32] {
        &self.gain_list
    }

    /// Re-query the device for its valid tuner gains.
    fn refresh_gains(&mut self) -> Result<(), RtlSdrError> {
        let dev = self.dev.ok_or(RtlSdrError::NotOpen)?;
        unsafe {
            let n = rtlsdr_sys::rtlsdr_get_tuner_gains(dev, std::ptr::null_mut());
            if n > 0 {
                let mut gains = vec![0i32; n as usize];
                let n2 = rtlsdr_sys::rtlsdr_get_tuner_gains(dev, gains.as_mut_ptr());
                if n2 == n {
                    gains.sort();
                    self.gain_list = gains;
                } else {
                    return Err(RtlSdrError::Device(
                        "rtlsdr_get_tuner_gains returned inconsistent count".into(),
                    ));
                }
            } else {
                self.gain_list.clear();
            }
        }
        Ok(())
    }

    /// Apply every cached setting to the open device.
    fn apply_all_settings(&mut self) -> Result<(), RtlSdrError> {
        let dev = self.dev.ok_or(RtlSdrError::NotOpen)?;
        unsafe {
            let check = |r: c_int, msg: &str| -> Result<(), RtlSdrError> {
                if r < 0 {
                    Err(RtlSdrError::Device(format!("{} returned {}", msg, r)))
                } else {
                    Ok(())
                }
            };

            check(
                rtlsdr_sys::rtlsdr_set_sample_rate(dev, self.sample_rate),
                "rtlsdr_set_sample_rate",
            )?;
            check(
                rtlsdr_sys::rtlsdr_set_center_freq(dev, self.freq),
                "rtlsdr_set_center_freq",
            )?;
            check(
                rtlsdr_sys::rtlsdr_set_freq_correction(dev, self.ppm),
                "rtlsdr_set_freq_correction",
            )?;
            check(
                rtlsdr_sys::rtlsdr_set_tuner_bandwidth(dev, 0),
                "rtlsdr_set_tuner_bandwidth",
            )?;
            check(
                rtlsdr_sys::rtlsdr_set_direct_sampling(dev, self.direct_sampling),
                "rtlsdr_set_direct_sampling",
            )?;
            check(
                rtlsdr_set_bias_tee(dev, if self.bias_t { 1 } else { 0 }),
                "rtlsdr_set_bias_tee",
            )?;
            check(
                rtlsdr_sys::rtlsdr_set_agc_mode(dev, if self.rtl_agc { 1 } else { 0 }),
                "rtlsdr_set_agc_mode",
            )?;
            check(
                rtlsdr_sys::rtlsdr_set_offset_tuning(
                    dev,
                    if self.offset_tuning { 1 } else { 0 },
                ),
                "rtlsdr_set_offset_tuning",
            )?;

            if self.tuner_agc {
                check(
                    rtlsdr_sys::rtlsdr_set_tuner_gain_mode(dev, 0),
                    "rtlsdr_set_tuner_gain_mode",
                )?;
            } else {
                check(
                    rtlsdr_sys::rtlsdr_set_tuner_gain_mode(dev, 1),
                    "rtlsdr_set_tuner_gain_mode",
                )?;
                check(
                    rtlsdr_sys::rtlsdr_set_tuner_gain(dev, self.gain),
                    "rtlsdr_set_tuner_gain",
                )?;
            }
        }
        Ok(())
    }
}

impl Default for RtlSdrSource {
    fn default() -> Self {
        Self::new()
    }
}

impl Source for RtlSdrSource {
    type Error = RtlSdrError;

    /// Open the RTL-SDR device by index and populate the gain list.
    fn open(&mut self, device_index: i32) -> Result<(), RtlSdrError> {
        if self.dev.is_some() {
            let _ = self.stop();
            if let Some(dev) = self.dev.take() {
                unsafe {
                    rtlsdr_sys::rtlsdr_close(dev);
                }
            }
        }

        let mut dev: rtlsdr_sys::rtlsdr_dev_t = std::ptr::null_mut();
        unsafe {
            let r = rtlsdr_sys::rtlsdr_open(&mut dev, device_index as u32);
            if r < 0 {
                return Err(RtlSdrError::Device(format!(
                    "rtlsdr_open returned {}",
                    r
                )));
            }
        }

        self.dev = Some(dev);
        self.device_index = device_index;
        self.refresh_gains()?;

        // Default to the largest available gain if any are known.
        if let Some(&g) = self.gain_list.last() {
            self.gain = g;
        }

        Ok(())
    }

    /// Set the centre frequency, retrying up to 10 times to match original C++ logic.
    fn set_freq(&mut self, freq_hz: u32) -> Result<(), RtlSdrError> {
        self.freq = freq_hz;
        if let Some(dev) = self.dev {
            unsafe {
                let mut attempts = 0;
                loop {
                    let r = rtlsdr_sys::rtlsdr_set_center_freq(dev, freq_hz);
                    if r < 0 {
                        return Err(RtlSdrError::Device(format!(
                            "rtlsdr_set_center_freq returned {}",
                            r
                        )));
                    }
                    let actual = rtlsdr_sys::rtlsdr_get_center_freq(dev);
                    if actual == freq_hz {
                        break;
                    }
                    attempts += 1;
                    if attempts >= 10 {
                        return Err(RtlSdrError::Device(
                            "Failed to tune after 10 attempts".into(),
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// Set manual tuner gain in tenths of a dB.
    fn set_gain(&mut self, gain_db_tenths: i32) -> Result<(), RtlSdrError> {
        self.gain = gain_db_tenths;
        if let Some(dev) = self.dev {
            unsafe {
                let r = rtlsdr_sys::rtlsdr_set_tuner_gain(dev, gain_db_tenths);
                if r < 0 {
                    return Err(RtlSdrError::Device(format!(
                        "rtlsdr_set_tuner_gain returned {}",
                        r
                    )));
                }
            }
        }
        Ok(())
    }

    /// Start the asynchronous sample stream.
    fn start(&mut self) -> Result<(), RtlSdrError> {
        if self.running {
            return Err(RtlSdrError::AlreadyRunning);
        }
        let dev = self.dev.ok_or(RtlSdrError::NotOpen)?;

        self.apply_all_settings()?;

        // Match original C++ calculation for the async buffer length.
        let async_count =
            ((self.sample_rate as f32 / (200.0 * 512.0)).round() as u32 * 512).max(512);

        let (tx, rx) = bounded::<Vec<Complex<f32>>>(32);
        self.rx = Some(rx);
        self.overflow.clear();

        unsafe {
            let r = rtlsdr_sys::rtlsdr_reset_buffer(dev);
            if r < 0 {
                return Err(RtlSdrError::Device(format!(
                    "rtlsdr_reset_buffer returned {}",
                    r
                )));
            }
        }

        let ctx = Box::new(AsyncContext { tx });
        let ctx_ptr = Box::into_raw(ctx);

        // Cast to usize so the closure automatically implements Send.
        let dev_usize = dev as usize;
        let ctx_usize = ctx_ptr as usize;

        let handle = thread::spawn(move || {
            unsafe {
                let _ = rtlsdr_sys::rtlsdr_read_async(
                    dev_usize as rtlsdr_sys::rtlsdr_dev_t,
                    async_callback,
                    ctx_usize as *mut c_void,
                    0,
                    async_count,
                );
                // Reclaim the box once `read_async` returns (after cancel).
                let _ = Box::from_raw(ctx_usize as *mut AsyncContext);
            }
        });

        self.worker = Some(handle);
        self.running = true;
        Ok(())
    }

    /// Stop the asynchronous stream and join the worker thread.
    fn stop(&mut self) -> Result<(), RtlSdrError> {
        if !self.running {
            return Err(RtlSdrError::NotRunning);
        }
        if let Some(dev) = self.dev {
            unsafe {
                let _ = rtlsdr_sys::rtlsdr_cancel_async(dev);
            }
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.rx = None;
        self.running = false;
        Ok(())
    }

    /// Read converted complex samples into `buf`.
    ///
    /// Blocks until enough samples have arrived or the stream ends.
    fn read_samples(&mut self, buf: &mut [Complex<f32>]) -> Result<usize, RtlSdrError> {
        if !self.running {
            return Err(RtlSdrError::NotRunning);
        }
        let rx = self.rx.as_ref().ok_or(RtlSdrError::NotRunning)?;

        let mut filled = 0usize;
        while filled < buf.len() {
            if !self.overflow.is_empty() {
                let to_copy = self.overflow.len().min(buf.len() - filled);
                buf[filled..filled + to_copy]
                    .copy_from_slice(&self.overflow[..to_copy]);
                self.overflow.drain(..to_copy);
                filled += to_copy;
            } else {
                match rx.recv() {
                    Ok(samples) => self.overflow = samples,
                    Err(_) => break, // channel disconnected -> stream ended
                }
            }
        }
        Ok(filled)
    }
}

impl Drop for RtlSdrSource {
    fn drop(&mut self) {
        let _ = self.stop();
        if let Some(dev) = self.dev.take() {
            unsafe {
                rtlsdr_sys::rtlsdr_close(dev);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_devices() {
        // This will return an empty list if no RTL-SDR is plugged in.
        let _devs = RtlSdrSource::list_devices();
    }

    #[test]
    fn test_default_state() {
        let src = RtlSdrSource::new();
        assert!(!src.running);
        assert_eq!(src.sample_rate, 2_400_000);
    }
}
