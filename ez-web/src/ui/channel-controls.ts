// Controls for whichever channel is currently selected: demod mode (Audio only, mirrors
// what ez-daemon's set_demod_mode actually accepts), volume, squelch, recording, and a
// local audio-monitor toggle. Rebuilt wholesale on setChannel() since selection changes
// are infrequent (a click), unlike the hardware panel's 4x/sec ticks.

import type { ChannelSpec, DemodMode, RecordingFormat, RecordingStatus } from "../types";
import { DEMOD_MODES } from "../types";

export class ChannelControls {
  private spec: ChannelSpec | null = null;
  private recording: RecordingStatus | null = null;
  private monitoringChannelId: number | null = null;

  onSetDemodMode: ((id: number, mode: DemodMode) => void) | null = null;
  onSetVolume: ((id: number, level: number) => void) | null = null;
  onSetSquelch: ((id: number, db: number) => void) | null = null;
  onStartRecording: ((id: number, format: RecordingFormat) => void) | null = null;
  onStopRecording: ((id: number) => void) | null = null;
  onToggleMonitor: ((id: number) => void) | null = null;
  onDelete: ((id: number) => void) | null = null;

  constructor(private readonly container: HTMLElement) {
    this.render();
  }

  setChannel(spec: ChannelSpec | null): void {
    this.spec = spec;
    this.render();
  }

  setRecording(status: RecordingStatus | null): void {
    this.recording = status;
    const el = this.container.querySelector<HTMLElement>('[data-field="recording-status"]');
    if (el) el.textContent = formatRecording(status);
  }

  setMonitoring(channelId: number | null): void {
    this.monitoringChannelId = channelId;
    const btn = this.container.querySelector<HTMLButtonElement>('[data-field="monitor-btn"]');
    if (btn && this.spec) {
      btn.textContent = channelId === this.spec.id ? "Stop monitoring" : "Monitor audio";
    }
  }

  private render(): void {
    const spec = this.spec;
    if (!spec) {
      this.container.innerHTML = `<h2>Channel</h2><div class="empty">Select a channel.</div>`;
      return;
    }

    const modeOptions = DEMOD_MODES.map(
      (m) => `<option value="${m}" ${m === spec.demod_mode ? "selected" : ""}>${m}</option>`,
    ).join("");

    this.container.innerHTML = `
      <h2>Channel ${spec.id} &mdash; ${spec.kind}</h2>
      ${
        spec.kind === "Audio"
          ? `<div class="row"><label>Demod</label><select data-field="mode">${modeOptions}</select></div>
             <div class="row"><label>Volume</label><input data-field="volume" type="range" min="0" max="1.5" step="0.01" value="1" /></div>
             <div class="row"><label>Squelch dB</label><input data-field="squelch" type="number" step="1" value="-100" /></div>
             <button data-field="monitor-btn">Monitor audio</button>`
          : ""
      }
      <div class="row" style="margin-top: 8px;">
        <label>Recording</label>
        <select data-field="format"><option value="Cf32">Cf32</option><option value="RawU8">RawU8</option></select>
        <button data-field="rec-start">Start</button>
        <button data-field="rec-stop" class="danger">Stop</button>
      </div>
      <div class="field-hint" data-field="recording-status">${formatRecording(this.recording)}</div>
      <button data-field="delete-btn" class="danger" style="margin-top: 8px;">Delete channel</button>
    `;

    if (spec.kind === "Audio") {
      const modeSelect: HTMLSelectElement = this.container.querySelector('[data-field="mode"]')!;
      modeSelect.addEventListener("change", () => {
        this.onSetDemodMode?.(spec.id, modeSelect.value as DemodMode);
      });
      const volume: HTMLInputElement = this.container.querySelector('[data-field="volume"]')!;
      volume.addEventListener("input", () => {
        this.onSetVolume?.(spec.id, Number(volume.value));
      });
      const squelch: HTMLInputElement = this.container.querySelector('[data-field="squelch"]')!;
      squelch.addEventListener("change", () => {
        this.onSetSquelch?.(spec.id, Number(squelch.value));
      });
      const monitorBtn: HTMLButtonElement = this.container.querySelector('[data-field="monitor-btn"]')!;
      monitorBtn.addEventListener("click", () => this.onToggleMonitor?.(spec.id));
      this.setMonitoring(this.monitoringChannelId);
    }

    const formatSelect: HTMLSelectElement = this.container.querySelector('[data-field="format"]')!;
    this.container.querySelector('[data-field="rec-start"]')!.addEventListener("click", () => {
      this.onStartRecording?.(spec.id, formatSelect.value as RecordingFormat);
    });
    this.container.querySelector('[data-field="rec-stop"]')!.addEventListener("click", () => {
      this.onStopRecording?.(spec.id);
    });
    this.container.querySelector('[data-field="delete-btn"]')!.addEventListener("click", () => {
      this.onDelete?.(spec.id);
    });
  }
}

function formatRecording(status: RecordingStatus | null): string {
  if (!status?.active) return "not recording";
  const mb = (status.bytes_written / (1024 * 1024)).toFixed(1);
  return `recording: ${mb} MiB, ${status.duration_sec.toFixed(0)}s (${status.path ?? ""})`;
}
