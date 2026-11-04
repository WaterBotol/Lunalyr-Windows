// Shared types for the lyric views.

import type { FontChoice, ReadingDisplay } from "../../contract";
import type { TrackMood } from "../../timing/mood";
import type { ViewportMetrics } from "../../timing/stage";
import type { Measurer } from "../dom";

/** Fixed for the lifetime of a built view (a change rebuilds it). */
export interface BuildEnv {
  metrics: ViewportMetrics;
  face: FontChoice;
  mood: TrackMood;
  readings: ReadingDisplay;
  measurer: Measurer;
  layer: HTMLElement;
}

/** Per-frame inputs shared by every view. */
export interface FrameEnv {
  time: number;
  reduced: boolean;
  userScale: number;
  /** CSS colour of the album accent. */
  accent: string;
}

/** SwiftUI shadow radius → CSS blur radius (CALayer radius is about half the CSS blur). */
export function shadowBlur(radius: number): number {
  return radius * 2;
}

export function shadow(color: string, radius: number, y = 0): string {
  return `0 ${y}px ${Math.round(shadowBlur(radius) * 10) / 10}px ${color}`;
}

export function white(alpha: number): string {
  return `rgba(255,255,255,${Math.round(Math.min(1, Math.max(0, alpha)) * 1000) / 1000})`;
}

export function black(alpha: number): string {
  return `rgba(0,0,0,${Math.round(Math.min(1, Math.max(0, alpha)) * 1000) / 1000})`;
}

export function quantize(v: number, step = 0.02): number {
  return Math.round(v / step) * step;
}
