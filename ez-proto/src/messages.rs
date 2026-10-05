//! Message types exchanged between daemon and clients.
//!
//! `ClientCommand` flows client -> daemon (control plane).
//! `ServerEvent` flows daemon -> client (telemetry/data plane), fanned out to every
//! subscriber of the relevant pipeline via an independent broadcast queue, so one slow
//! or disconnecting client never affects another.

use serde::{Deserialize, Serialize};

/// Bumped whenever a breaking wire-format change is made.
pub const PROTOCOL_VERSION: u32 = 6;

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
    pub id: ChannelId,
    /// Offset in Hz from the wideband center frequency. Positive = above center.
    pub center_offset_hz: i64,
    pub bandwidth_hz: u32,
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
        bandwidth_hz: u32,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareStatus {
    pub connected: bool,
    pub source_kind: String,
    pub frequency_hz: u64,
    pub sample_rate_hz: u32,
    pub gain_db: f64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingStatus {
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
}
