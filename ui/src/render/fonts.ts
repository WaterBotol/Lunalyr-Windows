// Typefaces. The Mac's four lyric "roles" (TrackMood.swift `LyricFace`) map
// to faces that ship with Windows, with bundled Manrope as the default:
//
//   manrope → Manrope (SIL OFL, bundled)        contrasting face: serif
//   system  → Segoe UI Variable                  contrasting face: serif
//   serif   → Georgia / Sitka Text / Cambria     contrasting face: manrope
//   slab    → Rockwell / Roboto Slab / Georgia   contrasting face: soft
//   soft    → Segoe UI Variable Display …        contrasting face: slab
//
// Editorial's second voice uses the contrasting face of the selected one
// (the Mac pairs sans↔serif and slab↔soft). CJK lines get a locale-matched
// fallback order so Chinese text is not set in Japanese glyph shapes.

import type { FontChoice } from "../contract";

export interface FontSpec {
  face: FontChoice;
  weight: number;
  size: number;
}

interface FaceInfo {
  families: string[];
  regular: number;
  bold: number;
  ui: number;
}

const FACES: Record<FontChoice, FaceInfo> = {
  manrope: { families: ['"Manrope Variable"', '"Segoe UI Variable Text"', '"Segoe UI Variable"', '"Segoe UI"'], regular: 400, bold: 700, ui: 500 },
  system: { families: ['"Segoe UI Variable Display"', '"Segoe UI Variable"', '"Segoe UI"', '"Manrope Variable"'], regular: 400, bold: 700, ui: 400 },
  serif: { families: ["Georgia", '"Sitka Text"', "Cambria", '"Times New Roman"'], regular: 400, bold: 700, ui: 400 },
  slab: { families: ["Rockwell", '"Roboto Slab"', '"Rockwell Nova"', "Georgia"], regular: 400, bold: 700, ui: 400 },
  soft: { families: ['"Segoe UI Variable Display"', '"Segoe UI Variable"', '"Segoe UI"', '"Manrope Variable"'], regular: 500, bold: 800, ui: 500 },
};

const GENERIC: Record<FontChoice, string> = {
  manrope: "system-ui, sans-serif",
  system: "system-ui, sans-serif",
  serif: "serif",
  slab: "serif",
  soft: "system-ui, sans-serif",
};

export type CjkLocale = "ja" | "zh" | "ko";

const CJK: Record<CjkLocale | "default", string[]> = {
  default: ['"Yu Gothic UI"', '"Microsoft YaHei UI"', '"Malgun Gothic"'],
  ja: ['"Yu Gothic UI"', "Meiryo", '"Microsoft YaHei UI"', '"Malgun Gothic"'],
  zh: ['"Microsoft YaHei UI"', '"Microsoft JhengHei UI"', '"Yu Gothic UI"', '"Malgun Gothic"'],
  ko: ['"Malgun Gothic"', '"Yu Gothic UI"', '"Microsoft YaHei UI"'],
};

const stackCache = new Map<string, string>();

export function fontStack(face: FontChoice, locale: CjkLocale | null = null): string {
  const key = `${face}:${locale ?? ""}`;
  let stack = stackCache.get(key);
  if (!stack) {
    const fallback = CJK[locale ?? "default"];
    // Serif faces fall back to a CJK serif-less stack too: Windows has no
    // matching Mincho/Song for every locale, and a mixed sans is calmer.
    stack = [...FACES[face].families, ...fallback, GENERIC[face]].join(", ");
    stackCache.set(key, stack);
  }
  return stack;
}

export function contrastingFace(face: FontChoice): FontChoice {
  switch (face) {
    case "manrope":
    case "system":
      return "serif";
    case "serif":
      return "manrope";
    case "slab":
      return "soft";
    case "soft":
      return "slab";
  }
}

/** `Settings.lyricFont(size:hero:mood:seed:alternate:)` for Windows. */
export function lyricFont(face: FontChoice, size: number, hero: boolean, alternate = false): FontSpec {
  const chosen = alternate ? contrastingFace(face) : face;
  const info = FACES[chosen];
  return { face: chosen, weight: hero ? info.bold : info.regular, size };
}

/** `Settings.uiFont`: notices and the track name's artist line. */
export function uiFont(face: FontChoice, size: number, bold = false): FontSpec {
  const info = FACES[face];
  return { face, weight: bold ? info.bold : info.ui, size };
}

/** Apply a font to an element (family is per face + CJK locale). */
export function applyFont(el: HTMLElement, font: FontSpec, locale: CjkLocale | null): void {
  el.style.fontFamily = fontStack(font.face, locale);
  el.style.fontWeight = String(font.weight);
  el.style.fontSize = `${font.size}px`;
}

// MARK: script detection

const KANA = /[぀-ヿㇰ-ㇿｦ-ﾟ]/;
const HANGUL = /[ᄀ-ᇿ㄰-㆏가-힯]/;
const HAN = /[㐀-䶿一-鿿豈-﫿]/;
const RTL_LETTER = /[\u{0590}-\u{08FF}\u{FB1D}-\u{FDFF}\u{FE70}-\u{FEFC}\u{10800}-\u{10FFF}\u{1E800}-\u{1EFFF}]/u;
const STRONG = /[\p{L}]/u;
/** Scripts whose letters join or reorder: never set them glyph by glyph. */
const COMPLEX = /[֐-ࣿऀ-෿฀-໿ༀ-࿿က-႟ក-៿᠀-᢯יִ-﷿ﹰ-ﻼ]/;

export function cjkLocale(text: string): CjkLocale | null {
  if (KANA.test(text)) return "ja";
  if (HANGUL.test(text)) return "ko";
  if (HAN.test(text)) return "zh";
  return null;
}

/** Direction from the first strong letter (Unicode bidi P2/P3, letters only). */
export function isRtl(text: string): boolean {
  for (const ch of text) {
    if (STRONG.test(ch)) return RTL_LETTER.test(ch);
  }
  return false;
}

export function isComplexScript(text: string): boolean {
  return COMPLEX.test(text);
}

export function htmlLang(locale: CjkLocale | null): string {
  return locale === "zh" ? "zh-Hans" : locale ?? "";
}
