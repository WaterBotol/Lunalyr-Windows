# Smoke test for CI (GitHub Actions windows-latest) or a local Windows machine.
#
# Launches the built app, optionally starts a Windows media session so GSMTC
# has something to report, waits, then records evidence:
#   - OS build (tells whether the 24H2 "raised desktop" path was exercised)
#   - the desktop window tree under Progman / WorkerW (is our window there?)
#   - a screenshot of the whole virtual screen
#   - the app's log file
# Fails if the app exits early or logs a panic.

param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [string]$OutDir = "smoke",
    [int]$Seconds = 40
)

$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path

# A running Lunalyr (an installed copy started at sign-in, `tauri dev`, or a
# leftover run) would receive our launch through the single-instance plugin
# and we would test the wrong process. Stop it and start with an empty log.
$others = @(Get-Process lunalyr -ErrorAction SilentlyContinue)
if ($others) {
    Write-Warning "Stopping running Lunalyr (pid $($others.Id -join ', '))"
    $others | Stop-Process -Force
    Start-Sleep -Seconds 2
}
$logDir = Join-Path $env:LOCALAPPDATA "io.github.waterbotol.lunalyr.windows\logs"
$log = Join-Path $logDir "lunalyr.log"
if (Test-Path $log) { Remove-Item $log -Force }

$os = Get-CimInstance Win32_OperatingSystem
"OS: $($os.Caption) $($os.Version) build $($os.BuildNumber)" | Tee-Object -FilePath "$OutDir/os.txt"

Add-Type -Language CSharp -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class DesktopTree {
    delegate bool EnumProc(IntPtr hwnd, IntPtr lParam);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr lParam);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumProc cb, IntPtr lParam);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern IntPtr GetParent(IntPtr h);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern int GetWindowLong(IntPtr h, int i);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    static string Describe(IntPtr h) {
        var c = new StringBuilder(256); GetClassName(h, c, 256);
        var t = new StringBuilder(256); GetWindowText(h, t, 256);
        RECT r; GetWindowRect(h, out r);
        return String.Format("0x{0:X} class={1} title=\"{2}\" visible={3} style=0x{4:X8} exstyle=0x{5:X8} rect=({6},{7},{8},{9}) parent=0x{10:X}",
            h.ToInt64(), c, t, IsWindowVisible(h), GetWindowLong(h, -16), GetWindowLong(h, -20), r.L, r.T, r.R, r.B, GetParent(h).ToInt64());
    }
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr FindWindow(string cls, string title);
    [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    public static bool Close(string title) {
        var h = FindWindow(null, title);
        return h != IntPtr.Zero && PostMessage(h, 0x0010 /* WM_CLOSE */, IntPtr.Zero, IntPtr.Zero);
    }
    // Closes visible standard dialogs (class #32770), e.g. the runner image's
    // "System Properties" / "Performance Options" windows. Lunalyr has none.
    public static int CloseDialogs() {
        int closed = 0;
        EnumWindows((h, _) => {
            var c = new StringBuilder(64); GetClassName(h, c, 64);
            if (c.ToString() == "#32770" && IsWindowVisible(h) && PostMessage(h, 0x0010, IntPtr.Zero, IntPtr.Zero)) closed++;
            return true;
        }, IntPtr.Zero);
        return closed;
    }
    public static string Dump() {
        var lines = new List<string>();
        EnumWindows((top, _) => {
            var c = new StringBuilder(256); GetClassName(top, c, 256);
            var name = c.ToString();
            if (name == "Progman" || name == "WorkerW") {
                lines.Add(Describe(top));
                EnumChildWindows(top, (child, __) => {
                    if (GetParent(child) == top) lines.Add("    " + Describe(child));
                    return true;
                }, IntPtr.Zero);
            }
            return true;
        }, IntPtr.Zero);
        return String.Join(Environment.NewLine, lines);
    }
}
"@

# Optional: a media session so GSMTC reports a track (Windows PowerShell 5.1
# can load WinRT types; pwsh cannot). Failure here is not a test failure:
# CI runners may have no audio endpoint.
$mediaScript = Join-Path $PSScriptRoot "ci-media-session.ps1"
$media = $null
if (Test-Path $mediaScript) {
    $media = Start-Process -FilePath "powershell.exe" -ArgumentList "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $mediaScript, "-Seconds", ($Seconds + 30) -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput "$OutDir/media-session.txt" -RedirectStandardError "$OutDir/media-session-err.txt"
    # Wait (up to 20 s) until the session is published.
    for ($i = 0; $i -lt 40; $i++) {
        if ((Test-Path "$OutDir/media-session.txt") -and (Select-String -Path "$OutDir/media-session.txt" -Pattern "started" -Quiet)) { break }
        if ($media.HasExited) { break }
        Start-Sleep -Milliseconds 500
    }
}

$env:RUST_LOG = "debug"
# --autostart: behave as when launched at sign-in (no first-run settings window).
$app = Start-Process -FilePath $Exe -ArgumentList "--autostart" -PassThru
Write-Host "Started $Exe (pid $($app.Id)); waiting $Seconds s"
Start-Sleep -Seconds ([math]::Max(5, $Seconds - 6))
# Show the bare desktop for the screenshot: minimise every window (including
# the runner's console), then give the wallpaper a moment to resume.
(New-Object -ComObject Shell.Application).MinimizeAll()
Start-Sleep -Seconds 6

[DesktopTree]::Dump() | Tee-Object -FilePath "$OutDir/desktop-tree.txt"

# Runner images sometimes show "System Properties" / "Performance Options".
for ($i = 0; $i -lt 3; $i++) {
    if ([DesktopTree]::CloseDialogs() -eq 0) { break }
    Start-Sleep -Milliseconds 700
}
Start-Sleep -Seconds 1

Add-Type -AssemblyName System.Windows.Forms, System.Drawing
function Save-Screen([string]$Path) {
    $bounds = [System.Windows.Forms.SystemInformation]::VirtualScreen
    $bitmap = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($bounds.Left, $bounds.Top, 0, 0, $bitmap.Size)
    $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose(); $bitmap.Dispose()
}
# Two frames a few seconds apart show whether the lyrics move with the song.
Save-Screen "$OutDir/desktop.png"
Start-Sleep -Seconds 3
Save-Screen "$OutDir/desktop-2.png"

$alive = -not $app.HasExited
if ($alive) { Stop-Process -Id $app.Id -Force }
if ($media -and -not $media.HasExited) { Stop-Process -Id $media.Id -Force }

if (Test-Path $log) {
    Copy-Item $log "$OutDir/lunalyr.log"
    Get-Content $log | Select-Object -Last 200
} else {
    Write-Warning "No log at $log"
}

if (-not $alive) { throw "Lunalyr exited early with code $($app.ExitCode)" }
if ((Test-Path $log) -and (Select-String -Path $log -Pattern "panicked" -Quiet)) { throw "Lunalyr logged a panic" }
Write-Host "Smoke test passed: app stayed up for $Seconds s"
