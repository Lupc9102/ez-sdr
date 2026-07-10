// Decoders for the data-plane binary WS frames. Mirrors the little-endian layouts
// documented in ez-daemon/src/web/wire.rs byte-for-byte — see that file for the
// authoritative offset tables. Deliberately hand-rolled (not a generic schema decoder):
// three fixed shapes, and matching the Rust doc comments field-by-field is easier to audit
// than a layer of indirection would be.

import type { AudioFrame, SpectrumFrame, TelemetryFrame } from "../types";

export function decodeSpectrum(buf: ArrayBuffer): SpectrumFrame {
  const view = new DataView(buf);
  const center_hz = view.getBigUint64(0, true);
  const sample_rate_hz = view.getUint32(8, true);
  const timestamp_ms = view.getBigUint64(12, true);
  const bin_count = view.getUint32(20, true);
  const bins = new Float32Array(buf.slice(24, 24 + bin_count * 4));
  return {
    center_hz: Number(center_hz),
    sample_rate_hz,
    timestamp_ms: Number(timestamp_ms),
    bins,
  };
}

export function decodeAudio(buf: ArrayBuffer): AudioFrame {
  const view = new DataView(buf);
  const channel_id = view.getUint32(0, true);
  const sample_rate_hz = view.getUint32(4, true);
  const sample_count = view.getUint32(8, true);
  const samples = new Float32Array(buf.slice(12, 12 + sample_count * 4));
  return { channel_id, sample_rate_hz, samples };
}

export function decodeTelemetry(buf: ArrayBuffer): TelemetryFrame {
  const view = new DataView(buf);
  const channel_id = view.getUint32(0, true);
  const apid = view.getUint16(4, true);
  const flags = view.getUint16(6, true);
  const width = view.getUint32(8, true);
  const height = view.getUint32(12, true);
  const rs_ok = view.getUint32(16, true);
  const rs_failed = view.getUint32(20, true);
  const pixel_count = view.getUint32(24, true);
  const pixels = new Uint8Array(buf.slice(28, 28 + pixel_count));
  return {
    channel_id,
    apid,
    width,
    height,
    rs_ok,
    rs_failed,
    costas_locked: (flags & 0b01) !== 0,
    frame_locked: (flags & 0b10) !== 0,
    pixels,
  };
}
