// Mirrors LyricGapTimingTests.swift, NextWordPreviewTests.swift (onsets and
// identity) and the line-index / phrase tests in RenderingPerformanceTests.swift.
import { describe, expect, it } from "vitest";
import type { Line, Word } from "../contract";
import {
  firstWordOnset,
  LineIndex,
  phraseTiming,
  readingPairs,
  rollingRange,
  timedWordKey,
  visualSeed,
  vocalWindows,
  wordsPerSecond,
} from "./index";

function line(start: number, end: number, words: Word[], wordTimed = false, isInstrumental = false, text = "x"): Line {
  return { start, end, text, words, isInstrumental, wordTimed };
}

describe("vocal windows (LyricGapTimingTests)", () => {
  it("native word end marks the actual vocal window", () => {
    const windows = vocalWindows([line(1, 30, [{ text: "Hold", start: 1.2, end: 4 }], true)]);
    expect(windows[0]).toEqual({ start: 1.2, end: 4 });
  });
  it("estimated words do not invent an earlier vocal end", () => {
    expect(vocalWindows([line(1, 30, [{ text: "Hold", start: 1, end: 25 }])])[0].end).toBe(30);
  });
  it("explicit instrumental lines are excluded", () => {
    expect(vocalWindows([line(5, 20, [], false, true, "")])).toEqual([]);
  });
  it("invalid or untimed empty lines are ignored", () => {
    expect(vocalWindows([line(Number.NaN, 20, [{ text: "Hold", start: 1, end: 3 }])])).toEqual([]);
    expect(vocalWindows([line(3, 5, [])])).toEqual([]);
  });
  it("sorts windows by start", () => {
    const windows = vocalWindows([line(8, 9, [{ text: "b", start: 8, end: 9 }]), line(2, 3, [{ text: "a", start: 2, end: 3 }])]);
    expect(windows.map((w) => w.start)).toEqual([2, 8]);
  });
});

describe("line index (RenderingPerformanceTests)", () => {
  it("matches the forward rule at boundaries and after seeks", () => {
    const index = new LineIndex([2, 5, 5, 8].map((start) => ({ start })));
    expect(index.line(1.99)).toBeNull();
    expect(index.line(2)).toBe(0);
    expect(index.line(5)).toBe(2);
    expect(index.line(100)).toBe(3);
    expect(index.line(3)).toBe(0);
    expect(index.line(Number.NaN)).toBeNull();
  });
  it("keeps the early-stop rule for malformed timelines and handles empty input", () => {
    expect(new LineIndex([2, 8, 5].map((start) => ({ start }))).line(6)).toBe(0);
    expect(new LineIndex([]).line(6)).toBeNull();
    expect(new LineIndex([2, Number.NaN, 9].map((start) => ({ start }))).line(10)).toBe(0);
  });
  it("agrees with a linear scan on ordered input", () => {
    const starts = Array.from({ length: 200 }, (_, i) => i * 1.7 + (i % 3 === 0 ? 0 : 0.2));
    const index = new LineIndex(starts.map((start) => ({ start })));
    for (let t = -1; t < 360; t += 0.37) {
      let expected: number | null = null;
      starts.forEach((s, i) => {
        if (s <= t) expected = i;
      });
      expect(index.line(t)).toBe(expected);
    }
  });
});

describe("phrase timing", () => {
  it("fades the whole phrase for mixed timing and keeps rolling runs separate", () => {
    const lines = [true, true, false, true].map((timed, index) => {
      const start = index * 2;
      return line(start, start + 2, [{ text: "phrase", start: start + 0.8, end: start + 1.5 }], timed, false, "whole phrase");
    });
    const phrase = lines[2];
    expect(phraseTiming(phrase, 6, 3.9).presence).toBe(0);
    expect(phraseTiming(phrase, 6, 4.35).presence).toBe(1);
    expect(phraseTiming(phrase, 6, 6.1).presence).toBeGreaterThan(0);
    expect(rollingRange(1, lines)).toEqual([0, 1]);
    expect(rollingRange(3, lines)).toEqual([3, 3]);
  });
  it("word-timed lines enter over 0.22 s and exit after their end", () => {
    const l = line(10, 12, [{ text: "a", start: 10, end: 12 }], true);
    expect(phraseTiming(l, 20, 10.11).progress).toBeCloseTo(0.5, 6);
    expect(phraseTiming(l, 20, 12.12).exit).toBe(0);
    expect(phraseTiming(l, 20, 12.12 + 0.46).exit).toBe(1);
    // The next line's start pulls the exit earlier.
    expect(phraseTiming(l, 12.1, 12.02 + 0.23).exit).toBeCloseTo(0.5, 6);
    // Line-only lyrics attack over 18% of their interval, within 0.12–0.32 s.
    const plain = line(10, 12, [{ text: "a", start: 10, end: 12 }], false);
    expect(phraseTiming(plain, 20, 10.16).progress).toBeCloseTo(0.5, 6);
    expect(phraseTiming(line(10, 10.4, [], false), 20, 10.06).progress).toBeCloseTo(0.5, 6);
  });
  it("rolling ranges reach back at most two word-timed lines", () => {
    const lines = Array.from({ length: 6 }, (_, i) => line(i, i + 1, [], true));
    expect(rollingRange(4, lines)).toEqual([2, 5]);
    expect(rollingRange(0, lines)).toEqual([0, 1]);
    expect(rollingRange(5, lines)).toEqual([3, 5]);
  });
});

describe("next-line onsets and word identity (NextWordPreviewTests)", () => {
  it("uses the real onset of the first sung word, or the line timestamp", () => {
    const next = line(10, 12, [
      { text: "…", start: 10, end: 10.05 },
      { text: "first", start: 10.2, end: 11 },
      { text: "word", start: 11, end: 12 },
    ], true, false, "… first word");
    expect(firstWordOnset(next)).toBe(10.2);
    expect(firstWordOnset({ ...next, wordTimed: false })).toBe(10);
    expect(firstWordOnset({ ...next, isInstrumental: true })).toBeNull();
    expect(firstWordOnset({ ...next, words: [{ text: "♪", start: 10, end: 11 }] })).toBeNull();
  });
  it("keeps a timed word's identity when it moves between line windows", () => {
    expect(timedWordKey("light", 8.2, 8.8)).toBe(timedWordKey("light", 8.2, 8.8));
    expect(visualSeed("light", 8.2).equals(visualSeed("light", 8.2))).toBe(true);
    expect(visualSeed("light", 8.2).equals(visualSeed("light", 8.3))).toBe(false);
  });
});

describe("readings and cadence", () => {
  it("pairs readings that differ from their word, only when counts match", () => {
    const words: Word[] = [{ text: "光", start: 0, end: 1 }, { text: "の", start: 1, end: 2 }, { text: "x", start: 2, end: 3 }];
    const readings: Word[] = [{ text: "hikari", start: 0, end: 1 }, { text: "no", start: 1, end: 2 }, { text: "x", start: 2, end: 3 }];
    const pairs = readingPairs(readings, words);
    expect(pairs.get(timedWordKey("光", 0, 1))).toBe("hikari");
    expect(pairs.size).toBe(2);
    expect(readingPairs(readings.slice(1), words).size).toBe(0);
    expect(readingPairs(null, words).size).toBe(0);
  });
  it("words per second spans first start to last end", () => {
    const lines = [line(0, 2, [{ text: "a", start: 0, end: 1 }, { text: "b", start: 1, end: 2 }]), line(2, 4, [{ text: "c", start: 2, end: 4 }])];
    expect(wordsPerSecond(lines)).toBeCloseTo(0.75, 10);
    expect(wordsPerSecond([line(0, 0.5, [{ text: "a", start: 0, end: 0.5 }])])).toBe(0);
  });
});
