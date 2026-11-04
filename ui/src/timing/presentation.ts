// Per-word and per-line presentation rules from NowPlayingView.swift,
// WordPresentation.swift, ScatteredLyricText.swift (`ScatteredWordFrame`)
// and InstrumentalInterludeView.swift. All pure functions of time.

import type { Line, ReadingDisplay, Word } from "../contract";
import { arrivalOnly, type ConcreteComposition, type WordBehaviour } from "./composition";
import type { VocalGap } from "./geometry";
import { wordReveal, wordVisibility } from "./geometry";
import { firstWordOnset, nextStartAfter, phraseTiming, type PhraseTiming, readingPairs, rollingRange } from "./lines";
import { hasLetterOrNumber, smoothstep } from "./math";
import { newPose, vocalGesture, type WordPose } from "./motion";
import type { Seed64 } from "./seed";
import { timedWordKey } from "./seed";

export interface WordPresentation {
  progress: number;
  opacity: number;
  preview: boolean;
}

/** `WordPresentation.make`. `isNext` is `index == nextIndex`. */
export function wordPresentation(
  word: Pick<Word, "start" | "end">,
  isNext: boolean,
  time: number,
  behaviour: WordBehaviour,
  out: WordPresentation = { progress: 1, opacity: 1, preview: false },
): WordPresentation {
  if (!behaviour.reveals) {
    out.progress = 1;
    out.opacity = 1;
    out.preview = false;
    return out;
  }
  const opacity = wordVisibility(time, word.start, word.end, behaviour.previews, isNext, behaviour.lead);
  out.progress = wordReveal(time, word.start, word.end);
  out.opacity = opacity;
  out.preview = time < word.start && opacity > 0;
  return out;
}

/** Continuous vocal emphasis: 0.1 s rise before the onset, 0.3 s release after the end. */
export function wordHeat(time: number, start: number, end: number): number {
  if (time < start - 0.1) return 0;
  if (time < start) return smoothstep((time - start + 0.1) / 0.1);
  if (time <= end) return 1;
  return 1 - smoothstep((time - end) / 0.3);
}

export interface ScatteredFrame extends WordPresentation {
  heat: number;
  fill: number;
  glow: number;
  drift: WordPose;
}

export function newScatteredFrame(): ScatteredFrame {
  return { progress: 1, opacity: 1, preview: false, heat: 0, fill: 1, glow: 0, drift: newPose() };
}

/** `ScatteredWordFrame.make` (energy is 0 on the wallpaper, as on the Mac). */
export function scatteredWordFrame(
  word: Pick<Word, "start" | "end">,
  isNext: boolean,
  time: number,
  behaviour: WordBehaviour,
  sustain: number,
  motionSeed: Seed64,
  energy: number,
  out: ScatteredFrame = newScatteredFrame(),
): ScatteredFrame {
  wordPresentation(word, isNext, time, behaviour, out);
  const heat = wordHeat(time, word.start, word.end);
  const progress = Math.min(Math.max((time - word.start) / Math.max(word.end - word.start, 0.001), 0), 1);
  out.heat = heat;
  out.fill = behaviour.reveals ? 1 : progress;
  out.glow = sustain > 0.01 && heat > 0.01 ? heat * sustain * smoothstep(progress / 0.45) : 0;
  vocalGesture(time - word.start, word.end - word.start, motionSeed, energy, false, out.drift);
  return out;
}

// MARK: instrumental breaks

/** `VCVocalGap.presence(at:)`: how far the lyrics have faded for the break. */
export function gapPresence(gap: VocalGap, time: number): number {
  const enter = smoothstep((time - gap.start) / 0.7);
  const leave = gap.nextVocal >= 0 ? 1 - smoothstep((time - gap.end + 0.65) / 0.65) : 1;
  return Math.min(enter, leave);
}

export interface InstrumentalCue {
  elapsed: number;
  opacity: number;
  /** null when the break runs to an unknown track end. */
  progress: number | null;
}

/** `VCVocalGap.instrumentalCue`: leaves room for the next-word preview before the vocal returns. */
export function instrumentalCue(gap: VocalGap, time: number, previewLead: number): InstrumentalCue | null {
  const elapsed = time - gap.start;
  const fadeIn = smoothstep((elapsed - 0.42) / 0.5);
  const knownEnd = gap.end > gap.start;
  const cueEnd = gap.end - (gap.nextVocal >= 0 ? Math.min(previewLead, Math.max(0, gap.end - gap.start - 0.85)) : 0);
  const fadeOut = knownEnd ? smoothstep((cueEnd - time) / 0.24) : 1;
  const opacity = Math.min(fadeIn, fadeOut);
  const progress = knownEnd ? Math.min(1, Math.max(0, elapsed / Math.max(0.01, cueEnd - gap.start))) : null;
  return opacity > 0 ? { elapsed, opacity, progress } : null;
}

export function cueBob(cue: InstrumentalCue, index: number): number {
  return Math.sin(cue.elapsed * 3.0 - index * 0.95);
}

export function cueHighlight(cue: InstrumentalCue, index: number): number {
  if (cue.progress === null) return 0;
  return smoothstep(cue.progress * 3 - index);
}

// MARK: which lines are on stage (NowPlayingView.currentLine)

export interface LineRules {
  /** Resolved (and renderable) composition for a line. */
  composition(index: number): ConcreteComposition;
  behaviour(composition: ConcreteComposition): WordBehaviour;
}

function bothRoll(lines: readonly Line[], a: number, b: number, rules: LineRules): boolean {
  return lines[a].wordTimed && lines[b].wordTimed && arrivalOnly(rules.composition(a)) && arrivalOnly(rules.composition(b));
}

/** Outgoing ink finishes its exit after a line boundary; rolling runs keep one stage. */
export function showsOutgoing(lines: readonly Line[], index: number, time: number, rules: LineRules): boolean {
  return index > 0 && index < lines.length && time < lines[index].start + 0.46 && !bothRoll(lines, index, index - 1, rules);
}

/** `nextLinePreview(after:time:)`: one preview at a time, within the preview lead of its first word. */
export function nextLinePreview(lines: readonly Line[], index: number, time: number, rules: LineRules): number | null {
  const next = index + 1;
  if (index < 0 || index >= lines.length || next >= lines.length) return null;
  if (lines[next].isInstrumental || !(time < lines[next].start)) return null;
  const style = rules.composition(next);
  const behaviour = rules.behaviour(style);
  if (!behaviour.previews) return null;
  if (lines[next].wordTimed && !behaviour.reveals) return null;
  if (lines[index].wordTimed && lines[next].wordTimed && arrivalOnly(style) && arrivalOnly(rules.composition(index))) return null;
  if (lines[index].wordTimed && lines[index].words.some((w) => w.start > time && hasLetterOrNumber(w.text))) return null;
  const onset = firstWordOnset(lines[next]);
  if (onset === null || !(onset > time) || onset - time > behaviour.lead) return null;
  return next;
}

export type LineRole = "current" | "outgoing" | "preview";

export interface LineStage {
  composition: ConcreteComposition;
  behaviour: WordBehaviour;
  pose: PhraseTiming;
  /** Word-timed line in a rolling layout. */
  rolls: boolean;
  /** Line opacity (renderLyricLine `stagePresence`). */
  presence: number;
  /** A line-timed line shows its first word while the phrase enters. */
  bridgesPreview: boolean;
}

/** The presentation of one line on the stage, or null when it draws nothing. */
export function lineStage(lines: readonly Line[], index: number, time: number, role: LineRole, rules: LineRules): LineStage | null {
  if (index < 0 || index >= lines.length) return null;
  const line = lines[index];
  if (line.words.length === 0) return null;
  const composition = rules.composition(index);
  const behaviour = rules.behaviour(composition);
  if (!(time >= line.start - Math.max(1, behaviour.lead))) return null;
  const outgoing = role === "outgoing";
  const previewing = role === "preview";
  const pose = phraseTiming(line, nextStartAfter(lines, index), time);
  const rolls = line.wordTimed && arrivalOnly(composition);
  const handsOffPreview = line.wordTimed && !outgoing && behaviour.reveals && behaviour.previews;
  const presence = previewing || (rolls && !outgoing) ? 1 : handsOffPreview ? 1 - pose.exit : pose.presence;
  const bridgesPreview = !line.wordTimed && !outgoing && behaviour.previews && time >= line.start && pose.progress < 1;
  if (!(presence > 0.001 || bridgesPreview)) return null;
  return { composition, behaviour, pose, rolls, presence, bridgesPreview };
}

/** Words and reading annotations for the rolling stage around `index` (`rollingWords(around:time:)`). */
export function rollingWords(
  lines: readonly Line[],
  readings: readonly (Word[] | null)[],
  index: number,
  time: number,
  display: ReadingDisplay,
  lead: number,
): { words: Word[]; readings: Map<string, string> } {
  const result: Word[] = [];
  const seen = new Set<string>();
  const annotations = new Map<string, string>();
  const [first, last] = rollingRange(index, lines);
  for (let row = first; row <= last; row++) {
    const rowReadings = readings[row] ?? null;
    if (display === "below") {
      for (const [key, value] of readingPairs(rowReadings, lines[row].words)) {
        if (!annotations.has(key)) annotations.set(key, value);
      }
    }
    const words = display === "replace" ? rowReadings ?? lines[row].words : lines[row].words;
    for (const word of words) {
      if (!(word.end >= time - 12)) continue;
      if (!(row <= index || word.start <= time + Math.max(1.2, lead))) continue;
      if (!hasLetterOrNumber(word.text)) continue;
      const key = timedWordKey(word.text, word.start, word.end);
      if (seen.has(key)) continue;
      seen.add(key);
      result.push(word);
    }
  }
  // Stable sort by start, as Swift's `sorted` on the collected order.
  result.sort((a, b) => a.start - b.start);
  return { words: result, readings: annotations };
}
