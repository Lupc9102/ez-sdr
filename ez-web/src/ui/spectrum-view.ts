// Main-thread side of the spectrum/waterfall display. Owns the <canvas> pair:
// a normal <canvas> underneath that simply blits the ImageBitmaps the worker paints
// (spectrum trace + waterfall, done entirely off-thread), and a transparent overlay
// canvas on top for interactive chrome (frequency scale, channel markers, bandpass
// drag handles) that's cheap to repaint on the main thread only when interaction state
// changes.

import { StreamSocket } from "../api/stream";
import type { ChannelSpec } from "../types";
import type { PaletteName } from "../render/colormap";
import SpectrumWorker from "../workers/spectrum-worker?worker";

export interface SpectrumMeta {
  centerHz: number;
  sampleRateHz: number;
  binCount: number;
  frameRate: number;
}

export interface ChannelEdit {
  id: number;
  center_offset_hz: number;
  bandwidth_hz: number;
}

type DragState =
  | { kind: "move"; id: number; startX: number; startOffsetHz: number }
  | { kind: "edge"; id: number; side: "low" | "high"; startX: number; startBwHz: number };

const MIN_BANDWIDTH_HZ = 1_000;

export class SpectrumView {
  private readonly worker: Worker;
  private socket: StreamSocket | null = null;
  private meta: SpectrumMeta = { centerHz: 0, sampleRateHz: 0, binCount: 0, frameRate: 0 };
  private channels: ChannelSpec[] = [];
  private selectedId: number | null = null;
  private drag: DragState | null = null;
  private readonly canvasCtx: CanvasRenderingContext2D;
  private readonly overlayCtx: CanvasRenderingContext2D;
  private resizeObserver: ResizeObserver;

  onMeta: ((meta: SpectrumMeta) => void) | null = null;
  onChannelEdit: ((edit: ChannelEdit) => void) | null = null;
  onChannelSelect: ((id: number | null) => void) | null = null;

  constructor(
    private readonly container: HTMLElement,
    private readonly canvas: HTMLCanvasElement,
    private readonly overlay: HTMLCanvasElement,
  ) {
    // The spectrum canvas is a plain 2D canvas on the main thread; it only ever receives
    // finished bitmaps from the worker (see attachStream / onmessage). We do NOT transfer
    // control to the worker — resizing a transferred OffscreenCanvas from the worker fails to
    // composite in Firefox, leaving a black canvas. Keeping the canvas on the main thread also
    // means resize() can size it directly (a transferred canvas forbids that).
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("2d canvas unsupported");
    this.canvasCtx = ctx;

    const rect = container.getBoundingClientRect();
    const w = Math.max(100, Math.floor(rect.width));
    const h = Math.max(100, Math.floor(rect.height));
    canvas.width = w;
    canvas.height = h;

    this.worker = new SpectrumWorker();
    this.worker.postMessage({ type: "init", width: w, height: h });
    this.worker.onmessage = (e: MessageEvent) => {
      const data = e.data as { type: string };
      if (data?.type === "bitmap") {
        const bmp = (e.data as { bitmap: ImageBitmap }).bitmap;
        this.canvasCtx.drawImage(bmp, 0, 0);
        bmp.close();
      } else if (data?.type === "meta") {
        this.meta = e.data as SpectrumMeta;
        this.onMeta?.(this.meta);
        this.drawOverlay();
      }
    };

    const octx = overlay.getContext("2d");
    if (!octx) throw new Error("2d canvas unsupported");
    this.overlayCtx = octx;

    this.resizeObserver = new ResizeObserver(() => this.resize());
    this.resizeObserver.observe(container);
    this.resize();

    overlay.addEventListener("pointerdown", (e) => this.pointerDown(e));
    overlay.addEventListener("pointermove", (e) => this.pointerMove(e));
    overlay.addEventListener("pointerup", (e) => this.pointerUp(e));
    overlay.addEventListener("pointercancel", () => (this.drag = null));
  }

  attachStream(channelId: number): void {
    this.socket?.close();
    // Binary frames are transferred (zero-copy) straight into the worker.
    this.socket = new StreamSocket("spectrum", channelId, (buf) => {
      this.worker.postMessage({ type: "frame", buf }, [buf]);
    });
    this.socket.connect();
  }

  detachStream(): void {
    this.socket?.close();
    this.socket = null;
  }

  setChannels(channels: ChannelSpec[]): void {
    this.channels = channels;
    this.drawOverlay();
  }

  setSelected(id: number | null): void {
    this.selectedId = id;
    this.drawOverlay();
  }

  setDbRange(minDb: number, maxDb: number): void {
    this.worker.postMessage({ type: "range", minDb, maxDb });
  }

  setPalette(name: PaletteName): void {
    this.worker.postMessage({ type: "palette", name });
  }

  private resize(): void {
    const rect = this.container.getBoundingClientRect();
    const w = Math.max(100, Math.floor(rect.width));
    const h = Math.max(100, Math.floor(rect.height));
    this.worker.postMessage({ type: "resize", width: w, height: h });
    // Size the main-thread canvas directly (allowed now that we don't transfer control) so
    // the worker's bitmaps blit 1:1.
    this.canvas.width = w;
    this.canvas.height = h;
    this.overlay.width = w;
    this.overlay.height = h;
    this.drawOverlay();
  }

  // --- coordinate mapping ---

  private hzToX(offsetHz: number): number {
    const { sampleRateHz } = this.meta;
    if (sampleRateHz === 0) return -1;
    return ((offsetHz + sampleRateHz / 2) / sampleRateHz) * this.overlay.width;
  }

  private xToHz(x: number): number {
    const { sampleRateHz } = this.meta;
    return (x / this.overlay.width) * sampleRateHz - sampleRateHz / 2;
  }

  private pxPerHz(): number {
    return this.meta.sampleRateHz === 0 ? 0 : this.overlay.width / this.meta.sampleRateHz;
  }

  // --- interaction: drag channel markers / bandpass edges ---

  private hitTest(x: number): DragState | null {
    const edgePx = 6;
    // Audio-type channels get draggable passband edges; test edges before bodies so a
    // narrow channel can still be resized.
    for (const ch of this.channels) {
      if (ch.kind !== "Audio") continue;
      const lo = this.hzToX(ch.center_offset_hz - ch.bandwidth_hz / 2);
      const hi = this.hzToX(ch.center_offset_hz + ch.bandwidth_hz / 2);
      if (Math.abs(x - lo) <= edgePx) {
        return { kind: "edge", id: ch.id, side: "low", startX: x, startBwHz: ch.bandwidth_hz };
      }
      if (Math.abs(x - hi) <= edgePx) {
        return { kind: "edge", id: ch.id, side: "high", startX: x, startBwHz: ch.bandwidth_hz };
      }
    }
    for (const ch of this.channels) {
      if (ch.kind === "Spectrum") continue;
      const lo = this.hzToX(ch.center_offset_hz - ch.bandwidth_hz / 2);
      const hi = this.hzToX(ch.center_offset_hz + ch.bandwidth_hz / 2);
      if (x >= lo && x <= hi) {
        return { kind: "move", id: ch.id, startX: x, startOffsetHz: ch.center_offset_hz };
      }
    }
    return null;
  }

  private pointerDown(e: PointerEvent): void {
    const x = e.offsetX;
    const hit = this.hitTest(x);
    if (hit) {
      this.drag = hit;
      this.overlay.setPointerCapture(e.pointerId);
      this.selectedId = hit.id;
      this.onChannelSelect?.(hit.id);
    } else {
      this.selectedId = null;
      this.onChannelSelect?.(null);
    }
    this.drawOverlay();
  }

  private pointerMove(e: PointerEvent): void {
    if (!this.drag) {
      const hit = this.hitTest(e.offsetX);
      this.overlay.style.cursor =
        hit?.kind === "edge" ? "ew-resize" : hit ? "grab" : "crosshair";
      return;
    }
    const ch = this.channels.find((c) => c.id === this.drag!.id);
    if (!ch) return;
    const pxPerHz = this.pxPerHz();
    if (pxPerHz === 0) return;
    const dxHz = (e.offsetX - this.drag.startX) / pxPerHz;

    if (this.drag.kind === "move") {
      ch.center_offset_hz = Math.round(this.drag.startOffsetHz + dxHz);
    } else {
      const delta = this.drag.side === "high" ? dxHz : -dxHz;
      ch.bandwidth_hz = Math.max(MIN_BANDWIDTH_HZ, Math.round(this.drag.startBwHz + 2 * delta));
    }
    this.drawOverlay();
  }

  private pointerUp(e: PointerEvent): void {
    if (!this.drag) return;
    const ch = this.channels.find((c) => c.id === this.drag!.id);
    this.overlay.releasePointerCapture(e.pointerId);
    this.drag = null;
    if (ch) {
      this.onChannelEdit?.({
        id: ch.id,
        center_offset_hz: ch.center_offset_hz,
        bandwidth_hz: ch.bandwidth_hz,
      });
    }
  }

  // --- overlay painting ---

  private drawOverlay(): void {
    const ctx = this.overlayCtx;
    const w = this.overlay.width;
    const h = this.overlay.height;
    ctx.clearRect(0, 0, w, h);
    if (this.meta.sampleRateHz === 0) return;

    // Frequency scale along the bottom edge.
    ctx.font = "11px monospace";
    ctx.fillStyle = "#8a93a5";
    ctx.strokeStyle = "#2a3242";
    ctx.lineWidth = 1;
    const ticks = 10;
    for (let i = 0; i <= ticks; i++) {
      const x = (i / ticks) * w;
      const hz = this.meta.centerHz + this.xToHz(x);
      ctx.beginPath();
      ctx.moveTo(x, h - 18);
      ctx.lineTo(x, h - 12);
      ctx.stroke();
      const label = formatHz(hz);
      const tw = ctx.measureText(label).width;
      ctx.fillText(label, Math.min(Math.max(2, x - tw / 2), w - tw - 2), h - 2);
    }

    // Channel passband markers.
    for (const ch of this.channels) {
      if (ch.kind === "Spectrum") continue;
      const lo = this.hzToX(ch.center_offset_hz - ch.bandwidth_hz / 2);
      const hi = this.hzToX(ch.center_offset_hz + ch.bandwidth_hz / 2);
      const cx = this.hzToX(ch.center_offset_hz);
      const selected = ch.id === this.selectedId;
      const color =
        ch.kind === "Audio" ? "87, 217, 163" : ch.kind === "AdsbPackets" ? "94, 158, 255" : "255, 160, 87";

      ctx.fillStyle = `rgba(${color}, ${selected ? 0.22 : 0.12})`;
      ctx.fillRect(lo, 0, hi - lo, h - 20);
      ctx.strokeStyle = `rgba(${color}, ${selected ? 0.9 : 0.5})`;
      ctx.lineWidth = selected ? 2 : 1;
      ctx.strokeRect(lo, 0, hi - lo, h - 20);

      // Center line.
      ctx.beginPath();
      ctx.setLineDash([4, 4]);
      ctx.moveTo(cx, 0);
      ctx.lineTo(cx, h - 20);
      ctx.stroke();
      ctx.setLineDash([]);

      ctx.fillStyle = `rgba(${color}, 0.95)`;
      ctx.fillText(`ch ${ch.id}`, Math.max(2, lo + 3), 14);
    }
  }
}

function formatHz(hz: number): string {
  const abs = Math.abs(hz);
  if (abs >= 1e9) return `${(hz / 1e9).toFixed(3)} GHz`;
  if (abs >= 1e6) return `${(hz / 1e6).toFixed(3)} MHz`;
  if (abs >= 1e3) return `${(hz / 1e3).toFixed(1)} kHz`;
  return `${hz.toFixed(0)} Hz`;
}
