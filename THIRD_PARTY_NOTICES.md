# Attribution and third-party components

Lunalyr for Windows is a port of [Lunalyr](https://github.com/WaterBotol/Lunalyr) (macOS), which is an unofficial modification of [Verso](https://github.com/AkkiCode06/verso) by Akki / AkkiCode06, pinned in [UPSTREAM.json](UPSTREAM.json). Verso's original GPLv3 plus Commons Clause licence is retained unchanged in [LICENSE](LICENSE) and is installed with the app as `licenses/LICENSE.txt`. Any licence notice or attribution required by the licence includes the Commons Clause License Condition notice at the top of that file.

The Windows port reuses the macOS app's design and its portable logic (lyric providers, LRC/YRC parsing, match ranking, word timing, typography and motion), re-implemented in Rust and TypeScript. No Swift, Metal or AppKit code is compiled into this app.

This file lists every third-party component bundled in the Windows app. It is updated whenever a component is added.

## Runtime frameworks

- [Tauri 2](https://github.com/tauri-apps/tauri) — MIT or Apache-2.0.
- [Microsoft Edge WebView2](https://developer.microsoft.com/microsoft-edge/webview2/) — installed by the Windows WebView2 bootstrapper; not redistributed in this repository.
- [windows-rs](https://github.com/microsoft/windows-rs) — MIT or Apache-2.0.

Rust crate and npm package licences are recorded in `Cargo.lock` / `package-lock.json`; all are permissive (MIT, Apache-2.0, BSD, ISC, Zlib, Unicode-3.0 or MPL-2.0 for unmodified dependencies).

No source code from VoxelWall, VoxelVerse or Lyrimuse has been incorporated, and no reference footage is distributed.
