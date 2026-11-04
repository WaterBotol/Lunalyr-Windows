import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Word } from "../../contract";
import type { Measurer } from "../dom";
import { px } from "../entrance";
import { EchoBackdrop, echoGhosts, prefixSizes } from "./echo";

/** Just enough of an element for the backdrop (the tests run without a DOM). */
class FakeElement {
  className = "";
  textContent = "";
  readonly style: Record<string, string> = {};
  readonly children: FakeElement[] = [];
  parent: FakeElement | null = null;
  setAttribute(): void {}
  appendChild(child: FakeElement): FakeElement {
    child.remove();
    child.parent = this;
    this.children.push(child);
    return child;
  }
  remove(): void {
    if (!this.parent) return;
    this.parent.children.splice(this.parent.children.indexOf(this), 1);
    this.parent = null;
  }
}

/** Text that measures 10 px per character and 24 px tall, like a monospaced face. */
class FakeMeasurer {
  readonly batches: string[][] = [];
  measure(elements: readonly FakeElement[]): { width: number; height: number }[] {
    this.batches.push(elements.map((e) => e.textContent));
    return elements.map((e) => ({ width: 10 * e.textContent.length, height: e.textContent ? 24 : 0 }));
  }
}

const words = (...texts: string[]): Word[] => texts.map((text, i) => ({ text, start: i, end: i + 1 }));
const STAGE = { width: 1600, height: 900 };

function backdrop(texts: string[], seed = 0): { echo: EchoBackdrop; measurer: FakeMeasurer; ghosts: FakeElement[] } {
  const measurer = new FakeMeasurer();
  const echo = new EchoBackdrop(words(...texts), seed, 60, STAGE, "manrope", null, measurer as unknown as Measurer);
  return { echo, measurer, ghosts: (echo.layer as unknown as FakeElement).children };
}

beforeEach(() => {
  vi.stubGlobal("document", { createElement: () => new FakeElement() });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("Echo backdrop", () => {
  it("measures each word once and one space, in a single batch", () => {
    const { measurer } = backdrop(["one", "two", "three"]);
    expect(measurer.batches).toEqual([["one", "two", "three", " "]]);
  });

  it("sizes each prefix from the words and spaces, as measuring the whole prefix would", () => {
    const seed = 3;
    const { echo, ghosts } = backdrop(["one", "two", "three"], seed);
    const scales = echoGhosts(seed).map((g) => g.scale);
    const expectShown = (text: string) => {
      ghosts.forEach((ghost, i) => {
        expect(ghost.style.display).toBe("");
        expect(ghost.textContent).toBe(text);
        // The whole prefix measures 10 px per character.
        expect(ghost.style.left).toBe(px((-10 * text.length * scales[i]) / 2));
        expect(ghost.style.top).toBe(px((-24 * scales[i]) / 2));
      });
    };
    echo.update(0, 1);
    for (const ghost of ghosts) expect(ghost.style.display).toBe("none");
    echo.update(2, 1);
    expectShown("one two");
    echo.update(3, 1);
    expectShown("one two three");
    // Seeking back rebuilds the shorter prefix.
    echo.update(1, 1);
    expectShown("one");
    echo.update(99, 0.5);
    expectShown("one two three");
  });

  it("stays linear in the length of the line", () => {
    const count = 5000;
    const { echo, measurer, ghosts } = backdrop(Array.from({ length: count }, (_, i) => `w${i % 10}`));
    expect(measurer.batches.length).toBe(1);
    expect(measurer.batches[0].length).toBe(count + 1);
    expect(measurer.batches[0].join("").length).toBe(count * 2 + 1);
    for (let revealed = 1; revealed <= count; revealed++) echo.update(revealed, 1);
    expect(ghosts[0].textContent.length).toBe(count * 2 + (count - 1));
    expect(ghosts[0].style.left).toBe(px((-10 * (count * 3 - 1) * echoGhosts(0)[0].scale) / 2));
  });

  it("adds widths and keeps the tallest height", () => {
    const { widths, heights } = prefixSizes(
      [{ width: 30, height: 20 }, { width: 0, height: 0 }, { width: 50, height: 28 }],
      { width: 8, height: 22 },
    );
    expect([...widths]).toEqual([30, 38, 96]);
    expect([...heights]).toEqual([20, 22, 28]);
    expect(prefixSizes([], { width: 8, height: 22 }).widths.length).toBe(0);
  });
});
