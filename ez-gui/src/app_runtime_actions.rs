//! Approved runtime operations are serialized through the app's live lifecycle.
//! The AI panel receives a result only after application or a bounded failure.
use super::*;
use crate::source_manager::{SourceMode, SourceStatus};

pub(crate) const RUNTIME_ACTION_CAPACITY: usize = 32;
const ACTION_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub(crate) enum RuntimeAction {
    Tune(u64),
    Gain(f64),
    SampleRate(u32),
    BiasTee(bool),
    Ppm(i32),
    StartRecording,
    StopRecording,
    StartAdsb,
    StopAdsb,
    Squelch(f32),
    Demod(crate::sdr_panel::DemodMode),
    SelectSatellite(String),
}
impl RuntimeAction {
    pub(crate) fn from_tool(name: &str, args: &serde_json::Value) -> Option<Result<Self, String>> {
        let finite = |key: &str| {
            args[key]
                .as_f64()
                .filter(|value| value.is_finite())
                .ok_or_else(|| format!("missing or nonfinite {key} argument"))
        };
        Some(match name {
            "tune_frequency" => args["hz"]
                .as_u64()
                .map(|hz| Self::Tune(hz.clamp(500_000, 1_770_000_000)))
                .ok_or("missing hz argument".into()),
            "set_gain" => finite("db").map(Self::Gain),
            "set_sample_rate" => args["rate"]
                .as_u64()
                .filter(|rate| *rate > 0 && *rate <= u64::from(u32::MAX))
                .map(|rate| Self::SampleRate(rate as u32))
                .ok_or("sample rate must be non-zero and fit u32".into()),
            "toggle_bias_tee" => args["on"]
                .as_bool()
                .map(Self::BiasTee)
                .ok_or("missing on argument".into()),
            "set_ppm" => args["ppm"]
                .as_i64()
                .map(|ppm| Self::Ppm(ppm.clamp(-10_000, 10_000) as i32))
                .ok_or("missing ppm argument".into()),
            "start_recording" => Ok(Self::StartRecording),
            "stop_recording" => Ok(Self::StopRecording),
            "start_adsb" => Ok(Self::StartAdsb),
            "stop_adsb" => Ok(Self::StopAdsb),
            "set_squelch" => finite("db").and_then(|db| {
                if (-240.0..=60.0).contains(&db) {
                    Ok(Self::Squelch(db as f32))
                } else {
                    Err("squelch must be between -240 and 60 dBFS".into())
                }
            }),
            "set_demod" => args["mode"]
                .as_str()
                .and_then(|mode| crate::sdr_panel::DemodMode::from_label(&mode.to_uppercase()))
                .map(Self::Demod)
                .ok_or("unsupported or missing mode argument".into()),
            "select_satellite" => args["name"]
                .as_str()
                .map(|name| Self::SelectSatellite(name.into()))
                .ok_or("missing name argument".into()),
            _ => return None,
        })
    }
    pub(crate) fn description(&self) -> String {
        match self {
            Self::Tune(hz) => format!("VFO {:.3} MHz", *hz as f64 / 1e6),
            Self::Gain(db) => format!("gain {db:.1} dB"),
            Self::SampleRate(rate) => format!("sample rate {:.3} MSps", f64::from(*rate) / 1e6),
            Self::BiasTee(on) => format!("bias tee {}", if *on { "ON" } else { "OFF" }),
            Self::Ppm(ppm) => format!("correction {ppm} ppm"),
            Self::StartRecording => "start recording".into(),
            Self::StopRecording => "stop recording".into(),
            Self::StartAdsb => "start ADS-B at 1090 MHz / 2.4 MSps".into(),
            Self::StopAdsb => "stop ADS-B".into(),
            Self::Squelch(db) => format!("squelch {db:.1} dBFS"),
            Self::Demod(mode) => format!("demodulation {}", mode.label()),
            Self::SelectSatellite(name) => format!("select satellite {name}"),
        }
    }
}
pub(crate) struct RuntimeActionRequest {
    pub action: RuntimeAction,
    pub result: crossbeam_channel::Sender<Result<String, String>>,
}
pub(crate) struct PendingRuntimeAction {
    request: RuntimeActionRequest,
    started: Instant,
    error_serial: u64,
}

impl CentralApp {
    pub(crate) fn tick_runtime_actions(&mut self) {
        if let Some(pending) = self.runtime_action.take() {
            let result = if self.runtime_error_serial != pending.error_serial {
                Some(Err(format!(
                    "daemon reported an error while awaiting {}: {}",
                    pending.request.action.description(),
                    self.runtime_last_error
                )))
            } else {
                self.runtime_action_result(&pending.request.action)
            };
            let result = result.or_else(|| {
                (pending.started.elapsed() >= ACTION_TIMEOUT).then(|| {
                    Err(format!(
                        "timed out awaiting {}; inspect receiver/recorder state before retrying",
                        pending.request.action.description()
                    ))
                })
            });
            if let Some(result) = result {
                if result.is_err() {
                    if let Ok(mut state) = self.shared.try_lock() {
                        if matches!(pending.request.action, RuntimeAction::StartAdsb) {
                            state.adsb_running = false;
                        }
                        state.source.cancel_daemon_control_request();
                    }
                }
                let _ = pending.request.result.try_send(result);
            } else {
                self.runtime_action = Some(pending);
            }
        }
        if self.runtime_action.is_some() {
            return;
        }
        let request = self
            .shared
            .try_lock()
            .ok()
            .and_then(|mut state| state.runtime_actions.pop_front());
        let Some(request) = request else {
            return;
        };
        let error_serial = self.runtime_error_serial;
        match self.apply_runtime_action(&request.action) {
            Ok(()) => {
                self.runtime_action = Some(PendingRuntimeAction {
                    request,
                    started: Instant::now(),
                    error_serial,
                })
            }
            Err(error) => {
                let _ = request.result.try_send(Err(error));
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn test_expire_runtime_action(&mut self) {
        self.runtime_action.as_mut().unwrap().started = Instant::now() - ACTION_TIMEOUT;
    }

    fn apply_runtime_action(&mut self, action: &RuntimeAction) -> Result<(), String> {
        match action {
            RuntimeAction::StartRecording => {
                let state = self
                    .shared
                    .try_lock()
                    .map_err(|_| "receiver state is busy")?;
                if state.source.status != SourceStatus::Running
                    || !state.source.capture_tuning_applied()
                {
                    return Err("wait for an acknowledged running receiver before recording".into());
                }
                if self.recorder_panel.recording && state.recording {
                    return Ok(());
                }
                drop(state);
                self.recorder_panel.start();
                if !self.recorder_panel.recording
                    && !self
                        .recorder_panel
                        .last_error
                        .starts_with("Starting daemon")
                {
                    return Err(self.recorder_panel.last_error.clone());
                }
            }
            RuntimeAction::StopRecording => {
                if !self.recorder_panel.recording {
                    return Ok(());
                }
                self.recorder_panel.last_error.clear();
                self.recorder_panel.stop();
                if !self.recorder_panel.last_error.is_empty()
                    && !self
                        .recorder_panel
                        .last_error
                        .starts_with("Stopping daemon")
                {
                    return Err(self.recorder_panel.last_error.clone());
                }
            }
            RuntimeAction::StartAdsb => {
                let state = self
                    .shared
                    .try_lock()
                    .map_err(|_| "receiver state is busy")?;
                if state.source.source_mode == SourceMode::Replay {
                    return Err("ADS-B acquisition cannot retune a replay file; use a compatible 1090 MHz / 2.4 MSps capture in Planes".into());
                }
                drop(state);
                self.adsb_panel
                    .set_region(crate::adsb_panel::AdsbRegion::ModeS1090);
                self.adsb_panel.begin();
            }
            RuntimeAction::StopAdsb => self.adsb_panel.stop_receiving(),
            RuntimeAction::SelectSatellite(name) => {
                self.satellite_panel.select_named_satellite(name)?
            }
            _ => {
                let mut state = self
                    .shared
                    .try_lock()
                    .map_err(|_| "receiver state is busy")?;
                if state.source.source_mode == SourceMode::Daemon
                    && state.source.status == SourceStatus::Opening
                {
                    return Err("wait for the daemon's initial hardware acknowledgement before changing controls".into());
                }
                match action {
                    RuntimeAction::Tune(hz) => crate::radio_ui::tune(&mut state, *hz),
                    RuntimeAction::Gain(gain) => {
                        if state.source.source_mode == SourceMode::Replay {
                            return Err("RF gain is unavailable for file replay".into());
                        }
                        if state.source.source_mode == SourceMode::Hardware
                            && !(0.0..=49.6).contains(gain)
                        {
                            return Err("RTL gain must be between 0 and 49.6 dB".into());
                        }
                        if state.source.gain_db != *gain {
                            state.source.gain_db = *gain;
                            crate::radio_ui::restart_if_running(&mut state.source);
                        }
                    }
                    RuntimeAction::SampleRate(rate) => {
                        if state.source.sample_rate_hz != *rate {
                            state.source.sample_rate_hz = *rate;
                            crate::radio_ui::restart_if_running(&mut state.source);
                        }
                    }
                    RuntimeAction::BiasTee(on) => {
                        if state.source.source_mode != SourceMode::Hardware {
                            return Err(
                                "bias tee control is supported only by local RTL hardware".into()
                            );
                        }
                        if state.source.bias_tee != *on {
                            state.source.bias_tee = *on;
                            crate::radio_ui::restart_if_running(&mut state.source);
                        }
                    }
                    RuntimeAction::Ppm(ppm) => {
                        if state.source.source_mode != SourceMode::Hardware {
                            return Err(
                                "PPM control is supported only by local RTL hardware".into()
                            );
                        }
                        if state.source.ppm_correction != *ppm {
                            state.source.ppm_correction = *ppm;
                            crate::radio_ui::restart_if_running(&mut state.source);
                        }
                    }
                    RuntimeAction::Squelch(db) => {
                        crate::radio_ui::set_power_squelch_level(&mut state, *db)?
                    }
                    RuntimeAction::Demod(mode) => {
                        if state.source.source_mode == SourceMode::Daemon
                            && matches!(
                                mode,
                                crate::sdr_panel::DemodMode::Raw
                                    | crate::sdr_panel::DemodMode::Dsb
                                    | crate::sdr_panel::DemodMode::Cw
                            )
                        {
                            return Err(
                                "this demodulation mode is unavailable in daemon audio".into()
                            );
                        }
                        crate::radio_ui::select_demod(&mut state, *mode);
                    }
                    _ => unreachable!(),
                }
            }
        }
        Ok(())
    }

    fn runtime_action_result(&self, action: &RuntimeAction) -> Option<Result<String, String>> {
        let state = self.shared.try_lock().ok()?;
        let source = &state.source;
        if !matches!(
            action,
            RuntimeAction::StopRecording | RuntimeAction::StopAdsb | RuntimeAction::Demod(_)
        ) {
            if let SourceStatus::Error(error) = &source.status {
                return Some(Err(error.clone()));
            }
        }
        let capture_ready =
            source.status == SourceStatus::Running && source.capture_tuning_applied();
        let preference = |message: String| {
            Some(Ok(format!(
                "{message}; preference retained while receiver is stopped"
            )))
        };
        let ready = match action {
            RuntimeAction::StartRecording => {
                if self.recorder_panel.recording && state.recording && !self.recorder_panel.last_filename.is_empty() {
                    return Some(Ok(format!("Recorder opened {}", self.recorder_panel.last_filename)));
                }
                if !self.recorder_panel.last_error.is_empty() && !self.recorder_panel.last_error.starts_with("Starting daemon") { return Some(Err(self.recorder_panel.last_error.clone())); }
                false
            }
            RuntimeAction::StopRecording => {
                if !self.recorder_panel.last_error.is_empty() && !self.recorder_panel.last_error.starts_with("Stopping daemon") { return Some(Err(self.recorder_panel.last_error.clone())); }
                if !self.recorder_panel.recording && !state.recording {
                    if self.recorder_panel.last_filename.is_empty() { return Some(Ok("Recorder is already stopped".into())); }
                    return Some(Ok(format!("Recorder stopped and finalized {} bytes", self.recorder_panel.bytes_written)));
                }
                false
            }
            RuntimeAction::StartAdsb => state.adsb_running && capture_ready && source.frequency_hz == 1_090_000_000
                && source.capture_center_frequency_hz() == 1_090_000_000 && source.sample_rate_hz == 2_400_000
                && (source.source_mode != SourceMode::Daemon || source.daemon_adsb_ready()),
            RuntimeAction::StopAdsb => !state.adsb_running && source.status == SourceStatus::Idle,
            RuntimeAction::Tune(frequency) => {
                if source.frequency_hz != *frequency { return Some(Err("requested VFO is outside the available capture or was superseded".into())); }
                if source.status == SourceStatus::Idle { return preference(action.description()); }
                capture_ready
            }
            RuntimeAction::Gain(gain) => {
                if source.source_mode == SourceMode::Simulated { return Some(Ok(format!("Demo gain preference set to {gain:.1} dB"))); }
                if source.status == SourceStatus::Idle { return preference(action.description()); }
                capture_ready && source.gain_request_applied(*gain)
            }
            RuntimeAction::SampleRate(rate) => {
                if source.status == SourceStatus::Idle { return preference(action.description()); }
                capture_ready && source.sample_rate_hz == *rate
            }
            RuntimeAction::BiasTee(_) | RuntimeAction::Ppm(_) => {
                if source.status == SourceStatus::Idle { return preference(action.description()); }
                capture_ready && source.local_hardware_request_applied()
            }
            RuntimeAction::Squelch(db) => {
                if let Some(error) = &source.daemon_squelch_error { return Some(Err(error.clone())); }
                if source.status == SourceStatus::Idle { return preference(action.description()); }
                if source.source_mode != SourceMode::Daemon { return Some(Ok(format!("Power squelch preference set to {db:.1} dBFS"))); }
                source.daemon_squelch_pending_db().is_none() && source.daemon_squelch_applied().is_some_and(|applied| applied.db == *db)
            }
            RuntimeAction::Demod(mode) => return Some(Ok(format!("Demodulation preference set to {}; active processing synchronizes it on the app cycle", mode.label()))),
            RuntimeAction::SelectSatellite(name) => {
                if name.to_ascii_lowercase().contains("meteor") { return Some(Ok("Meteor orbit selected for tracking; decoding uses existing files in the Meteor tab".into())); }
                if source.status == SourceStatus::Idle { return preference(action.description()); }
                capture_ready
            }
        };
        ready.then(|| Ok(format!("Receiver acknowledged {}", action.description())))
    }
}
