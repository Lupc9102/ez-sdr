//! Message types exchanged between daemon and clients.
//!
//! `ClientCommand` flows client -> daemon (control plane).
//! `ServerEvent` flows daemon -> client (telemetry/data plane), fanned out to every
//! subscriber of the relevant pipeline via an independent broadcast queue, so one slow
//! or disconnecting client never affects another.

use serde::{Deserialize, Serialize};

/// Bumped whenever a breaking wire-format change is made.
pub const PROTOCOL_VERSION: u32 = 12;

#[cfg(test)]
#[path = "audio_configuration_tests.rs"]
mod audio_configuration_tests;

/// Identifies a virtual channel (a narrowband slice of the wideband capture) that a
/// client has asked the daemon to carve out and route to a specific pipeline kind.
pub type ChannelId = u32;

/// High channel IDs are connection-owned. Low IDs remain explicit shared
/// channels for daemon/API users; GUI workflows use the IDs in Welcome.
pub const CLIENT_CHANNEL_START: ChannelId = 0x8000_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientChannels {
    pub spectrum: ChannelId,
    pub audio: ChannelId,
    pub adsb: ChannelId,
    pub lrpt: ChannelId,
}

impl ClientChannels {
    pub fn ids(self) -> [ChannelId; 4] {
        [self.spectrum, self.audio, self.adsb, self.lrpt]
    }
    pub fn contains(self, id: ChannelId) -> bool {
        self.ids().contains(&id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PipelineKind {
    /// Full-wideband FFT/waterfall view. Does not require a virtual channel.
    Spectrum,
    /// Narrowband demodulated audio for a tuned virtual channel.
    Audio,
    /// Event-driven ADS-B/Mode-S packet decoding (dump1090).
    AdsbPackets,
    /// Heavy multi-stage satellite telemetry frame decoding (lrpt-decode).
    LrptTelemetry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DemodMode {
    Raw,
    Am,
    Fm,
    Wfm,
    Lsb,
    Usb,
}

/// RF selection and PCM delivery are independent of the virtual channel clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioParameters {
    /// Full symmetric protected RF width for AM/FM/WFM; one-sided extent for SSB.
    /// Zero bypasses RF selection only in RAW mode.
    pub rf_bandwidth_hz: u32,
    /// PCM clock. Zero explicitly follows the actual channel clock.
    pub output_sample_rate_hz: u32,
}

impl AudioParameters {
    pub fn for_mode(mode: DemodMode) -> Self {
        Self {
            rf_bandwidth_hz: match mode {
                DemodMode::Raw => 0,
                DemodMode::Am => 15_000,
                DemodMode::Fm => 25_000,
                DemodMode::Wfm => 180_000,
                DemodMode::Lsb | DemodMode::Usb => 3_300,
            },
            output_sample_rate_hz: 48_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioConfiguration {
    pub mode: DemodMode,
    pub parameters: AudioParameters,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecordingFormat {
    /// Interleaved little-endian f32 I/Q, matching the existing `.cf32` recorder format.
    Cf32,
    /// Raw unsigned 8-bit interleaved I/Q, matching the existing `.iq` recorder format.
    RawU8,
}

/// A client-requested virtual channel: a narrowband slice of the wideband capture,
/// digitally down-converted and decimated by the daemon's channelizer, then routed to
/// exactly one pipeline kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelSpec {
    /// Required for LRPT telemetry; absent for other pipeline kinds.
    #[serde(default)]
    pub meteor_profile: Option<crate::MeteorProfile>,
    pub id: ChannelId,
    /// Offset in Hz from the wideband center frequency. Positive = above center.
    pub center_offset_hz: i64,
    /// Requested virtual-channel sample rate. Integer RF decimation can produce
    /// a higher actual rate, carried exactly by the RF capture clock.
    #[serde(alias = "bandwidth_hz")]
    pub channel_sample_rate_hz: u32,
    /// Optional for audio; omission selects mode-specific RF defaults and
    /// explicitly follows the channel clock for PCM. Absent on other kinds.
    #[serde(default)]
    pub audio: Option<AudioParameters>,
    pub kind: PipelineKind,
    pub demod_mode: Option<DemodMode>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClientCommand {
    /// Must be the first message sent on a new connection.
    Hello {
        client_name: String,
        protocol_version: u32,
    },
    SetFrequency {
        hz: u64,
    },
    SetSampleRate {
        hz: u32,
    },
    SetGain {
        db: f64,
    },
    Subscribe {
        channel: ChannelSpec,
    },
    Unsubscribe {
        channel_id: ChannelId,
    },
    SetDemodMode {
        channel_id: ChannelId,
        mode: DemodMode,
        #[serde(default)]
        audio: Option<AudioParameters>,
    },
    SetVolume {
        channel_id: ChannelId,
        level: f32,
    },
    SetSquelch {
        request_id: u64,
        channel_id: ChannelId,
        db: f32,
    },
    /// Re-tunes an already-created channel in place: changes its center offset and/or
    /// bandwidth without tearing down and recreating the pipeline. For virtual-channel
    /// kinds (Audio/AdsbPackets/LrptTelemetry) this re-centers the channelizer DDC while
    /// keeping the same output bus, so any live subscriber keeps receiving samples.
    Retune {
        channel_id: ChannelId,
        center_offset_hz: i64,
        #[serde(alias = "bandwidth_hz")]
        channel_sample_rate_hz: u32,
        /// Changes RF selection, mode and PCM clock atomically with the channel
        /// geometry, so both widening and narrowing mode changes are valid.
        #[serde(default)]
        audio: Option<AudioConfiguration>,
    },
    /// Permanent, daemon-global teardown of a channel created earlier via `Subscribe`.
    /// Unlike `Unsubscribe` (which is connection-local), this removes the channel's
    /// pipeline and, for virtual-channel kinds, its channelizer tap for everyone.
    RemoveChannel {
        channel_id: ChannelId,
    },
    StartRecording {
        channel_id: ChannelId,
        format: RecordingFormat,
    },
    StopRecording {
        channel_id: ChannelId,
    },
    /// Begin a fresh LRPT image product at the current tuning.
    StartTelemetryProduct {
        channel_id: ChannelId,
    },
    /// Retire queued FFT windows and begin a fresh post-settle observation.
    BeginSpectrumObservation {
        channel_id: ChannelId,
        revision: u64,
    },
    Ping {
        nonce: u64,
    },
    /// Explicit clean detach. The daemon also treats a dropped TCP connection as an
    /// implicit detach — this variant exists so a client can signal intent before
    /// closing, distinct from a network failure.
    Detach,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpectrumFrame {
    pub channel_id: ChannelId,
    pub center_hz: u64,
    pub sample_rate_hz: u32,
    /// dB magnitudes, fftshifted to ascending-frequency order (bin 0 = center - fs/2).
    pub bins: Vec<f32>,
    pub timestamp_ms: u64,
    /// Echoes the observation barrier that precedes this complete FFT window.
    pub observation_revision: u64,
    /// Unsmoothed FFT magnitudes for an explicitly requested scan observation.
    /// Ordinary display subscriptions omit this additional measurement payload.
    pub measurement_bins: Option<Vec<f32>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SquelchState {
    pub db: f32,
    /// Monotonic within this audio pipeline; identifies audio already gated
    /// using these settings, independent of control/data delivery order.
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioFrame {
    /// Actual RF input clock and acquisition provenance, retained independently
    /// of the PCM sample rate. A DSP reset also marks this frame discontinuous.
    pub clock: crate::CaptureClock,
    /// PCM stream clock. Its source rate/index describe generated PCM samples;
    /// `clock` above retains the original RF source rate, stride and epoch.
    pub audio_clock: crate::CaptureClock,
    /// Monotonic for this pipeline lifetime; exposes network/consumer omissions.
    pub sequence: u64,
    pub squelch: SquelchState,
    pub channel_id: ChannelId,
    pub sample_rate_hz: u32,
    pub samples: Vec<f32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AircraftTelemetry {
    pub icao: u32,
    pub address_type: String,
    pub source: String,
    pub callsign: Option<String>,
    pub altitude_ft: Option<i32>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub ground_speed_kt: Option<f64>,
    pub track_deg: Option<f64>,
    pub heading_deg: Option<f64>,
    pub heading_true: bool,
    pub airspeed_kt: Option<f64>,
    pub airspeed_true: bool,
    pub surface_direction_deg: Option<f64>,
    pub on_ground: Option<bool>,
    pub vertical_rate_fpm: Option<i32>,
    pub emergency: Option<u8>,
    pub squawk: Option<u32>,
    pub altitude_geometric: bool,
    pub position_seen_ms: Option<u64>,
    pub position_source: Option<String>,
    pub position_quality: Option<AircraftQuality>,
    pub msg_count: u64,
    pub last_seen_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AircraftQuality {
    pub reliable: bool,
    pub corrected_bits: u8,
    pub score: f64,
}

/// Preambles, accepted/rejected/Mode A/C, accepted by corrected bits, average
/// signal/noise power, peak signal power and strong-signal count.
pub type ReceiverDecodeStats = (u64, u64, u64, u64, u64, u64, u64, f64, f64, f64, u64);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReceiverSnapshot {
    pub timestamp_ms: u64,
    pub total_messages: u64,
    pub decode_stats: ReceiverDecodeStats,
    /// Accepted received messages in a fixed five-second window, per second.
    pub messages_per_second: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AircraftSnapshot {
    pub channel_id: ChannelId,
    pub aircraft: Vec<AircraftTelemetry>,
    pub receiver: ReceiverSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryFrame {
    /// A new identifier retires the entire previous image product. APID zero
    /// with empty dimensions/pixels announces a boundary before any image.
    pub product_id: u64,
    pub channel_id: ChannelId,
    pub apid: u16,
    pub width: u32,
    pub height: u32,
    /// Row-major 8-bit grayscale pixels for whatever has decoded so far.
    pub pixels: Vec<u8>,
    pub rs_ok: u32,
    pub rs_failed: u32,
    pub costas_locked: bool,
    pub frame_locked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GainMode {
    Automatic,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareStatus {
    pub connected: bool,
    pub source_kind: String,
    pub frequency_hz: u64,
    pub sample_rate_hz: u32,
    /// Selected supported gain step (zero is also possible in manual mode).
    pub gain_db: f64,
    /// Last accepted request, distinct from a hardware-selected gain step.
    #[serde(default)]
    pub requested_gain_db: Option<f64>,
    #[serde(default)]
    pub gain_mode: Option<GainMode>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingStatus {
    /// Why an automatically stopped or failed recording ended.
    pub stop_reason: Option<String>,
    pub channel_id: ChannelId,
    pub active: bool,
    pub path: Option<String>,
    pub bytes_written: u64,
    pub duration_sec: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerEvent {
    Welcome {
        protocol_version: u32,
        client_channels: ClientChannels,
        active_channels: Vec<ChannelSpec>,
    },
    Spectrum(SpectrumFrame),
    Audio(AudioFrame),
    /// Full snapshot of currently-tracked aircraft (daemon prunes stale entries itself).
    Aircraft(AircraftSnapshot),
    Telemetry(TelemetryFrame),
    Hardware(HardwareStatus),
    Recording(RecordingStatus),
    Pong {
        nonce: u64,
    },
    Error {
        message: String,
    },
    /// A correlated reply after the audio pipeline applied (or rejected) the
    /// threshold. Audio frames independently carry the actual applied revision.
    Squelch {
        channel_id: ChannelId,
        request_id: u64,
        result: Result<SquelchState, String>,
    },
    /// A subscribed channel was explicitly removed. Buffered older data can
    /// follow this control event and must be rejected by its channel identity.
    ChannelRemoved {
        channel_id: ChannelId,
    },
    /// Delivery updates skipped by this connection's bounded stream queues.
    /// Snapshot coalescing counts here too; this is not a count of lost RF samples.
    StreamGap {
        channel_id: ChannelId,
        dropped_frames: u64,
    },
}

#[cfg(test)]
mod hardware_status_tests {
    use super::*;
    #[test]
    fn hardware_gain_capabilities_are_backward_compatible_and_distinguish_manual_zero() {
        let legacy = r#"{"connected":true,"source_kind":"rtlsdr","frequency_hz":100000000,"sample_rate_hz":2048000,"gain_db":0.0,"error":null}"#;
        let mut status: HardwareStatus = serde_json::from_str(legacy).unwrap();
        assert_eq!(status.requested_gain_db, None);
        assert_eq!(status.gain_mode, None);
        status.requested_gain_db = Some(0.01);
        status.gain_mode = Some(GainMode::Manual);
        let json = serde_json::to_string(&status).unwrap();
        let decoded: HardwareStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.gain_db, 0.0);
        assert_eq!(decoded.requested_gain_db, Some(0.01));
        assert_eq!(decoded.gain_mode, Some(GainMode::Manual));
    }
}
