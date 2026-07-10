// Thin wrapper over `/api/*` — one function per ez-daemon/src/web/api.rs route. Every
// caller is responsible for its own error handling; this module only turns non-2xx
// responses into thrown Errors carrying the daemon's `{ "error": "..." }` body when present.

import type {
  ChannelId,
  ChannelMetrics,
  ChannelSpec,
  DemodMode,
  HardwareStatus,
  RecordingFormat,
  RecordingStatus,
  StatusResponse,
} from "../types";

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const resp = await fetch(path, {
    headers: init?.body ? { "Content-Type": "application/json" } : undefined,
    ...init,
  });
  if (!resp.ok) {
    let message = `${resp.status} ${resp.statusText}`;
    try {
      const body = await resp.json();
      if (typeof body?.error === "string") message = body.error;
    } catch {
      // Non-JSON error body (e.g. a plain 404 text response) — keep the status text.
    }
    throw new Error(message);
  }
  if (resp.status === 204) return undefined as T;
  return (await resp.json()) as T;
}

export function getStatus(): Promise<StatusResponse> {
  return request("/api/status");
}

export function getHardware(): Promise<HardwareStatus> {
  return request("/api/hardware");
}

export function setFrequency(hz: number): Promise<void> {
  return request("/api/hardware/frequency", { method: "POST", body: JSON.stringify({ hz }) });
}

export function setSampleRate(hz: number): Promise<void> {
  return request("/api/hardware/sample-rate", { method: "POST", body: JSON.stringify({ hz }) });
}

export function setGain(db: number): Promise<void> {
  return request("/api/hardware/gain", { method: "POST", body: JSON.stringify({ db }) });
}

export function listChannels(): Promise<ChannelMetrics[]> {
  return request("/api/channels");
}

export function createChannel(spec: ChannelSpec): Promise<ChannelSpec> {
  return request("/api/channels", { method: "POST", body: JSON.stringify(spec) });
}

export function setDemodMode(id: ChannelId, mode: DemodMode): Promise<void> {
  return request(`/api/channels/${id}/demod-mode`, {
    method: "POST",
    body: JSON.stringify({ mode }),
  });
}

export function setVolume(id: ChannelId, level: number): Promise<void> {
  return request(`/api/channels/${id}/volume`, {
    method: "POST",
    body: JSON.stringify({ level }),
  });
}

export function setSquelch(id: ChannelId, db: number): Promise<void> {
  return request(`/api/channels/${id}/squelch`, {
    method: "POST",
    body: JSON.stringify({ db }),
  });
}

export function getRecording(id: ChannelId): Promise<RecordingStatus | null> {
  return request<RecordingStatus>(`/api/channels/${id}/recording`).catch(() => null);
}

export function startRecording(id: ChannelId, format: RecordingFormat): Promise<RecordingStatus> {
  return request(`/api/channels/${id}/recording/start`, {
    method: "POST",
    body: JSON.stringify({ format }),
  });
}

export function stopRecording(id: ChannelId): Promise<RecordingStatus> {
  return request(`/api/channels/${id}/recording/stop`, { method: "POST" });
}

export function retuneChannel(
  id: ChannelId,
  center_offset_hz: number,
  bandwidth_hz: number,
): Promise<void> {
  return request(`/api/channels/${id}/retune`, {
    method: "POST",
    body: JSON.stringify({ center_offset_hz, bandwidth_hz }),
  });
}

export function deleteChannel(id: ChannelId): Promise<void> {
  return request(`/api/channels/${id}`, { method: "DELETE" });
}

export function listRecordings(): Promise<RecordingStatus[]> {
  return request("/api/recordings");
}
