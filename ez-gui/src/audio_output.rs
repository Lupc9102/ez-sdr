#[cfg(feature = "audio")]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
#[cfg(feature = "audio")]
use cpal::HostId;

/// Return the preferred audio host: PulseAudio when available, else system default.
/// ALSA's default device (dmix) fails on systems where PulseAudio owns the sound card,
/// so we prefer the PulseAudio backend directly.
#[cfg(feature = "audio")]
fn preferred_host() -> Result<cpal::Host, String> {
    for host_id in cpal::available_hosts() {
        if host_id == HostId::PulseAudio {
            match cpal::host_from_id(host_id) {
                Ok(host) => return Ok(host),
                Err(_) => continue,
            }
        }
    }
    Ok(cpal::default_host())
}
use crossbeam_channel::Receiver;
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc, Mutex,
};

/// None selects the system default; a stored CPAL ID selects that device only.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AudioOutputSelection {
    pub device_id: Option<String>,
    /// Zero uses the selected device's default; other rates are exact requests.
    pub sample_rate: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioOutputDeviceInfo {
    pub id: String,
    pub label: String,
    /// Standard rates plus range endpoints. Startup validates exact rates again.
    pub sample_rates: Vec<u32>,
    pub default_sample_rate: u32,
    pub is_default: bool,
}

pub enum AudioInputReceiver {
    Direct(Receiver<Vec<f32>>),
    Shared(Arc<Mutex<Receiver<Vec<f32>>>>),
}
impl From<Receiver<Vec<f32>>> for AudioInputReceiver {
    fn from(rx: Receiver<Vec<f32>>) -> Self {
        Self::Direct(rx)
    }
}
impl From<Arc<Mutex<Receiver<Vec<f32>>>>> for AudioInputReceiver {
    fn from(rx: Arc<Mutex<Receiver<Vec<f32>>>>) -> Self {
        Self::Shared(rx)
    }
}

#[cfg(any(feature = "audio", test))]
fn choose_output_device<T>(
    selection: &AudioOutputSelection,
    default: Option<T>,
    devices: impl IntoIterator<Item = (String, T)>,
) -> Result<T, String> {
    let Some(wanted) = selection.device_id.as_deref() else {
        return default.ok_or_else(|| "No default audio output device is available.".into());
    };
    let mut matching = devices.into_iter().filter(|(id, _)| id == wanted);
    let Some((_, device)) = matching.next() else {
        return Err(format!(
            "Selected audio output {wanted:?} is unavailable. Refresh and select an output device."
        ));
    };
    if matching.next().is_some() {
        return Err(format!(
            "Audio output identifier {wanted:?} is ambiguous; select another device."
        ));
    }
    Ok(device)
}

/// Call on a worker: operating-system discovery can block. Only plain metadata
/// leaves this function; no CPAL device or stream handles cross to the caller.
#[cfg(feature = "audio")]
pub fn enumerate_output_devices() -> Result<Vec<AudioOutputDeviceInfo>, String> {
    let host = preferred_host()?;
    let default_id = host
        .default_output_device()
        .and_then(|device| device.id().ok());
    let mut devices = Vec::new();
    for device in host.output_devices().map_err(|error| error.to_string())? {
        let Ok(id) = device.id() else { continue };
        let Ok(ranges) = device.supported_output_configs() else {
            continue;
        };
        let ranges: Vec<_> = ranges.collect();
        let default = device.default_output_config().ok();
        let default_rate = default
            .as_ref()
            .map_or(0, cpal::SupportedStreamConfig::sample_rate);
        let sample_rates = advertised_sample_rates(&ranges, default_rate);
        if sample_rates.is_empty() {
            continue;
        }
        devices.push(AudioOutputDeviceInfo {
            id: id.to_string(),
            label: device.to_string(),
            sample_rates,
            default_sample_rate: default_rate,
            is_default: default_id.as_ref() == Some(&id),
        });
    }
    Ok(devices)
}
#[cfg(not(feature = "audio"))]
pub fn enumerate_output_devices() -> Result<Vec<AudioOutputDeviceInfo>, String> {
    Err("Audio support not compiled in".into())
}

#[cfg(feature = "audio")]
fn supported_format(format: cpal::SampleFormat) -> bool {
    matches!(
        format,
        cpal::SampleFormat::F32 | cpal::SampleFormat::I16 | cpal::SampleFormat::U16
    )
}
#[cfg(feature = "audio")]
fn advertised_sample_rates(
    ranges: &[cpal::SupportedStreamConfigRange],
    default_rate: u32,
) -> Vec<u32> {
    let mut rates = vec![
        8_000, 11_025, 16_000, 22_050, 24_000, 32_000, 44_100, 48_000, 88_200, 96_000, 176_400,
        192_000,
    ];
    if default_rate > 0 {
        rates.push(default_rate);
    }
    for range in ranges
        .iter()
        .filter(|range| supported_format(range.sample_format()) && range.channels() > 0)
    {
        rates.extend([range.min_sample_rate(), range.max_sample_rate()]);
    }
    rates.retain(|&rate| {
        rate > 0
            && ranges.iter().any(|range| {
                supported_format(range.sample_format())
                    && range.channels() > 0
                    && range.contains_rate(rate)
            })
    });
    rates.sort_unstable();
    rates.dedup();
    rates
}
#[cfg(feature = "audio")]
fn choose_stream_config(
    default: Option<&cpal::SupportedStreamConfig>,
    ranges: &[cpal::SupportedStreamConfigRange],
    requested_rate: u32,
) -> Result<cpal::SupportedStreamConfig, String> {
    let rate = if requested_rate == 0 {
        default.map(cpal::SupportedStreamConfig::sample_rate).ok_or_else(|| "Could not determine the output device's default sample rate. Choose a supported rate explicitly.".to_string())?
    } else {
        requested_rate
    };
    if let Some(default) = default.filter(|config| {
        config.sample_rate() == rate
            && config.channels() > 0
            && supported_format(config.sample_format())
    }) {
        return Ok(default.clone());
    }
    let preferred_channels = default.map_or(2, cpal::SupportedStreamConfig::channels);
    ranges.iter().filter(|range| range.channels() > 0 && supported_format(range.sample_format()) && range.contains_rate(rate))
        .max_by_key(|range| (range.channels() == preferred_channels, range.sample_format() == cpal::SampleFormat::F32, range.channels() == 2, std::cmp::Reverse(range.channels())))
        .and_then(|range| range.clone().try_with_sample_rate(rate))
        .ok_or_else(|| format!("Audio output does not support {rate} Hz with F32, I16, or U16 samples. Choose a supported rate."))
}

#[cfg(feature = "audio")]
struct AudioBuffer {
    input: AudioInputReceiver,
    samples: std::collections::VecDeque<f32>,
    input_channels: usize,
    backlog_limit: usize,
}
#[cfg(feature = "audio")]
impl AudioBuffer {
    fn new(input: AudioInputReceiver, input_channels: usize, sample_rate: u32) -> Self {
        let backlog_limit = (sample_rate as usize / 5).max(1) * input_channels;
        Self {
            input,
            samples: std::collections::VecDeque::with_capacity(backlog_limit),
            input_channels,
            backlog_limit,
        }
    }
    fn fill<T: cpal::SizedSample + cpal::FromSample<f32>>(
        &mut self,
        data: &mut [T],
        channels: usize,
    ) {
        let input_channels = self.input_channels;
        let backlog_limit = self.backlog_limit;
        let append = |receiver: &Receiver<Vec<f32>>,
                      samples: &mut std::collections::VecDeque<f32>| {
            // Bound callback work even if a producer temporarily outpaces output.
            for chunk in receiver.try_iter().take(32) {
                // Discard oldest whole input frames across the old/new chunk
                // boundary; split stereo chunks must never swap left/right.
                let excess = samples
                    .len()
                    .saturating_add(chunk.len())
                    .saturating_sub(backlog_limit);
                let discard = excess.div_ceil(input_channels) * input_channels;
                let old_discard = discard.min(samples.len());
                samples.drain(..old_discard);
                samples.extend(chunk.into_iter().skip(discard - old_discard));
            }
        };
        match &self.input {
            AudioInputReceiver::Direct(receiver) => append(receiver, &mut self.samples),
            AudioInputReceiver::Shared(receiver) => {
                if let Ok(receiver) = receiver.try_lock() {
                    append(&receiver, &mut self.samples);
                }
            }
        }
        let mut frames = data.chunks_exact_mut(channels.max(1));
        for frame in &mut frames {
            let clean = |sample: f32| {
                if sample.is_finite() {
                    sample.clamp(-1.0, 1.0)
                } else {
                    0.0
                }
            };
            let (left, right) = if self.samples.len() >= self.input_channels {
                let left = clean(self.samples.pop_front().unwrap_or(0.0));
                let right = if self.input_channels == 2 {
                    clean(self.samples.pop_front().unwrap_or(0.0))
                } else {
                    left
                };
                (left, right)
            } else {
                // Retain a split stereo frame until the next input chunk.
                (0.0, 0.0)
            };
            if self.input_channels == 1 {
                frame.fill(T::from_sample(left));
            } else if frame.len() == 1 {
                frame[0] = T::from_sample((left + right) * 0.5);
            } else {
                frame.fill(T::from_sample(0.0));
                frame[0] = T::from_sample(left);
                frame[1] = T::from_sample(right);
            }
        }
        frames.into_remainder().fill(T::from_sample(0.0));
    }
}
#[cfg(feature = "audio")]
fn build_output_stream<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    input: AudioInputReceiver,
    input_channels: usize,
    failed: Arc<AtomicBool>,
    error_message: Arc<Mutex<Option<String>>>,
) -> Result<cpal::Stream, cpal::Error> {
    let channels = usize::from(config.channels).max(1);
    let mut buffer = AudioBuffer::new(input, input_channels, config.sample_rate);
    device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| buffer.fill(data, channels),
        move |error| {
            record_failure(
                &failed,
                &error_message,
                format!("Audio output failed: {error}"),
            )
        },
        None,
    )
}
fn record_failure(failed: &AtomicBool, message: &Mutex<Option<String>>, error: String) {
    if let Ok(mut message) = message.lock() {
        *message = Some(error);
    }
    failed.store(true, Ordering::Release);
}

/// Build a CPAL output stream on the calling thread.
/// The stream must be created and dropped on the same thread (CPAL requirement).
#[cfg(feature = "audio")]
fn create_cpal_stream(
    audio_rx: Receiver<Vec<f32>>,
    selection: &AudioOutputSelection,
    input_channels: usize,
    failed: &Arc<AtomicBool>,
    error_message: &Arc<Mutex<Option<String>>>,
) -> Result<(cpal::Stream, u32), String> {
    let audio_input = AudioInputReceiver::Direct(audio_rx);
    let device = if let Some(id) = &selection.device_id {
        let parsed: cpal::DeviceId = id
            .parse()
            .map_err(|error| format!("Invalid audio output identifier: {error}"))?;
        let host = cpal::host_from_id(parsed.host())
            .map_err(|error| format!("Audio output host unavailable: {error}"))?;
        let devices = host
            .output_devices()
            .map_err(|error| error.to_string())?
            .filter_map(|device| device.id().ok().map(|id| (id.to_string(), device)));
        choose_output_device(selection, None, devices)?
    } else {
        choose_output_device(
            selection,
            preferred_host()?.default_output_device(),
            std::iter::empty(),
        )?
    };
    let default = device.default_output_config().ok();
    let ranges: Vec<_> = device
        .supported_output_configs()
        .map_err(|error| format!("Could not query audio output {device}: {error}"))?
        .collect();
    let supported = choose_stream_config(default.as_ref(), &ranges, selection.sample_rate)
        .map_err(|error| format!("{device}: {error}"))?;
    let sample_rate = supported.sample_rate();
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let stream = match format {
        cpal::SampleFormat::F32 => build_output_stream::<f32>(
            &device,
            config,
            audio_input,
            input_channels,
            Arc::clone(failed),
            Arc::clone(error_message),
        ),
        cpal::SampleFormat::I16 => build_output_stream::<i16>(
            &device,
            config,
            audio_input,
            input_channels,
            Arc::clone(failed),
            Arc::clone(error_message),
        ),
        cpal::SampleFormat::U16 => build_output_stream::<u16>(
            &device,
            config,
            audio_input,
            input_channels,
            Arc::clone(failed),
            Arc::clone(error_message),
        ),
        _ => return Err(format!("Unsupported audio sample format: {format}")),
    }
    .map_err(|error| error.to_string())?;
    stream.play().map_err(|error| error.to_string())?;
    Ok((stream, sample_rate))
}

/// Dedicated audio worker that owns the CPAL stream on its own OS thread.
/// The UI thread sends audio via [`AudioWorker::push_audio`] (non-blocking)
/// and the worker thread plays it via CPAL callbacks.
#[cfg(feature = "audio")]
pub struct AudioWorker {
    audio_tx: crossbeam_channel::Sender<Vec<f32>>,
    stop_tx: crossbeam_channel::Sender<()>,
    handle: Option<std::thread::JoinHandle<()>>,
    failed: Arc<AtomicBool>,
    error_message: Arc<Mutex<Option<String>>>,
    sample_rate: Arc<AtomicU32>,
    input_channels: usize,
}

#[cfg(feature = "audio")]
impl AudioWorker {
    /// Create a dummy AudioWorker without spawning a thread.
    /// Used in test environments where CPAL may block on device enumeration.
    pub fn new_uninitialized() -> Self {
        let (audio_tx, _audio_rx) = crossbeam_channel::bounded::<Vec<f32>>(4);
        let (stop_tx, _stop_rx) = crossbeam_channel::bounded::<()>(1);
        let failed = Arc::new(AtomicBool::new(false));
        let error_message = Arc::new(Mutex::new(None));
        let sample_rate = Arc::new(AtomicU32::new(48_000));
        Self {
            audio_tx,
            stop_tx,
            handle: None,
            failed,
            error_message,
            sample_rate,
            input_channels: 1,
        }
    }

    /// Spawn a worker thread that owns the CPAL stream.
    pub fn new(selection: AudioOutputSelection, input_channels: usize) -> Result<Self, String> {
        if !matches!(input_channels, 1 | 2) {
            return Err(
                "Audio input must contain one mono channel or two interleaved stereo channels."
                    .into(),
            );
        }

        let (audio_tx, audio_rx) = crossbeam_channel::bounded(4);
        let (stop_tx, stop_rx) = crossbeam_channel::bounded(1);
        let failed = Arc::new(AtomicBool::new(false));
        let error_message = Arc::new(Mutex::new(None));
        let sample_rate = Arc::new(AtomicU32::new(48_000));

        let worker_failed = Arc::clone(&failed);
        let worker_error_message = Arc::clone(&error_message);
        let worker_sample_rate = Arc::clone(&sample_rate);

        let handle = std::thread::spawn(move || {
            match create_cpal_stream(
                audio_rx,
                &selection,
                input_channels,
                &worker_failed,
                &worker_error_message,
            ) {
                Ok((stream, rate)) => {
                    worker_sample_rate.store(rate, Ordering::Release);
                    // Keep the stream alive until stop is signalled.
                    let _ = stop_rx.recv();
                    drop(stream);
                }
                Err(error) => {
                    record_failure(&worker_failed, &worker_error_message, error);
                }
            }
        });

        Ok(Self {
            audio_tx,
            stop_tx,
            handle: Some(handle),
            failed,
            error_message,
            sample_rate,
            input_channels,
        })
    }

    /// Non-blocking send. Returns `false` if the channel is full (sample is dropped).
    pub fn push_audio(&self, samples: Vec<f32>) -> bool {
        self.audio_tx.try_send(samples).is_ok()
    }

    /// Signal the worker to stop and wait for it to finish.
    pub fn stop(mut self) {
        self.shutdown();
    }

    /// Non-consuming variant for call sites that cannot move out of `self`.
    /// Sends stop signal and detaches the thread (non-blocking).
    pub fn shutdown(&mut self) {
        let _ = self.stop_tx.send(());
        if let Some(handle) = self.handle.take() {
            std::mem::forget(handle);
        }
    }

    pub fn is_running(&self) -> bool {
        !self.has_failed()
    }

    pub fn has_failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }

    pub fn take_error(&self) -> Option<String> {
        self.error_message
            .lock()
            .ok()
            .and_then(|mut message| message.take())
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate.load(Ordering::Acquire)
    }

    pub fn input_channels(&self) -> usize {
        self.input_channels
    }

    pub fn mark_failed(&mut self) {
        self.failed.store(true, Ordering::Release);
    }
}

#[cfg(feature = "audio")]
impl Drop for AudioWorker {
    fn drop(&mut self) {
        let _ = self.stop_tx.send(());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub struct AudioOutput {
    #[cfg(feature = "audio")]
    worker: Option<AudioWorker>,
    sample_rate: u32,
    running: bool,
    failed: Arc<AtomicBool>,
    error_message: Arc<Mutex<Option<String>>>,
    selection: AudioOutputSelection,
    input_channels: usize,
}
impl Default for AudioOutput {
    fn default() -> Self {
        Self::new()
    }
}
impl AudioOutput {
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "audio")]
            worker: None,
            sample_rate: 48_000,
            running: false,
            failed: Arc::new(AtomicBool::new(false)),
            error_message: Arc::new(Mutex::new(None)),
            selection: AudioOutputSelection::default(),
            input_channels: 1,
        }
    }
    #[must_use = "check if audio output started successfully"]
    pub fn start(&mut self, rx: impl Into<AudioInputReceiver>) -> Result<(), String> {
        self.start_with_selection(rx, &AudioOutputSelection::default())
    }
    #[must_use = "check if the selected audio output started successfully"]
    pub fn start_with_selection(
        &mut self,
        rx: impl Into<AudioInputReceiver>,
        selection: &AudioOutputSelection,
    ) -> Result<(), String> {
        self.start_with_selection_channels(rx, selection, 1)
    }

    #[must_use = "check if the selected audio output started successfully"]
    pub fn start_with_selection_channels(
        &mut self,
        _rx: impl Into<AudioInputReceiver>,
        selection: &AudioOutputSelection,
        input_channels: usize,
    ) -> Result<(), String> {
        if !matches!(input_channels, 1 | 2) {
            return Err(
                "Audio input must contain one mono channel or two interleaved stereo channels."
                    .into(),
            );
        }
        if self.is_running() {
            return if &self.selection == selection && self.input_channels == input_channels {
                Ok(())
            } else {
                Err("Stop audio before changing its output device or sample rate.".into())
            };
        }
        self.stop();
        #[cfg(feature = "audio")]
        {
            let worker = AudioWorker::new(selection.clone(), input_channels)?;
            self.failed = Arc::clone(&worker.failed);
            self.error_message = Arc::clone(&worker.error_message);
            self.sample_rate = worker.sample_rate();
            self.input_channels = input_channels;
            self.selection = selection.clone();
            self.running = true;
            self.worker = Some(worker);
            Ok(())
        }
        #[cfg(not(feature = "audio"))]
        {
            let _ = (selection, input_channels);
            self.failed.store(true, Ordering::Release);
            if let Ok(mut msg) = self.error_message.lock() {
                *msg = Some("Audio support not compiled in".to_string());
            }
            Err("Audio support not compiled in".to_string())
        }
    }

    /// Non-blocking audio send via the worker's internal channel.
    /// Returns `false` if the channel is full (sample is dropped).
    pub fn push_audio(&self, samples: Vec<f32>) -> bool {
        #[cfg(feature = "audio")]
        {
            if let Some(worker) = &self.worker {
                return worker.push_audio(samples);
            }
        }
        false
    }

    pub fn is_running(&self) -> bool {
        self.running && !self.has_failed()
    }
    pub fn sample_rate(&self) -> u32 {
        #[cfg(feature = "audio")]
        {
            if let Some(worker) = &self.worker {
                return worker.sample_rate();
            }
        }
        self.sample_rate
    }
    pub fn input_channels(&self) -> usize {
        self.input_channels
    }
    pub fn has_failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }
    /// Drain the message without clearing failure state. Stop/restart resets it.
    pub fn take_error(&self) -> Option<String> {
        self.error_message
            .lock()
            .ok()
            .and_then(|mut message| message.take())
    }
    pub fn mark_failed(&mut self) {
        self.failed.store(true, Ordering::Release);
    }
    /// Alias for stop() so app.rs can call shutdown() on both AudioOutput and AudioWorker.
    pub fn shutdown(&mut self) {
        self.stop();
    }

    pub fn stop(&mut self) {
        #[cfg(feature = "audio")]
        {
            if let Some(worker) = self.worker.take() {
                worker.stop();
            }
        }
        self.running = false;
        self.failed.store(false, Ordering::Release);
        if let Ok(mut message) = self.error_message.lock() {
            *message = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_and_stop_reset_state() {
        let mut output = AudioOutput::new();
        assert!(!output.is_running());
        assert!(!output.has_failed());
        assert_eq!(output.sample_rate(), 48_000);
        output.mark_failed();
        assert!(output.has_failed());
        output.stop();
        output.stop();
        assert!(!output.is_running());
        assert!(!output.has_failed());
        assert_eq!(output.take_error(), None);
    }
    #[test]
    fn selection_uses_id_and_never_falls_back_to_another_device() {
        let selected = AudioOutputSelection {
            device_id: Some("host:second".into()),
            sample_rate: 44_100,
        };
        assert_eq!(
            choose_output_device(
                &selected,
                Some(99),
                [("host:first".into(), 1), ("host:second".into(), 2)]
            )
            .unwrap(),
            2
        );
        assert!(
            choose_output_device(&selected, Some(99), [("host:first".into(), 1)])
                .unwrap_err()
                .contains("unavailable")
        );
        assert!(choose_output_device(
            &selected,
            Some(99),
            [("host:second".into(), 1), ("host:second".into(), 2)]
        )
        .unwrap_err()
        .contains("ambiguous"));
        assert_eq!(
            choose_output_device(
                &AudioOutputSelection::default(),
                Some(99),
                [("host:first".into(), 1)]
            )
            .unwrap(),
            99
        );
        assert!(choose_output_device::<u32>(&AudioOutputSelection::default(), None, []).is_err());
        let decoded: AudioOutputSelection =
            serde_json::from_str(&serde_json::to_string(&selected).unwrap()).unwrap();
        assert_eq!(decoded, selected);
        let partial: AudioOutputSelection =
            serde_json::from_str(r#"{"device_id":"host:second"}"#).unwrap();
        assert_eq!(partial.sample_rate, 0);
        assert_eq!(partial.device_id.as_deref(), Some("host:second"));
        assert_eq!(
            serde_json::from_str::<AudioOutputSelection>("{}").unwrap(),
            AudioOutputSelection::default()
        );
    }
    #[test]
    fn asynchronous_stream_error_changes_running_state_and_can_be_drained() {
        let mut output = AudioOutput::new();
        output.running = true;
        let failed = Arc::clone(&output.failed);
        let error = Arc::clone(&output.error_message);
        std::thread::spawn(move || record_failure(&failed, &error, "device unplugged".into()))
            .join()
            .unwrap();
        assert!(!output.is_running());
        assert!(output.has_failed());
        assert_eq!(output.take_error().as_deref(), Some("device unplugged"));
        assert_eq!(output.take_error(), None);
        assert!(output.has_failed());
        output.stop();
        assert!(!output.has_failed());
    }
    #[test]
    fn changing_running_selection_requires_stop_without_reopening_hardware() {
        let mut output = AudioOutput::new();
        output.running = true;
        let (_, receiver) = crossbeam_channel::unbounded();
        assert!(output
            .start_with_selection(receiver, &AudioOutputSelection::default())
            .is_ok());
        let (_, receiver) = crossbeam_channel::unbounded();
        assert!(output
            .start_with_selection(
                receiver,
                &AudioOutputSelection {
                    sample_rate: 96_000,
                    ..Default::default()
                }
            )
            .unwrap_err()
            .contains("Stop audio"));
        assert!(output.is_running());
    }
    #[cfg(not(feature = "audio"))]
    #[test]
    fn audio_disabled_reports_failure_for_start_and_enumeration() {
        let mut output = AudioOutput::new();
        let (_, receiver) = crossbeam_channel::unbounded();
        assert!(output.start(receiver).unwrap_err().contains("not compiled"));
        assert!(output.has_failed());
        assert!(output.take_error().unwrap().contains("not compiled"));
        assert!(enumerate_output_devices().is_err());
    }
    #[cfg(feature = "audio")]
    fn range(
        channels: u16,
        min: u32,
        max: u32,
        format: cpal::SampleFormat,
    ) -> cpal::SupportedStreamConfigRange {
        cpal::SupportedStreamConfigRange::new(
            channels,
            min,
            max,
            cpal::SupportedBufferSize::Unknown,
            format,
        )
    }
    #[cfg(feature = "audio")]
    #[test]
    fn requested_rate_is_exact_and_supported_formats_are_selected() {
        let default = range(2, 48_000, 48_000, cpal::SampleFormat::F32).with_sample_rate(48_000);
        let ranges = [
            range(2, 44_100, 48_000, cpal::SampleFormat::I16),
            range(2, 96_000, 96_000, cpal::SampleFormat::U16),
            range(2, 192_000, 192_000, cpal::SampleFormat::F64),
        ];
        assert_eq!(
            choose_stream_config(Some(&default), &ranges, 0)
                .unwrap()
                .sample_rate(),
            48_000
        );
        let chosen = choose_stream_config(Some(&default), &ranges, 44_100).unwrap();
        assert_eq!(chosen.sample_rate(), 44_100);
        assert_eq!(chosen.sample_format(), cpal::SampleFormat::I16);
        assert_eq!(
            choose_stream_config(Some(&default), &ranges, 96_000)
                .unwrap()
                .sample_format(),
            cpal::SampleFormat::U16
        );
        assert!(choose_stream_config(Some(&default), &ranges, 22_050).is_err());
        assert!(choose_stream_config(Some(&default), &ranges, 192_000).is_err());
        assert!(choose_stream_config(None, &ranges, 0).is_err());
        assert_eq!(
            choose_stream_config(None, &ranges, 96_000)
                .unwrap()
                .sample_rate(),
            96_000
        );
    }
    #[cfg(feature = "audio")]
    #[test]
    fn default_rate_can_use_supported_format_and_rate_listing_has_no_unsupported_entries() {
        let default = range(2, 48_000, 48_000, cpal::SampleFormat::F64).with_sample_rate(48_000);
        let ranges = [
            range(2, 44_100, 48_000, cpal::SampleFormat::I16),
            range(0, 11_025, 11_025, cpal::SampleFormat::F32),
            range(2, 192_000, 192_000, cpal::SampleFormat::F64),
        ];
        assert_eq!(
            choose_stream_config(Some(&default), &ranges, 0)
                .unwrap()
                .sample_format(),
            cpal::SampleFormat::I16
        );
        assert_eq!(
            advertised_sample_rates(&ranges, 48_000),
            vec![44_100, 48_000]
        );
    }
    #[cfg(feature = "audio")]
    #[test]
    fn callbacks_convert_clamp_duplicate_channels_and_fill_silence() {
        let samples = [-2.0, 0.0, 2.0, f32::NAN];
        let (tx, rx) = crossbeam_channel::unbounded();
        tx.send(samples.to_vec()).unwrap();
        let mut buffer = AudioBuffer::new(rx.into(), 1, 48_000);
        let mut floats = [9.0; 11];
        buffer.fill(&mut floats, 2);
        assert_eq!(
            floats,
            [-1.0, -1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0]
        );
        let (tx, rx) = crossbeam_channel::unbounded();
        tx.send(samples.to_vec()).unwrap();
        let mut buffer = AudioBuffer::new(rx.into(), 1, 48_000);
        let mut signed = [0i16; 5];
        buffer.fill(&mut signed, 1);
        assert_eq!(signed, [i16::MIN, 0, i16::MAX, 0, 0]);
        let (tx, rx) = crossbeam_channel::unbounded();
        tx.send(samples.to_vec()).unwrap();
        let mut buffer = AudioBuffer::new(rx.into(), 1, 48_000);
        let mut unsigned = [0u16; 5];
        buffer.fill(&mut unsigned, 1);
        assert_eq!(unsigned, [0, 32768, u16::MAX, 32768, 32768]);
    }
    #[cfg(feature = "audio")]
    #[test]
    fn callback_does_not_wait_for_shared_receiver_lock() {
        let (_, rx) = crossbeam_channel::unbounded();
        let rx = Arc::new(Mutex::new(rx));
        let mut buffer = AudioBuffer::new(Arc::clone(&rx).into(), 1, 48_000);
        let _locked = rx.lock().unwrap();
        let mut output = [1.0f32; 2];
        buffer.fill(&mut output, 2);
        assert_eq!(output, [0.0, 0.0]);
    }

    #[test]
    fn input_channel_count_is_validated_without_opening_hardware() {
        let mut output = AudioOutput::new();
        assert_eq!(output.input_channels(), 1);
        for channels in [0, 3, usize::MAX] {
            let (_, receiver) = crossbeam_channel::unbounded();
            assert!(output
                .start_with_selection_channels(receiver, &AudioOutputSelection::default(), channels)
                .unwrap_err()
                .contains("mono channel"));
        }
        output.running = true;
        let (_, receiver) = crossbeam_channel::unbounded();
        assert!(output
            .start_with_selection_channels(receiver, &AudioOutputSelection::default(), 2)
            .unwrap_err()
            .contains("Stop audio"));
        output.input_channels = 2;
        let (_, receiver) = crossbeam_channel::unbounded();
        assert!(output
            .start_with_selection_channels(receiver, &AudioOutputSelection::default(), 2)
            .is_ok());
        assert_eq!(output.input_channels(), 2);
    }

    #[cfg(feature = "audio")]
    #[test]
    fn stereo_maps_to_mono_stereo_and_multichannel_without_swapping_sides() {
        for (channels, expected) in [
            (1, vec![0.0, 0.5]),
            (2, vec![0.75, -0.75, 0.25, 0.75]),
            (4, vec![0.75, -0.75, 0.0, 0.0, 0.25, 0.75, 0.0, 0.0]),
        ] {
            let (tx, rx) = crossbeam_channel::unbounded();
            tx.send(vec![0.75, -0.75, 0.25, 0.75]).unwrap();
            let mut buffer = AudioBuffer::new(rx.into(), 2, 48_000);
            let mut output = vec![9.0; expected.len()];
            buffer.fill(&mut output, channels);
            assert_eq!(output, expected);
        }
    }

    #[cfg(feature = "audio")]
    #[test]
    fn split_stereo_frames_wait_for_the_matching_right_sample() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut buffer = AudioBuffer::new(rx.into(), 2, 48_000);
        tx.send(vec![0.25]).unwrap();
        let mut output = [9.0f32; 2];
        buffer.fill(&mut output, 2);
        assert_eq!(output, [0.0, 0.0]);
        tx.send(vec![-0.75, 0.5, -0.5]).unwrap();
        buffer.fill(&mut output, 2);
        assert_eq!(output, [0.25, -0.75]);
        buffer.fill(&mut output, 2);
        assert_eq!(output, [0.5, -0.5]);
    }

    #[cfg(feature = "audio")]
    #[test]
    fn overload_keeps_fresh_audio_and_drops_only_whole_stereo_frames() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut buffer = AudioBuffer::new(rx.into(), 2, 10); // two frames = 200 ms
        tx.send(vec![0.1]).unwrap();
        let mut output = [0.0f32; 2];
        buffer.fill(&mut output, 2);
        tx.send(vec![-0.1, 0.2, -0.2, 0.3, -0.3, 0.4, -0.4])
            .unwrap();
        buffer.fill(&mut output, 2);
        assert_eq!(output, [0.3, -0.3]);
        buffer.fill(&mut output, 2);
        assert_eq!(output, [0.4, -0.4]);
        assert!(buffer.samples.len() <= 4);
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut mono = AudioBuffer::new(rx.into(), 1, 10);
        tx.send(vec![0.1, 0.2, 0.3, 0.4]).unwrap();
        let mut output = [0.0f32; 2];
        mono.fill(&mut output, 1);
        assert_eq!(output, [0.3, 0.4]);
    }
}
