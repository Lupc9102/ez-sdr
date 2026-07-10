// Message protocol between ui/spectrum-view.ts (main thread) and this worker. Typed here so
// both sides get compile-time checking on postMessage payloads instead of raw object literals.

import type { PaletteName } from "../render/colormap";

export type SpectrumWorkerInMessage =
  | { type: "init"; width: number; height: number }
  | { type: "frame"; buf: ArrayBuffer }
  | { type: "resize"; width: number; height: number }
  | { type: "range"; minDb: number; maxDb: number }
  | { type: "split"; spectrumFrac: number }
  | { type: "palette"; name: PaletteName };

export interface SpectrumWorkerMeta {
  type: "meta";
  centerHz: number;
  sampleRateHz: number;
  binCount: number;
  frameRate: number;
}

// The worker paints to its own OffscreenCanvas and ships a snapshot bitmap for the main
// thread to blit — see spectrum-worker.ts for why we avoid transferControlToOffscreen.
export interface SpectrumWorkerBitmap {
  type: "bitmap";
  bitmap: ImageBitmap;
}

export type SpectrumWorkerOutMessage = SpectrumWorkerMeta | SpectrumWorkerBitmap;
