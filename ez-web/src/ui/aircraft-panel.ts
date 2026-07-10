// ADS-B aircraft table. Unlike every other stream kind, /ws/stream/adsb-packets/{id}
// carries a JSON *array* of AircraftTelemetry snapshots per text frame (see ez-daemon's
// ws.rs stream_channel and ChannelSubscription::AdsbPackets) — no ServerEvent envelope,
// so this decodes the array directly rather than going through the control-plane types.

import { StreamSocket } from "../api/stream";
import type { AircraftTelemetry } from "../types";

export class AircraftPanel {
  private socket: StreamSocket | null = null;
  private channelId: number | null = null;

  onCreateChannel: (() => void) | null = null;

  constructor(private readonly container: HTMLElement) {
    this.renderEmpty();
  }

  setChannelId(id: number | null): void {
    if (id === this.channelId) return;
    this.socket?.close();
    this.socket = null;
    this.channelId = id;
    if (id === null) {
      this.renderEmpty();
      return;
    }
    this.container.innerHTML = `
      <h2>Aircraft (ch ${id})</h2>
      <table>
        <thead><tr>
          <th>ICAO</th><th>Callsign</th><th>Alt (ft)</th><th>Lat</th><th>Lon</th>
          <th>GS (kt)</th><th>Trk</th><th>VRate</th><th>Msgs</th>
        </tr></thead>
        <tbody data-field="rows"></tbody>
      </table>
      <div class="empty" data-field="empty">Waiting for aircraft&hellip;</div>
    `;
    this.socket = new StreamSocket(
      "adsb-packets",
      id,
      () => {},
      (text) => this.handleText(text),
    );
    this.socket.connect();
  }

  private renderEmpty(): void {
    this.container.innerHTML = `
      <h2>Aircraft</h2>
      <div class="empty">No ADS-B channel yet.</div>
      <button data-field="create">Create ADS-B channel</button>
    `;
    this.container.querySelector('[data-field="create"]')!.addEventListener("click", () => {
      this.onCreateChannel?.();
    });
  }

  private handleText(text: string): void {
    let aircraft: AircraftTelemetry[];
    try {
      aircraft = JSON.parse(text);
    } catch {
      return;
    }
    const rows = this.container.querySelector<HTMLElement>('[data-field="rows"]');
    const empty = this.container.querySelector<HTMLElement>('[data-field="empty"]');
    if (!rows || !empty) return;
    empty.style.display = aircraft.length === 0 ? "block" : "none";
    rows.innerHTML = aircraft
      .sort((a, b) => b.last_seen_ms - a.last_seen_ms)
      .map(
        (ac) => `<tr>
          <td>${ac.icao.toString(16).toUpperCase().padStart(6, "0")}</td>
          <td>${ac.callsign?.trim() || "&mdash;"}</td>
          <td>${ac.altitude_ft ?? "&mdash;"}</td>
          <td>${ac.lat?.toFixed(4) ?? "&mdash;"}</td>
          <td>${ac.lon?.toFixed(4) ?? "&mdash;"}</td>
          <td>${ac.ground_speed_kt?.toFixed(0) ?? "&mdash;"}</td>
          <td>${ac.track_deg?.toFixed(0) ?? "&mdash;"}</td>
          <td>${ac.vertical_rate_fpm?.toFixed(0) ?? "&mdash;"}</td>
          <td>${ac.msg_count}</td>
        </tr>`,
      )
      .join("");
  }

  close(): void {
    this.socket?.close();
    this.socket = null;
  }
}
