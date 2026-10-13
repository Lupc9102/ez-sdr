//! Central daemon state: owns the wideband hardware ingestion handle, the channelizer,
//! every currently-active per-channel pipeline, and the recording manager. Every connection
//! task (see `crate::server`) talks to the daemon exclusively through this type's methods —
//! it is the one place command routing and pipeline lifecycle meet.
//!
//! Low channel IDs are explicitly shared daemon resources. High IDs are reserved
//! per connection and used by GUI workflows. Unsubscribe only drops that connection's
//! forwarder; explicit deletion is reported to remaining control subscribers.
//! Active recordings prevent channel deletion and survive disconnect.

use parking_lot::FairMutex as Mutex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use ez_proto::{
    AudioConfiguration, AudioFrame, AudioParameters, ChannelId, ChannelSpec, DemodMode,
    HardwareStatus, PipelineKind, RecordingFormat, RecordingStatus, SpectrumFrame, TelemetryFrame,
};

use crate::auth::Authenticator;
use crate::broadcast::BroadcasterHandle;
use crate::bus::{OverflowPolicy, SampleBus, SampleBusHandle};
use crate::channelizer::Channelizer;
use crate::ingest::IngestHandle;
use crate::pipelines::audio::{AudioConfig, AudioPipeline};
use crate::pipelines::packet::PacketPipeline;
use crate::pipelines::spectrum::{SpectrumConfig, SpectrumPipeline};
use crate::pipelines::telemetry::{TelemetryConfig, TelemetryPipeline};
use crate::recording::RecordingManager;

const PIPELINE_POLL: Duration = Duration::from_millis(100);
const DEFAULT_SUBSCRIBE_CAPACITY: usize = 32;
const RECORDING_SUBSCRIBE_CAPACITY: usize = 64;
const WIDEBAND_SUBSCRIBE_CAPACITY: usize = 64;

enum ActivePipeline {
    Spectrum(Arc<Mutex<SpectrumPipeline>>),
    Audio(Arc<Mutex<AudioPipeline>>),
    AdsbPackets(Arc<Mutex<PacketPipeline>>),
    LrptTelemetry(Arc<Mutex<TelemetryPipeline>>),
}

struct ActiveChannel {
    spec: ChannelSpec,
    /// The channelizer's own id for this channel's virtual tap, or `None` for `Spectrum`
    /// channels (which subscribe directly to the wideband bus rather than a decimated tap —
    /// see the `PipelineKind::Spectrum` doc comment in `ez_proto`).
    internal_channel_id: Option<u64>,
    pipeline: ActivePipeline,
    running: Arc<AtomicBool>,
}

/// One caller's fresh output subscription to an active channel, typed per pipeline kind so
/// the connection task can translate each item into the right `ServerEvent` variant.
pub enum ChannelSubscription {
    Spectrum(BroadcasterHandle<SpectrumFrame>),
    Audio(BroadcasterHandle<AudioFrame>),
    AdsbPackets(BroadcasterHandle<ez_proto::AircraftSnapshot>),
    LrptTelemetry(BroadcasterHandle<TelemetryFrame>),
}

/// Read-only snapshot of one active channel for reporting to non-streaming observers (the
/// web API's `GET /api/channels` — see `crate::web`). Distinct from [`ChannelSubscription`],
/// which hands over a live receiving end rather than a point-in-time count.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMetrics {
    pub spec: ChannelSpec,
    /// How many independent subscribers (any transport: TCP forwarder threads, web
    /// WebSocket forwarders) are currently attached to this channel's pipeline output.
    pub subscriber_count: usize,
}

pub struct DaemonState {
    wideband: SampleBus,
    hardware: IngestHandle,
    channelizer: Arc<Mutex<Channelizer>>,
    channelizer_running: Arc<AtomicBool>,
    channelizer_thread: Option<JoinHandle<()>>,
    channels: Mutex<HashMap<ChannelId, ActiveChannel>>,
    next_client_channel: AtomicU32,
    recordings: Mutex<RecordingManager>,
    started_at: Instant,
    receiver_position: Mutex<Option<(f64, f64)>>,
    authenticator: Mutex<Authenticator>,
}

impl DaemonState {
    #[must_use]
    pub fn new(
        wideband: SampleBus,
        hardware: IngestHandle,
        wideband_center_hz: u64,
        wideband_rate_hz: u32,
        recording_dir: impl Into<PathBuf>,
    ) -> Self {
        let wideband_handle =
            wideband.subscribe(WIDEBAND_SUBSCRIBE_CAPACITY, OverflowPolicy::DropOldest);
        let channelizer = Arc::new(Mutex::new(Channelizer::new(
            wideband_handle,
            wideband_center_hz,
            wideband_rate_hz,
        )));
        let (channelizer_running, channelizer_thread) =
            spawn_locked_tick_thread("channelizer", Arc::clone(&channelizer), Channelizer::tick);

        Self {
            wideband,
            hardware,
            channelizer,
            channelizer_running,
            channelizer_thread: Some(channelizer_thread),
            channels: Mutex::new(HashMap::new()),
            next_client_channel: AtomicU32::new(ez_proto::CLIENT_CHANNEL_START),
            recordings: Mutex::new(RecordingManager::new(recording_dir)),
            started_at: Instant::now(),
            receiver_position: Mutex::new(None),
            authenticator: Mutex::new(Authenticator::default()),
        }
    }

    /// Installs the daemon-wide transport authenticator before listeners start accepting
    /// clients. An empty token is rejected by [`Authenticator::new`].
    pub fn set_auth_token(&self, token: Option<String>) -> anyhow::Result<()> {
        *self.authenticator.lock() = Authenticator::new(token)?;
        Ok(())
    }

    #[must_use]
    pub fn authentication_required(&self) -> bool {
        self.authenticator.lock().is_required()
    }

    #[must_use]
    pub fn authenticate(&self, token: &str) -> bool {
        self.authenticator.lock().accepts(token)
    }

    pub fn allocate_client_channels(&self) -> Result<ez_proto::ClientChannels> {
        loop {
            let base = self
                .next_client_channel
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(4))
                .map_err(|_| anyhow!("client channel namespace exhausted"))?;
            let allocated = ez_proto::ClientChannels {
                spectrum: base,
                audio: base + 1,
                adsb: base + 2,
                lrpt: base + 3,
            };
            if allocated.ids().iter().all(|id| !self.channel_exists(*id)) {
                return Ok(allocated);
            }
        }
    }

    pub fn channel_exists(&self, channel_id: ChannelId) -> bool {
        self.channels.lock().contains_key(&channel_id)
    }

    /// Release unused private pipelines when their owning connection closes.
    /// A recording keeps its tap alive until explicitly stopped by its owner/API.
    pub fn release_client_channels(&self, channels: ez_proto::ClientChannels) {
        for id in channels.ids() {
            if self.channel_exists(id) {
                let _ = self.remove_channel(id);
            }
        }
    }

    pub fn set_frequency(&self, hz: u64) -> anyhow::Result<()> {
        self.hardware.set_frequency(hz)?;
        self.channelizer.lock().retune(hz);
        Ok(())
    }

    /// Maximum concurrent virtual channels. Each channel spawns a pipeline OS
    /// thread plus a channelizer tap, so unauthenticated `Subscribe` floods
    /// would otherwise exhaust threads/CPU (each `Subscribe{id}` previously
    /// created unbounded resources).
    pub const MAX_CHANNELS: usize = 32;

    fn resolved_audio(spec: &ChannelSpec) -> AudioParameters {
        spec.audio.unwrap_or_else(|| {
            let mode = spec.demod_mode.unwrap_or(DemodMode::Raw);
            let mut p = AudioParameters::for_mode(mode);
            let fraction = if matches!(mode, DemodMode::Usb | DemodMode::Lsb) {
                0.44
            } else {
                0.8
            };
            p.rf_bandwidth_hz = p
                .rf_bandwidth_hz
                .min((fraction * f64::from(spec.channel_sample_rate_hz)) as u32);
            p.output_sample_rate_hz = 0;
            p
        })
    }

    /// Validates a channel request against the wideband geometry before any
    /// thread or tap is allocated.
    fn validate_spec(&self, spec: &ChannelSpec) -> anyhow::Result<()> {
        if spec.channel_sample_rate_hz == 0 {
            anyhow::bail!("channel {} bandwidth must be non-zero", spec.id);
        }
        if spec.kind == PipelineKind::AdsbPackets && spec.channel_sample_rate_hz != 2_400_000 {
            anyhow::bail!("ADS-B requires exactly 2.4 MHz bandwidth");
        }
        let wideband_rate = self.channelizer.lock().wideband_rate_hz();
        if wideband_rate == 0 {
            anyhow::bail!("wideband rate is zero; cannot create channels");
        }
        if spec.channel_sample_rate_hz > wideband_rate {
            anyhow::bail!(
                "channel {} bandwidth {} exceeds wideband rate {}",
                spec.id,
                spec.channel_sample_rate_hz,
                wideband_rate
            );
        }
        if spec.kind == PipelineKind::Audio {
            let decimation = (wideband_rate / spec.channel_sample_rate_hz).max(1);
            let rate = f64::from(wideband_rate) / f64::from(decimation);
            AudioPipeline::validate_parameters(
                spec.demod_mode.unwrap_or(DemodMode::Raw),
                Self::resolved_audio(spec),
                rate,
            )
            .map_err(anyhow::Error::msg)?;
        } else if spec.audio.is_some() {
            anyhow::bail!("audio parameters are only supported on audio channels");
        }
        if spec.kind == PipelineKind::LrptTelemetry {
            let profile = spec
                .meteor_profile
                .ok_or_else(|| anyhow!("LRPT requires an explicit meteor_profile"))?;
            if spec.channel_sample_rate_hz < profile.occupied_bandwidth_hz() {
                anyhow::bail!(
                    "LRPT {} occupies {} Hz; channel bandwidth {} is too narrow",
                    profile.label(),
                    profile.occupied_bandwidth_hz(),
                    spec.channel_sample_rate_hz
                );
            }
            let decimation = (wideband_rate / spec.channel_sample_rate_hz).max(1);
            lrpt_decode::validate_mission_sampling(wideband_rate / decimation, profile)?;
        } else if spec.meteor_profile.is_some() {
            anyhow::bail!("meteor_profile is only supported on LRPT channels");
        }
        let half = wideband_rate as i64 / 2;
        let offset_abs = spec.center_offset_hz.checked_abs().unwrap_or(i64::MAX);
        if offset_abs.saturating_add(spec.channel_sample_rate_hz as i64 / 2) > half {
            anyhow::bail!(
                "channel {} offset {} and bandwidth {} outside wideband ±{} Hz",
                spec.id,
                spec.center_offset_hz,
                spec.channel_sample_rate_hz,
                half
            );
        }
        Ok(())
    }

    /// Changes the hardware and channelizer capture rate when no narrowband/decoder channel
    /// is active. Wideband spectrum channels may remain because they consume source blocks
    /// directly and use each block's own sample-rate metadata.
    pub fn set_sample_rate(&self, hz: u32) -> anyhow::Result<()> {
        if hz == 0 {
            // A zero rate poisons downstream DSP (division by zero in pacing,
            // FIR sizing, and timestamp math) — reject at the control plane
            // instead of crashing the ingestion thread.
            anyhow::bail!("sample rate must be non-zero");
        }
        if self
            .channels
            .lock()
            .values()
            .any(|active| active.internal_channel_id.is_some())
        {
            anyhow::bail!(
                "cannot change sample rate while audio or decoder channels are active; remove them first"
            );
        }
        self.hardware.set_sample_rate(hz)?;
        self.channelizer.lock().set_wideband_rate_hz(hz)?;
        Ok(())
    }

    pub fn set_gain(&self, db: f64) -> anyhow::Result<()> {
        if !db.is_finite() {
            anyhow::bail!("gain must be finite, got {db}");
        }
        self.hardware.set_gain(db)?;
        Ok(())
    }

    #[must_use]
    pub fn hardware_status(&self) -> HardwareStatus {
        self.hardware.status()
    }

    #[must_use]
    pub fn active_channels(&self) -> Vec<ChannelSpec> {
        self.channels
            .lock()
            .values()
            .map(|c| c.spec.clone())
            .collect()
    }

    /// Seconds since this [`DaemonState`] was constructed, i.e. since the daemon finished
    /// wiring hardware and became ready to accept connections. Used only for reporting
    /// (`crate::web`'s `/api/status`), never for control decisions.
    #[must_use]
    pub fn uptime_sec(&self) -> f64 {
        self.started_at.elapsed().as_secs_f64()
    }

    /// Read-only per-channel snapshot for reporting — see [`ChannelMetrics`]. Unlike
    /// [`Self::subscribe`], this never creates a channel or hands back a live subscription.
    #[must_use]
    pub fn channel_metrics(&self) -> Vec<ChannelMetrics> {
        self.channels
            .lock()
            .values()
            .map(|c| ChannelMetrics {
                spec: c.spec.clone(),
                subscriber_count: match &c.pipeline {
                    ActivePipeline::Spectrum(p) => p.lock().subscriber_count(),
                    ActivePipeline::Audio(p) => p.lock().subscriber_count(),
                    ActivePipeline::AdsbPackets(p) => p.lock().subscriber_count(),
                    ActivePipeline::LrptTelemetry(p) => p.lock().subscriber_count(),
                },
            })
            .collect()
    }

    /// Configure coordinates of the IQ capture site for initial surface CPR.
    pub fn set_receiver_position(&self, position: Option<(f64, f64)>) -> Result<()> {
        if position.is_some_and(|(lat, lon)| {
            !lat.is_finite()
                || !lon.is_finite()
                || !(-90.0..=90.0).contains(&lat)
                || !(-180.0..=180.0).contains(&lon)
        }) {
            return Err(anyhow!(
                "receiver coordinates must be finite latitude/longitude"
            ));
        }
        let channels = self.channels.lock();
        *self.receiver_position.lock() = position;
        for channel in channels.values() {
            if let ActivePipeline::AdsbPackets(pipeline) = &channel.pipeline {
                pipeline.lock().set_receiver_position(position);
            }
        }
        Ok(())
    }

    /// Ensures a channel exists for `spec.id` (creating its pipeline, and a channelizer
    /// virtual tap if its kind needs one, on first use) and returns the caller's own fresh
    /// output subscription to it. A second `Subscribe` for the same id and kind just returns
    /// another subscription to the same running pipeline — see the module doc comment on
    /// why channels are daemon-global rather than per-client.
    pub fn subscribe(&self, mut spec: ChannelSpec) -> Result<(ChannelSubscription, bool)> {
        let id = spec.id;
        // Fast path: channel already exists — just return a fresh output subscription.
        {
            let channels = self.channels.lock();
            if let Some(existing) = channels.get(&id) {
                if existing.spec.kind != spec.kind
                    || existing.spec.meteor_profile != spec.meteor_profile
                {
                    return Err(anyhow!(
                        "channel {id} already exists with a different kind or Meteor profile; cannot resubscribe with these settings. Remove it or use another channel ID"
                    ));
                }
                return Ok((subscription_for(existing), false));
            }
            if channels.len() >= Self::MAX_CHANNELS {
                return Err(anyhow!(
                    "channel limit ({}) reached; remove a channel first",
                    Self::MAX_CHANNELS
                ));
            }
        }
        self.validate_spec(&spec)?;
        if spec.kind == PipelineKind::Audio {
            spec.audio = Some(Self::resolved_audio(&spec));
        }
        // Create the channel *without* holding the channels lock. This avoids a
        // lock-ordering hazard: create_channel → add_virtual_channel blocks on the
        // channelizer Mutex (held during tick's recv_timeout), and holding channels
        // during that wait would starve every other channels-lock consumer (list,
        // retune, remove, …) for the entire tick duration.
        let active = self.create_channel(spec)?;
        let sub = subscription_for(&active);
        let mut channels = self.channels.lock();
        // If another thread raced us and already inserted the same id, tear down
        // the duplicate we just built — its pipeline thread stops within one
        // PIPELINE_POLL tick — AND release its channelizer tap, which would
        // otherwise burn CPU in every channelizer tick forever.
        if let Some(existing) = channels.get(&id) {
            // Concurrent first subscribers must attach to the winner, never
            // replace its live pipeline/recording with the duplicate we built.
            active.running.store(false, Ordering::Relaxed);
            if let Some(internal_id) = active.internal_channel_id {
                self.channelizer.lock().remove_channel(internal_id);
            }
            if existing.spec.kind != active.spec.kind
                || existing.spec.meteor_profile != active.spec.meteor_profile
            {
                return Err(anyhow!(
                    "channel {id} already exists with a different kind or Meteor profile"
                ));
            }
            return Ok((subscription_for(existing), false));
        }
        channels.insert(id, active);
        // Enforce the cap against threads that all passed the pre-check above:
        // roll back our own insert (fail-closed) rather than evicting a
        // stranger's live channel.
        if channels.len() > Self::MAX_CHANNELS {
            if let Some(mine) = channels.remove(&id) {
                mine.running.store(false, Ordering::Relaxed);
                if let Some(internal_id) = mine.internal_channel_id {
                    self.channelizer.lock().remove_channel(internal_id);
                }
            }
            return Err(anyhow!(
                "channel limit ({}) reached; remove a channel first",
                Self::MAX_CHANNELS
            ));
        }
        Ok((sub, true))
    }

    /// Attaches to an already-created channel's output without the power to create one —
    /// unlike [`Self::subscribe`], which will happily create `spec.id` on first use, this
    /// errors if `channel_id` doesn't exist yet. Used by `crate::web`'s data-plane WebSocket
    /// (`/ws/stream/{kind}/{id}`), which only ever knows an id and kind from the URL path,
    /// never the bandwidth/offset/demod-mode a *creating* subscribe would need — channel
    /// creation stays exclusively a REST `POST /api/channels` or TCP
    /// `ClientCommand::Subscribe` responsibility, each of which has a real [`ChannelSpec`] in
    /// hand.
    pub fn subscribe_existing(
        &self,
        channel_id: ChannelId,
        kind: PipelineKind,
    ) -> Result<ChannelSubscription> {
        let channels = self.channels.lock();
        let existing = channels
            .get(&channel_id)
            .ok_or_else(|| anyhow!("unknown channel {channel_id}"))?;
        if existing.spec.kind != kind {
            return Err(anyhow!(
                "channel {channel_id} is {:?}, not {:?}",
                existing.spec.kind,
                kind
            ));
        }
        Ok(subscription_for(existing))
    }

    fn create_channel(&self, spec: ChannelSpec) -> Result<ActiveChannel> {
        match spec.kind {
            PipelineKind::Spectrum => {
                let input = self
                    .wideband
                    .subscribe(WIDEBAND_SUBSCRIBE_CAPACITY, OverflowPolicy::DropOldest);
                let mut spectrum = SpectrumPipeline::new(input, SpectrumConfig::default());
                spectrum.set_channel_id(spec.id);
                let pipeline = Arc::new(Mutex::new(spectrum));
                let (running, _thread) = spawn_locked_tick_thread(
                    "spectrum",
                    Arc::clone(&pipeline),
                    SpectrumPipeline::tick,
                );
                Ok(ActiveChannel {
                    spec,
                    internal_channel_id: None,
                    pipeline: ActivePipeline::Spectrum(pipeline),
                    running,
                })
            }
            PipelineKind::Audio => {
                let (internal_id, input) = self.add_virtual_channel(&spec)?;
                let mode = spec.demod_mode.unwrap_or(DemodMode::Raw);
                let pipeline = Arc::new(Mutex::new(AudioPipeline::new(
                    input,
                    AudioConfig {
                        audio: Some(Self::resolved_audio(&spec)),
                        channel_id: spec.id,
                        mode,
                    },
                )));
                let (running, _thread) =
                    spawn_locked_tick_thread("audio", Arc::clone(&pipeline), AudioPipeline::tick);
                Ok(ActiveChannel {
                    spec,
                    internal_channel_id: Some(internal_id),
                    pipeline: ActivePipeline::Audio(pipeline),
                    running,
                })
            }
            PipelineKind::AdsbPackets => {
                let (internal_id, input) = self.add_virtual_channel(&spec)?;
                let sample_rate_hz = self
                    .channelizer
                    .lock()
                    .channel_output_rate_hz(internal_id)
                    .unwrap_or(spec.channel_sample_rate_hz);
                let mut packet_pipeline = PacketPipeline::new(input, sample_rate_hz);
                packet_pipeline.set_receiver_position(*self.receiver_position.lock());
                let pipeline = Arc::new(Mutex::new(packet_pipeline));
                let (running, _thread) =
                    spawn_locked_tick_thread("adsb", Arc::clone(&pipeline), PacketPipeline::tick);
                Ok(ActiveChannel {
                    spec,
                    internal_channel_id: Some(internal_id),
                    pipeline: ActivePipeline::AdsbPackets(pipeline),
                    running,
                })
            }
            PipelineKind::LrptTelemetry => {
                let (internal_id, input) = self.add_virtual_channel(&spec)?;
                let sample_rate_hz = self
                    .channelizer
                    .lock()
                    .channel_output_rate_hz(internal_id)
                    .unwrap_or(spec.channel_sample_rate_hz);
                let telemetry = TelemetryPipeline::new(
                    input,
                    TelemetryConfig {
                        channel_id: spec.id,
                        sample_rate_hz,
                        profile: spec.meteor_profile.expect("validated LRPT profile"),
                        center_freq_hz: self
                            .channelizer
                            .lock()
                            .channel_center_hz(internal_id)
                            .expect("new channel"),
                    },
                );
                let telemetry = match telemetry {
                    Ok(pipeline) => pipeline,
                    Err(error) => {
                        self.channelizer.lock().remove_channel(internal_id);
                        return Err(error.into());
                    }
                };
                let pipeline = Arc::new(Mutex::new(telemetry));
                let (running, _thread) = spawn_locked_tick_thread(
                    "telemetry",
                    Arc::clone(&pipeline),
                    TelemetryPipeline::tick,
                );
                Ok(ActiveChannel {
                    spec,
                    internal_channel_id: Some(internal_id),
                    pipeline: ActivePipeline::LrptTelemetry(pipeline),
                    running,
                })
            }
        }
    }

    /// Carves a new channelizer virtual channel for `spec`, translating its wideband-
    /// center-relative `center_offset_hz` into the absolute frequency
    /// [`Channelizer::add_channel`] expects.
    fn add_virtual_channel(&self, spec: &ChannelSpec) -> Result<(u64, SampleBusHandle)> {
        let mut chan = self.channelizer.lock();
        let center_freq_hz = ((chan.wideband_center_hz() as i64)
            .saturating_add(spec.center_offset_hz))
        .max(0) as u64;
        chan.add_channel(center_freq_hz, spec.channel_sample_rate_hz)
    }

    pub fn set_demod_mode(&self, channel_id: ChannelId, mode: DemodMode) -> Result<()> {
        self.configure_audio(channel_id, mode, None)
    }

    pub fn configure_audio(
        &self,
        channel_id: ChannelId,
        mode: DemodMode,
        parameters: Option<AudioParameters>,
    ) -> Result<()> {
        let (mut proposed, pipeline) = {
            let channels = self.channels.lock();
            let active = channels
                .get(&channel_id)
                .ok_or_else(|| anyhow!("unknown channel {channel_id}"))?;
            let ActivePipeline::Audio(pipeline) = &active.pipeline else {
                return Err(anyhow!("channel {channel_id} is not audio"));
            };
            (active.spec.clone(), Arc::clone(pipeline))
        };
        let previous = Self::resolved_audio(&proposed);
        proposed.demod_mode = Some(mode);
        proposed.audio = parameters;
        let mut applied = Self::resolved_audio(&proposed);
        if parameters.is_none() {
            applied.output_sample_rate_hz = previous.output_sample_rate_hz;
        }
        proposed.audio = Some(applied);
        self.validate_spec(&proposed)?;
        pipeline.lock().configure(mode, applied);
        let mut channels = self.channels.lock();
        let active = channels
            .get_mut(&channel_id)
            .ok_or_else(|| anyhow!("audio channel removed while reconfiguring"))?;
        let ActivePipeline::Audio(current) = &active.pipeline else {
            return Err(anyhow!("audio channel replaced while reconfiguring"));
        };
        if !Arc::ptr_eq(current, &pipeline) {
            return Err(anyhow!("audio channel replaced while reconfiguring"));
        }
        active.spec = proposed;
        Ok(())
    }

    /// Re-tunes an existing channel in place — changes its center offset and/or bandwidth
    /// without recreating the pipeline. Virtual-channel kinds (Audio/AdsbPackets/
    /// LrptTelemetry) re-center the channelizer DDC while preserving its output bus, so any
    /// live subscriber keeps receiving samples; the `Spectrum` wideband tap has no virtual
    /// channel and just records the new spec.
    pub fn retune(
        &self,
        channel_id: ChannelId,
        center_offset_hz: i64,
        channel_sample_rate_hz: u32,
    ) -> Result<()> {
        self.retune_configured(channel_id, center_offset_hz, channel_sample_rate_hz, None)
    }

    pub fn retune_configured(
        &self,
        channel_id: ChannelId,
        center_offset_hz: i64,
        channel_sample_rate_hz: u32,
        audio: Option<AudioConfiguration>,
    ) -> Result<()> {
        let (internal_id, mut proposed, telemetry, audio_pipeline) = {
            let channels = self.channels.lock();
            let active = channels
                .get(&channel_id)
                .ok_or_else(|| anyhow!("unknown channel {channel_id}"))?;
            let telemetry = match &active.pipeline {
                ActivePipeline::LrptTelemetry(p) => Some(Arc::clone(p)),
                _ => None,
            };
            let audio_pipeline = match &active.pipeline {
                ActivePipeline::Audio(p) => Some(Arc::clone(p)),
                _ => None,
            };
            (
                active.internal_channel_id,
                active.spec.clone(),
                telemetry,
                audio_pipeline,
            )
        };
        proposed.center_offset_hz = center_offset_hz;
        proposed.channel_sample_rate_hz = channel_sample_rate_hz;
        if let Some(configuration) = audio {
            if audio_pipeline.is_none() {
                anyhow::bail!("audio configuration is only supported on audio channels");
            }
            proposed.demod_mode = Some(configuration.mode);
            proposed.audio = Some(configuration.parameters);
        }
        self.validate_spec(&proposed)?;
        if let Some(internal_id) = internal_id {
            let mut chan = self.channelizer.lock();
            let center_freq_hz =
                ((chan.wideband_center_hz() as i64).saturating_add(center_offset_hz)).max(0) as u64;
            chan.retune_channel(internal_id, center_freq_hz, channel_sample_rate_hz)?;
            let rate = chan
                .channel_output_rate_hz(internal_id)
                .expect("retuned channel");
            if let Some(pipeline) = telemetry {
                pipeline.lock().retune(rate, center_freq_hz)?;
            }
        }
        if let Some(pipeline) = audio_pipeline {
            pipeline.lock().configure(
                proposed.demod_mode.unwrap_or(DemodMode::Raw),
                Self::resolved_audio(&proposed),
            );
        }
        let mut channels = self.channels.lock();
        let active = channels
            .get_mut(&channel_id)
            .ok_or_else(|| anyhow!("channel removed while retuning"))?;
        if active.internal_channel_id != internal_id {
            anyhow::bail!("channel replaced while retuning");
        }
        active.spec = proposed;
        Ok(())
    }

    /// Permanently tears down a channel: stops its pipeline tick thread, removes its
    /// channelizer virtual tap (if any), and drops it from the channel table so it no longer
    /// appears in `active_channels()`/`GET /api/channels`. Unlike `Unsubscribe` (which only
    /// detaches one connection's forwarder), this destroys the shared channel for everyone.
    ///
    /// Control transports report removal by channel identity to subscribers.
    /// Active recordings must be stopped first.
    pub fn remove_channel(&self, channel_id: ChannelId) -> Result<()> {
        let mut recordings = self.recordings.lock();
        if recordings
            .status(channel_id)
            .is_some_and(|status| status.active)
        {
            return Err(anyhow!(
                "channel {channel_id} is recording; stop recording before removal"
            ));
        }
        let active = {
            let mut channels = self.channels.lock();
            channels
                .remove(&channel_id)
                .ok_or_else(|| anyhow!("unknown channel {channel_id}"))?
        };
        recordings.forget_finished(channel_id);
        active.running.store(false, Ordering::Relaxed);
        if let Some(internal_id) = active.internal_channel_id {
            self.channelizer.lock().remove_channel(internal_id);
        }
        Ok(())
    }

    pub fn start_telemetry_product(&self, channel_id: ChannelId) -> Result<()> {
        let pipeline = {
            let channels = self.channels.lock();
            let active = channels
                .get(&channel_id)
                .ok_or_else(|| anyhow!("unknown channel {channel_id}"))?;
            match &active.pipeline {
                ActivePipeline::LrptTelemetry(p) => Arc::clone(p),
                _ => anyhow::bail!("channel {channel_id} is not an LRPT channel"),
            }
        };
        pipeline.lock().start_new_product()?;
        Ok(())
    }

    /// Finalize the current decoder product without removing the shared pipeline. The live
    /// subscriber must remain attached until it receives the product-complete marker.
    pub fn finalize_telemetry_product(&self, channel_id: ChannelId) -> Result<()> {
        let pipeline = {
            let channels = self.channels.lock();
            let active = channels
                .get(&channel_id)
                .ok_or_else(|| anyhow!("unknown channel {channel_id}"))?;
            match &active.pipeline {
                ActivePipeline::LrptTelemetry(p) => Arc::clone(p),
                _ => anyhow::bail!("channel {channel_id} is not an LRPT channel"),
            }
        };
        pipeline.lock().finalize_product();
        Ok(())
    }

    pub fn set_volume(&self, channel_id: ChannelId, level: f32) -> Result<()> {
        self.with_audio_pipeline(channel_id, |p| p.set_volume(level))
    }

    pub fn set_squelch(&self, channel_id: ChannelId, db: f32) -> Result<ez_proto::SquelchState> {
        if !db.is_finite() || !(-240.0..=60.0).contains(&db) {
            anyhow::bail!("squelch must be finite and between -240 and 60 dBFS");
        }
        self.with_audio_pipeline(channel_id, |pipeline| {
            pipeline.set_squelch(db);
            pipeline.squelch_state()
        })
    }

    pub fn begin_spectrum_observation(&self, channel_id: ChannelId, revision: u64) -> Result<()> {
        let pipeline = {
            let channels = self.channels.lock();
            let active = channels
                .get(&channel_id)
                .ok_or_else(|| anyhow!("unknown channel {channel_id}"))?;
            match &active.pipeline {
                ActivePipeline::Spectrum(p) => Arc::clone(p),
                _ => return Err(anyhow!("channel {channel_id} is not a spectrum channel")),
            }
        };
        pipeline.lock().begin_observation(revision);
        Ok(())
    }

    fn with_audio_pipeline<T>(
        &self,
        channel_id: ChannelId,
        f: impl FnOnce(&mut AudioPipeline) -> T,
    ) -> Result<T> {
        let pipeline = {
            let channels = self.channels.lock();
            let active = channels
                .get(&channel_id)
                .ok_or_else(|| anyhow!("unknown channel {channel_id}"))?;
            match &active.pipeline {
                ActivePipeline::Audio(p) => Arc::clone(p),
                _ => return Err(anyhow!("channel {channel_id} is not an audio channel")),
            }
        };
        let result = f(&mut pipeline.lock());
        Ok(result)
    }

    /// Starts recording `channel_id`'s sample stream. Works for any channel kind, including
    /// `Spectrum` (which has no `internal_channel_id` of its own to
    /// subscribe to, so it records the raw wideband IQ it's itself fed from instead).
    pub fn start_recording(
        &self,
        channel_id: ChannelId,
        format: RecordingFormat,
    ) -> Result<RecordingStatus> {
        // Serialize creation with deletion so a removed tap cannot acquire a
        // recording between the active-recording check and channel teardown.
        let mut recordings = self.recordings.lock();
        let (internal_id, center_offset_hz) = {
            let channels = self.channels.lock();
            let active = channels
                .get(&channel_id)
                .ok_or_else(|| anyhow!("unknown channel {channel_id}"))?;
            (active.internal_channel_id, active.spec.center_offset_hz)
        };
        let (input, center_freq_hz) = match internal_id {
            Some(internal_id) => {
                let chan = self.channelizer.lock();
                let input = chan
                    .subscribe(internal_id, RECORDING_SUBSCRIBE_CAPACITY)
                    .ok_or_else(|| anyhow!("channel {channel_id} is no longer live"))?;
                let center_freq_hz = ((chan.wideband_center_hz() as i64)
                    .saturating_add(center_offset_hz))
                .max(0) as u64;
                (input, center_freq_hz)
            }
            None => (
                self.wideband
                    .subscribe(RECORDING_SUBSCRIBE_CAPACITY, OverflowPolicy::DropOldest),
                self.channelizer.lock().wideband_center_hz(),
            ),
        };

        recordings.start_recording(channel_id, format, center_freq_hz, input)?;
        recordings.status(channel_id).ok_or_else(|| {
            anyhow!("internal error: recording status missing immediately after start")
        })
    }

    pub fn stop_recording(&self, channel_id: ChannelId) -> Result<RecordingStatus> {
        self.recordings.lock().stop_recording(channel_id)
    }

    #[must_use]
    pub fn recording_status(&self, channel_id: ChannelId) -> Option<RecordingStatus> {
        self.recordings.lock().status(channel_id)
    }

    #[must_use]
    pub fn recording_statuses(&self) -> Vec<RecordingStatus> {
        self.recordings.lock().statuses()
    }
}

impl Drop for DaemonState {
    fn drop(&mut self) {
        self.channelizer_running.store(false, Ordering::Relaxed);
        if let Some(t) = self.channelizer_thread.take() {
            let _ = t.join();
        }
        // Per-channel pipeline threads are intentionally not joined here: they're
        // Arc<Mutex<_>>-shared with any still-running subscription forwarders in
        // `crate::server` and will exit within one `PIPELINE_POLL` tick of `running`
        // clearing below, mirroring the same fire-and-forget shutdown already used for
        // per-connection forwarder threads — joining would block this `Drop` on OS-thread
        // teardown for no benefit, since these threads hold no resource needing
        // synchronous cleanup (no file handles, nothing).
        let channels = self.channels.lock();
        for active in channels.values() {
            active.running.store(false, Ordering::Relaxed);
        }
    }
}

fn subscription_for(active: &ActiveChannel) -> ChannelSubscription {
    match &active.pipeline {
        ActivePipeline::Spectrum(p) => {
            ChannelSubscription::Spectrum(p.lock().subscribe(DEFAULT_SUBSCRIBE_CAPACITY))
        }
        ActivePipeline::Audio(p) => {
            ChannelSubscription::Audio(p.lock().subscribe(DEFAULT_SUBSCRIBE_CAPACITY))
        }
        ActivePipeline::AdsbPackets(p) => {
            ChannelSubscription::AdsbPackets(p.lock().subscribe(DEFAULT_SUBSCRIBE_CAPACITY))
        }
        ActivePipeline::LrptTelemetry(p) => {
            ChannelSubscription::LrptTelemetry(p.lock().subscribe(DEFAULT_SUBSCRIBE_CAPACITY))
        }
    }
}

/// The fair mutex hands the next tick lock to queued control/subscription waiters,
/// avoiding repeated reacquisition starvation during blocking sample receives.
/// Spawns an OS thread that locks `state`, calls `tick(poll_timeout)` on it, unlocks, and
/// repeats until `running` clears. This is the one seam every daemon-owned, continuously-
/// ticked type (`Channelizer`, every pipeline) shares — a thread-spawning utility over a
/// closure, not a shared trait or generic pipeline framework: each type's own `tick` still
/// fully owns its control flow, this just avoids pasting the same eight-line thread-spawn
/// loop once per type.
fn spawn_locked_tick_thread<P, F>(
    name: &'static str,
    state: Arc<Mutex<P>>,
    mut tick: F,
) -> (Arc<AtomicBool>, JoinHandle<()>)
where
    P: Send + 'static,
    F: FnMut(&mut P, Duration) -> bool + Send + 'static,
{
    let running = Arc::new(AtomicBool::new(true));
    let thread_running = Arc::clone(&running);
    let thread = std::thread::Builder::new()
        .name(format!("ez-daemon-{name}"))
        .spawn(move || {
            while thread_running.load(Ordering::Relaxed) {
                let mut guard = state.lock();
                tick(&mut guard, PIPELINE_POLL);
                drop(guard);
                // FIFO lock handoff lets control/subscription waiters run before
                // this worker re-enters a blocking receive on its next tick.
            }
        })
        .expect("spawning pipeline thread");
    (running, thread)
}

#[cfg(test)]
mod tests {
    include!("audio_configuration_tests.rs");
    use super::*;
    use crate::bus::SampleBus;
    use crate::ingest;
    use std::sync::atomic::AtomicBool as StdAtomicBool;

    fn test_state() -> DaemonState {
        let bus = SampleBus::new();
        let running = Arc::new(StdAtomicBool::new(true));
        let (hardware, _thread) = ingest::spawn(
            Box::new(crate::hardware::synthetic::SyntheticSource::default()),
            bus.clone(),
            running,
        )
        .unwrap();
        let dir =
            std::env::temp_dir().join(format!("ez-daemon-state-tests-{}", std::process::id()));
        DaemonState::new(bus, hardware, 100_000_000, 2_000_000, dir)
    }

    fn audio_spec(id: ChannelId, offset_hz: i64) -> ChannelSpec {
        ChannelSpec {
            audio: None,
            meteor_profile: None,
            id,
            center_offset_hz: offset_hz,
            channel_sample_rate_hz: 200_000,
            kind: PipelineKind::Audio,
            demod_mode: Some(DemodMode::Fm),
        }
    }

    #[test]
    fn lrpt_profiles_and_actual_channel_rates_are_validated_before_allocation() {
        let state = test_state();
        let mut spec = ChannelSpec {
            audio: None,
            meteor_profile: None,
            id: 15,
            center_offset_hz: 0,
            channel_sample_rate_hz: 400_000,
            kind: PipelineKind::LrptTelemetry,
            demod_mode: None,
        };
        assert!(state
            .subscribe(spec.clone())
            .err()
            .unwrap()
            .to_string()
            .contains("explicit meteor_profile"));
        for profile in ez_proto::MeteorProfile::ALL {
            spec.meteor_profile = Some(profile);
            for bandwidth in [100_000, 250_000, 285_000] {
                spec.channel_sample_rate_hz = bandwidth;
                assert!(state.subscribe(spec.clone()).is_err());
                assert!(state.active_channels().is_empty());
                assert_eq!(state.channelizer.lock().channel_count(), 0);
            }
            spec.channel_sample_rate_hz = 400_000;
            let (_, created) = state.subscribe(spec.clone()).unwrap();
            assert!(created);
            assert_eq!(state.active_channels()[0].meteor_profile, Some(profile));
            // Invalid retunes leave the existing channel/product untouched.
            assert!(state.retune(15, 0, 100_000).is_err());
            assert_eq!(state.active_channels()[0], spec);
            state.start_telemetry_product(15).unwrap();
            state.remove_channel(15).unwrap();
        }
        spec.kind = PipelineKind::Audio;
        assert!(state.subscribe(spec).is_err());
        assert_eq!(state.channelizer.lock().channel_count(), 0);
    }

    #[test]
    fn resubscribing_an_lrpt_channel_cannot_silently_change_its_transport() {
        let state = test_state();
        let spec = ChannelSpec {
            audio: None,
            meteor_profile: Some(ez_proto::MeteorProfile::Oqpsk72),
            id: 15,
            center_offset_hz: 0,
            channel_sample_rate_hz: 400_000,
            kind: PipelineKind::LrptTelemetry,
            demod_mode: None,
        };
        let (_, _) = state.subscribe(spec.clone()).unwrap();
        let mut other = spec.clone();
        other.meteor_profile = Some(ez_proto::MeteorProfile::Oqpsk80Interleaved);
        assert!(state.subscribe(other).is_err());
        assert_eq!(state.active_channels(), vec![spec]);
        assert_eq!(state.channelizer.lock().channel_count(), 1);
    }

    #[test]
    fn concurrent_first_subscribers_share_one_pipeline_and_one_virtual_tap() {
        let state = Arc::new(test_state());
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let state = Arc::clone(&state);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    state.subscribe(audio_spec(73, 0)).unwrap()
                })
            })
            .collect();
        let subscriptions: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
        assert_eq!(
            subscriptions.iter().filter(|(_, created)| *created).count(),
            1
        );
        assert_eq!(state.active_channels().len(), 1);
        assert_eq!(state.channelizer.lock().channel_count(), 1);
        assert_eq!(state.channel_metrics()[0].subscriber_count, 8);
        for (subscription, _) in subscriptions {
            let ChannelSubscription::Audio(audio) = subscription else {
                panic!("wrong kind")
            };
            let frame = audio.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(frame.channel_id, 73);
            assert!(!frame.samples.is_empty());
        }
    }

    #[test]
    fn client_channel_allocations_are_unique_skip_collisions_and_never_wrap() {
        let state = Arc::new(test_state());
        state
            .subscribe(audio_spec(ez_proto::CLIENT_CHANNEL_START, 0))
            .unwrap();
        let workers: Vec<_> = (0..16)
            .map(|_| {
                let state = Arc::clone(&state);
                std::thread::spawn(move || state.allocate_client_channels().unwrap())
            })
            .collect();
        let ids: Vec<_> = workers
            .into_iter()
            .flat_map(|w| w.join().unwrap().ids())
            .collect();
        assert_eq!(
            ids.iter()
                .copied()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            64
        );
        assert!(ids
            .iter()
            .all(|id| *id >= ez_proto::CLIENT_CHANNEL_START + 4));
        state
            .next_client_channel
            .store(u32::MAX - 3, Ordering::Relaxed);
        assert!(state.allocate_client_channels().is_err());
        assert_eq!(
            state.next_client_channel.load(Ordering::Relaxed),
            u32::MAX - 3
        );
    }

    #[test]
    fn owner_disconnect_preserves_recording_and_releases_other_private_channels() {
        let state = test_state();
        let ids = state.allocate_client_channels().unwrap();
        state.subscribe(audio_spec(ids.audio, 0)).unwrap();
        state
            .subscribe(ChannelSpec {
                audio: None,
                meteor_profile: None,
                id: ids.spectrum,
                kind: PipelineKind::Spectrum,
                channel_sample_rate_hz: 2_000_000,
                center_offset_hz: 0,
                demod_mode: None,
            })
            .unwrap();
        let status = state
            .start_recording(ids.audio, RecordingFormat::Cf32)
            .unwrap();
        assert!(state
            .remove_channel(ids.audio)
            .unwrap_err()
            .to_string()
            .contains("recording"));
        state.release_client_channels(ids);
        assert!(state.channel_exists(ids.audio));
        assert!(!state.channel_exists(ids.spectrum));
        let deadline = Instant::now() + Duration::from_secs(2);
        while state.recording_status(ids.audio).unwrap().bytes_written == 0 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        let completed = state.stop_recording(ids.audio).unwrap();
        assert!(!completed.active);
        assert!(completed.bytes_written > 0);
        assert_eq!(
            std::fs::metadata(status.path.unwrap()).unwrap().len(),
            completed.bytes_written
        );
        state.release_client_channels(ids);
        assert!(state.active_channels().is_empty());
    }

    #[test]
    fn subscribe_creates_a_channel_and_returns_a_working_subscription() {
        let state = test_state();
        let (sub, created) = state
            .subscribe(audio_spec(1, 0))
            .expect("subscribe should succeed");
        assert!(created);
        assert!(matches!(sub, ChannelSubscription::Audio(_)));
        assert_eq!(state.active_channels().len(), 1);
    }

    #[test]
    fn subscribing_the_same_id_twice_reuses_the_existing_pipeline() {
        let state = test_state();
        let (_a, c1) = state.subscribe(audio_spec(1, 0)).unwrap();
        let (_b, c2) = state.subscribe(audio_spec(1, 0)).unwrap();
        assert!(c1);
        assert!(!c2);
        assert_eq!(
            state.active_channels().len(),
            1,
            "second subscribe must not duplicate the channel"
        );
    }

    #[test]
    fn subscribing_same_id_with_a_different_kind_is_an_error() {
        let state = test_state();
        let _a = state.subscribe(audio_spec(1, 0)).unwrap();
        let spectrum_spec = ChannelSpec {
            audio: None,
            meteor_profile: None,
            id: 1,
            center_offset_hz: 0,
            channel_sample_rate_hz: 2_000_000,
            kind: PipelineKind::Spectrum,
            demod_mode: None,
        };
        let Err(err) = state.subscribe(spectrum_spec) else {
            panic!("expected kind-mismatch subscribe to fail");
        };
        assert!(err.to_string().contains("cannot resubscribe"));
    }

    #[test]
    fn spectrum_channel_has_no_internal_virtual_channel() {
        let state = test_state();
        let spec = ChannelSpec {
            audio: None,
            meteor_profile: None,
            id: 5,
            center_offset_hz: 0,
            channel_sample_rate_hz: 2_000_000,
            kind: PipelineKind::Spectrum,
            demod_mode: None,
        };
        let (sub, _) = state.subscribe(spec).unwrap();
        assert!(matches!(sub, ChannelSubscription::Spectrum(_)));
    }

    #[test]
    fn set_volume_on_unknown_channel_is_an_error() {
        let state = test_state();
        let err = state.set_volume(42, 1.0).unwrap_err();
        assert!(err.to_string().contains("unknown channel"));
    }

    #[test]
    fn set_volume_on_non_audio_channel_is_an_error() {
        let state = test_state();
        let spec = ChannelSpec {
            audio: None,
            meteor_profile: None,
            id: 9,
            center_offset_hz: 0,
            channel_sample_rate_hz: 2_000_000,
            kind: PipelineKind::Spectrum,
            demod_mode: None,
        };
        let _sub = state.subscribe(spec).unwrap();
        let err = state.set_volume(9, 1.0).unwrap_err();
        assert!(err.to_string().contains("not an audio channel"));
    }

    #[test]
    fn set_volume_and_squelch_on_an_audio_channel_succeed() {
        let state = test_state();
        let _sub = state.subscribe(audio_spec(1, 0)).unwrap();
        assert!(state.set_volume(1, 2.0).is_ok());
        assert!(state.set_squelch(1, -40.0).is_ok());
        assert!(state.set_demod_mode(1, DemodMode::Am).is_ok());
    }

    #[test]
    fn start_and_stop_recording_on_an_audio_channel() {
        let state = test_state();
        let _sub = state.subscribe(audio_spec(2, 0)).unwrap();
        let status = state
            .start_recording(2, RecordingFormat::Cf32)
            .expect("recording should start");
        assert!(status.active);
        let stopped = state.stop_recording(2).expect("recording should stop");
        assert!(!stopped.active);
    }

    #[test]
    fn start_recording_on_unknown_channel_is_an_error() {
        let state = test_state();
        let err = state
            .start_recording(123, RecordingFormat::Cf32)
            .unwrap_err();
        assert!(err.to_string().contains("unknown channel"));
    }

    #[test]
    fn set_frequency_retunes_the_channelizer() {
        let state = test_state();
        state.set_frequency(105_000_000).unwrap();
        assert_eq!(state.channelizer.lock().wideband_center_hz(), 105_000_000);
    }

    #[test]
    fn subscribe_rejects_invalid_specs_before_allocating() {
        let state = test_state();
        // Zero bandwidth.
        let mut bad = audio_spec(1, 0);
        bad.channel_sample_rate_hz = 0;
        assert!(state.subscribe(bad).is_err());
        // Bandwidth wider than the 2 MHz wideband.
        let mut bad = audio_spec(2, 0);
        bad.channel_sample_rate_hz = 4_000_000;
        assert!(state.subscribe(bad).is_err());
        // Offset outside ±1 MHz.
        let bad = audio_spec(3, 1_500_000);
        assert!(state.subscribe(bad).is_err());
        // Nothing was allocated for the rejected requests.
        assert_eq!(state.active_channels().len(), 0);
    }

    #[test]
    fn subscribe_enforces_channel_cap() {
        let state = test_state();
        for id in 0..DaemonState::MAX_CHANNELS {
            if state.subscribe(audio_spec(id as u32, 0)).is_err() {
                panic!("subscribe within cap must succeed");
            }
        }
        assert!(state
            .subscribe(audio_spec(DaemonState::MAX_CHANNELS as u32, 0))
            .is_err());
        assert_eq!(state.active_channels().len(), DaemonState::MAX_CHANNELS);
    }

    #[test]
    fn set_sample_rate_rejects_zero() {
        let state = test_state();
        assert!(state.set_sample_rate(0).is_err());
        assert!(state.set_sample_rate(2_000_000).is_ok());
    }

    #[test]
    fn set_sample_rate_updates_channelizer_with_only_spectrum_active() {
        let state = test_state();
        let spectrum = ChannelSpec {
            audio: None,
            meteor_profile: None,
            id: 5,
            center_offset_hz: 0,
            channel_sample_rate_hz: 2_000_000,
            kind: PipelineKind::Spectrum,
            demod_mode: None,
        };
        state.subscribe(spectrum).unwrap();

        state.set_sample_rate(2_400_000).unwrap();

        assert_eq!(state.channelizer.lock().wideband_rate_hz(), 2_400_000);
        let adsb = ChannelSpec {
            audio: None,
            meteor_profile: None,
            id: 6,
            center_offset_hz: 0,
            channel_sample_rate_hz: 2_400_000,
            kind: PipelineKind::AdsbPackets,
            demod_mode: None,
        };
        assert!(state.subscribe(adsb).is_ok());
    }

    #[test]
    fn set_sample_rate_rejects_active_virtual_channels() {
        let state = test_state();
        state.subscribe(audio_spec(1, 0)).unwrap();
        let err = state.set_sample_rate(2_400_000).unwrap_err();
        assert!(err.to_string().contains("audio or decoder channels"));
        assert_eq!(state.channelizer.lock().wideband_rate_hz(), 2_000_000);
    }

    #[test]
    fn set_gain_rejects_non_finite() {
        let state = test_state();
        assert!(state.set_gain(f64::NAN).is_err());
        assert!(state.set_gain(f64::INFINITY).is_err());
        assert!(state.set_gain(40.0).is_ok());
    }

    #[test]
    fn set_demod_mode_updates_the_channel_spec_mirror() {
        let state = test_state();
        let _sub = state.subscribe(audio_spec(1, 0)).unwrap();
        state.set_demod_mode(1, DemodMode::Am).unwrap();
        let spec = state
            .active_channels()
            .into_iter()
            .find(|s| s.id == 1)
            .expect("channel present");
        assert_eq!(spec.demod_mode, Some(DemodMode::Am));
    }

    #[test]
    fn retune_moves_an_audio_channel_center_and_bandwidth() {
        let state = test_state();
        let _sub = state.subscribe(audio_spec(1, 0)).unwrap();
        state.retune(1, 500_000, 100_000).unwrap();
        let spec = state
            .active_channels()
            .into_iter()
            .find(|s| s.id == 1)
            .expect("channel present");
        assert_eq!(spec.center_offset_hz, 500_000);
        assert_eq!(spec.channel_sample_rate_hz, 100_000);
    }

    #[test]
    fn retune_of_unknown_channel_is_an_error() {
        let state = test_state();
        assert!(state.retune(123, 0, 200_000).is_err());
    }

    #[test]
    fn remove_channel_drops_it_from_the_active_set() {
        let state = test_state();
        let _sub = state.subscribe(audio_spec(3, 0)).unwrap();
        assert_eq!(state.active_channels().len(), 1);
        state.remove_channel(3).unwrap();
        assert_eq!(state.active_channels().len(), 0);
    }

    #[test]
    fn remove_channel_of_unknown_channel_is_an_error() {
        let state = test_state();
        assert!(state.remove_channel(321).is_err());
    }

    #[test]
    fn hardware_status_reports_connected_synthetic_source() {
        let state = test_state();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            let status = state.hardware_status();
            if status.connected && status.source_kind == "synthetic" {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "hardware never reported connected status"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn validate_spec_handles_i64_min_offset_without_panic() {
        let state = test_state();
        let mut spec = audio_spec(42, 0);
        spec.center_offset_hz = i64::MIN;
        let res = state.subscribe(spec);
        assert!(res.is_err(), "i64::MIN offset must be rejected cleanly");

        let mut spec_max = audio_spec(43, 0);
        spec_max.center_offset_hz = i64::MAX;
        let res_max = state.subscribe(spec_max);
        assert!(res_max.is_err(), "i64::MAX offset must be rejected cleanly");
    }

    #[test]
    fn state_locks_release_after_panics() {
        let state = test_state();
        // Panic while holding the channelizer lock
        let channelizer = Arc::clone(&state.channelizer);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = channelizer.lock();
            panic!("poisoning channelizer mutex intentionally");
        }));
        // Fair mutexes release normally on panic; state access remains usable
        assert!(state.set_frequency(105_000_000).is_ok());

        // Panic while holding the channels lock
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = state.channels.lock();
            panic!("poisoning channels mutex intentionally");
        }));
        // State accessors must still work without crashing
        assert_eq!(state.active_channels().len(), 0);
        assert_eq!(state.channel_metrics().len(), 0);
    }
}
