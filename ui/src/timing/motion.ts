// Exact ports of Sources/Support/LyricMotion.c. Every function is a pure
// function of its arguments (seek-safe, no frame history). Poses can be
// written into a caller-owned object so the frame loop does not allocate.
//
// Coordinates are in lyric point-size units, independent of screen pixels.

import { cClamp, cSmooth } from "./math";
import type { Seed64 } from "./seed";

export interface WordPose {
  x: number;
  y: number;
  scale: number;
  angle: number;
  opacity: number;
  blur: number;
}

export function newPose(): WordPose {
  return { x: 0, y: 0, scale: 1, angle: 0, opacity: 1, blur: 0 };
}

function setPose(out: WordPose, x: number, y: number, scale: number, angle: number, opacity: number, blur: number): WordPose {
  out.x = x;
  out.y = y;
  out.scale = scale;
  out.angle = angle;
  out.opacity = opacity;
  out.blur = blur;
  return out;
}

const EASE_ENDPOINT = 1 - Math.exp(-7) * (Math.cos(10) + 0.7 * Math.sin(10));

/** `vc_motion_ease`: damped arrival with a small overshoot (max 1.12). */
export function motionEase(progress: number): number {
  const t = cClamp(progress, 0, 1);
  if (t === 0 || t === 1) return t;
  const value = 1 - Math.exp(-7 * t) * (Math.cos(10 * t) + 0.7 * Math.sin(10 * t));
  return cClamp(value / EASE_ENDPOINT, 0, 1.12);
}

/** `vc_motion_handoff`: the whole cluster follows each onset (smootherstep). */
export function motionHandoff(age: number, cadence: number): number {
  if (!Number.isFinite(age) || age <= 0) return 0;
  const duration = cClamp(cadence * 0.82, 0.2, 0.58);
  const t = cClamp(age / duration, 0, 1);
  return t * t * t * (10 + t * (-15 + 6 * t));
}

/** Rolling styles: 0 = Spoken, 1 = Cascade, 2 = Cutout. */
export type RollingStyle = 0 | 1 | 2;

/** `vc_rolling_pose`. `seed` is the word's FNV hash (`seed % 1021` sets its phase). */
export function rollingPose(
  style: RollingStyle,
  rankIn: number,
  ageIn: number,
  durationIn: number,
  seed: Seed64,
  energyIn: number,
  reduced: boolean,
  out: WordPose = newPose(),
): WordPose {
  const rank = cClamp(rankIn, -2, 9);
  const age = cClamp(ageIn, -60, 3600);
  const duration = cClamp(durationIn, 0.06, 30);
  const energy = cClamp(energyIn, 0, 1);
  const h = Math.max(0, rank);
  const phase = (seed.mod(1021) / 1021) * 6.2831853;
  const focus = Math.exp(-rank * rank * 2.4);
  const release = 1 - cSmooth((age - duration - 0.45) / 1.0);
  const fade = (1 - cSmooth((h - 2.4) / 1.7)) * release;
  // Extra clearance around the hero, then a tighter historical tail.
  const separation = 0.66 * rank + (style === 2 ? 0.86 : 0.7) * Math.tanh(rank * 1.2);
  let x = 0;
  let y = 0;
  let scale = 1;
  let angle = 0;
  let blur = 0;
  if (style === 0) {
    // Hero word, then a loose fan of receding fragments.
    x = Math.sin(h * 0.92 + phase * 0.22) * Math.min(h, 1) * 0.7;
    y = -separation;
    scale = 0.7 + 0.34 * focus;
    angle = Math.sin(phase + h * 0.6) * Math.min(h, 1) * 2.4;
  } else if (style === 1) {
    // Uneven, flowing column: differences in type do the emphasis.
    x = Math.sin(rank * 0.72 + phase * 0.12) * 0.28;
    y = 0.35 - separation;
    scale = 0.94 + 0.1 * focus;
    angle = Math.sin(phase) * 1.3 * (1 - focus);
  } else {
    // Labels travel into a loose collage rather than a masked ticker.
    x = Math.sin(rank * 0.72 + phase * 0.25) * 0.53;
    y = 0.3 - separation;
    scale = 0.94 + 0.1 * focus;
    angle = Math.sin(phase + rank * 0.28) * (1.6 + 2.5 * (1 - focus));
  }
  if (!reduced && age >= 0) {
    const sustain = cSmooth((duration - 0.35) / 1.4);
    const vocalArc = Math.sin(cClamp(age / duration, 0, 1) * 3.141592653589793);
    const breath = Math.sin(age * (2.0 + energy * 1.8) + phase) * sustain;
    y += breath * 0.055 * focus;
    scale += focus * (sustain * vocalArc * 0.12 + energy * 0.07);
    angle += Math.sin(age * 2.6 + phase) * energy * 0.75 * focus;
    blur = cSmooth((h - 2.4) / 1.7) * 1.6;
  }
  if (reduced) {
    angle = 0;
    blur = 0;
  }
  return setPose(out, x, y, scale, angle, fade, blur);
}

/** `vc_vocal_gesture`: the small lift and swell of a word while it is sung. */
export function vocalGesture(
  age: number,
  durationIn: number,
  seed: Seed64,
  energyIn: number,
  reduced: boolean,
  out: WordPose = newPose(),
): WordPose {
  setPose(out, 0, 0, 1, 0, 1, 0);
  if (reduced || !Number.isFinite(age) || age < 0) return out;
  const duration = cClamp(durationIn, 0.06, 30);
  const energy = cClamp(energyIn, 0, 1);
  const heat = 1 - cSmooth((age - duration) / 0.65);
  if (heat <= 0) return out;
  const phase = (seed.mod(997) / 997) * 6.2831853;
  const held = cSmooth((duration - 0.25) / 1.1);
  const arrive = cSmooth(age / 0.18);
  const vocalArc = Math.sin(cClamp(age / duration, 0, 1) * 3.141592653589793);
  const wind = Math.sin(age * (2.2 + energy) + phase) * held;
  out.x = Math.sin(age * 1.65 + phase) * 0.025 * held * heat * arrive;
  out.y = (-0.13 * held * vocalArc + wind * 0.055 - energy * 0.035) * heat * arrive;
  out.scale = 1 + (held * vocalArc * 0.12 + energy * 0.045) * heat * arrive;
  out.angle = wind * 1.2 * heat * arrive;
  return out;
}
