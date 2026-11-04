// Echo's backdrop (LyricComposition.swift `EchoBackdrop`): the phrase
// repeated behind itself at descending sizes, a few copies mirrored through
// the horizontal, all well under the live line. The repeats grow word by
// word with the vocal; every prefix of the line is measured once when the
// line is built, so revealing a word never measures again.

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

export class EchoBackdrop {
  readonly layer: HTMLDivElement;
  private readonly ghosts: { ghost: Ghost; el: HTMLDivElement; style: Styled }[] = [];
  private readonly prefixes: string[];
  private readonly sizes: Size[];
  private shown = -1;

  constructor(words: readonly Word[], seed: number, private readonly baseSize: number, private readonly stage: Size, face: FontChoice, locale: CjkLocale | null, measurer: Measurer) {
    this.layer = el("div", "ll-echo");
    this.layer.setAttribute("aria-hidden", "true");
    this.prefixes = words.map((_, i) => words.slice(0, i + 1).map((w) => w.text).join(" "));
    const font = lyricFont(face, baseSize, false);
    const probes = this.prefixes.map((text) => {
      const probe = el("div", "ll-echo-text");
      probe.textContent = text;
      applyFont(probe, font, locale);
      return probe;
    });
    this.sizes = measurer.measure(probes);
    probes.forEach((p) => p.remove());
    for (const ghost of echoGhosts(seed)) {
      const node = el("div", "ll-echo-text", this.layer);
      applyFont(node, lyricFont(face, baseSize * ghost.scale, false), locale);
      const style = new Styled(node);
      style.show(false);
      this.ghosts.push({ ghost, el: node, style });
    }
  }

  /** `revealed`: how many words of the line are shown; `presence`: the line's presence. */
  update(revealed: number, presence: number): void {
    const count = Math.min(revealed, this.prefixes.length);
    if (count !== this.shown) {
      this.shown = count;
      const stage = { width: (this.stage.width * 2) / 3, height: (this.stage.height * 2) / 3 };
      for (const g of this.ghosts) {
        if (count === 0) {
          g.style.show(false);
          continue;
        }
        g.style.show(true);
        g.el.textContent = this.prefixes[count - 1];
        const base = this.sizes[count - 1];
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
