// Framework-free unit tests for the data-plane binary decoders in ../src/api/wire.ts,
// asserting they decode the exact little-endian byte layouts documented in
// ez-daemon/src/web/wire.rs (the daemon's encoder, which has matching Rust tests).

import { test } from "node:test";
import assert from "node:assert/strict";
import { decodeSpectrum, decodeAudio, decodeTelemetry } from "../src/api/wire.ts";

function f32(buf: ArrayBuffer, offsetFloats: number): number {
  return new DataView(buf).getFloat32(offsetFloats * 4, true);
}

test("decodeSpectrum parses the header and bins", () => {
  const center = 433_000_000;
  const rate = 2_000_000;
  const ts = 123_456_789;
  const bins = [-10.0, -20.5, 3.25];

  const buf = new ArrayBuffer(24 + bins.length * 4);
  const dv = new DataView(buf);
  dv.setBigUint64(0, BigInt(center), true);
  dv.setUint32(8, rate, true);
  dv.setBigUint64(12, BigInt(ts), true);
  dv.setUint32(20, bins.length, true);
  bins.forEach((b, i) => dv.setFloat32(24 + i * 4, b, true));

  const frame = decodeSpectrum(buf);
  assert.equal(frame.center_hz, center);
  assert.equal(frame.sample_rate_hz, rate);
  assert.equal(frame.timestamp_ms, ts);
  assert.equal(frame.bins.length, bins.length);
  assert.equal(f32(frame.bins.buffer as ArrayBuffer, 0), -10.0);
  assert.equal(f32(frame.bins.buffer as ArrayBuffer, 1), -20.5);
  assert.equal(f32(frame.bins.buffer as ArrayBuffer, 2), 3.25);
});

test("decodeSpectrum with zero bins returns an empty array", () => {
  const buf = new ArrayBuffer(24);
  const dv = new DataView(buf);
  dv.setBigUint64(0, 100n, true);
  dv.setUint32(8, 2_000_000, true);
  dv.setBigUint64(12, 0n, true);
  dv.setUint32(20, 0, true);
  const frame = decodeSpectrum(buf);
  assert.equal(frame.bins.length, 0);
});

test("decodeAudio parses channel/sample metadata and samples", () => {
  const channelId = 7;
  const rate = 48_000;
  const samples = [0.5, -0.5];

  const buf = new ArrayBuffer(12 + samples.length * 4);
  const dv = new DataView(buf);
  dv.setUint32(0, channelId, true);
  dv.setUint32(4, rate, true);
  dv.setUint32(8, samples.length, true);
  samples.forEach((s, i) => dv.setFloat32(12 + i * 4, s, true));

  const frame = decodeAudio(buf);
  assert.equal(frame.channel_id, channelId);
  assert.equal(frame.sample_rate_hz, rate);
  assert.equal(frame.samples.length, samples.length);
  assert.equal(f32(frame.samples.buffer as ArrayBuffer, 0), 0.5);
  assert.equal(f32(frame.samples.buffer as ArrayBuffer, 1), -0.5);
});

test("decodeTelemetry parses flags and pixel payload", () => {
  const pixels = [1, 2, 3, 4, 5, 6, 7, 8];

  const buf = new ArrayBuffer(28 + pixels.length);
  const dv = new DataView(buf);
  dv.setUint32(0, 3, true);
  dv.setUint16(4, 65, true);
  dv.setUint16(6, 0b01, true); // costas_locked
  dv.setUint32(8, 4, true);
  dv.setUint32(12, 2, true);
  dv.setUint32(16, 10, true);
  dv.setUint32(20, 1, true);
  dv.setUint32(24, pixels.length, true);
  pixels.forEach((p, i) => dv.setUint8(28 + i, p));

  const frame = decodeTelemetry(buf);
  assert.equal(frame.channel_id, 3);
  assert.equal(frame.apid, 65);
  assert.equal(frame.costas_locked, true);
  assert.equal(frame.frame_locked, false);
  assert.equal(frame.width, 4);
  assert.equal(frame.height, 2);
  assert.equal(frame.rs_ok, 10);
  assert.equal(frame.rs_failed, 1);
  assert.deepEqual(Array.from(frame.pixels), pixels);
});

test("decodeTelemetry packs both lock flags when both set", () => {
  const buf = new ArrayBuffer(28);
  const dv = new DataView(buf);
  dv.setUint16(6, 0b11, true);

  const frame = decodeTelemetry(buf);
  assert.equal(frame.costas_locked, true);
  assert.equal(frame.frame_locked, true);
});
