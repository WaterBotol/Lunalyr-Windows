// Everything that is not a timed lyric line: the idle and "lyrics
// unavailable" notices, the loading indicator, title + artist, the plain
// lyrics block (NowPlayingView.lyricsSection) and the instrumental dots
// (InstrumentalInterludeView.swift).

import type { FontChoice } from "../../contract";
import { cueBob, cueHighlight, type InstrumentalCue } from "../../timing/presentation";
import { presentationScale, type ViewportMetrics } from "../../timing/stage";
import { el, Styled, type Measurer } from "../dom";
import { num, opacityValue, px } from "../entrance";
import { applyFont, cjkLocale, htmlLang, lyricFont, uiFont } from "../fonts";
import { black, shadow, white } from "./common";

/** A short centred notice ("Nothing playing", "Lyrics unavailable"). */
export class NoticeView {
  private readonly root: Styled;
  private readonly text: HTMLDivElement;
  private key = "";

  constructor(parent: HTMLElement) {
    const center = el("div", "ll-center", parent);
    this.text = el("div", "ll-notice", center);
    this.text.setAttribute("role", "status");
    this.root = new Styled(center);
    this.root.show(false);
  }

  show(text: string, size: number, face: FontChoice, opacity: number): void {
    const key = `${text}|${size}|${face}`;
    if (key !== this.key) {
      this.key = key;
      this.text.textContent = text;
      applyFont(this.text, uiFont(face, size), null);
    }
    this.root.show(opacity > 0.01);
    this.root.set("opacity", opacityValue(opacity));
  }

  hide(): void {
    this.root.show(false);
  }
}

/** Indeterminate ring, rotated by the frame loop so it stops with it. */
export class SpinnerView {
  private readonly root: Styled;
  private readonly ring: Styled;

  constructor(parent: HTMLElement) {
    const center = el("div", "ll-center", parent);
    const ring = el("div", "ll-spinner", center);
    ring.setAttribute("role", "progressbar");
    ring.setAttribute("aria-label", "Loading lyrics");
    this.root = new Styled(center);
    this.ring = new Styled(ring);
    this.root.show(false);
  }

  show(nowMs: number, reduced: boolean): void {
    this.root.show(true);
    const turns = reduced ? 0 : (nowMs / 900) % 1;
    this.ring.set("transform", `rotate(${num(turns * 360, 1)}deg)`);
    this.ring.set("opacity", reduced ? opacityValue(0.55 + 0.35 * Math.sin(nowMs / 400)) : "1");
  }

  hide(): void {
    this.root.show(false);
  }
}

/** A static, fitted block: title + artist, or plain (unsynced) lyrics. */
export class BlockView {
  private root: HTMLDivElement | null = null;
  private style: Styled | null = null;
  private key = "";
  private width = 0;
  private height = 0;

  constructor(private readonly parent: HTMLElement) {}

  private build(key: string, metrics: ViewportMetrics, measurer: Measurer, make: (root: HTMLDivElement) => void): void {
    if (key === this.key && this.root) return;
    this.root?.remove();
    const root = el("div", "ll-block");
    make(root);
    const [size] = measurer.measure([root]);
    this.width = size.width;
    this.height = size.height;
    root.style.left = px((metrics.size.width - size.width) / 2);
    root.style.top = px((metrics.size.height - size.height) / 2);
    root.style.width = px(size.width);
    this.parent.appendChild(root);
    this.root = root;
    this.style = new Styled(root);
    this.key = key;
  }

  showTrackName(title: string, artist: string, face: FontChoice, metrics: ViewportMetrics, measurer: Measurer, userScale: number): void {
    const base = metrics.baseSize;
    this.build(`track|${title}|${artist}|${face}|${base}|${metrics.stage.width}`, metrics, measurer, (root) => {
      root.classList.add("ll-trackname");
      root.style.maxWidth = px(metrics.stage.width);
      root.style.textShadow = shadow(black(0.5), 10, 2);
      const t = el("div", "ll-title", root);
      t.textContent = title;
      const locale = cjkLocale(title + artist);
      if (locale) root.lang = htmlLang(locale);
      applyFont(t, lyricFont(face, base * 0.5, true), locale);
      if (artist) {
        const a = el("div", "ll-artist", root);
        a.textContent = artist;
        a.style.marginTop = px(base * 0.12);
        applyFont(a, uiFont(face, base * 0.26), locale);
      }
    });
    this.place(metrics, userScale);
  }

  showPlain(lines: readonly string[], face: FontChoice, metrics: ViewportMetrics, measurer: Measurer, userScale: number): void {
    const base = metrics.baseSize;
    const shown = lines.slice(0, 14);
    this.build(`plain|${shown.join("\n")}|${face}|${base}|${metrics.stage.width}`, metrics, measurer, (root) => {
      root.classList.add("ll-plain");
      root.style.maxWidth = px(metrics.stage.width);
      root.style.textShadow = shadow(black(0.4), 8, 2);
      const locale = cjkLocale(shown.join(" "));
      if (locale) root.lang = htmlLang(locale);
      const font = lyricFont(face, base * 0.26, false);
      shown.forEach((line, i) => {
        const row = el("div", "ll-plain-line", root);
        row.dir = "auto";
        applyFont(row, font, locale);
        if (i > 0) row.style.marginTop = px(base * 0.1);
        if (line.trim().length === 0) {
          row.textContent = " ";
          row.style.opacity = "0";
        } else {
          row.textContent = line;
        }
      });
    });
    this.place(metrics, userScale);
  }

  private place(metrics: ViewportMetrics, userScale: number): void {
    if (!this.style) return;
    const fit = presentationScale({ width: this.width, height: this.height }, metrics.stage, userScale);
    this.style.show(true);
    this.style.set("transform", `scale(${num(fit)})`);
  }

  hide(): void {
    this.style?.show(false);
  }

  reset(): void {
    this.root?.remove();
    this.root = null;
    this.style = null;
    this.key = "";
  }
}

/** Three centred dots that pulse through an instrumental break. */
export class InterludeView {
  private readonly root: Styled;
  private readonly row: HTMLDivElement;
  private readonly dots: Styled[] = [];
  private diameter = 0;

  constructor(parent: HTMLElement) {
    const center = el("div", "ll-center", parent);
    this.row = el("div", "ll-interlude", center);
    this.row.setAttribute("role", "img");
    this.row.setAttribute("aria-label", "Instrumental break");
    for (let i = 0; i < 3; i++) this.dots.push(new Styled(el("div", "ll-dot", this.row)));
    this.root = new Styled(center);
    this.root.show(false);
  }

  show(cue: InstrumentalCue, baseSize: number, accent: [number, number, number], reduced: boolean): void {
    const diameter = Math.min(25, Math.max(15, baseSize * 0.31));
    if (diameter !== this.diameter) {
      this.diameter = diameter;
      this.row.style.gap = px(diameter * 0.65);
      for (const d of this.dots) {
        d.el.style.width = px(diameter);
        d.el.style.height = px(diameter);
      }
    }
    this.root.show(true);
    this.root.set("opacity", opacityValue(cue.opacity));
    const [r, g, b] = accent.map((c) => Math.round(Math.min(1, Math.max(0, c)) * 255));
    this.dots.forEach((dot, i) => {
      const highlight = cueHighlight(cue, i);
      dot.set("background", `rgba(${r},${g},${b},${num(0.32 + highlight * 0.62, 3)})`);
      dot.set("boxShadow", `inset 0 0 0 1px ${white(highlight * 0.22)}`);
      dot.set("transform", reduced ? "none" : `translateY(${px(cueBob(cue, i) * diameter * 0.32)})`);
    });
  }

  hide(): void {
    this.root.show(false);
  }
}
