// The lyric layer (NowPlayingView.lyricsSection / currentLine): decides each
// frame which lines are on stage — the outgoing line finishing its exit, the
// current line and the next line's preview — and drives their views, the
// notices, the loading indicator and the instrumental dots.
//
// Views are built (and measured) at the start of a frame, before any style
// is written, so a frame performs at most one forced layout, and only when a
// line appears for the first time. The next line is pre-built after the
// frame's writes.

import type { Line, LyricsView, Settings, TrackPayload, Word } from "../contract";
import { hasConnections, resolveComposition, wordBehaviour, type ConcreteComposition, type WordBehaviour } from "../timing/composition";
import { vocalGap, type VocalWindow } from "../timing/geometry";
import { LineIndex, readingPairs, vocalWindows, wordsPerSecond } from "../timing/lines";
import { graphemes, smoothstep } from "../timing/math";
import { deriveMood, type TrackMood } from "../timing/mood";
import {
  gapPresence,
  instrumentalCue,
  lineStage,
  nextLinePreview,
  rollingWords,
  showsOutgoing,
  type LineRole,
  type LineRules,
  type LineStage,
} from "../timing/presentation";
import type { ViewportMetrics } from "../timing/stage";
import { el, Measurer } from "./dom";
import { artworkSaturation, type HSB, type RGB } from "./palette";
import type { BuildEnv, FrameEnv } from "./views/common";
import { BlockView, InterludeView, NoticeView, SpinnerView } from "./views/notices";
import { PhraseLineView } from "./views/phrase";
import { RollingStageView } from "./views/rolling";
import { ScatteredLineView } from "./views/scattered";

const NOTICE_HOLD = 4;
const NOTICE_FADE = 1;
const IDLE_HOLD = 5;
const ROLE_LAYER: Record<LineRole, number> = { outgoing: 1, current: 2, preview: 3 };

type LineView = ScatteredLineView | PhraseLineView;

export interface SceneFrame {
  track: TrackPayload;
  lyrics: LyricsView;
  settings: Settings;
  /** Track time in seconds (contract clock). */
  time: number;
  /** Epoch milliseconds. */
  now: number;
  reduced: boolean;
  accent: RGB;
}

export class LyricScene {
  readonly layer: HTMLDivElement;
  private readonly overlay: HTMLDivElement;
  private readonly measurer: Measurer;
  private readonly notice: NoticeView;
  private readonly spinner: SpinnerView;
  private readonly block: BlockView;
  private readonly interlude: InterludeView;
  private readonly views = new Map<string, { view: LineView; lastUsed: number }>();
  private readonly rolling = new Map<LineRole, RollingStageView>();
  private lyrics: LyricsView | null = null;
  private lines: readonly Line[] = [];
  private readings: readonly (Word[] | null)[] = [];
  private index = new LineIndex([]);
  private windows: VocalWindow[] = [];
  private wps = 0;
  private mood: TrackMood = deriveMood({ wordsPerSecond: 0, artworkSaturation: 0.5 });
  private env: BuildEnv | null = null;
  private envKey = "";
  private behaviours = new Map<ConcreteComposition, WordBehaviour>();
  private settings: Settings | null = null;
  private compositions: ConcreteComposition[] = [];
  private frameCount = 0;
  private readonly rules: LineRules = {
    composition: (index) => this.compositionFor(index),
    behaviour: (composition) => this.behaviourFor(composition),
  };

  constructor(parent: HTMLElement) {
    this.layer = el("div", "ll-lyrics", parent);
    this.overlay = el("div", "ll-overlay", parent);
    this.measurer = new Measurer(this.layer);
    this.notice = new NoticeView(this.overlay);
    this.spinner = new SpinnerView(this.overlay);
    this.block = new BlockView(this.layer);
    this.interlude = new InterludeView(this.overlay);
  }

  /** New lyrics: rebuild the index, vocal windows and cadence; drop built lines. */
  setLyrics(lyrics: LyricsView): void {
    if (lyrics === this.lyrics) return;
    this.lyrics = lyrics;
    this.lines = lyrics.status === "synced" ? lyrics.lines : [];
    this.readings = lyrics.status === "synced" ? lyrics.readings ?? [] : [];
    this.index = new LineIndex(this.lines);
    this.windows = vocalWindows(this.lines);
    this.wps = wordsPerSecond(this.lines);
    this.invalidate();
  }

  /** Inputs that change how lines are built; any change rebuilds the views. */
  setEnvironment(metrics: ViewportMetrics, settings: Settings, palette: readonly HSB[]): void {
    this.settings = settings;
    this.mood = deriveMood({ wordsPerSecond: this.wps, artworkSaturation: artworkSaturation(palette) });
    const key = [metrics.size.width, metrics.size.height, settings.composition, settings.font, settings.readings, settings.wordReveal,
      this.mood.energy, this.mood.intensity, this.mood.organic].join("|");
    this.behaviours.clear();
    if (key === this.envKey && this.env) {
      this.env.metrics = metrics;
      return;
    }
    this.envKey = key;
    this.env = { metrics, face: settings.font, mood: this.mood, readings: settings.readings, measurer: this.measurer, layer: this.layer };
    this.invalidate();
  }

  /** Drop every built view (fonts loaded, lyrics or layout settings changed). */
  invalidate(): void {
    for (const { view } of this.views.values()) view.destroy();
    this.views.clear();
    for (const view of this.rolling.values()) view.destroy();
    this.rolling.clear();
    this.block.reset();
    this.measurer.clear();
    this.compositions = [];
  }

  private compositionFor(index: number): ConcreteComposition {
    let c = this.compositions[index];
    if (!c) {
      c = resolveComposition(this.settings?.composition ?? "editorial", this.mood, index);
      this.compositions[index] = c;
    }
    return c;
  }

  private behaviourFor(composition: ConcreteComposition): WordBehaviour {
    let b = this.behaviours.get(composition);
    if (!b && this.settings) {
      b = wordBehaviour(this.settings, composition);
      this.behaviours.set(composition, b);
    }
    return b ?? { previews: true, reveals: true, lead: 0.45 };
  }

  render(frame: SceneFrame): void {
    const env = this.env;
    if (!env) return;
    this.frameCount++;
    const { metrics } = env;
    const settings = frame.settings;
    const track = frame.track.track;
    let showLines = false;
    let busy = false;
    if (!track) {
      // Say so, then leave the background to itself.
      const since = frame.track.idleSince;
      const presence = since === null ? 0 : 1 - smoothstep(((frame.now - since) / 1000 - IDLE_HOLD) / NOTICE_FADE);
      if (presence > 0.01) this.notice.show("Nothing playing", metrics.baseSize * 0.42, settings.font, presence);
      else this.notice.hide();
      this.block.hide();
    } else {
      const lyrics = frame.lyrics;
      if (lyrics.status === "loading") {
        busy = true;
        this.notice.hide();
        this.block.hide();
      } else if (lyrics.status === "plain" && lyrics.lines.length > 0) {
        this.notice.hide();
        this.block.showPlain(lyrics.lines, settings.font, metrics, this.measurer, settings.lyricScale);
      } else if (lyrics.status === "notFound" || (lyrics.status === "synced" && lyrics.lines.length === 0) || lyrics.status === "plain") {
        this.renderNoLyrics(frame, lyrics.status === "notFound" ? lyrics.reportedAt : null);
      } else if (lyrics.status === "synced") {
        this.notice.hide();
        this.block.hide();
        showLines = true;
      } else {
        this.notice.hide();
        this.block.hide();
      }
    }
    if (busy) this.spinner.show(frame.now, frame.reduced);
    else this.spinner.hide();
    if (showLines) this.renderLines(frame, env);
    else this.hideLines();
  }

  private renderNoLyrics(frame: SceneFrame, reportedAt: number | null): void {
    const { metrics } = this.env!;
    const settings = frame.settings;
    const track = frame.track.track!;
    switch (settings.noLyrics) {
      case "nothing":
        this.notice.hide();
        this.block.hide();
        break;
      case "trackName":
        this.notice.hide();
        this.block.showTrackName(track.title, track.artist, settings.font, metrics, this.measurer, settings.lyricScale);
        break;
      case "notice": {
        this.block.hide();
        // An instrumental should not carry a permanent error notice.
        const presence = reportedAt === null ? 0 : 1 - smoothstep(((frame.now - reportedAt) / 1000 - NOTICE_HOLD) / NOTICE_FADE);
        if (presence > 0.01) this.notice.show("Lyrics unavailable", metrics.baseSize * 0.34, settings.font, presence);
        else this.notice.hide();
        break;
      }
    }
  }

  private hideLines(): void {
    for (const { view } of this.views.values()) view.setVisible(false);
    for (const view of this.rolling.values()) view.setVisible(false);
    this.interlude.hide();
  }

  private available(composition: ConcreteComposition): { width: number; height: number } {
    const stage = this.env!.metrics.stage;
    return hasConnections(composition) ? { width: stage.width * 0.92, height: stage.height * 0.86 } : stage;
  }

  private lineView(index: number, stage: LineStage): LineView {
    const line = this.lines[index];
    const key = `${index}:${line.wordTimed ? "w" : "p"}`;
    const cached = this.views.get(key);
    if (cached) {
      cached.lastUsed = this.frameCount;
      return cached.view;
    }
    const view = this.buildLine(index, stage.composition);
    this.views.set(key, { view, lastUsed: this.frameCount });
    return view;
  }

  private buildLine(index: number, composition: ConcreteComposition): LineView {
    const env = this.env!;
    const line = this.lines[index];
    const readings = this.readings[index] ?? null;
    if (!line.wordTimed) return new PhraseLineView(line, index, readings, composition, env);
    const words = env.readings === "replace" ? readings ?? line.words : line.words;
    const annotations = env.readings === "below" ? readingPairs(readings, line.words) : new Map<string, string>();
    let density = 0;
    for (const w of line.words) density += graphemes(w.text).length + 3;
    const densityScale = hasConnections(composition) ? Math.min(1, Math.max(0.52, Math.sqrt(110 / Math.max(density, 110)))) : 1;
    return new ScatteredLineView({
      words,
      annotations,
      composition,
      seed: index,
      wordTimed: true,
      baseSize: env.metrics.baseSize * densityScale,
      sweep: !this.behaviourFor(composition).reveals,
    }, env, null);
  }

  private rollingView(role: LineRole): RollingStageView {
    let view = this.rolling.get(role);
    if (!view) {
      view = new RollingStageView(this.env!);
      this.rolling.set(role, view);
    }
    return view;
  }

  private renderLines(frame: SceneFrame, env: BuildEnv): void {
    const lines = this.lines;
    const time = frame.time;
    const settings = frame.settings;
    const track = frame.track.track;
    const gap = settings.showInterludes ? vocalGap(this.windows, time, track?.duration ?? 0, 2.4) : null;
    const lyricOpacity = 1 - (gap ? gapPresence(gap, time) : 0);
    const active = this.index.line(time);
    const index = active ?? 0;
    const frameEnv: FrameEnv = {
      time,
      reduced: frame.reduced,
      userScale: settings.lyricScale,
      accent: `rgb(${frame.accent.map((c) => Math.round(Math.min(1, Math.max(0, c)) * 255)).join(",")})`,
    };

    // Phase 1: decide and build (all DOM reads happen here).
    const slots: [number, LineRole][] = [];
    if (showsOutgoing(lines, index, time, this.rules)) slots.push([index - 1, "outgoing"]);
    slots.push([index, "current"]);
    if (active !== null) {
      const next = nextLinePreview(lines, index, time, this.rules);
      if (next !== null) slots.push([next, "preview"]);
    }
    type Plan =
      | { kind: "rolling"; role: LineRole; stage: LineStage; view: RollingStageView; words: readonly Word[]; annotations: ReadonlyMap<string, string> }
      | { kind: "line"; role: LineRole; stage: LineStage; view: LineView };
    const plans: Plan[] = [];
    for (const [lineIndex, role] of slots) {
      const stage = lineStage(lines, lineIndex, time, role, this.rules);
      if (!stage) continue;
      if (stage.rolls) {
        const rolled = rollingWords(lines, this.readings, lineIndex, time, env.readings, stage.behaviour.lead);
        const view = this.rollingView(role);
        view.prepare(rolled.words, rolled.readings, stage.composition);
        plans.push({ kind: "rolling", role, stage, view, words: rolled.words, annotations: rolled.readings });
      } else {
        plans.push({ kind: "line", role, stage, view: this.lineView(lineIndex, stage) });
      }
    }

    // Phase 2: write.
    const shown = new Set<LineView | RollingStageView>();
    for (const plan of plans) {
      const opacity = plan.role === "preview" ? 1 : lyricOpacity;
      if (plan.kind === "rolling") {
        plan.view.update({
          words: plan.words,
          annotations: plan.annotations,
          composition: plan.stage.composition,
          behaviour: plan.stage.behaviour,
          presence: plan.stage.presence,
          lyricOpacity: opacity,
          available: this.available(plan.stage.composition),
        }, frameEnv);
      } else if (plan.view instanceof ScatteredLineView) {
        plan.view.update(plan.stage, frameEnv, env, opacity, this.available(plan.stage.composition));
      } else {
        plan.view.update(plan.stage, plan.role, frameEnv, opacity, this.available(plan.stage.composition));
      }
      plan.view.setLayer(ROLE_LAYER[plan.role]);
      plan.view.setVisible(true);
      shown.add(plan.view);
    }
    for (const { view } of this.views.values()) if (!shown.has(view)) view.setVisible(false);
    for (const view of this.rolling.values()) if (!shown.has(view)) view.setVisible(false);

    const cue = gap ? instrumentalCue(gap, time, settings.nextWordPreviewLead) : null;
    if (cue) this.interlude.show(cue, env.metrics.baseSize, frame.accent, frame.reduced);
    else this.interlude.hide();

    this.evict(index);
    this.prebuild(index, time);
  }

  /** Lines far from the playhead are dropped after a while. */
  private evict(index: number): void {
    for (const [key, entry] of this.views) {
      const line = Number(key.slice(0, key.indexOf(":")));
      if (Math.abs(line - index) > 2 && this.frameCount - entry.lastUsed > 300) {
        entry.view.destroy();
        this.views.delete(key);
      }
    }
  }

  /** Build the next line ahead of time (its measurement then never lands on its first frame). */
  private prebuild(index: number, time: number): void {
    for (const next of [index + 1, index + 2]) {
      if (next >= this.lines.length) return;
      const line = this.lines[next];
      if (line.words.length === 0 || line.start - time > 6) return;
      const composition = this.compositionFor(next);
      const rolls = line.wordTimed && (composition === "spoken" || composition === "cascade" || composition === "cutout");
      if (rolls) continue;
      const key = `${next}:${line.wordTimed ? "w" : "p"}`;
      if (this.views.has(key)) continue;
      this.views.set(key, { view: this.buildLine(next, composition), lastUsed: this.frameCount });
      return;
    }
  }
}
