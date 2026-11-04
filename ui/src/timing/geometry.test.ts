// Mirrors Tests/geometry_test.c (envelope, reveal, visibility, settle, gaps,
// line-end estimate, links and the connection memo).
import { describe, expect, it } from "vitest";
import {
  buildConnections,
  ConnectionCache,
  estimatedLineEnd,
  vocalGap,
  wordEnvelope,
  wordReveal,
  wordSettle,
  wordVisibility,
  type Box,
} from "./index";

function inside(b: Box, x: number, y: number): boolean {
  return x > b.x + 1e-6 && x < b.x + b.width - 1e-6 && y > b.y + 1e-6 && y < b.y + b.height - 1e-6;
}

describe("geometry (geometry_test.c)", () => {
  it("every rotated, growing ink corner fits the fixed envelope", () => {
    for (let angle = 0; angle <= 20; angle++) {
      const envelope = wordEnvelope(240, 42, 1.2, angle);
      for (let phase = 0; phase <= 100; phase++) {
        const rotation = angle * (phase / 100) * 0.017453292519943295;
        const scale = 0.8 + 0.377 * (phase / 100);
        for (const x of [-1, 1]) for (const y of [-1, 1]) {
          const px = x * 120 * scale;
          const py = y * 21 * scale;
          const rx = px * Math.cos(rotation) - py * Math.sin(rotation);
          const ry = px * Math.sin(rotation) + py * Math.cos(rotation);
          expect(Math.abs(rx)).toBeLessThanOrEqual(envelope.width / 2 + 1e-8);
          expect(Math.abs(ry)).toBeLessThanOrEqual(envelope.height / 2 + 1e-8);
        }
      }
    }
    expect(wordEnvelope(Number.NaN, 42, 1.2, 10).width).toBe(0);
  });

  it("arrival is monotonic during playback and recomputes on seek", () => {
    let previous = 0;
    for (let i = 0; i < 500; i++) {
      const value = wordReveal(i / 100, 1, 2);
      expect(value).toBeGreaterThanOrEqual(previous);
      expect(value).toBeLessThanOrEqual(1);
      previous = value;
    }
    expect(wordReveal(0.9, 1, 2)).toBe(0);
    expect(wordReveal(1.1, 1, 2)).toBeGreaterThan(0);
    expect(wordReveal(1.1, 1, 2)).toBeLessThan(1);
    expect(wordReveal(3, 1, 2)).toBe(1);
    expect(wordReveal(0.5, 1, 2)).toBe(0); // backwards seek
    expect(wordReveal(Number.NaN, 1, 2)).toBe(0);
  });

  it("visibility previews exactly one word without a blink at the handoff", () => {
    expect(wordVisibility(0.5, 1, 2, true, true, 0.5)).toBe(0);
    expect(wordVisibility(0.75, 1, 2, true, true, 0.5)).toBeCloseTo(0.125, 8);
    expect(wordVisibility(0.5, 1, 2, true, false, 0.5)).toBe(0);
    expect(wordVisibility(0.5, 1, 2, false, true, 0.5)).toBe(0);
    expect(wordVisibility(1, 1, 2, true, false, 0.5)).toBe(0.25);
    expect(wordVisibility(1.1, 1, 2, true, false, 0.5)).toBeGreaterThan(0.25);
    expect(wordVisibility(3, 1, 2, true, false, 0.5)).toBe(1);
    for (let i = 0; i <= 100; i++) {
      const p = i / 100;
      expect(wordSettle(p)).toBeGreaterThanOrEqual(0);
      expect(wordSettle(p)).toBeLessThanOrEqual(1.05);
    }
  });

  it("finds instrumental gaps", () => {
    const vocals = [{ start: 10, end: 14 }, { start: 22, end: 28 }];
    expect(vocalGap(vocals, 5, 40, 2.4)).toEqual({ start: 0, end: 10, nextVocal: 0 });
    expect(vocalGap(vocals, 12, 40, 2.4)).toBeNull();
    expect(vocalGap(vocals, 18, 40, 2.4)).toEqual({ start: 14, end: 22, nextVocal: 1 });
    expect(vocalGap(vocals, 32, 40, 2.4)).toEqual({ start: 28, end: 40, nextVocal: -1 });
    expect(vocalGap(vocals, 40, 40, 2.4)).toBeNull();
    expect(vocalGap([{ start: 0, end: 5 }, { start: 6, end: 10 }], 5.5, 20, 2.4)).toBeNull(); // a breath
    const overlapping = [{ start: 0, end: 8 }, { start: 4, end: 10 }, { start: 14, end: 18 }];
    expect(vocalGap(overlapping, 6, 22, 2.4)).toBeNull();
    expect(vocalGap(overlapping, 12, 22, 2.4)).toMatchObject({ start: 10, end: 14 });
    expect(vocalGap([], 5, 20, 2.4)?.nextVocal).toBe(-1);
    expect(vocalGap(vocals, Number.NaN, 40, 2.4)).toBeNull();
    // Unknown duration: the final gap has no end.
    expect(vocalGap(vocals, 32, 0, 2.4)).toEqual({ start: 28, end: -1, nextVocal: -1 });
  });

  it("estimates a conservative end for line-only lyrics before a long break", () => {
    expect(estimatedLineEnd(10, 14, 4, 0.45)).toBe(14);
    expect(estimatedLineEnd(10, 50, 4, 0.45)).toBeLessThan(17);
    expect(estimatedLineEnd(10, 30, 20, 0.5)).toBe(30);
    expect(estimatedLineEnd(10, 90, 4, Number.NaN)).toBeLessThan(17);
  });

  it("builds clipped links that never cross a word", () => {
    const view = { x: 0, y: 0, width: 1200, height: 700 };
    const horizontal = [{ x: 20, y: 100, width: 100, height: 40 }, { x: 300, y: 100, width: 200, height: 40 }];
    let links = buildConnections(horizontal, view, 6);
    expect(links.length).toBe(1);
    expect(links[0].ax).toBeCloseTo(126, 6);
    expect(links[0].bx).toBeCloseTo(294, 6);
    expect(links[0].ay).toBeCloseTo(120, 6);
    expect(links[0].by).toBeCloseTo(120, 6);
    const vertical = [{ x: 100, y: 20, width: 50, height: 60 }, { x: 100, y: 220, width: 50, height: 60 }];
    links = buildConnections(vertical, view, 6);
    expect(links.length).toBe(1);
    expect(links[0].ay).toBeCloseTo(86, 6);
    expect(links[0].by).toBeCloseTo(214, 6);
    expect(buildConnections([{ x: 20, y: 30, width: 150, height: 80 }, { x: 70, y: 50, width: 100, height: 80 }], view, 6)).toEqual([]);
    expect(buildConnections([{ x: Number.NaN, y: 30, width: 50, height: 50 }, { x: 100, y: 50, width: 40, height: 40 }], view, 6)).toEqual([]);
    expect(buildConnections([{ x: -500, y: -500, width: 30, height: 30 }, { x: 100, y: 50, width: 40, height: 40 }], view, 6)).toEqual([]);
    expect(buildConnections(horizontal, view, 6, 0)).toEqual([]);

    const words: Box[] = [
      { x: 30, y: 300, width: 110, height: 45 }, { x: 220, y: 80, width: 140, height: 50 }, { x: 490, y: 125, width: 60, height: 35 },
      { x: 650, y: 350, width: 400, height: 120 }, { x: 140, y: 530, width: 160, height: 50 }, { x: 550, y: 550, width: 90, height: 40 },
    ];
    const check = (segments: ReturnType<typeof buildConnections>) => {
      for (const s of segments) {
        expect([s.ax, s.ay, s.bx, s.by].every(Number.isFinite)).toBe(true);
        expect(s.ax >= 0 && s.ax <= 1200 && s.bx >= 0 && s.bx <= 1200).toBe(true);
        expect(s.ay >= 0 && s.ay <= 700 && s.by >= 0 && s.by <= 700).toBe(true);
        for (const w of words) {
          expect(inside(w, s.ax, s.ay)).toBe(false);
          expect(inside(w, s.bx, s.by)).toBe(false);
          for (let t = 1; t < 100; t++) {
            const f = t / 100;
            expect(inside(w, s.ax + (s.bx - s.ax) * f, s.ay + (s.by - s.ay) * f)).toBe(false);
          }
        }
      }
    };
    links = buildConnections(words, view, 8);
    expect(links.length).toBeGreaterThan(1);
    expect(links.length).toBeLessThanOrEqual(5);
    check(links);
    // A word growing to its peak stays clear of every existing connection.
    const grown = words.map((w, i) => (i === 3 ? { ...w, width: w.width * 1.1, height: w.height * 1.1 } : w));
    const grownLinks = buildConnections(grown, view, 8);
    expect(grownLinks.length).toBeLessThanOrEqual(5);

    // Redraws reuse geometry; only actual layout changes rebuild it.
    const memo = new ConnectionCache();
    for (let frame = 0; frame < 200; frame++) {
      const found = memo.get(grown, view, 8);
      expect(found.length).toBe(grownLinks.length);
      expect(memo.builds).toBe(1);
    }
    expect(memo.get(grown, view, 8, 1).length).toBe(1);
    expect(memo.builds).toBe(1);
    memo.get(grown, { ...view, width: 1000 }, 8);
    expect(memo.builds).toBe(2);
    const wider = grown.map((w, i) => (i === 0 ? { ...w, width: w.width + 10 } : w));
    memo.get(wider, { ...view, width: 1000 }, 8);
    expect(memo.builds).toBe(3);
    memo.get(wider, { ...view, width: 1000 }, 9);
    expect(memo.builds).toBe(4);
    memo.get(wider.slice(0, 5), { ...view, width: 1000 }, 9);
    expect(memo.builds).toBe(5);
    const empty = new ConnectionCache();
    const overlap = [{ x: 20, y: 30, width: 150, height: 80 }, { x: 70, y: 50, width: 100, height: 80 }];
    for (let frame = 0; frame < 10; frame++) expect(empty.get(overlap, view, 6).length).toBe(0);
    expect(empty.builds).toBe(1); // empty results are cached too
    expect(memo.get(new Array(65).fill(words[0]), view, 8).length).toBe(0);
  });
});
