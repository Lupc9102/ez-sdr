// Spectrum + waterfall renderer. Runs entirely inside a Web Worker: it decodes each binary
// spectrum frame, paints the dB trace + scrolling waterfall onto its OWN OffscreenCanvas, then
// snapshots the result with `transferToImageBitmap()` and hands the bitmap to the main thread,
// which simply blits it onto a normal canvas.
//
// We deliberately do NOT use `transferControlToOffscreen()`: once control is transferred the
// main-thread <canvas> composites the worker's OffscreenCanvas directly, and in Firefox resizing
// that OffscreenCanvas from inside the worker leaves the placeholder compositing a stale/empty
// 300x150 bitmap — a fully black spectrum with no error. The ImageBitmap + main-thread-blit
// pattern composites reliably across browsers and keeps all frame decoding/painting off-thread
// (the main thread never touches a raw spectrum frame, only a finished bitmap).
//
// `self` is shadowed locally instead of pulling in the "webworker" lib: mixing "dom" (needed by
// every main-thread module) and "webworker" ambient globals in one tsconfig produces conflicting
// declarations, and everything this file touches (OffscreenCanvas, ImageBitmap, MessageEvent)
// already resolves under plain "dom".

import { decodeSpectrum } from "../api/wire";
import { getColormap, type PaletteName } from "../render/colormap";
import type { SpectrumWorkerInMessage, SpectrumWorkerOutMessage } from "./spectrum-protocol";
import type { SpectrumFrame } from "../types";

interface WorkerSelf {
  postMessage(message: unknown, transfer?: Transferable[]): void;
  onmessage: ((ev: MessageEvent<SpectrumWorkerInMessage>) => void) | null;
}
declare const self: WorkerSelf;

let canvas: OffscreenCanvas | null = null;
let ctx: OffscreenCanvasRenderingContext2D | null = null;
let minDb = -120;
let maxDb = 0;
let spectrumFrac = 0.35;
let palette: PaletteName = "Classic";

// Auto-range: until the main thread sends an explicit `range` (manual override), track a
// smoothed min/max of the live frames so the trace/waterfall keep good contrast regardless
// of source level. Without this the default -120..0 window maps typical synthetic frames
// (≈ -86..-10 dB) to near-black colors.
let autoRange = true;
let autoMin = -100;
let autoMax = -20;

let framesSinceMeta = 0;
let lastMetaTime = 0;

function post(msg: SpectrumWorkerOutMessage): void {
  self.postMessage(msg);
}

function spectrumHeight(): number {
  return canvas ? Math.max(40, Math.floor(canvas.height * spectrumFrac)) : 0;
}

self.onmessage = (ev) => {
  const msg = ev.data;
  switch (msg.type) {
    case "init":
      canvas = new OffscreenCanvas(msg.width, msg.height);
      ctx = canvas.getContext("2d");
      break;
    case "frame":
      handleFrame(msg.buf);
      break;
    case "resize":
      if (canvas) {
        canvas.width = msg.width;
        canvas.height = msg.height;
        // After a resize the left/right edges of the spectrum region can show a stale column
        // until the next frame paints; clear so there is never a garbage sliver.
        ctx?.clearRect(0, 0, canvas.width, canvas.height);
      }
      break;
    case "range":
      // Manual override disables auto-range.
      autoRange = false;
      minDb = msg.minDb;
      maxDb = msg.maxDb;
      break;
    case "split":
      spectrumFrac = msg.spectrumFrac;
      break;
    case "palette":
      palette = msg.name;
      break;
  }
};

/** Peak value across the bin range that maps to pixel column `x`, so a spike between
 * sampled columns never disappears when there are more bins than pixels (e.g. a 4096-bin
 * FFT on a 1200px-wide canvas) — plain nearest-neighbor sampling would drop it. When there
 * are fewer bins than pixels this just returns the one bin covering `x`, i.e. correct
 * upsampling too, so one loop shape handles both cases. */
function peakForColumn(bins: Float32Array, binsPerPx: number, x: number): number {
  const start = Math.floor(x * binsPerPx);
  const end = Math.max(start + 1, Math.floor((x + 1) * binsPerPx));
  let peak = -Infinity;
  for (let b = start; b < end && b < bins.length; b++) {
    const v = bins[b]!;
    if (v > peak) peak = v;
  }
  return peak;
}

function handleFrame(buf: ArrayBuffer): void {
  if (!ctx || !canvas) return;
  const frame = decodeSpectrum(buf);
  if (frame.bins.length === 0) return;

  if (autoRange) {
    let fmin = Infinity;
    let fmax = -Infinity;
    for (let i = 0; i < frame.bins.length; i++) {
      const v = frame.bins[i]!;
      if (v < fmin) fmin = v;
      if (v > fmax) fmax = v;
    }
    if (fmin !== Infinity) {
      // Slow EMA so the window eases toward the live extremes instead of jittering.
      autoMin += (fmin - autoMin) * 0.02;
      autoMax += (fmax - autoMax) * 0.02;
      minDb = autoMin;
      maxDb = autoMax;
    }
  }

  drawSpectrum(frame);
  drawWaterfallRow(frame);

  // Hand the painted bitmap to the main thread. transferToImageBitmap snapshots the current
  // OffscreenCanvas content without detaching it, so the next frame can keep painting.
  const bitmap = canvas.transferToImageBitmap();
  self.postMessage({ type: "bitmap", bitmap }, [bitmap]);

  framesSinceMeta += 1;
  const now = performance.now();
  const elapsed = now - lastMetaTime;
  if (elapsed > 500) {
    post({
      type: "meta",
      centerHz: frame.center_hz,
      sampleRateHz: frame.sample_rate_hz,
      binCount: frame.bins.length,
      frameRate: framesSinceMeta / (elapsed / 1000),
    });
    framesSinceMeta = 0;
    lastMetaTime = now;
  }
}

function drawSpectrum(frame: SpectrumFrame): void {
  if (!ctx || !canvas) return;
  const width = canvas.width;
  const height = spectrumHeight();
  if (width === 0 || height === 0) return;
  const bins = frame.bins;
  const binsPerPx = bins.length / width;
  const range = Math.max(maxDb - minDb, 1);
  const dbToY = (db: number) => height - ((db - minDb) / range) * height;

  ctx.fillStyle = "#000005";
  ctx.fillRect(0, 0, width, height);

  // dB gridlines every 20 dB.
  ctx.strokeStyle = "rgba(90, 100, 120, 0.35)";
  ctx.fillStyle = "rgba(150, 160, 180, 0.8)";
  ctx.font = "10px monospace";
  ctx.lineWidth = 1;
  const step = 20;
  const top = Math.ceil(maxDb / step) * step;
  for (let db = top; db >= minDb; db -= step) {
    const y = dbToY(db);
    ctx.beginPath();
    ctx.moveTo(0, y);
    ctx.lineTo(width, y);
    ctx.stroke();
    ctx.fillText(`${db.toFixed(0)}`, 2, y - 2);
  }

  // Filled trace under the line.
  ctx.beginPath();
  ctx.moveTo(0, height);
  for (let x = 0; x < width; x++) {
    ctx.lineTo(x, dbToY(peakForColumn(bins, binsPerPx, x)));
  }
  ctx.lineTo(width, height);
  ctx.closePath();
  const gradient = ctx.createLinearGradient(0, 0, 0, height);
  gradient.addColorStop(0, "rgba(52, 152, 219, 0.55)");
  gradient.addColorStop(1, "rgba(10, 30, 60, 0.05)");
  ctx.fillStyle = gradient;
  ctx.fill();

  ctx.strokeStyle = "#3498db";
  ctx.lineWidth = 1.25;
  ctx.beginPath();
  for (let x = 0; x < width; x++) {
    const y = dbToY(peakForColumn(bins, binsPerPx, x));
    if (x === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  }
  ctx.stroke();
}

function drawWaterfallRow(frame: SpectrumFrame): void {
  if (!ctx || !canvas) return;
  const width = canvas.width;
  const wfTop = spectrumHeight();
  const wfHeight = canvas.height - wfTop;
  if (width === 0 || wfHeight <= 0) return;

  // Scroll existing waterfall history down by one row, then paint the new row at the
  // top of the waterfall region — drawImage reads a snapshot of its source, so
  // source/destination aliasing on the same canvas is well-defined.
  if (wfHeight > 1) {
    ctx.drawImage(canvas, 0, wfTop, width, wfHeight - 1, 0, wfTop + 1, width, wfHeight - 1);
  }

  const lut = getColormap(palette);
  const bins = frame.bins;
  const binsPerPx = bins.length / width;
  const range = Math.max(maxDb - minDb, 1);
  const row = ctx.createImageData(width, 1);
  for (let x = 0; x < width; x++) {
    const peak = peakForColumn(bins, binsPerPx, x);
    const norm = Math.min(Math.max((peak - minDb) / range, 0), 1);
    const lutIdx = Math.round(norm * 255) * 3;
    const p = x * 4;
    row.data[p] = lut[lutIdx]!;
    row.data[p + 1] = lut[lutIdx + 1]!;
    row.data[p + 2] = lut[lutIdx + 2]!;
    row.data[p + 3] = 255;
  }
  ctx.putImageData(row, 0, wfTop);
}
