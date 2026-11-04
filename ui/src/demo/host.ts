// In-memory stand-in for the Rust side, used by demo.html and by the
// wallpaper / settings pages when they are opened in a plain browser. It
// answers the contract commands and emits the contract events, so the pages
// run exactly the code they run inside the app.
//
// Pages of one browser profile share settings, pause and song offsets over a
// BroadcastChannel, so settings.html can drive an open demo.html.

import type { Bridge, EventName, EventPayloads, Unlisten } from "../bridge";
import type { CandidateSummary, Line, LookupInfo, LyricsView, Settings, Snapshot, SourceId, Track, Word } from "../contract";
import { COMMANDS, EVENTS } from "../contract";
import { defaultSettings, emptySnapshot, fallbackPalette, SOURCE_IDS, SOURCE_TITLES } from "../defaults";
import { songOffsetKey } from "../sha256";
import { trackSeed } from "../timing/seed";
import { syntheticAudio } from "./audio";
import { parseImport, precisionOf } from "./lrc";
import { asLineTimed, PALETTES, PLAIN_LYRICS, SONGS, type DemoSong } from "./samples";

export type DemoScenario = "word" | "line" | "plain" | "notFound" | "idle" | "loading" | "snapshot";
export const DEMO_SCENARIOS: readonly DemoScenario[] = ["word", "line", "plain", "notFound", "idle", "loading"];

export interface DemoOptions {
  scenario?: DemoScenario;
  time?: number;
  paused?: boolean;
  palette?: string;
  settings?: Partial<Settings>;
  /** Share settings with other demo pages of this browser. */
  share?: boolean;
  /** Keep notFound / idle notices up (re-issued every 3 s) for screenshots. */
  holdNotices?: boolean;
  /** Adopt what another open demo page is playing (default true). */
  follow?: boolean;
}

type Handler = (payload: unknown) => void;

interface SharedPlayback {
  scenario: DemoScenario;
  chosen: SourceId;
  pinned: boolean;
  track: Snapshot["track"];
  lyrics: LyricsView;
  lookup: LookupInfo;
  clock: Snapshot["clock"];
}

interface ChannelMessage {
  kind: "settings" | "render" | "hello" | "playback";
  from: string;
  settings?: Settings;
  render?: Snapshot["render"];
  playback?: SharedPlayback;
}

export class DemoHost implements Bridge {
  readonly kind = "demo" as const;
  private state: Snapshot = emptySnapshot();
  private readonly handlers = new Map<string, Set<Handler>>();
  private scenario: DemoScenario = "word";
  private song: DemoSong | null = null;
  private lineReadings: (Word[] | null)[] = [];
  private chosen: SourceId = "lyricsPlus";
  private customSnapshot: Snapshot | null = null;
  private loopTimer: ReturnType<typeof setTimeout> | null = null;
  private holdTimer: ReturnType<typeof setInterval> | null = null;
  private audioTimer: ReturnType<typeof setInterval> | null = null;
  private channel: BroadcastChannel | null = null;
  private readonly id = Math.random().toString(36).slice(2);
  private readonly follow: boolean;

  constructor(options: DemoOptions = {}) {
    this.state.settings = { ...defaultSettings(), ...options.settings };
    this.state.version = "demo";
    this.follow = options.follow ?? true;
    if (options.share !== false && typeof BroadcastChannel !== "undefined") {
      this.channel = new BroadcastChannel("lunalyr-demo");
      this.channel.onmessage = (event: MessageEvent<ChannelMessage>) => this.receive(event.data);
      this.post({ kind: "hello", from: this.id });
    }
    this.setPalette(options.palette ?? "dusk", false);
    this.setScenario(options.scenario ?? "word", { time: options.time, paused: options.paused, emit: false });
    if (options.holdNotices) this.holdTimer = setInterval(() => this.refreshNotices(), 3000);
  }

  // MARK: Bridge

  async listen<E extends EventName>(event: E, handler: (payload: EventPayloads[E]) => void): Promise<Unlisten> {
    let set = this.handlers.get(event);
    if (!set) this.handlers.set(event, (set = new Set()));
    set.add(handler as Handler);
    if (event === EVENTS.audio) this.updateAudio();
    return () => {
      set!.delete(handler as Handler);
      if (event === EVENTS.audio) this.updateAudio();
    };
  }

  async invoke<T = unknown>(command: string, args: Record<string, unknown> = {}): Promise<T> {
    return this.handle(command, args) as T;
  }

  private handle(command: string, args: Record<string, unknown>): unknown {
    switch (command) {
      case COMMANDS.getSnapshot:
        return structuredClone(this.state);
      case COMMANDS.updateSettings: {
        const patch = (args.patch ?? {}) as Partial<Settings>;
        this.applySettings({ ...this.state.settings, ...patch }, true);
        return structuredClone(this.state.settings);
      }
      case COMMANDS.reloadLyrics:
        this.reload();
        return null;
      case COMMANDS.selectCandidate:
        this.select(args.source as SourceId);
        return null;
      case COMMANDS.forgetSelection:
        this.forget();
        return null;
      case COMMANDS.setSongOffset:
        this.setSongOffset(Number(args.seconds));
        return null;
      case COMMANDS.importLyrics:
        return this.importLyrics(String(args.text ?? ""));
      case COMMANDS.openSettings:
        if (typeof window !== "undefined") window.open("./settings.html", "lunalyr-settings");
        return null;
      case COMMANDS.setUserPaused:
        this.setRender({ paused: Boolean(args.paused), reason: args.paused ? "user" : "" }, true);
        return null;
      default:
        throw new Error(`Unknown command ${command}`);
    }
  }

  private emit<E extends EventName>(event: E, payload: EventPayloads[E]): void {
    for (const handler of [...(this.handlers.get(event) ?? [])]) handler(structuredClone(payload));
  }

  // MARK: sharing between pages

  private post(message: ChannelMessage): void {
    this.channel?.postMessage(message);
  }

  private receive(message: ChannelMessage): void {
    if (!message || message.from === this.id) return;
    if (message.kind === "hello") {
      this.post({ kind: "settings", from: this.id, settings: this.state.settings });
      if (this.scenario !== "snapshot") this.share();
    } else if (message.kind === "playback" && message.playback && this.follow) {
      this.adopt(message.playback);
    } else if (message.kind === "settings" && message.settings) {
      this.applySettings(message.settings, false);
    } else if (message.kind === "render" && message.render) {
      this.setRender(message.render, false);
    }
  }

  /** Tell other demo pages what is playing (scenario, lyrics, lookup, clock). */
  private share(): void {
    if (!this.channel || this.scenario === "snapshot") return;
    this.post({
      kind: "playback",
      from: this.id,
      playback: { scenario: this.scenario, chosen: this.chosen, pinned: this.pinned, track: this.state.track, lyrics: this.state.lyrics, lookup: this.state.lookup, clock: this.state.clock },
    });
  }

  private adopt(p: SharedPlayback): void {
    if (this.scenario === "snapshot") return;
    this.scenario = p.scenario;
    this.song = p.scenario === "word" ? SONGS.word : p.scenario === "line" ? SONGS.line : null;
    this.lineReadings = this.song ? this.song.readings : [];
    this.chosen = p.chosen;
    this.pinned = p.pinned;
    this.state.track = p.track;
    this.state.lyrics = p.lyrics;
    this.state.lookup = p.lookup;
    this.state.clock = p.clock;
    this.scheduleLoop();
    this.emit(EVENTS.track, this.state.track);
    this.emit(EVENTS.lyrics, this.state.lyrics);
    this.emit(EVENTS.lookup, this.state.lookup);
    this.emit(EVENTS.clock, this.state.clock);
  }

  // MARK: demo controls

  get current(): Readonly<Snapshot> {
    return this.state;
  }

  get currentScenario(): DemoScenario {
    return this.scenario;
  }

  get duration(): number {
    if (this.customSnapshot) {
      const track = this.state.track.track;
      if (track && track.duration > 0) return track.duration;
      const lyrics = this.state.lyrics;
      if (lyrics.status === "synced" && lyrics.lines.length) return lyrics.lines[lyrics.lines.length - 1].end + 4;
      return 60;
    }
    return this.song?.duration ?? 60;
  }

  /** Current track time (contract extrapolation, without the offset). */
  position(now = Date.now()): number {
    const c = this.state.clock;
    return c.position + (c.playing ? ((now - c.sampledAt) / 1000) * c.rate : 0);
  }

  get playing(): boolean {
    return this.state.clock.playing;
  }

  setScenario(scenario: DemoScenario, options: { time?: number; paused?: boolean; emit?: boolean } = {}): void {
    const emit = options.emit ?? true;
    this.scenario = scenario;
    this.customSnapshot = scenario === "snapshot" ? this.customSnapshot : null;
    const now = Date.now();
    this.song = scenario === "word" ? SONGS.word : scenario === "line" ? SONGS.line : null;
    this.chosen = scenario === "line" ? "lrclib" : "lyricsPlus";
    this.pinned = false;
    if (scenario === "idle") {
      this.state.track = { ...this.state.track, track: null, idleSince: now };
    } else {
      const meta = this.song ?? (scenario === "plain" ? { title: PLAIN_LYRICS.title, artist: PLAIN_LYRICS.artist, album: "Sample Sessions", duration: 0 } : { title: "Quiet Instrumental", artist: "The Demo Ensemble", album: "Sample Sessions", duration: 0 });
      this.state.track = { ...this.state.track, track: this.makeTrack(meta.title, meta.artist, meta.album, meta.duration || 180), idleSince: null };
    }
    this.lineReadings = this.song ? this.song.readings : [];
    this.state.lyrics = this.lyricsView(now);
    this.state.lookup = this.lookupInfo();
    const playing = scenario === "word" || scenario === "line" ? !(options.paused ?? false) : !(options.paused ?? false) && scenario !== "idle";
    this.state.clock = { position: options.time ?? (this.song ? 0 : 0), sampledAt: now, playing, rate: 1, offset: this.offsetFor() };
    this.scheduleLoop();
    if (emit) {
      this.emit(EVENTS.track, this.state.track);
      this.emit(EVENTS.lyrics, this.state.lyrics);
      this.emit(EVENTS.lookup, this.state.lookup);
      this.emit(EVENTS.clock, this.state.clock);
      this.share();
    }
  }

  /** Render a Snapshot loaded from a file (demo.html?snapshot=url). */
  loadSnapshot(snapshot: Snapshot, options: { time?: number; paused?: boolean } = {}): void {
    this.customSnapshot = snapshot;
    this.scenario = "snapshot";
    this.song = null;
    const now = Date.now();
    this.state = structuredClone({ ...emptySnapshot(), ...snapshot, version: snapshot.version || "fixture" });
    this.state.settings = { ...defaultSettings(), ...snapshot.settings };
    this.lineReadings = snapshot.lyrics.status === "synced" ? snapshot.lyrics.readings : [];
    this.state.clock = {
      position: options.time ?? snapshot.clock?.position ?? 0,
      sampledAt: now,
      playing: options.paused === undefined ? snapshot.clock?.playing ?? true : !options.paused,
      rate: snapshot.clock?.rate ?? 1,
      offset: snapshot.clock?.offset ?? 0,
    };
    if (this.state.render.paused) this.state.render = { paused: false, reason: "" };
    this.scheduleLoop();
    this.emit(EVENTS.snapshot, this.state);
  }

  play(): void {
    if (this.state.clock.playing) return;
    this.state.clock = { ...this.state.clock, position: this.position(), sampledAt: Date.now(), playing: true };
    this.scheduleLoop();
    this.emit(EVENTS.clock, this.state.clock);
    this.share();
  }

  pause(): void {
    if (!this.state.clock.playing) return;
    this.state.clock = { ...this.state.clock, position: this.position(), sampledAt: Date.now(), playing: false };
    this.scheduleLoop();
    this.emit(EVENTS.clock, this.state.clock);
    this.share();
  }

  seek(position: number): void {
    this.state.clock = { ...this.state.clock, position: Math.max(0, position), sampledAt: Date.now() };
    this.scheduleLoop();
    this.emit(EVENTS.clock, this.state.clock);
    this.share();
  }

  setPalette(name: string, emit = true): void {
    const preset = PALETTES[name] ?? PALETTES.fallback;
    this.state.track = { ...this.state.track, palette: preset.hsb === PALETTES.fallback.hsb ? fallbackPalette() : { hsb: preset.hsb.map((c) => [...c] as [number, number, number]), fromArtwork: preset.fromArtwork } };
    if (emit) this.emit(EVENTS.track, this.state.track);
  }

  patchSettings(patch: Partial<Settings>): void {
    this.applySettings({ ...this.state.settings, ...patch }, true);
  }

  setUserPaused(paused: boolean): void {
    this.setRender({ paused, reason: paused ? "user" : "" }, true);
  }

  dispose(): void {
    if (this.audioTimer) clearInterval(this.audioTimer);
    if (this.loopTimer) clearTimeout(this.loopTimer);
    if (this.holdTimer) clearInterval(this.holdTimer);
    this.channel?.close();
  }

  // MARK: internals

  private makeTrack(title: string, artist: string, album: string | null, duration: number): Track {
    const recordingKey = `${title.toLowerCase()}|${artist.toLowerCase()}|${Math.round(duration)}`;
    return { title, artist, album, duration, sourceApp: "Demo.exe", recordingKey, seed: trackSeed(title, artist) };
  }

  private applySettings(next: Settings, broadcast: boolean): void {
    const previous = this.state.settings;
    this.state.settings = structuredClone(next);
    this.emit(EVENTS.settings, this.state.settings);
    this.updateAudio();
    if (broadcast) this.post({ kind: "settings", from: this.id, settings: this.state.settings });
    if (previous.readings !== next.readings || previous.toneMarks !== next.toneMarks) {
      if (this.state.lyrics.status === "synced") {
        this.state.lyrics = { ...this.state.lyrics, readings: this.readingsFor(this.state.lyrics.lines) };
        this.emit(EVENTS.lyrics, this.state.lyrics);
      }
    }
    const offset = this.offsetFor();
    if (offset !== this.state.clock.offset) {
      this.state.clock = { ...this.state.clock, position: this.position(), sampledAt: Date.now(), offset };
      this.emit(EVENTS.clock, this.state.clock);
    }
  }

  private setRender(render: Snapshot["render"], broadcast: boolean): void {
    this.state.render = { ...render };
    this.emit(EVENTS.render, this.state.render);
    this.updateAudio();
    if (broadcast) this.post({ kind: "render", from: this.id, render: this.state.render });
  }

  private offsetFor(): number {
    const track = this.state.track.track;
    const song = track ? this.state.settings.songOffsets[songOffsetKey(track.recordingKey)] ?? 0 : 0;
    return this.state.settings.lyricOffset + song;
  }

  private readingsFor(lines: Line[]): (Word[] | null)[] {
    const settings = this.state.settings;
    if (settings.readings === "off") return [];
    return lines.map((line, i) => {
      const reading = this.lineReadings[i] ?? null;
      if (!reading || reading.length !== line.words.length) return null;
      return settings.toneMarks ? reading : reading.map((w) => ({ ...w, text: w.text.normalize("NFD").replace(/[̀-ͯ]/g, "") }));
    });
  }

  private lyricsView(now: number): LyricsView {
    switch (this.scenario) {
      case "idle":
        return { status: "idle" };
      case "loading":
        return { status: "loading" };
      case "notFound":
        return { status: "notFound", reportedAt: now };
      case "plain":
        return { status: "plain", lines: PLAIN_LYRICS.lines, provenance: { precision: "Not synced", source: "lyrics.ovh" } };
      case "snapshot":
        return this.state.lyrics;
      default: {
        const song = this.song!;
        const lineVersion = song.id === "word" && this.chosen !== "lyricsPlus";
        const lines = lineVersion ? asLineTimed(song) : song.lines;
        this.lineReadings = song.readings;
        return {
          status: "synced",
          lines,
          provenance: { precision: lineVersion ? "Line-synced" : song.precision, source: SOURCE_TITLES[this.chosen] },
          readings: this.readingsFor(lines),
        };
      }
    }
  }

  private candidates(): CandidateSummary[] {
    const song = this.song;
    const track = this.state.track.track;
    if (!song || !track) return [];
    const rows: { source: SourceId; precision: string }[] = song.id === "word"
      ? [{ source: "lyricsPlus", precision: "Word-by-word" }, { source: "lrclib", precision: "Line-synced" }, { source: "netEase", precision: "Line-synced" }]
      : [{ source: "lrclib", precision: "Line-synced" }, { source: "qqMusic", precision: "Line-synced" }];
    return rows.map(({ source, precision }) => ({
      source,
      sourceTitle: SOURCE_TITLES[source],
      title: track.title,
      artist: track.artist,
      duration: track.duration,
      precision,
      lineCount: song.lines.length,
      hasReadings: song.readings.some(Boolean),
      chosen: source === this.chosen,
    }));
  }

  private pinned = false;

  private lookupInfo(): LookupInfo {
    if (this.scenario === "idle") return { candidates: [], states: [], pinned: false, cached: false };
    const found = new Set(this.candidates().map((c) => c.source));
    const enabled = new Set(this.state.settings.sources.enabled);
    const states: [SourceId, string][] = SOURCE_IDS.map((id) => [
      id,
      !enabled.has(id) ? "Disabled" : this.scenario === "loading" ? "Searching…" : found.has(id) ? "Match found" : id === "localFiles" ? "No folder chosen" : "No match",
    ]);
    return { candidates: this.candidates(), states, pinned: this.pinned, cached: false };
  }

  private select(source: SourceId): void {
    if (!this.song) return;
    this.chosen = source;
    this.pinned = true;
    this.state.lyrics = this.lyricsView(Date.now());
    this.state.lookup = this.lookupInfo();
    this.emit(EVENTS.lyrics, this.state.lyrics);
    this.emit(EVENTS.lookup, this.state.lookup);
    this.share();
  }

  private forget(): void {
    this.pinned = false;
    if (this.song) this.chosen = this.song.id === "line" ? "lrclib" : "lyricsPlus";
    this.reload();
  }

  private reload(): void {
    if (this.scenario === "idle" || this.scenario === "snapshot") return;
    const restore = this.scenario;
    this.state.lyrics = { status: "loading" };
    this.emit(EVENTS.lyrics, this.state.lyrics);
    setTimeout(() => {
      if (this.scenario !== restore) return;
      this.state.lyrics = this.lyricsView(Date.now());
      this.state.lookup = this.lookupInfo();
      this.emit(EVENTS.lyrics, this.state.lyrics);
      this.emit(EVENTS.lookup, this.state.lookup);
      this.share();
    }, 700);
  }

  private setSongOffset(seconds: number): void {
    const track = this.state.track.track;
    if (!track || !Number.isFinite(seconds)) return;
    const offsets = { ...this.state.settings.songOffsets };
    const key = songOffsetKey(track.recordingKey);
    const bounded = Math.max(-30, Math.min(30, seconds));
    if (bounded === 0) delete offsets[key];
    else offsets[key] = bounded;
    this.applySettings({ ...this.state.settings, songOffsets: offsets }, true);
  }

  private importLyrics(text: string): string {
    const track = this.state.track.track;
    if (!track) return "Play a song first.";
    const lines = parseImport(text, track.duration);
    if (lines.length === 0) return "This file has no usable LRC or TTML timestamps.";
    this.lineReadings = [];
    this.pinned = true;
    this.state.lyrics = { status: "synced", lines, provenance: { precision: precisionOf(lines), source: "Imported lyrics" }, readings: [] };
    this.state.lookup = { ...this.lookupInfo(), pinned: true };
    this.emit(EVENTS.lyrics, this.state.lyrics);
    this.emit(EVENTS.lookup, this.state.lookup);
    this.share();
    return `Imported ${lines.length} lines (${precisionOf(lines).toLowerCase()}). Saved for this recording.`;
  }

  /** Like the app: audio frames flow (~30 Hz) only while a visualiser is shown and the wallpaper runs. */
  private updateAudio(): void {
    const wanted = this.state.settings.background === "visualizer" && this.state.clock.playing && !this.state.render.paused
      && this.state.track.track !== null && (this.handlers.get(EVENTS.audio)?.size ?? 0) > 0;
    if (wanted && !this.audioTimer) {
      this.audioTimer = setInterval(() => this.emit(EVENTS.audio, syntheticAudio(this.position())), 33);
    } else if (!wanted && this.audioTimer) {
      clearInterval(this.audioTimer);
      this.audioTimer = null;
    }
  }

  private refreshNotices(): void {
    const now = Date.now();
    if (this.state.lyrics.status === "notFound") {
      this.state.lyrics = { status: "notFound", reportedAt: now };
      this.emit(EVENTS.lyrics, this.state.lyrics);
    }
    if (!this.state.track.track && this.state.track.idleSince !== null) {
      this.state.track = { ...this.state.track, idleSince: now };
      this.emit(EVENTS.track, this.state.track);
    }
  }

  /** Loop the demo song: one timer for the loop point, none while paused. */
  private scheduleLoop(): void {
    this.updateAudio();
    if (this.loopTimer) clearTimeout(this.loopTimer);
    this.loopTimer = null;
    const clock = this.state.clock;
    if (!clock.playing || (!this.song && !this.customSnapshot)) return;
    const remaining = (this.duration - this.position()) / Math.max(0.01, clock.rate);
    this.loopTimer = setTimeout(() => this.seek(0), Math.max(50, remaining * 1000));
  }
}
