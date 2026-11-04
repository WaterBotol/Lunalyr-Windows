// Screen metrics and stage fitting (Sources/UI/LyricStageFit.swift and
// SpokenTypographyView.swift `WordStageFit`). Sizes are CSS pixels, which
// play the role of macOS points.

export interface Size {
  width: number;
  height: number;
}

export interface ViewportMetrics {
  size: Size;
  /** Nominal lyric point size for this screen. */
  baseSize: number;
  /** The free area lyrics are fitted into (80% x 72% of the screen). */
  stage: Size;
}

export function viewportMetrics(size: Size): ViewportMetrics {
  const baseSize = Math.min(124, Math.max(24, Math.min(size.width, (size.height * 16) / 9) * 0.046));
  return { size, baseSize, stage: { width: size.width * 0.8, height: size.height * 0.72 } };
}

export function previewScale(desktop: Size, box: Size): number {
  if (!(desktop.width > 0) || !(desktop.height > 0)) return 0;
  return Math.min(box.width / desktop.width, box.height / desktop.height);
}

function validSize(s: Size): boolean {
  return Number.isFinite(s.width) && Number.isFinite(s.height) && s.width > 0 && s.height > 0;
}

/** `LyricStageFit.scale`: shrink to fit, never enlarge. */
export function stageFitScale(content: Size, available: Size): number {
  if (!validSize(content) || !validSize(available)) return 1;
  return Math.min(1, available.width / content.width, available.height / content.height);
}

/**
 * `LyricStageFit.presentationScale`: fit inside 75% of the stage, apply the
 * user's lyric size, then never exceed the stage itself. The Mac clamps the
 * user scale to 0.7–1.5; the Windows setting spans 0.5–2, so the clamp
 * follows the setting's range.
 */
export function presentationScale(content: Size, available: Size, userScale: number, low = 0.5, high = 2): number {
  const neutral = stageFitScale(content, { width: available.width * 0.75, height: available.height * 0.75 });
  const user = Number.isFinite(userScale) ? userScale : 1;
  const requested = neutral * Math.min(high, Math.max(low, user));
  if (!Number.isFinite(content.width) || !Number.isFinite(content.height) || !(available.width > 0) || !(available.height > 0) ||
      !(content.width > 0) || !(content.height > 0)) return requested;
  return Math.min(requested, available.width / content.width, available.height / content.height);
}

/** `WordStageFit.scale`: fit a rotated, scaled word around its intended centre inside the stage. */
export function wordStageFit(ink: Size, stage: Size, centre: { x: number; y: number }, scale: number, angle: number, margin: number): number {
  if (!Number.isFinite(ink.width) || !Number.isFinite(ink.height) || !Number.isFinite(stage.width) || !Number.isFinite(stage.height) ||
      !Number.isFinite(centre.x) || !Number.isFinite(centre.y) || !Number.isFinite(scale) || !Number.isFinite(angle) || !Number.isFinite(margin) ||
      !(ink.width > 0) || !(ink.height > 0) || !(stage.width > 0) || !(stage.height > 0) || !(scale > 0)) return 0;
  const radians = (angle * Math.PI) / 180;
  const cosine = Math.abs(Math.cos(radians));
  const sine = Math.abs(Math.sin(radians));
  const rotatedWidth = ink.width * cosine + ink.height * sine;
  const rotatedHeight = ink.height * cosine + ink.width * sine;
  const inset = Math.max(0, margin);
  const clearanceX = Math.max(0, Math.min(centre.x, stage.width - centre.x) - inset);
  const clearanceY = Math.max(0, Math.min(centre.y, stage.height - centre.y) - inset);
  return Math.min(1, (clearanceX * 2) / Math.max(1, rotatedWidth * scale), (clearanceY * 2) / Math.max(1, rotatedHeight * scale));
}
