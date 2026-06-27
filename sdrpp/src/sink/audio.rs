//! Audio sink — translated from `sink_modules/audio_sink/src/main.cpp`
//!
//! Provides real-time stereo audio output using **cpal**.  The sink receives
//! `f32` stereo samples (the native DSP format used by SDR++) and converts
//! them to `i16` for the hardware device.  A lock-free SPSC ring buffer sits
//! between the DSP thread and the CPAL callback so the callback never blocks.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Host, SampleFormat, SampleRate, Stream};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// -----------------------------------------------------------------------------
// Types
// -----------------------------------------------------------------------------

/// Stereo sample pair (translated from `dsp::stereo_t`).
pub use crate::dsp::Stereo as StereoSample;

/// Static metadata for an output device.
#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub name: String,
    pub is_default: bool,
    pub sample_rates: Vec<u32>,
    pub preferred_rate: u32,
}

// -----------------------------------------------------------------------------
// Lock-free SPSC ring buffer (real-time safe)
// -----------------------------------------------------------------------------

/// Ring capacity — must be a power of two so the slot mask is cheap.
const RING_SIZE: usize = 16384;

struct StereoRingBuffer {
    buf: Box<[StereoSample]>,
    write_idx: AtomicUsize,
    read_idx: AtomicUsize,
}

impl StereoRingBuffer {
    fn new() -> Self {
        Self {
            buf: vec![StereoSample::default(); RING_SIZE].into_boxed_slice(),
            write_idx: AtomicUsize::new(0),
            read_idx: AtomicUsize::new(0),
        }
    }

    #[inline]
    fn mask(&self, idx: usize) -> usize {
        idx & (RING_SIZE - 1)
    }

    /// Number of samples currently buffered.
    fn len(&self) -> usize {
        let w = self.write_idx.load(Ordering::Acquire);
        let r = self.read_idx.load(Ordering::Acquire);
        w.wrapping_sub(r)
    }

    /// Push samples from the producer side.  Returns how many were written.
    fn write(&self, samples: &[StereoSample]) -> usize {
        let w = self.write_idx.load(Ordering::Relaxed);
        let r = self.read_idx.load(Ordering::Acquire);
        let fill = w.wrapping_sub(r);
        let avail = RING_SIZE - 1 - fill;
        let to_write = samples.len().min(avail);
        for i in 0..to_write {
            let idx = self.mask(w.wrapping_add(i));
            // SAFETY: `idx` is masked to `[0, RING_SIZE)`, the allocation is
            // stable (Box), and we are the only writer.
            unsafe { (self.buf.as_ptr().add(idx) as *mut StereoSample).write(samples[i]) };
        }
        self.write_idx.store(w.wrapping_add(to_write), Ordering::Release);
        to_write
    }

    /// Pop samples from the consumer side.  Returns how many were read.
    fn read(&self, dst: &mut [StereoSample]) -> usize {
        let r = self.read_idx.load(Ordering::Relaxed);
        let w = self.write_idx.load(Ordering::Acquire);
        let avail = w.wrapping_sub(r);
        let to_read = dst.len().min(avail);
        for i in 0..to_read {
            let idx = self.mask(r.wrapping_add(i));
            // SAFETY: same bounds reasoning as `write`.
            dst[i] = unsafe { self.buf.as_ptr().add(idx).read() };
        }
        self.read_idx.store(r.wrapping_add(to_read), Ordering::Release);
        to_read
    }

    /// Discard all pending samples.
    fn clear(&self) {
        let w = self.write_idx.load(Ordering::Relaxed);
        self.read_idx.store(w, Ordering::Release);
    }
}

// -----------------------------------------------------------------------------
// AudioSink
// -----------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum AudioSinkError {
    #[error("no audio output devices available")]
    NoDevices,
    #[error("device id {0} is out of range")]
    InvalidDeviceId(usize),
    #[error("cpal enumeration failed: {0}")]
    CpalEnumerate(#[from] cpal::DevicesError),
    #[error("unsupported stream configuration: {0}")]
    StreamConfig(String),
    #[error("failed to open stream: {0}")]
    OpenStream(String),
    #[error("failed to start stream: {0}")]
    StartStream(String),
    #[error("config i/o error: {0}")]
    ConfigIo(#[from] std::io::Error),
    #[error("config serialization error: {0}")]
    ConfigSer(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, AudioSinkError>;

pub struct AudioSink {
    stream_name: String,
    host: Host,
    device_infos: Vec<DeviceInfo>,
    devices: Vec<Device>,
    current_device_id: usize,
    default_device_id: usize,
    sample_rate: u32,
    sr_id: usize,

    stream: Option<Stream>,
    running: bool,

    /// Producer pushes here; the CPAL callback pulls from here.
    ring: Arc<StereoRingBuffer>,

    /// Persistent settings — mirrors the C++ `config` global.
    config_path: Option<std::path::PathBuf>,
    config: serde_json::Value,
}

impl AudioSink {
    /// Create a new sink and auto-select the device saved in config (or the
    /// OS default).
    pub fn new(stream_name: impl Into<String>) -> Result<Self> {
        let stream_name = stream_name.into();
        let host = cpal::default_host();
        let (device_infos, devices, default_device_id) = Self::enum_devices(&host)?;

        if device_infos.is_empty() {
            return Err(AudioSinkError::NoDevices);
        }

        // Load existing JSON config, if any.
        let config_path = std::env::var("SDRPP_ROOT")
            .ok()
            .map(|r| std::path::PathBuf::from(r).join("audio_sink_config.json"));
        let config = config_path
            .as_ref()
            .and_then(|p| Self::load_json(p))
            .unwrap_or_else(|| serde_json::json!({}));

        let mut sink = Self {
            stream_name,
            host,
            device_infos,
            devices,
            current_device_id: 0,
            default_device_id,
            sample_rate: 48_000,
            sr_id: 0,
            stream: None,
            running: false,
            ring: Arc::new(StereoRingBuffer::new()),
            config_path,
            config,
        };

        // Select device stored in config, falling back to OS default.
        let saved_device = sink
            .config
            .get(&sink.stream_name)
            .and_then(|v| v.get("device"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        match saved_device {
            Some(name) => sink.select_by_name(&name),
            None => sink.select_by_id(default_device_id),
        }

        Ok(sink)
    }

    // -----------------------------------------------------------------------
    // Device / sample-rate inspection
    // -----------------------------------------------------------------------

    pub fn devices(&self) -> &[DeviceInfo] {
        &self.device_infos
    }

    pub fn current_device(&self) -> &DeviceInfo {
        &self.device_infos[self.current_device_id]
    }

    pub fn sample_rates(&self) -> &[u32] {
        &self.current_device().sample_rates
    }

    pub fn current_sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn current_sample_rate_id(&self) -> usize {
        self.sr_id
    }

    pub fn current_device_id(&self) -> usize {
        self.current_device_id
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    // -----------------------------------------------------------------------
    // Selection
    // -----------------------------------------------------------------------

    pub fn select_first(&mut self) {
        self.select_by_id(self.default_device_id);
    }

    pub fn select_by_name(&mut self, name: &str) {
        if let Some(id) = self.device_infos.iter().position(|d| d.name == name) {
            self.select_by_id(id);
        } else {
            self.select_first();
        }
    }

    pub fn select_by_id(&mut self, id: usize) {
        if id >= self.device_infos.len() {
            return;
        }
        self.current_device_id = id;

        let dev = &self.device_infos[id];
        let default_sr = dev.preferred_rate;

        // Attempt to restore the previously used rate for this device.
        let saved_sr = self
            .config
            .get(&self.stream_name)
            .and_then(|v| v.get("devices"))
            .and_then(|v| v.get(&dev.name))
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);

        let (sr_id, actual_sr) = if let Some(sr) = saved_sr {
            match dev.sample_rates.iter().position(|&r| r == sr) {
                Some(idx) => (idx, sr),
                None => Self::pick_default_rate(&dev.sample_rates, default_sr),
            }
        } else {
            Self::pick_default_rate(&dev.sample_rates, default_sr)
        };

        self.sample_rate = actual_sr;
        self.sr_id = sr_id;

        // Persist device choice.
        if let Some(map) = self.config.as_object_mut() {
            let entry = map
                .entry(self.stream_name.clone())
                .or_insert_with(|| serde_json::json!({}));
            if let Some(obj) = entry.as_object_mut() {
                obj.insert("device".to_string(), serde_json::json!(dev.name));
            }
        }
        let _ = self.save_config();

        // Restart stream so the new device / rate takes effect.
        if self.running {
            let _ = self.stop();
            let _ = self.start();
        }
    }

    pub fn select_sample_rate_by_id(&mut self, sr_id: usize) {
        let rates = &self.device_infos[self.current_device_id].sample_rates;
        if sr_id >= rates.len() {
            return;
        }
        self.sr_id = sr_id;
        self.sample_rate = rates[sr_id];

        let dev_name = &self.device_infos[self.current_device_id].name;
        if let Some(map) = self.config.as_object_mut() {
            let entry = map
                .entry(self.stream_name.clone())
                .or_insert_with(|| serde_json::json!({}));
            if let Some(obj) = entry.as_object_mut() {
                let devs = obj
                    .entry("devices")
                    .or_insert_with(|| serde_json::json!({}));
                if let Some(dev_map) = devs.as_object_mut() {
                    dev_map.insert(dev_name.clone(), serde_json::json!(self.sample_rate));
                }
            }
        }
        let _ = self.save_config();

        if self.running {
            let _ = self.stop();
            let _ = self.start();
        }
    }

    // -----------------------------------------------------------------------
    // Lifecycle
    // -----------------------------------------------------------------------

    /// Feed stereo `f32` samples into the sink.  This is the producer side;
    /// it never blocks and simply drops samples when the ring buffer is full.
    pub fn push_samples(&self, samples: &[StereoSample]) {
        self.ring.write(samples);
    }

    pub fn start(&mut self) -> Result<()> {
        if self.running {
            return Ok(());
        }
        self.do_start()?;
        self.running = true;
        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        if !self.running {
            return Ok(());
        }
        self.do_stop();
        self.running = false;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn enum_devices(host: &Host) -> Result<(Vec<DeviceInfo>, Vec<Device>, usize)> {
        let mut infos = Vec::new();
        let mut devices = Vec::new();
        let mut default_id = 0usize;

        let default_dev = host.default_output_device();

        for dev in host.output_devices()? {
            let name = dev.name().unwrap_or_else(|_| "Unknown".to_string());
            let is_default = default_dev
                .as_ref()
                .and_then(|d| d.name().ok())
                .map(|n| n == name)
                .unwrap_or(false);

            // Harvest supported sample-rate ranges.
            let mut sample_rates: Vec<u32> = Vec::new();
            let mut preferred_rate = 48_000u32;

            if let Ok(def_cfg) = dev.default_output_config() {
                preferred_rate = def_cfg.sample_rate().0;
                if !sample_rates.contains(&preferred_rate) {
                    sample_rates.push(preferred_rate);
                }
            }

            if let Ok(cfgs) = dev.supported_output_configs() {
                for cfg in cfgs {
                    let min = cfg.min_sample_rate().0;
                    let max = cfg.max_sample_rate().0;
                    for &sr in &[44_100u32, 48_000, 88_200, 96_000, 192_000] {
                        if sr >= min && sr <= max && !sample_rates.contains(&sr) {
                            sample_rates.push(sr);
                        }
                    }
                }
            }

            sample_rates.sort_unstable();
            if sample_rates.is_empty() {
                sample_rates.push(preferred_rate);
            }

            if is_default {
                default_id = infos.len();
            }

            infos.push(DeviceInfo {
                name,
                is_default,
                sample_rates: sample_rates.clone(),
                preferred_rate,
            });
            devices.push(dev);
        }

        Ok((infos, devices, default_id))
    }

    fn do_start(&mut self) -> Result<()> {
        let device = &self.devices[self.current_device_id];

        // Try to find an I16 config at the requested sample rate.  If that
        // isn't available, fall back to the default output config (which may
        // force us to stay in f32).
        let supported_configs = device
            .supported_output_configs()
            .map_err(|e| AudioSinkError::StreamConfig(e.to_string()))?;

        let mut chosen = None;
        for cfg in supported_configs {
            let min = cfg.min_sample_rate().0;
            let max = cfg.max_sample_rate().0;
            if cfg.sample_format() == SampleFormat::I16
                && self.sample_rate >= min
                && self.sample_rate <= max
            {
                chosen = Some(cfg.with_sample_rate(SampleRate(self.sample_rate)));
                break;
            }
        }

        let fallback = || {
            device
                .supported_output_configs()
                .ok()?
                .find(|c| {
                    c.min_sample_rate().0 <= self.sample_rate
                        && c.max_sample_rate().0 >= self.sample_rate
                })
                .map(|c| c.with_sample_rate(SampleRate(self.sample_rate)))
                .or_else(|| device.default_output_config().ok())
        };

        let supported = chosen
            .or_else(fallback)
            .ok_or_else(|| AudioSinkError::StreamConfig(
                "no usable output configuration found".into(),
            ))?;

        let sample_format = supported.sample_format();
        let config = supported.config();

        // If we fell back to the default config, adopt its sample rate so
        // the DSP path upstream sees a consistent value.
        let stream_sr = config.sample_rate.0;
        if stream_sr != self.sample_rate {
            self.sample_rate = stream_sr;
            self.sr_id = self
                .current_device()
                .sample_rates
                .iter()
                .position(|&r| r == stream_sr)
                .unwrap_or(0);
        }

        let ring = Arc::clone(&self.ring);
        let name = self.stream_name.clone();
        let err_cb = move |err: cpal::StreamError| {
            eprintln!("AudioSink '{}' stream error: {}", name, err);
        };

        // Choose the callback flavour that matches the device's native format.
        let stream = match sample_format {
            SampleFormat::I16 => device.build_output_stream(
                &config,
                move |data: &mut [i16], _info: &cpal::OutputCallbackInfo| {
                    Self::fill_i16(data, &ring);
                },
                err_cb,
                None,
            ),
            SampleFormat::F32 => device.build_output_stream(
                &config,
                move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                    Self::fill_f32(data, &ring);
                },
                err_cb,
                None,
            ),
            SampleFormat::U16 => device.build_output_stream(
                &config,
                move |data: &mut [u16], _info: &cpal::OutputCallbackInfo| {
                    Self::fill_u16(data, &ring);
                },
                err_cb,
                None,
            ),
            _ => {
                return Err(AudioSinkError::OpenStream(format!(
                    "unsupported sample format {:?}",
                    sample_format
                )));
            }
        }
        .map_err(|e| AudioSinkError::OpenStream(e.to_string()))?;

        stream
            .play()
            .map_err(|e| AudioSinkError::StartStream(e.to_string()))?;

        self.stream = Some(stream);
        Ok(())
    }

    fn do_stop(&mut self) {
        if let Some(stream) = self.stream.take() {
            let _ = stream.pause();
        }
        self.ring.clear();
    }

    /// Clamp `f32` → `i16` conversion.
    #[inline]
    fn f32_to_i16(v: f32) -> i16 {
        (v.clamp(-1.0, 1.0) * (i16::MAX as f32)) as i16
    }

    /// Fill an `i16` buffer from the ring.  This is the hot path requested by
    /// the user (f32 → i16 conversion).
    fn fill_i16(output: &mut [i16], ring: &StereoRingBuffer) {
        let frames = output.len() / 2;
        let mut tmp = [StereoSample::default(); 512];
        let mut written = 0;

        while written < frames {
            let chunk = (frames - written).min(tmp.len());
            let read = ring.read(&mut tmp[..chunk]);
            for i in 0..read {
                output[(written + i) * 2] = Self::f32_to_i16(tmp[i].l);
                output[(written + i) * 2 + 1] = Self::f32_to_i16(tmp[i].r);
            }
            // Silence on underrun.
            for i in read..chunk {
                output[(written + i) * 2] = 0;
                output[(written + i) * 2 + 1] = 0;
            }
            written += chunk;
        }
    }

    fn fill_f32(output: &mut [f32], ring: &StereoRingBuffer) {
        let frames = output.len() / 2;
        let mut tmp = [StereoSample::default(); 512];
        let mut written = 0;

        while written < frames {
            let chunk = (frames - written).min(tmp.len());
            let read = ring.read(&mut tmp[..chunk]);
            for i in 0..read {
                output[(written + i) * 2] = tmp[i].l;
                output[(written + i) * 2 + 1] = tmp[i].r;
            }
            for i in read..chunk {
                output[(written + i) * 2] = 0.0;
                output[(written + i) * 2 + 1] = 0.0;
            }
            written += chunk;
        }
    }

    fn fill_u16(output: &mut [u16], ring: &StereoRingBuffer) {
        let frames = output.len() / 2;
        let mut tmp = [StereoSample::default(); 512];
        let mut written = 0;

        while written < frames {
            let chunk = (frames - written).min(tmp.len());
            let read = ring.read(&mut tmp[..chunk]);
            for i in 0..read {
                let l = ((tmp[i].l.clamp(-1.0, 1.0) + 1.0) * 0.5 * (u16::MAX as f32)) as u16;
                let r = ((tmp[i].r.clamp(-1.0, 1.0) + 1.0) * 0.5 * (u16::MAX as f32)) as u16;
                output[(written + i) * 2] = l;
                output[(written + i) * 2 + 1] = r;
            }
            for i in read..chunk {
                output[(written + i) * 2] = u16::MAX / 2;
                output[(written + i) * 2 + 1] = u16::MAX / 2;
            }
            written += chunk;
        }
    }

    // -----------------------------------------------------------------------
    // Config helpers
    // -----------------------------------------------------------------------

    fn pick_default_rate(rates: &[u32], default: u32) -> (usize, u32) {
        match rates.iter().position(|&r| r == default) {
            Some(idx) => (idx, default),
            None => (0, rates[0]),
        }
    }

    fn load_json(p: &Path) -> Option<serde_json::Value> {
        std::fs::read_to_string(p)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    }

    fn save_config(&self) -> Result<()> {
        if let Some(ref p) = self.config_path {
            let txt = serde_json::to_string_pretty(&self.config)?;
            std::fs::write(p, txt)?;
        }
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Sink provider — mirrors the C++ `AudioSinkModule` factory
// -----------------------------------------------------------------------------

/// Factory context that can be handed to a ModuleManager-style registry.
pub struct AudioSinkProvider;

impl AudioSinkProvider {
    pub fn create(stream_name: &str) -> Result<AudioSink> {
        AudioSink::new(stream_name)
    }
}
