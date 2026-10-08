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

/// Complete local-device open request, captured once before the worker starts.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RtlOpenSettings {
    pub device: RtlDeviceSelection,
    pub frequency_hz: u32,
    pub sample_rate_hz: u32,
    pub gain_db: f64,
    pub ppm: i32,
    pub bias_tee: bool,
    pub tuner_agc: bool,
    pub rtl_agc: bool,
    pub direct_sampling_mode: i32,
    pub offset_tuning: bool,
}

#[cfg(test)]
type TestHardwareOpen =
    std::sync::Arc<dyn Fn(RtlOpenSettings) -> Result<Vec<u8>, String> + Send + Sync>;

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

#[cfg(any(feature = "rtlsdr", test))]
fn rtl_device_name(
    name: String,
    serial: Option<&str>,
    product: Option<&str>,
    manufacturer: Option<&str>,
) -> String {
    let lower = |value: &str| value.trim().to_ascii_lowercase();
    let blog_manufacturer = manufacturer.is_some_and(|value| {
        let value = lower(value);
        value.contains("rtlsdrblog") || value.contains("rtl-sdr blog")
    });
    let product_text = product.map(lower);
    let product_is_v4 = product_text.as_deref().is_some_and(|value| {
        value == "blog v4"
            || value.starts_with("blog v4 ")
            || value == "rtl-sdr blog v4"
            || (value == "v4" && blog_manufacturer)
    });
    let product_identifies_other_model = product_text.as_deref().is_some_and(|value| {
        value.contains("blog v1")
            || value.contains("blog v2")
            || value.contains("blog v3")
            || value.contains("v1")
            || value.contains("v2")
            || value.contains("v3")
    });
    let serial_is_v4_fallback = serial.is_some_and(|value| value.trim() == "00000001")
        && blog_manufacturer
        && !product_identifies_other_model;
    let is_candidate_name = {
        let value = lower(&name);
        value.contains("generic") || value.contains("rtl")
    };

    if is_candidate_name && (product_is_v4 || serial_is_v4_fallback) {
        "RTL-SDR Blog V4".into()
    } else {
        name
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
        let manufacturer = text(&manufacturer);
        let product = text(&product);
        let serial = text(&serial);
        // lib RTL-SDR can expose a generic RTL2832 name even when USB identity
        // strings identify the tuner. Prefer the specific V4 label when the
        // product/manufacturer or known factory serial confirms it.
        let name = rtl_device_name(
            name,
            serial.as_deref(),
            product.as_deref(),
            manufacturer.as_deref(),
        );
        devices.push(RtlDeviceInfo {
            index,
            name,
            manufacturer,
            product,
            serial,
        });
    }
    Ok(devices)
}

#[cfg(any(not(feature = "rtlsdr"), test))]
fn enumerate_rtl_devices() -> Result<Vec<RtlDeviceInfo>, String> {
    Err("RTL-SDR discovery requires a hardware-enabled build (--features rtlsdr).".into())
}

/// A local capture block carries its worker's immutable acquisition geometry.
/// Sample indices advance even when delivery overflows; replay rewinds use a
/// new epoch and unknown UTC rather than the viewer's wall clock.
#[derive(Debug, Clone)]
pub(crate) struct CaptureBlock {
    /// CU8 compatibility data for byte-only previews and ADS-B.
    pub samples: Vec<u8>,
    /// Native normalized CF32 replay. Radio DSP and CF32 recording consume
    /// these samples without clipping or quantizing through the preview bytes.
    pub float_samples: Option<Arc<[num_complex::Complex32]>>,
    pub clock: ez_proto::CaptureClock,
    pub center_hz: u64,
    /// When the worker completed this read, before any GUI queue delay.
    pub observed_at: Instant,
    pub source_generation: u64,
    pub source_mode: SourceMode,
    /// Sample coordinate across replay rewinds within this source worker.
    pub stream_sample_index: u64,
    pub session: Option<Arc<crate::capture_stream::CaptureSession>>,
    file_position: Option<u64>,
}

#[cfg(test)]
impl CaptureBlock {
    pub(crate) fn fixture(
        samples: Vec<u8>,
        clock: ez_proto::CaptureClock,
        generation: u64,
        stream_sample_index: u64,
    ) -> Self {
        Self {
            samples,
            float_samples: None,
            clock,
            center_hz: 1_090_000_000,
            observed_at: Instant::now(),
            source_generation: generation,
            source_mode: SourceMode::Hardware,
            stream_sample_index,
            session: None,
            file_position: None,
        }
    }
}

enum SourceMessage {
    Samples(CaptureBlock),
    EndOfStream,
    Error(String),
}

struct CaptureProducer {
    clock: ez_proto::CaptureClock,
    center_hz: u64,
    pending_i: Option<u8>,
    stream: Option<crate::capture_stream::CaptureStream>,
    session: Option<Arc<crate::capture_stream::CaptureSession>>,
    source_generation: u64,
    source_mode: SourceMode,
    stream_sample_index: u64,
    preview_gap: bool,
    running: Option<Arc<AtomicBool>>,
}

impl CaptureProducer {
    fn new(rate: u32, center_hz: u64) -> Self {
        let mut clock = ez_proto::CaptureClock::new(ez_proto::new_capture_epoch(), 0, rate, None);
        clock.discontinuous = true;
        Self {
            clock,
            center_hz,
            pending_i: None,
            stream: None,
            session: None,
            source_generation: 0,
            source_mode: SourceMode::Simulated,
            stream_sample_index: 0,
            preview_gap: false,
            running: None,
        }
    }

    fn attach_stream(
        mut self,
        stream: crate::capture_stream::CaptureStream,
        generation: u64,
        mode: SourceMode,
    ) -> Self {
        self.session = Some(stream.session());
        self.stream = Some(stream);
        self.source_generation = generation;
        self.source_mode = mode;
        self
    }

    fn rewind(&mut self) {
        self.clock.epoch = ez_proto::new_capture_epoch();
        self.clock.source_sample_index = 0;
        self.clock.discontinuous = true;
        self.pending_i = None;
    }

    fn finish(&self) {
        if let Some(session) = &self.session {
            session.finish();
        }
    }

    fn block(&mut self, samples: Vec<u8>, file_position: Option<u64>) -> Option<CaptureBlock> {
        self.block_with_float(samples, None, file_position)
    }

    #[cfg(any(feature = "rtlsdr", test))]
    fn block_from_acquisition(
        &mut self,
        samples: Vec<u8>,
        clock: ez_proto::CaptureClock,
    ) -> Option<CaptureBlock> {
        // The callback bridge assigned this clock before any queue drops. Keep
        // it through both native consumers and the GUI's separate lossy preview.
        self.clock = clock;
        self.pending_i = None;
        self.block(samples, None)
    }

    fn block_with_float(
        &mut self,
        mut samples: Vec<u8>,
        float_samples: Option<Arc<[num_complex::Complex32]>>,
        file_position: Option<u64>,
    ) -> Option<CaptureBlock> {
        // A blocking device/file read can return after Stop. Discard that read
        // before it reaches either the native stream or GUI preview.
        if self
            .running
            .as_ref()
            .is_some_and(|running| !running.load(Ordering::Acquire))
        {
            return None;
        }
        if let Some(i) = self.pending_i.take() {
            samples.insert(0, i);
        }
        if !samples.len().is_multiple_of(2) {
            self.pending_i = samples.pop();
        }
        if samples.is_empty() {
            return None;
        }
        let clock = self.clock;
        self.clock = self.clock.offset_samples((samples.len() / 2) as u64)?;
        self.clock.discontinuous = false;
        let block = CaptureBlock {
            samples,
            float_samples,
            clock,
            center_hz: self.center_hz,
            observed_at: Instant::now(),
            source_generation: self.source_generation,
            source_mode: self.source_mode.clone(),
            stream_sample_index: self.stream_sample_index,
            session: self.session.clone(),
            file_position,
        };
        self.stream_sample_index += (block.samples.len() / 2) as u64;
        if let Some(stream) = &self.stream {
            stream.publish(&block);
        }
        Some(block)
    }
}

impl Drop for CaptureProducer {
    fn drop(&mut self) {
        self.finish();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DaemonAudioTuning {
    frequency_hz: u64,
    offset_hz: i64,
    bandwidth_hz: u32,
}

/// Live input cannot wait for the UI without overrunning the hardware's USB
/// buffers. Drop only the overflow block; a full queue is not a dead receiver.
/// Counts drops so the UI can surface them for diagnosis.
fn send_live_samples(
    tx: &Sender<SourceMessage>,
    capture: &mut CaptureProducer,
    samples: Vec<u8>,
    dropped: &std::sync::atomic::AtomicU64,
) -> bool {
    let Some(block) = capture.block(samples, None) else {
        return true;
    };
    send_preview_block(tx, capture, block, dropped)
}

fn send_preview_block(
    tx: &Sender<SourceMessage>,
    capture: &mut CaptureProducer,
    mut block: CaptureBlock,
    dropped: &std::sync::atomic::AtomicU64,
) -> bool {
    // The GUI is a lossy preview consumer. Its loss must not contaminate the
    // independent decoder/recording stream that was published above.
    block.clock.discontinuous |= capture.preview_gap;
    match tx.try_send(SourceMessage::Samples(block)) {
        Ok(()) => {
            capture.preview_gap = false;
            true
        }
        Err(TrySendError::Disconnected(_)) => false,
        Err(TrySendError::Full(_)) => {
            dropped.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            capture.preview_gap = true;
            true // not a disconnect; the block is dropped but the source lives on
        }
    }
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

const DAEMON_SQUELCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

struct PendingSquelch {
    request_id: u64,
    db: f32,
    started: std::time::Instant,
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
    pub replay_cf32_amplitude: Option<lrpt_decode::Cf32Amplitude>,
    /// Current read position within the replay file (bytes).
    pub replay_position: u64,
    /// Total size of the replay file (bytes).
    pub replay_size: u64,
    /// `host:port` of the `ez-daemon` to connect to in `SourceMode::Daemon`.
    pub daemon_addr: String,
    daemon_client: Option<crate::daemon_client::DaemonClient>,
    daemon_channels: ez_proto::ClientChannels,
    daemon_session_ready: bool,
    daemon_channel_error: Option<String>,
    daemon_removals_pending: std::collections::HashSet<ez_proto::ChannelId>,
    /// Last frequency/sample-rate/gain actually sent to the daemon (or last value received
    /// FROM it) — lets `sync_daemon_controls` tell "user moved a slider" (send) apart from
    /// "the daemon just told us its own state" (already in sync, don't echo it back).
    daemon_last_center_hz: u64,
    daemon_last_sample_rate_hz: u32,
    daemon_last_gain_db: f64,
    daemon_applied_capture: Option<ez_proto::HardwareStatus>,
    daemon_adsb_confirmed: bool,
    daemon_requested_controls: Option<(u64, u32, f64)>,
    local_capture_accepted: bool,
    local_open_capture: Option<(u64, u32, f64)>,
    local_open_settings: Option<RtlOpenSettings>,
    #[cfg(test)]
    test_hardware_open: Option<TestHardwareOpen>,
    daemon_audio_mode: Option<ez_proto::DemodMode>,
    daemon_audio_tuning: Option<DaemonAudioTuning>,
    daemon_squelch_applied: Option<ez_proto::SquelchState>,
    daemon_squelch_pending: Option<PendingSquelch>,
    daemon_squelch_wanted: f32,
    daemon_squelch_last_requested: Option<f32>,
    daemon_squelch_next_request: u64,
    pub daemon_squelch_error: Option<String>,
    daemon_adsb_subscribed: bool,
    daemon_lrpt_subscribed: bool,
    daemon_recording_active: bool,
    tx: Option<Sender<SourceMessage>>,
    rx: Option<Receiver<SourceMessage>>,
    running: Arc<AtomicBool>,
    worker_handle: Option<std::thread::JoinHandle<()>>,
    restart_requested: bool,
    stream_generation: u64,
    capture_stream: crate::capture_stream::CaptureStream,
    /// Number of sample blocks dropped due to queue overflow (UI was behind).
    dropped_blocks: Arc<std::sync::atomic::AtomicU64>,
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

/// Local IDs used only by pure-command unit fixtures, before any handshake.
#[cfg(test)]
const DAEMON_AUDIO_CHANNEL_ID: ez_proto::ChannelId = 2;

/// The run-state of the SDR source.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceStatus {
    /// Source is stopped and ready to start.
    Idle,
    /// Source is initializing (device opening, etc.).
    Opening,
    /// Source is actively producing IQ samples.
    Running,
    /// Cancellation was requested; the worker still owns its device or file.
    Stopping,
    /// Source encountered a fatal error (carries error message).
    Error(String),
}

impl Default for SourceManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for SourceManager {
    fn drop(&mut self) {
        self.stop();
        // JoinHandle drop detaches the cancelled worker. It still owns and
        // closes its device if the driver returns; destruction cannot force a
        // synchronous USB call to return.
    }
}

impl SourceManager {
    /// Create a new `SourceManager` in `Idle` state with default tuning values.
    pub fn new() -> Self {
        let (tx, rx) = bounded(8);
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
            replay_cf32_amplitude: None,
            replay_position: 0,
            replay_size: 0,
            daemon_addr: "127.0.0.1:7890".to_string(),
            daemon_client: None,
            daemon_channels: ez_proto::ClientChannels {
                spectrum: 1,
                audio: 2,
                adsb: 3,
                lrpt: 4,
            },
            daemon_session_ready: false,
            daemon_channel_error: None,
            daemon_removals_pending: std::collections::HashSet::new(),
            daemon_last_center_hz: 109_000_000,
            daemon_last_sample_rate_hz: 2_048_000,
            daemon_applied_capture: None,
            daemon_adsb_confirmed: false,
            daemon_requested_controls: None,
            local_capture_accepted: false,
            local_open_capture: None,
            local_open_settings: None,
            #[cfg(test)]
            test_hardware_open: None,
            daemon_last_gain_db: 40.0,
            daemon_audio_mode: None,
            daemon_audio_tuning: None,
            daemon_squelch_applied: None,
            daemon_squelch_pending: None,
            daemon_squelch_wanted: -100.0,
            daemon_squelch_last_requested: None,
            daemon_squelch_next_request: 0,
            daemon_squelch_error: None,
            daemon_adsb_subscribed: false,
            daemon_lrpt_subscribed: false,
            daemon_recording_active: false,
            tx: Some(tx),
            rx: Some(rx),
            running: Arc::new(AtomicBool::new(false)),
            worker_handle: None,
            restart_requested: false,
            stream_generation: 0,
            capture_stream: crate::capture_stream::CaptureStream::default(),
            dropped_blocks: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    /// Identity of the latest started local worker or daemon connection attempt.
    /// Changes on restart even when capture settings remain identical; stopping,
    /// rejected starts, and VFO-only control changes leave it unchanged.
    pub fn stream_generation(&self) -> u64 {
        self.stream_generation
    }

    pub(crate) fn capture_stream(&self) -> crate::capture_stream::CaptureStream {
        self.capture_stream.clone()
    }

    pub fn capture_center_frequency_hz(&self) -> u64 {
        self.center_frequency_hz.unwrap_or(self.frequency_hz)
    }

    /// Validate the physical local RTL frequency without truncating RF/VFO
    /// frequencies above 4 GHz. Used only when the source is local RTL hardware.
    pub fn rtl_center_frequency_hz(&self) -> Result<u32, String> {
        checked_tuner_center(self.capture_center_frequency_hz(), self.frequency_offset_hz)
    }

    pub(crate) fn rtl_open_settings(&self) -> Result<RtlOpenSettings, String> {
        validate_rtl_options(self.direct_sampling, self.offset_tuning)?;
        Ok(RtlOpenSettings {
            device: self.rtl_device.clone(),
            frequency_hz: self.rtl_center_frequency_hz()?,
            sample_rate_hz: self.sample_rate_hz,
            gain_db: self.gain_db,
            ppm: self.ppm_correction,
            bias_tee: self.bias_tee,
            tuner_agc: self.tuner_agc,
            rtl_agc: self.rtl_agc,
            direct_sampling_mode: self.direct_sampling_mode(),
            offset_tuning: self.offset_tuning,
        })
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
                let has_saved_serial = self
                    .rtl_device
                    .serial
                    .as_deref()
                    .is_some_and(|serial| !serial.is_empty());
                if self.rtl_devices.len() == 1 && !has_saved_serial {
                    let index = self.rtl_devices[0].index;
                    if let Err(error) = self.select_rtl_device(index) {
                        self.rtl_device_refresh_error = Some(error);
                    }
                } else {
                    match resolve_rtl_device(&self.rtl_device, &self.rtl_devices) {
                        Ok(index) => {
                            self.rtl_device.index = index;
                            self.rtl_device_refresh_error = None;
                        }
                        Err(error) => self.rtl_device_refresh_error = Some(error),
                    }
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
            self.daemon_requested_controls = Some((frequency_hz, sample_rate_hz, self.gain_db));
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
    /// them through the internal channel. Hardware remains `Opening` until
    /// its first accepted sample block confirms acquisition.
    pub fn start(&mut self) {
        self.poll_shutdown();
        if matches!(self.status, SourceStatus::Running | SourceStatus::Opening) {
            return;
        }
        if self.worker_handle.is_some() {
            self.stop();
            if self.worker_handle.is_some() {
                self.restart_requested = true;
                return;
            }
        }
        if self.source_mode == SourceMode::Daemon {
            self.start_daemon();
            return;
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
        let _rtl_settings = if self.source_mode == SourceMode::Hardware {
            match self.rtl_open_settings() {
                Ok(settings) => Some(settings),
                Err(error) => {
                    self.status = SourceStatus::Error(error);
                    return;
                }
            }
        } else {
            None
        };
        self.local_capture_accepted = false;
        self.local_open_capture = Some((
            self.capture_center_frequency_hz(),
            self.sample_rate_hz,
            self.gain_db,
        ));
        self.local_open_settings = _rtl_settings.clone();
        #[cfg(test)]
        let test_hardware_open = self.test_hardware_open.clone();
        self.status = SourceStatus::Opening;
        // Each acquisition owns its cancellation flag. A stopped worker can
        // never be revived by a later start.
        self.running = Arc::new(AtomicBool::new(true));
        let running = self.running.clone();
        let dropped_blocks = self.dropped_blocks.clone();

        // Recreate channel if needed (after a previous stop)
        let tx = if let Some(tx) = self.tx.take() {
            tx
        } else {
            let (new_tx, new_rx) = bounded(8);
            self.rx = Some(new_rx);
            new_tx
        };

        let rate = self.sample_rate_hz;
        let center_hz = self.capture_center_frequency_hz();
        let source_mode = self.source_mode.clone();
        let replay_file = self.replay_file.clone();
        let replay_loop = self.replay_loop;
        let replay_speed = self.replay_speed;
        let replay_cf32_amplitude = self.replay_cf32_amplitude;
        if self.source_mode == SourceMode::Replay {
            self.replay_position = 0;
            self.replay_size = replay_file
                .as_deref()
                .and_then(|path| std::fs::metadata(path).ok())
                .filter(|metadata| metadata.is_file())
                .map_or(0, |metadata| metadata.len());
        }

        self.stream_generation = self.stream_generation.wrapping_add(1);
        let generation = self.stream_generation;
        let stream = self.capture_stream.clone();
        stream.start(generation);
        let handle = std::thread::spawn(move || {
            let mut capture = CaptureProducer::new(rate, center_hz).attach_stream(
                stream,
                generation,
                source_mode.clone(),
            );
            capture.running = Some(Arc::clone(&running));
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
                    let is_cs8 = extension == "cs8";
                    let amplitude = if is_cf32 {
                        match lrpt_decode::cf32_amplitude_for_path(
                            std::path::Path::new(&path),
                            replay_cf32_amplitude,
                        ) {
                            Ok(amplitude) => amplitude,
                            Err(message) => {
                                send_cancellable(&tx, &running, SourceMessage::Error(message));
                                return;
                            }
                        }
                    } else {
                        lrpt_decode::Cf32Amplitude::Normalized
                    };
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
                    let mut file_position = 0u64;
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
                                    file_position = 0;
                                    capture.rewind();
                                    continue;
                                }
                                capture.finish();
                                send_cancellable(&tx, &running, SourceMessage::EndOfStream);
                                break;
                            }
                            Ok(mut n) => {
                                let remainder = n % sample_bytes as usize;
                                if remainder != 0 {
                                    let aligned = n + sample_bytes as usize - remainder;
                                    if let Err(error) = reader.read_exact(&mut buf[n..aligned]) {
                                        send_cancellable(
                                            &tx,
                                            &running,
                                            SourceMessage::Error(format!(
                                                "Incomplete IQ sample: {error}"
                                            )),
                                        );
                                        break;
                                    }
                                    n = aligned;
                                }
                                let float_samples = if is_cf32 {
                                    Some(Arc::<[num_complex::Complex32]>::from(
                                        lrpt_decode::cf32_le_bytes_to_normalized(
                                            &buf[..n],
                                            amplitude,
                                        ),
                                    ))
                                } else {
                                    None
                                };
                                let chunk = if let Some(samples) = &float_samples {
                                    samples
                                        .iter()
                                        .flat_map(|sample| {
                                            [sample.re, sample.im]
                                                .map(lrpt_decode::normalized_component_to_uc8)
                                        })
                                        .collect()
                                } else if is_cs8 {
                                    // XOR moves the signed two's-complement midpoint to CU8.
                                    buf[..n].iter().map(|byte| byte ^ 0x80).collect()
                                } else {
                                    buf[..n].to_vec()
                                };

                                file_position += n as u64;
                                if let Some(block) = capture.block_with_float(
                                    chunk,
                                    float_samples,
                                    Some(file_position),
                                ) {
                                    // Production consumers run continuously;
                                    // a stalled GUI preview must not slow replay.
                                    // Standalone lossless readers retain their
                                    // cancellable backpressure contract.
                                    let delivered = if capture
                                        .stream
                                        .as_ref()
                                        .is_some_and(|stream| stream.has_consumers())
                                    {
                                        send_preview_block(
                                            &tx,
                                            &mut capture,
                                            block,
                                            &dropped_blocks,
                                        )
                                    } else {
                                        send_cancellable(
                                            &tx,
                                            &running,
                                            SourceMessage::Samples(block),
                                        )
                                    };
                                    if !delivered {
                                        break;
                                    }
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
                        let settings = _rtl_settings.expect("hardware open settings validated");
                        let opened = unsafe {
                            rtl_sdr_open(
                                &settings.device,
                                settings.frequency_hz,
                                settings.sample_rate_hz,
                                settings.ppm,
                                settings.bias_tee,
                                settings.gain_db,
                                settings.tuner_agc,
                                settings.rtl_agc,
                                settings.direct_sampling_mode,
                                settings.offset_tuning,
                            )
                        };
                        let dev = match opened {
                            Ok(dev) => dev,
                            Err(error) => {
                                send_cancellable(&tx, &running, SourceMessage::Error(error));
                                return;
                            }
                        };
                        // SAFETY: this worker owns dev. It joins the native
                        // pump before close, including a stop during startup.
                        let stream =
                            unsafe { ez_daemon::hardware::rtl_async::RtlAsyncStream::start(dev) };
                        let mut stream = match stream {
                            Ok(stream) => stream,
                            Err(error) => {
                                unsafe {
                                    rtl_sdr_close(dev);
                                }
                                send_cancellable(
                                    &tx,
                                    &running,
                                    SourceMessage::Error(error.to_string()),
                                );
                                return;
                            }
                        };
                        let mut buf = vec![0u8; 16384 * 2];
                        while running.load(Ordering::SeqCst) {
                            let n = match stream.read_uc8(&mut buf, rate) {
                                Ok(0) => {
                                    send_cancellable(
                                        &tx,
                                        &running,
                                        SourceMessage::Error(
                                            "RTL-SDR asynchronous stream ended.".into(),
                                        ),
                                    );
                                    break;
                                }
                                Ok(n) => n,
                                Err(error)
                                    if error
                                        .downcast_ref::<ez_daemon::hardware::ReadPending>()
                                        .is_some() =>
                                {
                                    continue
                                }
                                Err(error) => {
                                    send_cancellable(
                                        &tx,
                                        &running,
                                        SourceMessage::Error(error.to_string()),
                                    );
                                    break;
                                }
                            };
                            if let Some(block) = capture.block_from_acquisition(
                                buf[..n].to_vec(),
                                stream.capture_clock().expect("successful read has clock"),
                            ) {
                                if !send_preview_block(&tx, &mut capture, block, &dropped_blocks) {
                                    break;
                                }
                            }
                        }
                        if let Err(error) = stream.stop() {
                            send_cancellable(
                                &tx,
                                &running,
                                SourceMessage::Error(error.to_string()),
                            );
                        }
                        drop(stream);
                        // SAFETY: no read/callback can use dev after the join.
                        unsafe {
                            rtl_sdr_close(dev);
                        }
                    }
                    #[cfg(test)]
                    {
                        if let Some(open) = test_hardware_open {
                            match open(_rtl_settings.expect("validated hardware request")) {
                                Ok(samples) if !samples.is_empty() => {
                                    while running.load(Ordering::SeqCst) {
                                        if !send_live_samples(
                                            &tx,
                                            &mut capture,
                                            samples.clone(),
                                            &dropped_blocks,
                                        ) {
                                            break;
                                        }
                                        std::thread::sleep(iq_block_duration(samples.len(), rate));
                                    }
                                }
                                Ok(_) => {
                                    send_cancellable(
                                        &tx,
                                        &running,
                                        SourceMessage::Error(
                                            "test hardware returned no samples".into(),
                                        ),
                                    );
                                }
                                Err(error) => {
                                    send_cancellable(&tx, &running, SourceMessage::Error(error));
                                }
                            }
                        } else {
                            send_cancellable(&tx, &running, SourceMessage::Error("This build does not include RTL-SDR support. Use Demo, File Replay, or Daemon mode, or rebuild with --features rtlsdr.".into()));
                        }
                    }
                    #[cfg(all(not(feature = "rtlsdr"), not(test)))]
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

                        if !send_live_samples(&tx, &mut capture, buf.clone(), &dropped_blocks) {
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
        self.tx = None; // tx was moved into the worker thread
        if self.source_mode != SourceMode::Hardware {
            self.status = SourceStatus::Running;
        }
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
        self.daemon_session_ready = false;
        self.daemon_applied_capture = None;
        self.daemon_adsb_confirmed = false;
        self.daemon_channel_error = None;
        self.daemon_removals_pending.clear();
        self.daemon_audio_mode = None;
        self.daemon_audio_tuning = None;
        self.local_capture_accepted = false;
        self.reset_daemon_squelch();
        self.daemon_adsb_subscribed = false;
        self.daemon_lrpt_subscribed = false;
        self.daemon_recording_active = false;
        self.daemon_client = Some(client);
        self.stream_generation = self.stream_generation.wrapping_add(1);
    }

    #[cfg(test)]
    pub(crate) fn set_test_daemon_client(&mut self, client: crate::daemon_client::DaemonClient) {
        self.daemon_client = Some(client);
    }

    pub fn daemon_channel_ids(&self) -> ez_proto::ClientChannels {
        self.daemon_channels
    }

    fn sync_daemon_session(&mut self) {
        if self.daemon_session_ready || self.daemon_applied_capture.is_none() {
            return;
        }
        let Some(channels) = self
            .daemon_client
            .as_ref()
            .and_then(|client| client.client_channels())
        else {
            return;
        };
        self.daemon_channels = channels;
        self.daemon_session_ready = true;
        if let Some(client) = &self.daemon_client {
            client.send(ez_proto::ClientCommand::Subscribe {
                channel: self.daemon_spectrum_spec(),
            });
        }
    }

    fn active_daemon_data(&self, event: &ez_proto::ServerEvent) -> bool {
        match event {
            ez_proto::ServerEvent::StreamGap { channel_id, .. } => {
                (*channel_id == self.daemon_channels.spectrum && self.daemon_session_ready)
                    || (*channel_id == self.daemon_channels.audio
                        && self.daemon_audio_mode.is_some())
                    || (*channel_id == self.daemon_channels.adsb && self.daemon_adsb_subscribed)
                    || (*channel_id == self.daemon_channels.lrpt && self.daemon_lrpt_subscribed)
            }
            ez_proto::ServerEvent::Spectrum(frame) => {
                self.daemon_session_ready && frame.channel_id == self.daemon_channels.spectrum
            }
            ez_proto::ServerEvent::Audio(frame) => {
                self.daemon_audio_mode.is_some() && frame.channel_id == self.daemon_channels.audio
            }
            ez_proto::ServerEvent::Aircraft(snapshot) => {
                self.daemon_adsb_subscribed && snapshot.channel_id == self.daemon_channels.adsb
            }
            ez_proto::ServerEvent::Telemetry(frame) => {
                self.daemon_lrpt_subscribed && frame.channel_id == self.daemon_channels.lrpt
            }
            ez_proto::ServerEvent::Recording(status) => {
                status.channel_id == self.daemon_channels.spectrum
            }
            _ => true,
        }
    }

    fn reset_daemon_squelch(&mut self) {
        self.daemon_squelch_applied = None;
        self.daemon_squelch_pending = None;
        self.daemon_squelch_last_requested = None;
        self.daemon_squelch_error = None;
    }

    pub fn daemon_squelch_applied(&self) -> Option<ez_proto::SquelchState> {
        self.daemon_squelch_applied
    }
    pub fn daemon_squelch_pending_db(&self) -> Option<f32> {
        self.daemon_squelch_pending
            .as_ref()
            .map(|_| self.daemon_squelch_wanted)
    }

    /// Request through the actual audio channel, retaining only one in-flight
    /// command. Subsequent edits coalesce in the desired value/configuration.
    pub fn request_daemon_squelch(&mut self, db: f32) -> Result<(), String> {
        if !db.is_finite() {
            return Err("squelch threshold must be finite".into());
        }
        if self.source_mode != SourceMode::Daemon || self.daemon_audio_mode.is_none() {
            return Err("daemon audio channel is not active".into());
        }
        if self.daemon_client.as_ref().is_none_or(|client| {
            client.status() != crate::daemon_client::ConnectionStatus::Connected
        }) {
            return Err("daemon is not connected".into());
        }
        if self.daemon_squelch_error.take().is_some() {
            self.daemon_squelch_last_requested = None;
        }
        self.sync_daemon_squelch(db);
        self.daemon_squelch_error.clone().map_or(Ok(()), Err)
    }

    pub fn sync_daemon_squelch(&mut self, db: f32) {
        if !db.is_finite() {
            self.daemon_squelch_error = Some("squelch threshold must be finite".into());
            return;
        }
        if self.source_mode != SourceMode::Daemon || self.daemon_audio_mode.is_none() {
            return;
        }
        self.daemon_squelch_wanted = db;
        if self
            .daemon_squelch_pending
            .as_ref()
            .is_some_and(|request| request.started.elapsed() >= DAEMON_SQUELCH_TIMEOUT)
        {
            self.daemon_squelch_pending = None;
            self.daemon_squelch_error = Some("daemon squelch acknowledgement timed out".into());
        }
        if self.daemon_squelch_pending.is_some() || self.daemon_squelch_last_requested == Some(db) {
            return;
        }
        let Some(client) = self
            .daemon_client
            .as_ref()
            .filter(|client| client.status() == crate::daemon_client::ConnectionStatus::Connected)
        else {
            return;
        };
        self.daemon_squelch_next_request = self.daemon_squelch_next_request.wrapping_add(1);
        let request_id = self.daemon_squelch_next_request;
        self.daemon_squelch_last_requested = Some(db);
        self.daemon_squelch_error = None;
        if client.try_send(ez_proto::ClientCommand::SetSquelch {
            channel_id: self.daemon_channels.audio,
            db,
            request_id,
        }) {
            self.daemon_squelch_pending = Some(PendingSquelch {
                request_id,
                db,
                started: std::time::Instant::now(),
            });
        } else {
            self.daemon_squelch_error = Some("daemon squelch command could not be queued".into());
        }
    }

    fn observe_daemon_squelch(&mut self, state: ez_proto::SquelchState) {
        if state.db.is_finite()
            && self.daemon_squelch_applied.is_none_or(|applied| {
                state.revision > applied.revision
                    || (state.revision == applied.revision && state.db == applied.db)
            })
        {
            self.daemon_squelch_applied = Some(state);
        }
    }

    fn apply_daemon_squelch_reply(
        &mut self,
        channel_id: ez_proto::ChannelId,
        request_id: u64,
        result: &Result<ez_proto::SquelchState, String>,
    ) {
        if channel_id != self.daemon_channels.audio
            || self
                .daemon_squelch_pending
                .as_ref()
                .is_none_or(|pending| pending.request_id != request_id)
        {
            return;
        }
        let pending = self.daemon_squelch_pending.take().unwrap();
        match result {
            Ok(applied) => {
                self.observe_daemon_squelch(*applied);
                self.daemon_squelch_error = if !applied.db.is_finite() || applied.db != pending.db {
                    Some("daemon applied an unexpected squelch threshold".into())
                } else {
                    None
                };
            }
            Err(error) => self.daemon_squelch_error = Some(error.clone()),
        }
    }

    /// Control replies can precede old data already queued on the client. Never
    /// play audio from a lower applied revision or from a different channel.
    pub fn accept_daemon_audio(&self, frame: &ez_proto::AudioFrame) -> bool {
        self.source_mode == SourceMode::Daemon
            && self.status == SourceStatus::Running
            && self.daemon_audio_mode.is_some()
            && frame.channel_id == self.daemon_channels.audio
            && self.daemon_squelch_pending.is_none()
            && self
                .daemon_squelch_applied
                .is_some_and(|applied| frame.squelch == applied)
    }

    fn daemon_spectrum_spec(&self) -> ez_proto::ChannelSpec {
        ez_proto::ChannelSpec {
            id: self.daemon_channels.spectrum,
            center_offset_hz: 0,
            bandwidth_hz: self
                .daemon_applied_capture
                .as_ref()
                .map_or(self.sample_rate_hz, |hw| hw.sample_rate_hz),
            kind: ez_proto::PipelineKind::Spectrum,
            demod_mode: None,
        }
    }

    /// An automatic tune is acknowledged only when the active capture reports
    /// the requested geometry. Daemon control sends alone are not acknowledgements.
    pub(crate) fn capture_tuning_applied(&self) -> bool {
        if self.source_mode != SourceMode::Daemon {
            if self.status == SourceStatus::Idle {
                return true;
            }
            return self.status == SourceStatus::Running
                && self.local_capture_accepted
                && self.local_open_capture.is_some_and(|(center, rate, _)| {
                    center == self.capture_center_frequency_hz() && rate == self.sample_rate_hz
                });
        }
        self.daemon_applied_capture.as_ref().is_some_and(|hw| {
            self.status == SourceStatus::Running
                && hw.connected
                && hw.frequency_hz == self.capture_center_frequency_hz()
                && hw.sample_rate_hz == self.sample_rate_hz
                && self.frequency_hz.abs_diff(hw.frequency_hz) <= u64::from(hw.sample_rate_hz) / 2
        })
    }

    /// Configuration accepted by the running capture, separate from controls
    /// awaiting application. Local values are the driver-open request accepted
    /// before the first sample; they are not calibrated RF measurements.
    pub(crate) fn applied_capture_settings(&self) -> Option<(u64, u32, f64)> {
        if self.status != SourceStatus::Running {
            return None;
        }
        if self.source_mode == SourceMode::Daemon {
            return self
                .daemon_applied_capture
                .as_ref()
                .filter(|hw| hw.connected)
                .map(|hw| (hw.frequency_hz, hw.sample_rate_hz, hw.gain_db));
        }
        self.local_capture_accepted
            .then_some(self.local_open_capture)
            .flatten()
    }

    pub(crate) fn gain_request_applied(&self, gain: f64) -> bool {
        if self.source_mode == SourceMode::Daemon {
            return self.daemon_applied_capture.as_ref().is_some_and(|hw| {
                hw.connected && (hw.gain_db == gain || hw.requested_gain_db == Some(gain))
            });
        }
        self.local_capture_accepted
            && self
                .local_open_capture
                .is_some_and(|(_, _, value)| value == gain)
    }

    pub(crate) fn local_hardware_request_applied(&self) -> bool {
        self.local_capture_accepted
            && self.local_open_settings.as_ref().is_some_and(|opened| {
                self.rtl_open_settings()
                    .is_ok_and(|current| current == *opened)
            })
    }

    pub(crate) fn daemon_adsb_ready(&self) -> bool {
        self.daemon_adsb_confirmed
    }

    #[cfg(test)]
    pub(crate) fn test_set_hardware_open(
        &mut self,
        open: impl Fn(RtlOpenSettings) -> Result<Vec<u8>, String> + Send + Sync + 'static,
    ) {
        self.test_hardware_open = Some(std::sync::Arc::new(open));
    }

    pub(crate) fn cancel_daemon_control_request(&mut self) {
        self.daemon_requested_controls = None;
        if let Some(hardware) = self.daemon_applied_capture.clone() {
            self.apply_daemon_hardware(&hardware);
        }
    }

    fn apply_daemon_hardware(&mut self, hardware: &ez_proto::HardwareStatus) {
        self.daemon_applied_capture = hardware.connected.then_some(hardware.clone());
        if self
            .daemon_requested_controls
            .is_some_and(|(center, rate, gain)| {
                hardware.connected
                    && hardware.frequency_hz == center
                    && hardware.sample_rate_hz == rate
                    && hardware.requested_gain_db.unwrap_or(hardware.gain_db) == gain
            })
        {
            self.daemon_requested_controls = None;
        }
        if self.daemon_requested_controls.is_none() {
            if self.center_frequency_hz.is_some() {
                self.center_frequency_hz = Some(hardware.frequency_hz);
            } else {
                self.frequency_hz = hardware.frequency_hz;
            }
            self.sample_rate_hz = hardware.sample_rate_hz;
            self.gain_db = hardware.gain_db;
        }
        self.daemon_last_center_hz = hardware.frequency_hz;
        self.daemon_last_sample_rate_hz = hardware.sample_rate_hz;
        self.daemon_last_gain_db = hardware.gain_db;
    }

    pub(crate) fn begin_daemon_spectrum_observation(&self, revision: u64) -> bool {
        self.daemon_session_ready
            && self.daemon_client.as_ref().is_some_and(|client| {
                client.try_send(ez_proto::ClientCommand::BeginSpectrumObservation {
                    channel_id: self.daemon_channels.spectrum,
                    revision,
                })
            })
    }

    pub(crate) fn daemon_spectrum_received_at(&self) -> Option<Instant> {
        self.daemon_client.as_ref()?.spectrum_received_at()
    }

    /// Non-blocking pull of one event from the daemon connection (`SourceMode::Daemon` only;
    /// always `None` in other modes). Reflects connection status and daemon-reported hardware
    /// state into this manager's own `status`/`frequency_hz`/`sample_rate_hz`/`gain_db`
    /// fields — the daemon is authoritative for these in this mode, the same way a local
    /// worker thread is authoritative for them in the other modes.
    #[must_use]
    pub fn recv_daemon_event(&mut self) -> Option<ez_proto::ServerEvent> {
        self.poll_shutdown();
        self.sync_daemon_session();
        let (status, event) = {
            let client = self.daemon_client.as_ref()?;
            let mut event = None;
            for _ in 0..64 {
                let Some(next) = client.try_recv_event() else {
                    break;
                };
                if self.active_daemon_data(&next) {
                    event = Some(next);
                    break;
                }
            }
            (client.status(), event)
        };
        self.status = match status {
            crate::daemon_client::ConnectionStatus::Connecting => SourceStatus::Opening,
            crate::daemon_client::ConnectionStatus::Connected => SourceStatus::Running,
            crate::daemon_client::ConnectionStatus::Disconnected => SourceStatus::Idle,
            crate::daemon_client::ConnectionStatus::Error(e) => SourceStatus::Error(e),
        };
        if let Some(error) = &self.daemon_channel_error {
            self.status = SourceStatus::Error(error.clone());
        }
        if self.status != SourceStatus::Running && self.daemon_squelch_pending.take().is_some() {
            self.daemon_squelch_error =
                Some("daemon disconnected before squelch acknowledgement".into());
        }
        let event = event?;
        if let ez_proto::ServerEvent::ChannelRemoved { channel_id } = &event {
            let expected = self.daemon_removals_pending.remove(channel_id);
            let active = !expected
                && ((*channel_id == self.daemon_channels.spectrum && self.daemon_session_ready)
                    || (*channel_id == self.daemon_channels.audio
                        && self.daemon_audio_mode.is_some())
                    || (*channel_id == self.daemon_channels.adsb && self.daemon_adsb_subscribed)
                    || (*channel_id == self.daemon_channels.lrpt && self.daemon_lrpt_subscribed));
            if active {
                self.daemon_channel_error = Some(format!(
                    "daemon channel {channel_id} was removed; reconnect to restart the workflow"
                ));
                self.status = SourceStatus::Error(self.daemon_channel_error.clone().unwrap());
                self.daemon_audio_mode = None;
                self.reset_daemon_squelch();
                self.daemon_adsb_subscribed = false;
                self.daemon_lrpt_subscribed = false;
            }
        }
        if let ez_proto::ServerEvent::Squelch {
            channel_id,
            request_id,
            result,
        } = &event
        {
            self.apply_daemon_squelch_reply(*channel_id, *request_id, result);
        } else if let ez_proto::ServerEvent::Audio(frame) = &event {
            if frame.channel_id == self.daemon_channels.audio && self.daemon_audio_mode.is_some() {
                self.observe_daemon_squelch(frame.squelch);
            }
        } else if let ez_proto::ServerEvent::Aircraft(snapshot) = &event {
            if self.daemon_adsb_subscribed && snapshot.channel_id == self.daemon_channels.adsb {
                self.daemon_adsb_confirmed = true;
            }
        } else if let ez_proto::ServerEvent::Hardware(hw) = &event {
            self.apply_daemon_hardware(hw);
        } else if let ez_proto::ServerEvent::Recording(status) = &event {
            if status.channel_id == self.daemon_channels.spectrum {
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
        self.sync_daemon_session();
        if !self.daemon_session_ready {
            return Err("wait for the daemon channel allocation before recording".into());
        }
        let client = self
            .daemon_client
            .as_ref()
            .ok_or_else(|| "connect to the daemon before recording".to_string())?;
        if client.status() != crate::daemon_client::ConnectionStatus::Connected {
            return Err("wait for the daemon connection before recording".to_string());
        }
        client.send(ez_proto::ClientCommand::StartRecording {
            channel_id: self.daemon_channels.spectrum,
            format,
        });
        Ok(())
    }

    pub fn stop_daemon_recording(&mut self) -> Result<(), String> {
        if self.source_mode != SourceMode::Daemon {
            return Err("daemon recording is only available in Daemon source mode".to_string());
        }
        self.sync_daemon_session();
        if !self.daemon_session_ready {
            return Err("wait for the daemon channel allocation before recording".into());
        }
        let client = self
            .daemon_client
            .as_ref()
            .ok_or_else(|| "daemon is not connected".to_string())?;
        client.send(ez_proto::ClientCommand::StopRecording {
            channel_id: self.daemon_channels.spectrum,
        });
        Ok(())
    }

    /// Forwards local frequency/sample-rate/gain field changes to the daemon
    /// (`SourceMode::Daemon` only; a no-op otherwise) — call once per UI frame, separately
    /// from the per-event [`Self::recv_daemon_event`] drain loop, so a slider edit is sent
    /// exactly once even if several events are drained in the same frame.
    pub fn sync_daemon_controls(&mut self) {
        if self.daemon_client.is_none() || self.daemon_applied_capture.is_none() {
            return;
        }
        let wanted = (
            self.capture_center_frequency_hz(),
            self.sample_rate_hz,
            self.gain_db,
        );
        if wanted
            != (
                self.daemon_last_center_hz,
                self.daemon_last_sample_rate_hz,
                self.daemon_last_gain_db,
            )
        {
            self.daemon_requested_controls = Some(wanted);
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
        self.sync_daemon_session();
        if !self.daemon_session_ready || self.daemon_channel_error.is_some() {
            return;
        }
        // Wait for the actual wideband capture before constructing ADS-B.
        // Old workflow removals still advance while hardware is pending.
        let adsb_ready = self.daemon_applied_capture.as_ref().is_some_and(|hw| {
            hw.connected && hw.frequency_hz == 1_090_000_000 && hw.sample_rate_hz == 2_400_000
        });
        let commands = self.daemon_workflow_commands(
            audio_running && !adsb_running,
            adsb_running && adsb_ready,
            meteor_lrpt,
            demod_mode,
        );
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
        // Wait for the old pipeline teardown acknowledgement before reusing
        // its session ID. A delayed removal must never retire its replacement.
        let audio_running = audio_running
            && !self
                .daemon_removals_pending
                .contains(&self.daemon_channels.audio);
        let adsb_running = adsb_running
            && !self
                .daemon_removals_pending
                .contains(&self.daemon_channels.adsb);
        let meteor_lrpt = meteor_lrpt
            && !self
                .daemon_removals_pending
                .contains(&self.daemon_channels.lrpt);

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
                    id: self.daemon_channels.audio,
                    center_offset_hz: wanted_audio_tuning.map_or(0, |tuning| tuning.offset_hz),
                    bandwidth_hz: wanted_audio_tuning.map_or(0, |tuning| tuning.bandwidth_hz),
                    kind: ez_proto::PipelineKind::Audio,
                    demod_mode: Some(mode),
                },
            }),
            (Some(_), None) => {
                commands.push(ez_proto::ClientCommand::Unsubscribe {
                    channel_id: self.daemon_channels.audio,
                });
                commands.push(ez_proto::ClientCommand::RemoveChannel {
                    channel_id: self.daemon_channels.audio,
                });
            }
            (Some(current), Some(mode)) if current != mode => {
                commands.push(ez_proto::ClientCommand::SetDemodMode {
                    channel_id: self.daemon_channels.audio,
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
                    channel_id: self.daemon_channels.audio,
                    center_offset_hz: tuning.offset_hz,
                    bandwidth_hz: tuning.bandwidth_hz,
                });
            }
        }
        if self.daemon_audio_mode.is_some() && wanted_audio_mode.is_none() {
            self.reset_daemon_squelch();
        }
        self.daemon_audio_mode = wanted_audio_mode;
        self.daemon_audio_tuning = wanted_audio_tuning;

        if adsb_running && !self.daemon_adsb_subscribed {
            commands.push(ez_proto::ClientCommand::Subscribe {
                channel: ez_proto::ChannelSpec {
                    id: self.daemon_channels.adsb,
                    center_offset_hz: 0,
                    bandwidth_hz: 2_400_000,
                    kind: ez_proto::PipelineKind::AdsbPackets,
                    demod_mode: None,
                },
            });
        } else if !adsb_running && self.daemon_adsb_subscribed {
            commands.push(ez_proto::ClientCommand::Unsubscribe {
                channel_id: self.daemon_channels.adsb,
            });
            commands.push(ez_proto::ClientCommand::RemoveChannel {
                channel_id: self.daemon_channels.adsb,
            });
        }
        if self.daemon_adsb_subscribed != adsb_running {
            self.daemon_adsb_confirmed = false;
        }
        self.daemon_adsb_subscribed = adsb_running;

        if meteor_lrpt && !self.daemon_lrpt_subscribed {
            commands.push(ez_proto::ClientCommand::Subscribe {
                channel: ez_proto::ChannelSpec {
                    id: self.daemon_channels.lrpt,
                    center_offset_hz: 0,
                    bandwidth_hz: 288_000,
                    kind: ez_proto::PipelineKind::LrptTelemetry,
                    demod_mode: None,
                },
            });
        } else if !meteor_lrpt && self.daemon_lrpt_subscribed {
            commands.push(ez_proto::ClientCommand::Unsubscribe {
                channel_id: self.daemon_channels.lrpt,
            });
            commands.push(ez_proto::ClientCommand::RemoveChannel {
                channel_id: self.daemon_channels.lrpt,
            });
        }
        self.daemon_lrpt_subscribed = meteor_lrpt;
        for command in &commands {
            if let ez_proto::ClientCommand::RemoveChannel { channel_id } = command {
                self.daemon_removals_pending.insert(*channel_id);
            }
        }
        commands
    }

    /// Stop the SDR source.
    ///
    /// Requests cancellation without waiting for device/network/file I/O.
    /// Status stays Stopping until the worker releases its resources.
    pub fn stop(&mut self) {
        if self.daemon_recording_active {
            if let Some(client) = &self.daemon_client {
                client.send(ez_proto::ClientCommand::StopRecording {
                    channel_id: self.daemon_channels.spectrum,
                });
            }
        }
        // Connection teardown can wait on network I/O too. Retain ownership on
        // a background worker until it has actually closed.
        if let Some(client) = self.daemon_client.take() {
            self.worker_handle = Some(std::thread::spawn(move || drop(client)));
        }
        self.daemon_session_ready = false;
        self.daemon_applied_capture = None;
        self.daemon_requested_controls = None;
        self.daemon_adsb_confirmed = false;
        self.daemon_channel_error = None;
        self.daemon_removals_pending.clear();
        self.daemon_audio_mode = None;
        self.daemon_audio_tuning = None;
        self.local_capture_accepted = false;
        self.reset_daemon_squelch();
        self.daemon_adsb_subscribed = false;
        self.daemon_lrpt_subscribed = false;
        self.daemon_recording_active = false;
        self.running.store(false, Ordering::SeqCst);
        self.restart_requested = false;
        if let Some(handle) = &self.worker_handle {
            handle.thread().unpark();
        }
        // Recreate channel for next start()
        let (new_tx, new_rx) = bounded(8);
        self.tx = Some(new_tx);
        self.rx = Some(new_rx);
        self.status = if self.worker_handle.is_some() {
            SourceStatus::Stopping
        } else {
            SourceStatus::Idle
        };
        self.poll_shutdown();
    }

    /// Join only completed workers. A driver that never returns keeps its
    /// ownership and Stopping status without blocking the event loop.
    pub(crate) fn poll_shutdown(&mut self) {
        if self
            .worker_handle
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            let result = self.worker_handle.take().unwrap().join();
            if self.status == SourceStatus::Stopping {
                self.status = if result.is_ok() {
                    SourceStatus::Idle
                } else {
                    SourceStatus::Error("Receiver worker panicked during shutdown".into())
                };
            }
        }
        if self.restart_requested && self.worker_handle.is_none() {
            self.restart_requested = false;
            self.start();
        }
    }

    #[cfg(test)]
    pub(crate) fn wait_for_shutdown(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.status == SourceStatus::Stopping && Instant::now() < deadline {
            self.poll_shutdown();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_ne!(
            self.status,
            SourceStatus::Stopping,
            "Source shutdown timed out"
        );
    }

    /// Try to receive a pending chunk of IQ samples from the source thread.
    ///
    /// Returns `None` if no samples are available (non-blocking).
    #[cfg(test)]
    pub fn recv_samples(&mut self) -> Option<Vec<u8>> {
        self.recv_capture_block().map(|block| block.samples)
    }

    pub(crate) fn recv_capture_block(&mut self) -> Option<CaptureBlock> {
        self.poll_shutdown();
        if self.status == SourceStatus::Stopping {
            return None;
        }
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(SourceMessage::Samples(block)) => {
                    self.local_capture_accepted = true;
                    self.status = SourceStatus::Running;
                    if let Some(file_position) = block.file_position {
                        self.replay_position = file_position.min(self.replay_size);
                    }
                    Some(block)
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
                ui.add_enabled_ui(
                    !matches!(self.status, SourceStatus::Running | SourceStatus::Opening),
                    |ui| {
                        egui::ComboBox::from_id_salt("cf32_amplitude")
                            .selected_text(match self.replay_cf32_amplitude {
                                None => "CF32: metadata / normalized",
                                Some(lrpt_decode::Cf32Amplitude::Normalized) => "CF32: normalized",
                                Some(lrpt_decode::Cf32Amplitude::AdcCounts127_4) => {
                                    "CF32: legacy ADC counts"
                                }
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.replay_cf32_amplitude,
                                    None,
                                    "Metadata / normalized default",
                                );
                                ui.selectable_value(
                                    &mut self.replay_cf32_amplitude,
                                    Some(lrpt_decode::Cf32Amplitude::Normalized),
                                    "Normalized full scale",
                                );
                                ui.selectable_value(
                                    &mut self.replay_cf32_amplitude,
                                    Some(lrpt_decode::Cf32Amplitude::AdcCounts127_4),
                                    "Legacy ADC counts (byte minus 127.4)",
                                );
                            });
                    },
                );
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
                SourceStatus::Stopping => (egui::Color32::YELLOW, "Stopping..."),
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

#[cfg(any(test, feature = "rtlsdr"))]
fn rtl_ppm_result_applied(result: i32) -> bool {
    result >= 0 || result == -2
}

// SAFETY: This function calls raw FFI (`rtlsdr_open`, etc.) and must only be
// called when `feature = "rtlsdr"` is active and a real RTL-SDR device is
// available. The caller must ensure the returned pointer is eventually closed
// with `rtl_sdr_close`.
#[cfg(all(feature = "rtlsdr", not(test)))]
#[allow(clippy::too_many_arguments)]
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
        return cleanup_and_fail("RTL AGC mode");
    }
    // librtlsdr returns -2 when the exact requested correction is already set.
    // https://github.com/osmocom/rtl-sdr/blob/master/src/librtlsdr.c
    let ppm_result = unsafe { rtlsdr_sys::rtlsdr_set_freq_correction(dev, ppm) };
    if !rtl_ppm_result_applied(ppm_result) {
        return cleanup_and_fail("frequency correction");
    }
    let bias_on = if bias { 1 } else { 0 };
    if unsafe { rtlsdr_set_bias_tee(dev, bias_on) } < 0 {
        return cleanup_and_fail("bias tee");
    }
    if unsafe { rtlsdr_sys::rtlsdr_reset_buffer(dev) } < 0 {
        return cleanup_and_fail("USB sample buffer reset");
    }
    Ok(dev)
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

    #[test]
    fn native_callback_acquisition_clock_is_preserved_before_preview_loss_and_stop() {
        let stream = crate::capture_stream::CaptureStream::default();
        stream.start(17);
        let session = stream.session();
        let consumer = stream.subscribe(8);
        let mut producer = CaptureProducer::new(2_400_000, 137_100_000).attach_stream(
            stream,
            17,
            SourceMode::Hardware,
        );
        let first = ez_proto::CaptureClock::new(987, 0, 2_400_000, None);
        producer
            .block_from_acquisition([128, 129].repeat(64), first)
            .unwrap();
        let mut after = first.offset_samples(96).unwrap();
        after.discontinuous = true;
        let block = producer
            .block_from_acquisition([130, 131].repeat(32), after)
            .unwrap();
        assert_eq!(block.clock.source_sample_index, 96);
        assert!(block.clock.discontinuous);
        let a = consumer.recv_timeout(Duration::from_secs(1)).unwrap();
        let b = consumer.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(a.clock, first);
        assert_eq!(b.clock, after);
        assert_eq!(b.source_generation, 17);
        session.finish();
    }

    #[test]
    fn replay_capture_positions_and_loop_boundaries_are_identical_at_all_speeds() {
        use crate::adsb_decoder::{capture_tests::position_messages, AdsBDecoder};
        use ez_daemon::hardware::adsb::encode_uc8;
        let [odd, even] = position_messages(0x3c4b12);
        let odd = encode_uc8(&[odd]);
        let even = encode_uc8(&[even]);
        let mut raw = vec![127; 2 * 65_536];
        raw[..odd.len()].copy_from_slice(&odd);
        let end = raw.len();
        raw[end - even.len()..].copy_from_slice(&even);
        let recording = TestRecording::new(&raw);
        let mut positions = Vec::new();
        for speed in [0.1, 1.0, 10.0] {
            let mut source = recording.source(1);
            source.sample_rate_hz = 2_400_000;
            source.frequency_hz = 1_090_000_000;
            source.replay_speed = speed;
            source.replay_loop = true;
            source.start();
            let generation = source.stream_generation();
            let mut decoder = AdsBDecoder::new();
            let mut prior_epoch = None;
            for iteration in 0..3 {
                let mut epoch = 0;
                for block_number in 0..2 {
                    let mut block = None;
                    assert!(
                        wait_until(|| {
                            block = source.recv_capture_block();
                            block.is_some()
                        }),
                        "replay stalled at {speed}x"
                    );
                    let block = block.unwrap();
                    assert_eq!(source.stream_generation(), generation);
                    assert_eq!(block.center_hz, 1_090_000_000);
                    assert_eq!(block.clock.source_sample_index, block_number * 32_768);
                    assert_eq!(block.clock.source_sample_rate_hz, 2_400_000);
                    assert_eq!(block.clock.sample_stride, 1);
                    assert_eq!(block.clock.origin_unix_ns, None);
                    assert_eq!(block.clock.discontinuous, block_number == 0);
                    assert_eq!(source.replay_position, (block_number + 1) * 65_536);
                    if block_number == 0 {
                        epoch = block.clock.epoch;
                        assert_ne!(Some(epoch), prior_epoch);
                    } else {
                        assert_eq!(block.clock.epoch, epoch);
                    }
                    decoder.feed_iq_with_clock(
                        &block.samples,
                        block.clock,
                        dump1090::demod::InputFormat::Uc8,
                    );
                    let aircraft = decoder.get_aircraft();
                    assert_eq!(aircraft.len(), 1);
                    assert_eq!(
                        aircraft[0].has_position,
                        block_number == 1,
                        "a frame crossed a rewind at {speed}x in loop {iteration}"
                    );
                    if block_number == 1 {
                        positions.push((aircraft[0].lat, aircraft[0].lon));
                        assert!((aircraft[0].lat - 52.2572).abs() < 0.001);
                        assert!((aircraft[0].lon - 8.6676).abs() < 0.001);
                    }
                }
                prior_epoch = Some(epoch);
            }
            assert_eq!(source.dropped_blocks.load(Ordering::Relaxed), 0);
            source.stop();
            source.wait_for_shutdown();
        }
        assert!(positions.windows(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn capture_producer_retains_partial_samples_and_marks_only_real_boundaries() {
        let mut producer = CaptureProducer::new(2_400_000, 1090);
        assert!(producer.block(vec![1], None).is_none());
        let first = producer.block(vec![2, 3, 4, 5], Some(5)).unwrap();
        assert_eq!(first.samples, [1, 2, 3, 4]);
        assert_eq!(first.clock.source_sample_index, 0);
        assert!(first.clock.discontinuous);
        let second = producer.block(vec![6], Some(6)).unwrap();
        assert_eq!(second.samples, [5, 6]);
        assert_eq!(second.clock.source_sample_index, 2);
        assert!(!second.clock.discontinuous);
        assert_eq!(second.clock.epoch, first.clock.epoch);
        producer.rewind();
        let rewind = producer.block(vec![7, 8], Some(2)).unwrap();
        assert_ne!(rewind.clock.epoch, first.clock.epoch);
        assert_eq!(rewind.clock.source_sample_index, 0);
        assert!(rewind.clock.discontinuous);
    }
    use super::*;

    async fn session_event(
        source: &mut SourceManager,
        what: &str,
        predicate: impl Fn(&ez_proto::ServerEvent) -> bool,
    ) -> ez_proto::ServerEvent {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            for _ in 0..64 {
                let Some(event) = source.recv_daemon_event() else {
                    break;
                };
                if let ez_proto::ServerEvent::Error { message } = &event {
                    panic!("{what}: {message}");
                }
                if predicate(&event) {
                    return event;
                }
            }
            assert!(
                !matches!(source.status, SourceStatus::Error(_)),
                "{what}: {:?}",
                source.status
            );
            assert!(
                Instant::now() < deadline,
                "timed out {what}: {:?}",
                source.status
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn daemon_sessions_isolate_gui_mode_stop_disconnect_and_recording() {
        use crate::sdr_panel::DemodMode;
        use ez_proto::ServerEvent as E;
        let daemon =
            crate::daemon_client::tests::TestDaemon::spawn_with_rate("two-gui", 2_400_000).await;
        let mut a = SourceManager::new();
        let mut b = SourceManager::new();
        for (index, source) in [&mut a, &mut b].into_iter().enumerate() {
            source.source_mode = SourceMode::Daemon;
            source.daemon_addr = daemon.addr.to_string();
            source.start();
            session_event(source, "initial spectrum", |e| matches!(e, E::Spectrum(_))).await;
            if index == 0 {
                // Establish the shared RF acquisition before either GUI starts
                // recording. ADS-B requires real 1090 MHz / 2.4 MSps state.
                source.tune_and_restart(1_090_000_000, 2_400_000);
                session_event(source, "ADS-B acquisition", |e|
                    matches!(e, E::Hardware(hw) if hw.frequency_hz == 1_090_000_000 && hw.sample_rate_hz == 2_400_000)).await;
            }
            source.sync_daemon_workflows(true, false, false, DemodMode::Fm);
            session_event(source, "initial audio", |e| matches!(e, E::Audio(_))).await;
        }
        let a_ids = a.daemon_channel_ids();
        let b_ids = b.daemon_channel_ids();
        assert!(a_ids
            .ids()
            .iter()
            .all(|id| *id >= ez_proto::CLIENT_CHANNEL_START));
        assert!(a_ids.ids().iter().all(|id| !b_ids.contains(*id)));
        let observer =
            crate::daemon_client::DaemonClient::connect(daemon.addr, "foreign-observer".into());
        let deadline = Instant::now() + Duration::from_secs(5);
        while observer.client_channels().is_none() {
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        observer.send(ez_proto::ClientCommand::RemoveChannel {
            channel_id: b_ids.audio,
        });
        observer.send(ez_proto::ClientCommand::SetSquelch {
            request_id: 17,
            channel_id: b_ids.audio,
            db: 0.0,
        });
        observer.send(ez_proto::ClientCommand::BeginSpectrumObservation {
            channel_id: b_ids.spectrum,
            revision: 91,
        });
        let mut channel_errors = 0;
        let mut squelch_rejected = false;
        while channel_errors < 2 || !squelch_rejected {
            match observer.try_recv_event() {
                Some(E::Error { message }) => {
                    assert!(message.contains("another connection"));
                    channel_errors += 1;
                }
                Some(E::Squelch {
                    request_id: 17,
                    result: Err(message),
                    ..
                }) => {
                    assert!(message.contains("another connection"));
                    squelch_rejected = true;
                }
                _ => {}
            }
            assert!(
                Instant::now() < deadline,
                "TCP foreign control wasn't rejected"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        drop(observer);
        b.start_daemon_recording(ez_proto::RecordingFormat::Cf32)
            .unwrap();
        let E::Recording(before) = session_event(
            &mut b,
            "recording started",
            |e| matches!(e, E::Recording(s) if s.active && s.bytes_written > 0),
        )
        .await
        else {
            unreachable!()
        };
        // Both managers traverse the production workflow dispatcher. A changes
        // demodulation, stops audio, switches to ADS-B, then stops that decoder.
        a.sync_daemon_workflows(true, false, false, DemodMode::Am);
        session_event(&mut a, "changed audio mode", |e| matches!(e, E::Audio(_))).await;
        a.sync_daemon_workflows(false, true, false, DemodMode::Am);
        session_event(&mut a, "ADS-B snapshot", |e| matches!(e, E::Aircraft(_))).await;
        a.sync_daemon_workflows(false, false, false, DemodMode::Am);
        a.stop();
        let E::Audio(frame) = session_event(&mut b, "other GUI audio survives", |e| {
            matches!(e, E::Audio(_))
        })
        .await
        else {
            unreachable!()
        };
        assert_eq!(frame.channel_id, b_ids.audio);
        assert!(b.accept_daemon_audio(&frame));
        let E::Recording(after) = session_event(
            &mut b,
            "recording continues",
            |e| matches!(e, E::Recording(s) if s.active && s.bytes_written > before.bytes_written),
        )
        .await
        else {
            unreachable!()
        };
        assert_eq!(after.channel_id, b_ids.spectrum);
        // Inspect the actual channel table via the public web API after cleanup.
        let url = format!("http://{}/api/channels", daemon.web_addr);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let channels: Vec<ez_daemon::state::ChannelMetrics> =
                reqwest::get(&url).await.unwrap().json().await.unwrap();
            if channels.iter().all(|c| !a_ids.contains(c.spec.id)) {
                assert!(channels.iter().any(|c| c.spec.id == b_ids.audio));
                assert!(channels.iter().any(|c| c.spec.id == b_ids.spectrum));
                break;
            }
            assert!(Instant::now() < deadline, "A's private pipelines leaked");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        b.stop_daemon_recording().unwrap();
        let E::Recording(done) = session_event(
            &mut b,
            "recording completed",
            |e| matches!(e, E::Recording(s) if !s.active),
        )
        .await
        else {
            unreachable!()
        };
        assert!(done.bytes_written >= after.bytes_written);
        assert_eq!(
            std::fs::metadata(done.path.unwrap()).unwrap().len(),
            done.bytes_written
        );
        b.stop();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let channels: Vec<ez_daemon::state::ChannelMetrics> =
                reqwest::get(&url).await.unwrap().json().await.unwrap();
            if channels.is_empty() {
                break;
            }
            assert!(Instant::now() < deadline, "B's private pipelines leaked");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        daemon.shutdown().await;
    }

    #[test]
    fn daemon_session_filters_foreign_data_and_waits_for_own_teardown() {
        use ez_proto::{ServerEvent as E, *};
        let (client, _commands, events) = crate::daemon_client::DaemonClient::test_connection();
        let mut source = SourceManager::new();
        source.source_mode = SourceMode::Daemon;
        source.daemon_client = Some(client);
        source.daemon_session_ready = true;
        source.daemon_audio_mode = Some(ez_proto::DemodMode::Fm);
        source.daemon_adsb_subscribed = true;
        source.daemon_lrpt_subscribed = true;
        let foreign = 99;
        for event in [
            E::Spectrum(SpectrumFrame {
                channel_id: foreign,
                center_hz: 100_000_000,
                sample_rate_hz: 2_000_000,
                bins: vec![0.0],
                timestamp_ms: 1,
                observation_revision: 0,
                measurement_bins: None,
            }),
            E::Audio(AudioFrame {
                clock: ez_proto::CaptureClock::new(1, 0, 48_000, None),
                sequence: 1,
                channel_id: foreign,
                sample_rate_hz: 48000,
                samples: vec![1.0],
                squelch: SquelchState {
                    db: -100.0,
                    revision: 1,
                },
            }),
            E::Aircraft(AircraftSnapshot {
                channel_id: foreign,
                ..Default::default()
            }),
            E::Telemetry(TelemetryFrame {
                channel_id: foreign,
                apid: 1,
                width: 1,
                height: 1,
                pixels: vec![0],
                rs_ok: 0,
                rs_failed: 0,
                costas_locked: false,
                frame_locked: false,
            }),
            E::Recording(RecordingStatus {
                stop_reason: None,
                channel_id: foreign,
                active: true,
                path: None,
                bytes_written: 0,
                duration_sec: 0.0,
            }),
        ] {
            events.send(event).unwrap();
        }
        events.send(E::Pong { nonce: 77 }).unwrap();
        assert!(matches!(
            source.recv_daemon_event(),
            Some(E::Pong { nonce: 77 })
        ));
        assert!(!source.daemon_recording_active);
        assert!(source.daemon_squelch_applied().is_none());
        source.daemon_workflow_commands(false, false, false, crate::sdr_panel::DemodMode::Fm);
        assert!(source
            .daemon_workflow_commands(true, false, false, crate::sdr_panel::DemodMode::Fm)
            .is_empty());
        events
            .send(E::ChannelRemoved {
                channel_id: source.daemon_channels.audio,
            })
            .unwrap();
        let _ = source.recv_daemon_event();
        assert_eq!(source.status, SourceStatus::Running);
        assert!(source
            .daemon_workflow_commands(true, false, false, crate::sdr_panel::DemodMode::Fm)
            .iter()
            .any(|c| matches!(c, ClientCommand::Subscribe { .. })));
        events
            .send(E::ChannelRemoved {
                channel_id: source.daemon_channels.audio,
            })
            .unwrap();
        let _ = source.recv_daemon_event();
        assert!(matches!(&source.status, SourceStatus::Error(e) if e.contains("removed")));
        assert!(source.daemon_audio_mode.is_none());
        let _ = source.recv_daemon_event();
        assert!(matches!(&source.status, SourceStatus::Error(_)));
    }

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
        source.wait_for_shutdown();

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
        source.wait_for_shutdown();
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
        source.wait_for_shutdown();
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
    fn daemon_selected_gain_step_acknowledges_original_request_and_updates_ui() {
        let mut source = SourceManager::new();
        source.source_mode = SourceMode::Daemon;
        source.daemon_requested_controls = Some((100_000_000, 2_048_000, 29.9));
        source.gain_db = 29.9;
        let hardware = ez_proto::HardwareStatus {
            connected: true,
            source_kind: "rtlsdr".into(),
            frequency_hz: 100_000_000,
            sample_rate_hz: 2_048_000,
            gain_db: 29.7,
            requested_gain_db: Some(29.9),
            gain_mode: Some(ez_proto::GainMode::Manual),
            error: None,
        };
        source.apply_daemon_hardware(&hardware);
        assert!(source.daemon_requested_controls.is_none());
        assert_eq!(source.gain_db, 29.7);
        assert!(source.gain_request_applied(29.9));
        assert!(source.gain_request_applied(29.7));
        assert!(!source.gain_request_applied(40.0));
    }

    #[test]
    fn daemon_hardware_updates_preserve_independent_vfo_without_control_echo() {
        let hardware = ez_proto::HardwareStatus {
            connected: true,
            source_kind: "RTL-SDR".into(),
            frequency_hz: 118_000_000,
            sample_rate_hz: 2_048_000,
            gain_db: 40.0,
            requested_gain_db: None,
            gain_mode: None,
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
    fn labels_blog_v4_from_usb_identity_without_mislabeling_v3_or_generic_dongles() {
        assert_eq!(
            rtl_device_name(
                "Generic RTL2832U OEM".into(),
                Some("custom-serial"),
                Some("Blog V4"),
                Some("RTLSDRBlog"),
            ),
            "RTL-SDR Blog V4"
        );
        assert_eq!(
            rtl_device_name(
                "Generic RTL2832U OEM".into(),
                Some("00000001"),
                None,
                Some("RTLSDRBlog"),
            ),
            "RTL-SDR Blog V4"
        );
        assert_eq!(
            rtl_device_name(
                "Generic RTL2832U OEM".into(),
                Some("custom-serial"),
                Some("Blog V4"),
                None,
            ),
            "RTL-SDR Blog V4"
        );
        assert_eq!(
            rtl_device_name(
                "Generic RTL2832U OEM".into(),
                Some("00000000"),
                Some("Blog V3"),
                Some("RTLSDRBlog"),
            ),
            "Generic RTL2832U OEM"
        );
        assert_eq!(
            rtl_device_name("Generic RTL2832U OEM".into(), None, None, None),
            "Generic RTL2832U OEM"
        );
    }

    #[test]
    fn single_discovered_receiver_is_selected_when_no_serial_was_saved() {
        let mut source = SourceManager::new();
        let discovered = rtl_device(3, Some("only-receiver"));
        assert!(source.refresh_rtl_devices_with(move || Ok(vec![discovered])));
        assert!(wait_until(|| source.poll_rtl_devices()));
        assert_eq!(source.rtl_device.index, 3);
        assert_eq!(source.rtl_device.serial.as_deref(), Some("only-receiver"));
        assert_eq!(source.selected_rtl_device().unwrap().index, 3);
        assert!(source.rtl_device_refresh_error.is_none());
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
        source.wait_for_shutdown();
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
        source.wait_for_shutdown();
        source.start();
        source.stop();
        source.wait_for_shutdown();
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
    fn both_cf32_writers_replay_adc_levels_tones_and_adsb_without_clipping() {
        use ez_daemon::broadcast::OverflowPolicy;
        use ez_daemon::bus::{SampleBlock, SampleBus};
        use ez_daemon::hardware::{
            adsb::{encode_uc8, EXAMPLE_DF17_CALLSIGN},
            replay::{FileReplaySource, ReplayFormat},
            IqSource,
        };
        let mut raw: Vec<u8> = (0..=255).flat_map(|i| [i, 255 - i]).collect();
        for amplitude in [0.01, 0.25, 1.0] {
            for n in 0..4096 {
                let s = num_complex::Complex32::from_polar(amplitude, n as f32 * 0.1);
                raw.extend([
                    lrpt_decode::normalized_component_to_uc8(s.re),
                    lrpt_decode::normalized_component_to_uc8(s.im),
                ]);
            }
        }
        raw.extend(vec![127; 1024]);
        raw.extend(encode_uc8(&[EXAMPLE_DF17_CALLSIGN; 3]));
        let dir =
            std::env::temp_dir().join(format!("ez-cf32-path-roundtrip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut satellite = crate::satellite::recorder::Cf32StreamWriter::start(
            dir.to_str().unwrap(),
            None,
            2_400_000,
            1_090_000_000,
            0.0,
            0.0,
        )
        .unwrap();
        satellite.write(&raw).unwrap();
        satellite.stop().unwrap();
        let satellite_path = satellite.path().to_path_buf();
        let bus = SampleBus::new();
        let handle = bus.subscribe(8, OverflowPolicy::DropIncoming);
        let mut manager = ez_daemon::recording::RecordingManager::new(&dir);
        manager
            .start_recording(1, ez_proto::RecordingFormat::Cf32, 1_090_000_000, handle)
            .unwrap();
        bus.publish(SampleBlock {
            observed_at: None,
            capture: None,
            start_sample: 0,
            sample_rate_hz: 2_400_000,
            center_freq_hz: 1_090_000_000,
            samples: Arc::from(lrpt_decode::uc8_bytes_to_normalized(&raw)),
        });
        assert!(wait_until(
            || manager.status(1).unwrap().bytes_written == raw.len() as u64 * 4
        ));
        let daemon_path =
            std::path::PathBuf::from(manager.stop_recording(1).unwrap().path.unwrap());
        let mut original_decoder = crate::adsb_decoder::AdsBDecoder::new();
        original_decoder.feed_iq(&raw, 2_400_000);
        assert_eq!(original_decoder.total_messages, 3);
        for path in [satellite_path, daemon_path] {
            let metadata = lrpt_decode::cf32_amplitude_for_path(&path, None).unwrap();
            assert_eq!(metadata, lrpt_decode::Cf32Amplitude::Normalized);
            let mut source = SourceManager::new();
            source.source_mode = SourceMode::Replay;
            source.replay_file = Some(path.to_string_lossy().into_owned());
            source.sample_rate_hz = u32::MAX;
            source.start();
            let mut replayed = Vec::new();
            assert!(wait_until(|| {
                if let Some(chunk) = source.recv_samples() {
                    replayed.extend(chunk);
                }
                source.status == SourceStatus::Idle
            }));
            source.stop();
            source.wait_for_shutdown();
            assert_eq!(replayed, raw);
            let mut decoder = crate::adsb_decoder::AdsBDecoder::new();
            decoder.feed_iq(&replayed, 2_400_000);
            assert_eq!(decoder.total_messages, original_decoder.total_messages);
            assert_eq!(
                decoder.get_aircraft()[0].callsign,
                original_decoder.get_aircraft()[0].callsign
            );
            let mut daemon_reader = FileReplaySource::open(
                path.to_str().unwrap(),
                ReplayFormat::Cf32Le,
                u32::MAX,
                false,
                1000.0,
            )
            .unwrap();
            daemon_reader.start().unwrap();
            let mut buffer = vec![num_complex::Complex32::new(0.0, 0.0); raw.len() / 2];
            assert_eq!(daemon_reader.read_iq(&mut buffer).unwrap(), raw.len() / 2);
            let converted: Vec<u8> = buffer
                .iter()
                .flat_map(|s| {
                    [
                        lrpt_decode::normalized_component_to_uc8(s.re),
                        lrpt_decode::normalized_component_to_uc8(s.im),
                    ]
                })
                .collect();
            assert_eq!(converted, raw);
            daemon_reader.stop();
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn native_float_replay_preserves_weak_iq_headroom_and_clock() {
        let values = [0.0001_f32, 0.0002, 1.2, -1.3, -0.0003, 0.0004];
        let bytes: Vec<_> = values.into_iter().flat_map(f32::to_le_bytes).collect();
        let mut recording = TestRecording::new(&bytes);
        let path = recording.0.with_extension("cf32");
        std::fs::rename(&recording.0, &path).unwrap();
        recording.0 = path;
        let mut source = recording.source(1);
        source.replay_loop = false;
        let stream = source.capture_stream();
        let consumer = stream.subscribe(8);
        source.start();
        let mut block = None;
        assert!(wait_until(|| {
            block = consumer.try_recv();
            block.is_some()
        }));
        let block = block.unwrap();
        source.stop();
        source.wait_for_shutdown();
        let expected: Vec<_> = values
            .chunks_exact(2)
            .map(|pair| num_complex::Complex32::new(pair[0], pair[1]))
            .collect();
        assert_eq!(block.float_samples.as_deref().unwrap(), expected);
        assert_eq!(block.clock.source_sample_index, 0);
        assert_eq!(block.stream_sample_index, 0);
        let mut dsp = crate::radio_iq::RadioIqProcessor::new(crate::radio_iq::RadioIqConfig {
            input_rate: block.clock.source_sample_rate_hz,
            ..Default::default()
        });
        assert_eq!(
            dsp.process_complex(block.float_samples.as_deref().unwrap()),
            expected
        );
        assert_eq!(block.samples, [128, 128, 255, 0, 127, 128]);
        assert!(!block.session.as_ref().unwrap().progress().live);
    }

    #[test]
    fn legacy_cf32_import_has_explicit_units_and_no_peak_based_guessing() {
        let original = [90u8, 110, 127, 128, 140, 160, 180, 200];
        let bytes: Vec<u8> = original
            .iter()
            .flat_map(|v| (f32::from(*v) - 127.4).to_le_bytes())
            .collect();
        let mut recording = TestRecording::new(&bytes);
        let path = recording.0.with_extension("cf32");
        std::fs::rename(&recording.0, &path).unwrap();
        recording.0 = path;
        let mut source = recording.source(2);
        source.replay_cf32_amplitude = Some(lrpt_decode::Cf32Amplitude::AdcCounts127_4);
        source.start();
        let mut replayed = Vec::new();
        assert!(wait_until(|| {
            if let Some(chunk) = source.recv_samples() {
                replayed.extend(chunk);
            }
            source.status == SourceStatus::Idle
        }));
        source.stop();
        source.wait_for_shutdown();
        assert_eq!(replayed, original);
        use ez_daemon::hardware::{
            replay::{FileReplaySource, ReplayFormat},
            IqSource,
        };
        let mut reader = FileReplaySource::open_with_amplitude(
            recording.0.to_str().unwrap(),
            ReplayFormat::Cf32Le,
            u32::MAX,
            false,
            1000.0,
            Some(lrpt_decode::Cf32Amplitude::AdcCounts127_4),
        )
        .unwrap();
        reader.start().unwrap();
        let mut samples = [num_complex::Complex32::new(0.0, 0.0); 4];
        assert_eq!(reader.read_iq(&mut samples).unwrap(), 4);
        let replayed: Vec<u8> = samples
            .iter()
            .flat_map(|s| {
                [
                    lrpt_decode::normalized_component_to_uc8(s.re),
                    lrpt_decode::normalized_component_to_uc8(s.im),
                ]
            })
            .collect();
        assert_eq!(replayed, original);
    }

    #[test]
    fn replay_progress_tracks_input_bytes_at_each_block_and_before_eof() {
        for cf32 in [false, true] {
            let input = vec![0; 4 * 65_536 + 8];
            let mut recording = TestRecording::new(&input);
            if cf32 {
                let path = recording.0.with_extension("cf32");
                std::fs::rename(&recording.0, &path).unwrap();
                recording.0 = path;
            }
            let mut source = recording.source(1);
            source.start();
            for expected_position in [65_536, 131_072, 196_608, 262_144, 262_152] {
                let mut delivered = None;
                assert!(wait_until(|| {
                    delivered = source.recv_samples();
                    delivered.is_some()
                }));
                assert_eq!(source.replay_position, expected_position);
                assert_eq!(source.status, SourceStatus::Running);
                assert!(source.replay_position <= source.replay_size);
                let input_len = if expected_position == 262_152 {
                    8
                } else {
                    65_536
                };
                assert_eq!(
                    delivered.unwrap().len(),
                    input_len / if cf32 { 4 } else { 1 }
                );
            }
            assert_eq!(source.replay_position, source.replay_size); // reaches 100% before EOF is consumed
            assert!(wait_until(|| {
                source.recv_samples();
                source.status == SourceStatus::Idle
            }));
            source.stop();
            source.wait_for_shutdown();
        }
    }

    #[test]
    fn looped_cf32_progress_is_the_current_iteration_file_offset() {
        let mut recording = TestRecording::new(&vec![0; 65_536 + 8]);
        let path = recording.0.with_extension("cf32");
        std::fs::rename(&recording.0, &path).unwrap();
        recording.0 = path;
        let mut source = recording.source(1);
        source.replay_loop = true;
        source.start();
        for expected_position in [65_536, 65_544, 65_536, 65_544, 65_536, 65_544] {
            assert!(wait_until(|| source.recv_samples().is_some()));
            assert_eq!(source.replay_position, expected_position);
            assert_eq!(source.status, SourceStatus::Running);
        }
        source.stop();
        source.wait_for_shutdown();
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
        assert_eq!(output, [128, 128, 128, 128, 255, 0, 191, 64]);
        source.stop();
        source.wait_for_shutdown();
    }

    #[test]
    fn live_queue_full_discards_a_block_without_disconnect() {
        let (tx, rx) = bounded(1);
        let dropped = std::sync::atomic::AtomicU64::new(0);
        let mut capture = CaptureProducer::new(2_400_000, 1_090_000_000);
        assert!(send_live_samples(&tx, &mut capture, vec![1, 2], &dropped));
        assert!(send_live_samples(&tx, &mut capture, vec![3, 4], &dropped));
        assert!(
            matches!(rx.try_recv(), Ok(SourceMessage::Samples(block)) if block.samples == [1, 2] && block.clock.source_sample_index == 0)
        );
        assert!(send_live_samples(&tx, &mut capture, vec![5, 6], &dropped));
        let SourceMessage::Samples(next) = rx.try_recv().unwrap() else {
            panic!("expected recovered sample block")
        };
        assert_eq!(next.samples, [5, 6]);
        assert_eq!(next.clock.source_sample_index, 2);
        assert!(
            next.clock.discontinuous,
            "overflow must reach the decoder as a real sample gap"
        );
        assert_eq!(dropped.load(Ordering::Relaxed), 1);
        assert!(send_live_samples(&tx, &mut capture, vec![7, 8], &dropped));
        let SourceMessage::Samples(contiguous) = rx.try_recv().unwrap() else {
            panic!("expected next sample block")
        };
        assert_eq!(contiguous.clock.source_sample_index, 3);
        assert!(!contiguous.clock.discontinuous);
        drop(rx);
        assert!(!send_live_samples(&tx, &mut capture, vec![7, 8], &dropped));
    }

    #[test]
    fn preview_overflow_does_not_interrupt_independent_capture_consumers() {
        let stream = crate::capture_stream::CaptureStream::default();
        let input = stream.subscribe(8);
        stream.start(7);
        let (tx, rx) = bounded(1);
        let dropped = std::sync::atomic::AtomicU64::new(0);
        let mut capture = CaptureProducer::new(2_400_000, 1_090_000_000).attach_stream(
            stream.clone(),
            7,
            SourceMode::Hardware,
        );
        for sample in 0..4 {
            assert!(send_live_samples(
                &tx,
                &mut capture,
                vec![sample, 127],
                &dropped
            ));
        }
        assert_eq!(dropped.load(Ordering::Relaxed), 3);
        assert_eq!(rx.len(), 1);
        for index in 0..4 {
            let block = input.try_recv().unwrap();
            assert_eq!(block.clock.source_sample_index, index);
            assert_eq!(block.stream_sample_index, index);
            assert_eq!(block.clock.discontinuous, index == 0);
            assert_eq!(block.source_generation, 7);
            assert_eq!(block.source_mode, SourceMode::Hardware);
        }
        assert_eq!(input.dropped_count(), 0);
        drop(capture);
        let progress = stream.progress();
        assert_eq!(
            (progress.generation, progress.samples, progress.blocks),
            (7, 4, 4)
        );
        assert!(!progress.live);
    }

    #[test]
    fn independent_consumer_overflow_is_bounded_and_accounts_for_a_lost_tail() {
        let stream = crate::capture_stream::CaptureStream::default();
        let slow = stream.subscribe(1);
        let fast = stream.subscribe(4);
        stream.start(8);
        let mut capture = CaptureProducer::new(2_400_000, 1_090_000_000).attach_stream(
            stream.clone(),
            8,
            SourceMode::Hardware,
        );
        for _ in 0..4 {
            capture.block(vec![127; 8], None).unwrap();
        }
        assert_eq!(slow.queued_len(), 1);
        assert_eq!(slow.dropped_count(), 3);
        assert_eq!(fast.queued_len(), 4);
        assert_eq!(fast.dropped_count(), 0);
        assert_eq!(slow.try_recv().unwrap().stream_sample_index, 0);
        capture.block(vec![127; 8], None).unwrap();
        assert_eq!(slow.try_recv().unwrap().stream_sample_index, 16);
        capture.block(vec![127; 8], None).unwrap();
        capture.block(vec![127; 8], None).unwrap();
        assert_eq!(slow.try_recv().unwrap().stream_sample_index, 20);
        drop(capture);
        assert_eq!(stream.progress().samples, 28);
        assert!(!stream.progress().live);
        assert_eq!(slow.dropped_count(), 4);
        assert_eq!(slow.queued_len(), 0);
        // The final 4 missing samples are discoverable without another read.
        assert_eq!(stream.progress().samples - (20 + 4), 4);
    }

    #[test]
    fn replay_capture_consumers_finish_at_all_speeds_without_gui_reads() {
        let period = crate::local_adsb::tests::crossing_block();
        let raw = period.repeat(20);
        let recording = TestRecording::new(&raw);
        for speed in [0.1, 1.0, 10.0] {
            let mut source = recording.source(1);
            source.sample_rate_hz = 2_400_000;
            source.frequency_hz = 1_090_000_000;
            source.replay_speed = speed;
            let stream = source.capture_stream();
            let decoder = crate::local_adsb::LocalAdsbWorker::new(stream.clone());
            decoder.configure(true, Some((52.0, 4.0)));
            source.start();
            let mut final_snapshot = None;
            assert!(
                wait_until(|| {
                    final_snapshot = decoder.snapshot().filter(|snapshot| snapshot.complete);
                    final_snapshot.is_some()
                }),
                "replay stalled at {speed}x without GUI reads"
            );
            let snapshot = final_snapshot.unwrap();
            assert_eq!(snapshot.received_samples, (raw.len() / 2) as u64);
            assert_eq!(snapshot.source_samples, snapshot.received_samples);
            assert_eq!(snapshot.lost_samples, 0);
            assert_eq!(snapshot.dropped_blocks, 0);
            assert_eq!(snapshot.receiver.total_messages, 19);
            assert!(!stream.progress().live);
            assert!(source.dropped_blocks.load(Ordering::Relaxed) > 0);
            source.stop();
            source.wait_for_shutdown();
        }
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
        source.wait_for_shutdown();
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
        source.wait_for_shutdown();
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
            source.wait_for_shutdown();
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
        source.wait_for_shutdown();
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
        source.wait_for_shutdown();
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
    fn stop_does_not_wait_for_blocked_hardware_open() {
        let mut source = SourceManager::new();
        source.source_mode = SourceMode::Hardware;
        let (entered_tx, entered_rx) = crossbeam_channel::bounded(1);
        let (release_tx, release_rx) = crossbeam_channel::bounded(1);
        source.test_set_hardware_open(move |_| {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(vec![127, 127])
        });
        source.start();
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let subscription = source.capture_stream().subscribe(4);
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(250));
            release_tx.send(()).unwrap();
        });
        let started = Instant::now();
        source.stop();
        let elapsed = started.elapsed();
        assert_eq!(source.status, SourceStatus::Stopping);
        source.source_mode = SourceMode::Simulated;
        source.start();
        assert_eq!(source.status, SourceStatus::Stopping);
        assert_eq!(source.stream_generation(), 1);
        assert!(subscription.try_recv().is_none());
        release.join().unwrap();
        assert!(
            elapsed < Duration::from_millis(50),
            "Stop blocked for {elapsed:?}"
        );
        source.wait_for_shutdown();
        assert_eq!(source.stream_generation(), 2);
        let block = subscription.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(block.source_generation, 2);
        source.stop();
        source.wait_for_shutdown();
        eprintln!(
            "Blocked receiver Stop returned in {} us; restart waited for ownership release",
            elapsed.as_micros()
        );
    }

    #[test]
    fn stop_and_drop_discard_a_read_that_returns_after_cancellation() {
        for drop_source in [false, true] {
            let mut source = SourceManager::new();
            source.running = Arc::new(AtomicBool::new(true));
            source.status = SourceStatus::Running;
            source.capture_stream.start(1);
            let stream = source.capture_stream();
            let subscription = stream.subscribe(4);
            let mut capture = CaptureProducer::new(2_400_000, 1_090_000_000).attach_stream(
                stream.clone(),
                1,
                SourceMode::Hardware,
            );
            capture.running = Some(Arc::clone(&source.running));
            let (release, release_rx) = bounded(1);
            let (finished, finished_rx) = bounded(1);
            source.worker_handle = Some(std::thread::spawn(move || {
                // Stand in for a synchronous device/file read returning late.
                release_rx.recv().unwrap();
                assert!(capture.block(vec![127, 127], None).is_none());
                drop(capture);
                finished.send(()).unwrap();
            }));
            let started = Instant::now();
            source.stop();
            let mut source = Some(source);
            if drop_source {
                drop(source.take());
            }
            let elapsed = started.elapsed();
            assert!(
                elapsed < Duration::from_millis(50),
                "Source cancellation blocked for {elapsed:?}"
            );
            assert!(stream.progress().live); // Device ownership remains pending.
            release.send(()).unwrap();
            finished_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            if let Some(source) = &mut source {
                source.wait_for_shutdown();
            }
            assert!(!stream.progress().live);
            assert_eq!(stream.progress().samples, 0);
            assert_eq!(stream.total_samples(), 0);
            assert!(subscription.try_recv().is_none());
            eprintln!(
                "Late-read cancellation/drop={drop_source}: {} us, no post-cancel RF publication",
                elapsed.as_micros()
            );
        }
    }

    #[test]
    fn stop_on_idle_does_not_panic() {
        let mut sm = SourceManager::new();
        sm.stop();
        sm.wait_for_shutdown();
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
        sm.wait_for_shutdown();
        assert_eq!(sm.status, SourceStatus::Idle);
        assert_eq!(sm.stream_generation(), 1);
        sm.start();
        assert_eq!(sm.stream_generation(), 2);
        sm.stop();
        sm.wait_for_shutdown();
    }

    #[test]
    fn start_idempotent() {
        let mut sm = SourceManager::new();
        sm.start();
        sm.start(); // second start should be a no-op
        assert_eq!(sm.status, SourceStatus::Running);
        assert_eq!(sm.stream_generation(), 1);
        sm.stop();
        sm.wait_for_shutdown();
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
        sm.wait_for_shutdown();
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
        sm.wait_for_shutdown();
        assert_eq!(sm.stream_generation(), 1);
        sm.start();
        assert_eq!(sm.stream_generation(), 2);
        sm.stop();
        sm.wait_for_shutdown();
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
        sm.wait_for_shutdown();
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
    #[test]
    fn daemon_squelch_correlates_acknowledgements_coalesces_and_rejects_old_audio() {
        use ez_proto::{AudioFrame, ClientCommand, ServerEvent, SquelchState};
        let (client, mut commands, events) = crate::daemon_client::DaemonClient::test_connection();
        let mut source = SourceManager::new();
        source.source_mode = SourceMode::Daemon;
        source.status = SourceStatus::Running;
        source.daemon_audio_mode = Some(ez_proto::DemodMode::Fm);
        source.daemon_client = Some(client);
        source.observe_daemon_squelch(SquelchState {
            db: -100.0,
            revision: 1,
        });
        source.request_daemon_squelch(-50.0).unwrap();
        let ClientCommand::SetSquelch {
            channel_id,
            db,
            request_id,
        } = commands.try_recv().unwrap()
        else {
            panic!("wrong command")
        };
        assert_eq!((channel_id, db), (DAEMON_AUDIO_CHANNEL_ID, -50.0));
        source.request_daemon_squelch(-20.0).unwrap();
        assert!(commands.try_recv().is_err());
        assert_eq!(source.daemon_squelch_pending_db(), Some(-20.0));
        let audio = AudioFrame {
            clock: ez_proto::CaptureClock::new(1, 0, 48_000, None),
            sequence: 1,
            channel_id,
            sample_rate_hz: 48_000,
            samples: vec![1.0],
            squelch: SquelchState {
                db: -100.0,
                revision: 1,
            },
        };
        assert!(!source.accept_daemon_audio(&audio));
        for (foreign_channel, foreign_request) in [(99, request_id), (channel_id, request_id + 99)]
        {
            source.apply_daemon_squelch_reply(
                foreign_channel,
                foreign_request,
                &Ok(SquelchState {
                    db: -50.0,
                    revision: 2,
                }),
            );
            assert!(source.daemon_squelch_pending_db().is_some());
            assert_eq!(source.daemon_squelch_applied().unwrap().db, -100.0);
        }
        events
            .send(ServerEvent::Squelch {
                channel_id,
                request_id,
                result: Ok(SquelchState {
                    db: -50.0,
                    revision: 2,
                }),
            })
            .unwrap();
        source.recv_daemon_event().unwrap();
        assert_eq!(source.daemon_squelch_applied().unwrap().db, -50.0);
        assert!(!source.accept_daemon_audio(&audio));
        source.sync_daemon_squelch(-20.0);
        let ClientCommand::SetSquelch {
            request_id: final_request,
            db,
            ..
        } = commands.try_recv().unwrap()
        else {
            panic!("wrong command")
        };
        assert_eq!(db, -20.0);
        assert_ne!(final_request, request_id);
        // Audio can arrive before its control reply; a late reply must not roll
        // back the newer applied state. Older buffered frames remain rejected.
        let newest = SquelchState {
            db: -20.0,
            revision: 3,
        };
        source.observe_daemon_squelch(newest);
        source.apply_daemon_squelch_reply(channel_id, final_request, &Ok(newest));
        source.apply_daemon_squelch_reply(
            channel_id,
            request_id,
            &Ok(SquelchState {
                db: -50.0,
                revision: 2,
            }),
        );
        assert_eq!(source.daemon_squelch_applied(), Some(newest));
        source.observe_daemon_squelch(SquelchState {
            db: -99.0,
            revision: 3,
        });
        assert_eq!(source.daemon_squelch_applied(), Some(newest));
        assert!(source.accept_daemon_audio(&AudioFrame {
            squelch: newest,
            ..audio.clone()
        }));
        assert!(!source.accept_daemon_audio(&AudioFrame {
            channel_id: 99,
            squelch: newest,
            ..audio
        }));
        source.sync_daemon_squelch(-20.0);
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn daemon_squelch_failure_timeout_and_closed_queue_preserve_applied_state() {
        use ez_proto::{ClientCommand, SquelchState};
        let (client, mut commands, _events) = crate::daemon_client::DaemonClient::test_connection();
        let mut source = SourceManager::new();
        source.source_mode = SourceMode::Daemon;
        source.status = SourceStatus::Running;
        source.daemon_audio_mode = Some(ez_proto::DemodMode::Fm);
        source.daemon_client = Some(client);
        let applied = SquelchState {
            db: -100.0,
            revision: 1,
        };
        source.observe_daemon_squelch(applied);
        assert!(source.request_daemon_squelch(f32::NAN).is_err());
        source.request_daemon_squelch(-50.0).unwrap();
        let ClientCommand::SetSquelch { request_id, .. } = commands.try_recv().unwrap() else {
            panic!("wrong command")
        };
        source.apply_daemon_squelch_reply(
            DAEMON_AUDIO_CHANNEL_ID,
            request_id,
            &Err("channel removed".into()),
        );
        assert_eq!(
            source.daemon_squelch_error.as_deref(),
            Some("channel removed")
        );
        assert_eq!(source.daemon_squelch_applied(), Some(applied));
        source.request_daemon_squelch(-50.0).unwrap();
        commands.try_recv().unwrap();
        source.daemon_squelch_pending.as_mut().unwrap().started =
            std::time::Instant::now() - DAEMON_SQUELCH_TIMEOUT;
        source.sync_daemon_squelch(-50.0);
        assert!(source
            .daemon_squelch_error
            .as_ref()
            .unwrap()
            .contains("timed out"));
        assert!(source.daemon_squelch_pending_db().is_none());
        assert!(commands.try_recv().is_err()); // no per-frame retry storm
        assert_eq!(source.daemon_squelch_applied(), Some(applied));
        drop(commands);
        assert!(source
            .request_daemon_squelch(-30.0)
            .unwrap_err()
            .contains("queued"));
        assert_eq!(source.daemon_squelch_applied(), Some(applied));
    }
    #[test]
    fn rtl_ppm_accepts_already_applied_status_and_rejects_real_failures() {
        assert!(rtl_ppm_result_applied(0));
        assert!(rtl_ppm_result_applied(-2));
        for error in [-1, -3, -5] {
            assert!(!rtl_ppm_result_applied(error));
        }
    }
}
