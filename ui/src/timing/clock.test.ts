import { describe, expect, it } from "vitest";
import type { PlaybackClock } from "../contract";
import { ClockFollower, extrapolateClock, VisualClock } from "./clock";

const base: PlaybackClock = { position: 10, sampledAt: 1_000_000, playing: true, rate: 1, offset: 0 };

describe("clock extrapolation (docs/CONTRACT.md)", () => {
  it("adds elapsed wall time while playing, scaled by rate, plus the offset", () => {
    expect(extrapolateClock(base, 1_000_000)).toBe(10);
    expect(extrapolateClock(base, 1_002_500)).toBeCloseTo(12.5, 10);
    expect(extrapolateClock({ ...base, rate: 2 }, 1_001_000)).toBeCloseTo(12, 10);
    expect(extrapolateClock({ ...base, offset: 0.35 }, 1_001_000)).toBeCloseTo(11.35, 10);
    expect(extrapolateClock({ ...base, offset: -1 }, 1_000_000)).toBe(9);
  });

  it("holds the position while paused (offset still applies)", () => {
    const paused = { ...base, playing: false, offset: 0.2 };
    expect(extrapolateClock(paused, 1_000_000)).toBeCloseTo(10.2, 10);
    expect(extrapolateClock(paused, 9_000_000)).toBeCloseTo(10.2, 10);
  });

  it("never returns a non-finite time", () => {
    expect(extrapolateClock({ ...base, rate: Number.NaN }, 1_001_000)).toBeCloseTo(11, 10);
    expect(extrapolateClock({ ...base, offset: Number.NaN }, 1_000_000)).toBe(10);
    expect(extrapolateClock({ ...base, position: Number.NaN }, 1_000_000)).toBe(0);
  });
});

describe("ClockFollower", () => {
  it("equals the contract time without corrections", () => {
    const follower = new ClockFollower();
    follower.set(base, 1_000_000);
    for (const now of [1_000_000, 1_000_016, 1_003_000]) expect(follower.time(now)).toBe(extrapolateClock(base, now));
  });

  it("glides a small re-sync and never runs backwards", () => {
    const follower = new ClockFollower();
    follower.set(base, 1_000_000);
    // At +1 s a re-sync says playback is 120 ms behind what we extrapolated.
    const resync = { ...base, position: 10.88, sampledAt: 1_001_000 };
    const before = follower.time(1_001_000);
    follower.set(resync, 1_001_000);
    expect(follower.time(1_001_000)).toBeCloseTo(before, 10);
    let previous = follower.time(1_001_000);
    for (let ms = 1; ms <= 1000; ms += 1) {
      const value = follower.time(1_001_000 + ms);
      expect(value).toBeGreaterThan(previous);
      previous = value;
    }
    // Converged on the contract time.
    expect(follower.time(1_002_000)).toBeCloseTo(extrapolateClock(resync, 1_002_000), 10);
  });

  it("applies seeks, pauses and rate changes immediately", () => {
    const follower = new ClockFollower();
    follower.set(base, 1_000_000);
    const seek = { ...base, position: 42, sampledAt: 1_000_500 };
    follower.set(seek, 1_000_500);
    expect(follower.time(1_000_500)).toBe(42);
    const paused = { ...seek, playing: false, position: 42.1 };
    follower.set(paused, 1_000_600);
    expect(follower.time(1_000_600)).toBeCloseTo(42.1, 10);
    follower.set({ ...paused, playing: true, sampledAt: 1_000_700 }, 1_000_700);
    expect(follower.time(1_000_700)).toBeCloseTo(42.1, 10);
  });
});

describe("VisualClock", () => {
  it("integrates rate so a speed change never jumps the phase", () => {
    const clock = new VisualClock();
    expect(clock.advance(100, 1)).toBe(0);
    let phase = 0;
    for (let i = 1; i <= 60; i++) phase = clock.advance(100 + i / 60, 1);
    expect(phase).toBeCloseTo(1, 6);
    const before = phase;
    const after = clock.advance(100 + 61 / 60, 4);
    // Rate eases in (0.65 s time constant): one frame moves far less than 4/60.
    expect(after - before).toBeGreaterThan(1 / 60);
    expect(after - before).toBeLessThan(4 / 60);
  });

  it("ignores repeated timestamps and clamps long gaps", () => {
    const clock = new VisualClock();
    clock.advance(0, 1);
    const a = clock.advance(0.1, 1);
    expect(clock.advance(0.1, 1)).toBe(a);
    expect(clock.advance(0.05, 1)).toBe(a);
    const b = clock.advance(3600, 1);
    expect(b - a).toBeCloseTo(0.25, 10);
  });
});
