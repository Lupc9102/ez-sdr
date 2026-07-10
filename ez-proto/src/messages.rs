//! Message types exchanged between daemon and clients.
//!
//! `ClientCommand` flows client -> daemon (control plane).
//! `ServerEvent` flows daemon -> client (telemetry/data plane), fanned out to every
//! subscriber of the relevant pipeline via an independent broadcast queue, so one slow
//! or disconnecting client never affects another.

use serde::{Deserialize, Serialize};

/// Bumped whenever a breaking wire-format change is made.
pub const PROTOCOL_VERSION: u32 = 1;

/// Identifies a virtual channel (a narrowband slice of the wideband capture) that a
/// client has asked the daemon to carve out and route to a specific pipeline kind.
pub type ChannelId = u32;

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
    pub center_hz: u64,
    pub sample_rate_hz: u32,
    /// dB magnitudes, fftshifted to ascending-frequency order (bin 0 = center - fs/2).
    pub bins: Vec<f32>,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioFrame {
    pub channel_id: ChannelId,
    pub sample_rate_hz: u32,
    pub samples: Vec<f32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AircraftTelemetry {
    pub icao: u32,
    pub callsign: Option<String>,
    pub altitude_ft: Option<i32>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub ground_speed_kt: Option<f64>,
    pub track_deg: Option<f64>,
    pub vertical_rate_fpm: Option<i32>,
    pub msg_count: u64,
    pub last_seen_ms: u64,
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
        active_channels: Vec<ChannelSpec>,
    },
    Spectrum(SpectrumFrame),
    Audio(AudioFrame),
    /// Full snapshot of currently-tracked aircraft (daemon prunes stale entries itself).
    Aircraft(Vec<AircraftTelemetry>),
    Telemetry(TelemetryFrame),
    Hardware(HardwareStatus),
    Recording(RecordingStatus),
    Pong {
        nonce: u64,
    },
    Error {
        message: String,
    },
}
