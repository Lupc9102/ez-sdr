// Data-plane WebSocket client for `/ws/stream/{kind}/{id}` — one socket per subscribed
// stream, binary frames (except adsb-packets, which is JSON text; see ez-daemon's ws.rs).
//
// Raw ArrayBuffers are handed to the callback untouched so hot paths (the spectrum worker)
// can transfer them to a Worker without a copy. Reconnects with capped backoff like
// ControlSocket, but gives up permanently once `close()` is called or the server refuses
// the socket `MAX_REJECTS` times in a row (e.g. the channel was torn down server-side) —
// hammering a 404 route forever would just be noise.

import { streamUrl } from "./control";
import type { PipelineKindPath } from "../types";

const BACKOFF_INITIAL_MS = 500;
const BACKOFF_MAX_MS = 10_000;
const MAX_REJECTS = 5;

export class StreamSocket {
  private ws: WebSocket | null = null;
  private backoffMs = BACKOFF_INITIAL_MS;
  private closed = false;
  private rejects = 0;
  private everOpened = false;
  private reconnectTimer: number | undefined;

  constructor(
    kind: PipelineKindPath,
    id: number,
    private readonly onBinary: (buf: ArrayBuffer) => void,
    private readonly onText?: (text: string) => void,
    private readonly url: string = streamUrl(kind, id),
  ) {}

  connect(): void {
    if (this.closed || this.ws) return;
    const ws = new WebSocket(this.url);
    ws.binaryType = "arraybuffer";
    this.ws = ws;

    ws.onopen = () => {
      this.backoffMs = BACKOFF_INITIAL_MS;
      this.rejects = 0;
      this.everOpened = true;
    };
    ws.onmessage = (msg: MessageEvent) => {
      if (msg.data instanceof ArrayBuffer) this.onBinary(msg.data);
      else if (typeof msg.data === "string") this.onText?.(msg.data);
    };
    ws.onclose = () => {
      const opened = this.everOpened;
      this.ws = null;
      this.everOpened = false;
      if (!opened) this.rejects += 1;
      if (this.rejects >= MAX_REJECTS) {
        this.closed = true;
        return;
      }
      this.scheduleReconnect();
    };
    ws.onerror = () => {
      // onclose always follows.
    };
  }

  private scheduleReconnect(): void {
    if (this.closed || this.reconnectTimer !== undefined) return;
    this.reconnectTimer = window.setTimeout(() => {
      this.reconnectTimer = undefined;
      this.connect();
    }, this.backoffMs);
    this.backoffMs = Math.min(this.backoffMs * 2, BACKOFF_MAX_MS);
  }

  close(): void {
    this.closed = true;
    if (this.reconnectTimer !== undefined) window.clearTimeout(this.reconnectTimer);
    this.ws?.close();
    this.ws = null;
  }
}
