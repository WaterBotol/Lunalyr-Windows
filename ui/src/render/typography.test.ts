// Mirrors the typography tests in RenderingPerformanceTests.swift.
import { describe, expect, it } from "vitest";
import type { Word } from "../contract";
import { ALL_COMPOSITIONS, arrivalOnly, type ConcreteComposition } from "../timing/composition";
import { NEUTRAL_MOOD } from "../timing/mood";
import { timedWordKey } from "../timing/seed";
import { contrastingFace, lyricFont } from "./fonts";
import { flowLayout, lineStyle, prepareScattered, prepareSpoken, type ScatteredKey } from "./typography";

const words: Word[] = [
  { text: "Keep", start: 10, end: 10.3 },
  { text: "the", start: 10.3, end: 10.6 },
  { text: "light", start: 10.6, end: 13 },
  { text: "♪", start: 13, end: 13.1 },
];

function key(size = 44, spread = 1, composition: ConcreteComposition = "constellationEditorial", overrides: Partial<ScatteredKey> = {}): ScatteredKey {
  return { words, baseSize: size, style: lineStyle(0), mood: NEUTRAL_MOOD, composition, spread, face: "manrope", readings: new Map(), wordTimed: true, ...overrides };
}

describe("scattered typography", () => {
  it("every composition keeps small words legible at large scale", () => {
    for (const style of ALL_COMPOSITIONS) {
      const sizes = arrivalOnly(style)
        ? prepareSpoken(words, 220, style, "manrope").map((w) => w.pointSize)
        : prepareScattered(key(220, 1, style)).map((w) => w.size);
      expect(Math.min(...sizes), style).toBeGreaterThan(220 * 0.55);
      expect(Math.max(...sizes) / Math.max(1, Math.min(...sizes)), style).toBeLessThan(1.9);
    }
  });

  it("keeps punctuation filtering and held-word glyphs", () => {
    const prepared = prepareScattered(key());
    expect(prepared.length).toBe(3);
    expect(prepared[2].label).toBe("LIGHT");
    expect(prepared[2].characters.join("")).toBe("LIGHT");
    expect(prepared[2].sustain).toBeGreaterThan(0.9);
    expect(prepared[2].isHero).toBe(true);
    expect(prepared[0].isHero).toBe(false);
  });

  it("resizing scales metrics linearly; spread moves offsets but not identity", () => {
    const first = prepareScattered(key());
    const doubled = prepareScattered(key(88));
    expect(doubled[0].size).toBeCloseTo(first[0].size * 2, 4);
    const wider = prepareScattered(key(88, 1.5));
    expect(wider[0].waveOffset).not.toBe(doubled[0].waveOffset);
    expect(first.map((w) => w.key)).toEqual(wider.map((w) => w.key));
  });

  it("inline readings use the source word's font rules", () => {
    const word: Word = { text: "光", start: 0, end: 1 };
    const readings = new Map([[timedWordKey(word.text, word.start, word.end), "hikari"]]);
    const scattered = prepareScattered({ ...key(80, 1, "editorial"), words: [word], readings })[0];
    expect(scattered.reading?.text).toBe("hikari");
    expect(scattered.reading?.font).toEqual(lyricFont("manrope", scattered.size * 0.78, scattered.isHero, scattered.alternateFace));
    const spoken = prepareSpoken([word], 80, "spoken", "manrope", readings)[0];
    expect(spoken.reading?.text).toBe("hikari");
    expect(spoken.reading?.font).not.toEqual(spoken.font);
  });

  it("editorial sets some words in the contrasting face, never a hero", () => {
    const many: Word[] = Array.from({ length: 40 }, (_, i) => ({ text: `word${i}`, start: i, end: i + 0.3 }));
    const prepared = prepareScattered({ ...key(44, 1, "editorial"), words: many });
    const alternates = prepared.filter((w) => w.alternateFace);
    expect(alternates.length).toBeGreaterThan(0);
    expect(alternates.length).toBeLessThan(prepared.length);
    for (const w of alternates) {
      expect(w.isHero).toBe(false);
      expect(w.font.face).toBe(contrastingFace("manrope"));
    }
    // Scattered has no second face.
    expect(prepareScattered({ ...key(44, 1, "scattered"), words: many }).some((w) => w.alternateFace)).toBe(false);
  });

  it("rolling styles keep a timed word's identity, hash and font across windows", () => {
    for (const style of ["spoken", "cascade", "cutout"] as ConcreteComposition[]) {
      const first = prepareSpoken(words, 44, style, "manrope");
      const shifted = prepareSpoken(words.slice(1), 44, style, "manrope");
      expect(first[1].key).toBe(shifted[0].key);
      expect(first[1].hash.equals(shifted[0].hash)).toBe(true);
      expect(first[1].font).toEqual(shifted[0].font);
    }
  });
});

describe("flow layout", () => {
  it("wraps rows at the stage width and aligns them", () => {
    const items = [{ width: 100, height: 40 }, { width: 120, height: 50 }, { width: 90, height: 30 }];
    const placed = flowLayout(items, 240, 10, 20, "leading");
    expect(placed.width).toBe(230);
    expect(placed.height).toBe(50 + 20 + 30);
    expect(placed.centres[0]).toEqual({ x: 50, y: 25 });
    expect(placed.centres[1]).toEqual({ x: 170, y: 25 });
    expect(placed.centres[2]).toEqual({ x: 45, y: 85 });
    expect(flowLayout(items, 240, 10, 20, "trailing").centres[2].x).toBe(230 - 45);
    expect(flowLayout(items, 240, 10, 20, "center").centres[2].x).toBe(115);
    // RTL mirrors positions within the same rows.
    expect(flowLayout(items, 240, 10, 20, "leading", true).centres[0]).toEqual({ x: 180, y: 25 });
  });

  it("reports an oversized word honestly", () => {
    const placed = flowLayout([{ width: 500, height: 40 }], 240, 10, 20, "center");
    expect(placed.width).toBe(500);
  });
});
