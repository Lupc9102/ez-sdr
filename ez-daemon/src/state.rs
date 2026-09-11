//! Central daemon state: owns the wideband hardware ingestion handle, the channelizer,
//! every currently-active per-channel pipeline, and the recording manager. Every connection
//! task (see `crate::server`) talks to the daemon exclusively through this type's methods —
//! it is the one place command routing and pipeline lifecycle meet.
//!
//! Channels are daemon-global, multi-tenant resources, not per-client: [`DaemonState::subscribe`]
//! on a [`ChannelId`] that already exists just hands the caller a fresh output subscription
//! to the already-running pipeline (exactly the "attach without disrupting" requirement),
//! rather than erroring or creating a duplicate. `Unsubscribe` is purely connection-local
//! (handled entirely in `crate::server` by stopping that one client's forwarder) — it never
//! tears down the shared channel/pipeline, since another client or an active recording may
//! still depend on it. There is deliberately no wire-protocol verb for permanent channel
//! teardown yet; channels live for the daemon's process lifetime once created.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use ez_proto::{
    AircraftTelemetry, AudioFrame, ChannelId, ChannelSpec, DemodMode, HardwareStatus, PipelineKind,
    RecordingFormat, RecordingStatus, SpectrumFrame, TelemetryFrame,
};

use crate::broadcast::BroadcasterHandle;
use crate::bus::{OverflowPolicy, SampleBus, SampleBusHandle};
use crate::channelizer::Channelizer;
use crate::ingest::IngestHandle;
use crate::pipelines::audio::{AudioConfig, AudioPipeline};
use crate::pipelines::packet::PacketPipeline;
use crate::pipelines::spectrum::{SpectrumConfig, SpectrumPipeline};
use crate::pipelines::telemetry::{TelemetryConfig, TelemetryPipeline};
use crate::recording::RecordingManager;

/// LRPT's standard symbol rate. Fixed by the format, independent of whatever bandwidth a
/// client happens to request for the channel (see [`DaemonState::create_channel`]).
const LRPT_SYMBOL_RATE_HZ: u32 = 72_000;
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
    AdsbPackets(BroadcasterHandle<Vec<AircraftTelemetry>>),
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
    recordings: Mutex<RecordingManager>,
    started_at: Instant,
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
            recordings: Mutex::new(RecordingManager::new(recording_dir)),
            started_at: Instant::now(),
        }
    }

    pub fn set_frequency(&self, hz: u64) {
        self.hardware.set_frequency(hz);
        self.channelizer.lock().unwrap().retune(hz);
    }

    /// Maximum concurrent virtual channels. Each channel spawns a pipeline OS
    /// thread plus a channelizer tap, so unauthenticated `Subscribe` floods
    /// would otherwise exhaust threads/CPU (each `Subscribe{id}` previously
    /// created unbounded resources).
    pub const MAX_CHANNELS: usize = 32;

    /// Validates a channel request against the wideband geometry before any
    /// thread or tap is allocated.
    fn validate_spec(&self, spec: &ChannelSpec) -> anyhow::Result<()> {
        if spec.bandwidth_hz == 0 {
            anyhow::bail!("channel {} bandwidth must be non-zero", spec.id);
        }
        let wideband_rate = self.channelizer.lock().unwrap().wideband_rate_hz();
        if wideband_rate == 0 {
            anyhow::bail!("wideband rate is zero; cannot create channels");
        }
        if spec.bandwidth_hz > wideband_rate {
            anyhow::bail!(
                "channel {} bandwidth {} exceeds wideband rate {}",
                spec.id,
                spec.bandwidth_hz,
                wideband_rate
            );
        }
        let half = wideband_rate as i64 / 2;
        if spec.center_offset_hz.abs() > half {
            anyhow::bail!(
                "channel {} offset {} outside wideband ±{} Hz",
                spec.id,
                spec.center_offset_hz,
                half
            );
        }
        Ok(())
    }

    /// Forwards to the hardware source. Does **not** rebuild already-active virtual
    /// channels' decimators for the new rate (unlike [`Self::set_frequency`], which drives
    /// [`Channelizer::retune`]) — every virtual channel's FIR/decimation factor is sized for
    /// the wideband rate at the moment it was added, and live-migrating that is a materially
    /// bigger change than retuning (which only touches the NCO mix). Channels added *after*
    /// this call see the new rate correctly; channels active from before it keep filtering
    /// as if the rate hadn't changed until removed and re-added. No current caller (GUI or
    /// test) exercises a live sample-rate change with active channels, so this is left as a
    /// documented limitation rather than guessed at.
    pub fn set_sample_rate(&self, hz: u32) -> anyhow::Result<()> {
        if hz == 0 {
            // A zero rate poisons downstream DSP (division by zero in pacing,
            // FIR sizing, and timestamp math) — reject at the control plane
            // instead of crashing the ingestion thread.
            anyhow::bail!("sample rate must be non-zero");
        }
        self.hardware.set_sample_rate(hz);
        Ok(())
    }

    pub fn set_gain(&self, db: f64) -> anyhow::Result<()> {
        if !db.is_finite() {
            anyhow::bail!("gain must be finite, got {db}");
        }
        self.hardware.set_gain(db);
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
            .unwrap()
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
            .unwrap()
            .values()
            .map(|c| ChannelMetrics {
                spec: c.spec.clone(),
                subscriber_count: match &c.pipeline {
                    ActivePipeline::Spectrum(p) => p.lock().unwrap().subscriber_count(),
                    ActivePipeline::Audio(p) => p.lock().unwrap().subscriber_count(),
                    ActivePipeline::AdsbPackets(p) => p.lock().unwrap().subscriber_count(),
                    ActivePipeline::LrptTelemetry(p) => p.lock().unwrap().subscriber_count(),
                },
            })
            .collect()
    }

    /// Ensures a channel exists for `spec.id` (creating its pipeline, and a channelizer
    /// virtual tap if its kind needs one, on first use) and returns the caller's own fresh
    /// output subscription to it. A second `Subscribe` for the same id and kind just returns
    /// another subscription to the same running pipeline — see the module doc comment on
    /// why channels are daemon-global rather than per-client.
    pub fn subscribe(&self, spec: ChannelSpec) -> Result<ChannelSubscription> {
        let id = spec.id;
        // Fast path: channel already exists — just return a fresh output subscription.
        {
            let channels = self.channels.lock().unwrap();
            if let Some(existing) = channels.get(&id) {
                if existing.spec.kind != spec.kind {
                    return Err(anyhow!(
                        "channel {} already exists as {:?}, cannot resubscribe as {:?}",
                        id,
                        existing.spec.kind,
                        spec.kind
                    ));
                }
                return Ok(subscription_for(existing));
            }
            if channels.len() >= Self::MAX_CHANNELS {
                return Err(anyhow!(
                    "channel limit ({}) reached; remove a channel first",
                    Self::MAX_CHANNELS
                ));
            }
        }
        self.validate_spec(&spec)?;
        // Create the channel *without* holding the channels lock. This avoids a
        // lock-ordering hazard: create_channel → add_virtual_channel blocks on the
        // channelizer Mutex (held during tick's recv_timeout), and holding channels
        // during that wait would starve every other channels-lock consumer (list,
        // retune, remove, …) for the entire tick duration.
        let active = self.create_channel(spec)?;
        let sub = subscription_for(&active);
        let mut channels = self.channels.lock().unwrap();
        // If another thread raced us and already inserted the same id, tear down
        // the duplicate we just built — its pipeline thread stops within one
        // PIPELINE_POLL tick — AND release its channelizer tap, which would
        // otherwise burn CPU in every channelizer tick forever.
        if let Some(old) = channels.insert(id, active) {
            old.running.store(false, Ordering::Relaxed);
            if let Some(internal_id) = old.internal_channel_id {
                self.channelizer.lock().unwrap().remove_channel(internal_id);
            }
        }
        // Enforce the cap against threads that all passed the pre-check above:
        // roll back our own insert (fail-closed) rather than evicting a
        // stranger's live channel.
        if channels.len() > Self::MAX_CHANNELS {
            if let Some(mine) = channels.remove(&id) {
                mine.running.store(false, Ordering::Relaxed);
                if let Some(internal_id) = mine.internal_channel_id {
                    self.channelizer.lock().unwrap().remove_channel(internal_id);
                }
            }
            return Err(anyhow!(
                "channel limit ({}) reached; remove a channel first",
                Self::MAX_CHANNELS
            ));
        }
        Ok(sub)
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
        let channels = self.channels.lock().unwrap();
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
                let pipeline = Arc::new(Mutex::new(SpectrumPipeline::new(
                    input,
                    SpectrumConfig::default(),
                )));
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
                let pipeline = Arc::new(Mutex::new(PacketPipeline::new(input)));
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
                    .unwrap()
                    .channel_output_rate_hz(internal_id)
                    .unwrap_or(spec.bandwidth_hz);
                let pipeline = Arc::new(Mutex::new(TelemetryPipeline::new(
                    input,
                    TelemetryConfig {
                        channel_id: spec.id,
                        sample_rate_hz,
                        symbol_rate_hz: LRPT_SYMBOL_RATE_HZ,
                    },
                )));
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
        let mut chan = self.channelizer.lock().unwrap();
        let center_freq_hz =
            (chan.wideband_center_hz() as i64 + spec.center_offset_hz).max(0) as u64;
        chan.add_channel(center_freq_hz, spec.bandwidth_hz)
    }

    pub fn set_demod_mode(&self, channel_id: ChannelId, mode: DemodMode) -> Result<()> {
        self.with_audio_pipeline(channel_id, |p| p.set_mode(mode))?;
        // Keep the ChannelSpec mirror in sync so `GET /api/channels` (and the control-plane
        // `Welcome`/`active_channels`) reports the mode that's actually running, not the
        // creation-time value. The mode is applied to the live pipeline above; this only
        // fixes the read-side snapshot.
        let mut channels = self.channels.lock().unwrap();
        if let Some(active) = channels.get_mut(&channel_id) {
            active.spec.demod_mode = Some(mode);
        }
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
        bandwidth_hz: u32,
    ) -> Result<()> {
        let internal_id = {
            let channels = self.channels.lock().unwrap();
            let active = channels
                .get(&channel_id)
                .ok_or_else(|| anyhow!("unknown channel {channel_id}"))?;
            active.internal_channel_id
        };
        if let Some(internal_id) = internal_id {
            let mut chan = self.channelizer.lock().unwrap();
            let center_freq_hz =
                (chan.wideband_center_hz() as i64 + center_offset_hz).max(0) as u64;
            chan.retune_channel(internal_id, center_freq_hz, bandwidth_hz)?;
        }
        let mut channels = self.channels.lock().unwrap();
        if let Some(active) = channels.get_mut(&channel_id) {
            active.spec.center_offset_hz = center_offset_hz;
            active.spec.bandwidth_hz = bandwidth_hz;
        }
        Ok(())
    }

    /// Permanently tears down a channel: stops its pipeline tick thread, removes its
    /// channelizer virtual tap (if any), and drops it from the channel table so it no longer
    /// appears in `active_channels()`/`GET /api/channels`. Unlike `Unsubscribe` (which only
    /// detaches one connection's forwarder), this destroys the shared channel for everyone.
    ///
    /// Note: subscribers that already hold a forwarder to the pipeline's broadcaster keep
    /// their handle (it just stops publishing new frames) and are expected to notice the
    /// channel is gone via a subsequent `GET /api/channels` refresh rather than an explicit
    /// disconnect event.
    pub fn remove_channel(&self, channel_id: ChannelId) -> Result<()> {
        let active = {
            let mut channels = self.channels.lock().unwrap();
            channels
                .remove(&channel_id)
                .ok_or_else(|| anyhow!("unknown channel {channel_id}"))?
        };
        active.running.store(false, Ordering::Relaxed);
        if let Some(internal_id) = active.internal_channel_id {
            self.channelizer.lock().unwrap().remove_channel(internal_id);
        }
        Ok(())
    }

    pub fn set_volume(&self, channel_id: ChannelId, level: f32) -> Result<()> {
        self.with_audio_pipeline(channel_id, |p| p.set_volume(level))
    }

    pub fn set_squelch(&self, channel_id: ChannelId, db: f32) -> Result<()> {
        self.with_audio_pipeline(channel_id, |p| p.set_squelch(db))
    }

    fn with_audio_pipeline(
        &self,
        channel_id: ChannelId,
        f: impl FnOnce(&mut AudioPipeline),
    ) -> Result<()> {
        let pipeline = {
            let channels = self.channels.lock().unwrap();
            let active = channels
                .get(&channel_id)
                .ok_or_else(|| anyhow!("unknown channel {channel_id}"))?;
            match &active.pipeline {
                ActivePipeline::Audio(p) => Arc::clone(p),
                _ => return Err(anyhow!("channel {channel_id} is not an audio channel")),
            }
        };
        f(&mut pipeline.lock().unwrap());
        Ok(())
    }

    /// Starts recording `channel_id`'s sample stream. Works for any channel kind, including
    /// `Spectrum` (which has no `internal_channel_id` of its own to
    /// subscribe to, so it records the raw wideband IQ it's itself fed from instead).
    pub fn start_recording(
        &self,
        channel_id: ChannelId,
        format: RecordingFormat,
    ) -> Result<RecordingStatus> {
        let (internal_id, center_offset_hz) = {
            let channels = self.channels.lock().unwrap();
            let active = channels
                .get(&channel_id)
                .ok_or_else(|| anyhow!("unknown channel {channel_id}"))?;
            (active.internal_channel_id, active.spec.center_offset_hz)
        };
        let (input, center_freq_hz) = match internal_id {
            Some(internal_id) => {
                let chan = self.channelizer.lock().unwrap();
                let input = chan
                    .subscribe(internal_id, RECORDING_SUBSCRIBE_CAPACITY)
                    .ok_or_else(|| anyhow!("channel {channel_id} is no longer live"))?;
                let center_freq_hz =
                    (chan.wideband_center_hz() as i64 + center_offset_hz).max(0) as u64;
                (input, center_freq_hz)
            }
            None => (
                self.wideband
                    .subscribe(RECORDING_SUBSCRIBE_CAPACITY, OverflowPolicy::DropOldest),
                self.channelizer.lock().unwrap().wideband_center_hz(),
            ),
        };

        let mut recordings = self.recordings.lock().unwrap();
        recordings.start_recording(channel_id, format, center_freq_hz, input)?;
        recordings.status(channel_id).ok_or_else(|| {
            anyhow!("internal error: recording status missing immediately after start")
        })
    }

    pub fn stop_recording(&self, channel_id: ChannelId) -> Result<RecordingStatus> {
        self.recordings.lock().unwrap().stop_recording(channel_id)
    }

    #[must_use]
    pub fn recording_status(&self, channel_id: ChannelId) -> Option<RecordingStatus> {
        self.recordings.lock().unwrap().status(channel_id)
    }

    #[must_use]
    pub fn recording_statuses(&self) -> Vec<RecordingStatus> {
        self.recordings.lock().unwrap().statuses()
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
        if let Ok(channels) = self.channels.lock() {
            for active in channels.values() {
                active.running.store(false, Ordering::Relaxed);
            }
        }
    }
}

fn subscription_for(active: &ActiveChannel) -> ChannelSubscription {
    match &active.pipeline {
        ActivePipeline::Spectrum(p) => {
            ChannelSubscription::Spectrum(p.lock().unwrap().subscribe(DEFAULT_SUBSCRIBE_CAPACITY))
        }
        ActivePipeline::Audio(p) => {
            ChannelSubscription::Audio(p.lock().unwrap().subscribe(DEFAULT_SUBSCRIBE_CAPACITY))
        }
        ActivePipeline::AdsbPackets(p) => ChannelSubscription::AdsbPackets(
            p.lock().unwrap().subscribe(DEFAULT_SUBSCRIBE_CAPACITY),
        ),
        ActivePipeline::LrptTelemetry(p) => ChannelSubscription::LrptTelemetry(
            p.lock().unwrap().subscribe(DEFAULT_SUBSCRIBE_CAPACITY),
        ),
    }
}

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
                let mut guard = state.lock().unwrap();
                tick(&mut guard, PIPELINE_POLL);
                drop(guard);
                std::thread::yield_now();
            }
        })
        .expect("spawning pipeline thread");
    (running, thread)
}

#[cfg(test)]
mod tests {
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
            id,
            center_offset_hz: offset_hz,
            bandwidth_hz: 200_000,
            kind: PipelineKind::Audio,
            demod_mode: Some(DemodMode::Fm),
        }
    }

    #[test]
    fn subscribe_creates_a_channel_and_returns_a_working_subscription() {
        let state = test_state();
        let sub = state
            .subscribe(audio_spec(1, 0))
            .expect("subscribe should succeed");
        assert!(matches!(sub, ChannelSubscription::Audio(_)));
        assert_eq!(state.active_channels().len(), 1);
    }

    #[test]
    fn subscribing_the_same_id_twice_reuses_the_existing_pipeline() {
        let state = test_state();
        let _a = state.subscribe(audio_spec(1, 0)).unwrap();
        let _b = state.subscribe(audio_spec(1, 0)).unwrap();
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
            id: 1,
            center_offset_hz: 0,
            bandwidth_hz: 2_000_000,
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
            id: 5,
            center_offset_hz: 0,
            bandwidth_hz: 2_000_000,
            kind: PipelineKind::Spectrum,
            demod_mode: None,
        };
        let sub = state.subscribe(spec).unwrap();
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
            id: 9,
            center_offset_hz: 0,
            bandwidth_hz: 2_000_000,
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
        state.set_frequency(105_000_000);
        assert_eq!(
            state.channelizer.lock().unwrap().wideband_center_hz(),
            105_000_000
        );
    }

    #[test]
    fn subscribe_rejects_invalid_specs_before_allocating() {
        let state = test_state();
        // Zero bandwidth.
        let mut bad = audio_spec(1, 0);
        bad.bandwidth_hz = 0;
        assert!(state.subscribe(bad).is_err());
        // Bandwidth wider than the 2 MHz wideband.
        let mut bad = audio_spec(2, 0);
        bad.bandwidth_hz = 4_000_000;
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
        assert_eq!(spec.bandwidth_hz, 100_000);
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
}
