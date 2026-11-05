param(
  [Parameter(Mandatory)][string]$Exe,
  [string]$Output = "artifacts/smoke"
)
$ErrorActionPreference = "Stop"
$path = (Resolve-Path $Exe).Path
$null = New-Item -Path $Output -ItemType Directory -Force
$outputPath = (Resolve-Path $Output).Path
Get-CimInstance Win32_OperatingSystem | Select-Object Caption,Version,BuildNumber | Out-File "$outputPath/os.txt"
if ($env:GITHUB_ACTIONS -eq "true") { (New-Object -ComObject Shell.Application).MinimizeAll() }
$app = Start-Process -FilePath $path -ArgumentList "--smoke", "`"$outputPath`"" -PassThru
try {
  if (-not $app.WaitForExit(75000)) { throw "Lunalyr did not finish the fixture smoke test." }
  if (Test-Path "$outputPath/lunalyr.log") { Get-Content "$outputPath/lunalyr.log" }
  if ($app.ExitCode -ne 0) { throw "Lunalyr exited with code $($app.ExitCode)." }
  foreach ($name in @("settings.png", "wallpaper.png", "desktop.png")) {
    if (-not (Test-Path "$outputPath/$name")) { throw "No $name was captured." }
  }
} finally {
  if (-not $app.HasExited) { Stop-Process -Id $app.Id }
}
