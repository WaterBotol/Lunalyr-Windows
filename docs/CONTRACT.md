# Rust ↔ renderer contract

The Rust side (Tauri app + `lunalyr-core`) owns all state: now playing, the
lyric lookup, settings and the platform layer. Webviews only render and send
user actions. The shapes are defined three times and must stay in step:

| Rust | TypeScript |
|---|---|
| `crates/lunalyr-core/src/model.rs` (Word, Line, Provenance, SourceId) | `ui/src/contract.ts` |
| `crates/lunalyr-core/src/contract.rs` (events, Track, PlaybackClock, LyricsView, …) | `ui/src/contract.ts` |
| `crates/lunalyr-core/src/settings.rs` (Settings) | `ui/src/contract.ts` |

JSON is camelCase. Times on the track timeline are **seconds**; wall-clock
instants are **Unix epoch milliseconds** (compare with `Date.now()`).

## Windows

* `wallpaper-<n>` — one per monitor, loads `wallpaper.html`, parented behind
  the desktop icons by the platform layer. Never focused, no input.
* `settings` — normal window, loads `settings.html`.
* `demo.html` — browser-only harness (no Tauri) used to develop and verify the
  renderer on any OS. It feeds the same contract objects from sample data.

## Startup

A page calls `get_snapshot` on load, then listens to events. Every event
carries a complete value for its slice of state (no diffs), so a missed event
is repaired by the next one.

## Events (Rust → webviews)

| Event | Payload | When |
|---|---|---|
| `lunalyr://track` | `TrackPayload` | track or artwork palette changed |
| `lunalyr://lyrics` | `LyricsView` | lookup started / finished / readings changed |
| `lunalyr://clock` | `PlaybackClock` | GSMTC timeline/playback change, offset change, periodic re-sync (≤ 5 s) |
| `lunalyr://settings` | `Settings` | any settings change |
| `lunalyr://render` | `RenderState` | per wallpaper window (`emit_to`): covered / user pause / session lock |
| `lunalyr://lookup` | `LookupInfo` | candidates and per-source states (settings window) |
| `lunalyr://audio` | `AudioFrame` | ~30–60 Hz, only while the visualiser background is on |

## Clock

```
t = clock.position
  + (clock.playing ? (Date.now() - clock.sampledAt) / 1000 * clock.rate : 0)
  + clock.offset
```

`offset` already includes the global lyric offset, the per-song offset and the
platform sync lead. Renderers must not add their own.

## Commands (webviews → Rust)

See `COMMANDS` in `ui/src/contract.ts`: `get_snapshot`, `update_settings`,
`reload_lyrics`, `select_candidate`, `forget_selection`, `set_song_offset`,
`import_lyrics`, `open_settings`, `set_user_paused`.
