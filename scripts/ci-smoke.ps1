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
    [int]$Seconds = 25
)

$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path

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
    $media = Start-Process -FilePath "powershell.exe" -ArgumentList "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $mediaScript, "-Seconds", ($Seconds + 20) -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput "$OutDir/media-session.txt" -RedirectStandardError "$OutDir/media-session-err.txt"
    Start-Sleep -Seconds 3
}

$env:RUST_LOG = "debug"
$app = Start-Process -FilePath $Exe -PassThru
Write-Host "Started $Exe (pid $($app.Id)); waiting $Seconds s"
Start-Sleep -Seconds $Seconds

[DesktopTree]::Dump() | Tee-Object -FilePath "$OutDir/desktop-tree.txt"

Add-Type -AssemblyName System.Windows.Forms, System.Drawing
$bounds = [System.Windows.Forms.SystemInformation]::VirtualScreen
$bitmap = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$graphics.CopyFromScreen($bounds.Left, $bounds.Top, 0, 0, $bitmap.Size)
$bitmap.Save("$OutDir/desktop.png", [System.Drawing.Imaging.ImageFormat]::Png)
$graphics.Dispose(); $bitmap.Dispose()

$alive = -not $app.HasExited
if ($alive) { Stop-Process -Id $app.Id -Force }
if ($media -and -not $media.HasExited) { Stop-Process -Id $media.Id -Force }

$logDir = Join-Path $env:LOCALAPPDATA "io.github.waterbotol.lunalyr.windows\logs"
$log = Join-Path $logDir "lunalyr.log"
if (Test-Path $log) {
    Copy-Item $log "$OutDir/lunalyr.log"
    Get-Content $log | Select-Object -Last 200
} else {
    Write-Warning "No log at $log"
}

if (-not $alive) { throw "Lunalyr exited early with code $($app.ExitCode)" }
if ((Test-Path $log) -and (Select-String -Path $log -Pattern "panicked" -Quiet)) { throw "Lunalyr logged a panic" }
Write-Host "Smoke test passed: app stayed up for $Seconds s"
