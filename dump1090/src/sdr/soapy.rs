//! SoapySDR source - translated from sdr_soapy.c

use std::ffi::{c_char, c_int, c_long, c_void, CStr, CString};
use std::ptr;

use crate::convert;
use crate::sdr::SdrSource;

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
    size: usize,
    keys: *mut *mut c_char,
    vals: *mut *mut c_char,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SoapySDRRange {
    pub minimum: f64,
    pub maximum: f64,
    pub step: f64,
}

pub const SOAPY_SDR_RX: i32 = 2;
pub const SOAPY_SDR_CS16: *const c_char = b"CS16\0".as_ptr() as *const c_char;

#[allow(dead_code)]
extern "C" {
    fn SoapySDRDevice_enumerateStrArgs(args: *const c_char, length: *mut usize) -> *mut SoapySDRKwargs;
    fn SoapySDRKwargsList_clear(args: *mut SoapySDRKwargs, length: usize);
    fn SoapySDRDevice_makeStrArgs(args: *const c_char) -> *mut SoapySDRDevice;
    fn SoapySDRDevice_unmake(dev: *mut SoapySDRDevice);
    fn SoapySDRDevice_lastError() -> *const c_char;
    fn SoapySDRDevice_getHardwareInfo(dev: *mut SoapySDRDevice) -> SoapySDRKwargs;
    fn SoapySDRKwargs_clear(kwargs: *mut SoapySDRKwargs);
    fn SoapySDRDevice_getDriverKey(dev: *mut SoapySDRDevice) -> *mut c_char;
    fn SoapySDRDevice_getHardwareKey(dev: *mut SoapySDRDevice) -> *mut c_char;
    fn SoapySDR_free(ptr: *mut c_void);
    fn SoapySDRDevice_writeSetting(dev: *mut SoapySDRDevice, key: *const c_char, value: *const c_char) -> c_int;
    fn SoapySDRDevice_getNumChannels(dev: *mut SoapySDRDevice, direction: i32) -> usize;
    fn SoapySDRDevice_setSampleRate(dev: *mut SoapySDRDevice, direction: i32, channel: usize, rate: f64) -> c_int;
    fn SoapySDRDevice_setAntenna(dev: *mut SoapySDRDevice, direction: i32, channel: usize, name: *const c_char) -> c_int;
    fn SoapySDRDevice_getAntenna(dev: *mut SoapySDRDevice, direction: i32, channel: usize) -> *mut c_char;
    fn SoapySDRDevice_listAntennas(dev: *mut SoapySDRDevice, direction: i32, channel: usize, length: *mut usize) -> *mut *mut c_char;
    fn SoapySDRStrings_clear(strs: *mut *mut c_char, length: usize);
    fn SoapySDRDevice_setFrequency(dev: *mut SoapySDRDevice, direction: i32, channel: usize, freq: f64, args: *const SoapySDRKwargs) -> c_int;
    fn SoapySDRDevice_getGainRange(dev: *mut SoapySDRDevice, direction: i32, channel: usize) -> SoapySDRRange;
    fn SoapySDRDevice_hasGainMode(dev: *mut SoapySDRDevice, direction: i32, channel: usize) -> bool;
    fn SoapySDRDevice_setGainMode(dev: *mut SoapySDRDevice, direction: i32, channel: usize, automatic: bool) -> c_int;
    fn SoapySDRDevice_setGain(dev: *mut SoapySDRDevice, direction: i32, channel: usize, gain: f64) -> c_int;
    fn SoapySDRDevice_setGainElement(dev: *mut SoapySDRDevice, direction: i32, channel: usize, element: *const c_char, gain: f64) -> c_int;
    fn SoapySDRDevice_getGain(dev: *mut SoapySDRDevice, direction: i32, channel: usize) -> f64;
    fn SoapySDRDevice_setBandwidth(dev: *mut SoapySDRDevice, direction: i32, channel: usize, bw: f64) -> c_int;
    fn SoapySDRDevice_setupStream(
        dev: *mut SoapySDRDevice,
        direction: i32,
        format: *const c_char,
        channels: *const usize,
        numChans: usize,
        args: *const SoapySDRKwargs,
    ) -> *mut SoapySDRStream;
    fn SoapySDRDevice_activateStream(
        dev: *mut SoapySDRDevice,
        stream: *mut SoapySDRStream,
        flags: c_int,
        timeNs: i64,
        numElems: usize,
    ) -> c_int;
    fn SoapySDRDevice_readStream(
        dev: *mut SoapySDRDevice,
        stream: *mut SoapySDRStream,
        buffs: *mut *mut c_void,
        numElems: usize,
        flags: *mut c_int,
        timeNs: *mut i64,
        timeoutUs: c_long,
    ) -> c_int;
    fn SoapySDRDevice_closeStream(dev: *mut SoapySDRDevice, stream: *mut SoapySDRStream);
    fn SoapySDRDevice_getFrequency(dev: *mut SoapySDRDevice, direction: i32, channel: usize) -> f64;
    fn SoapySDRDevice_getSampleRate(dev: *mut SoapySDRDevice, direction: i32, channel: usize) -> f64;
    fn SoapySDRDevice_getBandwidth(dev: *mut SoapySDRDevice, direction: i32, channel: usize) -> f64;
    fn SoapySDRDevice_getGainMode(dev: *mut SoapySDRDevice, direction: i32, channel: usize) -> bool;
    fn SoapySDRDevice_hasDCOffset(dev: *mut SoapySDRDevice, direction: i32, channel: usize) -> bool;
    fn SoapySDRDevice_getDCOffsetMode(dev: *mut SoapySDRDevice, direction: i32, channel: usize) -> bool;
    fn SoapySDRDevice_getDCOffset(dev: *mut SoapySDRDevice, direction: i32, channel: usize, offsetI: *mut f64, offsetQ: *mut f64) -> c_int;
    fn SoapySDRDevice_hasIQBalance(dev: *mut SoapySDRDevice, direction: i32, channel: usize) -> bool;
    fn SoapySDRDevice_getIQBalance(dev: *mut SoapySDRDevice, direction: i32, channel: usize, balanceI: *mut f64, balanceQ: *mut f64) -> c_int;
    fn SoapySDRDevice_hasFrequencyCorrection(dev: *mut SoapySDRDevice, direction: i32, channel: usize) -> bool;
    fn SoapySDRDevice_getFrequencyCorrection(dev: *mut SoapySDRDevice, direction: i32, channel: usize) -> f64;
    fn SoapySDRDevice_listGains(dev: *mut SoapySDRDevice, direction: i32, channel: usize, length: *mut usize) -> *mut *mut c_char;
    fn SoapySDRDevice_getGainElement(dev: *mut SoapySDRDevice, direction: i32, channel: usize, element: *const c_char) -> f64;
}

fn last_err() -> String {
    unsafe {
        let ptr = SoapySDRDevice_lastError();
        if ptr.is_null() {
            "soapy: unknown error".to_string()
        } else {
            CStr::from_ptr(ptr).to_string_lossy().into_owned()
        }
    }
}

fn soapy_check(code: c_int, msg: &str) -> anyhow::Result<()> {
    if code != 0 {
        Err(anyhow::anyhow!("soapy: {} failed: {}", msg, last_err()))
    } else {
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct SoapyConfig {
    pub dev_name: Option<String>,
    pub channel: usize,
    pub antenna: Option<String>,
    pub bandwidth: f64,
    pub enable_agc: bool,
    pub gain: f64,
    pub gain_elements: Vec<(String, f64)>,
    pub settings: Vec<(String, String)>,
}

impl Default for SoapyConfig {
    fn default() -> Self {
        Self {
            dev_name: None,
            channel: 0,
            antenna: None,
            bandwidth: 0.0,
            enable_agc: false,
            gain: 999_999.0,
            gain_elements: Vec::new(),
            settings: Vec::new(),
        }
    }
}

pub struct SoapySdr {
    dev: *mut SoapySDRDevice,
    stream: *mut SoapySDRStream,
    config: SoapyConfig,
    freq: f64,
    sample_rate: f64,
    gain: f64,
}

unsafe impl Send for SoapySdr {}

impl SoapySdr {
    pub fn new(config: SoapyConfig, sample_rate: f64, freq: f64) -> Self {
        Self {
            dev: ptr::null_mut(),
            stream: ptr::null_mut(),
            config,
            freq,
            sample_rate,
            gain: 999_999.0,
        }
    }
}

impl Drop for SoapySdr {
    fn drop(&mut self) {
        if !self.stream.is_null() {
            unsafe { SoapySDRDevice_closeStream(self.dev, self.stream) };
            self.stream = ptr::null_mut();
        }
        if !self.dev.is_null() {
            unsafe { SoapySDRDevice_unmake(self.dev) };
            self.dev = ptr::null_mut();
        }
    }
}

impl SdrSource for SoapySdr {
    fn start(&mut self) -> anyhow::Result<()> {
        let dev_name_c = self.config.dev_name.as_deref().unwrap_or("");
        let dev_name_ptr = CString::new(dev_name_c)?;
        let mut length: usize = 0;

        let results = unsafe {
            SoapySDRDevice_enumerateStrArgs(dev_name_ptr.as_ptr(), &mut length)
        };

        if length == 0 {
            unsafe { SoapySDRKwargsList_clear(results, length) };
            return Err(anyhow::anyhow!("soapy: no device found"));
        }
        if length > 1 {
            unsafe { SoapySDRKwargsList_clear(results, length) };
            return Err(anyhow::anyhow!("soapy: please select a single device with --device"));
        }

        unsafe { SoapySDRKwargsList_clear(results, length) };

        self.dev = unsafe { SoapySDRDevice_makeStrArgs(dev_name_ptr.as_ptr()) };
        if self.dev.is_null() {
            return Err(anyhow::anyhow!("soapy: failed to create device: {}", last_err()));
        }

        let dev = self.dev;
        let ch = self.config.channel;

        if ch > 0 {
            let supported = unsafe { SoapySDRDevice_getNumChannels(dev, SOAPY_SDR_RX) };
            if ch >= supported {
                return Err(anyhow::anyhow!(
                    "soapy: device only supports {} channels, not {}",
                    supported,
                    ch + 1
                ));
            }
        }

        soapy_check(
            unsafe { SoapySDRDevice_setSampleRate(dev, SOAPY_SDR_RX, ch, self.sample_rate) },
            "setSampleRate",
        )?;

        if let Some(ref ant) = self.config.antenna {
            let ant_c = CString::new(ant.as_str())?;
            soapy_check(
                unsafe { SoapySDRDevice_setAntenna(dev, SOAPY_SDR_RX, ch, ant_c.as_ptr()) },
                "setAntenna",
            )?;
        }

        soapy_check(
            unsafe { SoapySDRDevice_setFrequency(dev, SOAPY_SDR_RX, ch, self.freq, ptr::null()) },
            "setFrequency",
        )?;

        if self.config.enable_agc {
            if unsafe { SoapySDRDevice_hasGainMode(dev, SOAPY_SDR_RX, ch) } {
                soapy_check(
                    unsafe { SoapySDRDevice_setGainMode(dev, SOAPY_SDR_RX, ch, true) },
                    "setGainMode",
                )?;
            } else {
                return Err(anyhow::anyhow!("soapy: device does not support AGC"));
            }
        } else {
            if unsafe { SoapySDRDevice_hasGainMode(dev, SOAPY_SDR_RX, ch) } {
                soapy_check(
                    unsafe { SoapySDRDevice_setGainMode(dev, SOAPY_SDR_RX, ch, false) },
                    "setGainMode",
                )?;
            }
            let gain = if self.config.gain >= 999_000.0 {
                let range = unsafe { SoapySDRDevice_getGainRange(dev, SOAPY_SDR_RX, ch) };
                range.maximum
            } else {
                self.config.gain
            };
            soapy_check(
                unsafe { SoapySDRDevice_setGain(dev, SOAPY_SDR_RX, ch, gain) },
                "setGain",
            )?;
            self.gain = gain;
        }

        let bw = if self.config.bandwidth > 0.0 {
            self.config.bandwidth
        } else {
            3.0e6
        };
        soapy_check(
            unsafe { SoapySDRDevice_setBandwidth(dev, SOAPY_SDR_RX, ch, bw) },
            "setBandwidth",
        )?;

        let channels: [usize; 1] = [ch];
        let stream_args = SoapySDRKwargs {
            size: 0,
            keys: ptr::null_mut(),
            vals: ptr::null_mut(),
        };

        self.stream = unsafe {
            SoapySDRDevice_setupStream(
                dev,
                SOAPY_SDR_RX,
                SOAPY_SDR_CS16,
                channels.as_ptr(),
                1,
                &stream_args,
            )
        };

        if self.stream.is_null() {
            return Err(anyhow::anyhow!("soapy: setupStream failed: {}", last_err()));
        }

        soapy_check(
            unsafe { SoapySDRDevice_activateStream(dev, self.stream, 0, 0, 0) },
            "activateStream",
        )?;

        Ok(())
    }

    fn stop(&mut self) {
        if !self.stream.is_null() {
            unsafe { SoapySDRDevice_closeStream(self.dev, self.stream) };
            self.stream = ptr::null_mut();
        }
        if !self.dev.is_null() {
            unsafe { SoapySDRDevice_unmake(self.dev) };
            self.dev = ptr::null_mut();
        }
    }

    fn set_frequency(&mut self, freq: u64) -> anyhow::Result<()> {
        self.freq = freq as f64;
        if !self.dev.is_null() {
            soapy_check(
                unsafe { SoapySDRDevice_setFrequency(self.dev, SOAPY_SDR_RX, self.config.channel, self.freq, ptr::null()) },
                "setFrequency",
            )?;
        }
        Ok(())
    }

    fn set_sample_rate(&mut self, rate: u32) -> anyhow::Result<()> {
        self.sample_rate = rate as f64;
        if !self.dev.is_null() {
            soapy_check(
                unsafe { SoapySDRDevice_setSampleRate(self.dev, SOAPY_SDR_RX, self.config.channel, self.sample_rate) },
                "setSampleRate",
            )?;
        }
        Ok(())
    }

    fn set_gain(&mut self, gain: f64) -> anyhow::Result<()> {
        self.gain = gain;
        if !self.dev.is_null() {
            soapy_check(
                unsafe { SoapySDRDevice_setGain(self.dev, SOAPY_SDR_RX, self.config.channel, gain) },
                "setGain",
            )?;
        }
        Ok(())
    }

    fn read_samples(&mut self, buf: &mut [u16]) -> anyhow::Result<usize> {
        if self.dev.is_null() || self.stream.is_null() {
            return Ok(0);
        }

        let want_samples = buf.len();
        let mut read_buf = vec![0u8; want_samples * 4]; // CS16 = 4 bytes per sample
        let mut buf_ptr = read_buf.as_mut_ptr() as *mut c_void;
        let mut flags: c_int = 0;
        let mut time_ns: i64 = 0;

        let samples_read = unsafe {
            SoapySDRDevice_readStream(
                self.dev,
                self.stream,
                &mut buf_ptr,
                want_samples,
                &mut flags,
                &mut time_ns,
                5_000_000,
            )
        };

        if samples_read <= 0 {
            return Ok(0);
        }

        let n = samples_read as usize;
        convert::convert_sc16_to_mag(&read_buf[..n * 4], &mut buf[..n]);
        Ok(n)
    }
}
