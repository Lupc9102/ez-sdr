// 256-entry RGB lookup tables for spectrum/waterfall colour mapping. Mirrors the palette
// set in ez-gui/src/spectrum.rs's `ColorMap` enum for feature parity (same named options),
// though these are freshly built from published colour-space anchor points rather than a
// port of that LUT byte-for-byte — pixel-identical output was never the goal, matching the
// same set of look-and-feel palettes is.

export type PaletteName =
  | "Classic"
  | "Viridis"
  | "Plasma"
  | "Magma"
  | "Grayscale"
  | "Hot"
  | "Inferno"
  | "Turbo";

export const PALETTE_NAMES: PaletteName[] = [
  "Classic",
  "Viridis",
  "Plasma",
  "Magma",
  "Grayscale",
  "Hot",
  "Inferno",
  "Turbo",
];

type Rgb = [number, number, number];

function hex(h: string): Rgb {
  const n = parseInt(h, 16);
  return [(n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff];
}

// Anchor stops for each palette, evenly spaced across [0, 1] unless noted. Linearly
// interpolated in RGB space when building the 256-entry LUT — cheap and banding-free
// at 256 steps for all of these.
const STOPS: Record<Exclude<PaletteName, "Turbo">, Rgb[]> = {
  // EZ-SDR's original scheme: dark background rising through blue/green/yellow/red.
  Classic: [hex("000020"), hex("0000ff"), hex("00ff00"), hex("ffff00"), hex("ff0000")],
  Grayscale: [hex("000000"), hex("ffffff")],
  Hot: [hex("000000"), hex("ff0000"), hex("ffff00"), hex("ffffff")],
  Viridis: [
    hex("440154"),
    hex("482878"),
    hex("3e4989"),
    hex("31688e"),
    hex("26828e"),
    hex("1f9e89"),
    hex("35b779"),
    hex("6ece58"),
    hex("fde725"),
  ],
  Plasma: [
    hex("0d0887"),
    hex("47039f"),
    hex("7301a8"),
    hex("9c179e"),
    hex("bd3786"),
    hex("d8576b"),
    hex("ed7953"),
    hex("fbb32f"),
    hex("f0f921"),
  ],
  Magma: [
    hex("000004"),
    hex("180f3e"),
    hex("451077"),
    hex("721f81"),
    hex("9f2f7f"),
    hex("cd4071"),
    hex("f1605d"),
    hex("fd9567"),
    hex("fcfdbf"),
  ],
  Inferno: [
    hex("000004"),
    hex("1b0c41"),
    hex("4a0c6b"),
    hex("781c6d"),
    hex("a52c60"),
    hex("cf4446"),
    hex("ed6925"),
    hex("fbb61a"),
    hex("fcffa4"),
  ],
};

function interpolateStops(stops: Rgb[], t: number): Rgb {
  const clamped = Math.min(Math.max(t, 0), 1);
  const segments = stops.length - 1;
  const scaled = clamped * segments;
  const i = Math.min(Math.floor(scaled), segments - 1);
  const frac = scaled - i;
  const a = stops[i]!;
  const b = stops[i + 1]!;
  return [
    a[0] + (b[0] - a[0]) * frac,
    a[1] + (b[1] - a[1]) * frac,
    a[2] + (b[2] - a[2]) * frac,
  ];
}

// Google's public-domain Turbo colormap polynomial approximation (Anton Mikhailov, 2019):
// https://research.google/blog/turbo-an-improved-rainbow-colormap-for-visualization/
// Degree-5 polynomial per channel, coefficients reproduced verbatim from that source.
function turbo(t: number): Rgb {
  const x = Math.min(Math.max(t, 0), 1);
  const r = 0.13572138 + x * (4.6153926 + x * (-42.66032258 + x * (132.13108234 + x * (-152.94239396 + x * 59.28637943))));
  const g = 0.09140261 + x * (2.19418839 + x * (4.84296658 + x * (-14.18503333 + x * (4.27729857 + x * 2.82956604))));
  const b = 0.1066733 + x * (12.64194608 + x * (-60.58204836 + x * (110.36276771 + x * (-89.90310912 + x * 27.34824973))));
  return [r * 255, g * 255, b * 255];
}

const cache = new Map<PaletteName, Uint8ClampedArray>();

/** Builds (and memoizes) a 256-entry RGB LUT, 3 bytes per entry, for the given palette. */
export function getColormap(name: PaletteName): Uint8ClampedArray {
  const cached = cache.get(name);
  if (cached) return cached;

  const lut = new Uint8ClampedArray(256 * 3);
  for (let i = 0; i < 256; i++) {
    const t = i / 255;
    const [r, g, b] = name === "Turbo" ? turbo(t) : interpolateStops(STOPS[name], t);
    lut[i * 3] = r;
    lut[i * 3 + 1] = g;
    lut[i * 3 + 2] = b;
  }
  cache.set(name, lut);
  return lut;
}
