import { describe, expect, it } from "vitest";
import type { Composition } from "../contract";
import {
  ALL_COMPOSITIONS,
  arrivalFor,
  arrivalOnly,
  deriveMood,
  hasConnections,
  IMPLEMENTED_COMPOSITIONS,
  matchingComposition,
  renderableComposition,
  resolveComposition,
  travelBounds,
  type ConcreteComposition,
} from "./index";

describe("composition mapping", () => {
  const m1 = new Set<ConcreteComposition>(["editorial", "spoken"]);

  it("maps every composition to an implemented one (rolling layouts to Spoken)", () => {
    for (const c of ALL_COMPOSITIONS) {
      const mapped = renderableComposition(c, m1);
      expect(m1.has(mapped), c).toBe(true);
      expect(arrivalOnly(mapped), c).toBe(arrivalOnly(c));
    }
    expect(renderableComposition("cascade", m1)).toBe("spoken");
    expect(renderableComposition("constellation", m1)).toBe("editorial");
    expect(renderableComposition("cutout", new Set<ConcreteComposition>([...m1, "cutout"]))).toBe("cutout");
  });

  it("automatic follows LyricComposition.matching, per line", () => {
    const calm = { energy: 0.3, organic: 0.8 };
    const busy = { energy: 0.8, organic: 0.2 };
    const middle = { energy: 0.5, organic: 0.5 };
    expect(matchingComposition(busy, 3)).toBe("constellation");
    expect(matchingComposition(calm, 3)).toBe("echo");
    expect(matchingComposition(middle, 2)).toBe("editorial");
    expect(matchingComposition(middle, 3)).toBe("scattered");
    expect(matchingComposition(middle, -3)).toBe("scattered");
    const all = new Set(ALL_COMPOSITIONS);
    expect(resolveComposition("automatic", busy, 1, all)).toBe("constellation");
    expect(resolveComposition("automatic", middle, 1, m1)).toBe("editorial");
    expect(resolveComposition("spoken", middle, 1, m1)).toBe("spoken");
  });

  it("every setting value resolves with the shipped implementation set", () => {
    const values: Composition[] = ["automatic", ...ALL_COMPOSITIONS];
    for (const value of values) {
      for (let line = 0; line < 4; line++) {
        expect(IMPLEMENTED_COMPOSITIONS.has(resolveComposition(value, { energy: 0.5, organic: 0.5 }, line))).toBe(true);
      }
    }
  });

  it("automatic arrivals match the Mac resolution", () => {
    expect(arrivalFor("editorial")).toBe("fade");
    expect(arrivalFor("spoken")).toBe("glide");
    expect(arrivalFor("scattered")).toBe("pop");
    expect(arrivalFor("constellation")).toBe("orbit");
    expect(arrivalFor("constellationEditorial")).toBe("lift");
    expect(arrivalFor("cutout")).toBe("cutoutFold");
    expect(travelBounds("glide", 100)).toEqual({ width: 144, height: 40 });
    expect(travelBounds("fade", 100)).toEqual({ width: 0, height: 0 });
    expect(travelBounds("orbit", 100, false)).toEqual({ width: 0, height: 0 });
    expect(hasConnections("constellationEditorial")).toBe(true);
  });
});

describe("track mood", () => {
  it("without a genre, cadence and cover saturation decide", () => {
    const slow = deriveMood({ wordsPerSecond: 0.8, artworkSaturation: 0.5 });
    const fast = deriveMood({ wordsPerSecond: 3.4, artworkSaturation: 0.5 });
    expect(slow.energy).toBeCloseTo(0.275, 10);
    expect(fast.energy).toBeCloseTo(0.725, 10);
    expect(fast.intensity).toBeCloseTo(0.25 + 0.725 * 1.3 + 0.1, 10);
    expect(deriveMood({ wordsPerSecond: 0, artworkSaturation: 1 }).organic).toBeCloseTo(0.58, 10);
    expect(deriveMood({ wordsPerSecond: 0, artworkSaturation: 0.5 }).energy).toBeCloseTo(0.5, 10);
  });
  it("genre tables keep the Mac's separate orders", () => {
    const mood = deriveMood({ genre: "Rap Rock", wordsPerSecond: 0, artworkSaturation: 0.5 });
    expect(mood.energy).toBeCloseTo(0.85, 10);
    expect(mood.organic).toBeCloseTo(0.3, 10);
  });
});
