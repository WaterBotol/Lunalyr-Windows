// Default values for every slice of the contract. Settings mirror
// crates/lunalyr-core/src/settings.rs `Settings::default()`; they are used
// before the first snapshot arrives and by the browser demo host.

import type { LookupInfo, LyricsView, Palette, PlaybackClock, RenderState, Settings, Snapshot, SourceId, TrackPayload } from "./contract";

export const SOURCE_IDS: readonly SourceId[] = ["localFiles", "lyricsPlus", "netEase", "lrclib", "qqMusic", "kugou", "kuwo", "lyricsOvh"];

export const SOURCE_TITLES: Record<SourceId, string> = {
  localFiles: "Local lyric files",
  lyricsPlus: "Lyrics+ / KPoe",
  netEase: "NetEase Cloud Music",
  lrclib: "LRCLIB",
  qqMusic: "QQ Music",
  kugou: "Kugou Music",
  kuwo: "Kuwo Music",
  lyricsOvh: "lyrics.ovh",
};

export const SOURCE_TIMING: Record<SourceId, string> = {
  localFiles: "LRC / TTML / YRC + readings",
  lyricsPlus: "Word / syllable timing",
  netEase: "Word or line timing",
  lrclib: "Word or line timing",
  qqMusic: "Line timing",
  kugou: "Line timing",
  kuwo: "Line timing",
  lyricsOvh: "Plain text fallback",
};

export function defaultSettings(): Settings {
  return {
    composition: "editorial",
    lyricScale: 1,
    frameRate: 30,
    pauseWhenCovered: true,
    lyricOffset: 0,
    nextWordPreview: true,
    nextWordPreviewLead: 0.45,
    wordReveal: true,
    showInterludes: true,
    noLyrics: "notice",
    scrimStrength: 1,
    vividness: 1,
    effectSpeed: 1,
    font: "manrope",
    background: "colour",
    visualizer: "wavefield",
    audioReactive: false,
    audioSensitivity: 1,
    readings: "off",
    toneMarks: true,
    displays: "all",
    launchAtLogin: false,
    reduceMotion: false,
    sources: {
      order: [...SOURCE_IDS],
      enabled: SOURCE_IDS.filter((s) => s !== "localFiles"),
      mode: "smart",
      preferWordTiming: true,
      preferWordTimedLrclib: true,
      repairQqGaps: true,
      localFolder: "",
    },
    songOffsets: {},
  };
}

/** Value ranges enforced by the Rust side (`Settings::sanitized`). */
export const SETTING_RANGES = {
  lyricScale: [0.5, 2],
  lyricOffset: [-30, 30],
  nextWordPreviewLead: [0.1, 2],
  scrimStrength: [0, 1.5],
  vividness: [0, 1.5],
  effectSpeed: [0.25, 2],
  audioSensitivity: [0.1, 3],
} as const;

/** ColorExtractor.fallbackPalette (sRGB), used before artwork resolves. */
const FALLBACK_RGB: [number, number, number][] = [
  [0.23, 0.16, 0.58], [0.12, 0.4, 0.44], [0.65, 0.28, 0.43],
  [0.15, 0.3, 0.56], [0.54, 0.25, 0.16], [0.43, 0.19, 0.56],
  [0.1, 0.46, 0.4], [0.55, 0.34, 0.16], [0.3, 0.19, 0.52],
];

/** NSColor.getHue(_:saturation:brightness:) for sRGB components. */
export function rgbToHsb(r: number, g: number, b: number): [number, number, number] {
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const delta = max - min;
  let hue = 0;
  if (delta > 0) {
    if (max === r) hue = ((g - b) / delta) % 6;
    else if (max === g) hue = (b - r) / delta + 2;
    else hue = (r - g) / delta + 4;
    hue /= 6;
    if (hue < 0) hue += 1;
  }
  return [hue, max === 0 ? 0 : delta / max, max];
}

export const FALLBACK_HSB: [number, number, number][] = FALLBACK_RGB.map(([r, g, b]) => rgbToHsb(r, g, b));

export function fallbackPalette(): Palette {
  return { hsb: FALLBACK_HSB.map((c) => [...c] as [number, number, number]), fromArtwork: false };
}

export function emptyTrack(): TrackPayload {
  return { track: null, palette: fallbackPalette(), idleSince: null };
}

export function idleLyrics(): LyricsView {
  return { status: "idle" };
}

export function stoppedClock(): PlaybackClock {
  return { position: 0, sampledAt: 0, playing: false, rate: 1, offset: 0 };
}

export function runningRender(): RenderState {
  return { paused: false, reason: "" };
}

export function emptyLookup(): LookupInfo {
  return { candidates: [], states: [], pinned: false, cached: false, searching: false };
}

export function emptySnapshot(): Snapshot {
  return {
    track: emptyTrack(),
    lyrics: idleLyrics(),
    clock: stoppedClock(),
    settings: defaultSettings(),
    render: runningRender(),
    lookup: emptyLookup(),
    version: "",
  };
}
