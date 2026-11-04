// The frame loop: requestAnimationFrame throttled to the chosen frame rate
// (15 / 30 / 60), fully stopped — no rAF, no timers — while inactive (the
// wallpaper is covered, the user paused it, the session is locked or the
// page is hidden). Resuming renders on the very next animation frame.

export type FrameCallback = (now: number) => void;

export class FrameLoop {
  private interval = 1000 / 30;
  private last = -Infinity;
  private handle: number | null = null;
  private watchdog: ReturnType<typeof setTimeout> | null = null;
  private rafSeen = false;
  private active = false;
  private readonly tick = (now: number) => {
    this.handle = null;
    this.rafSeen = true;
    if (!this.active) return;
    this.runFrame(now);
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
    this.interval = 1000 / value;
  }

  get fps(): number {
    return Math.round(1000 / this.interval);
  }

  get running(): boolean {
    return this.active;
  }

  setActive(active: boolean): void {
    if (active === this.active) return;
    this.active = active;
    if (active) {
      this.last = -Infinity;
      this.rafSeen = true;
      this.handle = requestAnimationFrame(this.tick);
      this.watchdog = setTimeout(this.check, 1000);
    } else {
      if (this.handle !== null) cancelAnimationFrame(this.handle);
      if (this.watchdog !== null) clearTimeout(this.watchdog);
      this.handle = null;
      this.watchdog = null;
    }
  }

  private runFrame(now: number): void {
    const elapsed = now - this.last;
    if (elapsed < this.interval - 1) return;
    // Keep the cadence aligned to the target interval; resync after a stall.
    this.last = elapsed > this.interval * 2 ? now : this.last + this.interval;
    try {
      this.frame(now);
    } catch (error) {
      console.error(error);
    }
  }
}
