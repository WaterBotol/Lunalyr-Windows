namespace Lunalyr.Core;

public sealed class PlaybackClock
{
    private double reportedPosition;
    private double anchorPosition;
    private long anchorTime;
    private bool wasPlaying;
    private double rate = 1;

    public static double Rate(double? reportedRate) =>
        reportedRate is > 0 && double.IsFinite(reportedRate.Value) ? reportedRate.Value : 1;

    public double Update(bool newTrack, double position, double reportAge, long now, bool playing, double playbackRate)
    {
        double estimate = anchorPosition + (wasPlaying ? (now - anchorTime) / 1000d * rate : 0);
        // Some players republish the same position with a fresh timestamp.
        // Only a changed position can correct the clock for the same track.
        if (newTrack || position != reportedPosition)
        {
            double age = playing && reportAge is > 0 and <= 3 ? reportAge : 0;
            anchorPosition = position + age * playbackRate;
            anchorTime = now;
            reportedPosition = position;
        }
        else if (playing != wasPlaying || playbackRate != rate)
        {
            anchorPosition = estimate;
            anchorTime = now;
        }
        wasPlaying = playing;
        rate = playbackRate;
        return anchorPosition + (playing ? (now - anchorTime) / 1000d * rate : 0);
    }
}
