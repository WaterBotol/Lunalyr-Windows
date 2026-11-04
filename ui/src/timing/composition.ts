// Lyric compositions and word arrivals (Sources/UI/LyricComposition.swift),
// plus the Windows mapping from the `Composition` setting to what is drawn.

import type { Composition, Settings } from "../contract";

export type ConcreteComposition = Exclude<Composition, "automatic">;

export const ALL_COMPOSITIONS: readonly ConcreteComposition[] = [
  "scattered",
  "constellation",
  "constellationEditorial",
  "echo",
  "editorial",
  "spoken",
  "cascade",
  "cutout",
];

export const COMPOSITION_TITLES: Record<Composition, string> = {
  automatic: "Match the music",
  scattered: "Scattered",
  constellation: "Constellation",
  constellationEditorial: "Constellation + Editorial",
  echo: "Echo",
  editorial: "Editorial",
  spoken: "Spoken",
  cascade: "Cascade",
  cutout: "Cutout",
};

export const COMPOSITION_DETAILS: Record<Composition, string> = {
  automatic: "The track's energy and cadence pick a layout line by line.",
  scattered: "Words set loose across the line.",
  constellation: "Fragments linked by hairlines, like a star chart.",
  constellationEditorial: "Connected fragments with Editorial’s contrasting typefaces.",
  echo: "The phrase repeated at many sizes, some inverted.",
  editorial: "Two typefaces in one line, set like a spread.",
  spoken: "Each sung word takes focus; recent words settle around it.",
  cascade: "A moving column of words, arriving with the voice.",
  cutout: "Uneven type and album-colour labels, built word by word.",
};

export function hasConnections(c: ConcreteComposition): boolean {
  return c === "constellation" || c === "constellationEditorial";
}

export function hasEditorialTypography(c: ConcreteComposition): boolean {
  return c === "editorial" || c === "constellationEditorial";
}

/** Rolling layouts: words arrive one by one into a free stage (SpokenTypographyView). */
export function arrivalOnly(c: ConcreteComposition): boolean {
  return c === "spoken" || c === "cascade" || c === "cutout";
}

/** `LyricComposition.matching`: coarse, per-track choice for "Match the music". */
export function matchingComposition(mood: { energy: number; organic: number }, seed: number): ConcreteComposition {
  if (mood.energy > 0.68 && mood.organic < 0.45) return "constellation";
  if (mood.energy < 0.38 && mood.organic > 0.62) return "echo";
  return Math.abs(seed) % 2 === 0 ? "editorial" : "scattered";
}

/**
 * Compositions the renderer draws itself. Anything else is shown with the
 * nearest implemented layout: rolling layouts as Spoken, the others as
 * Editorial.
 */
export const IMPLEMENTED_COMPOSITIONS: ReadonlySet<ConcreteComposition> = new Set<ConcreteComposition>(ALL_COMPOSITIONS);

export function renderableComposition(
  c: ConcreteComposition,
  implemented: ReadonlySet<ConcreteComposition> = IMPLEMENTED_COMPOSITIONS,
): ConcreteComposition {
  if (implemented.has(c)) return c;
  return arrivalOnly(c) ? "spoken" : "editorial";
}

/** `Settings.resolvedComposition(for:seed:)`, then mapped to an implemented layout. */
export function resolveComposition(
  setting: Composition,
  mood: { energy: number; organic: number },
  lineIndex: number,
  implemented: ReadonlySet<ConcreteComposition> = IMPLEMENTED_COMPOSITIONS,
): ConcreteComposition {
  const chosen = setting === "automatic" ? matchingComposition(mood, lineIndex) : setting;
  return renderableComposition(chosen, implemented);
}

// MARK: word arrivals

export type WordArrival =
  | "fade"
  | "lift"
  | "pop"
  | "orbit"
  | "glide"
  | "unfold"
  | "cutoutFold"
  | "cutoutSlide";

/** `WordArrival.automatic.resolved(for:)` — Windows has no per-style arrival picker. */
export function arrivalFor(c: ConcreteComposition): WordArrival {
  switch (c) {
    case "scattered":
      return "pop";
    case "constellation":
      return "orbit";
    case "constellationEditorial":
    case "echo":
      return "lift";
    case "editorial":
      return "fade";
    case "spoken":
      return "glide";
    case "cascade":
      return "lift";
    case "cutout":
      return "cutoutFold";
  }
}

export function isCutoutArrival(a: WordArrival): boolean {
  return a === "cutoutFold" || a === "cutoutSlide";
}

export function requiresReveal(a: WordArrival): boolean {
  return isCutoutArrival(a);
}

export function travelRotation(a: WordArrival): number {
  return a === "orbit" || a === "cutoutSlide" ? 12 : 0;
}

/** `WordArrival.travelBounds`: room reserved for the entrance's travel. */
export function travelBounds(a: WordArrival, pointSize: number, active = true): { width: number; height: number } {
  if (!active) return { width: 0, height: 0 };
  const horizontal = a === "orbit" ? 0.32 : a === "glide" || a === "cutoutSlide" ? 0.72 : 0;
  const vertical = a === "lift" ? 0.48 : a === "glide" || a === "orbit" ? 0.2 : 0;
  return { width: horizontal * pointSize * 2, height: vertical * pointSize * 2 };
}

// MARK: settings behaviour (Settings.previewsNextWord / revealsWords)

export interface WordBehaviour {
  previews: boolean;
  reveals: boolean;
  lead: number;
}

/** Windows keeps one preview/reveal setting for all layouts; entrances that need a reveal force it on. */
export function wordBehaviour(settings: Pick<Settings, "nextWordPreview" | "wordReveal" | "nextWordPreviewLead">, c: ConcreteComposition): WordBehaviour {
  return {
    previews: settings.nextWordPreview,
    reveals: arrivalOnly(c) || requiresReveal(arrivalFor(c)) || settings.wordReveal,
    lead: settings.nextWordPreviewLead,
  };
}
