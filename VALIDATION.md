# Validation

What has been verified, how, and what still needs a real Windows machine. There is no Windows machine in the development setup: everything Windows-specific was either type-checked from macOS or run on GitHub Actions `windows-latest`.

## Verified on GitHub Actions `windows-latest`

The runner is **Windows Server 2025 Datacenter, build 26100** — the Windows 11 24H2 code base with the new "raised desktop" layout (Progman has `WS_EX_NOREDIRECTIONBITMAP`; `SHELLDLL_DefView` is a layered child of Progman). It has an interactive 1024×768 desktop and no audio device. Workflow: [`.github/workflows/windows.yml`](.github/workflows/windows.yml); smoke test: [`scripts/ci-smoke.ps1`](scripts/ci-smoke.ps1). Each run uploads the installer and a `smoke-<commit>` artifact (OS build, desktop window tree, two screenshots, the app log).

| Check | Evidence (CI run on `main`) |
|---|---|
| Rust unit/integration tests on Windows (lyric core, controller, platform helpers, DSP) | `cargo test --workspace --locked` passes |
| Renderer unit tests and typecheck on Windows | `npm test`, `npm run typecheck` pass |
| Clippy on Windows | `cargo clippy --workspace --all-targets` passes |
| App and NSIS installer build | `npm run tauri build -- --bundles nsis` produces `Lunalyr_0.1.0_x64-setup.exe` |
| App starts and stays up (release build, `--autostart`) | smoke test: alive after 40 s, no panic in the log |
| Desktop layout detection on 24H2 | log: `wallpaper: raised (24H2+) desktop (Progman …, DefView …, WorkerW …)` |
| Wallpaper parented behind the icons, correct z-order and styles | window tree: Progman → `SHELLDLL_DefView`, then `Lunalyr wallpaper` (`WS_CHILD|WS_DISABLED`, ex-style `WS_EX_LAYERED|WS_EX_TRANSPARENT|WS_EX_NOACTIVATE|WS_EX_TOOLWINDOW`, full monitor rect), then Explorer's `WorkerW` last |
| WebView2 renders inside the reparented layered child | screenshot: the album-colour gradient replaces the runner's blue wallpaper with the desktop icons drawn on top; log: `wallpaper-display1-1 loaded and requested a snapshot` (page loaded, IPC round trip works) |
| GSMTC now playing | log: `media: following the media session of powershell.exe`, `now playing: "Amazing Grace" by Judy Collins (245 s, powershell.exe)` from a System Media Transport Controls session published by [`scripts/ci-media-session.ps1`](scripts/ci-media-session.ps1) |
| Lyric lookup from Windows | log: `lyrics for "Amazing Grace" by Judy Collins: Word-by-word · NetEase (24 lines; 6 candidates)` about 3 s after the track appeared |
| Lyrics drawn on the wallpaper and advancing with the timeline | two screenshots 3 s apart: "And grace" then "And grace my fears" in the Editorial composition, the active word highlighted, desktop icons on top ([docs/screenshots/ci-windows-24h2-wallpaper.png](docs/screenshots/ci-windows-24h2-wallpaper.png)) |
| Display power notifications | log: `desktop: displays on` → `DisplaysAsleep(false)` right after registering for `GUID_SESSION_DISPLAY_STATUS` |
| Deferred wallpaper creation (no window creation inside WebView2 callbacks) | log: rebuild → `attached` → `loaded and requested a snapshot` |

Not exercised by CI: the legacy (pre-24H2) WorkerW layout, multiple monitors, mixed DPI, Explorer restarts, full-screen coverage, session lock, WASAPI loopback (no audio device on runners), the Japanese phonetic analyser, autostart, the settings window and the installer's install/uninstall.

## Adversarial code reviews

Because the Windows-specific code cannot run on the development machine, every area was reviewed by independent reviewer agents, and every finding was handed to a separate skeptic instructed to refute it before it was accepted:

| Review | Findings | Survived refutation | Highlights fixed |
|---|---|---|---|
| App shell (concurrency, Tauri semantics, controller vs macOS, Windows/CI behaviour) | 25 | 22 (overlapping) | UI-thread deadlock when a settings change created a webview inside a WebView2 callback; per-window pause leaking to every monitor (plain `listen()` targets *Any*); late durations not re-running lookups; launch-at-login not read back |
| Platform layer and lyric core vs the Swift sources | 21 | 20 | stack overflow from a crafted LRCLIB Lyricsfile (YAML depth limit); quadratic YAML scan; timeline anchor not reset between tracks; 24H2 WorkerW replacement causing needless recreates; DSP band layout and stereo handling vs `AudioAnalyzer.swift`; Korean final ㅇ; system proxy |
| Renderer, bridge and settings UI | 20 | 16 | frame loop waking on every vsync; quadratic Echo measurement; failed imports reported as success; About links inert under Tauri; settings pause toggle not synced |

Each batch of fixes was then checked by a second independent verifier (re-reading the diff and re-running the tests and the Windows-target lint) before being committed.

## Verified on the development Mac

| Check | Command | Result |
|---|---|---|
| Lyric core | `cargo test -p lunalyr-core` | 178 passed (149 unit, 29 integration with fixture HTTP) |
| Live provider lookups | `cargo test -p lunalyr-core --test live -- --ignored` | 7 passed: NetEase word-by-word, Lyrics+ mixed, LRCLIB/QQ/Kugou/Kuwo line-synced, lyrics.ovh plain (metadata only printed) |
| App shell, platform helpers, DSP | `cargo test -p lunalyr --lib` | 63 passed (controller state machine, coverage/timeline/style logic, audio analysis) |
| Windows target type-check and lint | `cargo clippy -p lunalyr --all-targets --target x86_64-pc-windows-msvc` | clean |
| Renderer | `npm test` / `npm run typecheck` / `npm run build` | 141 tests passed; clean; built |
| Motion/timing parity | `ui/src/timing/golden.test.ts` | TypeScript matches vectors generated from the **unmodified macOS C** (`LyricMotion.c`, `ConstellationGeometry.c`) to 1e-11 |
| Renderer visual check | `demo.html` in a browser | all eight compositions, line-timed, readings (pinyin), plain, notices, Spectrum and Wavefield visualisers, settings window |
| Shell end to end on macOS (stand-in platform) | `cargo run -p lunalyr --features custom-protocol` with `LUNALYR_DEV_TITLE=…` | track → lookup → `Word-by-word · NetEase (24 lines; 6 candidates)`; wallpaper page loaded and fetched its snapshot |

## Needs a real Windows desktop

These depend on hardware, user setups or Explorer behaviour that CI cannot reproduce:

1. The legacy WorkerW layout on Windows 10 and Windows 11 23H2 (0x052C split, parenting into the WorkerW behind the icons).
2. Desktop interaction with the wallpaper attached: icon clicks and drags, right-click menu, rubber-band selection, keyboard focus.
3. Multiple monitors, negative virtual-screen origins, mixed DPI (a child window reports its parent's DPI, so WebView2's scale may follow the primary monitor), monitor hot-plug and rotation.
4. Explorer restarts: wallpapers are recreated (`ShellRestarted`); Explorer replacing only its WorkerW is repaired by the 2 s watchdog without recreating webviews.
5. Coverage pausing with real apps: maximised windows, exclusive full-screen games, video players, Task View, full-screen Start, the lock screen; pausing while the displays are off.
6. Real players' GSMTC timelines and artwork: Spotify, Apple Music for Windows, Edge/Chrome/Firefox media sessions, pause/seek/resume behaviour and sync accuracy (adjust with the global or per-song offset).
7. WASAPI loopback with real devices: mix formats, device switching (headphones/Bluetooth), device loss, silence, the Wavefield visualiser's response.
8. Japanese readings from `JapanesePhoneticAnalyzer` on different Windows editions and display languages.
9. Launch at login via the Run key (and Task Manager's startup toggle), single-instance behaviour, tray menu.
10. CPU/GPU and memory cost over hours on laptops and multi-monitor desktops; WebView2 throttling behind the icons (native occlusion calculation is disabled).
11. Installer: per-user install, WebView2 bootstrapper on machines without the runtime, SmartScreen prompt (the app is unsigned), uninstall.
12. Installed fonts (Segoe UI Variable, Yu Gothic UI, Microsoft YaHei UI, Malgun Gothic) and CJK/RTL rendering in WebView2.
