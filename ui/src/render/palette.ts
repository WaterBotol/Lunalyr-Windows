// Album colours (Sources/UI/AlbumWallpaperPalette.swift and the accent from
// NowPlayingView). The 3x3 HSB grid from the artwork becomes five evenly
// spaced RGB stops shared by the colour field and the visualiser.

import { FALLBACK_HSB } from "../defaults";
import { smoothstep } from "../timing/math";

export type RGB = [number, number, number];
export type HSB = [number, number, number];

export const SAMPLE_INDICES = [0, 2, 4, 6, 8] as const;

/** AlbumWallpaperPalette's HSB → sRGB (channel offsets 5, 3, 1). */
export function hsbToRgb([h, s, v]: HSB): RGB {
  const hue = h * 6;
  const channel = (offset: number) => {
    const k = (hue + offset) % 6;
    return v * (1 - s * Math.max(0, Math.min(k, 4 - k, 1)));
  };
  return [channel(5), channel(3), channel(1)];
}

function cell(grid: readonly HSB[], index: number): HSB {
  const value = grid[index];
  if (value && value.length === 3 && value.every(Number.isFinite)) return value;
  return FALLBACK_HSB[index];
}

/** The five wallpaper stops (grid cells 0, 2, 4, 6, 8). */
export function albumStops(grid: readonly HSB[]): RGB[] {
  return SAMPLE_INDICES.map((i) => hsbToRgb(cell(grid, i)));
}

export function interpolateStops(from: readonly RGB[], to: readonly RGB[], fraction: number): RGB[] {
  const t = Math.min(1, Math.max(0, fraction));
  return to.map((end, i) => {
    const start = from[i] ?? end;
    return [start[0] + (end[0] - start[0]) * t, start[1] + (end[1] - start[1]) * t, start[2] + (end[2] - start[2]) * t];
  });
}

export function averageStop(stops: readonly RGB[]): RGB {
  const sum: RGB = [0, 0, 0];
  for (const s of stops) {
    sum[0] += s[0];
    sum[1] += s[1];
    sum[2] += s[2];
  }
  const n = Math.max(1, stops.length);
  return [sum[0] / n, sum[1] / n, sum[2] / n];
}

export function cssRgb([r, g, b]: RGB, alpha = 1): string {
  const c = (v: number) => Math.round(Math.min(1, Math.max(0, v)) * 255);
  return alpha >= 1 ? `rgb(${c(r)},${c(g)},${c(b)})` : `rgba(${c(r)},${c(g)},${c(b)},${Math.max(0, alpha).toFixed(3)})`;
}

/** AlbumWallpaperPalette.cssGradient. */
export function cssGradient(stops: readonly RGB[]): string {
  return `linear-gradient(120deg, ${stops.map((s) => cssRgb(s)).join(", ")})`;
}

/** NowPlayingView.lyricAccent: grid cell 2, saturation 0.2–0.65, brightness >= 0.78. */
export function accentColour(grid: readonly HSB[]): RGB {
  const [h, s, b] = cell(grid, 2);
  return hsbToRgb([h, Math.min(0.65, Math.max(0.2, s)), Math.max(0.78, b)]);
}

/** Mean saturation of the grid (NowPlayingView.artworkSaturation). */
export function artworkSaturation(grid: readonly HSB[]): number {
  if (grid.length === 0) return 0.5;
  let sum = 0;
  for (const c of grid) sum += Number.isFinite(c[1]) ? c[1] : 0.5;
  return sum / grid.length;
}

/** The 0.9 s smoothstep crossfade between two palettes (NowPlayingView.wallpaperPalette). */
export class PaletteFade {
  private from: RGB[] | null = null;
  private to: RGB[];
  private changedAt = -Infinity;
  private key = "";

  constructor(grid: readonly HSB[]) {
    this.to = albumStops(grid);
    this.key = JSON.stringify(this.to);
  }

  set(grid: readonly HSB[], now: number): void {
    const next = albumStops(grid);
    const key = JSON.stringify(next);
    if (key === this.key) return;
    this.from = this.at(now);
    this.to = next;
    this.key = key;
    this.changedAt = now;
  }

  /** `now` in milliseconds. */
  at(now: number): RGB[] {
    const progress = smoothstep((now - this.changedAt) / 900);
    if (progress >= 1 || !this.from) return this.to;
    return interpolateStops(this.from, this.to, progress);
  }

  settled(now: number): boolean {
    return !this.from || now - this.changedAt >= 900;
  }
}
