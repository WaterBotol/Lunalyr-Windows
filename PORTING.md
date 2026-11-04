# Porting Lunalyr to Windows

Lunalyr for macOS is a Swift/SwiftUI/AppKit/Metal app. None of its UI, rendering, now-playing or audio-capture layers exist on Windows, so this repository is a **port**: the design and the portable logic are carried over and re-implemented; every platform layer is rebuilt on Windows APIs. The macOS repository (`WaterBotol/Lunalyr`, commit `82bf584`) was the read-only reference.

## 1. Architecture decision

**Chosen: Tauri 2 — a Rust process (state, lyric lookup, Windows APIs) with the lyric renderer in TypeScript running in WebView2, one webview per monitor.**

The alternative was C#/.NET 8 with WinUI 3 or WPF, Win2D/Direct2D for lyrics and WebView2 for the web visualisers. A short spike compared them on the three criteria in the brief:

| Criterion | Tauri 2 + WebView2 (chosen) | .NET 8 + WinUI 3/WPF + Win2D |
|---|---|---|
| **Text quality** | Chromium on Windows rasterises glyphs through **DirectWrite** (Skia's DirectWrite font backend), shapes with HarfBuzz and uses system font fallback, so CJK, RTL and colour fonts work. Transform/opacity animations are composited on the GPU at device pixel ratio. | Win2D/DirectWrite gives the same glyph quality, with finer control (e.g. per-glyph geometry). WPF's own text stack is weaker under animated transforms. |
| **CPU/GPU cost of a full-screen animated wallpaper** | Higher fixed memory: WebView2 runs browser, GPU and renderer processes (roughly 100–200 MB shared across all monitors). Per-frame cost depends on the renderer: compositor-only properties, 30 fps by default, a frame loop that stops completely when covered/locked/paused. | Lower fixed memory (tens of MB) and direct control of the swap chain. But the bundled **web visualisers need WebView2 anyway**, so a visualiser wallpaper pays the browser cost in both designs; WPF also cannot draw lyrics over a WebView2 HWND (the "airspace" problem). |
| **Build and test from a Mac without Windows** | Verified in the spike: `cargo check --target x86_64-pc-windows-msvc` of the whole app (Tauri, WebView2 bindings, `windows` crate) takes ~8 s on this Mac once `llvm-rc` is on PATH. The core crate and the app's controller run their unit tests natively on macOS; the renderer runs in any browser through `demo.html`, so typography and motion can be **seen and screenshot on the Mac**. The app shell itself runs on macOS with a development stand-in platform layer. | WinUI 3's XAML compiler is a Windows executable, so it cannot build on macOS at all. WPF builds with `EnableWindowsTargeting` but cannot run; Win2D cannot run off Windows. Nothing visual could be checked before CI. (.NET was also not installed here; its macOS installer needs an admin password.) |

The deciding factor is the third row combined with the visualisers: with Tauri, everything except the thin Windows API layer can be built, run and looked at on the Mac, and lyrics and web visualisers share one compositor. The memory overhead is the accepted cost; it is mitigated by sharing one WebView2 environment across all windows, a 30 fps default and hard pausing.

### Process layout

```
lunalyr.exe (Rust, Tauri 2)
├─ platform/windows      GSMTC now playing · Progman/WorkerW attach · desktop watcher
│                        (displays, Explorer restarts, full-screen coverage, lock) ·
│                        WASAPI loopback · autostart · Japanese readings
├─ controller            state machine: track → lookup → lyrics; clock; settings; pausing
├─ lunalyr-core (crate)  model · LRC/YRC/Lyricsfile/TTML parsing · ranking · providers ·
│                        cache & saved choices · palette · settings · romanisation
└─ events / commands (docs/CONTRACT.md)
     ├─ wallpaper-<display>  WebView2, wallpaper.html — background + lyric renderer (TS)
     └─ settings             WebView2, settings.html
```

State lives only in Rust. Webviews render and send user actions; every event carries a full value for its slice of state, so a reloaded or re-attached webview recovers from `get_snapshot`. Lyric timing is extrapolated per frame in the renderer from a clock anchor (`position`, `sampledAt`, `playing`, `rate`, `offset`), the same model as the macOS `NowPlayingMonitor`.

## 2. Platform mapping

| macOS (Lunalyr) | Windows (this port) | Notes |
|---|---|---|
| AppleScript to Music + `com.apple.Music.playerInfo` notifications, 2 s re-anchor | **GSMTC** (`GlobalSystemMediaTransportControlsSessionManager`): media properties, timeline (`Position` at `LastUpdatedTime`), playback info, thumbnail | Works for Spotify, Apple Music for Windows, browsers and any app using SMTC; no automation permission. Some players only update the timeline on seek, so it is re-sampled every few seconds. |
| Desktop-level `NSWindow` per display | **Progman/WorkerW**: send `0x052C` to Progman; legacy layout parents the window into the WorkerW behind `SHELLDLL_DefView`; Windows 11 24H2+ "raised desktop" (Progman has `WS_EX_NOREDIRECTIONBITMAP`) makes it a `WS_CHILD` + `WS_EX_LAYERED` child of Progman z-ordered directly below `SHELLDLL_DefView`, with WorkerW kept last | Algorithm cross-checked against Lively Wallpaper's `WinDesktopCore.cs` (which quotes Microsoft's guidance for the raised desktop). One window per monitor in physical pixels; the process is Per-Monitor-V2 DPI aware. |
| `NSScreen` change notifications | Hidden top-level window: `WM_DISPLAYCHANGE`, `WM_SETTINGCHANGE`, `WM_DPICHANGED` → re-attach; `TaskbarCreated` (+ Explorer PID check) and a destroy hook on our parent window (Progman, plus the WorkerW in the legacy layout) → recreate the wallpapers; a 2 s watchdog re-attaches anything that came loose (e.g. Explorer replacing the 24H2 WorkerW sibling) | Window creation is always posted to the event loop, never run inside a WebView2 callback (that deadlocks). |
| `WindowRenderActivity` (occlusion) | Coverage poll: a visible, non-cloaked, maximised or full-screen window on a monitor pauses that monitor's renderer; `SHQueryUserNotificationState` for exclusive D3D full-screen; session lock and displays off (`GUID_SESSION_DISPLAY_STATUS`) pause everything; audio capture stops whenever no wallpaper is rendering | Setting: *Pause when covered* (on by default on Windows). |
| Core Audio process tap / ScreenCaptureKit | **WASAPI loopback** of the default render endpoint, restarted on default-device change | Milestone 2. Analysis (FFT bands, flux, beat) ported from `AudioAnalyzer.swift` / `AudioAnalysis.c`. |
| Menu-bar extra | Notification-area (tray) icon and menu | |
| SwiftUI settings workspace | `settings.html` in a WebView2 window | |
| `UserDefaults` | `%APPDATA%\io.github.waterbotol.lunalyr.windows\settings.json` (atomic writes, defaults for missing fields) | |
| `~/Library/Application Support/Verso Custom/Lyrics` | `%LOCALAPPDATA%\io.github.waterbotol.lunalyr.windows\lyrics\` | Same cache/pin scheme (SHA-256 file names, 14-day expiry, 150-file cap). |
| `SMAppService` launch at login | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` | |
| Japanese: `CFStringTokenizer` readings | `Windows.Globalization.JapanesePhoneticAnalyzer` (built into Windows, no dictionary to ship) + kana→romaji in Rust | Milestone 2. Avoids bundling a 20–50 MB kuromoji/Sudachi dictionary. |
| Mandarin/Korean: ICU transliteration | Pinyin crate; hand-written Korean Revised Romanization | Milestone 2. |
| Metal scenes | Web visualiser engines (bundled HTML/WebGL) + a built-in canvas spectrum | Metal shaders are not ported. |
| Sparkle updates | Not ported yet (NSIS installer; updater is future work) | |

## 3. Reused vs rewritten

| Area | Status |
|---|---|
| Lyric providers (LRCLIB, NetEase; then QQ, Kugou, Kuwo, lyrics.ovh, Lyrics+/KPoe) | **Re-implemented in Rust** with the same requests, matching and scoring. |
| LRC (incl. word stamps, `[offset:]`), YRC, LRCLIB Lyricsfile, TTML parsing | **Re-implemented in Rust**, tests mirror the macOS suites. |
| Match ranking (`ranked`/`choose`, `vc_match_score`), timing tiers, provenance | **Re-implemented in Rust**, same thresholds. |
| Word timing and motion (`LyricMotion.c`, `ConstellationGeometry.c`, `WordPresentation.swift`) | **Ported to TypeScript** function by function, unit-tested against the macOS expectations. |
| Compositions and typography (Editorial, Spoken first) | **Re-implemented in TypeScript/CSS** from the SwiftUI views. |
| Album colour palette (`ColorExtractor`, `AlbumWallpaperPalette`, animated gradient) | Extraction in Rust; gradient in the renderer. |
| Per-song offsets, saved choices, imports, cache | Same behaviour; new storage locations. |
| Bundled web visualisers (`*Engine.html`) | **Reused as-is** (Milestone 2), with their MIT notices. |
| Swift UI, AppKit windows, Metal shaders, AppleScript, Core Audio taps, ScreenCaptureKit, Sparkle | **Not reused** — replaced by the Windows layers above. |

## 4. Risks

| Risk | Mitigation / status |
|---|---|
| **The WorkerW technique is undocumented** and changed in Windows 11 24H2. Explorer can recreate its windows at any time. | Both layouts implemented; Explorer restarts, WorkerW destruction and a periodic watchdog all trigger re-attach. CI smoke test records the window tree and a desktop screenshot on the runner's Windows build. Still needs a check on real Windows 10 and Windows 11 23H2/24H2 desktops with icons, multiple monitors and mixed DPI. |
| A Tauri (tao) window restyled to `WS_CHILD` and reparented may fight tao's own style handling, or WebView2 may not present into a layered child. | Lively and other WebView2 wallpaper apps reparent WebView2 hosts successfully. Needs visual confirmation on real hardware; the fallback is to host WebView2 in our own raw child window via wry. |
| GSMTC timelines are coarse for some players (e.g. updated only on seek/pause), which would drift word timing. Some players (notably some browsers) publish no timeline at all. | Microsoft defines `Position` as current as of `LastUpdatedTime`, so the renderer extrapolates from that instant while playing; the timeline is re-sampled every ≤ 3 s; 0.10 s sync lead; global and per-song offsets. Without a timeline, time counts from when the track was first seen. Needs listening tests per player (Spotify, Apple Music for Windows, Edge/Chrome, Firefox). |
| WebView2 memory overhead and battery cost on laptops. | 30 fps default, renderer stops entirely when covered/locked/paused, single WebView2 environment. Measure on hardware. |
| Word segmentation differs from macOS for CJK (UAX #29 vs dictionary). | Only affects estimated word timing of line-synced CJK lyrics (per character on Windows). Real word timing is unaffected. |
| Unofficial provider APIs (NetEase, QQ, Kugou, Kuwo) can change or block. | Same exposure as macOS; each provider fails independently with a visible state; LRCLIB is the documented API. |
| No code signing yet: SmartScreen will warn on the installer. | Signing needs a certificate; out of scope for this port. |
| Proxies: lyric requests use the Windows system proxy only in its common `host:port` form, read at start-up. Per-protocol WinINet values (e.g. SOCKS-only), PAC/WPAD scripts and proxies switched on later are not handled. | Behind such a proxy lookups fail with a visible per-source error. A proper fix reads the proxy per request from the platform layer. |
| Cross-compiling the full installer on a Mac (cargo-xwin) is possible but untested. | CI on `windows-latest` builds the installer instead. |

### Deliberate differences from macOS

- CJK line-synced lyrics distribute estimated word timing per character (Unicode word boundaries have no dictionary). Real word timing is unaffected.
- LRC files whose multi-timestamp rows would expand to more than twice the 1 MB file limit are rejected (macOS expands them without a bound). Real files are far below this.
- Kugou is queried through `mobileservice.kugou.com`: the macOS host's TLS certificate no longer matches its name.
- Readings: there is no system language recogniser; Han-only lines count as Japanese only when the song has kana, otherwise as Mandarin. Scripts without a transliteration (Cyrillic, Greek, Devanagari, Thai) get no reading.
- Japanese readings come from Windows' `JapanesePhoneticAnalyzer` instead of macOS's tokenizer.
- A refresh (reload, source change, corrected duration) keeps the current lyrics on screen until the new result arrives, as macOS does; only a new song shows the loading state.

## 5. Milestones

**Milestone 1 — MVP**
- Tray app with GSMTC now playing.
- LRCLIB + NetEase providers with LRC/YRC/Lyricsfile parsing and the macOS ranking rules.
- Wallpaper window behind the desktop icons on every monitor, showing synced, word-timed lyrics in the Editorial and Spoken compositions over an album-colour gradient.
- Persisted settings; settings window.
- Unit tests for providers, parsing and timing (Rust and TypeScript).
- `windows-latest` CI: tests, installer build, desktop smoke test.

**Milestone 2**
- WASAPI loopback driving a visualiser (built-in spectrum and the bundled Wavefield web engine).
- More compositions (Constellation + Editorial, Scattered, Echo, Cascade, Cutout) and providers (QQ, Kugou, Kuwo, lyrics.ovh, Lyrics+/KPoe, local files, TTML import).
- Romanisation: Japanese via the Windows phonetic analyser, pinyin, Korean RR.

**Later**: transparent overlay on the user's own wallpaper, motion artwork, updater, code signing, lock screen.

The status of each item and how it was verified is in [VALIDATION.md](VALIDATION.md).
