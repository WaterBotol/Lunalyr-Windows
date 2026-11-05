# Windows parity

## Reference baseline

Source verification took place on 5 November 2026 AEDT, before changing the implementation.

| Role | Revision |
| --- | --- |
| Windows replacement branch parent, WaterBotol/Lunalyr-Windows main | `38435864103d489114ef6f466694a9ffecbd9e53` |
| Public macOS main, WaterBotol/Lunalyr | `710ac5b3da826410bebca4172e6efa799f865af9` |
| macOS product reference, local Star Trails and Larger GIFs source bundle | `adc801be711f1e6a0ec049e1f064ec9d6db06bfc` |
| Public and local macOS merge base | `82bf58487e91fdd2a40e8126af32fa5107e8b77b` |

The public macOS main is 17 commits ahead of the merge base. These commits change documentation and build/repository files, with no changes under Sources or ThirdParty. The local bundle is 292 commits ahead of that base and contains all seven earlier local GUI bundle tips. It includes the recovered settings, tray and onboarding work, plus later timing, highlight, lyric-card and Star Trails changes. It is the visual and behavioural reference. The two additional public branches, docs-accuracy (`c04ac0a00a2ca502aff12564a4a5962f4fa85a05`) and repo-polish (`58c151176855843f51807f26f44424e53bcc1d93`), are already merged into public main.

GUI Test Build 16 identifies itself as macOS 0.4.10 build 17. Its three embedded visualiser engines byte-match the local source bundle. The ZIP does not identify a Git SHA, so that check corroborates the engine provenance without proving the executable's exact revision. The local settings, menu bar, onboarding and Star Trails screenshots supplied the layout and colour references.

## Replacement scope

The replacement branch removes the legacy Rust/Tauri app, renderer, installer and build pipeline. It uses a new WinForms and WebView2 host with a separate C# lyric core and a browser-based wallpaper renderer. This branch contains the source and CI recipes. The repository's earlier commits and releases retain the prior port.

- Windows media sessions supply track information, static artwork, position and player controls. Explorer hosts the desktop surfaces, with a separate surface per display.
- Settings, the notification-area panel and six-step onboarding follow the local macOS layout. They share the same renderer and settings state.
- The lyric core handles enhanced LRC, line LRC, YRC, TTML and text. It includes local export, Lyrics+, NetEase, LRCLIB, QQ, Kugou, Kuwo and lyrics.ovh adapters, source choices and per-recording offsets.
- All eight compositions, per-style entrances, next-word preview, mixed timing, lyric and highlight colour, and animated GIF cards are implemented. Cards use the reference's 1080 × 1350 dimensions.
- The three web visualiser engines and Wavefield presets come directly from the reference. All 28 ambient effects and seven music scenes have WebGL shader translations. A nine-region palette follows cover changes, including in the Star Trails scene.
- Windows integration includes opt-in WASAPI loopback, global shortcuts, optional launch at login, wallpaper import, battery controls and an x64 package.

## Skipped or different

| Reference feature | Windows result and reason |
| --- | --- |
| Your wallpaper overlay | Disabled. The opaque Explorer-hosted WebView cannot reliably composite transparent lyrics over arbitrary system wallpaper, slideshow or third-party wallpaper surfaces. An opaque fallback would hide the wallpaper, and replacing it would discard the behaviour of the user's existing setup. Importing an image or video works in a separate Lunalyr-owned surface. |
| Animated Apple Music cover lookup and exclusions | Static media-session thumbnails supply colour. Windows media sessions do not expose animated cover URLs. An unofficial integration with a signed-in player's private caches would risk account privacy and asset misidentification. |
| Direct Apple Music cached-lyrics access and macOS Apple Events | The Windows host uses the supported media-session interface. It has no access to Apple Music's private lyric cache and does not read player credentials or modify a player database. |
| Sparkle automatic updates | The About page opens the Windows releases page. The macOS feed contains Mac binaries and has separate signing and trust rules. Windows needs its own signed installer and feed before automatic replacement can be enabled. |
| Legacy profile upgrade | Uses a separate profile and install location. The legacy app's profile, media files, lyric caches, autostart and installer registration are left intact. There is no automatic conversion of stored choices or preferences. Manually import source files in the new app. |
| Native fonts, window chrome and menu-bar ticker | Uses Windows window controls, Segoe UI and bundled Manrope. The Windows tray has a fixed-size icon and shows lyric text in its flyout. Private macOS fonts and a persistent text label in the system tray are unavailable through supported Windows APIs. |
| Native audio visualisers | Silk Ribbons, Spectral Aurora, Spectrum Tunnel, Aurora Curtains, Fosfora Aurora, Colour Strata and Overprint use shader translations with Windows spectrum input and a different render budget. Star Trails is a separate 2D implementation with album-coloured sky, craft and trails. It omits the Metal particle simulation and isolated vocal, pad and percussion triggers. Wavefield and Wavefield Sphere use the bundled web engine, so they differ from the native Metal modes. The web engines run in Chromium. |
| Typography and timing fidelity | The same composition families and control labels use a Windows canvas layout. Fine-grained typography, entrance easing, track-specific mood selection, end-of-line choreography and some provider parsing details differ. Translation and background voices are not surfaced. |
| Romanisation and local interchange | Displays source-supplied readings, including paired `.roma.lrc` files. macOS tokenizers and system pronunciation databases are not available in the Windows renderer. It does not invent readings for unrecognised text. YAML and the complete Lyrimuse export variants are outside this build. |
| Wallpaper effects and display power | The WebGL ambient effects use a lower pixel budget and a colour fallback when WebGL cannot start. Coverage, lock and battery are handled. Display-off notifications, mixed-DPI, Explorer restarts and third-party shell configurations still require Windows testing. |
| Installer signing | The CI preview is unsigned. No publisher credentials or signing infrastructure are configured. |

## Verification

Build and fixture coverage is recorded with the branch's verification run. The lyric fixtures test parser timing, duplicate timestamps, source ranking, local imports and atomic preferences. Renderer tests exercise mixed cues, word preview, nine-stop colour and GIF framing. The Windows smoke test publishes a Windows media session and captures the Settings WebView, wallpaper WebView and Explorer desktop with fixture lyrics. Live player compatibility, endpoint availability, real audio capture, and installer installation on a user's machine are separate checks.
