// Line-level timing: the active-line index (LyricLineIndex.swift), phrase
// entrance/exit (WordPresentation.swift `LyricPhraseTiming`), vocal windows
// (LyricGapTiming.swift) and reading pairs (`LyricReadingPairs`).

import type { Line, Word } from "../contract";
import type { VocalWindow } from "./geometry";
import { hasLetterOrNumber, smoothstep } from "./math";
import { timedWordKey } from "./seed";

/** Binary search over ordered line starts; the old forward scan for malformed timelines. */
export class LineIndex {
  private readonly starts: Float64Array;
  private readonly ordered: boolean;

  constructor(lines: readonly Pick<Line, "start">[]) {
    this.starts = new Float64Array(lines.length);
    let ordered = true;
    for (let i = 0; i < lines.length; i++) {
      const start = lines[i].start;
      this.starts[i] = start;
      if (!Number.isFinite(start) || (i > 0 && !(this.starts[i - 1] <= start))) ordered = false;
    }
    this.ordered = ordered;
  }

  /** Index of the last line whose start is <= time, or null. */
  line(time: number): number | null {
    const starts = this.starts;
    if (!this.ordered) {
      // Retain the old early-stop rule for malformed imported timelines.
      let found: number | null = null;
      for (let i = 0; i < starts.length; i++) {
        if (starts[i] <= time) found = i;
        else break;
      }
      return found;
    }
    let low = 0;
    let high = starts.length;
    while (low < high) {
      const middle = low + ((high - low) >> 1);
      if (starts[middle] <= time) low = middle + 1;
      else high = middle;
    }
    return low === 0 ? null : low - 1;
  }
}

export interface PhraseTiming {
  progress: number;
  exit: number;
  presence: number;
}

/** `LyricPhraseTiming(line:nextStart:time:)` — line-only lyrics animate as one phrase. */
export function phraseTiming(line: Pick<Line, "start" | "end" | "wordTimed">, nextStart: number, time: number): PhraseTiming {
  const duration = Math.max(0, Math.min(line.end, nextStart) - line.start);
  const attack = line.wordTimed ? 0.22 : Math.min(0.32, Math.max(0.12, duration * 0.18));
  const progress = smoothstep((time - line.start) / attack);
  const exit = smoothstep((time - Math.min(line.end + 0.12, nextStart - 0.08)) / 0.46);
  return { progress, exit, presence: progress * (1 - exit) };
}

/** The next line's start, or the line end + 0.8 for the last line (renderLyricLine). */
export function nextStartAfter(lines: readonly Line[], index: number): number {
  return index + 1 < lines.length ? lines[index + 1].start : lines[index].end + 0.8;
}

/** `LyricPhraseTiming.firstWordOnset`: the real onset of the first sung word, or the line start. */
export function firstWordOnset(line: Line): number | null {
  if (line.isInstrumental) return null;
  const first = line.words.find((w) => hasLetterOrNumber(w.text));
  if (!first) return null;
  return line.wordTimed ? first.start : line.start;
}

/** `LyricPhraseTiming.rollingRange`: rolling words stay inside their run of real word timing. */
export function rollingRange(index: number, lines: readonly Pick<Line, "wordTimed">[]): [number, number] {
  let first = index;
  let last = index;
  while (first > Math.max(0, index - 2) && lines[first - 1].wordTimed) first -= 1;
  if (last + 1 < lines.length && lines[last + 1].wordTimed) last += 1;
  return [first, last];
}

/** `LyricGapTiming.windows`: where vocals actually happen, sorted by start. */
export function vocalWindows(lines: readonly Line[]): VocalWindow[] {
  const result: VocalWindow[] = [];
  for (const line of lines) {
    if (line.isInstrumental || line.words.length === 0) continue;
    let start = line.start;
    let end = line.end;
    if (line.wordTimed) {
      let min = Infinity;
      let max = -Infinity;
      for (const w of line.words) {
        if (w.start < min) min = w.start;
        if (w.end > max) max = w.end;
      }
      start = min;
      end = max;
    }
    // Estimated words do not supply an authoritative vocal end.
    if (!Number.isFinite(start) || !Number.isFinite(end) || start < 0 || !(end > start)) continue;
    result.push({ start, end });
  }
  return result.sort((a, b) => a.start - b.start);
}

/** LyricsViewModel `wordsPerSecond`, used for the track mood. */
export function wordsPerSecond(lines: readonly Line[]): number {
  if (lines.length === 0) return 0;
  const span = lines[lines.length - 1].end - lines[0].start;
  if (!(span > 1)) return 0;
  let words = 0;
  for (const line of lines) words += line.words.length;
  return words / span;
}

/** `LyricReadingPairs.match`: reading text per timed word where it differs from the word. */
export function readingPairs(readings: readonly Word[] | null | undefined, words: readonly Word[]): Map<string, string> {
  const result = new Map<string, string>();
  if (!readings || readings.length !== words.length) return result;
  for (let i = 0; i < words.length; i++) {
    const word = words[i];
    const reading = readings[i];
    if (reading.text !== word.text && reading.text.length > 0) {
      result.set(timedWordKey(word.text, word.start, word.end), reading.text);
    }
  }
  return result;
}
