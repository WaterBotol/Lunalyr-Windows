// What the settings window says about the lookup, the pause and failed
// commands. Pure, so it is tested without a DOM.

import type { RenderState } from "../contract";
import type { AppState } from "../store";

export interface LookupStatus {
  /** A lookup is running for the current track: show progress and hold Reload. */
  searching: boolean;
  /** Shown under the source and timing, or null. */
  note: string | null;
  /** Placeholder for an empty list of matches. */
  emptyMatches: string;
}

/**
 * Progress comes from `LookupInfo.searching`, not from the lyrics: a refresh
 * keeps the current lyrics on screen while it runs.
 */
export function lookupStatus(state: Pick<AppState, "track" | "lyrics" | "lookup">): LookupStatus {
  const playing = state.track.track !== null;
  const searching = playing && state.lookup.searching === true;
  const note = searching ? "Looking for lyrics…" : playing && state.lyrics.status === "notFound" ? "No lyrics found for this song." : null;
  return { searching, note, emptyMatches: searching ? "Searching…" : "No other matches." };
}

/** The "Pause the wallpaper now" switch: on only for the user's own pause. */
export function pausedByUser(render: RenderState): boolean {
  return render.paused && render.reason === "user";
}

/** A failed command as a sentence (Tauri rejects with the Rust error string). */
export function errorText(error: unknown): string {
  if (typeof error === "string" && error.trim()) return error;
  if (error instanceof Error && error.message) return error.message;
  if (error && typeof error === "object" && "message" in error && typeof error.message === "string" && error.message) return error.message;
  return "Something went wrong. Please try again.";
}
