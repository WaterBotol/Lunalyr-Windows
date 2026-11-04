// Mirrors the stage-fit tests in RenderingPerformanceTests.swift.
import { describe, expect, it } from "vitest";
import { presentationScale, previewScale, stageFitScale, viewportMetrics, wordStageFit } from "./stage";

describe("stage fitting", () => {
  it("tall lines and readings fit short or portrait displays", () => {
    for (const available of [{ width: 900, height: 400 }, { width: 400, height: 900 }]) {
      const content = { width: 1400, height: 1700 };
      const fit = stageFitScale(content, available);
      expect(content.width * fit).toBeLessThanOrEqual(available.width + 0.001);
      expect(content.height * fit).toBeLessThanOrEqual(available.height + 0.001);
    }
  });

  it("previews preserve the display aspect and the lyric stage", () => {
    for (const size of [{ width: 1512, height: 982 }, { width: 1080, height: 1920 }, { width: 3440, height: 1440 }]) {
      const layout = viewportMetrics(size);
      const scale = previewScale(size, { width: 256, height: 144 });
      const preview = { width: size.width * scale, height: size.height * scale };
      expect(preview.width / preview.height).toBeCloseTo(size.width / size.height, 4);
      expect(preview.width).toBeLessThanOrEqual(256.001);
      expect(preview.height).toBeLessThanOrEqual(144.001);
      const stage = layout.stage;
      const content = { width: stage.width * 2, height: stage.height * 3 };
      for (const user of [0.5, 1, 1.5, 2]) {
        const fit = presentationScale(content, stage, user);
        expect(content.width * fit).toBeLessThanOrEqual(stage.width + 0.001);
        expect(content.height * fit).toBeLessThanOrEqual(stage.height + 0.001);
      }
    }
  });

  it("short content is shown at the user's size, long content at 75% of the stage", () => {
    const stage = { width: 1536, height: 777.6 };
    expect(presentationScale({ width: 400, height: 100 }, stage, 1)).toBe(1);
    expect(presentationScale({ width: 400, height: 100 }, stage, 1.5)).toBe(1.5);
    expect(presentationScale({ width: 400, height: 100 }, stage, 0.1)).toBe(0.5);
    expect(presentationScale({ width: 1536, height: 300 }, stage, 1)).toBeCloseTo(0.75, 10);
  });

  it("base size and stage follow the Mac viewport metrics", () => {
    const m = viewportMetrics({ width: 1920, height: 1080 });
    expect(m.baseSize).toBeCloseTo(88.32, 6);
    expect(m.stage).toEqual({ width: 1536, height: 777.6 });
    expect(viewportMetrics({ width: 400, height: 300 }).baseSize).toBe(24);
    expect(viewportMetrics({ width: 6000, height: 3000 }).baseSize).toBe(124);
  });

  it("wide rotated glyphs stay inside the rolling stage", () => {
    const ink = { width: 1800 + 22, height: 150 + 6 };
    for (const centre of [150, 380, 640]) {
      for (const angle of [-16, 0, 16]) {
        const fit = wordStageFit(ink, { width: 800, height: 500 }, { x: centre, y: 90 }, 1.3, angle, 30);
        const radians = (angle * Math.PI) / 180;
        const extent = ((ink.width * Math.abs(Math.cos(radians)) + ink.height * Math.abs(Math.sin(radians))) * 1.3 * fit) / 2;
        expect(fit).toBeGreaterThan(0);
        expect(centre - extent).toBeGreaterThanOrEqual(30 - 0.001);
        expect(centre + extent).toBeLessThanOrEqual(770 + 0.001);
        const vertical = ((ink.height * Math.abs(Math.cos(radians)) + ink.width * Math.abs(Math.sin(radians))) * 1.3 * fit) / 2;
        expect(vertical).toBeLessThanOrEqual(60 + 0.001);
      }
    }
  });

  it("does not enlarge short words and rejects invalid geometry", () => {
    expect(wordStageFit({ width: 80, height: 40 }, { width: 800, height: 500 }, { x: 400, y: 250 }, 1.2, 5, 30)).toBe(1);
    expect(wordStageFit({ width: Infinity, height: 40 }, { width: 800, height: 500 }, { x: 400, y: 250 }, 1.2, 5, 30)).toBe(0);
  });
});
