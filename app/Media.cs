using System.Drawing.Imaging;
using Lunalyr.Core;
using Windows.Media.Control;

namespace Lunalyr;

internal sealed class Media
{
    private GlobalSystemMediaTransportControlsSessionManager? manager;
    private GlobalSystemMediaTransportControlsSession? session;
    private Track? track;
    private string? artwork;
    private readonly PlaybackClock clock = new();

    public string[] Players { get; private set; } = [];

    public async Task<Playback> Read(string preferred)
    {
        manager ??= await GlobalSystemMediaTransportControlsSessionManager.RequestAsync();
        var sessions = manager.GetSessions();
        Players = sessions.Select(item => item.SourceAppUserModelId).Distinct().Order().ToArray();
        session = preferred.Length > 0 ? sessions.FirstOrDefault(item => item.SourceAppUserModelId == preferred)
            : manager.GetCurrentSession();
        if (preferred.Length == 0 && session?.GetPlaybackInfo().PlaybackStatus != GlobalSystemMediaTransportControlsSessionPlaybackStatus.Playing)
            session = sessions.FirstOrDefault(item => item.GetPlaybackInfo().PlaybackStatus == GlobalSystemMediaTransportControlsSessionPlaybackStatus.Playing) ?? session;
        if (session is null) { track = null; return new Playback(); }

        var metadata = await session.TryGetMediaPropertiesAsync();
        if (string.IsNullOrWhiteSpace(metadata.Title)) { track = null; return new Playback(); }
        var timeline = session.GetTimelineProperties();
        var info = session.GetPlaybackInfo();
        string artist = string.IsNullOrWhiteSpace(metadata.Artist) ? metadata.AlbumArtist.Trim() : metadata.Artist.Trim();
        var identity = new Track(metadata.Title.Trim(), artist, metadata.AlbumTitle.Trim(), 0, session.SourceAppUserModelId);
        double duration = (timeline.EndTime - timeline.StartTime).TotalSeconds;
        if (duration is <= 0 or > 86_400) duration = 0;
        bool changed = track is null || (track with { Duration = 0 }) != identity;
        bool playing = info.PlaybackStatus == GlobalSystemMediaTransportControlsSessionPlaybackStatus.Playing;
        // Keep the recording and lyric lookup stable across incomplete timeline reports.
        var currentTrack = !changed && track is { Duration: > 0 } known ? known : identity with { Duration = duration };
        track = currentTrack;
        double rate = PlaybackClock.Rate(info.PlaybackRate);
        double position = clock.Update(changed, Math.Max(0, (timeline.Position - timeline.StartTime).TotalSeconds),
            (DateTimeOffset.UtcNow - timeline.LastUpdatedTime).TotalSeconds, Environment.TickCount64, playing, rate);
        if (changed) artwork = await Art(metadata);
        if (currentTrack.Duration > 0) position = Math.Min(currentTrack.Duration, position);
        return new Playback(currentTrack, position, playing, artwork, info.Controls.IsPreviousEnabled,
            info.Controls.IsPlayPauseToggleEnabled, info.Controls.IsNextEnabled, rate);
    }

    public async Task Control(string action)
    {
        var current = session;
        if (current is null) return;
        _ = action switch
        {
            "previous" => await current.TrySkipPreviousAsync(),
            "play" => await current.TryTogglePlayPauseAsync(),
            "next" => await current.TrySkipNextAsync(),
            _ => false
        };
    }

    private static async Task<string?> Art(GlobalSystemMediaTransportControlsSessionMediaProperties metadata)
    {
        if (metadata.Thumbnail is null) return null;
        try
        {
            using var input = await metadata.Thumbnail.OpenReadAsync();
            if (input.Size == 0 || input.Size > 8_000_000) return null;
            using var image = Image.FromStream(input.AsStreamForRead());
            using var thumbnail = new Bitmap(image, 160, 160);
            using var output = new MemoryStream();
            thumbnail.Save(output, ImageFormat.Png);
            return "data:image/png;base64," + Convert.ToBase64String(output.ToArray());
        }
        catch (Exception error) when (error is ArgumentException or IOException or System.Runtime.InteropServices.COMException)
        {
            return null;
        }
    }
}
