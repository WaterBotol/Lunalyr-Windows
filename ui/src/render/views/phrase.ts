// Line-timed lyrics: the whole line enters as one phrase with the
// composition's entrance (NowPlayingView.lineTimedLyric). While the next line
// is due, only its first word shows, faintly; as the phrase arrives that
// preview bridges into it.

import type { Line, Word } from "../../contract";
import { arrivalFor, travelBounds, travelRotation, type ConcreteComposition, type WordArrival } from "../../timing/composition";
import { wordVisibility } from "../../timing/geometry";
import { readingPairs } from "../../timing/lines";
import { hasLetterOrNumber } from "../../timing/math";
import type { LineRole, LineStage } from "../../timing/presentation";
import { presentationScale } from "../../timing/stage";
import { el, Styled } from "../dom";
import { blurFilter, entranceClip, entranceTransform, newEntrance, num, opacityValue, px, wordEntrance, type Entrance } from "../entrance";
import { applyFont, cjkLocale, htmlLang, isRtl, lyricFont } from "../fonts";
import type { BuildEnv, FrameEnv } from "./common";
import { ScatteredLineView } from "./scattered";

export class PhraseLineView {
  readonly root: HTMLDivElement;
  readonly width: number;
  readonly height: number;
  private readonly rootStyle: Styled;
  private readonly previewStyle: Styled;
  private readonly phraseStyle: Styled;
  private readonly arrival: WordArrival;
  private readonly pointSize: number;
  private readonly phraseWidth: number;
  private readonly phraseHeight: number;
  private readonly direction: number;
  private readonly entrance: Entrance = newEntrance();

  constructor(private readonly line: Line, index: number, readings: Word[] | null, composition: ConcreteComposition, env: BuildEnv) {
    const { baseSize, stage, size } = env.metrics;
    this.arrival = arrivalFor(composition);
    this.direction = index % 2 === 0 ? 1 : -1;
    const annotations = env.readings === "below" ? readingPairs(readings, line.words) : new Map<string, string>();
    this.pointSize = baseSize * (composition === "spoken" ? 0.86 : 0.72);
    const travel = travelBounds(this.arrival, this.pointSize);
    const rotates = travelRotation(this.arrival) > 0;
    const padH = travel.width / 2 + (rotates ? this.pointSize * 0.45 : 0);
    const padV = travel.height / 2 + (rotates ? stage.width * 0.11 : 0);
    const displayWords = env.readings === "replace" ? readings ?? line.words : line.words;
    const previewText = displayWords.find((w) => hasLetterOrNumber(w.text))?.text ?? "";
    const font = lyricFont(env.face, this.pointSize, composition === "spoken");
    const text = env.readings === "replace" && readings ? readings.map((w) => w.text).join(" ") : line.text;
    const locale = cjkLocale(text);

    this.root = el("div", "ll-view ll-phrase-line");
    if (locale) this.root.lang = htmlLang(locale);
    this.rootStyle = new Styled(this.root);

    const preview = el("div", "ll-phrase-preview");
    preview.textContent = previewText;
    applyFont(preview, font, locale);
    if (isRtl(previewText)) preview.dir = "rtl";
    this.previewStyle = new Styled(preview);

    const phrase = el("div", "ll-phrase");
    this.phraseStyle = new Styled(phrase);
    let inner: ScatteredLineView | null = null;
    if (annotations.size === 0) {
      const body = el("div", "ll-phrase-text", phrase);
      body.textContent = text;
      body.dir = "auto";
      applyFont(body, font, locale);
      body.style.maxWidth = px(Math.max(80, stage.width - 2 * padH));
    } else {
      inner = new ScatteredLineView({
        words: line.words,
        annotations,
        composition: "editorial",
        seed: index,
        wordTimed: false,
        baseSize,
        sweep: false,
      }, env, phrase);
      inner.showStatic();
    }
    const [phraseSize, previewSize] = env.measurer.measure([phrase, preview]);
    this.phraseWidth = phraseSize.width;
    this.phraseHeight = phraseSize.height;
    this.width = Math.max(phraseSize.width, previewSize.width) + 2 * padH;
    this.height = Math.max(phraseSize.height, previewSize.height) + 2 * padV;
    for (const [node, s] of [[phrase, phraseSize], [preview, previewSize]] as const) {
      node.style.left = px((this.width - s.width) / 2);
      node.style.top = px((this.height - s.height) / 2);
      node.style.width = px(s.width);
      node.style.height = px(s.height);
      this.root.appendChild(node);
    }
    this.root.style.width = px(this.width);
    this.root.style.height = px(this.height);
    this.root.style.left = px((size.width - this.width) / 2);
    this.root.style.top = px((size.height - this.height) / 2);
    this.rootStyle.show(false);
    env.layer.appendChild(this.root);
  }

  setVisible(visible: boolean): void {
    this.rootStyle.show(visible);
  }

  setLayer(z: number): void {
    this.rootStyle.set("zIndex", String(z));
  }

  update(stage: LineStage, role: LineRole, frame: FrameEnv, lyricOpacity: number, available: { width: number; height: number }): void {
    const time = frame.time;
    const previewing = role === "preview";
    const pose = stage.pose;
    const previewOpacity = previewing
      ? wordVisibility(time, this.line.start, this.line.end, true, true, stage.behaviour.lead)
      : 0.25 * (1 - pose.progress);
    const showPreview = (previewing || stage.bridgesPreview) && previewOpacity > 0.001;
    this.previewStyle.set("visibility", showPreview ? "" : "hidden");
    if (showPreview) this.previewStyle.set("opacity", opacityValue(previewOpacity));
    this.phraseStyle.set("visibility", previewing ? "hidden" : "");
    if (!previewing) {
      const e = wordEntrance(pose.progress, this.arrival, this.pointSize, this.direction, pose.presence, frame.reduced, this.entrance);
      this.phraseStyle.set("opacity", opacityValue(e.opacity));
      this.phraseStyle.set("transform", entranceTransform(e, this.phraseWidth, this.phraseHeight));
      this.phraseStyle.set("filter", blurFilter(e.blur));
      this.phraseStyle.set("clipPath", entranceClip(e));
    }
    const fit = presentationScale({ width: this.width, height: this.height }, available, frame.userScale);
    this.rootStyle.set("transform", `scale(${num(fit)})`);
    this.rootStyle.set("opacity", opacityValue(previewing ? 1 : lyricOpacity));
  }

  destroy(): void {
    this.root.remove();
  }
}
