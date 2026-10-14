//! Independent secondary audio receiver sharing the primary capture.
//!
//! The local path owns a second native DSP worker subscribed directly to the
//! capture stream, so its tuning does not follow the primary VFO.  Daemon mode
//! uses a second authenticated connection and a connection-owned audio channel;
//! it never retunes the primary client's hardware channel.

use crate::audio_output::{AudioOutput, AudioOutputSelection};
use crate::capture_stream::CaptureStream;
use crate::demod::RadioConfig;
use crate::demod::{DemodConfig, DemodWorker};
use crate::radio_iq::RadioIqConfig;
use crate::radio_squelch::SquelchMode;
use crate::sdr_panel::DemodMode;
use crate::source_manager::{SourceManager, SourceMode, SourceStatus};
use ez_proto::{ClientCommand, PipelineKind, ServerEvent};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

/// Persistent settings for the independent B receiver.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SecondaryVfoSettings {
    pub enabled: bool,
    pub frequency_hz: u64,
    /// Canonical label from [`DemodMode::label`], kept as a string for config compatibility.
    pub mode_label: String,
    pub volume: f32,
    pub squelch_db: f32,
    /// Zero selects the mode's standard RF width.
    pub rf_bandwidth_hz: f32,
    pub cw_tone_hz: f32,
    pub wfm_stereo: bool,
}

impl Default for SecondaryVfoSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            frequency_hz: 100_000_000,
            mode_label: "FM".into(),
            volume: 0.5,
            squelch_db: -100.0,
            rf_bandwidth_hz: 0.0,
            cw_tone_hz: 800.0,
            wfm_stereo: false,
        }
    }
}

struct DaemonPath {
    client: crate::daemon_client::DaemonClient,
    channel_id: Option<ez_proto::ChannelId>,
    applied: Option<(i64, u32, ez_proto::AudioConfiguration)>,
    squelch_db: Option<f32>,
}

/// Independent A/B audio path. It owns its decoder and output endpoint and
/// holds only a subscriber to the source's immutable IQ capture stream.
pub struct SecondaryVfo {
    capture: CaptureStream,
    local_worker: Option<DemodWorker>,
    output: AudioOutput,
    resampler: crate::audio_resampler::AudioResampler,
    daemon: Option<DaemonPath>,
    output_shape: Option<(AudioOutputSelection, usize)>,
    pub error: Option<String>,
    pub status: String,
}

impl SecondaryVfo {
    pub fn new(capture: CaptureStream) -> Self {
        Self {
            capture,
            local_worker: None,
            output: AudioOutput::new(),
            resampler: crate::audio_resampler::AudioResampler::default(),
            daemon: None,
            output_shape: None,
            error: None,
            status: "Stopped".into(),
        }
    }

    /// Draw B receiver controls and show the actual primary capture coverage.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        settings: &mut SecondaryVfoSettings,
        source: &SourceManager,
        selection: &AudioOutputSelection,
    ) {
        ui.checkbox(&mut settings.enabled, "Enable B receiver");
        ui.horizontal(|ui| {
            ui.label("B frequency (Hz)");
            ui.add(
                egui::DragValue::new(&mut settings.frequency_hz)
                    .range(1..=u64::MAX)
                    .speed(1_000.0),
            );
        });
        ui.horizontal(|ui| {
            ui.label("Mode");
            egui::ComboBox::from_id_salt("secondary-vfo-mode")
                .selected_text(&settings.mode_label)
                .show_ui(ui, |ui| {
                    for mode in [
                        DemodMode::Am,
                        DemodMode::Fm,
                        DemodMode::Wfm,
                        DemodMode::Lsb,
                        DemodMode::Usb,
                        DemodMode::Dsb,
                        DemodMode::Cw,
                        DemodMode::Raw,
                    ] {
                        ui.selectable_value(
                            &mut settings.mode_label,
                            mode.label().to_owned(),
                            mode.label(),
                        );
                    }
                });
        });
        ui.add(egui::Slider::new(&mut settings.volume, 0.0..=1.0).text("B volume"));
        ui.add(egui::Slider::new(&mut settings.squelch_db, -120.0..=0.0).text("B squelch dB"));
        ui.add(
            egui::DragValue::new(&mut settings.rf_bandwidth_hz)
                .range(0.0..=500_000.0)
                .speed(100.0)
                .prefix("RF Hz "),
        );
        if DemodMode::from_label(&settings.mode_label) == Some(DemodMode::Wfm) {
            ui.checkbox(&mut settings.wfm_stereo, "WFM stereo");
        }
        if DemodMode::from_label(&settings.mode_label) == Some(DemodMode::Cw) {
            ui.add(egui::Slider::new(&mut settings.cw_tone_hz, 100.0..=2_000.0).text("CW beat Hz"));
        }
        ui.label(format!(
            "Capture: {:.6} MHz ± {:.1} kHz · output: {}",
            source.capture_center_frequency_hz() as f64 / 1_000_000.0,
            f64::from(source.sample_rate_hz) * 0.4 / 1_000.0,
            if selection.sample_rate == 0 {
                "device default".to_owned()
            } else {
                format!("{} Hz", selection.sample_rate)
            },
        ));
        ui.label(&self.status);
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
    }

    /// Update the second DSP chain and deliver any ready PCM without depending
    /// on a repaint for capture or demodulation progress.
    pub fn tick(
        &mut self,
        source: &SourceManager,
        settings: &SecondaryVfoSettings,
        selection: &AudioOutputSelection,
    ) {
        if !settings.enabled {
            self.stop();
            self.status = "Stopped".into();
            return;
        }
        let Some(mode) = DemodMode::from_label(&settings.mode_label) else {
            self.fail("Choose a supported B demodulation mode.");
            return;
        };
        let mode = mode.resolve(settings.frequency_hz);
        if source.source_mode == SourceMode::Daemon {
            self.tick_daemon(source, settings, selection, mode);
        } else {
            self.tick_local(source, settings, selection, mode);
        }
    }

    /// Stop B playback and release both capture and daemon subscriptions.
    pub fn stop(&mut self) {
        self.local_worker = None;
        self.daemon = None;
        self.output.stop();
        self.output_shape = None;
        self.resampler.reset();
    }

    fn tick_local(
        &mut self,
        source: &SourceManager,
        settings: &SecondaryVfoSettings,
        selection: &AudioOutputSelection,
        mode: DemodMode,
    ) {
        self.daemon = None;
        if source.status != SourceStatus::Running {
            self.fail("The primary RF capture is not running.");
            return;
        }
        let center_hz = source.capture_center_frequency_hz();
        let input_rate = source.sample_rate_hz;
        let offset = (i128::from(settings.frequency_hz) - i128::from(center_hz)) as f64;
        let rf_width = normalized_rf_width(settings, mode);
        let Some(decimation) = channel_decimation(input_rate, offset, rf_width, mode) else {
            self.fail("B frequency is outside the active RF capture; retune the primary capture center or choose a closer B frequency.");
            return;
        };
        // The decimator emits at the exact rational rate. Keeping only the
        // integer quotient skews the demodulator clock for replay/capture
        // rates that are not divisible by the decimation factor.
        let channel_rate = f64::from(input_rate) / f64::from(decimation);
        let channels = output_channels(mode, settings.wfm_stereo);
        if let Err(error) = self.ensure_output(selection, channels) {
            self.fail(error);
            return;
        }
        let Some(sink) = self.output.sink() else {
            self.fail("B audio output is unavailable.");
            return;
        };
        let worker = self
            .local_worker
            .get_or_insert_with(|| DemodWorker::new_capture(self.capture.clone()));
        let audio_rate = self.output.sample_rate().max(8_000);
        let squelch_db = finite_clamp(settings.squelch_db, -120.0, 0.0, -100.0);
        let volume = finite_clamp(settings.volume, 0.0, 1.0, 0.5);
        let radio = RadioConfig {
            enabled: true,
            preview: false,
            measure_power: true,
            frequency_hz: settings.frequency_hz,
            center_hz,
            generation: source.stream_generation(),
            iq: RadioIqConfig {
                input_rate,
                decimation,
                dc_remove: true,
                invert: false,
            },
            audio_running: true,
            volume,
            squelch_db,
            squelch_mode: if squelch_db > -100.0 {
                SquelchMode::Power
            } else {
                SquelchMode::Off
            },
            ctcss_tone_hz: None,
            rds_enabled: false,
            rds_incremental: false,
            rds_region: crate::radio_rds::RdsRegion::Europe,
            sink: Some(sink),
        };
        let demod = DemodConfig {
            mode,
            input_rate: channel_rate,
            audio_rate,
            lpf_cutoff: if mode == DemodMode::Wfm {
                15_000.0
            } else {
                5_000.0
            },
            rf_bandwidth: rf_width as f32,
            stereo: mode == DemodMode::Wfm && settings.wfm_stereo,
        };
        let advanced = crate::config::AdvancedConfig {
            cw_tone_hz: finite_clamp(settings.cw_tone_hz, 100.0, 2_000.0, 800.0),
            wfm_stereo: settings.wfm_stereo,
            ..Default::default()
        };
        worker.configure_advanced(&advanced, mode);
        worker.configure_radio(radio, demod);
        self.error = None;
        self.status = if settings.wfm_stereo && mode == DemodMode::Wfm {
            if worker
                .radio_snapshot()
                .is_some_and(|snapshot| snapshot.stereo_locked)
            {
                "Listening on B · WFM stereo pilot locked".into()
            } else {
                "Listening on B · WFM stereo waiting for pilot; mono fallback".into()
            }
        } else {
            format!("Listening on B · {}", mode.label())
        };
    }

    fn tick_daemon(
        &mut self,
        source: &SourceManager,
        settings: &SecondaryVfoSettings,
        selection: &AudioOutputSelection,
        mode: DemodMode,
    ) {
        self.local_worker = None;
        if source.status != SourceStatus::Running {
            self.stop();
            self.error = None;
            self.status = "Waiting for the primary daemon connection".into();
            return;
        }
        let center_hz = source.capture_center_frequency_hz();
        let offset = i128::from(settings.frequency_hz) - i128::from(center_hz);
        let Ok(offset_hz) = i64::try_from(offset) else {
            self.fail("B frequency cannot be represented as a daemon capture offset.");
            return;
        };
        let input_rate = source.sample_rate_hz;
        let rf_width = normalized_rf_width(settings, mode);
        if channel_decimation(input_rate, offset_hz as f64, rf_width, mode).is_none() {
            self.fail("B frequency is outside the daemon capture; retune the primary capture center or choose a closer B frequency.");
            return;
        }
        // The daemon audio pipeline currently exposes RAW as mono-I; keep the
        // device geometry aligned with the actual wire frame.
        let channels = usize::from(mode == DemodMode::Wfm && settings.wfm_stereo) + 1;
        if let Err(error) = self.ensure_output(selection, channels) {
            self.fail(error);
            return;
        }
        let channel_rate = match mode {
            DemodMode::Wfm => 256_000,
            DemodMode::Cw => 48_000,
            _ => 200_000,
        }
        .min(input_rate);
        let Ok(addr) = source.daemon_addr.parse::<SocketAddr>() else {
            self.fail("Daemon address must be an IP address and port.");
            return;
        };
        if self.daemon.is_none() {
            let token = (!source.daemon_token.is_empty())
                .then(|| ez_proto::AuthToken::new(source.daemon_token.clone()));
            self.daemon = Some(DaemonPath {
                client: crate::daemon_client::DaemonClient::connect_with_token(
                    addr,
                    "ez-gui-secondary-vfo".into(),
                    token,
                ),
                channel_id: None,
                applied: None,
                squelch_db: None,
            });
        }
        let Some(daemon) = self.daemon.as_mut() else {
            self.fail("Could not create the B daemon connection.");
            return;
        };
        match daemon.client.status() {
            crate::daemon_client::ConnectionStatus::Connecting => {
                self.error = None;
                self.status = "Connecting B to daemon…".into();
                return;
            }
            crate::daemon_client::ConnectionStatus::Error(error) => {
                self.error = Some(error);
                self.status = "B daemon connection failed; toggle B off and on to retry".into();
                self.output.stop();
                self.output_shape = None;
                return;
            }
            crate::daemon_client::ConnectionStatus::Disconnected => {
                self.error = Some("B daemon connection was closed.".into());
                self.status = "B daemon connection closed; toggle B off and on to retry".into();
                self.output.stop();
                self.output_shape = None;
                return;
            }
            crate::daemon_client::ConnectionStatus::Connected => {}
        }
        let Some(client_channels) = daemon.client.client_channels() else {
            self.fail("B daemon handshake did not provide channel IDs.");
            return;
        };
        let channel_id = *daemon.channel_id.get_or_insert_with(|| {
            client_channels
                .ids()
                .into_iter()
                .max()
                .unwrap_or(ez_proto::CLIENT_CHANNEL_START)
                .saturating_add(1)
        });
        let protocol_mode = daemon_mode(mode);
        let mut parameters = ez_proto::AudioParameters::for_mode(protocol_mode);
        parameters.rf_bandwidth_hz = rf_width;
        parameters.output_sample_rate_hz = self.output.sample_rate().max(8_000);
        parameters.stereo = mode == DemodMode::Wfm && settings.wfm_stereo;
        parameters.cw_tone_hz =
            finite_clamp(settings.cw_tone_hz, 100.0, 2_000.0, 800.0).round() as u32;
        let config = ez_proto::AudioConfiguration {
            mode: protocol_mode,
            parameters,
        };
        let desired = (offset_hz, channel_rate, config);
        if daemon.applied != Some(desired) {
            let command = if daemon.applied.is_some() {
                ClientCommand::Retune {
                    channel_id,
                    center_offset_hz: offset_hz,
                    channel_sample_rate_hz: channel_rate,
                    audio: Some(config),
                }
            } else {
                ClientCommand::Subscribe {
                    channel: ez_proto::ChannelSpec {
                        audio: Some(config.parameters),
                        meteor_profile: None,
                        id: channel_id,
                        center_offset_hz: offset_hz,
                        channel_sample_rate_hz: channel_rate,
                        kind: PipelineKind::Audio,
                        demod_mode: Some(protocol_mode),
                    },
                }
            };
            if !daemon.client.try_send(command) {
                self.fail("B daemon command queue is closed.");
                return;
            }
            daemon.applied = Some(desired);
        }
        let squelch_db = finite_clamp(settings.squelch_db, -120.0, 0.0, -100.0);
        if daemon.squelch_db != Some(squelch_db) {
            if !daemon.client.try_send(ClientCommand::SetSquelch {
                request_id: 1,
                channel_id,
                db: squelch_db,
            }) {
                self.fail("B daemon squelch command queue is closed.");
                return;
            }
            daemon.squelch_db = Some(squelch_db);
        }
        let output_rate = self.output.sample_rate().max(8_000);
        let mut frame_count = 0;
        while let Some(event) = daemon.client.try_recv_event() {
            match event {
                ServerEvent::Audio(frame) if frame.channel_id == channel_id => {
                    let samples =
                        self.resampler
                            .process_frame(&frame, output_rate, settings.volume);
                    if !samples.is_empty() {
                        self.output.push_audio(samples);
                    }
                    frame_count += 1;
                    if frame_count >= 16 {
                        break;
                    }
                }
                ServerEvent::Error { message } => {
                    self.error = Some(message);
                    self.status = "B daemon reported an error; check the message above".into();
                }
                _ => {}
            }
        }
        if self.error.is_none() {
            self.status = if settings.wfm_stereo && mode == DemodMode::Wfm {
                "Listening on B · daemon WFM stereo when pilot locks; mono fallback otherwise"
                    .into()
            } else {
                format!("Listening on B · daemon {}", mode.label())
            };
        }
    }

    fn ensure_output(
        &mut self,
        selection: &AudioOutputSelection,
        channels: usize,
    ) -> Result<(), String> {
        let wanted = (selection.clone(), channels);
        if self.output_shape.as_ref() == Some(&wanted) && self.output.is_running() {
            return Ok(());
        }
        self.output.stop();
        let (_tx, rx) = crossbeam_channel::bounded(1);
        self.output
            .start_with_selection_channels(rx, selection, channels)?;
        self.output_shape = Some(wanted);
        self.resampler.reset();
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn set_test_output_start_hook(
        &mut self,
        hook: impl Fn(&AudioOutputSelection, usize) -> Result<u32, String> + Send + Sync + 'static,
    ) {
        self.output.set_test_start_hook(hook);
    }

    fn fail(&mut self, message: impl Into<String>) {
        self.error = Some(message.into());
        self.status = "B receiver unavailable".into();
        self.local_worker = None;
        self.daemon = None;
        self.output.stop();
        self.output_shape = None;
        self.resampler.reset();
    }
}

fn finite_clamp(value: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

fn normalized_rf_width(settings: &SecondaryVfoSettings, mode: DemodMode) -> u32 {
    let default = mode.default_rf_bandwidth_hz();
    let selected = if settings.rf_bandwidth_hz.is_finite() && settings.rf_bandwidth_hz > 0.0 {
        settings.rf_bandwidth_hz
    } else {
        default
    };
    selected.clamp(100.0, 500_000.0).round() as u32
}

fn channel_decimation(input_rate: u32, offset: f64, width: u32, mode: DemodMode) -> Option<u32> {
    if input_rate < 8_000 || !offset.is_finite() {
        return None;
    }
    let edge = offset.abs()
        + if matches!(mode, DemodMode::Lsb | DemodMode::Usb) {
            f64::from(width)
        } else {
            f64::from(width) * 0.5
        };
    let max_decimation = (input_rate / 8_000).next_power_of_two().checked_div(2)?;
    let max_decimation = max_decimation.clamp(1, 1024);
    let mut candidate = 1;
    while candidate <= max_decimation {
        candidate *= 2;
    }
    candidate /= 2;
    while candidate >= 1 {
        let rate = f64::from(input_rate / candidate);
        if rate >= 8_000.0 && edge <= rate * 0.4 {
            return Some(candidate);
        }
        if candidate == 1 {
            break;
        }
        candidate /= 2;
    }
    None
}

fn output_channels(mode: DemodMode, wfm_stereo: bool) -> usize {
    usize::from(mode == DemodMode::Raw || (mode == DemodMode::Wfm && wfm_stereo)) + 1
}

fn daemon_mode(mode: DemodMode) -> ez_proto::DemodMode {
    match mode {
        DemodMode::Raw => ez_proto::DemodMode::Raw,
        DemodMode::Auto => ez_proto::DemodMode::Fm,
        DemodMode::Am => ez_proto::DemodMode::Am,
        DemodMode::Fm => ez_proto::DemodMode::Fm,
        DemodMode::Wfm => ez_proto::DemodMode::Wfm,
        DemodMode::Lsb => ez_proto::DemodMode::Lsb,
        DemodMode::Usb => ez_proto::DemodMode::Usb,
        DemodMode::Dsb => ez_proto::DemodMode::Dsb,
        DemodMode::Cw => ez_proto::DemodMode::Cw,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn enabled_daemon_b_waits_for_the_primary_source_connection() {
        let mut source = SourceManager::new();
        source.source_mode = SourceMode::Daemon;
        source.status = SourceStatus::Idle;
        let mut receiver = SecondaryVfo::new(source.capture_stream());
        receiver.tick(
            &source,
            &SecondaryVfoSettings {
                enabled: true,
                ..SecondaryVfoSettings::default()
            },
            &AudioOutputSelection::default(),
        );
        assert!(receiver.daemon.is_none());
        assert!(!receiver.output.is_running());
        assert!(receiver.status.contains("Waiting"));
    }

    use super::*;
    use crate::audio_output::AudioSink;
    use crate::demod::RadioConfig;
    use crate::radio_iq::RadioIqConfig;
    use ez_daemon::broadcast::OverflowPolicy;
    use num_complex::Complex32;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn settings_are_backward_compatible_and_canonical_mode_labels_parse() {
        let old: SecondaryVfoSettings = serde_json::from_str("{}").unwrap();
        assert!(!old.enabled);
        assert_eq!(old.mode_label, "FM");
        assert_eq!(DemodMode::from_label(&old.mode_label), Some(DemodMode::Fm));
        for mode in [
            DemodMode::Am,
            DemodMode::Fm,
            DemodMode::Wfm,
            DemodMode::Lsb,
            DemodMode::Usb,
            DemodMode::Dsb,
            DemodMode::Cw,
            DemodMode::Raw,
        ] {
            assert_eq!(DemodMode::from_label(mode.label()), Some(mode));
        }
    }

    #[test]
    fn capture_coverage_accounts_for_vfo_offset_and_demod_passband() {
        assert_eq!(
            channel_decimation(2_048_000, 0.0, 150_000, DemodMode::Wfm),
            Some(8)
        );
        assert_eq!(
            channel_decimation(2_048_000, 50_000.0, 150_000, DemodMode::Wfm),
            Some(4)
        );
        assert_eq!(
            channel_decimation(2_048_000, 750_000.0, 12_500, DemodMode::Fm),
            Some(1)
        );
        assert_eq!(
            channel_decimation(2_048_000, 1_100_000.0, 12_500, DemodMode::Fm),
            None
        );
        assert_eq!(channel_decimation(0, 0.0, 12_500, DemodMode::Fm), None);
    }

    #[test]
    fn channel_decimation_keeps_entire_cw_ssb_and_stereo_wfm_passbands() {
        for (mode, width) in [
            (DemodMode::Cw, 200),
            (DemodMode::Lsb, 2_800),
            (DemodMode::Wfm, 150_000),
        ] {
            let factor = channel_decimation(2_048_000, 30_000.0, width, mode).unwrap();
            assert!(30_000.0 + f64::from(width) <= f64::from(2_048_000 / factor) * 0.4);
        }
        assert_eq!(output_channels(DemodMode::Wfm, true), 2);
        assert_eq!(output_channels(DemodMode::Wfm, false), 1);
        assert_eq!(output_channels(DemodMode::Am, true), 1);
    }

    fn fm_carrier(
        sample_rate: u32,
        count: usize,
        start: usize,
        offset_hz: f64,
        tone_hz: f64,
    ) -> Vec<Complex32> {
        (0..count)
            .map(|index| {
                let time = (start + index) as f64 / f64::from(sample_rate);
                let phase = std::f64::consts::TAU * offset_hz * time
                    + 2_000.0 / tone_hz * (std::f64::consts::TAU * tone_hz * time).sin();
                Complex32::from_polar(0.28, phase as f32)
            })
            .collect()
    }

    fn combined_fm(sample_rate: u32, count: usize, start: usize) -> Vec<Complex32> {
        let left = fm_carrier(sample_rate, count, start, -25_000.0, 700.0);
        let right = fm_carrier(sample_rate, count, start, 30_000.0, 1_700.0);
        left.into_iter()
            .zip(right)
            .map(|(left, right)| left + right)
            .collect()
    }

    fn native_radio(
        frequency_hz: u64,
        center_hz: u64,
        input_rate: u32,
        sink: AudioSink,
        volume: f32,
    ) -> (RadioConfig, DemodConfig) {
        let width = 12_500.0;
        let decimation = channel_decimation(
            input_rate,
            (i128::from(frequency_hz) - i128::from(center_hz)) as f64,
            width as u32,
            DemodMode::Fm,
        )
        .unwrap();
        let channel_rate = f64::from(input_rate) / f64::from(decimation);
        (
            RadioConfig {
                enabled: true,
                preview: false,
                measure_power: false,
                frequency_hz,
                center_hz,
                generation: 1,
                iq: RadioIqConfig {
                    input_rate,
                    decimation,
                    dc_remove: false,
                    invert: false,
                },
                audio_running: true,
                volume,
                squelch_db: -100.0,
                squelch_mode: crate::radio_squelch::SquelchMode::Off,
                ctcss_tone_hz: None,
                rds_enabled: false,
                rds_incremental: false,
                rds_region: crate::radio_rds::RdsRegion::Europe,
                sink: Some(sink),
            },
            DemodConfig {
                mode: DemodMode::Fm,
                input_rate: channel_rate,
                audio_rate: 48_000,
                lpf_cutoff: 12_000.0,
                rf_bandwidth: width,
                stereo: false,
            },
        )
    }

    fn publish_complex(
        stream: &CaptureStream,
        samples: Vec<Complex32>,
        start_sample: u64,
        sample_rate: u32,
    ) {
        let mut block = crate::source_manager::CaptureBlock::fixture(
            vec![127; samples.len() * 2],
            ez_proto::CaptureClock::new(77, start_sample, sample_rate, None),
            1,
            start_sample,
        );
        block.center_hz = 100_000_000;
        block.float_samples = Some(Arc::from(samples));
        block.session = Some(stream.session());
        stream.publish(&block);
    }

    fn take_audio(rx: &crossbeam_channel::Receiver<Vec<f32>>) -> Vec<f32> {
        rx.recv_timeout(Duration::from_secs(3))
            .expect("native B worker should produce an audio frame")
    }

    fn tone_amplitude(samples: &[f32], frequency_hz: f64, sample_rate_hz: f64) -> f64 {
        let samples = &samples[samples.len().min(2_000)..];
        let mut real = 0.0;
        let mut imaginary = 0.0;
        for (index, sample) in samples.iter().enumerate() {
            let phase = std::f64::consts::TAU * frequency_hz * index as f64 / sample_rate_hz;
            real += f64::from(*sample) * phase.cos();
            imaginary -= f64::from(*sample) * phase.sin();
        }
        2.0 * real.hypot(imaginary) / samples.len().max(1) as f64
    }

    #[test]
    fn two_capture_vfos_recover_independent_fm_tones_then_mute_retune_and_reset_on_gap() {
        let sample_rate = 512_003;
        let count = sample_rate as usize / 2;
        let stream = CaptureStream::default();
        stream.start(1);
        let (sink_a, audio_a) = AudioSink::fixture(8);
        let (sink_b, audio_b) = AudioSink::fixture(8);
        let worker_a = DemodWorker::new_capture(stream.clone());
        let worker_b = DemodWorker::new_capture(stream.clone());
        let (radio_a, demod_a) = native_radio(99_975_000, 100_000_000, sample_rate, sink_a, 1.0);
        let (radio_b, demod_b) =
            native_radio(100_030_000, 100_000_000, sample_rate, sink_b.clone(), 1.0);
        assert_eq!(demod_b.input_rate, f64::from(sample_rate) / 4.0);
        worker_a.configure_radio(radio_a, demod_a);
        worker_b.configure_radio(radio_b, demod_b);
        let bus_b = worker_b.audio_stream().unwrap();
        let observed_b = bus_b.bus.subscribe(8, OverflowPolicy::DropOldest);
        publish_complex(&stream, combined_fm(sample_rate, count, 0), 0, sample_rate);

        let a = take_audio(&audio_a);
        let b = take_audio(&audio_b);
        observed_b.try_recv().expect("initial B clock metadata");
        assert!(tone_amplitude(&a, 700.0, 48_000.0) > tone_amplitude(&a, 1_700.0, 48_000.0) * 3.0);
        assert!(tone_amplitude(&b, 1_700.0, 48_000.0) > tone_amplitude(&b, 700.0, 48_000.0) * 3.0);

        let (mut retuned_b, demod_b) =
            native_radio(99_975_000, 100_000_000, sample_rate, sink_b.clone(), 0.0);
        retuned_b.volume = 0.0;
        worker_b.configure_radio(retuned_b, demod_b);
        publish_complex(
            &stream,
            combined_fm(sample_rate, count, count),
            count as u64,
            sample_rate,
        );
        let muted = take_audio(&audio_b);
        assert!(muted.iter().all(|sample| sample.abs() < 1e-7));
        observed_b.try_recv().expect("muted B clock metadata");

        let (radio_b, demod_b) = native_radio(100_030_000, 100_000_000, sample_rate, sink_b, 1.0);
        worker_b.configure_radio(radio_b, demod_b);
        // Skip one complete half-second block. The B chain must retire its old
        // discriminator/filter history and mark the recovered clock discontinuous.
        publish_complex(
            &stream,
            combined_fm(sample_rate, count, count * 3),
            (count * 3) as u64,
            sample_rate,
        );
        let recovered = take_audio(&audio_b);
        assert!(tone_amplitude(&recovered, 1_700.0, 48_000.0) > 0.01);
        let frame = observed_b
            .recv_timeout(Duration::from_secs(3))
            .expect("B metadata frame after a capture gap");
        assert!(frame.clock.discontinuous);
        assert_eq!(frame.center_hz, 100_000_000);
        assert_eq!(frame.input_samples, count as u64);
    }
}
