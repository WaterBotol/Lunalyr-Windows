import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FrameLoop, PRECISE_SLACK_MS, TIMER_SLACK_MS } from "./loop";

/** Windows' default timer resolution. */
const WINDOWS_TICK = 15.625;

/**
 * A display on vitest's fake clock: animation frames run at each vsync
 * (multiples of 1000 / hz, with the vsync time as timestamp) and timers fire
 * in between. With a `granularity`, timers fire late, at the next multiple of
 * it, like Windows' default 15.6 ms timer resolution.
 */
class Display {
  readonly period: number;
  rafCalls = 0;
  timerWakes = 0;
  /** No animation frames (the host's occlusion throttling behind the desktop icons). */
  stalled = false;
  private readonly callbacks = new Map<number, FrameRequestCallback>();
  private nextId = 0;
  private vsync: number;

  constructor(hz: number, public granularity = 0) {
    this.period = 1000 / hz;
    this.vsync = Math.floor(performance.now() / this.period);
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      this.callbacks.set(++this.nextId, callback);
      return this.nextId;
    });
    vi.stubGlobal("cancelAnimationFrame", (id: number) => {
      this.callbacks.delete(id);
    });
    const fakeSetTimeout = globalThis.setTimeout;
    vi.stubGlobal("setTimeout", (handler: () => void, ms = 0) => {
      const at = performance.now() + ms;
      const late = this.granularity > 0 ? Math.ceil(at / this.granularity) * this.granularity - at : 0;
      return fakeSetTimeout(() => {
        this.timerWakes++;
        handler();
      }, ms + late);
    });
  }

  get pendingFrames(): number {
    return this.callbacks.size;
  }

  /** Runs for `ms`: timers fire on the fake clock, animation frames at every vsync. */
  run(ms: number): void {
    const end = performance.now() + ms;
    for (;;) {
      const t = (this.vsync + 1) * this.period;
      if (t > end + 1e-9) break;
      vi.advanceTimersByTime(t - performance.now());
      this.vsync++;
      if (this.stalled) continue;
      const due = [...this.callbacks.values()];
      this.callbacks.clear();
      for (const callback of due) {
        this.rafCalls++;
        callback(t);
      }
    }
    vi.advanceTimersByTime(Math.max(0, end - performance.now()));
  }

  /** Wake-ups per second while running for `ms`. */
  wakeRate(ms: number): { raf: number; timers: number } {
    const raf = this.rafCalls;
    const timers = this.timerWakes;
    this.run(ms);
    return { raf: ((this.rafCalls - raf) * 1000) / ms, timers: ((this.timerWakes - timers) * 1000) / ms };
  }
}

/** The vsyncs an always-rAF loop draws on (the cadence rule in FrameLoop.runFrame). */
function referenceFrames(hz: number, fps: number, ms: number): number[] {
  const period = 1000 / hz;
  const interval = 1000 / fps;
  const frames: number[] = [];
  let last = -Infinity;
  for (let k = 1; k * period <= ms + 1e-9; k++) {
    const t = k * period;
    const elapsed = t - last;
    if (elapsed < interval - 1) continue;
    last = elapsed > interval * 2 ? t : last + interval;
    frames.push(k);
  }
  return frames;
}

const vsyncIndex = (display: Display, frames: number[]) => frames.map((t) => Math.round(t / display.period));

function start(fps: number): { loop: FrameLoop; frames: number[] } {
  const frames: number[] = [];
  const loop = new FrameLoop((now) => frames.push(now));
  loop.setFps(fps);
  loop.setActive(true);
  return { loop, frames };
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe("frame loop", () => {
  it.each([
    [60, 15],
    [144, 15],
    [144, 30],
    [240, 30],
  ])("on a %i Hz display at %i fps, sleeps between frames and still draws each on its vsync", (hz, fps) => {
    const display = new Display(hz);
    const { loop, frames } = start(fps);
    display.run(5_000);
    expect(vsyncIndex(display, frames)).toEqual(referenceFrames(hz, fps, 5_000));
    expect(loop.vsync).toBeCloseTo(1000 / hz, 6);
    // Timers have fired on time, so sleeps now end just before each frame:
    // one timer and about one animation frame per frame drawn.
    expect(loop.timerSlack).toBe(PRECISE_SLACK_MS);
    const rate = display.wakeRate(5_000);
    expect(rate.raf).toBeLessThanOrEqual(2 * fps + 1);
    expect(rate.timers).toBeLessThanOrEqual(fps + 1);
    expect(vsyncIndex(display, frames)).toEqual(referenceFrames(hz, fps, 10_000));
  });

  it.each([
    [60, 15],
    [144, 15],
    [144, 30],
    [240, 30],
  ])("allows for 15.6 ms timer granularity: on %i Hz at %i fps no frame is late", (hz, fps) => {
    const display = new Display(hz, WINDOWS_TICK);
    const { loop, frames } = start(fps);
    const rate = display.wakeRate(10_000);
    expect(vsyncIndex(display, frames)).toEqual(referenceFrames(hz, fps, 10_000));
    expect(loop.timerSlack).toBe(TIMER_SLACK_MS);
    // Still fewer wake-ups than refreshes.
    expect(rate.raf + rate.timers).toBeLessThan(hz);
  });

  it("goes back to the 16 ms allowance at the first late timer", () => {
    const display = new Display(144);
    const { loop, frames } = start(30);
    display.run(5_000);
    expect(loop.timerSlack).toBe(PRECISE_SLACK_MS);
    display.granularity = WINDOWS_TICK;
    display.run(5_000);
    expect(loop.timerSlack).toBe(TIMER_SLACK_MS);
    // At most the frame of that first late timer slips by a vsync; the cadence is kept.
    const expected = new Set(referenceFrames(144, 30, 10_000));
    const drawn = vsyncIndex(display, frames);
    expect(drawn.length).toBe(expected.size);
    expect(drawn.filter((k) => !expected.has(k)).length).toBeLessThanOrEqual(1);
  });

  it.each([
    [60, 30],
    [60, 60],
    [144, 60],
    [120, 30],
  ])("on a %i Hz display at %i fps, keeps to animation frames when a sleep would not skip two vsyncs", (hz, fps) => {
    const display = new Display(hz);
    const { frames } = start(fps);
    display.run(10_000);
    expect(vsyncIndex(display, frames)).toEqual(referenceFrames(hz, fps, 10_000));
    // Only the once-a-second stall check runs on a timer.
    expect(display.timerWakes).toBeLessThanOrEqual(10);
  });

  it("learns the refresh interval from animation frame timestamps", () => {
    const display = new Display(144);
    const { loop } = start(60);
    expect(loop.vsync).toBeCloseTo(1000 / 60, 6);
    display.run(500);
    expect(loop.vsync).toBeCloseTo(1000 / 144, 6);
  });

  it("stops completely while inactive, even mid-sleep", () => {
    const display = new Display(60);
    const { loop, frames } = start(15);
    display.run(1000);
    while (display.pendingFrames > 0) display.run(display.period);
    expect(vi.getTimerCount()).toBe(2); // asleep: the sleep and the stall check
    loop.setActive(false);
    expect(vi.getTimerCount()).toBe(0);
    expect(display.pendingFrames).toBe(0);
    const drawn = frames.length;
    display.run(5000);
    expect(frames.length).toBe(drawn);
    expect(vi.getTimerCount()).toBe(0);
    // Resuming draws on the very next animation frame.
    loop.setActive(true);
    display.run(display.period);
    expect(frames.length).toBe(drawn + 1);
  });

  it("drives frames from a timer while animation frames are stalled, then returns to them", () => {
    const display = new Display(60);
    const { frames } = start(15);
    display.run(2000);
    display.stalled = true;
    let drawn = frames.length;
    display.run(4000);
    // The once-a-second check notices within two seconds and keeps the rate.
    expect(frames.length - drawn).toBeGreaterThanOrEqual(15 * 2);
    display.stalled = false;
    display.run(1000);
    drawn = frames.length;
    const rafBefore = display.rafCalls;
    display.run(2000);
    expect(frames.length - drawn).toBeGreaterThanOrEqual(29);
    expect(frames.length - drawn).toBeLessThanOrEqual(31);
    expect(display.rafCalls).toBeGreaterThan(rafBefore);
    // Back on vsyncs.
    for (const t of frames.slice(drawn)) expect(Math.abs(t / display.period - Math.round(t / display.period))).toBeLessThan(1e-6);
  });

  it("a new frame rate ends a sleep planned for the old one", () => {
    const display = new Display(60);
    const { loop, frames } = start(15);
    display.run(1000);
    while (display.pendingFrames > 0) display.run(display.period);
    loop.setFps(60);
    const drawn = frames.length;
    display.run(100);
    expect(frames.length - drawn).toBeGreaterThanOrEqual(5);
  });
});
