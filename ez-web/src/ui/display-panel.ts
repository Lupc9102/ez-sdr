// Spectrum/waterfall display preferences — currently just the colour palette (see
// render/colormap.ts for the LUTs and the worker's "palette" message).

import { PALETTE_NAMES, type PaletteName } from "../render/colormap";

export class DisplayPanel {
  onSetPalette: ((name: PaletteName) => void) | null = null;

  constructor(private readonly container: HTMLElement) {
    container.innerHTML = `
      <h2>Display</h2>
      <div class="row">
        <label>Palette</label>
        <select data-field="palette">
          ${PALETTE_NAMES.map((p) => `<option value="${p}">${p}</option>`).join("")}
        </select>
      </div>
    `;
    const select: HTMLSelectElement = this.container.querySelector('[data-field="palette"]')!;
    select.value = "Classic";
    select.addEventListener("change", () => this.onSetPalette?.(select.value as PaletteName));
  }
}
