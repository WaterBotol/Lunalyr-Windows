// Word preparation for both lyric renderers, ported from
// Sources/UI/LyricTypography.swift (`ScatteredTypography`, `SpokenTypography`)
// and the row layout from ScatteredLyricText.swift (`FlowLayout`,
// `LyricLineStyle`). Pure: no DOM, so it is unit-tested in Node.

import type { FontChoice, Word } from "../contract";
import { hasConnections, hasEditorialTypography, type ConcreteComposition } from "../timing/composition";
import { graphemes, hasLetterOrNumber, smoothstep } from "../timing/math";
import type { TrackMood } from "../timing/mood";
import { fnvString, Seed64, timedWordKey } from "../timing/seed";
import { lyricFont, type FontSpec } from "./fonts";

export type RowAlignment = "leading" | "center" | "trailing";

/** `LyricLineStyle.forLine`: cycling settings that re-compose consecutive lines. */
export interface LineStyle {
  rowAlignment: RowAlignment;
  wave: number;
  rotationSpread: number;
  sizeContrast: number;
  uppercaseHeroes: boolean;
}

export function lineStyle(index: number): LineStyle {
  switch (Math.abs(index) % 5) {
    case 0:
      return { rowAlignment: "center", wave: 0, rotationSpread: 1.3, sizeContrast: 0.9, uppercaseHeroes: true };
    case 1:
      return { rowAlignment: "leading", wave: 0.09, rotationSpread: 0.5, sizeContrast: 0.45, uppercaseHeroes: false };
    case 2:
      return { rowAlignment: "center", wave: 0.15, rotationSpread: 2.1, sizeContrast: 1.0, uppercaseHeroes: true };
    case 3:
      return { rowAlignment: "trailing", wave: -0.1, rotationSpread: 0.9, sizeContrast: 0.6, uppercaseHeroes: false };
    default:
      return { rowAlignment: "center", wave: 0.06, rotationSpread: 0, sizeContrast: 0.25, uppercaseHeroes: true };
  }
}

export interface Reading {
  text: string;
  font: FontSpec;
}

export interface ScatteredToken {
  index: number;
  source: Word;
  key: string;
  size: number;
  isHero: boolean;
  /** Degrees. */
  rotation: number;
  waveOffset: number;
  uppercased: boolean;
  alternateFace: boolean;
  label: string;
  characters: string[];
  font: FontSpec;
  reading: Reading | null;
  /** How long the syllable is held, 0..1. */
  sustain: number;
  motionSeed: Seed64;
}

export interface ScatteredKey {
  words: readonly Word[];
  baseSize: number;
  style: LineStyle;
  mood: TrackMood;
  composition: ConcreteComposition;
  /** Constellation spread (Mac default 1). */
  spread: number;
  face: FontChoice;
  readings: ReadonlyMap<string, string>;
  wordTimed: boolean;
}

const FUNCTION_WORDS = new Set([
  "a", "an", "the", "and", "or", "but", "so", "if", "as", "of", "to", "in",
  "on", "at", "by", "for", "with", "from", "up", "out", "is", "am", "are",
  "was", "were", "be", "been", "do", "did", "it", "its", "i", "im", "you",
  "youre", "me", "my", "we", "he", "she", "they", "them", "this", "that",
  "there", "here", "just", "got", "gonna", "wanna",
]);

const EDGE_PUNCTUATION = /^[\p{P}\p{Zs}\t]+|[\p{P}\p{Zs}\t]+$/gu;

/** How hard the singer leans on a word, from its length against the line's average. */
function drag(word: Word, average: number): number {
  const cleaned = word.text.toLowerCase().replace(EDGE_PUNCTUATION, "");
  if (graphemes(cleaned).length <= 2 || FUNCTION_WORDS.has(cleaned) || !(average > 0.01)) return 0;
  return smoothstep(((word.end - word.start) / average - 1.15) / 0.85);
}

const GOLDEN = 2654435761;

/** `ScatteredTypography.prepare`. Punctuation-only tokens are dropped. */
export function prepareScattered(key: ScatteredKey): ScatteredToken[] {
  const words = key.words.filter((w) => hasLetterOrNumber(w.text));
  const durations = words.map((w) => w.end - w.start).filter((d) => d > 0.01);
  const average = durations.length === 0 ? 0.3 : durations.reduce((a, b) => a + b, 0) / durations.length;
  const fullness = Math.min(words.length / 7, 1);
  const connections = hasConnections(key.composition);
  const editorialAmount = hasEditorialTypography(key.composition) ? 1 : 0;
  return words.map((word, index) => {
    const hash = fnvString(word.text);
    const value = hash.xor(Seed64.fromNumber(index * GOLDEN));
    const held = key.wordTimed ? drag(word, average) : 0;
    const isHero = held > 0.25;
    const jitter = value.div(100).mod(5) / 5;
    let size = isHero
      ? key.baseSize * (0.58 + 0.3 * key.style.sizeContrast * held + 0.05 * jitter)
      : key.baseSize * (0.64 + 0.06 * jitter);
    let alternate = false;
    if (editorialAmount > 0) {
      size *= 1 + 0.1 * editorialAmount * (isHero ? 1 : jitter);
      const eligible = key.composition === "constellationEditorial" ? index % 2 === 0 : value.div(10_000).mod(3) === 0;
      alternate = !isHero && eligible && value.div(700_000).mod(100) < editorialAmount * 100;
    }
    let offset = Math.sin(index * 0.85) * key.style.wave * key.baseSize * key.mood.intensity;
    if (connections) {
      const spread = value.div(7).mod(100) / 100 - 0.5;
      const reach = (0.3 + 0.7 * fullness) * key.spread;
      offset += (spread * key.baseSize * 2.1 + Math.sin(index * 2.3) * key.baseSize * 0.42) * reach;
      size *= isHero ? 1.06 : 0.96;
    }
    size = Math.max(size, 11);
    const rotation = (value.div(1000).mod(9) - 4) * 0.35 * key.style.rotationSpread * key.mood.intensity;
    const uppercased = isHero && key.style.uppercaseHeroes;
    const label = uppercased ? word.text.toUpperCase() : word.text;
    const wordKey = timedWordKey(word.text, word.start, word.end);
    const readingText = key.readings.get(wordKey);
    return {
      index,
      source: word,
      key: wordKey,
      size,
      isHero,
      rotation,
      waveOffset: offset,
      uppercased,
      alternateFace: alternate,
      label,
      characters: graphemes(label),
      font: lyricFont(key.face, size, isHero, alternate),
      reading: readingText ? { text: readingText, font: lyricFont(key.face, size * 0.78, isHero, alternate) } : null,
      sustain: smoothstep((word.end - word.start - 0.45) / 0.85),
      motionSeed: hash,
    };
  });
}

export interface SpokenToken {
  index: number;
  source: Word;
  key: string;
  hash: Seed64;
  hero: boolean;
  pointSize: number;
  /** Cutout's album-colour label behind the word. */
  tile: boolean;
  font: FontSpec;
  reading: Reading | null;
}

const FILLERS = new Set(["a", "an", "the", "i", "in", "of", "to", "and", "or", "it", "is", "my", "me", "you"]);

/** `SpokenTypography.prepare`: identity, size and face depend only on the word, never its window position. */
export function prepareSpokenWord(word: Word, index: number, baseSize: number, composition: ConcreteComposition, face: FontChoice, readings: ReadonlyMap<string, string>): SpokenToken {
  const voice = Array.from(word.text.toLowerCase()).filter((c) => /\p{L}/u.test(c)).join("");
  const filler = FILLERS.has(voice);
  const hash = fnvString(word.text);
  const hero = !filler && (graphemes(word.text).length >= 5 || word.end - word.start > 0.65);
  const columnSize = baseSize * (hero ? 0.88 : filler ? 0.62 : 0.74);
  const pointSize = composition === "spoken" ? baseSize * 1.1 : columnSize;
  const heavy = hero || composition === "spoken";
  const key = timedWordKey(word.text, word.start, word.end);
  const readingText = readings.get(key);
  return {
    index,
    source: word,
    key,
    hash,
    hero,
    pointSize,
    tile: composition === "cutout" && (hero || hash.mod(4) === 1),
    font: lyricFont(face, pointSize, heavy),
    reading: readingText ? { text: readingText, font: lyricFont(face, pointSize * 0.78, heavy) } : null,
  };
}

export function prepareSpoken(words: readonly Word[], baseSize: number, composition: ConcreteComposition, face: FontChoice, readings: ReadonlyMap<string, string> = new Map()): SpokenToken[] {
  return words.map((word, index) => prepareSpokenWord(word, index, baseSize, composition, face, readings));
}

// MARK: FlowLayout

export interface FlowItem {
  width: number;
  height: number;
}

export interface FlowPlacement {
  /** Centre of each item's reserved slot. */
  centres: { x: number; y: number }[];
  width: number;
  height: number;
}

/** `FlowLayout`: wraps items into rows at `maxWidth`, aligning each row; RTL mirrors the order. */
export function flowLayout(items: readonly FlowItem[], maxWidth: number, spacing: number, lineSpacing: number, alignment: RowAlignment, rtl = false): FlowPlacement {
  const rows: { items: number[]; width: number; height: number }[] = [];
  let current = { items: [] as number[], width: 0, height: 0 };
  items.forEach((size, index) => {
    const projected = current.items.length === 0 ? size.width : current.width + spacing + size.width;
    if (projected > maxWidth && current.items.length > 0) {
      rows.push(current);
      current = { items: [], width: 0, height: 0 };
    }
    current.width = current.items.length === 0 ? size.width : current.width + spacing + size.width;
    current.height = Math.max(current.height, size.height);
    current.items.push(index);
  });
  if (current.items.length > 0) rows.push(current);
  const width = rows.reduce((m, r) => Math.max(m, r.width), 0);
  const height = rows.reduce((sum, r) => sum + r.height, 0) + lineSpacing * Math.max(rows.length - 1, 0);
  const centres = items.map(() => ({ x: 0, y: 0 }));
  let y = 0;
  for (const row of rows) {
    let x = alignment === "leading" ? 0 : alignment === "center" ? width / 2 - row.width / 2 : width - row.width;
    for (const index of row.items) {
      const item = items[index];
      const cx = x + item.width / 2;
      centres[index] = { x: rtl ? width - cx : cx, y: y + row.height / 2 };
      x += item.width + spacing;
    }
    y += row.height + lineSpacing;
  }
  return { centres, width, height };
}
