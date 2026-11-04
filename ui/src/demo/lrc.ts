// A small LRC / TTML reader so `import_lyrics` works in the browser demo.
// The app itself parses imports in Rust (crates/lunalyr-core/src/parse).

import type { Line, Word } from "../contract";
import { distributeWords } from "./samples";

const STAMP = /\[(\d{1,3}):(\d{1,2})(?:[.:](\d{1,3}))?\]/g;
const WORD_STAMP = /<(\d{1,3}):(\d{1,2})(?:[.:](\d{1,3}))?>/g;

function seconds(min: string, sec: string, frac?: string): number {
  const fraction = frac ? Number(frac) / 10 ** frac.length : 0;
  return Number(min) * 60 + Number(sec) + fraction;
}

export function parseLrc(text: string, duration = 0): Line[] {
  let offset = 0;
  const rows: { time: number; body: string }[] = [];
  for (const raw of text.split(/\r?\n/)) {
    const meta = raw.match(/^\[offset:\s*([+-]?\d+)\]/i);
    if (meta) {
      offset = Number(meta[1]) / 1000;
      continue;
    }
    const stamps = [...raw.matchAll(STAMP)];
    if (stamps.length === 0) continue;
    const body = raw.replace(STAMP, "").trim();
    for (const s of stamps) rows.push({ time: seconds(s[1], s[2], s[3]), body });
  }
  rows.sort((a, b) => a.time - b.time);
  const lines: Line[] = [];
  rows.forEach((row, i) => {
    const start = Math.max(0, row.time - offset);
    const next = i + 1 < rows.length ? Math.max(0, rows[i + 1].time - offset) : duration > start ? duration : start + 4;
    const plain = row.body.replace(WORD_STAMP, "").replace(/\s+/g, " ").trim();
    if (!plain) return;
    const wordStamps = [...row.body.matchAll(WORD_STAMP)];
    let words: Word[] = [];
    let wordTimed = false;
    if (wordStamps.length > 0) {
      const parts = row.body.split(WORD_STAMP);
      // split() interleaves captured groups; rebuild (time, text) pairs.
      const pairs: { time: number; text: string }[] = [];
      for (let k = 0; k < wordStamps.length; k++) {
        const s = wordStamps[k];
        const textPart = parts[1 + k * 4 + 3] ?? "";
        pairs.push({ time: seconds(s[1], s[2], s[3]) - offset, text: textPart.trim() });
      }
      words = pairs
        .map((p, k) => ({ text: p.text, start: p.time, end: k + 1 < pairs.length ? pairs[k + 1].time : Math.min(next, p.time + 1.2) }))
        .filter((w) => w.text.length > 0 && w.end > w.start);
      wordTimed = words.length > 0;
    }
    const end = wordTimed ? Math.max(...words.map((w) => w.end)) : Math.min(next - 0.2, start + Math.max(2.4, plain.split(" ").length * 0.45 + 0.8));
    if (!wordTimed) words = distributeWords(plain, start, end);
    lines.push({ start, end, text: plain, words, isInstrumental: false, wordTimed });
  });
  return lines;
}

function ttmlTime(value: string | null): number | null {
  if (!value) return null;
  const v = value.trim();
  if (/^[\d.]+s$/.test(v)) return Number(v.slice(0, -1));
  if (/^[\d.]+ms$/.test(v)) return Number(v.slice(0, -2)) / 1000;
  const parts = v.split(":").map(Number);
  if (parts.some((p) => !Number.isFinite(p))) return null;
  return parts.reduce((total, part) => total * 60 + part, 0);
}

export function parseTtml(text: string): Line[] {
  if (typeof DOMParser === "undefined") return [];
  const doc = new DOMParser().parseFromString(text, "application/xml");
  const lines: Line[] = [];
  for (const p of Array.from(doc.getElementsByTagName("p"))) {
    const start = ttmlTime(p.getAttribute("begin"));
    const end = ttmlTime(p.getAttribute("end"));
    if (start === null || end === null || !(end > start)) continue;
    const spans = Array.from(p.getElementsByTagName("span")).filter((s) => s.getAttribute("begin"));
    const words: Word[] = spans
      .map((s) => ({ text: (s.textContent ?? "").trim(), start: ttmlTime(s.getAttribute("begin")) ?? start, end: ttmlTime(s.getAttribute("end")) ?? end }))
      .filter((w) => w.text.length > 0);
    const lineText = (p.textContent ?? "").replace(/\s+/g, " ").trim();
    if (!lineText) continue;
    const wordTimed = words.length > 0;
    lines.push({ start, end, text: lineText, words: wordTimed ? words : distributeWords(lineText, start, end), isInstrumental: false, wordTimed });
  }
  return lines.sort((a, b) => a.start - b.start);
}

export function parseImport(text: string, duration: number): Line[] {
  return text.trimStart().startsWith("<") ? parseTtml(text) : parseLrc(text, duration);
}

export function precisionOf(lines: Line[]): string {
  if (lines.length === 0) return "Not synced";
  const timed = lines.filter((l) => l.wordTimed).length;
  if (timed === lines.length) return "Word-by-word";
  if (timed > 0) return "Mixed timing";
  return "Line-synced";
}
