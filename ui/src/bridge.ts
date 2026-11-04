// The only module that knows about Tauri. Inside the app it forwards to
// `@tauri-apps/api` (invoke / listen); in a plain browser the caller supplies
// an in-memory host (ui/src/demo/host.ts) with the same interface.

import type { AudioFrame, LookupInfo, LyricsView, PlaybackClock, RenderState, Settings, Snapshot, TrackPayload } from "./contract";
import { EVENTS } from "./contract";

export interface EventPayloads {
  [EVENTS.snapshot]: Snapshot;
  [EVENTS.track]: TrackPayload;
  [EVENTS.lyrics]: LyricsView;
  [EVENTS.clock]: PlaybackClock;
  [EVENTS.settings]: Settings;
  [EVENTS.render]: RenderState;
  [EVENTS.lookup]: LookupInfo;
  [EVENTS.audio]: AudioFrame;
}

export type EventName = keyof EventPayloads;
export type Unlisten = () => void;

export interface Bridge {
  readonly kind: "tauri" | "demo";
  invoke<T = unknown>(command: string, args?: Record<string, unknown>): Promise<T>;
  listen<E extends EventName>(event: E, handler: (payload: EventPayloads[E]) => void): Promise<Unlisten>;
}

export function insideTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function tauriBridge(): Promise<Bridge> {
  const [core, events, windows] = await Promise.all([
    import("@tauri-apps/api/core"),
    import("@tauri-apps/api/event"),
    import("@tauri-apps/api/webviewWindow"),
  ]);
  // Listen as THIS window. A plain `listen()` registers an "Any" target, and
  // Tauri delivers every event to Any listeners, including `emit_to` aimed
  // at another window (one monitor's pause would pause them all) and the
  // wallpaper-only audio frames. Window-scoped listeners still receive
  // app-wide `emit` broadcasts.
  const label = windows.getCurrentWebviewWindow().label;
  return {
    kind: "tauri",
    invoke: <T>(command: string, args?: Record<string, unknown>) => core.invoke<T>(command, args),
    listen: async (event, handler) => {
      const unlisten = await events.listen(event, (message) => handler(message.payload as never), {
        target: { kind: "WebviewWindow", label },
      });
      return unlisten;
    },
  };
}

/** Tauri when running inside the app, otherwise whatever `fallback` builds (the demo feed). */
export async function createBridge(fallback: () => Bridge | Promise<Bridge>): Promise<Bridge> {
  return insideTauri() ? tauriBridge() : fallback();
}
