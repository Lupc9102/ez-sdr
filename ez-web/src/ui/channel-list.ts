// Channel table + create-channel form. Table rows re-render on every setChannels() call
// (cheap — a handful of rows), but the create form lives in its own DOM subtree so
// re-rendering the list never clobbers an in-progress form edit.

import { DEMOD_MODES, type ChannelMetrics, type ChannelSpec, type DemodMode, type PipelineKind } from "../types";

const KINDS: PipelineKind[] = ["Spectrum", "Audio", "AdsbPackets", "LrptTelemetry"];

export class ChannelList {
  private channels: ChannelMetrics[] = [];
  private selectedId: number | null = null;
  private readonly rows: HTMLElement;
  private readonly empty: HTMLElement;
  private readonly idInput: HTMLInputElement;
  private readonly kindSelect: HTMLSelectElement;
  private readonly offsetInput: HTMLInputElement;
  private readonly bwInput: HTMLInputElement;
  private readonly modeRow: HTMLElement;
  private readonly modeSelect: HTMLSelectElement;

  onSelect: ((id: number) => void) | null = null;
  onCreate: ((spec: ChannelSpec) => void) | null = null;

  constructor(container: HTMLElement) {
    container.innerHTML = `
      <h2>Channels</h2>
      <table>
        <thead><tr><th>ID</th><th>Kind</th><th>Offset</th><th>BW</th><th>Mode</th><th>Subs</th></tr></thead>
        <tbody data-field="rows"></tbody>
      </table>
      <div class="empty" data-field="empty">No channels yet.</div>
      <div class="row"><label>ID</label><input data-field="new-id" type="number" min="0" /></div>
      <div class="row"><label>Kind</label><select data-field="new-kind"></select></div>
      <div class="row"><label>Offset Hz</label><input data-field="new-offset" type="number" step="1" value="0" /></div>
      <div class="row"><label>Bandwidth Hz</label><input data-field="new-bw" type="number" step="1" value="200000" /></div>
      <div class="row" data-field="new-mode-row"><label>Demod</label><select data-field="new-mode"></select></div>
      <button data-field="create-btn" class="primary">Create channel</button>
    `;
    this.rows = container.querySelector('[data-field="rows"]')!;
    this.empty = container.querySelector('[data-field="empty"]')!;
    this.idInput = container.querySelector('[data-field="new-id"]')!;
    this.kindSelect = container.querySelector('[data-field="new-kind"]')!;
    this.offsetInput = container.querySelector('[data-field="new-offset"]')!;
    this.bwInput = container.querySelector('[data-field="new-bw"]')!;
    this.modeRow = container.querySelector('[data-field="new-mode-row"]')!;
    this.modeSelect = container.querySelector('[data-field="new-mode"]')!;

    this.kindSelect.innerHTML = KINDS.map((k) => `<option value="${k}">${k}</option>`).join("");
    this.modeSelect.innerHTML = DEMOD_MODES.map((m) => `<option value="${m}">${m}</option>`).join("");
    this.kindSelect.addEventListener("change", () => this.syncModeVisibility());
    this.syncModeVisibility();

    const createBtn: HTMLButtonElement = container.querySelector('[data-field="create-btn"]')!;
    createBtn.addEventListener("click", () => this.submitCreate());
  }

  setChannels(channels: ChannelMetrics[]): void {
    this.channels = channels;
    this.renderRows();
    if (!this.idInput.value || this.channels.some((c) => c.spec.id === Number(this.idInput.value))) {
      const nextId = channels.reduce((max, c) => Math.max(max, c.spec.id), 0) + 1;
      this.idInput.value = String(nextId);
    }
  }

  setSelected(id: number | null): void {
    this.selectedId = id;
    for (const tr of this.rows.querySelectorAll("tr")) {
      tr.classList.toggle("selected", tr.getAttribute("data-id") === String(id));
    }
  }

  private syncModeVisibility(): void {
    this.modeRow.style.display = this.kindSelect.value === "Audio" ? "flex" : "none";
  }

  private renderRows(): void {
    this.empty.style.display = this.channels.length === 0 ? "block" : "none";
    this.rows.innerHTML = this.channels
      .map((c) => {
        const s = c.spec;
        const selected = s.id === this.selectedId ? " selected" : "";
        return `<tr data-id="${s.id}" class="${selected}">
          <td>${s.id}</td>
          <td>${s.kind}</td>
          <td>${formatHz(s.center_offset_hz)}</td>
          <td>${formatHz(s.bandwidth_hz)}</td>
          <td>${s.demod_mode ?? "&mdash;"}</td>
          <td>${c.subscriber_count}</td>
        </tr>`;
      })
      .join("");
    for (const tr of this.rows.querySelectorAll("tr")) {
      tr.addEventListener("click", () => {
        const id = Number(tr.getAttribute("data-id"));
        this.onSelect?.(id);
      });
    }
  }

  private submitCreate(): void {
    const id = Number(this.idInput.value);
    const kind = this.kindSelect.value as PipelineKind;
    const center_offset_hz = Number(this.offsetInput.value);
    const bandwidth_hz = Number(this.bwInput.value);
    if (!Number.isFinite(id) || !Number.isFinite(center_offset_hz) || !Number.isFinite(bandwidth_hz)) return;
    const demod_mode: DemodMode | null = kind === "Audio" ? (this.modeSelect.value as DemodMode) : null;
    this.onCreate?.({ id, center_offset_hz, bandwidth_hz, kind, demod_mode });
  }
}

function formatHz(hz: number): string {
  const abs = Math.abs(hz);
  if (abs >= 1e6) return `${(hz / 1e6).toFixed(3)} MHz`;
  if (abs >= 1e3) return `${(hz / 1e3).toFixed(1)} kHz`;
  return `${hz.toFixed(0)} Hz`;
}
