// Renderer contract — TypeScript mirror of crates/lunalyr-core/src/contract.rs,
// model.rs and settings.rs. See docs/CONTRACT.md. Change all of them together.
//
// Times on the track timeline are SECONDS. Wall-clock instants are Unix epoch
// MILLISECONDS, comparable with Date.now().

export interface Word {
  text: string;
  start: number;
  end: number;
}

export interface Line {
  start: number;
  end: number;
  text: string;
  words: Word[];
  isInstrumental: boolean;
  wordTimed: boolean;
  romanization?: string | null;
}

export interface Provenance {
  /** "Word-by-word" | "Mixed timing" | "Line-synced" | "Not synced" */
  precision: string;
  source: string;
}

export type SourceId =
  | "localFiles"
  | "lyricsPlus"
  | "netEase"
  | "lrclib"
  | "qqMusic"
  | "kugou"
  | "kuwo"
  | "lyricsOvh";

export interface Track {
  title: string;
  artist: string;
  album: string | null;
  /** Seconds; 0 when unknown. */
  duration: number;
  sourceApp: string;
  recordingKey: string;
  /** Stable per-track seed (macOS trackSeed). */
  seed: number;
}

export interface Palette {
  /** 9 cells of [hue, saturation, brightness] in 0..1, 3x3 row-major from top-left. */
  hsb: [number, number, number][];
  fromArtwork: boolean;
}

export interface TrackPayload {
  track: Track | null;
  palette: Palette;
  /** Epoch ms when playback stopped; null while a session exists. */
  idleSince: number | null;
}

/** t = position + (playing ? (Date.now() - sampledAt) / 1000 * rate : 0) + offset */
export interface PlaybackClock {
  position: number;
  sampledAt: number;
  playing: boolean;
  rate: number;
  offset: number;
}

export type LyricsView =
  | { status: "idle" }
  | { status: "loading" }
  | {
      status: "synced";
      lines: Line[];
      provenance: Provenance;
      /** Per line: reading words (same count as line.words) or null. Empty when readings are off. */
      readings: (Word[] | null)[];
    }
  | { status: "plain"; lines: string[]; provenance: Provenance }
  | { status: "notFound"; reportedAt: number };

export interface RenderState {
  paused: boolean;
  /** "covered" | "user" | "session-locked" | "" */
  reason: string;
}

export interface CandidateSummary {
  source: SourceId;
  sourceTitle: string;
  title: string;
  artist: string;
  duration: number | null;
  precision: string;
  lineCount: number;
  hasReadings: boolean;
  chosen: boolean;
}

export interface LookupInfo {
  candidates: CandidateSummary[];
  /** [source, human-readable state] pairs. */
  states: [SourceId, string][];
  pinned: boolean;
  cached: boolean;
}

export interface AudioFrame {
  level: number;
  bass: number;
  mid: number;
  treble: number;
  beat: number;
  flux: number;
  centroid: number;
  /** Log-spaced magnitude bands, 0..1. */
  bands: number[];
  /** Base64 little-endian f32 mono PCM window (optional). */
  pcm?: string;
  sampleRate: number;
}

export type Composition =
  | "automatic"
  | "scattered"
  | "constellation"
  | "constellationEditorial"
  | "echo"
  | "editorial"
  | "spoken"
  | "cascade"
  | "cutout";

export type FontChoice = "manrope" | "system" | "serif" | "slab" | "soft";
export type BackgroundMode = "colour" | "visualizer";
export type VisualizerStyle = "wavefield" | "spectrum";
export type NoLyricsBehaviour = "notice" | "trackName" | "nothing";
export type ReadingDisplay = "off" | "below" | "replace";
export type DisplaySelection = "all" | "primary";
export type MatchMode = "smart" | "priority";

export interface SourceSettings {
  order: SourceId[];
  enabled: SourceId[];
  mode: MatchMode;
  preferWordTiming: boolean;
  preferWordTimedLrclib: boolean;
  repairQqGaps: boolean;
  localFolder: string;
}

export interface Settings {
  composition: Composition;
  lyricScale: number;
  frameRate: 15 | 30 | 60;
  pauseWhenCovered: boolean;
  lyricOffset: number;
  nextWordPreview: boolean;
  nextWordPreviewLead: number;
  wordReveal: boolean;
  showInterludes: boolean;
  noLyrics: NoLyricsBehaviour;
  scrimStrength: number;
  vividness: number;
  effectSpeed: number;
  font: FontChoice;
  background: BackgroundMode;
  visualizer: VisualizerStyle;
  audioReactive: boolean;
  audioSensitivity: number;
  readings: ReadingDisplay;
  toneMarks: boolean;
  displays: DisplaySelection;
  launchAtLogin: boolean;
  reduceMotion: boolean;
  sources: SourceSettings;
  songOffsets: Record<string, number>;
}

export interface Snapshot {
  track: TrackPayload;
  lyrics: LyricsView;
  clock: PlaybackClock;
  settings: Settings;
  render: RenderState;
  lookup: LookupInfo;
  version: string;
}

/** Tauri event names. */
export const EVENTS = {
  snapshot: "lunalyr://snapshot",
  track: "lunalyr://track",
  lyrics: "lunalyr://lyrics",
  clock: "lunalyr://clock",
  settings: "lunalyr://settings",
  render: "lunalyr://render",
  lookup: "lunalyr://lookup",
  audio: "lunalyr://audio",
} as const;

/** Tauri command names (invoke). Arguments are camelCase objects. */
export const COMMANDS = {
  /** () -> Snapshot */
  getSnapshot: "get_snapshot",
  /** ({ patch: Partial<Settings> }) -> Settings  (shallow merge; `sources` replaced whole) */
  updateSettings: "update_settings",
  /** () -> void — re-run the lookup for the current track, bypassing caches */
  reloadLyrics: "reload_lyrics",
  /** ({ source: SourceId }) -> void — use and remember this candidate for the recording */
  selectCandidate: "select_candidate",
  /** () -> void — forget the saved choice and look up automatically */
  forgetSelection: "forget_selection",
  /** ({ seconds: number }) -> void — per-recording offset for the current track */
  setSongOffset: "set_song_offset",
  /** ({ text: string }) -> string — import LRC/TTML for the current track; returns a status message */
  importLyrics: "import_lyrics",
  /** () -> void */
  openSettings: "open_settings",
  /** ({ paused: boolean }) -> void — user pause of all wallpapers */
  setUserPaused: "set_user_paused",
} as const;
