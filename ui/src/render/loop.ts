// The frame loop: requestAnimationFrame throttled to the chosen frame rate
// (15 / 30 / 60), fully stopped — no rAF, no timers — while inactive (the
// wallpaper is covered, the user paused it, the session is locked or the
// page is hidden). Resuming renders on the very next animation frame.
//
// Frames are always drawn in an animation frame (vsync-aligned), but between
// frames the loop sleeps on a timer whenever that skips at least two vsyncs
// (the vsync interval is measured from rAF timestamps), so at 15 or 30 fps on
// a 60–144 Hz display the page wakes in one short burst per frame instead of
// on every refresh. A sleep ends early enough to allow for Windows' ~16 ms
// timer granularity, then rAF lands the frame on its vsync. Only after a long
// run of timers that all fired on time does the loop trust them and end
// sleeps closer to the frame; the first late timer restores the allowance.

export type FrameCallback = (now: number) => void;

/** How late a timer may fire: Windows' default timer resolution is 15.6 ms. */
export const TIMER_SLACK_MS = 16;
/** The allowance once timers have proven precise. */
export const PRECISE_SLACK_MS = 4;
/** Timers count as precise after this many in a row fired within PRECISE_LATENESS_MS. */
export const PRECISE_RUN = 32;
const PRECISE_LATENESS_MS = 2;
/** A frame may start this much before its slot (rAF timestamps jitter). */
const FRAME_TOLERANCE_MS = 1;
const VSYNC_SAMPLES = 8;
const DEFAULT_VSYNC_MS = 1000 / 60;

export class FrameLoop {
  private interval = 1000 / 30;
  private last = -Infinity;
  private handle: number | null = null;
  private sleeper: ReturnType<typeof setTimeout> | null = null;
  private watchdog: ReturnType<typeof setTimeout> | null = null;
  private rafSeen = false;
  private active = false;
  /** Timestamp of the previous tick when it requested this one directly (consecutive vsyncs). */
  private chained: number | null = null;
  private readonly samples = new Float64Array(VSYNC_SAMPLES);
  private readonly sorted = new Float64Array(VSYNC_SAMPLES);
  private sampleCount = 0;
  private sampleNext = 0;
  private vsyncMs = DEFAULT_VSYNC_MS;
  /** When the pending sleep should end (performance time). */
  private wakeAt = 0;
  /** Consecutive sleeps that ended within PRECISE_LATENESS_MS of `wakeAt`. */
  private onTime = 0;

  private readonly tick = (now: number) => {
    this.handle = null;
    this.rafSeen = true;
    if (!this.active) return;
    if (this.chained !== null) this.sampleVsync(now - this.chained);
    this.runFrame(now);
    this.scheduleNext(now);
  };
  private readonly wake = () => {
    this.sleeper = null;
    if (!this.active) return;
    this.onTime = performance.now() - this.wakeAt <= PRECISE_LATENESS_MS ? this.onTime + 1 : 0;
    this.chained = null;
    this.handle = requestAnimationFrame(this.tick);
  };
  // Some hosts stop rAF for a window they consider occluded without marking
  // the page hidden. A once-a-second check then drives frames from a timer
  // until rAF comes back.
  private readonly check = () => {
    this.watchdog = null;
    if (!this.active) return;
    if (this.rafSeen) {
      this.watchdog = setTimeout(this.check, 1000);
    } else {
      this.runFrame(performance.now());
      this.watchdog = setTimeout(this.check, this.interval);
    }
    this.rafSeen = false;
  };

  constructor(private readonly frame: FrameCallback) {}

  setFps(fps: number): void {
    const value = fps === 15 || fps === 30 || fps === 60 ? fps : 30;
    if (1000 / value === this.interval) return;
    this.interval = 1000 / value;
    // A sleep planned for the old rate ends now.
    if (this.sleeper !== null) {
      clearTimeout(this.sleeper);
      this.sleeper = null;
      this.chained = null;
      this.handle = requestAnimationFrame(this.tick);
    }
  }

  get fps(): number {
    return Math.round(1000 / this.interval);
  }

  get running(): boolean {
    return this.active;
  }

  /** Estimated display refresh interval in ms (median of recent consecutive rAF timestamps). */
  get vsync(): number {
    return this.vsyncMs;
  }

  /** How long before a frame is due a sleep ends (the allowance for late timers). */
  get timerSlack(): number {
    return this.onTime >= PRECISE_RUN ? PRECISE_SLACK_MS : TIMER_SLACK_MS;
  }

  setActive(active: boolean): void {
    if (active === this.active) return;
    this.active = active;
    if (active) {
      this.last = -Infinity;
      this.rafSeen = true;
      this.chained = null;
      this.handle = requestAnimationFrame(this.tick);
      this.watchdog = setTimeout(this.check, 1000);
    } else {
      if (this.handle !== null) cancelAnimationFrame(this.handle);
      if (this.sleeper !== null) clearTimeout(this.sleeper);
      if (this.watchdog !== null) clearTimeout(this.watchdog);
      this.handle = null;
      this.sleeper = null;
      this.watchdog = null;
    }
  }

  /**
   * After a tick: sleep until shortly before the next frame is due when that
   * skips at least two vsyncs (one timer wake replaces them), otherwise ask
   * for the next animation frame. Exactly one of the two is ever pending.
   */
  private scheduleNext(frameTime: number): void {
    const wakeAt = this.last + this.interval - FRAME_TOLERANCE_MS - this.timerSlack;
    const sleep = wakeAt - performance.now();
    if (sleep >= 2 * this.vsyncMs) {
      this.chained = null;
      this.wakeAt = wakeAt;
      this.sleeper = setTimeout(this.wake, sleep);
    } else {
      this.chained = frameTime;
      this.handle = requestAnimationFrame(this.tick);
    }
  }

  /** One rAF-to-rAF interval; the median ignores dropped frames and stalls. */
  private sampleVsync(delta: number): void {
    if (!(delta >= 2 && delta <= 100)) return;
    this.samples[this.sampleNext] = delta;
    this.sampleNext = (this.sampleNext + 1) % VSYNC_SAMPLES;
    this.sampleCount = Math.min(this.sampleCount + 1, VSYNC_SAMPLES);
    const n = this.sampleCount;
    for (let i = 0; i < n; i++) {
      const value = this.samples[i];
      let j = i - 1;
      while (j >= 0 && this.sorted[j] > value) {
        this.sorted[j + 1] = this.sorted[j];
        j--;
      }
      this.sorted[j + 1] = value;
    }
    this.vsyncMs = this.sorted[n >> 1];
  }

  private runFrame(now: number): void {
    const elapsed = now - this.last;
    if (elapsed < this.interval - FRAME_TOLERANCE_MS) return;
    // Keep the cadence aligned to the target interval; resync after a stall.
    this.last = elapsed > this.interval * 2 ? now : this.last + this.interval;
    try {
      this.frame(now);
    } catch (error) {
      console.error(error);
    }
  }
}
