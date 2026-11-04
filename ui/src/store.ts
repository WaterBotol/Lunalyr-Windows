// Client-side state: the last complete value of every contract slice.
//
// Every event carries a whole slice (no diffs), so merging is replacement.
// The one subtlety is start-up: listeners are attached before `get_snapshot`
// is sent, and any slice that arrives by event while the request is in
// flight is newer than the snapshot's copy and must not be overwritten.

import type { Bridge, EventName, EventPayloads, Unlisten } from "./bridge";
import type { AudioFrame, LookupInfo, LyricsView, PlaybackClock, RenderState, Settings, Snapshot, TrackPayload } from "./contract";
import { COMMANDS, EVENTS } from "./contract";
import { emptySnapshot } from "./defaults";

export type Slice = "track" | "lyrics" | "clock" | "settings" | "render" | "lookup";
export const SLICES: readonly Slice[] = ["track", "lyrics", "clock", "settings", "render", "lookup"];

export interface AppState {
  track: TrackPayload;
  lyrics: LyricsView;
  clock: PlaybackClock;
  settings: Settings;
  render: RenderState;
  lookup: LookupInfo;
  version: string;
  /** True once a snapshot has been applied. */
  ready: boolean;
}

export type Change = Slice | "version" | "ready";
export type StoreListener = (changed: ReadonlySet<Change>, state: Readonly<AppState>) => void;

const EVENT_SLICE: Partial<Record<EventName, Slice>> = {
  [EVENTS.track]: "track",
  [EVENTS.lyrics]: "lyrics",
  [EVENTS.clock]: "clock",
  [EVENTS.settings]: "settings",
  [EVENTS.render]: "render",
  [EVENTS.lookup]: "lookup",
};

/** Slices whose identical re-sends are dropped (the clock always re-anchors). */
const DEDUPED: ReadonlySet<Slice> = new Set(["track", "lyrics", "settings", "render", "lookup"]);

export class Store {
  private current: AppState;
  private readonly listeners = new Set<StoreListener>();
  private inFlight: Set<Slice> | null = null;
  private readonly signatures = new Map<Slice, string>();

  constructor(initial: Snapshot = emptySnapshot()) {
    this.current = { ...initial, ready: false };
  }

  get state(): Readonly<AppState> {
    return this.current;
  }

  subscribe(listener: StoreListener): Unlisten {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** Call just before sending `get_snapshot`. */
  beginSnapshot(): void {
    this.inFlight = new Set();
  }

  /** The `get_snapshot` response: fills every slice not already refreshed by an event. */
  applySnapshot(snapshot: Snapshot): void {
    const skip = this.inFlight ?? new Set<Slice>();
    this.inFlight = null;
    const changed = new Set<Change>();
    const next: AppState = { ...this.current };
    for (const slice of SLICES) {
      if (skip.has(slice)) continue;
      if (this.assign(next, slice, snapshot[slice])) changed.add(slice);
    }
    if (next.version !== snapshot.version) {
      next.version = snapshot.version;
      changed.add("version");
    }
    if (!next.ready) {
      next.ready = true;
      changed.add("ready");
    }
    this.commit(next, changed);
  }

  applyEvent<E extends EventName>(event: E, payload: EventPayloads[E]): void {
    if (event === EVENTS.snapshot) {
      // A pushed snapshot is authoritative and newer than any request in flight.
      const snapshot = payload as Snapshot;
      if (this.inFlight) for (const slice of SLICES) this.inFlight.add(slice);
      const changed = new Set<Change>();
      const next: AppState = { ...this.current };
      for (const slice of SLICES) if (this.assign(next, slice, snapshot[slice])) changed.add(slice);
      if (next.version !== snapshot.version) {
        next.version = snapshot.version;
        changed.add("version");
      }
      if (!next.ready) {
        next.ready = true;
        changed.add("ready");
      }
      this.commit(next, changed);
      return;
    }
    const slice = EVENT_SLICE[event];
    if (!slice) return;
    this.inFlight?.add(slice);
    const next: AppState = { ...this.current };
    if (this.assign(next, slice, payload as AppState[Slice])) this.commit(next, new Set<Change>([slice]));
  }

  private assign(target: AppState, slice: Slice, value: AppState[Slice]): boolean {
    if (value === undefined || value === null) return false;
    if (DEDUPED.has(slice)) {
      const signature = JSON.stringify(value);
      if (this.signatures.get(slice) === signature && target[slice] !== undefined) return false;
      this.signatures.set(slice, signature);
    }
    (target as unknown as Record<Slice, unknown>)[slice] = value;
    return true;
  }

  private commit(next: AppState, changed: Set<Change>): void {
    if (changed.size === 0) return;
    this.current = next;
    for (const listener of [...this.listeners]) listener(changed, next);
  }
}

export interface ConnectOptions {
  /** Audio frames are not stored; they go straight to this callback. */
  onAudio?: (frame: AudioFrame) => void;
  /** Events to listen to; defaults to all of them. */
  events?: readonly EventName[];
}

/** Listen first, then request the snapshot, so nothing between the two is lost. */
export async function connectStore(bridge: Bridge, store: Store, options: ConnectOptions = {}): Promise<Unlisten> {
  const names = options.events ?? (Object.values(EVENTS) as EventName[]);
  const unlistens: Unlisten[] = [];
  for (const name of names) {
    if (name === EVENTS.audio) {
      if (options.onAudio) unlistens.push(await bridge.listen(EVENTS.audio, options.onAudio));
      continue;
    }
    unlistens.push(await bridge.listen(name, (payload) => store.applyEvent(name, payload as never)));
  }
  store.beginSnapshot();
  try {
    const snapshot = await bridge.invoke<Snapshot>(COMMANDS.getSnapshot);
    store.applySnapshot(snapshot);
  } catch (error) {
    console.error("get_snapshot failed", error);
    store.applySnapshot({ ...emptySnapshot(), ...store.state });
  }
  return () => unlistens.forEach((u) => u());
}
