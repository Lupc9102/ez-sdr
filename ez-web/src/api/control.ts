// Control-plane WebSocket client for `/ws/control` (JSON ClientCommand out, ServerEvent in).
//
// Reconnects automatically with capped exponential backoff — a daemon restart or network
// blip should self-heal without the user reloading the page. The server pushes Welcome +
// Hardware immediately on connect (no Hello handshake — see ez-daemon/src/web/ws.rs), so a
// fresh connection re-syncs hardware state on its own.

import type { ClientCommand, ServerEvent } from "../types";

export type ControlListener = (event: ServerEvent) => void;
export type ConnectionListener = (connected: boolean) => void;

const BACKOFF_INITIAL_MS = 500;
const BACKOFF_MAX_MS = 10_000;

export class ControlSocket {
  private ws: WebSocket | null = null;
  private backoffMs = BACKOFF_INITIAL_MS;
  private closed = false;
  private reconnectTimer: number | undefined;
  private readonly listeners = new Set<ControlListener>();
  private readonly connectionListeners = new Set<ConnectionListener>();

  constructor(private readonly url: string = controlUrl()) {}

  connect(): void {
    if (this.closed || this.ws) return;
    const ws = new WebSocket(this.url);
    this.ws = ws;

    ws.onopen = () => {
      this.backoffMs = BACKOFF_INITIAL_MS;
      for (const l of this.connectionListeners) l(true);
    };
    ws.onmessage = (msg: MessageEvent) => {
      if (typeof msg.data !== "string") return;
      let event: ServerEvent;
      try {
        event = JSON.parse(msg.data) as ServerEvent;
      } catch {
        return;
      }
      for (const l of this.listeners) l(event);
    };
    ws.onclose = () => {
      this.ws = null;
      for (const l of this.connectionListeners) l(false);
      this.scheduleReconnect();
    };
    ws.onerror = () => {
      // onclose always follows; reconnect is handled there.
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

  send(cmd: ClientCommand): boolean {
    if (this.ws?.readyState !== WebSocket.OPEN) return false;
    this.ws.send(JSON.stringify(cmd));
    return true;
  }

  onEvent(listener: ControlListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  onConnection(listener: ConnectionListener): () => void {
    this.connectionListeners.add(listener);
    return () => this.connectionListeners.delete(listener);
  }

  close(): void {
    this.closed = true;
    if (this.reconnectTimer !== undefined) window.clearTimeout(this.reconnectTimer);
    this.ws?.close();
    this.ws = null;
  }
}

export function controlUrl(): string {
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  return `${proto}//${location.host}/ws/control`;
}

export function streamUrl(kind: string, id: number): string {
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  return `${proto}//${location.host}/ws/stream/${kind}/${id}`;
}
