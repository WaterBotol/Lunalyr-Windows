import { describe, expect, it } from "vitest";
import type { LookupInfo, LyricsView, Track } from "../contract";
import { emptyLookup, emptyTrack } from "../defaults";
import { errorText, lookupStatus, pausedByUser } from "./status";

const track: Track = { title: "Song", artist: "Artist", album: null, duration: 200, sourceApp: "Player.exe", recordingKey: "song|artist|200", seed: 1 };
const synced: LyricsView = { status: "synced", lines: [], provenance: { precision: "Line-synced", source: "LRCLIB" }, readings: [] };

function state(lyrics: LyricsView, lookup: Partial<LookupInfo> = {}, playing = true) {
  return { track: { ...emptyTrack(), track: playing ? track : null }, lyrics, lookup: { ...emptyLookup(), ...lookup } };
}

describe("settings lookup status", () => {
  it("shows progress from lookup.searching while a refresh keeps the lyrics", () => {
    expect(lookupStatus(state(synced, { searching: true }))).toEqual({ searching: true, note: "Looking for lyrics…", emptyMatches: "Searching…" });
    expect(lookupStatus(state(synced))).toEqual({ searching: false, note: null, emptyMatches: "No other matches." });
  });

  it("does not infer progress from loading lyrics", () => {
    expect(lookupStatus(state({ status: "loading" })).searching).toBe(false);
    expect(lookupStatus(state({ status: "loading" }, { searching: true })).note).toBe("Looking for lyrics…");
  });

  it("says when nothing was found, once the search is over", () => {
    const notFound: LyricsView = { status: "notFound", reportedAt: 1 };
    expect(lookupStatus(state(notFound)).note).toBe("No lyrics found for this song.");
    expect(lookupStatus(state(notFound, { searching: true })).note).toBe("Looking for lyrics…");
  });

  it("shows nothing without a track", () => {
    expect(lookupStatus(state({ status: "idle" }, { searching: true }, false))).toEqual({ searching: false, note: null, emptyMatches: "No other matches." });
  });
});

describe("settings pause switch", () => {
  it("is on only for the user's own pause", () => {
    expect(pausedByUser({ paused: true, reason: "user" })).toBe(true);
    expect(pausedByUser({ paused: true, reason: "session-locked" })).toBe(false);
    expect(pausedByUser({ paused: true, reason: "covered" })).toBe(false);
    expect(pausedByUser({ paused: false, reason: "" })).toBe(false);
  });
});

describe("command errors", () => {
  it("reads the Rust error string, an Error or an error-like object", () => {
    expect(errorText("This file has no usable LRC or TTML timestamps.")).toBe("This file has no usable LRC or TTML timestamps.");
    expect(errorText(new Error("Nothing is playing."))).toBe("Nothing is playing.");
    expect(errorText({ message: "IPC failed" })).toBe("IPC failed");
    expect(errorText(undefined)).toMatch(/went wrong/);
    expect(errorText("")).toMatch(/went wrong/);
  });
});
