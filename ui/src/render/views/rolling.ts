// The free lyric stage of the rolling layouts — Spoken, Cascade, Cutout —
// ported from SpokenTypographyView.swift. Each sung word takes focus; the
// whole cluster follows every onset (motion handoff) and recent words settle
// around the hero via `rollingPose`, fading while still inside the stage.
//
// One stage persists across line changes inside a word-timed run, so words
// keep their DOM node (keyed by timed-word identity) and are measured once.

import type { Word } from "../../contract";
import { arrivalFor, travelBounds, travelRotation, type ConcreteComposition, type WordArrival, type WordBehaviour } from "../../timing/composition";
import { smoothstep } from "../../timing/math";
import { motionHandoff, newPose, rollingPose, type RollingStyle, type WordPose } from "../../timing/motion";
import { wordPresentation, type WordPresentation } from "../../timing/presentation";
import { presentationScale, wordStageFit } from "../../timing/stage";
import { el, Styled } from "../dom";
import { blurFilter, entranceClip, entranceTransform, newEntrance, num, opacityValue, px, wordEntrance, type Entrance } from "../entrance";
import { applyFont, cjkLocale, htmlLang, isRtl } from "../fonts";
import { prepareSpokenWord, type SpokenToken } from "../typography";
import { black, shadow, type BuildEnv, type FrameEnv } from "./common";

interface Token {
  token: SpokenToken;
  root: HTMLDivElement;
  outer: Styled;
  inner: Styled;
  glyphs: Styled;
  tile: Styled | null;
  width: number;
  height: number;
  pose: WordPose;
  presentation: WordPresentation;
  entrance: Entrance;
  lastUsed: number;
  direction: number;
}

export interface RollingUpdate {
  words: readonly Word[];
  annotations: ReadonlyMap<string, string>;
  composition: ConcreteComposition;
  behaviour: WordBehaviour;
  presence: number;
  lyricOpacity: number;
  available: { width: number; height: number };
}

export class RollingStageView {
  readonly root: HTMLDivElement;
  private readonly rootStyle: Styled;
  private readonly tokens = new Map<string, Token>();
  private composition: ConcreteComposition | null = null;
  private arrival: WordArrival = "glide";
  private readonly width: number;
  private readonly height: number;
  private frameCounter = 0;

  constructor(private readonly env: BuildEnv) {
    const { stage, baseSize, size } = env.metrics;
    this.width = stage.width;
    this.height = baseSize * 7.2;
    this.root = el("div", "ll-view ll-rolling", env.layer);
    this.root.style.width = px(this.width);
    this.root.style.height = px(this.height);
    this.root.style.left = px((size.width - this.width) / 2);
    this.root.style.top = px((size.height - this.height) / 2);
    this.rootStyle = new Styled(this.root);
    this.rootStyle.show(false);
  }

  setVisible(visible: boolean): void {
    this.rootStyle.show(visible);
  }

  setLayer(z: number): void {
    this.rootStyle.set("zIndex", String(z));
  }

  /** Creates and measures tokens for words not seen before (one batch). Call before any style writes. */
  prepare(words: readonly Word[], annotations: ReadonlyMap<string, string>, composition: ConcreteComposition): void {
    if (composition !== this.composition) {
      for (const t of this.tokens.values()) t.root.remove();
      this.tokens.clear();
      this.composition = composition;
      this.arrival = arrivalFor(composition);
    }
    const created: Token[] = [];
    for (let i = 0; i < words.length; i++) {
      const word = words[i];
      const key = `${word.start}|${word.end}|${word.text}`;
      if (this.tokens.has(key)) continue;
      const token = prepareSpokenWord(word, i, this.env.metrics.baseSize, composition, this.env.face, annotations);
      const t = this.build(token, composition);
      this.tokens.set(key, t);
      created.push(t);
    }
    if (created.length === 0) return;
    const sizes = this.env.measurer.measure(created.map((t) => t.root));
    created.forEach((t, i) => {
      t.width = sizes[i].width;
      t.height = sizes[i].height;
      t.root.style.width = px(t.width);
      t.root.style.height = px(t.height);
      t.outer.show(false);
      this.root.appendChild(t.root);
    });
  }

  private build(token: SpokenToken, composition: ConcreteComposition): Token {
    const root = el("div", "ll-token");
    const inner = el("div", "ll-token-ink", root);
    let tile: Styled | null = null;
    if (token.tile) {
      const tileEl = el("div", "ll-tile", inner);
      tile = new Styled(tileEl);
      inner.classList.add("ll-tiled");
    }
    const glyphs = el("div", "ll-glyphs", inner);
    glyphs.textContent = token.source.text;
    const locale = cjkLocale(token.source.text);
    if (locale) root.lang = htmlLang(locale);
    if (isRtl(token.source.text)) glyphs.dir = "rtl";
    applyFont(glyphs, token.font, locale);
    glyphs.style.textShadow = shadow(black(0.25), 7, 2);
    if (token.reading) {
      const reading = el("div", "ll-reading", inner);
      reading.textContent = token.reading.text;
      applyFont(reading, token.reading.font, cjkLocale(token.reading.text));
      reading.style.marginTop = px(token.pointSize * 0.02);
      reading.style.textShadow = shadow(black(0.25), 7, 2);
    }
    const glyphStyle = new Styled(glyphs);
    if (composition === "cascade" && token.hero) glyphs.classList.add("ll-accent");
    return {
      token,
      root,
      outer: new Styled(root),
      inner: new Styled(inner),
      glyphs: glyphStyle,
      tile,
      width: 0,
      height: 0,
      pose: newPose(),
      presentation: { progress: 1, opacity: 1, preview: false },
      entrance: newEntrance(),
      lastUsed: 0,
      direction: token.hash.mod(2) === 0 ? 1 : -1,
    };
  }

  update(u: RollingUpdate, frame: FrameEnv): void {
    const time = frame.time;
    const reduced = frame.reduced;
    const words = u.words;
    const baseSize = this.env.metrics.baseSize;
    this.frameCounter++;
    // Cursor: sum of handoffs (reduced motion: a step per onset).
    let cursor = 0;
    let nextIndex = -1;
    for (let i = 0; i < words.length; i++) {
      const w = words[i];
      if (reduced) cursor += time >= w.start ? 1 : 0;
      else cursor += motionHandoff(time - w.start, i + 1 < words.length ? words[i + 1].start - w.start : w.end - w.start);
      if (nextIndex < 0 && w.start > time) nextIndex = i;
    }
    const style: RollingStyle = u.composition === "spoken" ? 0 : u.composition === "cascade" ? 1 : 2;
    const columnShift = u.composition === "spoken" ? 0 : -this.width * 0.13;
    const stageSize = { width: this.width, height: this.height };
    for (let i = 0; i < words.length; i++) {
      const w = words[i];
      const t = this.tokens.get(`${w.start}|${w.end}|${w.text}`);
      if (!t) continue;
      t.lastUsed = this.frameCounter;
      const rank = cursor - 1 - i;
      if (rank < -2 || rank > 5.2) {
        t.outer.show(false);
        continue;
      }
      const p = wordPresentation(w, i === nextIndex, time, u.behaviour, t.presentation);
      const pose = rollingPose(style, rank, time - w.start, w.end - w.start, t.token.hash, 0, reduced, t.pose);
      const shown = p.opacity * (p.preview ? 1 : pose.opacity);
      if (shown <= 0.001) {
        t.outer.show(false);
        continue;
      }
      t.outer.show(true);
      const focus = Math.exp(-rank * rank * 2.4);
      const cx = this.width * 0.5 + columnShift + pose.x * baseSize;
      const cy = this.height * 0.54 + pose.y * baseSize;
      const entering = !reduced && p.progress < 1;
      const reach = travelBounds(this.arrival, t.token.pointSize, entering);
      const fit = wordStageFit({ width: t.width + reach.width, height: t.height + reach.height }, stageSize, { x: cx, y: cy }, pose.scale,
        Math.abs(pose.angle) + (entering ? travelRotation(this.arrival) : 0), baseSize * 0.22);
      t.outer.set("transform", `translate(${px(cx - t.width / 2)},${px(cy - t.height / 2)}) rotate(${num(pose.angle, 3)}deg) scale(${num(pose.scale * fit)})`);
      t.outer.set("filter", blurFilter(pose.blur));
      t.outer.set("zIndex", String(Math.round(focus * 1000)));
      const e = wordEntrance(p.progress, this.arrival, t.token.pointSize, t.direction, shown, reduced, t.entrance);
      t.inner.set("opacity", opacityValue(e.opacity));
      t.inner.set("transform", entranceTransform(e, t.width, t.height));
      t.inner.set("filter", blurFilter(e.blur));
      t.inner.set("clipPath", entranceClip(e));
      if (t.tile) {
        const reveal = p.preview ? 1 : Math.max(0.05, smoothstep(p.progress));
        t.tile.set("transform", `scaleX(${num(reveal)})${reduced ? "" : " rotate(-1.2deg)"}`);
        t.tile.set("background", frame.accent);
      }
      if (u.composition === "cascade" && t.token.hero) t.glyphs.set("color", frame.accent);
    }
    // Words that left the window: hide now, drop after a while.
    for (const [key, t] of this.tokens) {
      if (t.lastUsed === this.frameCounter) continue;
      t.outer.show(false);
      if (this.frameCounter - t.lastUsed > 240) {
        t.root.remove();
        this.tokens.delete(key);
      }
    }
    const fit = presentationScale(stageSize, u.available, frame.userScale);
    this.rootStyle.set("transform", `scale(${num(fit)})`);
    this.rootStyle.set("opacity", opacityValue(u.presence * u.lyricOpacity));
  }

  destroy(): void {
    this.root.remove();
    this.tokens.clear();
  }
}
