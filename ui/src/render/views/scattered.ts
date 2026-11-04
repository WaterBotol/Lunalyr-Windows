// A lyric line as a typographic collage: Editorial, Scattered, Constellation
// (+ Editorial) and the words of Echo. Port of ScatteredLyricText.swift with
// ScatteredWordInk, HeldWordText, FlowLayout and ConstellationLines.
//
// Built once per line: tokens are prepared, every word's ink is measured in
// one batch, slots are reserved with the fixed ink envelope and rows are laid
// out. Each frame then only writes transforms, opacity, filters and shadows,
// and only for values that changed.

import type { Word } from "../../contract";
import { arrivalFor, hasConnections, travelBounds, travelRotation, type ConcreteComposition, type WordArrival } from "../../timing/composition";
import { buildConnections, wordEnvelope, type Segment } from "../../timing/geometry";
import { smoothstep } from "../../timing/math";
import type { LineStage } from "../../timing/presentation";
import { newScatteredFrame, scatteredWordFrame, type ScatteredFrame } from "../../timing/presentation";
import { presentationScale } from "../../timing/stage";
import { el, Styled } from "../dom";
import { blurFilter, entranceClip, entranceTransform, newEntrance, num, opacityValue, px, wordEntrance, type Entrance } from "../entrance";
import { applyFont, cjkLocale, htmlLang, isComplexScript, isRtl, type CjkLocale } from "../fonts";
import { flowLayout, lineStyle, prepareScattered, type ScatteredToken } from "../typography";
import { black, quantize, shadow, white, type BuildEnv, type FrameEnv } from "./common";
import { EchoBackdrop } from "./echo";

const SVG = "http://www.w3.org/2000/svg";
const LINK_OPACITY = 0.2; // Settings.constellationLinkOpacity default

interface WordInk {
  token: ScatteredToken;
  box: HTMLDivElement;
  boxStyle: Styled;
  motion: Styled;
  glyphs: Styled;
  lit: Styled | null;
  chars: Styled[];
  width: number;
  height: number;
  frame: ScatteredFrame;
  entrance: Entrance;
  direction: number;
  shadowKey: string;
}

export interface ScatteredSpec {
  words: readonly Word[];
  annotations: ReadonlyMap<string, string>;
  composition: ConcreteComposition;
  /** Line index: picks the line style and entrance directions. */
  seed: number;
  wordTimed: boolean;
  baseSize: number;
  /** Sweep the fill across each word (word reveal off). */
  sweep: boolean;
}

export class ScatteredLineView {
  readonly root: HTMLDivElement;
  readonly width: number;
  readonly height: number;
  readonly composition: ConcreteComposition;
  private readonly rootStyle: Styled;
  private readonly inks: WordInk[] = [];
  private readonly arrival: WordArrival;
  private readonly links: { segment: Segment; group: Styled }[] = [];
  private readonly rtl: boolean;
  private readonly echo: EchoBackdrop | null = null;
  private readonly starts: number[];

  constructor(spec: ScatteredSpec, env: BuildEnv, parent: HTMLElement | null) {
    this.composition = spec.composition;
    this.arrival = arrivalFor(spec.composition);
    const style = lineStyle(spec.seed);
    const text = spec.words.map((w) => w.text).join(" ");
    const locale = cjkLocale(text);
    this.rtl = isRtl(text);
    const tokens = prepareScattered({
      words: spec.words,
      baseSize: spec.baseSize,
      style,
      mood: env.mood,
      composition: spec.composition,
      spread: 1,
      face: env.face,
      readings: spec.annotations,
      wordTimed: spec.wordTimed,
    });

    this.root = el("div", "ll-view ll-scattered");
    if (locale) this.root.lang = htmlLang(locale);
    this.rootStyle = new Styled(this.root);

    const motions: HTMLDivElement[] = [];
    for (const token of tokens) {
      const motion = this.buildInk(token, spec, locale);
      motions.push(motion.el);
      this.inks.push(motion.ink);
    }
    const sizes = env.measurer.measure(motions);

    const connections = hasConnections(spec.composition);
    const items = this.inks.map((ink, i) => {
      ink.width = sizes[i].width;
      ink.height = sizes[i].height;
      if (!spec.wordTimed) return { width: ink.width + 2 * spec.baseSize * 0.08, height: ink.height };
      const t = ink.token;
      const envelope = wordEnvelope(ink.width, ink.height, 1.38, Math.abs(t.rotation) + travelRotation(this.arrival) + 2);
      const travel = travelBounds(this.arrival, t.size);
      return { width: envelope.width + travel.width + 2 * spec.baseSize * 0.1, height: envelope.height + travel.height + Math.abs(t.waveOffset) * 2 };
    });
    const fullness = Math.min(tokens.length / 7, 1);
    const placed = flowLayout(
      items,
      env.metrics.stage.width,
      spec.baseSize * (connections ? 0.06 : 0.02),
      spec.baseSize * (connections ? 0.34 + 0.96 * fullness : 0.3),
      spec.wordTimed ? style.rowAlignment : "center",
      this.rtl,
    );
    this.width = placed.width;
    this.height = placed.height;
    this.starts = spec.words.map((w) => w.start);

    // Echo lays repeats of the whole phrase behind the live line.
    if (spec.composition === "echo" && spec.wordTimed) {
      this.echo = new EchoBackdrop(spec.words, spec.seed, spec.baseSize, env.metrics.stage, env.face, locale, env.measurer);
      this.echo.layer.style.left = px(this.width / 2);
      this.echo.layer.style.top = px(this.height / 2);
      this.root.appendChild(this.echo.layer);
    }

    // Constellation hairlines between the fixed slots, behind the words.
    if (spec.wordTimed && connections && tokens.length > 1) {
      const boxes = this.inks.map((ink, i) => {
        const c = placed.centres[i];
        const w = items[i].width;
        const h = items[i].height - Math.abs(ink.token.waveOffset) * 2;
        return { x: c.x - w / 2, y: c.y + ink.token.waveOffset - h / 2, width: w, height: h };
      });
      const segments = buildConnections(boxes, { x: 0, y: 0, width: Math.max(1, this.width), height: Math.max(1, this.height) }, 6);
      if (segments.length) {
        const svg = document.createElementNS(SVG, "svg");
        svg.setAttribute("class", "ll-links");
        svg.setAttribute("width", String(this.width));
        svg.setAttribute("height", String(this.height));
        svg.setAttribute("aria-hidden", "true");
        for (const segment of segments) {
          const g = document.createElementNS(SVG, "g");
          const line = document.createElementNS(SVG, "line");
          line.setAttribute("x1", String(segment.ax));
          line.setAttribute("y1", String(segment.ay));
          line.setAttribute("x2", String(segment.bx));
          line.setAttribute("y2", String(segment.by));
          g.appendChild(line);
          for (const [x, y] of [[segment.ax, segment.ay], [segment.bx, segment.by]]) {
            const dot = document.createElementNS(SVG, "circle");
            dot.setAttribute("cx", String(x));
            dot.setAttribute("cy", String(y));
            dot.setAttribute("r", "1");
            g.appendChild(dot);
          }
          svg.appendChild(g);
          this.links.push({ segment, group: new Styled(g as unknown as HTMLElement) });
        }
        this.root.appendChild(svg);
      }
    }

    this.inks.forEach((ink, i) => {
      const c = placed.centres[i];
      const wave = spec.wordTimed ? ink.token.waveOffset : 0;
      const box = ink.box;
      box.style.left = px(c.x - ink.width / 2);
      box.style.top = px(c.y + wave - ink.height / 2);
      box.style.width = px(ink.width);
      box.style.height = px(ink.height);
      box.appendChild(motions[i]);
      this.root.appendChild(box);
    });
    this.root.style.width = px(this.width);
    this.root.style.height = px(this.height);
    if (parent) {
      // Embedded (readings on a line-timed phrase): in flow, sized by its rows.
      this.root.style.position = "relative";
      parent.appendChild(this.root);
    } else {
      this.root.style.left = px((env.metrics.size.width - this.width) / 2);
      this.root.style.top = px((env.metrics.size.height - this.height) / 2);
      this.rootStyle.show(false);
      env.layer.appendChild(this.root);
    }
  }

  private buildInk(token: ScatteredToken, spec: ScatteredSpec, locale: CjkLocale | null): { el: HTMLDivElement; ink: WordInk } {
    const box = el("div", "ll-word");
    const motion = el("div", "ll-ink");
    const glyphs = el("div", "ll-glyphs", motion);
    applyFont(glyphs, token.font, locale);
    if (isRtl(token.label)) glyphs.dir = "rtl";
    const chars: Styled[] = [];
    let lit: Styled | null = null;
    const held = spec.wordTimed && token.sustain > 0.02 && !isComplexScript(token.label);
    if (held) {
      for (const character of token.characters) {
        const span = el("span", "ll-char", glyphs);
        span.textContent = character;
        span.style.padding = `0 ${px(token.size * 0.02)}`;
        chars.push(new Styled(span));
      }
    } else if (spec.wordTimed && spec.sweep) {
      const dim = el("span", "ll-dim", glyphs);
      dim.textContent = token.label;
      const litSpan = el("span", "ll-lit", glyphs);
      litSpan.textContent = token.label;
      lit = new Styled(litSpan);
    } else {
      glyphs.textContent = token.label;
    }
    if (token.reading) {
      const reading = el("div", "ll-reading", motion);
      reading.textContent = token.reading.text;
      applyFont(reading, token.reading.font, cjkLocale(token.reading.text));
      reading.style.marginTop = px(token.size * 0.02);
    }
    const ink: WordInk = {
      token,
      box,
      boxStyle: new Styled(box),
      motion: new Styled(motion),
      glyphs: new Styled(glyphs),
      lit,
      chars,
      width: 0,
      height: 0,
      frame: newScatteredFrame(),
      entrance: newEntrance(),
      direction: token.index % 2 === 0 ? 1 : -1,
      shadowKey: "",
    };
    return { el: motion, ink };
  }

  setVisible(visible: boolean): void {
    this.rootStyle.show(visible);
  }

  setLayer(z: number): void {
    this.rootStyle.set("zIndex", String(z));
  }

  /** Per-frame update for a word-timed line. */
  update(stage: LineStage, frame: FrameEnv, env: BuildEnv, lyricOpacity: number, available: { width: number; height: number }): void {
    const time = frame.time;
    const behaviour = stage.behaviour;
    let nextIndex = -1;
    for (let i = 0; i < this.inks.length; i++) {
      if (this.inks[i].token.source.start > time) {
        nextIndex = i;
        break;
      }
    }
    for (let i = 0; i < this.inks.length; i++) this.updateWord(this.inks[i], i === nextIndex, time, behaviour, frame.reduced);

    if (this.echo) {
      let revealed = this.starts.length;
      if (behaviour.reveals) {
        revealed = 0;
        for (const start of this.starts) if (time >= start) revealed++;
      }
      this.echo.update(revealed, stage.presence);
    }

    for (const link of this.links) {
      const a = this.inks[link.segment.from].frame;
      const b = this.inks[link.segment.to].frame;
      const shown = Math.min(a.opacity, b.opacity);
      const heat = Math.max(a.heat, b.heat);
      link.group.set("opacity", opacityValue(Math.min(LINK_OPACITY * (1 + 0.45 * heat), 0.75) * shown));
    }

    const pose = stage.pose;
    const fit = presentationScale({ width: this.width, height: this.height }, available, frame.userScale);
    const phraseScale = 0.96 + 0.04 * pose.progress - 0.025 * pose.exit;
    const offsetY = ((1 - pose.progress) * 0.22 - pose.exit * 0.36) * env.metrics.baseSize;
    this.rootStyle.set("transform", `scale(${num(fit)}) translateY(${px(offsetY)}) scale(${num(phraseScale)})`);
    this.rootStyle.set("opacity", opacityValue(stage.presence * lyricOpacity));
    this.rootStyle.set("filter", blurFilter(pose.exit * 3.5));
  }

  private updateWord(ink: WordInk, isNext: boolean, time: number, behaviour: LineStage["behaviour"], reduced: boolean): void {
    const t = ink.token;
    const f = scatteredWordFrame(t.source, isNext, time, behaviour, t.sustain, t.motionSeed, 0, ink.frame);
    const e = wordEntrance(f.progress, this.arrival, t.size, ink.direction, f.opacity, reduced, ink.entrance);
    const box = ink.boxStyle;
    const visible = e.opacity > 0.001;
    box.set("visibility", visible ? "" : "hidden");
    if (!visible) return;
    box.set("opacity", opacityValue(e.opacity));
    box.set("transform", entranceTransform(e, ink.width, ink.height));
    box.set("filter", blurFilter(e.blur));
    box.set("clipPath", entranceClip(e));
    box.set("zIndex", f.heat > 0.05 ? "1" : "0");

    let motion = "";
    if (!reduced) {
      const d = f.drift;
      if (d.x !== 0 || d.y !== 0) motion += `translate(${px(d.x * t.size)},${px(d.y * t.size)}) `;
      if (d.angle !== 0) motion += `rotate(${num(d.angle, 3)}deg) `;
      if (d.scale !== 1) motion += `scale(${num(d.scale)}) `;
    }
    const swell = 1 + 0.07 * f.heat + 0.03 * f.glow;
    if (swell !== 1) motion += `scale(${num(swell)}) `;
    if (t.rotation !== 0) motion += `rotate(${num(t.rotation, 3)}deg)`;
    ink.motion.set("transform", motion.trim() || "none");

    // White glow for held notes, then the soft drop shadow that deepens with heat.
    const heat = quantize(f.heat);
    const glow = quantize(f.glow);
    const fill = quantize(f.fill, 0.01);
    const shadowKey = `${heat}|${glow}|${fill}`;
    const wordShadow = (glow > 0 ? shadow(white(0.45 * glow * fill), 10 + 22 * glow) + ", " : "") + shadow(black(0.28 + 0.24 * heat), 7 + 12 * heat, 2);
    if (shadowKey !== ink.shadowKey) {
      ink.shadowKey = shadowKey;
      ink.motion.set("textShadow", wordShadow);
    }

    if (ink.chars.length > 0) {
      const count = ink.chars.length;
      for (let i = 0; i < count; i++) {
        const lit = smoothstep(f.fill * count - i);
        const c = ink.chars[i];
        c.set("opacity", opacityValue(0.28 + 0.72 * lit));
        const grow = 1 + 0.07 * lit * t.sustain;
        c.set("transform", grow !== 1 ? `scale(${num(grow)})` : "none");
        const charGlow = quantize(glow * lit);
        c.set("textShadow", (charGlow > 0 ? shadow(white(0.55 * charGlow), 6 + 18 * charGlow) + ", " : "") + wordShadow);
      }
    } else if (ink.lit) {
      const p = Math.min(Math.max(f.fill, 0), 1);
      if (p >= 1) {
        ink.lit.show(true);
        ink.lit.set("maskImage", "none");
      } else if (p <= 0) {
        ink.lit.show(false);
      } else {
        ink.lit.show(true);
        const to = this.rtl ? "left" : "right";
        ink.lit.set("maskImage", `linear-gradient(to ${to}, #000 0%, #000 ${num(Math.max(p - 0.12, 0) * 100, 2)}%, transparent ${num(p * 100, 2)}%, transparent 100%)`);
      }
    }
  }

  /** For line-timed phrases with readings: static ink, fully shown. */
  showStatic(): void {
    for (const ink of this.inks) ink.boxStyle.set("visibility", "");
  }

  destroy(): void {
    this.root.remove();
  }
}
