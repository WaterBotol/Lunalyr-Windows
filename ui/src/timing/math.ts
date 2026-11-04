// Small numeric helpers shared by every timing port.
//
// Two different "clamp" semantics exist in the macOS sources and both are
// kept, because the golden tests compare against the original C:
//  - `cClamp` is LyricMotion.c's `clamp`: a non-finite input (NaN or ±Inf)
//    returns `low`.
//  - `smoothstep` is Swift's `Easing.smoothstep`: clamped 3x²-2x³. Swift
//    propagates NaN there; this port returns 0 instead so a bad timestamp can
//    never poison a style value.

export function cClamp(x: number, low: number, high: number): number {
  return Number.isFinite(x) ? Math.min(high, Math.max(low, x)) : low;
}

/** LyricMotion.c `smooth`: clamp to 0..1 (non-finite → 0), then 3x²-2x³. */
export function cSmooth(x: number): number {
  const t = cClamp(x, 0, 1);
  return t * t * (3 - 2 * t);
}

/** Swift `Easing.smoothstep`. */
export function smoothstep(x: number): number {
  if (Number.isNaN(x)) return 0;
  const t = x < 0 ? 0 : x > 1 ? 1 : x;
  return t * t * (3 - 2 * t);
}

export function clamp(x: number, low: number, high: number): number {
  return x < low ? low : x > high ? high : x;
}

export function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

/** True when the text contains a letter or a number (Swift `isLetter || isNumber`). */
const LETTER_OR_NUMBER = /[\p{L}\p{N}]/u;
export function hasLetterOrNumber(text: string): boolean {
  return LETTER_OR_NUMBER.test(text);
}

/** Grapheme clusters (Swift `Character`s), falling back to code points. */
type SegmenterLike = { segment(input: string): Iterable<{ segment: string }> };
let graphemeSegmenter: SegmenterLike | null | undefined;
export function graphemes(text: string): string[] {
  if (graphemeSegmenter === undefined) {
    const Segmenter = (Intl as unknown as { Segmenter?: new (locale?: string, options?: { granularity: string }) => SegmenterLike }).Segmenter;
    graphemeSegmenter = Segmenter ? new Segmenter(undefined, { granularity: "grapheme" }) : null;
  }
  if (!graphemeSegmenter) return Array.from(text);
  const out: string[] = [];
  for (const part of graphemeSegmenter.segment(text)) out.push(part.segment);
  return out;
}
