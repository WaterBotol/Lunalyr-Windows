// Small accessible form controls bound to the settings store. Each control
// renders from the latest Settings and sends `update_settings` patches;
// controls the user is interacting with are not overwritten by echoes.

import type { Settings } from "../contract";

let uid = 0;
export function id(prefix: string): string {
  uid += 1;
  return `${prefix}-${uid}`;
}

export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Record<string, string | boolean | number | undefined> = {},
  ...children: (Node | string | null | undefined | false)[]
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (value === undefined || value === false) continue;
    if (key === "class") node.className = String(value);
    else if (key === "text") node.textContent = String(value);
    else node.setAttribute(key, value === true ? "" : String(value));
  }
  for (const child of children) {
    if (child === null || child === undefined || child === false) continue;
    node.append(child);
  }
  return node;
}

export type Patch = (patch: Partial<Settings>) => void;
export type Refresher = (settings: Settings) => void;

export interface Binder {
  patch: Patch;
  /** Register a function that re-renders a control from new settings. */
  watch(refresh: Refresher): void;
  settings(): Settings;
}

function busy(el: Element): boolean {
  return document.activeElement === el && el.getAttribute("data-dragging") === "1";
}

export function row(label: string | Node, control: Node, help?: string, forId?: string): HTMLDivElement {
  const helpId = help ? id("help") : undefined;
  const labelNode = typeof label === "string" ? h("label", { class: "label", for: forId, text: label }) : label;
  const r = h("div", { class: "row" }, h("div", { class: "row-main" }, labelNode, control));
  if (help && helpId) {
    r.append(h("p", { class: "help", id: helpId, text: help }));
    if (control instanceof HTMLElement) {
      const target = control.matches("input,select,button") ? control : control.querySelector("input,select,button");
      target?.setAttribute("aria-describedby", helpId);
    }
  }
  return r;
}

export function toggle(binder: Binder, label: string, key: keyof Settings, help?: string, disabled?: (s: Settings) => string | null): HTMLDivElement {
  const inputId = id("toggle");
  const input = h("input", { type: "checkbox", id: inputId, role: "switch", class: "switch" });
  input.addEventListener("change", () => binder.patch({ [key]: input.checked } as Partial<Settings>));
  const wrap = row(label, input, help, inputId);
  const note = h("p", { class: "help note" });
  wrap.append(note);
  binder.watch((s) => {
    input.checked = Boolean(s[key]);
    const reason = disabled?.(s) ?? null;
    input.disabled = reason !== null;
    note.textContent = reason ?? "";
    note.hidden = reason === null;
  });
  return wrap;
}

export function select<T extends string>(
  binder: Binder,
  label: string,
  options: [T, string][],
  get: (s: Settings) => T,
  set: (value: T) => Partial<Settings>,
  help?: string,
): HTMLDivElement {
  const selectId = id("select");
  const el = h("select", { id: selectId });
  for (const [value, text] of options) el.append(h("option", { value, text }));
  el.addEventListener("change", () => binder.patch(set(el.value as T)));
  binder.watch((s) => {
    if (document.activeElement !== el) el.value = get(s);
  });
  return row(label, el, help, selectId);
}

export function choice<T extends string>(
  binder: Binder,
  legend: string,
  options: { value: T; label: string; detail?: string; disabled?: boolean }[],
  get: (s: Settings) => T,
  set: (value: T) => Partial<Settings>,
): HTMLFieldSetElement {
  const name = id("choice");
  const fieldset = h("fieldset", { class: "choice" }, h("legend", { text: legend }));
  const inputs: HTMLInputElement[] = [];
  for (const option of options) {
    const inputId = id("opt");
    const input = h("input", { type: "radio", name, id: inputId, value: option.value, disabled: option.disabled });
    input.addEventListener("change", () => {
      if (input.checked) binder.patch(set(option.value));
    });
    inputs.push(input);
    fieldset.append(
      h("label", { class: `choice-option${option.disabled ? " disabled" : ""}`, for: inputId },
        input,
        h("span", { class: "choice-text" }, h("span", { class: "choice-label", text: option.label }), option.detail ? h("span", { class: "choice-detail", text: option.detail }) : null)),
    );
  }
  binder.watch((s) => {
    const value = get(s);
    for (const input of inputs) input.checked = input.value === value;
  });
  return fieldset;
}

export interface SliderOptions {
  min: number;
  max: number;
  step: number;
  format: (v: number) => string;
  help?: string;
}

/** A slider with a live value read-out; sends throttled patches while dragging. */
export function slider(binder: Binder, label: string, get: (s: Settings) => number, set: (v: number) => Partial<Settings>, o: SliderOptions): HTMLDivElement {
  const inputId = id("slider");
  const input = h("input", { type: "range", id: inputId, min: o.min, max: o.max, step: o.step });
  const output = h("output", { for: inputId, class: "value" });
  let timer: ReturnType<typeof setTimeout> | null = null;
  let pending: number | null = null;
  const flush = () => {
    timer = null;
    if (pending !== null) binder.patch(set(pending));
    pending = null;
  };
  input.addEventListener("input", () => {
    const v = Number(input.value);
    output.textContent = o.format(v);
    input.setAttribute("aria-valuetext", o.format(v));
    input.setAttribute("data-dragging", "1");
    pending = v;
    if (!timer) timer = setTimeout(flush, 120);
  });
  input.addEventListener("change", () => {
    input.removeAttribute("data-dragging");
    if (timer) clearTimeout(timer);
    pending = Number(input.value);
    flush();
  });
  const control = h("div", { class: "slider" }, input, output);
  binder.watch((s) => {
    if (busy(input)) return;
    const v = get(s);
    input.value = String(v);
    output.textContent = o.format(v);
    input.setAttribute("aria-valuetext", o.format(v));
  });
  return row(label, control, o.help, inputId);
}

export function button(text: string, onClick: () => void, attrs: Record<string, string | boolean | undefined> = {}): HTMLButtonElement {
  const b = h("button", { type: "button", ...attrs, text });
  b.addEventListener("click", onClick);
  return b;
}

export function section(title: string, ...children: (Node | null | false)[]): HTMLElement {
  const headingId = id("section");
  return h("section", { class: "group", "aria-labelledby": headingId }, h("h2", { id: headingId, text: title }), ...children);
}

export function seconds(v: number, signed = true): string {
  const text = `${Math.abs(v).toFixed(2)} s`;
  if (!signed) return text;
  return v > 0 ? `+${text}` : v < 0 ? `−${text}` : text;
}

export function percent(v: number): string {
  return `${Math.round(v * 100)}%`;
}
