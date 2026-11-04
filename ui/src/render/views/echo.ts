// Echo's backdrop (LyricComposition.swift `EchoBackdrop`): the phrase
// repeated behind itself at descending sizes, a few copies mirrored through
// the horizontal, all well under the live line. The repeats grow word by
// word with the vocal. Each word (and one space) is measured once, in one
// batch, when the line is built; a prefix's size is the running sum of its
// words and spaces, and its text is built only when it is shown, so neither
// grows with the square of the line's length (lyrics are untrusted text).

import type { FontChoice, Word } from "../../contract";
import { wordStageFit, type Size } from "../../timing/stage";
import { el, Styled, type Measurer } from "../dom";
import { num, opacityValue, px } from "../entrance";
import { applyFont, lyricFont, type CjkLocale } from "../fonts";

interface Ghost {
  scale: number;
  dx: number;
  dy: number;
  angle: number;
  opacity: number;
  flipped: boolean;
}

const VARIANTS: Ghost[] = [
  { scale: 0.85, dx: -0.3, dy: -0.62, angle: -2.0, opacity: 0.055, flipped: true },
  { scale: 0.42, dx: 0.46, dy: -0.44, angle: 1.5, opacity: 0.085, flipped: false },
  { scale: 0.3, dx: -0.5, dy: 0.48, angle: -1.0, opacity: 0.1, flipped: false },
  { scale: 0.6, dx: 0.34, dy: 0.66, angle: 2.5, opacity: 0.065, flipped: true },
  { scale: 0.22, dx: 0.1, dy: -0.78, angle: 0.0, opacity: 0.12, flipped: false },
];

/** Fixed per line: the seed rotates which variant leads. */
export function echoGhosts(seed: number): Ghost[] {
  const offset = Math.abs(seed) % VARIANTS.length;
  return [...VARIANTS.slice(offset), ...VARIANTS.slice(0, offset)];
}

const ECHO_STRENGTH = 1; // Settings.echoStrength default

/**
 * Sizes of the line's prefixes (words joined by single spaces) from each
 * word's size and the space's: widths add up, the height is the tallest
 * part so far. `widths[i]` / `heights[i]` belong to the first i + 1 words.
 */
export function prefixSizes(words: readonly Size[], space: Size): { widths: Float64Array; heights: Float64Array } {
  const widths = new Float64Array(words.length);
  const heights = new Float64Array(words.length);
  let width = 0;
  let height = 0;
  for (let i = 0; i < words.length; i++) {
    if (i > 0) {
      width += space.width;
      height = Math.max(height, space.height);
    }
    width += words[i].width;
    height = Math.max(height, words[i].height);
    widths[i] = width;
    heights[i] = height;
  }
  return { widths, heights };
}

export class EchoBackdrop {
  readonly layer: HTMLDivElement;
  private readonly ghosts: { ghost: Ghost; el: HTMLDivElement; style: Styled }[] = [];
  private readonly words: string[];
  private readonly widths: Float64Array;
  private readonly heights: Float64Array;
  private shown = -1;
  /** The text of the first `prefixCount` words, extended as words are revealed. */
  private prefix = "";
  private prefixCount = 0;

  constructor(words: readonly Word[], seed: number, private readonly baseSize: number, private readonly stage: Size, face: FontChoice, locale: CjkLocale | null, measurer: Measurer) {
    this.layer = el("div", "ll-echo");
    this.layer.setAttribute("aria-hidden", "true");
    this.words = words.map((w) => w.text);
    const font = lyricFont(face, baseSize, false);
    // `.ll-echo-text` keeps white space (white-space: pre), so the lone space measures too.
    const probes = [...this.words, " "].map((text) => {
      const probe = el("div", "ll-echo-text");
      probe.textContent = text;
      applyFont(probe, font, locale);
      return probe;
    });
    const sizes = measurer.measure(probes);
    probes.forEach((p) => p.remove());
    const prefixes = prefixSizes(sizes.slice(0, -1), sizes[sizes.length - 1]);
    this.widths = prefixes.widths;
    this.heights = prefixes.heights;
    for (const ghost of echoGhosts(seed)) {
      const node = el("div", "ll-echo-text", this.layer);
      applyFont(node, lyricFont(face, baseSize * ghost.scale, false), locale);
      const style = new Styled(node);
      style.show(false);
      this.ghosts.push({ ghost, el: node, style });
    }
  }

  /** The first `count` words joined by spaces; built on demand, extended in place while words are revealed. */
  private prefixText(count: number): string {
    if (count < this.prefixCount) {
      this.prefix = "";
      this.prefixCount = 0;
    }
    while (this.prefixCount < count) {
      this.prefix += this.prefixCount === 0 ? this.words[0] : ` ${this.words[this.prefixCount]}`;
      this.prefixCount++;
    }
    return this.prefix;
  }

  /** `revealed`: how many words of the line are shown; `presence`: the line's presence. */
  update(revealed: number, presence: number): void {
    const count = Math.max(0, Math.min(revealed, this.words.length));
    if (count !== this.shown) {
      this.shown = count;
      const stage = { width: (this.stage.width * 2) / 3, height: (this.stage.height * 2) / 3 };
      const text = count > 0 ? this.prefixText(count) : "";
      for (const g of this.ghosts) {
        if (count === 0) {
          g.style.show(false);
          continue;
        }
        g.style.show(true);
        g.el.textContent = text;
        const base = { width: this.widths[count - 1], height: this.heights[count - 1] };
        const size = { width: base.width * g.ghost.scale, height: base.height * g.ghost.scale };
        const x = this.baseSize * g.ghost.dx * 3.2;
        const y = this.baseSize * g.ghost.dy * 1.5;
        const fit = wordStageFit(size, stage, { x: stage.width / 2 + x, y: stage.height / 2 + y }, 1, g.ghost.angle, 4);
        g.el.style.left = px(-size.width / 2);
        g.el.style.top = px(-size.height / 2);
        g.style.set("transform", `translate(${px(x)},${px(y)}) scale(1,${g.ghost.flipped ? -1 : 1}) rotate(${num(g.ghost.angle, 2)}deg) scale(${num(fit)})`);
      }
    }
    if (count === 0) return;
    for (const g of this.ghosts) g.style.set("opacity", opacityValue(g.ghost.opacity * presence * ECHO_STRENGTH));
  }
}
