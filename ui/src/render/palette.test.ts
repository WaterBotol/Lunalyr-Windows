import { describe, expect, it } from "vitest";
import { FALLBACK_HSB, rgbToHsb } from "../defaults";
import { blobLayout, scrimGradient } from "./background";
import { blurFilter, entranceClip, entranceTransform, wordEntrance } from "./entrance";
import { cjkLocale, contrastingFace, fontStack, isComplexScript, isRtl, lyricFont } from "./fonts";
import { accentColour, albumStops, artworkSaturation, hsbToRgb, interpolateStops, PaletteFade } from "./palette";

describe("album palette (AlbumWallpaperPalette.swift)", () => {
  it("converts HSB like the Mac and samples five stops", () => {
    expect(hsbToRgb([0, 1, 1])).toEqual([1, 0, 0]);
    expect(hsbToRgb([1 / 3, 1, 1]).map((v) => Math.round(v * 1000) / 1000)).toEqual([0, 1, 0]);
    expect(hsbToRgb([0.5, 0, 0.4])).toEqual([0.4, 0.4, 0.4]);
    const stops = albumStops(FALLBACK_HSB);
    expect(stops.length).toBe(5);
    // Round trip of the fallback colours (ColorExtractor.fallbackPalette cell 0).
    expect(stops[0].map((v) => Math.round(v * 100) / 100)).toEqual([0.23, 0.16, 0.58]);
    expect(rgbToHsb(0.23, 0.16, 0.58)[2]).toBeCloseTo(0.58, 10);
  });

  it("falls back per cell for short or invalid grids", () => {
    const stops = albumStops([[0, 1, 1]]);
    expect(stops[0]).toEqual([1, 0, 0]);
    expect(stops[1]).toEqual(albumStops(FALLBACK_HSB)[1]);
  });

  it("crossfades over 0.9 s with smoothstep", () => {
    const red: [number, number, number][] = Array(9).fill([0, 1, 1]);
    const blue: [number, number, number][] = Array(9).fill([2 / 3, 1, 1]);
    const fade = new PaletteFade(red);
    fade.set(blue, 1000);
    expect(fade.at(1000)[0][0]).toBeCloseTo(1, 6);
    expect(fade.at(1450)[0][0]).toBeCloseTo(0.5, 6);
    expect(fade.at(1900)[0]).toEqual(albumStops(blue)[0]);
    fade.set(red, -Infinity);
    expect(fade.at(2000)[0]).toEqual([1, 0, 0]);
    expect(interpolateStops([[0, 0, 0]], [[1, 1, 1]], 0.25)[0]).toEqual([0.25, 0.25, 0.25]);
  });

  it("accent keeps saturation 0.2–0.65 and brightness >= 0.78", () => {
    const grid: [number, number, number][] = Array(9).fill([0.5, 1, 0.2]);
    const [r, g, b] = accentColour(grid);
    expect(Math.max(r, g, b)).toBeCloseTo(0.78, 6);
    expect(1 - Math.min(r, g, b) / Math.max(r, g, b)).toBeCloseTo(0.65, 6);
    expect(artworkSaturation(grid)).toBe(1);
  });
});

describe("colour field", () => {
  it("places five blobs with the 1.2x overscan and soft edges", () => {
    const blobs = blobLayout(0, 1920, 1080);
    expect(blobs.length).toBe(5);
    // Slot 0 at t=0: cos(0)·0.26·W right of centre, scaled 1.2 about the centre.
    expect(blobs[0].x).toBeCloseTo(960 + 1.2 * Math.cos(0) * 0.26 * 1920, 6);
    expect(blobs[0].radius).toBeCloseTo(1.2 * 1080 * 0.72 * (1 + 0.05 * Math.sin(0)), 6);
    expect(blobs.every((b) => b.softness > 0.05 && b.softness < 0.2)).toBe(true);
    expect(scrimGradient(1)).toContain("rgba(0,0,0,0.34)");
    expect(scrimGradient(0)).toContain("rgba(0,0,0,0)");
  });
});

describe("word entrances (WordEntrance)", () => {
  it("fade only blurs and fades; glide travels from the side and below", () => {
    const fade = wordEntrance(0.5, "fade", 40, 1, 0.5, false);
    expect([fade.x, fade.y, fade.rotate, fade.scale]).toEqual([0, 0, 0, 1]);
    expect(fade.blur).toBeCloseTo(1.75, 10);
    const glide = wordEntrance(0, "glide", 40, -1, 0.25, false);
    expect(glide.x).toBeCloseTo(-0.72 * 40, 10);
    expect(glide.y).toBeCloseTo(0.2 * 40, 10);
    expect(entranceTransform(wordEntrance(1, "glide", 40, 1, 1, false), 100, 40)).toBe("none");
  });
  it("reduced motion removes travel, rotation and blur", () => {
    for (const style of ["glide", "orbit", "pop", "cutoutFold", "lift"] as const) {
      const e = wordEntrance(0.2, style, 40, 1, 0.3, true);
      expect([e.x, e.y, e.rotate, e.rotateX, e.scale, e.blur, e.reveal]).toEqual([0, 0, 0, 0, 1, 0, 1]);
      expect(e.opacity).toBe(0.3);
    }
  });
  it("cutout entrances unmask from the leading edge", () => {
    const e = wordEntrance(0.5, "cutoutFold", 40, 1, 1, false);
    expect(e.reveal).toBeCloseTo(0.94, 10);
    expect(entranceClip(e)).toBe("inset(0 6% 0 0)");
    expect(blurFilter(0.01)).toBe("none");
  });
});

describe("fonts and scripts", () => {
  it("pairs faces like the Mac and keeps locale-matched CJK fallbacks", () => {
    expect(contrastingFace("manrope")).toBe("serif");
    expect(contrastingFace("slab")).toBe("soft");
    expect(lyricFont("manrope", 40, true, true)).toEqual({ face: "serif", weight: 700, size: 40 });
    expect(fontStack("manrope", "zh").indexOf("Microsoft YaHei UI")).toBeLessThan(fontStack("manrope", "zh").indexOf("Yu Gothic UI"));
    expect(fontStack("manrope", "ja").indexOf("Yu Gothic UI")).toBeLessThan(fontStack("manrope", "ja").indexOf("Microsoft YaHei UI"));
    expect(fontStack("manrope").startsWith('"Manrope Variable"')).toBe(true);
  });
  it("detects CJK locale, direction and complex scripts", () => {
    expect(cjkLocale("夜空に光る")).toBe("ja");
    expect(cjkLocale("晚安 月亮")).toBe("zh");
    expect(cjkLocale("안녕")).toBe("ko");
    expect(cjkLocale("hello")).toBeNull();
    expect(isRtl("مساء الخير")).toBe(true);
    expect(isRtl("— שלום")).toBe(true);
    expect(isRtl("hello שלום")).toBe(false);
    expect(isComplexScript("नमस्ते")).toBe(true);
    expect(isComplexScript("hello")).toBe(false);
  });
});
