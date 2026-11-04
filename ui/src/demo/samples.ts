// Built-in sample material for the browser demo. All lyrics here are
// original text written for Lunalyr's demo (the short Japanese, Mandarin,
// Arabic and Korean phrases are everyday greetings and images), so the
// harness ships no third-party song lyrics.

import type { Line, Word } from "../contract";
import { FALLBACK_HSB } from "../defaults";
import { graphemes } from "../timing/math";

export interface DemoSong {
  id: "word" | "line";
  title: string;
  artist: string;
  album: string;
  duration: number;
  lines: Line[];
  /** Reading words per line (same count as the line's words), when the line has one. */
  readings: (Word[] | null)[];
  precision: string;
}

type WordSpec = [text: string, duration: number, gapAfter?: number];

function timedLine(start: number, specs: WordSpec[], joiner = " ", readings?: string[]): { line: Line; readings: Word[] | null } {
  const words: Word[] = [];
  let cursor = start;
  for (const [text, duration, gap = 0.06] of specs) {
    words.push({ text, start: round(cursor), end: round(cursor + duration) });
    cursor += duration + gap;
  }
  const line: Line = {
    start,
    end: words[words.length - 1].end,
    text: specs.map((s) => s[0]).join(joiner),
    words,
    isInstrumental: false,
    wordTimed: true,
  };
  const reading = readings ? words.map((w, i) => ({ text: readings[i] ?? w.text, start: w.start, end: w.end })) : null;
  return { line, readings: reading };
}

function round(value: number): number {
  return Math.round(value * 1000) / 1000;
}

type SegmenterLike = { segment(input: string): Iterable<{ segment: string; isWordLike?: boolean }> };

/** Word tokens like crates/lunalyr-core `word_tokens`: punctuation sticks to its word. */
export function wordTokens(text: string): string[] {
  const Segmenter = (Intl as unknown as { Segmenter?: new (l?: string, o?: { granularity: string }) => SegmenterLike }).Segmenter;
  const segments = Segmenter ? [...new Segmenter(undefined, { granularity: "word" }).segment(text)].map((s) => s.segment) : text.split(/(\s+)/);
  const tokens: string[] = [];
  let leading = "";
  for (const segment of segments) {
    if (/[\p{L}\p{N}]/u.test(segment)) {
      tokens.push((tokens.length === 0 ? leading : "") + segment);
      if (tokens.length === 1) leading = "";
    } else if (tokens.length > 0) {
      tokens[tokens.length - 1] += segment;
    } else {
      leading += segment;
    }
  }
  return tokens.map((t) => t.trim()).filter((t) => t.length > 0);
}

/** `distribute_words` (crates/lunalyr-core/src/timing.rs): estimated words for a line-timed line. */
export function distributeWords(text: string, start: number, end: number): Word[] {
  const tokens = wordTokens(text);
  if (tokens.length === 0) return [];
  const window = Math.max(end - start, 0.3);
  const usable = Math.min(window, Math.max(tokens.length * 0.42, window * 0.88));
  const weights = tokens.map((t) => graphemes(t).length + 1.5);
  const total = weights.reduce((a, b) => a + b, 0);
  let cursor = start;
  return tokens.map((token, i) => {
    const slice = usable * (weights[i] / total);
    const word = { text: token, start: round(cursor), end: round(cursor + slice) };
    cursor += slice;
    return word;
  });
}

function lineTimed(rows: [start: number, text: string][], duration: number): Line[] {
  return rows.map(([start, text], i) => {
    const next = i + 1 < rows.length ? rows[i + 1][0] : duration;
    const words = wordTokens(text).length;
    const end = round(Math.min(next - 0.4, start + Math.max(2.4, words * 0.45 + 0.8)));
    return { start, end, text, words: distributeWords(text, start, end), isInstrumental: false, wordTimed: false };
  });
}

function wordSong(): DemoSong {
  const built = [
    timedLine(2.0, [["Light", 0.34], ["the", 0.18], ["paper", 0.4], ["lanterns", 0.62], ["on", 0.2], ["the", 0.16], ["river", 0.95]]),
    timedLine(6.2, [["every", 0.36], ["little", 0.34], ["flame", 0.55], ["remembers", 0.7], ["you", 1.1]]),
    timedLine(10.4, [["hold", 1.5], ["on", 0.35], ["tight,", 1.2]]),
    timedLine(14.4, [["we", 0.2], ["were", 0.22], ["made", 0.36], ["of", 0.16], ["summer", 0.6], ["and", 0.18], ["starlight", 1.3]]),
    timedLine(22.4, [["夜空", 0.6], ["に", 0.25], ["光る", 0.7], ["星", 1.2]], "", ["yozora", "ni", "hikaru", "hoshi"]),
    timedLine(26.4, [["carry", 0.42], ["me", 0.2], ["home", 0.9], ["through", 0.3], ["the", 0.16], ["quiet", 1.1]]),
    timedLine(30.6, [["晚安", 0.8], ["月亮", 1.2]], "", ["wǎn'ān", "yuèliang"]),
    timedLine(33.8, [["and", 0.2], ["the", 0.16], ["lanterns", 0.7], ["drift", 0.6], ["on…", 1.4]]),
  ];
  return {
    id: "word",
    title: "Paper Lanterns",
    artist: "The Demo Ensemble",
    album: "Sample Sessions",
    duration: 41,
    lines: built.map((b) => b.line),
    readings: built.map((b) => b.readings),
    precision: "Word-by-word",
  };
}

function lineSong(): DemoSong {
  const duration = 38;
  return {
    id: "line",
    title: "Northbound",
    artist: "The Demo Ensemble",
    album: "Sample Sessions",
    duration,
    lines: lineTimed([
      [1.6, "We drove until the morning found us"],
      [5.6, "Radio humming songs we never knew"],
      [9.6, "Every mile a little lighter"],
      [13.2, "مساء الخير يا طريق"],
      [16.6, "Northbound, northbound, into the blue"],
      [24.0, "Windows down and nowhere left to be"],
      [28.0, "안녕, 다시 만나요"],
      [31.6, "We drove until the morning found us"],
    ], duration),
    readings: [null, null, null, null, null, null, null, null],
    precision: "Line-synced",
  };
}

export const SONGS = { word: wordSong(), line: lineSong() } as const;

/** A word-timed song reduced to line timing (what a line-synced candidate would give). */
export function asLineTimed(song: DemoSong): Line[] {
  return song.lines.map((line) => ({ ...line, wordTimed: false, words: distributeWords(line.text, line.start, line.end) }));
}

export const PLAIN_LYRICS = {
  title: "Harbour Light",
  artist: "The Demo Ensemble",
  lines: [
    "Paper boats along the harbour wall",
    "carry every letter that I never sent",
    "",
    "the gulls are writing circles in the air",
    "and the tide keeps every promise that it meant",
    "",
    "so leave a light on in the window",
    "leave the door a little open too",
    "",
    "I'll follow the lamps along the harbour wall",
    "and the evening will carry me through",
  ],
};

export type HSB = [number, number, number];

export const PALETTES: Record<string, { label: string; hsb: HSB[]; fromArtwork: boolean }> = {
  fallback: { label: "No artwork", hsb: FALLBACK_HSB, fromArtwork: false },
  dusk: {
    label: "Dusk",
    fromArtwork: true,
    hsb: [[0.05, 0.72, 0.82], [0.08, 0.62, 0.72], [0.95, 0.5, 0.62], [0.75, 0.5, 0.48], [0.02, 0.76, 0.74], [0.86, 0.46, 0.56], [0.7, 0.55, 0.38], [0.1, 0.6, 0.86], [0.9, 0.5, 0.45]],
  },
  ocean: {
    label: "Ocean",
    fromArtwork: true,
    hsb: [[0.55, 0.72, 0.6], [0.5, 0.6, 0.5], [0.6, 0.66, 0.72], [0.48, 0.55, 0.45], [0.58, 0.8, 0.55], [0.52, 0.5, 0.66], [0.62, 0.6, 0.4], [0.45, 0.5, 0.6], [0.57, 0.7, 0.5]],
  },
  forest: {
    label: "Forest",
    fromArtwork: true,
    hsb: [[0.3, 0.55, 0.45], [0.25, 0.5, 0.55], [0.14, 0.55, 0.8], [0.36, 0.6, 0.35], [0.28, 0.65, 0.5], [0.42, 0.45, 0.42], [0.2, 0.5, 0.3], [0.33, 0.45, 0.6], [0.12, 0.4, 0.5]],
  },
  neon: {
    label: "Neon",
    fromArtwork: true,
    hsb: [[0.88, 0.85, 0.85], [0.8, 0.8, 0.7], [0.52, 0.8, 0.9], [0.72, 0.75, 0.55], [0.92, 0.9, 0.8], [0.6, 0.8, 0.75], [0.78, 0.7, 0.5], [0.5, 0.85, 0.8], [0.95, 0.8, 0.6]],
  },
  mono: {
    label: "Monochrome",
    fromArtwork: true,
    hsb: [[0.6, 0.08, 0.35], [0.6, 0.06, 0.5], [0.58, 0.1, 0.7], [0.6, 0.05, 0.25], [0.62, 0.08, 0.45], [0.6, 0.06, 0.6], [0.6, 0.04, 0.2], [0.6, 0.07, 0.4], [0.6, 0.05, 0.55]],
  },
};
