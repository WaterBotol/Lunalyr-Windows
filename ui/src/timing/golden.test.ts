// Value-for-value checks against the macOS C sources. golden.fixture.json is
// produced by compiling LyricMotion.c and ConstellationGeometry.c unmodified
// with ui/tools/golden.c and printing
// their outputs for a deterministic spread of inputs.
import { describe, expect, it } from "vitest";
import goldenJson from "./golden.fixture.json";
import {
  buildConnections,
  estimatedLineEnd,
  fnvString,
  motionEase,
  motionHandoff,
  rollingPose,
  Seed64,
  visualSeed,
  vocalGap,
  vocalGesture,
  wordEnvelope,
  wordReveal,
  wordSettle,
  wordVisibility,
  type RollingStyle,
  type VocalWindow,
} from "./index";

type Num = number | string;
interface Golden {
  ease: [Num, Num][];
  handoff: [Num, Num, Num][];
  rolling: [number, Num, Num, Num, string, Num, number, Num[]][];
  gesture: [Num, Num, string, Num, number, Num[]][];
  reveal: Num[][];
  envelope: Num[][];
  lineEnd: Num[][];
  gaps: [string, Num, Num, Num, number, Num, Num, number][];
  connections: { view: number[]; clearance: number; boxes: number[][]; segments: number[][] }[];
  fnv: { text: string; start: Num; hash: string; visual: string; metrics: [number, string, number, number, number, number, number][]; spoken: number[] }[];
}
const golden = goldenJson as unknown as Golden;

function n(v: Num): number {
  if (typeof v === "number") return v;
  if (v === "NaN") return Number.NaN;
  if (v === "Infinity") return Number.POSITIVE_INFINITY;
  if (v === "-Infinity") return Number.NEGATIVE_INFINITY;
  if (v === "-0") return -0;
  return Number(v);
}

function close(actual: number, expected: number, context: string): void {
  if (Number.isNaN(expected)) {
    expect(Number.isNaN(actual), context).toBe(true);
    return;
  }
  const tolerance = 1e-11 * Math.max(1, Math.abs(expected));
  if (Math.abs(actual - expected) > tolerance) {
    throw new Error(`${context}: expected ${expected}, got ${actual}`);
  }
}

describe("LyricMotion.c parity", () => {
  it("vc_motion_ease", () => {
    for (const [t, value] of golden.ease) close(motionEase(n(t)), n(value), `ease(${t})`);
  });

  it("vc_motion_handoff", () => {
    for (const [age, cadence, value] of golden.handoff) close(motionHandoff(n(age), n(cadence)), n(value), `handoff(${age}, ${cadence})`);
  });

  it("vc_rolling_pose", () => {
    expect(golden.rolling.length).toBeGreaterThan(200);
    for (const [style, rank, age, duration, seed, energy, reduced, pose] of golden.rolling) {
      const p = rollingPose(style as RollingStyle, n(rank), n(age), n(duration), Seed64.fromString(seed), n(energy), reduced === 1);
      const values = [p.x, p.y, p.scale, p.angle, p.opacity, p.blur];
      values.forEach((v, i) => close(v, n(pose[i]), `rolling(${style},${rank},${age},${duration},${seed})[${i}]`));
    }
  });

  it("vc_vocal_gesture", () => {
    for (const [age, duration, seed, energy, reduced, pose] of golden.gesture) {
      const p = vocalGesture(n(age), n(duration), Seed64.fromString(seed), n(energy), reduced === 1);
      const values = [p.x, p.y, p.scale, p.angle, p.opacity, p.blur];
      values.forEach((v, i) => close(v, n(pose[i]), `gesture(${age},${duration},${seed})[${i}]`));
    }
  });
});

describe("ConstellationGeometry.c parity", () => {
  it("vc_word_reveal / vc_word_visibility / vc_word_settle", () => {
    for (const row of golden.reveal) {
      const [t, s, e, reveal, v00, v01, v10, v11, vLong, settle] = row.map(n);
      close(wordReveal(t, s, e), reveal, `reveal(${t},${s},${e})`);
      close(wordVisibility(t, s, e, false, false, 0.5), v00, "vis00");
      close(wordVisibility(t, s, e, false, true, 0.5), v01, "vis01");
      close(wordVisibility(t, s, e, true, false, 0.5), v10, "vis10");
      close(wordVisibility(t, s, e, true, true, 0.5), v11, "vis11");
      close(wordVisibility(t, s, e, true, true, 1.25), vLong, "visLong");
      close(wordSettle(t / 3), settle, `settle(${t / 3})`);
    }
  });

  it("vc_word_envelope", () => {
    for (const row of golden.envelope) {
      const [w, h, peak, degrees, ew, eh] = row.map(n);
      const box = wordEnvelope(w, h, peak, degrees);
      close(box.width, ew, "envelope width");
      close(box.height, eh, "envelope height");
    }
  });

  it("vc_estimated_line_end", () => {
    for (const row of golden.lineEnd) {
      const [start, next, words, perWord, value] = row.map(n);
      close(estimatedLineEnd(start, next, words, perWord), value, `lineEnd(${row.join(",")})`);
    }
  });

  it("vc_vocal_gap", () => {
    const sets: Record<string, VocalWindow[]> = {
      a: [{ start: 10, end: 14 }, { start: 22, end: 28 }],
      b: [{ start: 0, end: 5 }, { start: 6, end: 10 }],
      c: [{ start: 0, end: 8 }, { start: 4, end: 10 }, { start: 14, end: 18 }],
      d: [{ start: 3, end: 4 }, { start: 4.5, end: 9 }, { start: 12.5, end: 13 }, { start: 13, end: 20 }],
    };
    for (const [name, duration, time, minimum, found, start, end, next] of golden.gaps) {
      const gap = vocalGap(sets[name], n(time), n(duration), n(minimum));
      const context = `gap(${name}, d=${duration}, t=${time}, min=${minimum})`;
      expect(gap !== null, context).toBe(found === 1);
      if (gap) {
        close(gap.start, n(start), context);
        close(gap.end, n(end), context);
        expect(gap.nextVocal, context).toBe(next);
      }
    }
  });

  it("vc_build_connections", () => {
    for (const set of golden.connections) {
      const boxes = set.boxes.map(([x, y, width, height]) => ({ x, y, width, height }));
      const [x, y, width, height] = set.view;
      const segments = buildConnections(boxes, { x, y, width, height }, set.clearance);
      expect(segments.length).toBe(set.segments.length);
      segments.forEach((s, i) => {
        const [ax, ay, bx, by, from, to] = set.segments[i];
        close(s.ax, ax, "ax");
        close(s.ay, ay, "ay");
        close(s.bx, bx, "bx");
        close(s.by, by, "by");
        expect([s.from, s.to]).toEqual([from, to]);
      });
    }
  });
});

describe("FNV seeds (WordPresentation.swift, LyricTypography.swift)", () => {
  it("matches the Mac hashes, visual seeds and derived digits", () => {
    for (const row of golden.fnv) {
      const hash = fnvString(row.text);
      expect(hash.toString(), row.text).toBe(row.hash);
      expect(visualSeed(row.text, n(row.start)).toString(), `${row.text}@${row.start}`).toBe(row.visual);
      for (const [index, valueText, jitter, eligible, alternate, spread, rotation] of row.metrics) {
        const value = hash.xor(Seed64.fromNumber(index * 2654435761));
        expect(value.toString()).toBe(valueText);
        expect(value.div(100).mod(5)).toBe(jitter);
        expect(value.div(10_000).mod(3)).toBe(eligible);
        expect(value.div(700_000).mod(100)).toBe(alternate);
        expect(value.div(7).mod(100)).toBe(spread);
        expect(value.div(1000).mod(9) - 4).toBe(rotation);
      }
      const [m3, m300, m4, m2, m1021, v997] = row.spoken;
      expect(hash.mod(3)).toBe(m3);
      expect(hash.div(300).mod(100)).toBe(m300);
      expect(hash.mod(4)).toBe(m4);
      expect(hash.mod(2)).toBe(m2);
      expect(hash.mod(1021)).toBe(m1021);
      expect(visualSeed(row.text, n(row.start)).mod(997)).toBe(v997);
    }
  });
});
