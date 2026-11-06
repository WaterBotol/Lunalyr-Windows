# Lunalyr for Windows

Animated lyrics and album-coloured wallpapers behind the Windows desktop icons. This replacement uses the macOS preview as its reference. See [Windows parity](WINDOWS-PARITY.md) for the source revisions, differences and verification.

## Run

Use the per-user installer or extract the entire portable ZIP and open `Lunalyr.exe`. The build targets 64-bit Windows 10 version 2004 and later. It uses the [Microsoft Edge WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/#download-section). Windows 11 generally includes it.

Play a track in an app that exposes a Windows media session. Open Lunalyr from the notification area to control playback, change the wallpaper, adjust lyric timing or copy an animated lyric card. Settings contains the live preview, lyric styles, sources and power controls. Online sources receive track metadata when enabled. Optional system audio is analysed locally in memory.

The replacement stores its preferences and imported files under `%LOCALAPPDATA%\Lunalyr\Windows2`. It uses a separate installation path and keeps the legacy app's data intact. Local lyric exports and wallpapers can be imported again from Settings.

## Build and test

Install the .NET 10 SDK, Node.js 22 or newer, and LLVM Clang with the Visual Studio C++ tools on Windows. The .NET build compiles and packages the native lyric rules automatically. Build the app and run the fixture checks from the repository root.

```powershell
dotnet run --project tests/Lunalyr.Tests.csproj -c Release
node --test tests/renderer.test.mjs
dotnet publish app/Lunalyr.csproj -c Release -r win-x64 --self-contained true -o artifacts/win-x64
./scripts/smoke.ps1 -Exe artifacts/win-x64/Lunalyr.exe
```

The smoke test uses a temporary app profile, publishes a Windows media session and loads a local mixed-timing lyric fixture. It captures the Settings WebView, wallpaper WebView and actual Windows desktop. Build the per-user installer with Inno Setup 6 using `installer/Lunalyr.iss` after publishing. The GitHub workflow packages both formats without creating a release.

The portable recording and mixed-timing rules live in [shared/LyricRules](shared/LyricRules). The Mac app imports a pinned copy of that source and runs the same fixtures. Update the rules and fixtures here, then sync that revision into the Mac checkout.

[Project licence](LICENSE) · [Third-party notices](THIRD_PARTY_NOTICES.md)
