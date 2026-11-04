// Visualiser background (settings.background === "visualizer").
//
//  - "spectrum": a calm, album-coloured spectrum drawn on a half-resolution
//    Canvas 2D over the colour field, driven by `lunalyr://audio` frames
//    (log-spaced bands). Bands rise quickly and fall slowly; without audio a
//    slow idle swell keeps the shape alive.
//  - "wavefield": the bundled Wavefield web engine
//    (ui/public/engines/WavefieldEngine.html) in a same-origin iframe, driven
//    once per wallpaper frame through `window.lunalyrWavefield.draw(frame)`,
//    exactly like Sources/Visualizers/WebVisualizerView.swift: the frame keys
//    are clock, width, height, preset, projection, sensitivity, motion,
//    reduced, paused, preview, albumGradient, pcm and sampleRate, and an album
//    gradient overlay blended with `mix-blend-mode: color` sits on top of the
//    engine's canvas. The iframe only exists while the scene is selected.
//
// Both stop with the frame loop: nothing draws while the wallpaper is paused.

import type { AudioFrame, Settings } from "../contract";
import type { ViewportMetrics } from "../timing/stage";
import { el } from "./dom";
import { cssGradient, type RGB } from "./palette";

/** Whether the visualiser background can be chosen in this build. */
export const VISUALIZER_AVAILABLE = true;

/** Wavefield expects exactly 2048 little-endian f32 samples (8192 bytes → 10924 base64 chars). */
export const WAVEFIELD_PCM_BASE64_LENGTH = 10924;

export interface WavefieldFrame {
  clock: number;
  width: number;
  height: number;
  preset: string;
  projection: "preset" | "screen" | "sphere";
  sensitivity: number;
  motion: number;
  reduced: boolean;
  paused: boolean;
  preview: boolean;
  albumGradient?: string;
  pcm?: string;
  sampleRate?: number;
}

/** The frame handed to the engine (pure; tested). */
export function wavefieldFrame(options: {
  clock: number;
  width: number;
  height: number;
  settings: Pick<Settings, "audioSensitivity" | "effectSpeed" | "audioReactive">;
  reduced: boolean;
  albumGradient: string | null;
  audio: AudioFrame | null;
}): WavefieldFrame {
  const frame: WavefieldFrame = {
    clock: options.clock,
    width: Math.round(options.width),
    height: Math.round(options.height),
    preset: "default",
    projection: "preset",
    sensitivity: options.settings.audioSensitivity,
    motion: options.settings.effectSpeed,
    reduced: options.reduced,
    paused: false,
    preview: false,
  };
  if (options.albumGradient) frame.albumGradient = options.albumGradient;
  const audio = options.audio;
  if (options.settings.audioReactive && audio?.pcm && audio.pcm.length === WAVEFIELD_PCM_BASE64_LENGTH && audio.sampleRate > 0) {
    frame.pcm = audio.pcm;
    frame.sampleRate = audio.sampleRate;
  }
  return frame;
}

/** Peak-hold smoothing: fast attack, slow release, frame-rate independent. */
export function smoothBands(current: Float32Array, target: readonly number[], dt: number, attack = 0.06, release = 0.45): Float32Array {
  const up = 1 - Math.exp(-dt / attack);
  const down = 1 - Math.exp(-dt / release);
  for (let i = 0; i < current.length; i++) {
    const t = Math.min(1, Math.max(0, target[i] ?? 0));
    const c = current[i];
    current[i] = c + (t - c) * (t > c ? up : down);
  }
  return current;
}

const AUDIO_STALE_MS = 600;
const BARS = 48;

class SpectrumView {
  readonly canvas: HTMLCanvasElement;
  private readonly ctx: CanvasRenderingContext2D | null;
  private readonly levels = new Float32Array(BARS);
  private readonly target: number[] = new Array(BARS).fill(0);
  private lastTime = 0;
  private width = 0;
  private height = 0;
  static readonly renderScale = 0.5;

  constructor(parent: HTMLElement) {
    this.canvas = el("canvas", "ll-spectrum", parent);
    this.ctx = this.canvas.getContext("2d", { alpha: true });
  }

  resize(width: number, height: number): void {
    this.width = width;
    this.height = height;
    this.canvas.width = Math.max(1, Math.round(width * SpectrumView.renderScale));
    this.canvas.height = Math.max(1, Math.round(height * SpectrumView.renderScale));
  }

  render(nowMs: number, stops: readonly RGB[], audio: AudioFrame | null, sensitivity: number, reduced: boolean): void {
    const ctx = this.ctx;
    if (!ctx) return;
    const dt = this.lastTime ? Math.min(0.25, (nowMs - this.lastTime) / 1000) : 1 / 30;
    this.lastTime = nowMs;
    const gain = Math.max(0.1, Math.min(3, sensitivity)) * (reduced ? 0.6 : 1);
    if (audio && audio.bands.length > 0) {
      // Resample the log bands onto the bars, lows in the middle.
      const n = audio.bands.length;
      for (let i = 0; i < BARS; i++) {
        const f = Math.abs(i - (BARS - 1) / 2) / ((BARS - 1) / 2);
        const position = f * (n - 1);
        const a = Math.floor(position);
        const b = Math.min(n - 1, a + 1);
        const v = audio.bands[a] + (audio.bands[b] - audio.bands[a]) * (position - a);
        this.target[i] = Math.pow(Math.max(0, v) * gain, 0.85) * (0.75 + 0.25 * audio.level);
      }
    } else {
      // Idle: a slow, low swell so the scene never freezes into a flat line.
      const t = nowMs / 1000;
      for (let i = 0; i < BARS; i++) {
        const f = i / (BARS - 1);
        this.target[i] = 0.08 + 0.05 * Math.sin(t * 0.7 + f * 6.2) + 0.03 * Math.sin(t * 0.37 + f * 13);
      }
    }
    smoothBands(this.levels, this.target, dt, reduced ? 0.2 : 0.06, reduced ? 0.9 : 0.45);

    const w = this.canvas.width;
    const h = this.canvas.height;
    ctx.clearRect(0, 0, w, h);
    const slot = w / BARS;
    const barWidth = slot * 0.56;
    const maxHeight = h * 0.2;
    const base = h * 0.94;
    for (let i = 0; i < BARS; i++) {
      const level = this.levels[i];
      const barHeight = Math.max(2, level * maxHeight);
      const colour = paletteAt(stops, i / (BARS - 1));
      const x = i * slot + (slot - barWidth) / 2;
      const gradient = ctx.createLinearGradient(0, base - barHeight, 0, base);
      gradient.addColorStop(0, rgba(colour, 0.5));
      gradient.addColorStop(1, rgba(colour, 0.04));
      ctx.fillStyle = gradient;
      roundedBar(ctx, x, base - barHeight, barWidth, barHeight, Math.min(barWidth / 2, 6));
    }
    // A thin glow line tracing the tops.
    ctx.beginPath();
    for (let i = 0; i < BARS; i++) {
      const x = i * slot + slot / 2;
      const y = base - Math.max(2, this.levels[i] * maxHeight) - 3;
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.strokeStyle = "rgba(255,255,255,0.18)";
    ctx.lineWidth = 1;
    ctx.stroke();
  }

  get size(): { width: number; height: number } {
    return { width: this.width, height: this.height };
  }
}

function roundedBar(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number): void {
  ctx.beginPath();
  ctx.moveTo(x, y + h);
  ctx.lineTo(x, y + r);
  ctx.quadraticCurveTo(x, y, x + r, y);
  ctx.lineTo(x + w - r, y);
  ctx.quadraticCurveTo(x + w, y, x + w, y + r);
  ctx.lineTo(x + w, y + h);
  ctx.closePath();
  ctx.fill();
}

/** Colour along the palette, lifted toward white so dark covers still read. */
function paletteAt(stops: readonly RGB[], f: number): RGB {
  const position = Math.min(1, Math.max(0, f)) * (stops.length - 1);
  const a = Math.floor(position);
  const b = Math.min(stops.length - 1, a + 1);
  const t = position - a;
  const c = stops[a];
  const d = stops[b];
  const mix = (u: number, v: number) => (u + (v - u) * t) * 0.7 + 0.3;
  return [mix(c[0], d[0]), mix(c[1], d[1]), mix(c[2], d[2])];
}

function rgba([r, g, b]: RGB, alpha: number): string {
  const c = (v: number) => Math.round(Math.min(1, Math.max(0, v)) * 255);
  return `rgba(${c(r)},${c(g)},${c(b)},${alpha})`;
}

class WavefieldView {
  readonly iframe: HTMLIFrameElement;
  private ready = false;
  private overlay: HTMLDivElement | null = null;
  private gradient = "";
  private failed = false;

  constructor(parent: HTMLElement) {
    this.iframe = el("iframe", "ll-wavefield", parent);
    this.iframe.title = "Visualiser";
    this.iframe.setAttribute("aria-hidden", "true");
    this.iframe.tabIndex = -1;
    this.iframe.addEventListener("load", () => this.onLoad());
    this.iframe.src = "./engines/WavefieldEngine.html";
  }

  /** True once the engine is known not to work here (missing or threw); the colour field takes over. */
  get broken(): boolean {
    return this.failed;
  }

  private onLoad(): void {
    const doc = this.iframe.contentDocument;
    const engine = (this.iframe.contentWindow as unknown as { lunalyrWavefield?: unknown } | null)?.lunalyrWavefield;
    if (!doc?.body || !engine) {
      // Blocked (CSP, missing file) or not same-origin: fall back quietly.
      this.fail("Wavefield engine did not load");
      return;
    }
    // The same album blend the macOS host injects as a user script.
    doc.body.style.isolation = "isolate";
    const overlay = doc.createElement("div");
    overlay.style.cssText = "position:fixed;inset:0;z-index:1;pointer-events:none;mix-blend-mode:color";
    overlay.setAttribute("aria-hidden", "true");
    doc.body.append(overlay);
    this.overlay = overlay;
    this.gradient = "";
    this.ready = true;
  }

  draw(frame: WavefieldFrame): void {
    if (!this.ready || this.failed) return;
    const next = frame.albumGradient ?? "";
    if (this.overlay && next !== this.gradient) {
      this.gradient = next;
      this.overlay.style.backgroundImage = next;
    }
    const engine = (this.iframe.contentWindow as unknown as { lunalyrWavefield?: { draw(frame: WavefieldFrame): unknown } } | null)?.lunalyrWavefield;
    if (!engine) return;
    try {
      engine.draw(frame);
    } catch (error) {
      this.fail(error);
    }
  }

  private fail(reason: unknown): void {
    if (this.failed) return;
    this.failed = true;
    this.iframe.style.display = "none";
    console.error("Wavefield engine:", reason);
  }

  destroy(): void {
    this.iframe.remove();
  }
}

export class VisualizerLayer {
  private readonly root: HTMLDivElement;
  private spectrum: SpectrumView | null = null;
  private wavefield: WavefieldView | null = null;
  private enabled = false;
  private style: Settings["visualizer"] = "wavefield";
  private audio: AudioFrame | null = null;
  private audioAt = 0;
  private width = 0;
  private height = 0;
  private filterKey = "";

  constructor(parent: HTMLElement) {
    this.root = el("div", "ll-visualizer", parent);
    this.root.style.display = "none";
  }

  /** True when the visualiser replaces the colour field (Wavefield draws its own background). */
  get active(): boolean {
    return this.enabled && this.style === "wavefield" && !(this.wavefield?.broken ?? false);
  }

  /** True when anything visualiser-related draws this frame. */
  get drawing(): boolean {
    return this.enabled;
  }

  configure(settings: Settings): void {
    this.enabled = VISUALIZER_AVAILABLE && settings.background === "visualizer";
    this.style = settings.visualizer;
    this.root.style.display = this.enabled ? "" : "none";
    if (this.enabled && this.style === "spectrum") {
      if (!this.spectrum) {
        this.spectrum = new SpectrumView(this.root);
        this.spectrum.resize(this.width, this.height);
      }
    } else if (this.spectrum) {
      this.spectrum.canvas.remove();
      this.spectrum = null;
    }
    if (this.enabled && this.style === "wavefield") {
      if (!this.wavefield) this.wavefield = new WavefieldView(this.root);
    } else if (this.wavefield) {
      this.wavefield.destroy();
      this.wavefield = null;
    }
    const filter = settings.vividness === 1 ? "none" : `saturate(${settings.vividness})`;
    if (filter !== this.filterKey) {
      this.filterKey = filter;
      this.root.style.filter = filter;
    }
  }

  resize(width: number, height: number): void {
    this.width = width;
    this.height = height;
    this.spectrum?.resize(width, height);
  }

  setPaused(_paused: boolean): void {
    // Drawing is driven by the wallpaper frame loop, which is already stopped.
  }

  pushAudio(frame: AudioFrame): void {
    this.audio = frame;
    this.audioAt = performance.now();
  }

  /** Draws the visualiser for this frame (`now` is performance time in ms). */
  render(now: number, _flow: number, stops: readonly RGB[], settings: Settings, metrics: ViewportMetrics, reduced: boolean, albumColours: boolean): void {
    if (!this.enabled) return;
    const fresh = settings.audioReactive && this.audio && now - this.audioAt < AUDIO_STALE_MS ? this.audio : null;
    if (this.spectrum) this.spectrum.render(now, stops, fresh, settings.audioSensitivity, reduced);
    if (this.wavefield) {
      this.wavefield.draw(wavefieldFrame({
        clock: now / 1000,
        width: metrics.size.width,
        height: metrics.size.height,
        settings,
        reduced,
        albumGradient: albumColours ? cssGradient(stops) : null,
        audio: fresh,
      }));
    }
  }
}
