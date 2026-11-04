// Playback clock extrapolation (docs/CONTRACT.md) and the ambient animation
// clock (Sources/UI/VisualClock.swift).

import type { PlaybackClock } from "../contract";
import { smoothstep } from "./math";

/**
 * The contract's track time:
 *   t = position + (playing ? (now - sampledAt) / 1000 * rate : 0) + offset
 * `offset` already contains the global offset, the song offset and the
 * platform sync lead; renderers add nothing of their own.
 */
export function extrapolateClock(clock: PlaybackClock, nowMs: number): number {
  const rate = Number.isFinite(clock.rate) ? clock.rate : 1;
  const elapsed = clock.playing ? ((nowMs - clock.sampledAt) / 1000) * rate : 0;
  const t = clock.position + (Number.isFinite(elapsed) ? elapsed : 0) + (Number.isFinite(clock.offset) ? clock.offset : 0);
  return Number.isFinite(t) ? t : 0;
}

/**
 * Follows the contract time, but absorbs a small re-sync (a periodic clock
 * event that disagrees with the running extrapolation by a few tens of
 * milliseconds) over a short glide instead of jumping. Seeks, pauses, rate
 * changes and any correction over `jumpThreshold` apply at once. Once the
 * glide ends the value is exactly the contract time again.
 */
export class ClockFollower {
  private clock: PlaybackClock | null = null;
  private correction = 0;
  private correctionAt = 0;
  private correctionDuration = 0;

  constructor(readonly jumpThreshold = 0.35) {}

  get current(): PlaybackClock | null {
    return this.clock;
  }

  set(next: PlaybackClock, nowMs: number): void {
    const previous = this.clock;
    if (previous && previous.playing && next.playing && previous.rate === next.rate) {
      const shown = this.time(nowMs);
      const target = extrapolateClock(next, nowMs);
      const delta = shown - target;
      if (Math.abs(delta) > 1e-6 && Math.abs(delta) < this.jumpThreshold) {
        this.correction = delta;
        this.correctionAt = nowMs;
        // Long enough that time never runs backwards (max slope 1.5 * delta / duration).
        this.correctionDuration = Math.max(250, Math.abs(delta) * 4000);
      } else {
        this.correction = 0;
      }
    } else {
      this.correction = 0;
    }
    this.clock = next;
  }

  time(nowMs: number): number {
    if (!this.clock) return 0;
    const base = extrapolateClock(this.clock, nowMs);
    if (this.correction === 0) return base;
    const progress = (nowMs - this.correctionAt) / this.correctionDuration;
    if (progress >= 1) {
      this.correction = 0;
      return base;
    }
    return base + this.correction * (1 - smoothstep(Math.max(0, progress)));
  }
}

/**
 * Accumulates animation phase from a rate that changes over time
 * (`phase += dt * rate`), easing the rate itself with a 0.65 s time constant
 * and clamping long gaps (display sleep, a paused wallpaper) to 0.25 s.
 */
export class VisualClock {
  private phase = 0;
  private lastTime: number | null = null;
  private smoothedRate: number | null = null;
  static readonly rateTimeConstant = 0.65;

  constructor(initialPhase = 0) {
    this.phase = initialPhase;
  }

  /** `time` in seconds (any monotonic origin). */
  advance(time: number, rate: number): number {
    const target = rate;
    if (this.lastTime === null) {
      this.lastTime = time;
      this.smoothedRate = target;
      return this.phase;
    }
    if (!(time > this.lastTime)) return this.phase;
    const dt = Math.min(time - this.lastTime, 0.25);
    this.lastTime = time;
    const current = this.smoothedRate ?? target;
    const blend = 1 - Math.exp(-dt / VisualClock.rateTimeConstant);
    const eased = current + (target - current) * blend;
    this.smoothedRate = eased;
    this.phase += dt * eased;
    return this.phase;
  }

  get value(): number {
    return this.phase;
  }
}
