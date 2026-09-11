// Mirrors ez-proto/src/messages.rs. Field names and enum variant spellings must match
// exactly — everything here rides serde's default (externally-tagged) JSON representation:
// unit variants serialize as their bare variant name string ("Spectrum"), struct/newtype
// variants as { "VariantName": <payload> }.

export type ChannelId = number;

export type PipelineKind = "Spectrum" | "Audio" | "AdsbPackets" | "LrptTelemetry";

/** URL path segment used by `/api/channels/*` and `/ws/stream/{kind}/{id}` — see
 * ez-daemon/src/web/ws.rs's `parse_pipeline_kind`. Distinct from the JSON `PipelineKind`
 * spelling above (kebab-case in the URL, PascalCase in JSON bodies). */
export type PipelineKindPath = "spectrum" | "audio" | "adsb-packets" | "lrpt-telemetry";

export function pipelineKindToPath(kind: PipelineKind): PipelineKindPath {
  switch (kind) {
    case "Spectrum":
      return "spectrum";
    case "Audio":
      return "audio";
    case "AdsbPackets":
      return "adsb-packets";
    case "LrptTelemetry":
      return "lrpt-telemetry";
  }
}

export type DemodMode = "Raw" | "Am" | "Fm" | "Wfm" | "Lsb" | "Usb";

export const DEMOD_MODES: DemodMode[] = ["Raw", "Am", "Fm", "Wfm", "Lsb", "Usb"];

export type RecordingFormat = "Cf32" | "RawU8";

export interface ChannelSpec {
  id: ChannelId;
  center_offset_hz: number;
  bandwidth_hz: number;
  kind: PipelineKind;
  demod_mode: DemodMode | null;
}

export interface ChannelMetrics {
  spec: ChannelSpec;
  subscriber_count: number;
}

export interface HardwareStatus {
  connected: boolean;
  source_kind: string;
  frequency_hz: number;
  sample_rate_hz: number;
  gain_db: number;
  error: string | null;
}

export interface RecordingStatus {
  channel_id: ChannelId;
  active: boolean;
  path: string | null;
  bytes_written: number;
  duration_sec: number;
}

export interface StatusResponse {
  uptime_sec: number;
  hardware: HardwareStatus;
}

export interface AircraftTelemetry {
  icao: number;
  callsign: string | null;
  altitude_ft: number | null;
  lat: number | null;
  lon: number | null;
  ground_speed_kt: number | null;
  track_deg: number | null;
  vertical_rate_fpm: number | null;
  msg_count: number;
  last_seen_ms: number;
}

// Decoded (not wire-identical) shapes for the binary data-plane frames — see api/wire.ts.
export interface SpectrumFrame {
  center_hz: number;
  sample_rate_hz: number;
  timestamp_ms: number;
  bins: Float32Array;
}

export interface AudioFrame {
  channel_id: ChannelId;
  sample_rate_hz: number;
  samples: Float32Array;
}

export interface TelemetryFrame {
  channel_id: ChannelId;
  apid: number;
  width: number;
  height: number;
  rs_ok: number;
  rs_failed: number;
  costas_locked: boolean;
  frame_locked: boolean;
  pixels: Uint8Array;
}

// --- /ws/control JSON control-plane messages ---

export type ClientCommand =
  | { Hello: { client_name: string; protocol_version: number } }
  | { SetFrequency: { hz: number } }
  | { SetSampleRate: { hz: number } }
  | { SetGain: { db: number } }
  | { Subscribe: { channel: ChannelSpec } }
  | { Unsubscribe: { channel_id: ChannelId } }
  | { SetDemodMode: { channel_id: ChannelId; mode: DemodMode } }
  | { SetVolume: { channel_id: ChannelId; level: number } }
  | { SetSquelch: { channel_id: ChannelId; db: number } }
  | { StartRecording: { channel_id: ChannelId; format: RecordingFormat } }
  | { StopRecording: { channel_id: ChannelId } }
  | { Ping: { nonce: number } }
  | "Detach";

export type ServerEvent =
  | { Welcome: { protocol_version: number; active_channels: ChannelSpec[] } }
  | { Hardware: HardwareStatus }
  | { Recording: RecordingStatus }
  | { Aircraft: AircraftTelemetry[] }
  | { Pong: { nonce: number } }
  | { Error: { message: string } }
  // The control socket can technically also carry Spectrum/Audio/Telemetry (apply_command's
  // Subscribe path is shared with TCP), but this UI never issues Subscribe over /ws/control —
  // it always uses the dedicated binary /ws/stream/{kind}/{id} route for those instead.
  | { Spectrum: unknown }
  | { Audio: unknown }
  | { Telemetry: unknown };
