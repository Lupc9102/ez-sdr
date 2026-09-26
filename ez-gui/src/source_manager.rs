#![deny(unsafe_op_in_unsafe_fn)]

//! SDR source configuration and management.
//!
//! Provides [`SourceManager`] for enumerating, selecting, and connecting to
//! SoapySDR-compatible devices (RTL-SDR, `HackRF`, Airspy, `LimeSDR`, etc.)
//! as well as file and network IQ sources.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use crossbeam_channel::{bounded, Receiver, SendTimeoutError, Sender, TryRecvError, TrySendError};
use std::time::{Duration, Instant};

/// A current librtlsdr USB index and, when readable, its USB identity strings.
/// Indices are only meaningful for the most recent discovery snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtlDeviceInfo {
    pub index: u32,
    pub name: String,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial: Option<String>,
}

impl RtlDeviceInfo {
    pub fn label(&self) -> String {
        match &self.serial {
            Some(serial) => format!("{}: {} · {}", self.index, self.name, serial),
            None => format!("{}: {}", self.index, self.name),
        }
    }
}

/// Persist a unique serial where available so USB index changes cannot silently
/// select a different receiver. Devices with absent/duplicate serials use index.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RtlDeviceSelection {
    pub index: u32,
    pub serial: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DirectSamplingBranch {
    I,
    #[default]
    Q,
}

impl DirectSamplingBranch {
    pub fn label(self) -> &'static str {
        match self {
            Self::I => "I branch",
            Self::Q => "Q branch",
        }
    }

    pub fn mode(self) -> i32 {
        match self {
            Self::I => 1,
            Self::Q => 2,
        }
    }
}

fn checked_tuner_center(center_hz: u64, frequency_offset_hz: i64) -> Result<u32, String> {
    let physical = if frequency_offset_hz >= 0 {
        center_hz.checked_add(frequency_offset_hz as u64)
    } else {
        center_hz.checked_sub(frequency_offset_hz.unsigned_abs())
    }
    .ok_or_else(|| {
        "RTL-SDR capture center plus frequency offset is outside the valid frequency range."
            .to_string()
    })?;
    u32::try_from(physical).map_err(|_| "RTL-SDR capture center plus frequency offset exceeds the hardware API's 32-bit frequency range.".to_string())
}

fn resolve_rtl_device(
    selection: &RtlDeviceSelection,
    devices: &[RtlDeviceInfo],
) -> Result<u32, String> {
    if let Some(serial) = selection
        .serial
        .as_deref()
        .filter(|serial| !serial.is_empty())
    {
        let mut matching = devices
            .iter()
            .filter(|device| device.serial.as_deref() == Some(serial));
        let Some(device) = matching.next() else {
            return Err(format!("Selected RTL-SDR serial {serial:?} is not connected. Refresh and select a receiver."));
        };
        if matching.next().is_some() {
            return Err(format!(
                "More than one RTL-SDR has serial {serial:?}. Refresh and select its USB index."
            ));
        }
        Ok(device.index)
    } else {
        devices
            .iter()
            .find(|device| device.index == selection.index)
            .map(|device| device.index)
            .ok_or_else(|| {
                format!(
                    "RTL-SDR device {} is not connected. Refresh and select a receiver.",
                    selection.index
                )
            })
    }
}

fn validate_rtl_options(direct_sampling: bool, offset_tuning: bool) -> Result<(), String> {
    if direct_sampling && offset_tuning {
        Err("RTL-SDR offset tuning is unavailable in direct-sampling mode.".into())
    } else {
        Ok(())
    }
}

#[cfg(all(feature = "rtlsdr", not(test)))]
fn enumerate_rtl_devices() -> Result<Vec<RtlDeviceInfo>, String> {
    // SAFETY: librtlsdr owns the returned name strings; each is copied before
    // the next call. USB text buffers follow librtlsdr's documented 256-byte
    // size and conversion below never reads beyond those initialized buffers.
    let count = unsafe { rtlsdr_sys::rtlsdr_get_device_count() };
    let mut devices = Vec::new();
    for index in 0..count {
        let name = unsafe { rtlsdr_sys::rtlsdr_get_device_name(index) };
        let name = if name.is_null() {
            "RTL-SDR".to_string()
        } else {
            unsafe { std::ffi::CStr::from_ptr(name) }
                .to_string_lossy()
                .into_owned()
        };
        let mut manufacturer = [0; 256];
        let mut product = [0; 256];
        let mut serial = [0; 256];
        let identity_read = unsafe {
            rtlsdr_sys::rtlsdr_get_device_usb_strings(
                index,
                manufacturer.as_mut_ptr(),
                product.as_mut_ptr(),
                serial.as_mut_ptr(),
            )
        } == 0;
        let text = |buffer: &[std::ffi::c_char; 256]| {
            if !identity_read {
                return None;
            }
            let bytes: Vec<u8> = buffer
                .iter()
                .take_while(|&&byte| byte != 0)
                .map(|&byte| byte as u8)
                .collect();
            let value = String::from_utf8_lossy(&bytes).trim().to_string();
            (!value.is_empty()).then_some(value)
        };
        devices.push(RtlDeviceInfo {
            index,
            name,
            manufacturer: text(&manufacturer),
            product: text(&product),
            serial: text(&serial),
        });
    }
    Ok(devices)
}

#[cfg(any(not(feature = "rtlsdr"), test))]
fn enumerate_rtl_devices() -> Result<Vec<RtlDeviceInfo>, String> {
    Err("RTL-SDR discovery requires a hardware-enabled build (--features rtlsdr).".into())
}

enum SourceMessage {
    Samples(Vec<u8>),
    EndOfStream,
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DaemonAudioTuning {
    frequency_hz: u64,
    offset_hz: i64,
    bandwidth_hz: u32,
}

/// Live input cannot wait for the UI without overrunning the hardware's USB
/// buffers. Drop only the overflow block; a full queue is not a dead receiver.
fn send_live_samples(tx: &Sender<SourceMessage>, samples: Vec<u8>) -> bool {
    !matches!(
        tx.try_send(SourceMessage::Samples(samples)),
        Err(TrySendError::Disconnected(_))
    )
}

/// File replay and terminal messages must survive temporary UI stalls. A short
/// timeout lets Stop interrupt backpressure without losing a recording block.
fn send_cancellable(
    tx: &Sender<SourceMessage>,
    running: &AtomicBool,
    mut message: SourceMessage,
) -> bool {
    while running.load(Ordering::Acquire) {
        match tx.send_timeout(message, Duration::from_millis(20)) {
            Ok(()) => return true,
            Err(SendTimeoutError::Timeout(pending)) => message = pending,
            Err(SendTimeoutError::Disconnected(_)) => return false,
        }
    }
    false
}

fn wait_cancellable(running: &AtomicBool, duration: Duration) -> bool {
    let started = Instant::now();
    while running.load(Ordering::Acquire) {
        let remaining = duration.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return true;
        }
        std::thread::park_timeout(remaining.min(Duration::from_millis(20)));
    }
    false
}

fn iq_block_duration(byte_count: usize, sample_rate: u32) -> Duration {
    Duration::from_secs_f64((byte_count / 2) as f64 / f64::from(sample_rate))
}

/// Manages the SDR source lifecycle — starting, stopping, and reading IQ samples.
///
/// Supports three modes: real RTL-SDR hardware (behind `feature = "rtlsdr"`),
/// simulated multi-signal IQ generation for demo/testing, and replay from a
/// recorded IQ file.
pub struct SourceManager {
    /// Current source status (Idle / Opening / Running / Error).
    pub status: SourceStatus,
    /// Logical tuned/displayed channel (VFO) frequency in Hz.
    pub frequency_hz: u64,
    /// Independent logical capture center; None follows the tuned VFO.
    pub center_frequency_hz: Option<u64>,
    /// Local RTL tuner = logical capture center + this signed offset.
    /// Negative values subtract a transverter LO; other source kinds ignore it.
    pub frequency_offset_hz: i64,
    /// ADC sample rate in samples-per-second.
    pub sample_rate_hz: u32,
    /// RF gain in dB (typically 0.0–49.6 for RTL-SDR).
    pub gain_db: f64,
    /// Bias-T enable (4.5 V DC on antenna connector, RTL-SDR V3).
    pub bias_tee: bool,
    /// Frequency correction in parts-per-million.
    pub ppm_correction: i32,
    /// Direct-sampling mode for HF reception below 24 MHz.
    pub direct_sampling: bool,
    /// Branch used when direct_sampling is enabled. Q is the common HF input.
    pub direct_sampling_branch: DirectSamplingBranch,
    /// Ask librtlsdr to move the tuner DC spike outside the received passband.
    /// This is a hardware option, distinct from a frequency-display offset.
    pub offset_tuning: bool,
    /// Selected local RTL receiver (unique serial preferred over current index).
    pub rtl_device: RtlDeviceSelection,
    /// Latest completed USB enumeration. Refresh it from a background worker.
    pub rtl_devices: Vec<RtlDeviceInfo>,
    pub rtl_device_refresh_error: Option<String>,
    rtl_device_refresh: Option<Receiver<Result<Vec<RtlDeviceInfo>, String>>>,
    /// Tuner (hardware) AGC mode. When true, the RTL-SDR manages gain automatically.
    pub tuner_agc: bool,
    /// RTL AGC mode (separate from tuner AGC).
    pub rtl_agc: bool,
    /// Device temperature in degrees Celsius (if available).
    pub temperature: f32,
    /// Current source operating mode.
    pub source_mode: SourceMode,
    /// Path to an IQ recording file (used in Replay mode).
    pub replay_file: Option<String>,
    /// Whether to loop the replay file when the end is reached.
    pub replay_loop: bool,
    /// Replay speed multiplier (0.1×–10×).
    pub replay_speed: f32,
    /// Current read position within the replay file (bytes).
    pub replay_position: u64,
    /// Total size of the replay file (bytes).
    pub replay_size: u64,
    /// `host:port` of the `ez-daemon` to connect to in `SourceMode::Daemon`.
    pub daemon_addr: String,
    daemon_client: Option<crate::daemon_client::DaemonClient>,
    /// Last frequency/sample-rate/gain actually sent to the daemon (or last value received
    /// FROM it) — lets `sync_daemon_controls` tell "user moved a slider" (send) apart from
    /// "the daemon just told us its own state" (already in sync, don't echo it back).
    daemon_last_center_hz: u64,
    daemon_last_sample_rate_hz: u32,
    daemon_last_gain_db: f64,
    daemon_audio_mode: Option<ez_proto::DemodMode>,
    daemon_audio_tuning: Option<DaemonAudioTuning>,
    daemon_adsb_subscribed: bool,
    daemon_lrpt_subscribed: bool,
    daemon_recording_active: bool,
    tx: Option<Sender<SourceMessage>>,
    rx: Option<Receiver<SourceMessage>>,
    running: Arc<AtomicBool>,
    worker_handle: Option<std::thread::JoinHandle<()>>,
    stream_generation: u64,
}

/// The operating mode of the SDR source.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum SourceMode {
    /// Generate synthetic IQ data (default; works without real hardware).
    #[default]
    Simulated,
    /// Receive live samples from the selected RTL-SDR device.
    Hardware,
    /// Read IQ samples from a previously recorded file.
    Replay,
    /// Attach to a running `ez-daemon` over TCP instead of owning hardware/synthesis
    /// locally — see `crate::daemon_client::DaemonClient`.
    Daemon,
}

/// Wideband spectrum channel id this manager subscribes with in `SourceMode::Daemon`. Fixed
/// rather than user-configurable: exactly one spectrum view is wired up to daemon mode so far
/// (see `CentralApp`'s daemon-event routing), so there's only ever one channel to name.
const DAEMON_SPECTRUM_CHANNEL_ID: ez_proto::ChannelId = 1;
const DAEMON_AUDIO_CHANNEL_ID: ez_proto::ChannelId = 2;
const DAEMON_ADSB_CHANNEL_ID: ez_proto::ChannelId = 3;
const DAEMON_LRPT_CHANNEL_ID: ez_proto::ChannelId = 4;

/// The run-state of the SDR source.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceStatus {
    /// Source is stopped and ready to start.
    Idle,
    /// Source is initializing (device opening, etc.).
    Opening,
    /// Source is actively producing IQ samples.
    Running,
    /// Source encountered a fatal error (carries error message).
    Error(String),
}

impl Default for SourceManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceManager {
    /// Create a new `SourceManager` in `Idle` state with default tuning values.
    pub fn new() -> Self {
        let (tx, rx) = bounded(32);
        Self {
            status: SourceStatus::Idle,
            frequency_hz: 109_000_000,
            center_frequency_hz: None,
            frequency_offset_hz: 0,
            sample_rate_hz: 2_048_000,
            gain_db: 40.0,
            bias_tee: false,
            ppm_correction: 0,
            direct_sampling: false,
            direct_sampling_branch: DirectSamplingBranch::default(),
            offset_tuning: false,
            rtl_device: RtlDeviceSelection::default(),
            rtl_devices: Vec::new(),
            rtl_device_refresh_error: None,
            rtl_device_refresh: None,
            tuner_agc: false,
            rtl_agc: false,
            temperature: 0.0,
            source_mode: SourceMode::Simulated,
            replay_file: None,
            replay_loop: false,
            replay_speed: 1.0,
            replay_position: 0,
            replay_size: 0,
            daemon_addr: "127.0.0.1:7890".to_string(),
            daemon_client: None,
            daemon_last_center_hz: 109_000_000,
            daemon_last_sample_rate_hz: 2_048_000,
            daemon_last_gain_db: 40.0,
            daemon_audio_mode: None,
            daemon_audio_tuning: None,
            daemon_adsb_subscribed: false,
            daemon_lrpt_subscribed: false,
            daemon_recording_active: false,
            tx: Some(tx),
            rx: Some(rx),
            running: Arc::new(AtomicBool::new(false)),
            worker_handle: None,
            stream_generation: 0,
        }
    }

    /// Identity of the latest started local worker or daemon connection attempt.
    /// Changes on restart even when capture settings remain identical; stopping,
    /// rejected starts, and VFO-only control changes leave it unchanged.
    pub fn stream_generation(&self) -> u64 {
        self.stream_generation
    }

    pub fn capture_center_frequency_hz(&self) -> u64 {
        self.center_frequency_hz.unwrap_or(self.frequency_hz)
    }

    /// Validate the physical local RTL frequency without truncating RF/VFO
    /// frequencies above 4 GHz. Used only when the source is local RTL hardware.
    pub fn rtl_center_frequency_hz(&self) -> Result<u32, String> {
        checked_tuner_center(self.capture_center_frequency_hz(), self.frequency_offset_hz)
    }

    pub fn direct_sampling_mode(&self) -> i32 {
        if self.direct_sampling {
            self.direct_sampling_branch.mode()
        } else {
            0
        }
    }

    /// Start one USB discovery worker. Repeated clicks while it is running are
    /// ignored, so discovery cannot accumulate worker threads or queued scans.
    /// Poll with `poll_rtl_devices` each frame; no USB work occurs on the caller.
    pub fn refresh_rtl_devices(&mut self) -> bool {
        self.refresh_rtl_devices_with(enumerate_rtl_devices)
    }

    fn refresh_rtl_devices_with(
        &mut self,
        enumerate: impl FnOnce() -> Result<Vec<RtlDeviceInfo>, String> + Send + 'static,
    ) -> bool {
        if self.rtl_device_refresh.is_some() {
            return false;
        }
        let (sender, receiver) = bounded(1);
        self.rtl_device_refresh = Some(receiver);
        self.rtl_device_refresh_error = None;
        std::thread::spawn(move || {
            // A dropped manager disconnects this bounded channel; the worker
            // never waits for a UI consumer or keeps a SourceManager alive.
            let _ = sender.send(enumerate());
        });
        true
    }

    pub fn rtl_devices_refreshing(&self) -> bool {
        self.rtl_device_refresh.is_some()
    }

    /// Consume a discovery result without waiting. Returns true when the result
    /// changed; failed discovery preserves the previous snapshot and selection.
    pub fn poll_rtl_devices(&mut self) -> bool {
        let result =
            self.rtl_device_refresh
                .as_ref()
                .and_then(|receiver| match receiver.try_recv() {
                    Ok(result) => Some(result),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => {
                        Some(Err("RTL-SDR discovery worker stopped unexpectedly.".into()))
                    }
                });
        let Some(result) = result else { return false };
        self.rtl_device_refresh = None;
        match result {
            Ok(devices) => {
                self.rtl_devices = devices;
                match resolve_rtl_device(&self.rtl_device, &self.rtl_devices) {
                    Ok(index) => self.rtl_device.index = index,
                    Err(error) => self.rtl_device_refresh_error = Some(error),
                }
            }
            Err(error) => self.rtl_device_refresh_error = Some(error),
        }
        true
    }

    /// Validate against the completed discovery list and remember a unique USB
    /// serial. The caller restarts a running hardware source to apply selection.
    pub fn select_rtl_device(&mut self, index: u32) -> Result<(), String> {
        if self.rtl_devices_refreshing() {
            return Err("Wait for RTL-SDR discovery before selecting a receiver.".into());
        }
        let device = self
            .rtl_devices
            .iter()
            .find(|device| device.index == index)
            .ok_or_else(|| {
                "That RTL-SDR device is no longer listed. Refresh the device list.".to_string()
            })?;
        let serial = device
            .serial
            .as_ref()
            .filter(|serial| {
                !serial.is_empty()
                    && self
                        .rtl_devices
                        .iter()
                        .filter(|device| device.serial.as_ref() == Some(*serial))
                        .count()
                        == 1
            })
            .cloned();
        self.rtl_device = RtlDeviceSelection { index, serial };
        self.rtl_device_refresh_error = None;
        Ok(())
    }

    pub fn selected_rtl_device(&self) -> Option<&RtlDeviceInfo> {
        let index = resolve_rtl_device(&self.rtl_device, &self.rtl_devices).ok()?;
        self.rtl_devices.iter().find(|device| device.index == index)
    }

    /// Retune acquisition and the logical VFO to the same frequency and sample rate,
    /// restarting local sources so the worker cannot continue sampling stale settings.
    /// Daemon mode uses its live control channel instead of reconnecting.
    pub fn tune_and_restart(&mut self, frequency_hz: u64, sample_rate_hz: u32) {
        self.frequency_hz = frequency_hz;
        self.center_frequency_hz = Some(frequency_hz);
        self.sample_rate_hz = sample_rate_hz;
        if self.source_mode == SourceMode::Daemon {
            self.sync_daemon_controls();
        } else if self.status == SourceStatus::Running {
            self.stop();
            self.start();
        } else {
            self.start();
        }
    }

    /// Start the SDR source.
    ///
    /// Spawns a worker thread that generates or replays IQ samples and sends
    /// them through the internal channel. Switches status to `Running`.
    pub fn start(&mut self) {
        if matches!(self.status, SourceStatus::Running | SourceStatus::Opening) {
            return;
        }
        if self.source_mode == SourceMode::Daemon {
            self.start_daemon();
            return;
        }
        if self.worker_handle.is_some() {
            self.stop();
        }
        if self.sample_rate_hz == 0
            || (self.source_mode == SourceMode::Replay
                && (!self.replay_speed.is_finite() || self.replay_speed <= 0.0))
        {
            self.status = SourceStatus::Error(
                "Sample rate and replay speed must be greater than zero".into(),
            );
            return;
        }
        let mut rtl_frequency = 0;
        if self.source_mode == SourceMode::Hardware {
            if let Err(error) = validate_rtl_options(self.direct_sampling, self.offset_tuning) {
                self.status = SourceStatus::Error(error);
                return;
            }
            match self.rtl_center_frequency_hz() {
                Ok(frequency) => rtl_frequency = frequency,
                Err(error) => {
                    self.status = SourceStatus::Error(error);
                    return;
                }
            }
        }
        self.status = SourceStatus::Opening;
        // stop() joins the previous local worker before this flag is reused.
        self.running.store(true, Ordering::SeqCst);
        let running = self.running.clone();

        // Recreate channel if needed (after a previous stop)
        let tx = if let Some(tx) = self.tx.take() {
            tx
        } else {
            let (new_tx, new_rx) = bounded(32);
            self.rx = Some(new_rx);
            new_tx
        };

        #[allow(unused_variables)]
        let freq = rtl_frequency;
        let rate = self.sample_rate_hz;
        let _ppm = self.ppm_correction;
        let _bias = self.bias_tee;
        let _gain = self.gain_db;
        let _tuner_agc = self.tuner_agc;
        let _rtl_agc = self.rtl_agc;
        let _direct = self.direct_sampling_mode();
        let _offset_tuning = self.offset_tuning;
        let _rtl_device = self.rtl_device.clone();
        let source_mode = self.source_mode.clone();
        let replay_file = self.replay_file.clone();
        let replay_loop = self.replay_loop;
        let replay_speed = self.replay_speed;
        if self.source_mode == SourceMode::Replay {
            self.replay_position = 0;
            self.replay_size = replay_file
                .as_deref()
                .and_then(|path| std::fs::metadata(path).ok())
                .filter(|metadata| metadata.is_file())
                .map_or(0, |metadata| metadata.len());
        }

        let handle = std::thread::spawn(move || {
            match source_mode {
                SourceMode::Replay => {
                    let path = if let Some(p) = replay_file {
                        p
                    } else {
                        send_cancellable(
                            &tx,
                            &running,
                            SourceMessage::Error(
                                "Choose an IQ recording before starting file replay".to_string(),
                            ),
                        );
                        return;
                    };
                    let extension = std::path::Path::new(&path)
                        .extension()
                        .and_then(|extension| extension.to_str())
                        .unwrap_or_default()
                        .to_ascii_lowercase();
                    let is_cf32 = matches!(extension.as_str(), "cf32" | "fc32");
                    let sample_bytes = if is_cf32 { 8 } else { 2 };
                    match std::fs::metadata(&path) {
                        Ok(metadata)
                            if metadata.is_file()
                                && metadata.len() > 0
                                && metadata.len() % sample_bytes == 0 => {}
                        Ok(_) => {
                            send_cancellable(&tx, &running, SourceMessage::Error(
                                "Choose a nonempty regular IQ recording with complete I/Q samples".into(),
                            ));
                            return;
                        }
                        Err(error) => {
                            send_cancellable(
                                &tx,
                                &running,
                                SourceMessage::Error(format!(
                                    "Could not open IQ recording {path}: {error}"
                                )),
                            );
                            return;
                        }
                    }
                    let file = if let Ok(f) = std::fs::File::open(&path) {
                        std::io::BufReader::new(f)
                    } else {
                        send_cancellable(
                            &tx,
                            &running,
                            SourceMessage::Error(format!("Could not open IQ recording: {path}")),
                        );
                        return;
                    };
                    use std::io::Read;
                    let mut reader = file;
                    let buf_size = 65536;
                    let mut buf = vec![0u8; buf_size];
                    while running.load(Ordering::Acquire) {
                        match reader.read(&mut buf) {
                            Ok(0) => {
                                if replay_loop {
                                    let file2 = match std::fs::File::open(&path) {
                                        Ok(f) => std::io::BufReader::new(f),
                                        Err(error) => {
                                            send_cancellable(
                                                &tx,
                                                &running,
                                                SourceMessage::Error(format!(
                                                    "Could not reopen IQ recording {path}: {error}"
                                                )),
                                            );
                                            break;
                                        }
                                    };
                                    reader = file2;
                                    continue;
                                }
                                send_cancellable(&tx, &running, SourceMessage::EndOfStream);
                                break;
                            }
                            Ok(n) => {
                                let chunk = if is_cf32 {
                                    let mut out = Vec::with_capacity(n / 4);
                                    for chunk in buf[..n].chunks_exact(8) {
                                        let i = f32::from_le_bytes(chunk[0..4].try_into().unwrap());
                                        let q = f32::from_le_bytes(chunk[4..8].try_into().unwrap());
                                        let to_u8 = |value: f32| {
                                            if value.is_finite() {
                                                ((value.clamp(-1.0, 1.0) * 127.0) as i16 + 127)
                                                    as u8
                                            } else {
                                                127
                                            }
                                        };
                                        let i_u8 = to_u8(i);
                                        let q_u8 = to_u8(q);
                                        out.push(i_u8);
                                        out.push(q_u8);
                                    }
                                    out
                                } else {
                                    buf[..n].to_vec()
                                };

                                if !send_cancellable(&tx, &running, SourceMessage::Samples(chunk)) {
                                    break;
                                }
                                let num_samples = if is_cf32 { n / 8 } else { n / 2 };
                                let sleep_ms = (num_samples as f64 / f64::from(rate) * 1000.0
                                    / f64::from(replay_speed))
                                    as u64;
                                if !wait_cancellable(
                                    &running,
                                    Duration::from_millis(sleep_ms.max(1)),
                                ) {
                                    break;
                                }
                            }
                            Err(error) => {
                                send_cancellable(
                                    &tx,
                                    &running,
                                    SourceMessage::Error(format!(
                                        "Could not read IQ recording {path}: {error}"
                                    )),
                                );
                                break;
                            }
                        }
                    }
                }
                SourceMode::Hardware => {
                    #[cfg(all(feature = "rtlsdr", not(test)))]
                    {
                        // SAFETY: `rtl_sdr_open` is an `unsafe` FFI wrapper but
                        // passes valid arguments to the wrapped C functions and
                        // initialises the device handle on success.
                        let opened = unsafe {
                            rtl_sdr_open(
                                &_rtl_device,
                                freq,
                                rate,
                                _ppm,
                                _bias,
                                _gain,
                                _tuner_agc,
                                _rtl_agc,
                                _direct,
                                _offset_tuning,
                            )
                        };
                        let dev = match opened {
                            Ok(dev) => dev,
                            Err(error) => {
                                send_cancellable(&tx, &running, SourceMessage::Error(error));
                                return;
                            }
                        };
                        let mut buf = vec![0u8; 16384 * 2];
                        while running.load(Ordering::SeqCst) {
                            // SAFETY: `dev` is checked non-null; `buf` is a
                            // mutable Vec with a valid pointer and length.
                            let n = match unsafe { rtl_sdr_read_sync(dev, &mut buf) } {
                                Ok(n) => n,
                                Err(error) => {
                                    send_cancellable(&tx, &running, SourceMessage::Error(error));
                                    break;
                                }
                            };
                            if n > 0 {
                                if !send_live_samples(&tx, buf[..n].to_vec()) {
                                    break;
                                }
                            }
                        }
                        // SAFETY: `dev` is non-null and was opened above.
                        unsafe {
                            rtl_sdr_close(dev);
                        }
                    }
                    #[cfg(any(not(feature = "rtlsdr"), test))]
                    {
                        send_cancellable(&tx, &running, SourceMessage::Error(
                            "This build does not include RTL-SDR support. Use Demo, File Replay, or Daemon mode, or rebuild with --features rtlsdr."
                                .to_string(),
                        ));
                    }
                }
                SourceMode::Simulated => {
                    let mut phase: f64 = 0.0;
                    let mut burst_phase: f64 = 0.0;
                    let buf_size = 16384;
                    let mut buf = vec![0u8; buf_size];
                    let sample_rate_f = f64::from(rate);
                    let block_duration = iq_block_duration(buf_size, rate);

                    while running.load(Ordering::SeqCst) {
                        let block_started = Instant::now();

                        for i in (0..buf_size).step_by(2) {
                            let t = phase / sample_rate_f;
                            let noise_i = (rand_f64(phase * 137.1) * 6.0 - 3.0) as i16;
                            let noise_q = (rand_f64(phase * 251.7) * 6.0 - 3.0) as i16;

                            let fm_phase = 2.0 * std::f64::consts::PI * 200_000.0 * t;
                            let fm_i = (25.0 * fm_phase.cos()) as i16;
                            let fm_q = (25.0 * fm_phase.sin()) as i16;

                            let nbfm_phase = 2.0 * std::f64::consts::PI * -100_000.0 * t;
                            let nbfm_env = if (burst_phase * 0.5).sin() > 0.3 {
                                8.0
                            } else {
                                0.0
                            };
                            let nbfm_i = (nbfm_env * nbfm_phase.cos()) as i16;
                            let nbfm_q = (nbfm_env * nbfm_phase.sin()) as i16;

                            let am_phase = 2.0 * std::f64::consts::PI * 50_000.0 * t;
                            let am_env =
                                12.0 * (1.0 + 0.5 * (2.0 * std::f64::consts::PI * 440.0 * t).sin());
                            let am_i = (am_env * am_phase.cos()) as i16;
                            let am_q = (am_env * am_phase.sin()) as i16;

                            let pulse = if (burst_phase * 0.1).sin() > 0.95 {
                                40
                            } else {
                                0
                            };
                            let total_i = noise_i + fm_i + nbfm_i + am_i + pulse;
                            let total_q = noise_q + fm_q + nbfm_q + am_q;

                            buf[i] = (i32::from(total_i) + 127).clamp(0, 255) as u8;
                            buf[i + 1] = (i32::from(total_q) + 127).clamp(0, 255) as u8;

                            phase += 1.0;
                            burst_phase += 1.0;
                            if phase >= sample_rate_f * 10.0 {
                                phase -= sample_rate_f * 10.0;
                            }
                            if burst_phase >= 10000.0 {
                                burst_phase -= 10000.0;
                            }
                        }

                        if !send_live_samples(&tx, buf.clone()) {
                            break;
                        }
                        if !wait_cancellable(
                            &running,
                            block_duration.saturating_sub(block_started.elapsed()),
                        ) {
                            break;
                        }
                    }
                }
                SourceMode::Daemon => {
                    unreachable!(
                        "start() branches to start_daemon() before ever spawning this worker \
                         thread in Daemon mode"
                    )
                }
            }
        });
        self.worker_handle = Some(handle);
        self.stream_generation = self.stream_generation.wrapping_add(1);
        self.tx = None; // tx was moved into the worker thread
        self.status = SourceStatus::Running;
    }

    /// Starts `SourceMode::Daemon`: connects to `self.daemon_addr` in the background and
    /// subscribes to the daemon's wideband spectrum pipeline. Unlike the local worker
    /// threads above, no samples ever flow through `self.tx`/`self.rx` in this mode — events
    /// arrive via [`Self::recv_daemon_event`] instead, polled from `CentralApp`'s own loop.
    ///
    /// Spectrum is subscribed immediately. [`Self::sync_daemon_workflows`] adds or removes
    /// audio, ADS-B, and LRPT subscriptions as the corresponding desktop workflow changes.
    fn start_daemon(&mut self) {
        self.status = SourceStatus::Opening;
        let addr: std::net::SocketAddr = match self.daemon_addr.parse() {
            Ok(a) => a,
            Err(e) => {
                self.status = SourceStatus::Error(format!(
                    "invalid daemon address {:?}: {e}",
                    self.daemon_addr
                ));
                return;
            }
        };
        let client = crate::daemon_client::DaemonClient::connect(addr, "ez-gui".to_string());
        client.send(ez_proto::ClientCommand::Subscribe {
            channel: self.daemon_spectrum_spec(),
        });
        self.daemon_audio_mode = None;
        self.daemon_audio_tuning = None;
        self.daemon_adsb_subscribed = false;
        self.daemon_lrpt_subscribed = false;
        self.daemon_recording_active = false;
        self.daemon_client = Some(client);
        self.stream_generation = self.stream_generation.wrapping_add(1);
    }

    fn daemon_spectrum_spec(&self) -> ez_proto::ChannelSpec {
        ez_proto::ChannelSpec {
            id: DAEMON_SPECTRUM_CHANNEL_ID,
            center_offset_hz: 0,
            bandwidth_hz: self.sample_rate_hz,
            kind: ez_proto::PipelineKind::Spectrum,
            demod_mode: None,
        }
    }

    fn apply_daemon_hardware(&mut self, hardware: &ez_proto::HardwareStatus) {
        if self.center_frequency_hz.is_some() {
            self.center_frequency_hz = Some(hardware.frequency_hz);
        } else {
            self.frequency_hz = hardware.frequency_hz;
        }
        self.sample_rate_hz = hardware.sample_rate_hz;
        self.gain_db = hardware.gain_db;
        self.daemon_last_center_hz = hardware.frequency_hz;
        self.daemon_last_sample_rate_hz = hardware.sample_rate_hz;
        self.daemon_last_gain_db = hardware.gain_db;
    }

    /// Non-blocking pull of one event from the daemon connection (`SourceMode::Daemon` only;
    /// always `None` in other modes). Reflects connection status and daemon-reported hardware
    /// state into this manager's own `status`/`frequency_hz`/`sample_rate_hz`/`gain_db`
    /// fields — the daemon is authoritative for these in this mode, the same way a local
    /// worker thread is authoritative for them in the other modes.
    #[must_use]
    pub fn recv_daemon_event(&mut self) -> Option<ez_proto::ServerEvent> {
        let (status, event) = {
            let client = self.daemon_client.as_ref()?;
            (client.status(), client.try_recv_event())
        };
        self.status = match status {
            crate::daemon_client::ConnectionStatus::Connecting => SourceStatus::Opening,
            crate::daemon_client::ConnectionStatus::Connected => SourceStatus::Running,
            crate::daemon_client::ConnectionStatus::Disconnected => SourceStatus::Idle,
            crate::daemon_client::ConnectionStatus::Error(e) => SourceStatus::Error(e),
        };
        let event = event?;
        if let ez_proto::ServerEvent::Hardware(hw) = &event {
            self.apply_daemon_hardware(hw);
        } else if let ez_proto::ServerEvent::Recording(status) = &event {
            if status.channel_id == DAEMON_SPECTRUM_CHANNEL_ID {
                self.daemon_recording_active = status.active;
            }
        }
        Some(event)
    }

    pub fn start_daemon_recording(
        &mut self,
        format: ez_proto::RecordingFormat,
    ) -> Result<(), String> {
        if self.source_mode != SourceMode::Daemon {
            return Err("daemon recording is only available in Daemon source mode".to_string());
        }
        let client = self
            .daemon_client
            .as_ref()
            .ok_or_else(|| "connect to the daemon before recording".to_string())?;
        if client.status() != crate::daemon_client::ConnectionStatus::Connected {
            return Err("wait for the daemon connection before recording".to_string());
        }
        client.send(ez_proto::ClientCommand::StartRecording {
            channel_id: DAEMON_SPECTRUM_CHANNEL_ID,
            format,
        });
        Ok(())
    }

    pub fn stop_daemon_recording(&mut self) -> Result<(), String> {
        if self.source_mode != SourceMode::Daemon {
            return Err("daemon recording is only available in Daemon source mode".to_string());
        }
        let client = self
            .daemon_client
            .as_ref()
            .ok_or_else(|| "daemon is not connected".to_string())?;
        client.send(ez_proto::ClientCommand::StopRecording {
            channel_id: DAEMON_SPECTRUM_CHANNEL_ID,
        });
        Ok(())
    }

    /// Forwards local frequency/sample-rate/gain field changes to the daemon
    /// (`SourceMode::Daemon` only; a no-op otherwise) — call once per UI frame, separately
    /// from the per-event [`Self::recv_daemon_event`] drain loop, so a slider edit is sent
    /// exactly once even if several events are drained in the same frame.
    pub fn sync_daemon_controls(&mut self) {
        if self.daemon_client.is_none() {
            return;
        }
        let commands = self.daemon_control_commands();
        if let Some(client) = &self.daemon_client {
            for command in commands {
                client.send(command);
            }
        }
    }

    fn daemon_control_commands(&mut self) -> Vec<ez_proto::ClientCommand> {
        let mut commands = Vec::new();
        let capture_center = self.capture_center_frequency_hz();
        if capture_center != self.daemon_last_center_hz {
            commands.push(ez_proto::ClientCommand::SetFrequency { hz: capture_center });
            self.daemon_last_center_hz = capture_center;
        }
        if self.sample_rate_hz != self.daemon_last_sample_rate_hz {
            commands.push(ez_proto::ClientCommand::SetSampleRate {
                hz: self.sample_rate_hz,
            });
            self.daemon_last_sample_rate_hz = self.sample_rate_hz;
        }
        if (self.gain_db - self.daemon_last_gain_db).abs() > f64::EPSILON {
            commands.push(ez_proto::ClientCommand::SetGain { db: self.gain_db });
            self.daemon_last_gain_db = self.gain_db;
        }
        commands
    }

    /// Makes the daemon subscriptions match the desktop's currently active task. Spectrum is
    /// always present; audio, ADS-B, and LRPT are attached lazily and use stable ids so the
    /// operation is idempotent across UI frames.
    pub fn sync_daemon_workflows(
        &mut self,
        audio_running: bool,
        adsb_running: bool,
        meteor_lrpt: bool,
        demod_mode: crate::sdr_panel::DemodMode,
    ) {
        if self.daemon_client.is_none() {
            return;
        }
        let commands =
            self.daemon_workflow_commands(audio_running, adsb_running, meteor_lrpt, demod_mode);
        if let Some(client) = &self.daemon_client {
            for command in commands {
                client.send(command);
            }
        }
    }

    fn daemon_workflow_commands(
        &mut self,
        audio_running: bool,
        adsb_running: bool,
        meteor_lrpt: bool,
        demod_mode: crate::sdr_panel::DemodMode,
    ) -> Vec<ez_proto::ClientCommand> {
        let mut commands = Vec::new();

        let mut wanted_audio_mode = if audio_running && !adsb_running && !meteor_lrpt {
            match demod_mode.resolve(self.frequency_hz) {
                crate::sdr_panel::DemodMode::Raw | crate::sdr_panel::DemodMode::Auto => {
                    Some(ez_proto::DemodMode::Fm)
                }
                crate::sdr_panel::DemodMode::Am => Some(ez_proto::DemodMode::Am),
                crate::sdr_panel::DemodMode::Fm => Some(ez_proto::DemodMode::Fm),
                crate::sdr_panel::DemodMode::Wfm => Some(ez_proto::DemodMode::Wfm),
                crate::sdr_panel::DemodMode::Lsb => Some(ez_proto::DemodMode::Lsb),
                crate::sdr_panel::DemodMode::Usb => Some(ez_proto::DemodMode::Usb),
                crate::sdr_panel::DemodMode::Dsb | crate::sdr_panel::DemodMode::Cw => None,
            }
        } else {
            None
        };
        let wanted_audio_tuning = if wanted_audio_mode.is_some() {
            let offset =
                i128::from(self.frequency_hz) - i128::from(self.capture_center_frequency_hz());
            match i64::try_from(offset) {
                Ok(offset) => Some(DaemonAudioTuning {
                    frequency_hz: self.frequency_hz,
                    offset_hz: offset,
                    bandwidth_hz: 200_000.min(self.sample_rate_hz),
                }),
                Err(_) => {
                    self.status = SourceStatus::Error(
                        "Audio VFO is too far from the daemon capture center.".into(),
                    );
                    wanted_audio_mode = None;
                    None
                }
            }
        } else {
            None
        };
        match (self.daemon_audio_mode, wanted_audio_mode) {
            (None, Some(mode)) => commands.push(ez_proto::ClientCommand::Subscribe {
                channel: ez_proto::ChannelSpec {
                    id: DAEMON_AUDIO_CHANNEL_ID,
                    center_offset_hz: wanted_audio_tuning.map_or(0, |tuning| tuning.offset_hz),
                    bandwidth_hz: wanted_audio_tuning.map_or(0, |tuning| tuning.bandwidth_hz),
                    kind: ez_proto::PipelineKind::Audio,
                    demod_mode: Some(mode),
                },
            }),
            (Some(_), None) => {
                commands.push(ez_proto::ClientCommand::Unsubscribe {
                    channel_id: DAEMON_AUDIO_CHANNEL_ID,
                });
                commands.push(ez_proto::ClientCommand::RemoveChannel {
                    channel_id: DAEMON_AUDIO_CHANNEL_ID,
                });
            }
            (Some(current), Some(mode)) if current != mode => {
                commands.push(ez_proto::ClientCommand::SetDemodMode {
                    channel_id: DAEMON_AUDIO_CHANNEL_ID,
                    mode,
                });
            }
            _ => {}
        }
        if self.daemon_audio_mode.is_some()
            && wanted_audio_mode.is_some()
            && self.daemon_audio_tuning != wanted_audio_tuning
        {
            if let Some(tuning) = wanted_audio_tuning {
                commands.push(ez_proto::ClientCommand::Retune {
                    channel_id: DAEMON_AUDIO_CHANNEL_ID,
                    center_offset_hz: tuning.offset_hz,
                    bandwidth_hz: tuning.bandwidth_hz,
                });
            }
        }
        self.daemon_audio_mode = wanted_audio_mode;
        self.daemon_audio_tuning = wanted_audio_tuning;

        if adsb_running && !self.daemon_adsb_subscribed {
            commands.push(ez_proto::ClientCommand::Subscribe {
                channel: ez_proto::ChannelSpec {
                    id: DAEMON_ADSB_CHANNEL_ID,
                    center_offset_hz: 0,
                    bandwidth_hz: 2_400_000,
                    kind: ez_proto::PipelineKind::AdsbPackets,
                    demod_mode: None,
                },
            });
        } else if !adsb_running && self.daemon_adsb_subscribed {
            commands.push(ez_proto::ClientCommand::Unsubscribe {
                channel_id: DAEMON_ADSB_CHANNEL_ID,
            });
            commands.push(ez_proto::ClientCommand::RemoveChannel {
                channel_id: DAEMON_ADSB_CHANNEL_ID,
            });
        }
        self.daemon_adsb_subscribed = adsb_running;

        if meteor_lrpt && !self.daemon_lrpt_subscribed {
            commands.push(ez_proto::ClientCommand::Subscribe {
                channel: ez_proto::ChannelSpec {
                    id: DAEMON_LRPT_CHANNEL_ID,
                    center_offset_hz: 0,
                    bandwidth_hz: 288_000,
                    kind: ez_proto::PipelineKind::LrptTelemetry,
                    demod_mode: None,
                },
            });
        } else if !meteor_lrpt && self.daemon_lrpt_subscribed {
            commands.push(ez_proto::ClientCommand::Unsubscribe {
                channel_id: DAEMON_LRPT_CHANNEL_ID,
            });
            commands.push(ez_proto::ClientCommand::RemoveChannel {
                channel_id: DAEMON_LRPT_CHANNEL_ID,
            });
        }
        self.daemon_lrpt_subscribed = meteor_lrpt;
        commands
    }

    /// Stop the SDR source.
    ///
    /// Signals the worker thread to exit and recreates the sample channel for
    /// the next `start()` call. Status returns to `Idle`.
    pub fn stop(&mut self) {
        if self.daemon_recording_active {
            if let Some(client) = &self.daemon_client {
                client.send(ez_proto::ClientCommand::StopRecording {
                    channel_id: DAEMON_SPECTRUM_CHANNEL_ID,
                });
            }
        }
        // Dropping tears the connection down (signals the worker, sends Detach, joins it).
        // Safe to do unconditionally regardless of mode — `None` in every non-Daemon mode.
        self.daemon_client = None;
        self.daemon_audio_mode = None;
        self.daemon_audio_tuning = None;
        self.daemon_adsb_subscribed = false;
        self.daemon_lrpt_subscribed = false;
        self.daemon_recording_active = false;
        self.running.store(false, Ordering::SeqCst);
        // Wake paced replay/demo immediately; backpressure checks cancellation
        // every 20 ms. Join before start() reuses the running flag so old local
        // workers cannot leak across stop/start. RTL exits after its next USB read.
        if let Some(handle) = self.worker_handle.take() {
            handle.thread().unpark();
            let _ = handle.join();
        }
        // Recreate channel for next start()
        let (new_tx, new_rx) = bounded(32);
        self.tx = Some(new_tx);
        self.rx = Some(new_rx);
        self.status = SourceStatus::Idle;
    }

    /// Try to receive a pending chunk of IQ samples from the source thread.
    ///
    /// Returns `None` if no samples are available (non-blocking).
    pub fn recv_samples(&mut self) -> Option<Vec<u8>> {
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(SourceMessage::Samples(samples)) => {
                    if self.source_mode == SourceMode::Replay && self.replay_size > 0 {
                        self.replay_position =
                            (self.replay_position + samples.len() as u64).min(self.replay_size);
                    }
                    Some(samples)
                }
                Ok(SourceMessage::EndOfStream) => {
                    if self.source_mode == SourceMode::Replay {
                        self.replay_position = self.replay_size;
                    }
                    self.status = SourceStatus::Idle;
                    None
                }
                Ok(SourceMessage::Error(message)) => {
                    self.status = SourceStatus::Error(message);
                    None
                }
                Err(TryRecvError::Disconnected) => {
                    if matches!(self.status, SourceStatus::Running | SourceStatus::Opening) {
                        self.status = SourceStatus::Error(
                            "Receiver worker stopped unexpectedly; stop and restart the source"
                                .into(),
                        );
                    }
                    None
                }
                Err(TryRecvError::Empty) => None,
            }
        } else {
            None
        }
    }

    /// Render the egui-based source control panel (mode, frequency, gain, etc.).
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Source");

        // Source mode selection
        ui.horizontal(|ui| {
            ui.label("Mode:");
            if ui
                .selectable_label(self.source_mode == SourceMode::Simulated, "Demo")
                .on_hover_text("Generated signals for learning and testing without hardware.")
                .clicked()
            {
                self.source_mode = SourceMode::Simulated;
            }
            if cfg!(feature = "rtlsdr")
                && ui
                    .selectable_label(self.source_mode == SourceMode::Hardware, "RTL-SDR")
                    .on_hover_text("Receive live IQ from the selected RTL-SDR device.")
                    .clicked()
            {
                self.source_mode = SourceMode::Hardware;
            }
            if ui
                .selectable_label(self.source_mode == SourceMode::Replay, "File Replay")
                .clicked()
            {
                self.source_mode = SourceMode::Replay;
            }
            if ui
                .selectable_label(self.source_mode == SourceMode::Daemon, "Daemon")
                .on_hover_text("Attach to a running ez-daemon over the network instead of owning hardware locally.")
                .clicked()
            {
                self.source_mode = SourceMode::Daemon;
            }
        });

        if self.source_mode == SourceMode::Daemon {
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Daemon address:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.daemon_addr)
                        .desired_width(160.0)
                        .hint_text("127.0.0.1:7890"),
                );
            });
            ui.label(egui::RichText::new("Daemon mode provides spectrum, audio, aircraft, and LRPT streams; local-only tools may still be unavailable.").color(egui::Color32::YELLOW));
        }

        if self.source_mode == SourceMode::Replay {
            ui.separator();
            ui.horizontal(|ui| {
                let mut path = self.replay_file.clone().unwrap_or_default();
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut path)
                            .desired_width(300.0)
                            .hint_text("Path to .iq / .bin / .raw file"),
                    )
                    .changed()
                {
                    if path.is_empty() {
                        self.replay_file = None;
                    } else {
                        self.replay_file = Some(path);
                    }
                }
                if ui
                    .button("📂 Browse")
                    .on_hover_text("Open a file picker to select an IQ recording file.")
                    .clicked()
                {
                    if let Some(picked) = rfd::FileDialog::new()
                        .add_filter("IQ files", &["iq", "bin", "raw", "cs8", "cu8", "cf32"])
                        .add_filter("All files", &["*"])
                        .pick_file()
                    {
                        self.replay_file = picked.to_str().map(std::string::ToString::to_string);
                    }
                }
                ui.separator();
                ui.checkbox(&mut self.replay_loop, "Loop");
                ui.label("Speed:");
                ui.add(
                    egui::Slider::new(&mut self.replay_speed, 0.1..=10.0)
                        .text("x")
                        .logarithmic(true),
                );
            });
            if let Some(path) = &self.replay_file {
                ui.label(format!("File: {path}"));
                if self.replay_size > 0 {
                    let mb = self.replay_size as f64 / 1_048_576.0;
                    ui.label(format!("Size: {mb:.1} MB"));
                }
            }
        }

        ui.separator();
        ui.horizontal(|ui| {
            let (color, label) = match &self.status {
                SourceStatus::Running => (egui::Color32::GREEN, "Running"),
                SourceStatus::Idle => (egui::Color32::GRAY, "Idle"),
                SourceStatus::Opening => (egui::Color32::YELLOW, "Opening..."),
                SourceStatus::Error(e) => (egui::Color32::RED, e.as_str()),
            };
            ui.colored_label(color, format!("● {label}"));
            if self.replay_position > 0 && self.replay_size > 0 {
                let pct = self.replay_position as f64 / self.replay_size as f64 * 100.0;
                ui.separator();
                ui.label(format!("Pos: {pct:.1}%"));
            }
        });
        ui.add(
            egui::Slider::new(&mut self.frequency_hz, 500_000..=1_770_000_000)
                .text("Frequency (Hz)")
                .custom_formatter(|v, _| format!("{:.3} MHz", v / 1e6)),
        );
        ui.horizontal(|ui| {
            let mut freq_mhz = self.frequency_hz as f64 / 1e6;
            if ui
                .add(
                    egui::DragValue::new(&mut freq_mhz)
                        .speed(0.001)
                        .range(0.5..=1770.0)
                        .prefix("MHz "),
                )
                .changed()
            {
                self.frequency_hz = (freq_mhz * 1e6) as u64;
            }
            if ui.small_button("-1MHz").clicked() {
                self.frequency_hz = self.frequency_hz.saturating_sub(1_000_000).max(500_000);
            }
            if ui.small_button("-100k").clicked() {
                self.frequency_hz = self.frequency_hz.saturating_sub(100_000).max(500_000);
            }
            if ui.small_button("-10k").clicked() {
                self.frequency_hz = self.frequency_hz.saturating_sub(10_000).max(500_000);
            }
            if ui.small_button("+10k").clicked() {
                self.frequency_hz = self.frequency_hz.saturating_add(10_000).min(1_770_000_000);
            }
            if ui.small_button("+100k").clicked() {
                self.frequency_hz = self.frequency_hz.saturating_add(100_000).min(1_770_000_000);
            }
            if ui.small_button("+1MHz").clicked() {
                self.frequency_hz = self
                    .frequency_hz
                    .saturating_add(1_000_000)
                    .min(1_770_000_000);
            }
        });
        ui.add(
            egui::Slider::new(&mut self.sample_rate_hz, 225_001..=3_200_000)
                .text("Sample rate (Hz)")
                .custom_formatter(|v, _| format!("{:.3} MSps", v / 1e6)),
        );
        if self.source_mode != SourceMode::Replay {
            ui.horizontal(|ui| {
                ui.label("Gain:");
                ui.add(egui::Slider::new(&mut self.gain_db, 0.0..=49.6).step_by(0.1).text("dB").custom_formatter(|v, _| format!("{v:.1} dB")))
                    .on_hover_text("RF gain in dB. RTL-SDR range: 0–49.6 dB in 0.9 dB steps.");
                ui.horizontal(|ui| {
                    for (label, val, tip) in [
                        ("Auto", 0.0, "Automatic gain control (AGC). Good starting point but can overload with strong signals."),
                        ("Low", 15.0, "~15 dB — use near strong transmitters to avoid overload / intermodulation."),
                        ("Med", 30.0, "~30 dB — good general-purpose starting point for most setups."),
                        ("High", 40.0, "~40 dB — use for weak signals: satellites, distant stations. Watch for overload."),
                        ("Max", 49.6, "49.6 dB maximum gain. Only use with weak signals and quiet RF environment."),
                    ] {
                        if ui.small_button(label).on_hover_text(tip).clicked() {
                            self.gain_db = val;
                        }
                    }
                });
            });
        }
        // Bias-tee/direct-sampling/PPM are RTL-SDR FFI-specific hardware knobs with no wire
        // protocol representation (see `ez_proto::ClientCommand`) — showing them in Daemon
        // mode would imply they do something there when they silently wouldn't.
        if self.source_mode == SourceMode::Hardware {
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.bias_tee, "Bias Tee (4.5V)")
                    .on_hover_text("Sends 4.5V DC down the coax center pin to power a mast-mounted LNA or filtered LNA. RTL-SDR Blog V3 only. Do NOT enable with passive antennas — it can damage cheap dongles.");
                ui.checkbox(&mut self.direct_sampling, "Direct Sampling")
                    .on_hover_text("Bypasses the RTL-SDR tuner and feeds the ADC directly — enables HF reception below 24 MHz (typically 500 kHz–14 MHz). RTL-SDR V3 only. Reduces sensitivity significantly.");
            });
            ui.add(egui::Slider::new(&mut self.ppm_correction, -100..=100).text("PPM correction"))
                .on_hover_text("Frequency error correction in parts-per-million. RTL-SDR crystals typically drift ±20–50 PPM. At 1090 MHz, 10 PPM = 10.9 kHz error. Tune to a known frequency (FM station, GPS L1) and adjust until it aligns.");
        }
        ui.horizontal(|ui| {
            if ui.button("Start").clicked() {
                self.start();
            }
            if ui.button("Stop").clicked() {
                self.stop();
            }
        });
        if self.temperature > 0.0 {
            ui.label(format!("Temperature: {:.1}°C", self.temperature));
        }
    }
}

/// Simple deterministic pseudo-random (LCG, no `sin()` — which gets slow for large values)
fn rand_f64(seed: f64) -> f64 {
    let x = seed * 1664525.0 + 1013904223.0;
    let frac = x - (x * (1.0 / 4294967296.0)).floor() * 4294967296.0;
    frac / 4294967296.0
}

// SAFETY: This function calls raw FFI (`rtlsdr_open`, etc.) and must only be
// called when `feature = "rtlsdr"` is active and a real RTL-SDR device is
// available. The caller must ensure the returned pointer is eventually closed
// with `rtl_sdr_close`.
#[cfg(all(feature = "rtlsdr", not(test)))]
unsafe fn rtl_sdr_open(
    selection: &RtlDeviceSelection,
    frequency: u32,
    rate: u32,
    ppm: i32,
    bias: bool,
    gain_db: f64,
    tuner_agc: bool,
    rtl_agc: bool,
    direct_sampling_mode: i32,
    offset_tuning: bool,
) -> Result<*mut std::ffi::c_void, String> {
    validate_rtl_options(direct_sampling_mode != 0, offset_tuning)?;
    let devices = enumerate_rtl_devices()?;
    let index = resolve_rtl_device(selection, &devices)?;
    extern "C" {
        // Older rtlsdr_sys lacks this librtlsdr extension; using the crate's
        // bindings for all other calls also retains its system-library link.
        fn rtlsdr_set_bias_tee(dev: *mut std::ffi::c_void, on: i32) -> i32;
    }
    let mut dev: *mut std::ffi::c_void = std::ptr::null_mut();
    let opened = unsafe { rtlsdr_sys::rtlsdr_open(&mut dev, index) };
    if opened != 0 || dev.is_null() {
        return Err(format!("Could not open RTL-SDR device {index} (USB error {opened}). Check USB access and whether another program is using it."));
    }
    let cleanup_and_fail = |what: &str| -> Result<*mut std::ffi::c_void, String> {
        unsafe {
            rtl_sdr_close(dev);
        }
        Err(format!(
            "Could not configure RTL-SDR device {index}: {what}."
        ))
    };
    if let Some(serial) = selection
        .serial
        .as_deref()
        .filter(|serial| !serial.is_empty())
    {
        let mut actual = [0; 256];
        let read = unsafe {
            rtlsdr_sys::rtlsdr_get_usb_strings(
                dev,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                actual.as_mut_ptr(),
            )
        };
        let bytes: Vec<u8> = actual
            .iter()
            .take_while(|&&byte| byte != 0)
            .map(|&byte| byte as u8)
            .collect();
        if read != 0 || String::from_utf8_lossy(&bytes).trim() != serial {
            return cleanup_and_fail("USB identity changed while opening; refresh the device list");
        }
    }
    if unsafe { rtlsdr_sys::rtlsdr_set_sample_rate(dev, rate) } < 0 {
        return cleanup_and_fail("sample rate");
    }
    // Configure the receive path before setting frequency, especially for HF
    // where tuning through the normal tuner would fail before direct sampling.
    if unsafe { rtlsdr_sys::rtlsdr_set_direct_sampling(dev, direct_sampling_mode) } < 0 {
        return cleanup_and_fail("direct sampling is unsupported");
    }
    // R820T/R828D tuners reject offset tuning (already use a low IF). A newly
    // opened handle defaults to off, so only request it when explicitly enabled.
    if offset_tuning && unsafe { rtlsdr_sys::rtlsdr_set_offset_tuning(dev, 1) } < 0 {
        return cleanup_and_fail(
            "offset tuning is unsupported by this tuner; disable Offset tuning",
        );
    }
    if unsafe { rtlsdr_sys::rtlsdr_set_center_freq(dev, frequency) } < 0 {
        return cleanup_and_fail("center frequency");
    }
    // Tuner AGC (auto gain) vs manual gain.
    if tuner_agc {
        if unsafe { rtlsdr_sys::rtlsdr_set_tuner_gain_mode(dev, 0) } < 0 {
            return cleanup_and_fail("tuner gain mode");
        }
    } else {
        if unsafe { rtlsdr_sys::rtlsdr_set_tuner_gain_mode(dev, 1) } < 0 {
            return cleanup_and_fail("tuner gain mode");
        }
        if unsafe { rtlsdr_sys::rtlsdr_set_tuner_gain(dev, (gain_db * 10.0) as i32) } < 0 {
            return cleanup_and_fail("tuner gain");
        }
    }
    if unsafe { rtlsdr_sys::rtlsdr_set_agc_mode(dev, i32::from(rtl_agc)) } < 0 {
        eprintln!("rtlsdr: warning: failed to set RTL AGC mode");
    }
    if unsafe { rtlsdr_sys::rtlsdr_set_freq_correction(dev, ppm) } < 0 {
        eprintln!("rtlsdr: warning: failed to set frequency correction");
    }
    let bias_on = if bias { 1 } else { 0 };
    if unsafe { rtlsdr_set_bias_tee(dev, bias_on) } < 0 {
        eprintln!("rtlsdr: warning: failed to set bias tee");
    }
    if unsafe { rtlsdr_sys::rtlsdr_reset_buffer(dev) } < 0 {
        return cleanup_and_fail("USB sample buffer reset");
    }
    Ok(dev)
}

// SAFETY: `dev` must be a valid device handle from `rtl_sdr_open`. `buf` must
// be a valid mutable slice. The FFI writes `n_read` bytes into the buffer.
#[cfg(all(feature = "rtlsdr", not(test)))]
unsafe fn rtl_sdr_read_sync(dev: *mut std::ffi::c_void, buf: &mut [u8]) -> Result<usize, String> {
    let mut n_read = 0;
    let result = unsafe {
        rtlsdr_sys::rtlsdr_read_sync(dev, buf.as_mut_ptr().cast(), buf.len() as i32, &mut n_read)
    };
    if result < 0 {
        return Err(format!(
            "RTL-SDR USB read failed ({result}); reconnect the receiver and restart."
        ));
    }
    if n_read <= 0 || n_read as usize > buf.len() || n_read % 2 != 0 {
        return Err("RTL-SDR returned an empty or incomplete I/Q block.".into());
    }
    Ok(n_read as usize)
}

// SAFETY: `dev` must be a non-null handle from `rtl_sdr_open` that has not
// been closed yet. After this call the handle is invalid.
#[cfg(all(feature = "rtlsdr", not(test)))]
unsafe fn rtl_sdr_close(dev: *mut std::ffi::c_void) {
    unsafe {
        rtlsdr_sys::rtlsdr_close(dev);
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_center_follows_vfo_by_default_and_can_stay_independent() {
        let mut source = SourceManager::new();
        source.frequency_hz = 118_500_000;
        assert_eq!(source.capture_center_frequency_hz(), 118_500_000);
        assert_eq!(source.rtl_center_frequency_hz().unwrap(), 118_500_000);
        source.center_frequency_hz = Some(118_000_000);
        assert_eq!(source.capture_center_frequency_hz(), 118_000_000);
        assert_eq!(source.rtl_center_frequency_hz().unwrap(), 118_000_000);
        source.frequency_hz = 118_525_000;
        assert_eq!(source.rtl_center_frequency_hz().unwrap(), 118_000_000);
        assert_eq!(source.frequency_hz, 118_525_000);
        source.center_frequency_hz = None;
        assert_eq!(source.capture_center_frequency_hz(), 118_525_000);
    }

    #[test]
    fn acquisition_retune_replaces_independent_center_and_restarts_worker() {
        let mut source = SourceManager::new();
        source.frequency_hz = 118_500_000;
        source.center_frequency_hz = Some(118_000_000);
        source.start();
        let previous_worker = source.worker_handle.as_ref().unwrap().thread().id();

        source.tune_and_restart(1_090_000_000, 2_000_000);

        assert_eq!(source.frequency_hz, 1_090_000_000);
        assert_eq!(source.center_frequency_hz, Some(1_090_000_000));
        assert_eq!(source.capture_center_frequency_hz(), 1_090_000_000);
        assert_eq!(source.sample_rate_hz, 2_000_000);
        assert_ne!(
            source.worker_handle.as_ref().unwrap().thread().id(),
            previous_worker
        );
        assert!(wait_until(|| source.recv_samples().is_some()));
        source.stop();
    }

    #[test]
    fn transverter_offset_handles_high_rf_and_all_arithmetic_boundaries() {
        let mut source = SourceManager::new();
        source.frequency_hz = 10_368_200_000;
        source.center_frequency_hz = Some(10_368_000_000);
        source.frequency_offset_hz = -10_224_000_000;
        assert_eq!(source.rtl_center_frequency_hz().unwrap(), 144_000_000);
        assert_eq!(source.frequency_hz, 10_368_200_000);
        assert_eq!(source.capture_center_frequency_hz(), 10_368_000_000);
        assert_eq!(
            checked_tuner_center(u64::from(u32::MAX), 0).unwrap(),
            u32::MAX
        );
        assert_eq!(checked_tuner_center(100, -100).unwrap(), 0);
        assert_eq!(checked_tuner_center(100, 25).unwrap(), 125);
        assert!(checked_tuner_center(u64::from(u32::MAX), 1).is_err());
        assert!(checked_tuner_center(100, -101).is_err());
        assert!(checked_tuner_center(u64::MAX, 1).is_err());
        assert!(checked_tuner_center(1, i64::MIN).is_err());
        assert_eq!(checked_tuner_center(1u64 << 63, i64::MIN).unwrap(), 0);
    }

    #[test]
    fn invalid_physical_center_fails_before_hardware_worker_starts() {
        let mut source = SourceManager::new();
        source.source_mode = SourceMode::Hardware;
        source.frequency_hz = u64::MAX;
        source.frequency_offset_hz = 1;
        source.start();
        assert!(
            matches!(&source.status, SourceStatus::Error(error) if error.contains("frequency offset"))
        );
        assert!(source.worker_handle.is_none());
        assert!(!source.running.load(Ordering::Acquire));
    }

    #[test]
    fn local_frequency_offset_does_not_affect_demo_or_replay() {
        let mut source = SourceManager::new();
        source.frequency_offset_hz = i64::MIN;
        source.start();
        assert!(wait_until(|| source.recv_samples().is_some()));
        source.stop();
        let file = TestRecording::new(&[12, 34, 56, 78]);
        let mut replay = file.source(2);
        replay.frequency_offset_hz = i64::MIN;
        replay.start();
        let mut received = None;
        assert!(wait_until(|| {
            received = replay.recv_samples();
            received.is_some()
        }));
        replay.stop();
        assert_eq!(received, Some(vec![12, 34, 56, 78]));
    }

    #[test]
    fn direct_sampling_keeps_enable_boolean_and_defaults_to_q_branch() {
        let mut source = SourceManager::new();
        assert_eq!(source.direct_sampling_branch, DirectSamplingBranch::Q);
        assert_eq!(source.direct_sampling_mode(), 0);
        source.direct_sampling = true;
        assert_eq!(source.direct_sampling_mode(), 2);
        source.direct_sampling_branch = DirectSamplingBranch::I;
        assert_eq!(source.direct_sampling_mode(), 1);
        source.direct_sampling = false;
        assert_eq!(source.direct_sampling_mode(), 0);
        for branch in [DirectSamplingBranch::I, DirectSamplingBranch::Q] {
            let decoded: DirectSamplingBranch =
                serde_json::from_str(&serde_json::to_string(&branch).unwrap()).unwrap();
            assert_eq!(decoded, branch);
        }
    }

    #[test]
    fn daemon_hardware_controls_use_capture_center_and_ignore_local_offset() {
        let mut source = SourceManager::new();
        source.frequency_hz = 118_500_000;
        source.center_frequency_hz = Some(118_000_000);
        source.frequency_offset_hz = -100_000_000;
        let commands = source.daemon_control_commands();
        assert_eq!(
            commands,
            vec![ez_proto::ClientCommand::SetFrequency { hz: 118_000_000 }]
        );
        source.frequency_hz += 25_000;
        assert!(
            source.daemon_control_commands().is_empty(),
            "VFO-only tuning must not retune capture hardware"
        );
        source.center_frequency_hz = Some(119_000_000);
        assert_eq!(
            source.daemon_control_commands(),
            vec![ez_proto::ClientCommand::SetFrequency { hz: 119_000_000 }]
        );
        let spectrum = source.daemon_spectrum_spec();
        assert_eq!(spectrum.center_offset_hz, 0);
        assert_eq!(spectrum.bandwidth_hz, source.sample_rate_hz);
        source.center_frequency_hz = None;
        assert_eq!(
            source.daemon_control_commands(),
            vec![ez_proto::ClientCommand::SetFrequency {
                hz: source.frequency_hz
            }]
        );
    }

    #[test]
    fn daemon_audio_subscribes_and_retunes_vfo_without_moving_capture() {
        let mut source = SourceManager::new();
        source.frequency_hz = 118_500_000;
        source.center_frequency_hz = Some(118_000_000);
        let commands =
            source.daemon_workflow_commands(true, false, false, crate::sdr_panel::DemodMode::Am);
        assert_eq!(
            commands,
            vec![ez_proto::ClientCommand::Subscribe {
                channel: ez_proto::ChannelSpec {
                    id: DAEMON_AUDIO_CHANNEL_ID,
                    center_offset_hz: 500_000,
                    bandwidth_hz: 200_000,
                    kind: ez_proto::PipelineKind::Audio,
                    demod_mode: Some(ez_proto::DemodMode::Am),
                }
            }]
        );
        assert!(source
            .daemon_workflow_commands(true, false, false, crate::sdr_panel::DemodMode::Am)
            .is_empty());
        source.frequency_hz = 117_950_000;
        assert_eq!(
            source.daemon_workflow_commands(true, false, false, crate::sdr_panel::DemodMode::Am),
            vec![ez_proto::ClientCommand::Retune {
                channel_id: DAEMON_AUDIO_CHANNEL_ID,
                center_offset_hz: -50_000,
                bandwidth_hz: 200_000,
            }]
        );
        assert_eq!(source.capture_center_frequency_hz(), 118_000_000);
        source.center_frequency_hz = Some(117_500_000);
        assert_eq!(
            source.daemon_workflow_commands(true, false, false, crate::sdr_panel::DemodMode::Am),
            vec![ez_proto::ClientCommand::Retune {
                channel_id: DAEMON_AUDIO_CHANNEL_ID,
                center_offset_hz: 450_000,
                bandwidth_hz: 200_000,
            }]
        );
    }

    #[test]
    fn daemon_center_following_retunes_absolute_audio_even_when_offset_stays_zero() {
        let mut source = SourceManager::new();
        source.daemon_workflow_commands(true, false, false, crate::sdr_panel::DemodMode::Fm);
        source.frequency_hz += 25_000;
        assert_eq!(
            source.daemon_workflow_commands(true, false, false, crate::sdr_panel::DemodMode::Fm),
            vec![ez_proto::ClientCommand::Retune {
                channel_id: DAEMON_AUDIO_CHANNEL_ID,
                center_offset_hz: 0,
                bandwidth_hz: 200_000,
            }]
        );
    }

    #[test]
    fn daemon_hardware_updates_preserve_independent_vfo_without_control_echo() {
        let hardware = ez_proto::HardwareStatus {
            connected: true,
            source_kind: "RTL-SDR".into(),
            frequency_hz: 118_000_000,
            sample_rate_hz: 2_048_000,
            gain_db: 40.0,
            error: None,
        };
        let mut source = SourceManager::new();
        source.frequency_hz = 118_500_000;
        source.center_frequency_hz = Some(117_000_000);
        source.apply_daemon_hardware(&hardware);
        assert_eq!(source.frequency_hz, 118_500_000);
        assert_eq!(source.capture_center_frequency_hz(), 118_000_000);
        assert!(source.daemon_control_commands().is_empty());
        source.center_frequency_hz = None;
        source.apply_daemon_hardware(&hardware);
        assert_eq!(source.frequency_hz, 118_000_000);
        assert!(source.daemon_control_commands().is_empty());
    }

    #[test]
    fn daemon_audio_overflow_unsubscribes_instead_of_wrapping_vfo_offset() {
        let mut source = SourceManager::new();
        source.daemon_workflow_commands(true, false, false, crate::sdr_panel::DemodMode::Am);
        source.center_frequency_hz = Some(0);
        source.frequency_hz = u64::MAX;
        assert_eq!(
            source.daemon_workflow_commands(true, false, false, crate::sdr_panel::DemodMode::Am),
            vec![
                ez_proto::ClientCommand::Unsubscribe {
                    channel_id: DAEMON_AUDIO_CHANNEL_ID
                },
                ez_proto::ClientCommand::RemoveChannel {
                    channel_id: DAEMON_AUDIO_CHANNEL_ID
                },
            ]
        );
        assert!(matches!(source.status, SourceStatus::Error(_)));
    }

    fn rtl_device(index: u32, serial: Option<&str>) -> RtlDeviceInfo {
        RtlDeviceInfo {
            index,
            name: "RTL2838".into(),
            manufacturer: Some("Test manufacturer".into()),
            product: Some("Test receiver".into()),
            serial: serial.map(str::to_string),
        }
    }

    #[test]
    fn demo_pacing_counts_iq_pairs_and_keeps_submillisecond_precision() {
        assert_eq!(
            iq_block_duration(16_384, 2_048_000),
            Duration::from_millis(4)
        );
        assert_eq!(
            iq_block_duration(16_384, 3_000_000),
            Duration::from_nanos(2_730_667)
        );
        assert_eq!(
            iq_block_duration(2_000_000, 1_000_000),
            Duration::from_secs(1)
        );
    }

    #[test]
    fn rtl_selection_follows_unique_serial_across_usb_index_reordering() {
        let mut source = SourceManager::new();
        source.rtl_devices = vec![rtl_device(0, Some("first")), rtl_device(1, Some("wanted"))];
        source.select_rtl_device(1).unwrap();
        assert_eq!(source.rtl_device.serial.as_deref(), Some("wanted"));
        assert_eq!(source.selected_rtl_device().unwrap().index, 1);
        let reordered = vec![rtl_device(0, Some("wanted")), rtl_device(1, Some("first"))];
        assert!(source.refresh_rtl_devices_with(move || Ok(reordered)));
        assert!(wait_until(|| source.poll_rtl_devices()));
        assert_eq!(source.rtl_device.index, 0);
        assert_eq!(source.rtl_device.serial.as_deref(), Some("wanted"));
        assert_eq!(
            source.selected_rtl_device().unwrap().serial.as_deref(),
            Some("wanted")
        );
        assert!(source.rtl_device_refresh_error.is_none());
    }

    #[test]
    fn rtl_selection_refuses_missing_or_ambiguous_serial_instead_of_wrong_receiver() {
        let selection = RtlDeviceSelection {
            index: 0,
            serial: Some("wanted".into()),
        };
        assert!(
            resolve_rtl_device(&selection, &[rtl_device(0, Some("replacement"))])
                .unwrap_err()
                .contains("not connected")
        );
        assert!(resolve_rtl_device(
            &selection,
            &[rtl_device(0, Some("wanted")), rtl_device(1, Some("wanted"))]
        )
        .unwrap_err()
        .contains("More than one"));
    }

    #[test]
    fn rtl_selection_validates_index_and_handles_serialless_or_duplicate_devices() {
        let mut source = SourceManager::new();
        source.rtl_devices = vec![
            rtl_device(0, None),
            rtl_device(1, Some("duplicate")),
            rtl_device(2, Some("duplicate")),
        ];
        for index in 0..=2 {
            source.select_rtl_device(index).unwrap();
            assert_eq!(source.rtl_device.index, index);
            assert_eq!(source.rtl_device.serial, None);
            assert_eq!(source.selected_rtl_device().unwrap().index, index);
        }
        let previous = source.rtl_device.clone();
        assert!(source.select_rtl_device(3).is_err());
        assert_eq!(source.rtl_device, previous);
        assert!(resolve_rtl_device(&previous, &[]).is_err());
    }

    #[test]
    fn rtl_refresh_is_single_flight_and_does_not_stop_the_active_source() {
        let mut source = SourceManager::new();
        source.start();
        let ui_thread = std::thread::current().id();
        let (release, wait) = bounded(1);
        assert!(source.refresh_rtl_devices_with(move || {
            assert_ne!(std::thread::current().id(), ui_thread);
            wait.recv_timeout(Duration::from_secs(3)).unwrap();
            Ok(vec![rtl_device(0, Some("receiver"))])
        }));
        assert!(source.rtl_devices_refreshing());
        assert!(!source.refresh_rtl_devices_with(|| panic!("overlapping discovery must not run")));
        assert!(source.select_rtl_device(0).is_err());
        assert!(!source.poll_rtl_devices());
        assert!(
            wait_until(|| source.recv_samples().is_some()),
            "UI should still receive samples while USB discovery waits"
        );
        release.send(()).unwrap();
        assert!(wait_until(|| source.poll_rtl_devices()));
        assert!(!source.rtl_devices_refreshing());
        assert_eq!(source.status, SourceStatus::Running);
        source.stop();
    }

    #[test]
    fn rtl_refresh_failure_preserves_selection_and_previous_snapshot() {
        let mut source = SourceManager::new();
        source.rtl_devices = vec![rtl_device(0, Some("known"))];
        source.select_rtl_device(0).unwrap();
        let selected = source.rtl_device.clone();
        assert!(source.refresh_rtl_devices_with(|| Err("USB unavailable".into())));
        assert!(wait_until(|| source.poll_rtl_devices()));
        assert_eq!(
            source.rtl_device_refresh_error.as_deref(),
            Some("USB unavailable")
        );
        assert_eq!(source.rtl_device, selected);
        assert_eq!(source.rtl_devices, vec![rtl_device(0, Some("known"))]);

        let (sender, receiver) = bounded(1);
        source.rtl_device_refresh = Some(receiver);
        drop(sender);
        assert!(source.poll_rtl_devices());
        assert!(!source.rtl_devices_refreshing());
        assert!(source
            .rtl_device_refresh_error
            .as_deref()
            .unwrap()
            .contains("stopped unexpectedly"));
        assert_eq!(source.rtl_device, selected);
    }

    #[test]
    fn dropping_source_does_not_wait_for_usb_discovery() {
        let mut source = SourceManager::new();
        let (release, wait) = bounded(1);
        let (finished, completion) = bounded(1);
        source.refresh_rtl_devices_with(move || {
            wait.recv_timeout(Duration::from_secs(3)).unwrap();
            finished.send(()).unwrap();
            Ok(vec![])
        });
        let started = Instant::now();
        drop(source);
        assert!(started.elapsed() < Duration::from_millis(250));
        release.send(()).unwrap();
        completion.recv_timeout(Duration::from_secs(3)).unwrap();
    }

    #[test]
    fn offset_tuning_rejects_direct_sampling_without_spawning_worker() {
        assert!(validate_rtl_options(false, true).is_ok());
        assert!(validate_rtl_options(true, false).is_ok());
        let mut source = SourceManager::new();
        source.source_mode = SourceMode::Hardware;
        source.direct_sampling = true;
        source.offset_tuning = true;
        source.start();
        assert!(
            matches!(&source.status, SourceStatus::Error(error) if error.contains("direct-sampling"))
        );
        assert!(source.worker_handle.is_none());
        assert!(!source.running.load(Ordering::Acquire));
    }

    #[test]
    fn selected_rtl_identity_and_offset_survive_source_restart_and_serde() {
        let mut source = SourceManager::new();
        source.rtl_devices = vec![rtl_device(2, Some("persist-me"))];
        source.select_rtl_device(2).unwrap();
        source.offset_tuning = true;
        source.start();
        source.stop();
        source.start();
        source.stop();
        assert!(source.offset_tuning);
        assert_eq!(source.rtl_device.index, 2);
        assert_eq!(source.rtl_device.serial.as_deref(), Some("persist-me"));
        let restored: RtlDeviceSelection =
            serde_json::from_str(&serde_json::to_string(&source.rtl_device).unwrap()).unwrap();
        assert_eq!(restored, source.rtl_device);
        let partial: RtlDeviceSelection = serde_json::from_str(r#"{"index":2}"#).unwrap();
        assert_eq!(
            partial,
            RtlDeviceSelection {
                index: 2,
                serial: None
            }
        );
    }

    struct TestRecording(std::path::PathBuf);

    impl TestRecording {
        fn new(bytes: &[u8]) -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "ez-sdr-source-{}-{}.cu8",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::write(&path, bytes).unwrap();
            Self(path)
        }

        fn source(&self, capacity: usize) -> SourceManager {
            let mut source = SourceManager::new();
            source.source_mode = SourceMode::Replay;
            source.replay_file = Some(self.0.to_string_lossy().into_owned());
            source.sample_rate_hz = u32::MAX;
            let (tx, rx) = bounded(capacity);
            source.tx = Some(tx);
            source.rx = Some(rx);
            source
        }
    }

    impl Drop for TestRecording {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn wait_until(mut predicate: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if predicate() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        false
    }

    #[test]
    fn cf32_replay_sanitizes_nonfinite_and_extreme_values_without_worker_panic() {
        let values = [
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
            0.0,
            f32::MAX,
            -f32::MAX,
            0.5,
            -0.5,
        ];
        let bytes: Vec<u8> = values.into_iter().flat_map(f32::to_le_bytes).collect();
        let mut recording = TestRecording::new(&bytes);
        let path = recording.0.with_extension("cf32");
        std::fs::rename(&recording.0, &path).unwrap();
        recording.0 = path;
        let mut source = recording.source(2);
        source.replay_loop = false;
        source.start();
        let mut output = Vec::new();
        assert!(
            wait_until(|| {
                if let Some(samples) = source.recv_samples() {
                    output.extend(samples);
                }
                source.status == SourceStatus::Idle
            }),
            "{:?}",
            source.status
        );
        assert_eq!(output, [127, 127, 127, 127, 254, 0, 190, 64]);
        source.stop();
    }

    #[test]
    fn live_queue_full_discards_a_block_without_disconnect() {
        let (tx, rx) = bounded(1);
        assert!(send_live_samples(&tx, vec![1, 2]));
        assert!(send_live_samples(&tx, vec![3, 4]));
        assert!(matches!(rx.try_recv(), Ok(SourceMessage::Samples(samples)) if samples == [1, 2]));
        assert!(send_live_samples(&tx, vec![5, 6]));
        drop(rx);
        assert!(!send_live_samples(&tx, vec![7, 8]));
    }

    #[test]
    fn demo_recovers_after_ui_stall_saturates_sample_queue() {
        let mut source = SourceManager::new();
        let (tx, rx) = bounded(2);
        source.tx = Some(tx);
        source.rx = Some(rx);
        source.start();
        let filled = wait_until(|| source.rx.as_ref().unwrap().is_full());
        std::thread::sleep(Duration::from_millis(60));
        while source.recv_samples().is_some() {}
        let recovered = wait_until(|| source.recv_samples().is_some());
        let status = source.status.clone();
        source.stop();
        assert!(filled, "demo did not fill the deliberately small queue");
        assert!(recovered, "demo died when the UI stopped draining samples");
        assert_eq!(status, SourceStatus::Running);
    }

    #[test]
    fn replay_backpressure_preserves_every_byte_and_delivers_eof() {
        let expected: Vec<_> = (0..36 * 65_536).map(|i| (i % 251) as u8).collect();
        let file = TestRecording::new(&expected);
        let mut source = file.source(2);
        source.start();
        let filled = wait_until(|| source.rx.as_ref().unwrap().is_full());
        std::thread::sleep(Duration::from_millis(60));
        let mut actual = Vec::new();
        let completed = wait_until(|| {
            while let Some(samples) = source.recv_samples() {
                actual.extend(samples);
            }
            source.status != SourceStatus::Running
        });
        let status = source.status.clone();
        source.stop();
        assert!(filled, "replay did not reach backpressure");
        assert!(completed, "replay never delivered its terminal state");
        assert_eq!(status, SourceStatus::Idle);
        assert_eq!(
            actual, expected,
            "backpressure lost or reordered recording data"
        );
    }

    #[test]
    fn replay_stop_cancels_full_sample_and_eof_sends() {
        for chunks in [1, 3] {
            let file = TestRecording::new(&vec![127; chunks * 65_536]);
            let mut source = file.source(1);
            source.start();
            let filled = wait_until(|| source.rx.as_ref().unwrap().is_full());
            // One chunk leaves EOF blocked; three leave the next sample blocked.
            std::thread::sleep(Duration::from_millis(60));
            let started = Instant::now();
            source.stop();
            assert!(filled);
            assert!(started.elapsed() < Duration::from_millis(250));
            assert_eq!(source.status, SourceStatus::Idle);
            assert!(source.worker_handle.is_none());
        }
    }

    #[test]
    fn replay_stop_interrupts_slow_playback_pacing() {
        let file = TestRecording::new(&vec![127; 65_536]);
        let mut source = file.source(1);
        source.sample_rate_hz = 1;
        source.replay_speed = 0.1;
        source.start();
        let filled = wait_until(|| source.rx.as_ref().unwrap().is_full());
        std::thread::sleep(Duration::from_millis(10));
        let started = Instant::now();
        source.stop();
        assert!(filled);
        assert!(started.elapsed() < Duration::from_millis(250));
    }

    #[test]
    fn empty_looped_replay_fails_instead_of_spinning() {
        let file = TestRecording::new(&[]);
        let mut source = file.source(1);
        source.replay_loop = true;
        source.start();
        let reported = wait_until(|| {
            source.recv_samples();
            matches!(source.status, SourceStatus::Error(_))
        });
        source.stop();
        assert!(reported);
    }

    #[test]
    fn disconnected_local_worker_cannot_stay_running() {
        let mut source = SourceManager::new();
        source.status = SourceStatus::Running;
        source.tx.take();
        assert!(source.recv_samples().is_none());
        assert!(matches!(source.status, SourceStatus::Error(_)));
    }

    #[test]
    fn source_manager_default_new() {
        let sm = SourceManager::new();
        assert_eq!(sm.status, SourceStatus::Idle);
        assert_eq!(sm.frequency_hz, 109_000_000);
        assert_eq!(sm.sample_rate_hz, 2_048_000);
        assert_eq!(sm.gain_db, 40.0);
        assert!(!sm.bias_tee);
        assert_eq!(sm.ppm_correction, 0);
        assert!(!sm.direct_sampling);
        assert_eq!(sm.temperature, 0.0);
        assert_eq!(sm.source_mode, SourceMode::Simulated);
        assert!(sm.replay_file.is_none());
        assert!(!sm.replay_loop);
        assert!((sm.replay_speed - 1.0).abs() < f32::EPSILON);
        assert_eq!(sm.replay_position, 0);
        assert_eq!(sm.replay_size, 0);
        assert_eq!(sm.daemon_addr, "127.0.0.1:7890");
        assert!(sm.daemon_client.is_none());
        assert!(sm.tx.is_some());
        assert!(sm.rx.is_some());
        assert!(!sm.running.load(Ordering::SeqCst));
        assert!(sm.worker_handle.is_none());
    }

    #[test]
    fn source_mode_default_is_simulated() {
        assert_eq!(SourceMode::default(), SourceMode::Simulated);
    }

    #[test]
    fn source_mode_debug_clone_partial_eq() {
        let a = SourceMode::Simulated;
        let b = SourceMode::Replay;
        assert_eq!(a, a);
        assert_ne!(a, b);
        assert_eq!(format!("{a:?}"), "Simulated");
        assert_eq!(a.clone(), a);
    }

    #[test]
    fn source_status_debug_clone_partial_eq() {
        let idle = SourceStatus::Idle;
        let running = SourceStatus::Running;
        let err1 = SourceStatus::Error("oops".into());
        let err2 = SourceStatus::Error("oops".into());
        assert_eq!(idle, idle);
        assert_ne!(idle, running);
        assert_eq!(err1, err2);
        assert_ne!(err1, idle);
        assert_eq!(idle.clone(), idle);
        assert_eq!(format!("{running:?}"), "Running");
        assert_eq!(format!("{err1:?}"), r#"Error("oops")"#);
    }

    #[test]
    fn stop_on_idle_does_not_panic() {
        let mut sm = SourceManager::new();
        sm.stop();
        assert_eq!(sm.status, SourceStatus::Idle);
    }

    #[test]
    fn recv_samples_returns_none_when_idle() {
        let mut sm = SourceManager::new();
        assert!(sm.recv_samples().is_none());
    }

    #[test]
    fn start_stop_lifecycle() {
        let mut sm = SourceManager::new();
        assert_eq!(sm.stream_generation(), 0);
        sm.start();
        assert_eq!(sm.status, SourceStatus::Running);
        assert!(sm.worker_handle.is_some());
        assert_eq!(sm.stream_generation(), 1);
        sm.stop();
        assert_eq!(sm.status, SourceStatus::Idle);
        assert_eq!(sm.stream_generation(), 1);
        sm.start();
        assert_eq!(sm.stream_generation(), 2);
        sm.stop();
    }

    #[test]
    fn start_idempotent() {
        let mut sm = SourceManager::new();
        sm.start();
        sm.start(); // second start should be a no-op
        assert_eq!(sm.status, SourceStatus::Running);
        assert_eq!(sm.stream_generation(), 1);
        sm.stop();
    }

    #[test]
    fn vfo_only_changes_do_not_advance_stream_generation() {
        let mut sm = SourceManager::new();
        sm.center_frequency_hz = Some(sm.frequency_hz);
        sm.start();
        let generation = sm.stream_generation();
        sm.frequency_hz += 25_000;
        sm.start();
        assert_eq!(sm.stream_generation(), generation);
        sm.stop();
    }

    #[test]
    fn rejected_local_start_does_not_advance_stream_generation() {
        let mut sm = SourceManager::new();
        sm.sample_rate_hz = 0;
        sm.start();
        assert!(matches!(sm.status, SourceStatus::Error(_)));
        assert_eq!(sm.stream_generation(), 0);
        assert!(sm.worker_handle.is_none());
    }

    #[test]
    fn daemon_stream_generation_counts_attempts_and_opening_is_idempotent() {
        let mut sm = SourceManager::new();
        sm.source_mode = SourceMode::Daemon;
        // Port zero requires no running server. The background attempt may fail,
        // but the manager stays Opening until the UI polls connection events.
        sm.daemon_addr = "127.0.0.1:0".into();
        sm.start();
        assert_eq!(sm.status, SourceStatus::Opening);
        assert_eq!(sm.stream_generation(), 1);
        sm.start();
        assert_eq!(sm.stream_generation(), 1);
        sm.stop();
        assert_eq!(sm.stream_generation(), 1);
        sm.start();
        assert_eq!(sm.stream_generation(), 2);
        sm.stop();
    }

    #[test]
    fn daemon_mode_is_distinct_and_not_the_default() {
        assert_ne!(SourceMode::Daemon, SourceMode::Simulated);
        assert_ne!(SourceMode::Daemon, SourceMode::Replay);
        assert_ne!(SourceMode::default(), SourceMode::Daemon);
        assert_eq!(format!("{:?}", SourceMode::Daemon), "Daemon");
    }

    #[test]
    fn recv_daemon_event_is_none_outside_daemon_mode() {
        let mut sm = SourceManager::new();
        assert!(sm.recv_daemon_event().is_none());
    }

    #[test]
    fn sync_daemon_controls_is_a_no_op_outside_daemon_mode() {
        let mut sm = SourceManager::new();
        sm.frequency_hz = 200_000_000;
        sm.sync_daemon_controls(); // must not panic with no daemon_client set
    }

    #[test]
    fn start_daemon_with_invalid_address_reports_error_status() {
        let mut sm = SourceManager::new();
        sm.source_mode = SourceMode::Daemon;
        sm.daemon_addr = "not-a-valid-address".to_string();
        sm.start();
        assert!(
            matches!(sm.status, SourceStatus::Error(_)),
            "expected Error status, got {:?}",
            sm.status
        );
        assert_eq!(sm.stream_generation(), 0);
        sm.stop();
    }

    #[cfg(not(feature = "rtlsdr"))]
    #[test]
    fn rand_f64_deterministic() {
        let a = rand_f64(0.0);
        let b = rand_f64(0.0);
        assert!((a - b).abs() < f64::EPSILON);
        // Range check
        assert!((0.0..1.0).contains(&a));
        // Different seeds yield different values
        let c = rand_f64(1.0);
        assert!((c - a).abs() > 1e-10, "expected different seeds to diverge");
        // Known value from LCG formula
        let known = 1013904223.0 / 4294967296.0;
        assert!((a - known).abs() < 1e-10);
    }
}
