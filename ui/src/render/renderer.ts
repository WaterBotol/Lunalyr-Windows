// The wallpaper renderer shared by wallpaper.html and demo.html.
//
// Rendering approach: DOM text with CSS transforms, not Canvas 2D.
//  - Text is shaped and rasterised by the browser's own text stack
//    (DirectWrite in WebView2): complex scripts, CJK fallback, bidi and
//    variable fonts work, and glyphs re-rasterise crisply at any
//    devicePixelRatio and scale (no `will-change` pinning of raster scale).
//  - Each line is measured once, in one batched read, when it is built; the
//    frame loop then only writes transform / opacity / filter / shadow and
//    skips identical values, so there is no per-frame layout.
//  - The colour field is a single WebGL quad at half resolution.
//  - The loop is throttled to `settings.frameRate` and fully stopped while
//    `RenderState.paused` or `document.hidden`.

import "@fontsource-variable/manrope";
import "../styles/wallpaper.css";
import type { AudioFrame } from "../contract";
import type { AppState, Change, Store } from "../store";
import { ClockFollower, VisualClock } from "../timing/clock";
import { viewportMetrics, type ViewportMetrics } from "../timing/stage";
import { GradientBackground, scrimGradient } from "./background";
import { el } from "./dom";
import { FrameLoop } from "./loop";
import { accentColour, PaletteFade, type HSB, type RGB } from "./palette";
import { LyricScene } from "./scene";
import { VisualizerLayer } from "./visualizer";

export interface RendererOptions {
  /** Epoch milliseconds (defaults to Date.now). */
  now?: () => number;
  /** Starting phase of the ambient clock (demo screenshots use 0). */
  ambientPhase?: number;
}

/** A still frame is painted from a timer if no animation frame arrives first (rAF can stall behind the desktop icons). */
const STILL_FALLBACK_MS = 100;

export class WallpaperRenderer {
  readonly root: HTMLDivElement;
  private readonly background: GradientBackground;
  private readonly visualizer: VisualizerLayer;
  private readonly scrim: HTMLDivElement;
  private readonly scene: LyricScene;
  private readonly loop: FrameLoop;
  private readonly follower = new ClockFollower();
  private readonly ambient: VisualClock;
  private readonly palette: PaletteFade;
  private readonly now: () => number;
  private accent: RGB;
  private metrics: ViewportMetrics;
  private readonly reducedQuery: MediaQueryList | null;
  private unsubscribe: (() => void) | null = null;
  private scrimKey = "";
  private readonly onResize = () => this.resize();
  private readonly onVisibility = () => {
    this.updateActive();
    this.repaint();
  };
  /** Lines measured with a fallback face are rebuilt, and a paused wallpaper repaints them. */
  private readonly onFonts = () => {
    this.scene.invalidate();
    this.repaint();
  };
  private readonly onReduced = () => this.repaint();
  /** Something on screen changed outside the store (fonts, the visualiser engine). */
  private readonly repaint = () => {
    if (!this.loop.running) this.renderSoon();
  };

  constructor(container: HTMLElement, private readonly store: Store, options: RendererOptions = {}) {
    this.now = options.now ?? Date.now;
    this.ambient = new VisualClock(options.ambientPhase ?? 0);
    this.root = el("div", "ll-root", container);
    this.background = new GradientBackground(this.root);
    // Wavefield ready or failed: the engine or the colour field now draws.
    this.visualizer = new VisualizerLayer(this.root, this.repaint);
    this.scrim = el("div", "ll-scrim", this.root);
    this.scene = new LyricScene(this.root);
    this.loop = new FrameLoop((now) => this.frame(now));
    const state = store.state;
    this.palette = new PaletteFade(state.track.palette.hsb as HSB[]);
    this.accent = accentColour(state.track.palette.hsb as HSB[]);
    this.metrics = viewportMetrics({ width: window.innerWidth, height: window.innerHeight });
    this.reducedQuery = typeof matchMedia === "function" ? matchMedia("(prefers-reduced-motion: reduce)") : null;
  }

  start(): void {
    if (this.unsubscribe) return;
    this.unsubscribe = this.store.subscribe((changed, state) => this.apply(changed, state));
    window.addEventListener("resize", this.onResize);
    document.addEventListener("visibilitychange", this.onVisibility);
    document.fonts?.addEventListener?.("loadingdone", this.onFonts);
    this.reducedQuery?.addEventListener?.("change", this.onReduced);
    this.resize();
    this.apply(new Set<Change>(["track", "lyrics", "clock", "settings", "render", "lookup"]), this.store.state);
  }

  stop(): void {
    this.unsubscribe?.();
    this.unsubscribe = null;
    this.loop.setActive(false);
    this.cancelStill();
    window.removeEventListener("resize", this.onResize);
    document.removeEventListener("visibilitychange", this.onVisibility);
    document.fonts?.removeEventListener?.("loadingdone", this.onFonts);
    this.reducedQuery?.removeEventListener?.("change", this.onReduced);
  }

  /** Audio frames for the visualiser background (lunalyr://audio). */
  pushAudio(frame: AudioFrame): void {
    this.visualizer.pushAudio(frame);
  }

  /** Draw one frame now (used by the demo for still screenshots). */
  renderOnce(): void {
    this.frame(performance.now());
  }

  get running(): boolean {
    return this.loop.running;
  }

  private apply(changed: ReadonlySet<Change>, state: Readonly<AppState>): void {
    const nowPerf = performance.now();
    if (changed.has("clock")) this.follower.set(state.clock, this.now());
    if (changed.has("track")) {
      // Crossfade palette changes (0.9 s); the first snapshot and paused wallpapers switch at once.
      const instant = changed.has("ready") || state.render.paused;
      this.palette.set(state.track.palette.hsb as HSB[], instant ? -Infinity : nowPerf);
      this.accent = accentColour(state.track.palette.hsb as HSB[]);
    }
    if (changed.has("lyrics")) this.scene.setLyrics(state.lyrics);
    if (changed.has("settings") || changed.has("track") || changed.has("lyrics")) {
      this.scene.setEnvironment(this.metrics, state.settings, state.track.palette.hsb as HSB[]);
    }
    if (changed.has("settings")) {
      this.loop.setFps(state.settings.frameRate);
      const key = String(state.settings.scrimStrength);
      if (key !== this.scrimKey) {
        this.scrimKey = key;
        this.scrim.style.background = scrimGradient(state.settings.scrimStrength);
      }
      this.visualizer.configure(state.settings);
      this.background.setVisible(!this.visualizer.active);
    }
    if (changed.has("render") || changed.has("settings")) this.updateActive();
    if (!this.loop.running) this.renderSoon();
  }

  private resize(): void {
    const width = window.innerWidth;
    const height = window.innerHeight;
    this.metrics = viewportMetrics({ width, height });
    this.background.resize(width, height);
    this.visualizer.resize(width, height);
    this.scene.setEnvironment(this.metrics, this.store.state.settings, this.store.state.track.palette.hsb as HSB[]);
    if (!this.loop.running) this.renderSoon();
  }

  private updateActive(): void {
    const paused = this.store.state.render.paused || document.hidden;
    this.loop.setActive(!paused);
    this.visualizer.setPaused(paused);
    if (this.loop.running || !this.stillAllowed()) this.cancelStill();
  }

  private still: { frame: number; timer: ReturnType<typeof setTimeout> } | null = null;

  /**
   * While the user has paused the wallpaper it is still on screen, so a state
   * change repaints one still frame: on the next animation frame, or from a
   * timer when the host has stopped animation frames (behind the desktop
   * icons). Covered, locked or hidden wallpapers draw nothing until they
   * resume (the loop then renders at once).
   */
  private renderSoon(): void {
    if (this.still || !this.unsubscribe || !this.stillAllowed()) return;
    const paint = () => {
      this.cancelStill();
      if (!this.loop.running && this.unsubscribe && this.stillAllowed()) this.frame(performance.now());
    };
    this.still = { frame: requestAnimationFrame(paint), timer: setTimeout(paint, STILL_FALLBACK_MS) };
  }

  private cancelStill(): void {
    if (!this.still) return;
    cancelAnimationFrame(this.still.frame);
    clearTimeout(this.still.timer);
    this.still = null;
  }

  /** Only the user's own pause leaves the wallpaper on screen to repaint. */
  private stillAllowed(): boolean {
    const render = this.store.state.render;
    return !document.hidden && (!render.paused || render.reason === "user");
  }

  private reduced(): boolean {
    return this.store.state.settings.reduceMotion || (this.reducedQuery?.matches ?? false);
  }

  private frame(nowPerf: number): void {
    const state = this.store.state;
    const settings = state.settings;
    const now = this.now();
    const time = this.follower.time(now);
    const stops = this.palette.at(nowPerf);
    // Ambient clock: flow at 0.70 × effect speed; the colour field uses a third of it.
    const flow = this.ambient.advance(nowPerf / 1000, settings.effectSpeed * 0.7);
    const reduced = this.reduced();
    // Wavefield draws its own background; the spectrum sits over the colour field.
    const field = !this.visualizer.active;
    this.background.setVisible(field);
    if (field) this.background.render(flow * 0.34, stops, settings.vividness);
    if (this.visualizer.drawing) this.visualizer.render(nowPerf, flow, stops, settings, this.metrics, reduced, state.track.palette.fromArtwork);
    this.scene.render({ track: state.track, lyrics: state.lyrics, settings, time, now, reduced, accent: this.accent });
  }
}
