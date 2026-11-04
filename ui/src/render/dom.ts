// DOM helpers for the renderer. Per-frame work only ever WRITES styles, and
// only when a value actually changed; all reads (measurement) happen when a
// line is built, in one batch, before any write of that frame.

export function el<K extends keyof HTMLElementTagNameMap>(tag: K, className?: string, parent?: Element): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (parent) parent.appendChild(node);
  return node;
}

type Prop = "transform" | "opacity" | "filter" | "clipPath" | "textShadow" | "zIndex" | "display" | "visibility" | "maskImage" | "background" | "color" | "boxShadow";

/** Remembers the last value written per property and skips identical writes. */
export class Styled {
  private readonly last: Partial<Record<Prop, string>> = {};
  constructor(readonly el: HTMLElement) {}

  set(prop: Prop, value: string): void {
    if (this.last[prop] === value) return;
    this.last[prop] = value;
    const style = this.el.style as unknown as Record<string, string>;
    if (prop === "maskImage") {
      style.webkitMaskImage = value;
      style.maskImage = value;
    } else {
      style[prop] = value;
    }
  }

  /** Hidden elements skip every other write; toggling uses `display`. */
  show(visible: boolean): void {
    this.set("display", visible ? "" : "none");
  }
}

export interface Size {
  width: number;
  height: number;
}

/**
 * Off-screen measuring host inside the lyric layer (so inherited text styles
 * match). Elements are appended, measured together in one layout, and then
 * moved by the caller into their real parent.
 */
export class Measurer {
  readonly host: HTMLDivElement;

  constructor(parent: HTMLElement) {
    this.host = el("div", "ll-measure", parent);
    this.host.setAttribute("aria-hidden", "true");
  }

  measure(elements: readonly HTMLElement[]): Size[] {
    for (const e of elements) this.host.appendChild(e);
    const sizes = elements.map((e) => {
      const r = e.getBoundingClientRect();
      return { width: r.width, height: r.height };
    });
    return sizes;
  }

  clear(): void {
    this.host.replaceChildren();
  }
}
