// `WordEntrance` (ScatteredLyricText.swift) as numbers, plus helpers that
// turn transforms into CSS strings. Kept pure for tests and reuse by every
// view.

import { isCutoutArrival, type WordArrival } from "../timing/composition";
import { motionEase } from "../timing/motion";

export interface Entrance {
  opacity: number;
  blur: number;
  x: number;
  y: number;
  /** Degrees about the bottom edge (unfold / cutout fold). */
  rotateX: number;
  rotate: number;
  scale: number;
  /** Visible fraction of the paper mask, or 1 when unmasked. */
  reveal: number;
  /** Mask anchored at the leading (true) or trailing edge. */
  fromLeading: boolean;
}

export function newEntrance(): Entrance {
  return { opacity: 1, blur: 0, x: 0, y: 0, rotateX: 0, rotate: 0, scale: 1, reveal: 1, fromLeading: true };
}

export function wordEntrance(
  progress: number,
  style: WordArrival,
  size: number,
  direction: number,
  visibility: number | null,
  reduced: boolean,
  out: Entrance = newEntrance(),
): Entrance {
  const paper = isCutoutArrival(style);
  const eased = reduced ? 1 : paper ? progress : motionEase(progress);
  const remaining = 1 - eased;
  out.opacity = visibility ?? progress;
  out.blur = reduced ? 0 : Math.max(0, 1 - progress) * (style === "fade" ? 3.5 : 2.5);
  out.reveal = paper && eased < 1 ? 0.88 + 0.12 * eased : 1;
  out.fromLeading = direction >= 0;
  out.scale = !reduced && (style === "pop" || paper) ? 0.76 + 0.24 * eased : 1;
  out.rotate = !reduced && (style === "orbit" || style === "cutoutSlide") ? remaining * direction * 12 : 0;
  out.rotateX = !reduced && (style === "unfold" || style === "cutoutFold") ? (paper ? -30 : -48) * remaining : 0;
  out.x = !reduced && (style === "orbit" || style === "glide" || style === "cutoutSlide")
    ? remaining * direction * size * (style === "orbit" ? 0.32 : 0.72) : 0;
  out.y = !reduced && (style === "lift" || style === "glide" || style === "orbit")
    ? remaining * size * (style === "lift" ? 0.48 : 0.2) : 0;
  return out;
}

export function px(v: number): string {
  return `${Math.round(v * 100) / 100}px`;
}

function num(v: number, digits = 4): string {
  const f = 10 ** digits;
  return String(Math.round(v * f) / f);
}

/** CSS transform for an entrance; `height` anchors the 3D fold at the bottom edge. */
export function entranceTransform(e: Entrance, width: number, height: number): string {
  let t = "";
  if (e.x !== 0 || e.y !== 0) t += `translate(${px(e.x)},${px(e.y)}) `;
  if (e.rotateX !== 0) {
    // SwiftUI perspective 0.25: vanishing distance four times the larger side.
    const distance = Math.max(width, height, 1) * 4;
    t += `translateY(${px(height / 2)}) perspective(${px(distance)}) rotateX(${num(-e.rotateX, 3)}deg) translateY(${px(-height / 2)}) `;
  }
  if (e.rotate !== 0) t += `rotate(${num(e.rotate, 3)}deg) `;
  if (e.scale !== 1) t += `scale(${num(e.scale)}) `;
  return t.length ? t.trimEnd() : "none";
}

/** clip-path for the paper (cutout) mask. */
export function entranceClip(e: Entrance): string {
  if (e.reveal >= 1) return "none";
  const hidden = `${num((1 - e.reveal) * 100, 3)}%`;
  return e.fromLeading ? `inset(0 ${hidden} 0 0)` : `inset(0 0 0 ${hidden})`;
}

export function blurFilter(radius: number): string {
  return radius > 0.05 ? `blur(${num(radius, 2)}px)` : "none";
}

export function opacityValue(v: number): string {
  return num(Math.min(1, Math.max(0, v)), 3);
}

export { num };
