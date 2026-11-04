import { describe, expect, it } from "vitest";
import type { Bridge, EventName, EventPayloads } from "./bridge";
import type { PlaybackClock, Snapshot } from "./contract";
import { COMMANDS, EVENTS } from "./contract";
import { emptySnapshot } from "./defaults";
import { connectStore, Store, type Change } from "./store";

function snapshot(overrides: Partial<Snapshot> = {}): Snapshot {
  return { ...emptySnapshot(), version: "1.0", ...overrides };
}

const clockAt = (position: number): PlaybackClock => ({ position, sampledAt: 1000, playing: true, rate: 1, offset: 0 });

describe("Store event merging", () => {
  it("replaces whole slices and reports what changed", () => {
    const store = new Store();
    const seen: Change[][] = [];
    store.subscribe((changed) => seen.push([...changed]));
    store.applySnapshot(snapshot());
    store.applyEvent(EVENTS.clock, clockAt(5));
    store.applyEvent(EVENTS.lyrics, { status: "loading" });
    expect(store.state.clock.position).toBe(5);
    expect(store.state.lyrics.status).toBe("loading");
    expect(store.state.ready).toBe(true);
    expect(seen.at(-2)).toEqual(["clock"]);
    expect(seen.at(-1)).toEqual(["lyrics"]);
  });

  it("drops identical re-sends except for the clock", () => {
    const store = new Store();
    store.applySnapshot(snapshot());
    let calls = 0;
    store.subscribe(() => calls++);
    store.applyEvent(EVENTS.lyrics, { status: "loading" });
    store.applyEvent(EVENTS.lyrics, { status: "loading" });
    expect(calls).toBe(1);
    store.applyEvent(EVENTS.clock, clockAt(1));
    store.applyEvent(EVENTS.clock, clockAt(1));
    expect(calls).toBe(3);
  });

  it("keeps slices that arrived by event while the snapshot was in flight", () => {
    const store = new Store();
    store.beginSnapshot();
    store.applyEvent(EVENTS.clock, clockAt(42));
    store.applyEvent(EVENTS.lyrics, { status: "notFound", reportedAt: 99 });
    store.applySnapshot(snapshot({ clock: clockAt(1), lyrics: { status: "loading" }, render: { paused: true, reason: "user" } }));
    expect(store.state.clock.position).toBe(42);
    expect(store.state.lyrics).toEqual({ status: "notFound", reportedAt: 99 });
    expect(store.state.render.paused).toBe(true);
    expect(store.state.version).toBe("1.0");
    // After the snapshot, events apply normally again.
    store.applyEvent(EVENTS.clock, clockAt(50));
    expect(store.state.clock.position).toBe(50);
  });

  it("a pushed snapshot is authoritative over an older in-flight response", () => {
    const store = new Store();
    store.beginSnapshot();
    store.applyEvent(EVENTS.snapshot, snapshot({ clock: clockAt(7), version: "2.0" }));
    store.applySnapshot(snapshot({ clock: clockAt(1) }));
    expect(store.state.clock.position).toBe(7);
    expect(store.state.ready).toBe(true);
  });
});

class FakeBridge implements Bridge {
  readonly kind = "demo" as const;
  readonly handlers = new Map<string, ((payload: unknown) => void)[]>();
  resolveSnapshot: ((s: Snapshot) => void) | null = null;
  invoke<T>(command: string): Promise<T> {
    expect(command).toBe(COMMANDS.getSnapshot);
    return new Promise<T>((resolve) => {
      this.resolveSnapshot = (s) => resolve(s as T);
    });
  }
  async listen<E extends EventName>(event: E, handler: (payload: EventPayloads[E]) => void) {
    const list = this.handlers.get(event) ?? [];
    list.push(handler as (payload: unknown) => void);
    this.handlers.set(event, list);
    return () => {
      this.handlers.set(event, (this.handlers.get(event) ?? []).filter((h) => h !== handler));
    };
  }
  emit(event: EventName, payload: unknown) {
    for (const h of this.handlers.get(event) ?? []) h(payload);
  }
}

describe("connectStore", () => {
  it("listens before requesting the snapshot and merges in order", async () => {
    const bridge = new FakeBridge();
    const store = new Store();
    const audio: number[] = [];
    const connecting = connectStore(bridge, store, { onAudio: (f) => audio.push(f.level) });
    await new Promise((r) => setTimeout(r, 0));
    expect(bridge.handlers.get(EVENTS.clock)?.length).toBe(1);
    bridge.emit(EVENTS.clock, clockAt(30));
    bridge.emit(EVENTS.audio, { level: 0.5, bass: 0, mid: 0, treble: 0, beat: 0, flux: 0, centroid: 0, bands: [], sampleRate: 48000 });
    bridge.resolveSnapshot!(snapshot({ clock: clockAt(3) }));
    const unlisten = await connecting;
    expect(store.state.clock.position).toBe(30);
    expect(store.state.ready).toBe(true);
    expect(audio).toEqual([0.5]);
    unlisten();
    expect(bridge.handlers.get(EVENTS.clock)?.length).toBe(0);
  });
});
