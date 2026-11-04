# Starts a Windows media session (System Media Transport Controls) so that
# GSMTC reports a playing track during the CI smoke test. Must run in
# Windows PowerShell 5.1 (powershell.exe), which can load WinRT types.
#
# The track is the traditional hymn "Amazing Grace" (public-domain words),
# labelled as Judy Collins' recording so lyric providers have a match. The
# audio is a generated quiet tone of the same length as that recording.

param([int]$Seconds = 45)
$ErrorActionPreference = "Stop"

$durationSeconds = 245
$sampleRate = 8000
$path = Join-Path $env:TEMP "lunalyr-ci-tone.wav"
$samples = $durationSeconds * $sampleRate
$stream = [System.IO.File]::Create($path)
$writer = New-Object System.IO.BinaryWriter($stream)
$writer.Write([System.Text.Encoding]::ASCII.GetBytes("RIFF"))
$writer.Write([int](36 + $samples))
$writer.Write([System.Text.Encoding]::ASCII.GetBytes("WAVEfmt "))
$writer.Write([int]16); $writer.Write([int16]1); $writer.Write([int16]1)
$writer.Write([int]$sampleRate); $writer.Write([int]$sampleRate); $writer.Write([int16]1); $writer.Write([int16]8)
$writer.Write([System.Text.Encoding]::ASCII.GetBytes("data"))
$writer.Write([int]$samples)
$buffer = New-Object byte[] $samples
for ($i = 0; $i -lt $samples; $i++) {
    $buffer[$i] = [byte](128 + [math]::Round(6 * [math]::Sin(2 * [math]::PI * 220 * $i / $sampleRate)))
}
$writer.Write($buffer)
$writer.Close()

[void][Windows.Media.Playback.MediaPlayer, Windows.Media.Playback, ContentType = WindowsRuntime]
[void][Windows.Media.Core.MediaSource, Windows.Media.Core, ContentType = WindowsRuntime]
[void][Windows.Media.MediaPlaybackType, Windows.Media, ContentType = WindowsRuntime]

$player = New-Object Windows.Media.Playback.MediaPlayer
$player.Source = [Windows.Media.Core.MediaSource]::CreateFromUri([Uri]$path)
$smtc = $player.SystemMediaTransportControls
$smtc.IsEnabled = $true
$smtc.IsPlayEnabled = $true
$smtc.IsPauseEnabled = $true
$updater = $smtc.DisplayUpdater
$updater.Type = [Windows.Media.MediaPlaybackType]::Music
$updater.MusicProperties.Title = "Amazing Grace"
$updater.MusicProperties.Artist = "Judy Collins"
$updater.Update()
$player.Play()
Write-Output "Media session started: Amazing Grace - Judy Collins ($durationSeconds s tone)"
Start-Sleep -Seconds $Seconds
$player.Pause()
$player.Dispose()
