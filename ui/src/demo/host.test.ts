import { afterEach, describe, expect, it } from "vitest";
import type { LyricsView, PlaybackClock, Settings } from "../contract";
import { COMMANDS, EVENTS } from "../contract";
import { songOffsetKey } from "../sha256";
import { Store, connectStore } from "../store";
import { DemoHost } from "./host";
import { parseLrc, precisionOf } from "./lrc";
import { distributeWords, SONGS, wordTokens } from "./samples";

const hosts: DemoHost[] = [];
function host(options: ConstructorParameters<typeof DemoHost>[0] = {}): DemoHost {
  const h = new DemoHost({ share: false, ...options });
  hosts.push(h);
  return h;
}
afterEach(() => {
  while (hosts.length) hosts.pop()!.dispose();
});

describe("demo host (in-memory contract feed)", () => {
  it("answers get_snapshot and merges update_settings patches", async () => {
    const h = host({ scenario: "word", paused: true, time: 3 });
    const snapshot = await h.invoke<{ settings: Settings; clock: PlaybackClock }>(COMMANDS.getSnapshot);
    expect(snapshot.clock.position).toBe(3);
    const seen: Settings[] = [];
    await h.listen(EVENTS.settings, (s) => seen.push(s));
    const result = await h.invoke<Settings>(COMMANDS.updateSettings, { patch: { lyricScale: 1.4, composition: "spoken" } });
    expect(result.lyricScale).toBe(1.4);
    expect(result.composition).toBe("spoken");
    expect(result.frameRate).toBe(30);
    expect(seen.at(-1)?.lyricScale).toBe(1.4);
  });

  it("song and global offsets reach the clock offset", async () => {
    const h = host({ scenario: "word", paused: true });
    const clocks: PlaybackClock[] = [];
    await h.listen(EVENTS.clock, (c) => clocks.push(c));
    await h.invoke(COMMANDS.setSongOffset, { seconds: 0.3 });
    const key = songOffsetKey(h.current.track.track!.recordingKey);
    expect(h.current.settings.songOffsets[key]).toBe(0.3);
    expect(clocks.at(-1)?.offset).toBeCloseTo(0.3, 10);
    await h.invoke(COMMANDS.updateSettings, { patch: { lyricOffset: -0.1 } });
    expect(clocks.at(-1)?.offset).toBeCloseTo(0.2, 10);
    await h.invoke(COMMANDS.setSongOffset, { seconds: 0 });
    expect(h.current.settings.songOffsets[key]).toBeUndefined();
  });

  it("selecting a candidate swaps the lyrics and pins the choice", async () => {
    const h = host({ scenario: "word", paused: true });
    const lyrics: LyricsView[] = [];
    await h.listen(EVENTS.lyrics, (l) => lyrics.push(l));
    await h.invoke(COMMANDS.selectCandidate, { source: "lrclib" });
    const view = lyrics.at(-1)!;
    expect(view.status).toBe("synced");
    if (view.status === "synced") {
      expect(view.provenance.precision).toBe("Line-synced");
      expect(view.lines.every((l) => !l.wordTimed)).toBe(true);
    }
    expect(h.current.lookup.pinned).toBe(true);
    expect(h.current.lookup.candidates.find((c) => c.chosen)?.source).toBe("lrclib");
  });

  it("readings are only sent while enabled, without tone marks when asked", async () => {
    const h = host({ scenario: "word", paused: true });
    expect(h.current.lyrics.status === "synced" && h.current.lyrics.readings.length).toBe(0);
    await h.invoke(COMMANDS.updateSettings, { patch: { readings: "below" } });
    const lyrics = h.current.lyrics;
    expect(lyrics.status).toBe("synced");
    if (lyrics.status !== "synced") return;
    const japanese = lyrics.readings.findIndex((r) => r?.[0]?.text === "yozora");
    expect(japanese).toBeGreaterThan(-1);
    await h.invoke(COMMANDS.updateSettings, { patch: { toneMarks: false } });
    const after = h.current.lyrics;
    if (after.status !== "synced") return;
    expect(after.readings.flat().map((w) => w?.text)).toContain("wan'an");
  });

  it("imports LRC through the same command the app uses", async () => {
    const h = host({ scenario: "word", paused: true });
    const message = await h.invoke<string>(COMMANDS.importLyrics, { text: "[00:01.00]<00:01.00>Hello <00:01.50>world\n[00:03.00]Plain line" });
    expect(message).toMatch(/Imported 2 lines/);
    const lyrics = h.current.lyrics;
    expect(lyrics.status === "synced" && lyrics.provenance.precision).toBe("Mixed timing");
  });

  it("feeds a Store through connectStore", async () => {
    const h = host({ scenario: "line", time: 4, paused: true });
    const store = new Store();
    await connectStore(h, store);
    expect(store.state.ready).toBe(true);
    expect(store.state.track.track?.title).toBe("Northbound");
    h.seek(9);
    expect(store.state.clock.position).toBe(9);
    h.setUserPaused(true);
    expect(store.state.render).toEqual({ paused: true, reason: "user" });
  });
});

describe("demo lyrics helpers", () => {
  it("parses LRC with offsets, repeated stamps and word stamps", () => {
    const lines = parseLrc("[offset:+500]\n[00:02.00][00:10.00]Again\n[00:05.00]<00:05.00>Word <00:05.40>by <00:05.70>word", 20);
    expect(lines.map((l) => l.start)).toEqual([1.5, 4.5, 9.5]);
    expect(lines[1].wordTimed).toBe(true);
    expect(lines[1].words.map((w) => w.text)).toEqual(["Word", "by", "word"]);
    expect(lines[1].words[0].end).toBeCloseTo(4.9, 10);
    expect(precisionOf(lines)).toBe("Mixed timing");
  });

  it("splits words like the core and distributes a line window", () => {
    expect(wordTokens("“Hello,” she said — again!")).toEqual(["“Hello,”", "she", "said —", "again!"]);
    const words = distributeWords("one two three", 10, 13);
    expect(words.length).toBe(3);
    expect(words[0].start).toBe(10);
    expect(words[2].end).toBeLessThanOrEqual(13);
  });

  it("ships only word-timed lines in the word-timed sample", () => {
    expect(SONGS.word.lines.every((l) => l.wordTimed)).toBe(true);
    expect(SONGS.line.lines.every((l) => !l.wordTimed && l.words.length > 0)).toBe(true);
  });
});
