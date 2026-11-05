using System.Drawing.Imaging;
using Lunalyr.Core;
using Windows.Media.Control;

namespace Lunalyr;

internal sealed class Media
{
    private GlobalSystemMediaTransportControlsSessionManager? manager;
    private GlobalSystemMediaTransportControlsSession? session;
    private string identity = "";
    private string? artwork;
    private double sampledPosition;
    private long sampledAt;
    private DateTimeOffset timelineStamp;
    private bool wasPlaying;

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
        if (session is null) { identity = ""; return new Playback(); }

        var metadata = await session.TryGetMediaPropertiesAsync();
        if (string.IsNullOrWhiteSpace(metadata.Title)) { identity = ""; return new Playback(); }
        var timeline = session.GetTimelineProperties();
        var info = session.GetPlaybackInfo();
        var track = new Track(metadata.Title.Trim(), metadata.Artist.Trim(), metadata.AlbumTitle.Trim(),
            Math.Clamp((timeline.EndTime - timeline.StartTime).TotalSeconds, 0, 86_400), session.SourceAppUserModelId);
        bool playing = info.PlaybackStatus == GlobalSystemMediaTransportControlsSessionPlaybackStatus.Playing;
        bool changed = identity != track.Key;
        if (changed)
        {
            identity = track.Key;
            artwork = await Art(metadata);
        }

        long now = Environment.TickCount64;
        if (changed || timeline.LastUpdatedTime != timelineStamp || playing != wasPlaying)
        {
            double offset = changed ? 0 : Math.Clamp((DateTimeOffset.UtcNow - timeline.LastUpdatedTime).TotalSeconds, 0, 3);
            sampledPosition = Math.Max(0, (timeline.Position - timeline.StartTime).TotalSeconds + (playing ? offset : 0));
            sampledAt = now;
            timelineStamp = timeline.LastUpdatedTime;
            wasPlaying = playing;
        }
        double position = sampledPosition + (playing ? (now - sampledAt) / 1000.0 * (info.PlaybackRate ?? 1) : 0);
        if (track.Duration > 0) position = Math.Min(track.Duration, position);
        return new Playback(track, position, playing, artwork, info.Controls.IsPreviousEnabled,
            info.Controls.IsPlayPauseToggleEnabled, info.Controls.IsNextEnabled);
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
