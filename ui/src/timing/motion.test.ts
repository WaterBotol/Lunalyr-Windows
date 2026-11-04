// Mirrors Tests/motion_test.c.
import { describe, expect, it } from "vitest";
import { motionEase, motionHandoff, rollingPose, Seed64, vocalGesture, type RollingStyle } from "./index";

const seed197 = Seed64.fromNumber(197);
const seed101 = Seed64.fromNumber(101);

describe("motion (motion_test.c)", () => {
  it("handoff is monotonic and bounded; ease stays inside its overshoot", () => {
    expect(motionHandoff(-1, 0.4)).toBe(0);
    expect(motionHandoff(2, 0.4)).toBe(1);
    let previous = 0;
    for (let i = 0; i <= 1000; i++) {
      const value = motionHandoff(i * 0.001, 0.4);
      expect(value).toBeGreaterThanOrEqual(previous);
      expect(value).toBeLessThanOrEqual(1);
      previous = value;
      const eased = motionEase(i * 0.001);
      expect(eased).toBeGreaterThanOrEqual(0);
      expect(eased).toBeLessThanOrEqual(1.12);
    }
  });

  for (const style of [0, 1, 2] as RollingStyle[]) {
    it(`rolling pose style ${style}: exits, energy, seeks, reduced motion, clearance, continuity`, () => {
      // Retires before it reaches a stage edge.
      expect(rollingPose(style, 5, 1, 10, seed197, 0.6, false).opacity).toBe(0);
      const a = rollingPose(style, 0, 0.5, 3, seed197, 0, false);
      const b = rollingPose(style, 0, 0.5, 3, seed197, 1, false);
      expect(b.scale).toBeGreaterThan(a.scale); // energy affects the active word
      const seek = rollingPose(style, 0, 0.5, 3, seed197, 1, false);
      expect(seek).toEqual(b); // independent of frame history
      expect(rollingPose(style, 1, 5, 0.4, seed197, 1, false).opacity).toBe(0); // no stale words in a long gap
      const reduced = rollingPose(style, 0, 0.5, 3, seed197, 1, true);
      expect(reduced.angle).toBe(0);
      expect(reduced.blur).toBe(0);
      const hero = rollingPose(style, 0, 0.2, 0.4, seed197, 0, true);
      const neighbour = rollingPose(style, 1, 0.2, 0.4, seed197, 0, true);
      expect(hero.y - neighbour.y).toBeGreaterThan(1.2); // two large words have clearance
      for (let i = 0; i < 900; i++) {
        const r = -1 + i * 0.01;
        const p = rollingPose(style, r, 0.5, 2, seed197, 0.8, false);
        const q = rollingPose(style, r + 0.00001, 0.5, 2, seed197, 0.8, false);
        expect(Math.abs(p.y - q.y)).toBeLessThan(0.001);
        expect(Math.abs(p.opacity - q.opacity)).toBeLessThan(0.001);
        expect(Math.abs(p.scale - q.scale)).toBeLessThan(0.001);
        expect(Math.abs(p.angle - q.angle)).toBeLessThan(0.001);
        expect(Number.isFinite(p.scale) && p.scale > 0.3 && p.scale < 1.5).toBe(true);
      }
    });
  }

  it("vocal gesture settles, moves while held, and survives invalid input", () => {
    const settled = vocalGesture(30, 0.4, seed101, 0.7, false);
    expect([settled.x, settled.y, settled.scale]).toEqual([0, 0, 1]);
    const first = vocalGesture(0.7, 2, seed101, 0.3, false);
    const held = vocalGesture(1.2, 2, seed101, 0.3, false);
    expect(first.y).not.toBe(held.y);
    const invalid = rollingPose(0, Number.NaN, Number.POSITIVE_INFINITY, Number.NaN, Seed64.fromNumber(0), Number.NaN, false);
    expect(Number.isFinite(invalid.x) && Number.isFinite(invalid.y) && Number.isFinite(invalid.opacity)).toBe(true);
  });

  it("writes into a caller-owned pose without allocating a new one", () => {
    const out = { x: 9, y: 9, scale: 9, angle: 9, opacity: 9, blur: 9 };
    const returned = rollingPose(0, 0, 0.5, 1, seed197, 0, false, out);
    expect(returned).toBe(out);
    expect(out.scale).not.toBe(9);
  });
});
