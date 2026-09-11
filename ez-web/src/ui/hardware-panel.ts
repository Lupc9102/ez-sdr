// Hardware status readout + frequency/sample-rate/gain controls. Ticks arrive over
// /ws/control roughly 4x/sec (see ez-daemon's CONTROL_HW_TICK); inputs only sync their
// displayed value when the user isn't actively focused on them, so a live tick never
// clobbers an in-progress edit.

import type { HardwareStatus } from "../types";

export class HardwarePanel {
  private readonly freqInput: HTMLInputElement;
  private readonly rateInput: HTMLInputElement;
  private readonly gainInput: HTMLInputElement;
  private readonly statusLine: HTMLElement;
  private readonly errorLine: HTMLElement;

  onSetFrequency: ((hz: number) => void) | null = null;
  onSetSampleRate: ((hz: number) => void) | null = null;
  onSetGain: ((db: number) => void) | null = null;

  constructor(container: HTMLElement) {
    container.innerHTML = `
      <h2>Hardware</h2>
      <div class="row"><label>Source</label><span data-field="source">&mdash;</span></div>
      <div class="row"><label>Frequency</label><input data-field="freq" type="number" step="1" /></div>
      <div class="row"><label>Sample rate</label><input data-field="rate" type="number" step="1" /></div>
      <div class="row"><label>Gain (dB)</label><input data-field="gain" type="number" step="0.5" /></div>
      <div class="empty" data-field="error"></div>
    `;
    this.statusLine = container.querySelector('[data-field="source"]')!;
    this.errorLine = container.querySelector('[data-field="error"]')!;
    this.freqInput = container.querySelector('[data-field="freq"]')!;
    this.rateInput = container.querySelector('[data-field="rate"]')!;
    this.gainInput = container.querySelector('[data-field="gain"]')!;

    this.freqInput.addEventListener("change", () => {
      const hz = Number(this.freqInput.value);
      if (Number.isFinite(hz)) this.onSetFrequency?.(hz);
    });
    this.rateInput.addEventListener("change", () => {
      const hz = Number(this.rateInput.value);
      if (Number.isFinite(hz)) this.onSetSampleRate?.(hz);
    });
    this.gainInput.addEventListener("change", () => {
      const db = Number(this.gainInput.value);
      if (Number.isFinite(db)) this.onSetGain?.(db);
    });
  }

  setConnected(connected: boolean): void {
    if (!connected) this.statusLine.textContent = "disconnected";
  }

  setHardware(status: HardwareStatus): void {
    this.statusLine.textContent = `${status.source_kind}${status.connected ? "" : " (down)"}`;
    if (document.activeElement !== this.freqInput) this.freqInput.value = String(status.frequency_hz);
    if (document.activeElement !== this.rateInput) this.rateInput.value = String(status.sample_rate_hz);
    if (document.activeElement !== this.gainInput) this.gainInput.value = String(status.gain_db);
    this.errorLine.textContent = status.error ?? "";
  }
}
