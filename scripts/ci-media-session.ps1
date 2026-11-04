# Publishes a Windows media session (System Media Transport Controls) so that
# GSMTC reports a playing track during the CI smoke test. Must run in
# Windows PowerShell 5.1 (powershell.exe), which can load WinRT types.
#
# CI runners have no audio device, so nothing is actually played: the
# MediaPlayer's automatic SMTC integration is switched off and this script
# reports status, metadata and a moving timeline itself, the way a real
# player does. The track is the traditional hymn "Amazing Grace"
# (public-domain words), labelled as Judy Collins' recording so lyric
# providers have a match.

param(
    [int]$Seconds = 60,
    [double]$StartAt = 20,
    [double]$Duration = 245
)
$ErrorActionPreference = "Stop"

[void][Windows.Media.Playback.MediaPlayer, Windows.Media.Playback, ContentType = WindowsRuntime]
[void][Windows.Media.SystemMediaTransportControlsTimelineProperties, Windows.Media, ContentType = WindowsRuntime]
[void][Windows.Media.MediaPlaybackType, Windows.Media, ContentType = WindowsRuntime]
[void][Windows.Media.MediaPlaybackStatus, Windows.Media, ContentType = WindowsRuntime]

$player = New-Object Windows.Media.Playback.MediaPlayer
$player.CommandManager.IsEnabled = $false
$smtc = $player.SystemMediaTransportControls
$smtc.IsEnabled = $true
$smtc.IsPlayEnabled = $true
$smtc.IsPauseEnabled = $true
$updater = $smtc.DisplayUpdater
$updater.Type = [Windows.Media.MediaPlaybackType]::Music
$updater.MusicProperties.Title = "Amazing Grace"
$updater.MusicProperties.Artist = "Judy Collins"
$updater.Update()
$smtc.PlaybackStatus = [Windows.Media.MediaPlaybackStatus]::Playing

$started = [DateTime]::UtcNow
$timeline = New-Object Windows.Media.SystemMediaTransportControlsTimelineProperties
$timeline.StartTime = [TimeSpan]::Zero
$timeline.EndTime = [TimeSpan]::FromSeconds($Duration)
$timeline.MinSeekTime = [TimeSpan]::Zero
$timeline.MaxSeekTime = [TimeSpan]::FromSeconds($Duration)
Write-Output ("Media session started: Amazing Grace - Judy Collins, position {0} s of {1} s at {2:o}" -f $StartAt, $Duration, $started)

# Report the position every 2 s, like players that refresh their timeline.
while (([DateTime]::UtcNow - $started).TotalSeconds -lt $Seconds) {
    $elapsed = ([DateTime]::UtcNow - $started).TotalSeconds
    $timeline.Position = [TimeSpan]::FromSeconds([math]::Min($Duration, $StartAt + $elapsed))
    $smtc.UpdateTimelineProperties($timeline)
    Start-Sleep -Milliseconds 2000
}
$smtc.PlaybackStatus = [Windows.Media.MediaPlaybackStatus]::Stopped
$player.Dispose()
