// Mirrors NextWordPreviewTests.swift and the frame/interlude tests in
// RenderingPerformanceTests.swift, plus the stage rules of NowPlayingView.
import { describe, expect, it } from "vitest";
import type { Line, Settings, Word } from "../contract";
import {
  ALL_COMPOSITIONS,
  arrivalOnly,
  cueBob,
  cueHighlight,
  fnvString,
  gapPresence,
  instrumentalCue,
  lineStage,
  nextLinePreview,
  rollingWords,
  scatteredWordFrame,
  showsOutgoing,
  wordBehaviour,
  wordHeat,
  wordPresentation,
  type ConcreteComposition,
  type LineRules,
} from "./index";

const on: Pick<Settings, "nextWordPreview" | "wordReveal" | "nextWordPreviewLead"> = { nextWordPreview: true, wordReveal: true, nextWordPreviewLead: 0.5 };

function rules(composition: ConcreteComposition, settings = on): LineRules {
  return { composition: () => composition, behaviour: (c) => wordBehaviour(settings, c) };
}

describe("word presentation (NextWordPreviewTests)", () => {
  it("every style can preview exactly one future word", () => {
    for (const style of ALL_COMPOSITIONS) {
      const behaviour = wordBehaviour(on, style);
      const next = wordPresentation({ start: 2, end: 3 }, true, 1.75, behaviour);
      const later = wordPresentation({ start: 3, end: 4 }, false, 1.75, behaviour);
      expect(next.opacity, style).toBeCloseTo(0.125, 3);
      expect(next.preview, style).toBe(true);
      expect(later.opacity, style).toBeCloseTo(0, 3);
    }
  });
  it("every style can hide the preview", () => {
    for (const style of ALL_COMPOSITIONS) {
      const behaviour = wordBehaviour({ ...on, nextWordPreview: false }, style);
      expect(wordPresentation({ start: 2, end: 3 }, true, 1.75, behaviour).opacity, style).toBeCloseTo(0, 3);
    }
  });
  it("rolling and cutout layouts always reveal words", () => {
    const off = { ...on, wordReveal: false };
    for (const style of ALL_COMPOSITIONS) {
      const behaviour = wordBehaviour(off, style);
      expect(behaviour.reveals, style).toBe(arrivalOnly(style));
    }
    const shown = wordPresentation({ start: 1, end: 2 }, false, 0.5, wordBehaviour(off, "editorial"));
    expect(shown).toEqual({ progress: 1, opacity: 1, preview: false });
    expect(wordPresentation({ start: 1, end: 2 }, false, 1.1, wordBehaviour(off, "cutout")).progress).toBeLessThan(1);
  });
});

describe("scattered word frames (RenderingPerformanceTests)", () => {
  const behaviour = wordBehaviour(on, "constellationEditorial");
  const light = { start: 10.6, end: 13 };
  it("lighting is continuous at word boundaries", () => {
    for (const boundary of [light.start, light.end, light.end + 0.3]) {
      expect(Math.abs(wordHeat(boundary - 0.0001, light.start, light.end) - wordHeat(boundary + 0.0001, light.start, light.end))).toBeLessThan(0.001);
    }
    expect(wordHeat(light.end + 0.1, light.start, light.end)).toBeGreaterThan(wordHeat(light.end + 0.2, light.start, light.end));
  });
  it("settled and not-yet-arriving words keep identical frames; arrival and seeks change them", () => {
    const keep = { start: 10, end: 10.3 };
    const seed = fnvString("Keep");
    const frame = (t: number) => JSON.stringify(scatteredWordFrame(keep, true, t, wordBehaviour(on, "scattered"), 0, seed, 0));
    expect(frame(0)).toBe(frame(8));
    expect(frame(20)).toBe(frame(30));
    expect(frame(10.01)).not.toBe(frame(10.08));
    expect(frame(20)).not.toBe(frame(8));
  });
  it("held words glow while sung; fill sweeps only when reveal is off", () => {
    const seed = fnvString("light");
    const held = scatteredWordFrame(light, false, 12, behaviour, 1, seed, 0);
    expect(held.glow).toBeGreaterThan(0.5);
    expect(held.fill).toBe(1);
    const sweep = scatteredWordFrame(light, false, 11.8, wordBehaviour({ ...on, wordReveal: false }, "editorial"), 1, seed, 0);
    expect(sweep.fill).toBeCloseTo(0.5, 6);
    expect(sweep.opacity).toBe(1);
  });
});

describe("instrumental cue (RenderingPerformanceTests)", () => {
  it("dots pulse and leave room for the next-word preview", () => {
    const gap = { start: 10, end: 26, nextVocal: 3 };
    expect(instrumentalCue(gap, 10, 0.5)).toBeNull();
    const cue = instrumentalCue(gap, 16, 0.5)!;
    expect(cue.opacity).toBe(1);
    expect(cueBob(cue, 0)).not.toBe(cueBob(cue, 1));
    expect(cueHighlight(cue, 0)).toBeGreaterThan(cueHighlight(cue, 2));
    expect(instrumentalCue({ start: 10, end: -1, nextVocal: -1 }, 17, 0.5)?.progress).toBeNull();
    expect(instrumentalCue(gap, 25.55, 0.5)).toBeNull();
  });
  it("lyrics fade out for the break and back before the vocal returns", () => {
    const gap = { start: 10, end: 26, nextVocal: 3 };
    expect(gapPresence(gap, 10)).toBe(0);
    expect(gapPresence(gap, 11)).toBe(1);
    expect(gapPresence(gap, 26)).toBe(0);
    expect(gapPresence({ ...gap, nextVocal: -1 }, 26)).toBe(1);
  });
});

function words(start: number, texts: string[], step = 0.5): Word[] {
  return texts.map((text, i) => ({ text, start: start + i * step, end: start + i * step + step * 0.9 }));
}

function wordLine(start: number, texts: string[], wordTimed = true): Line {
  const ws = words(start, texts);
  return { start, end: ws[ws.length - 1].end, text: texts.join(" "), words: ws, isInstrumental: false, wordTimed };
}

describe("stage rules (NowPlayingView.currentLine)", () => {
  const lines = [wordLine(10, ["one", "two", "three"]), wordLine(12, ["four", "five"]), wordLine(14, ["six", "seven"], false)];

  it("previews the next line only after the current line's last word and within the lead", () => {
    const r = rules("editorial");
    expect(nextLinePreview(lines, 0, 10.9, r)).toBeNull(); // "three" not yet sung
    expect(nextLinePreview(lines, 0, 11.2, r)).toBeNull(); // more than 0.5 s before "four"
    expect(nextLinePreview(lines, 0, 11.6, r)).toBe(1);
    expect(nextLinePreview(lines, 0, 12.0, r)).toBeNull(); // the line has started
    expect(nextLinePreview(lines, 1, 13.6, rules("editorial", { ...on, nextWordPreview: false }))).toBeNull();
  });

  it("rolling runs never preview or keep an outgoing line", () => {
    const r = rules("spoken");
    expect(nextLinePreview(lines, 0, 11.6, r)).toBeNull();
    expect(showsOutgoing(lines, 1, 12.1, r)).toBe(false);
    // A line-timed line after a rolling run is still previewed and keeps the outgoing rolling line.
    expect(nextLinePreview(lines, 1, 13.6, r)).toBe(2);
    expect(showsOutgoing(lines, 2, 14.1, r)).toBe(true);
    expect(showsOutgoing(lines, 2, 14.5, r)).toBe(false);
  });

  it("stage presence follows the Mac rules for current, outgoing and preview lines", () => {
    const r = rules("editorial");
    expect(lineStage(lines, 1, 10.5, "current", r)).toBeNull(); // more than max(1, lead) early
    const handsOff = lineStage(lines, 0, 9.5, "current", r)!;
    expect(handsOff.presence).toBe(1); // word reveal + preview: the words fade themselves
    const outgoing = lineStage(lines, 0, 11.8, "outgoing", r)!;
    expect(outgoing.presence).toBeCloseTo(0.5, 6);
    expect(lineStage(lines, 0, 12.3, "outgoing", r)).toBeNull(); // fully exited
    expect(lineStage(lines, 1, 11.6, "preview", r)!.presence).toBe(1);
    const spoken = lineStage(lines, 0, 10.2, "current", rules("spoken"))!;
    expect(spoken.rolls).toBe(true);
    const phrase = lineStage(lines, 2, 14.05, "current", r)!;
    expect(phrase.rolls).toBe(false);
    expect(phrase.bridgesPreview).toBe(true);
  });

  it("collects rolling words across the run, deduplicated and sorted", () => {
    const run = [wordLine(10, ["one", "two"]), wordLine(11, ["three", "four"]), wordLine(12, ["five", "♪", "six"])];
    const { words: rolled } = rollingWords(run, [], 1, 11.2, "off", 0.45);
    expect(rolled.map((w) => w.text)).toEqual(["one", "two", "three", "four", "five", "six"].slice(0, 5));
    const later = rollingWords(run, [], 1, 22.6, "off", 0.45);
    expect(later.words.map((w) => w.text)).toEqual(["two", "three", "four", "five", "six"]);
    expect(rollingWords(run, [], 1, 40, "off", 0.45).words).toEqual([]);
    const readings: (Word[] | null)[] = [null, run[1].words.map((w) => ({ ...w, text: w.text.toUpperCase() })), null];
    const replaced = rollingWords(run, readings, 1, 11.2, "replace", 0.45);
    expect(replaced.words.map((w) => w.text)).toContain("THREE");
    const below = rollingWords(run, readings, 1, 11.2, "below", 0.45);
    expect(below.readings.size).toBe(2);
  });
});
