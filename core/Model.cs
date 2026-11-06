using System.Text.Json;
using System.Text.Json.Serialization;

namespace Lunalyr.Core;

public sealed record Track(string Title, string Artist, string Album, double Duration, string Player)
{
    public string Key => string.Join("\n", Player, Artist, Title, Album,
        Math.Round(Duration / 2).ToString(System.Globalization.CultureInfo.InvariantCulture));
}

public sealed record Playback(Track? Track = null, double Position = 0, bool Playing = false,
    string? Artwork = null, bool CanPrevious = false, bool CanPlay = false, bool CanNext = false, double Rate = 1);
public sealed record Word(string Text, double Start, double End);
public sealed record Line(string Text, double? Start, double? End, Word[] Words, string? Reading = null)
{
    internal bool EndFromNextLine { get; init; }
}
public sealed record Lyrics(string Source, Line[] Lines, bool Instrumental = false)
{
    public int Quality => Lines.Any(line => line.Words.Length > 0)
        ? Lines.All(line => line.Words.Length > 0) ? 4 : 3
        : Lines.Any(line => line.Start.HasValue) ? 2 : Instrumental ? 1 : 0;
}

public sealed record Choice(string? Path = null, string? Source = null, double Offset = 0);

public sealed record Preferences
{
    public bool Onboarded { get; init; }
    public string Theme { get; init; } = "system";
    public bool WallpaperEnabled { get; init; } = true;
    public bool LyricsVisible { get; init; } = true;
    public string Background { get; init; } = "match";
    public string Visualiser { get; init; } = "silk";
    public string Preset { get; init; } = "default";
    public string Projection { get; init; } = "preset";
    public string Effect { get; init; } = "auto";
    public string Pattern { get; init; } = "auto";
    public string Placement { get; init; } = "fill";
    public string? Wallpaper { get; init; }
    public bool AlbumGradient { get; init; } = true;
    public bool Scrim { get; init; } = true;
    public double Speed { get; init; } = 1;
    public double Richness { get; init; } = 1;
    public double Darkening { get; init; } = 1;
    public bool Audio { get; init; }
    public double Sensitivity { get; init; } = 1;
    public double Punch { get; init; } = 1;
    public string Composition { get; init; } = "constellationEditorial";
    public string Font { get; init; } = "manrope";
    public bool Editorial { get; init; }
    public double LyricSize { get; init; } = 1;
    public double Spread { get; init; } = 1;
    public double Links { get; init; } = .20;
    public string TextColour { get; init; } = "white";
    public double ColourStrength { get; init; } = .75;
    public string CustomText { get; init; } = "#a9d7ff";
    public string CustomEnd { get; init; } = "#ffbada";
    public string Highlight { get; init; } = "complement";
    public string CustomHighlight { get; init; } = "#ff92a3";
    public Dictionary<string, string> Arrivals { get; init; } = [];
    public Dictionary<string, bool> Reveals { get; init; } = [];
    public Dictionary<string, bool> NextWords { get; init; } = [];
    public double PreviewLead { get; init; } = .45;
    public string Reading { get; init; } = "off";
    public bool Interludes { get; init; } = true;
    public string MissingLyrics { get; init; } = "notice";
    public string Player { get; init; } = "";
    public string LookupPolicy { get; init; } = "accuracy";
    public bool PreferWords { get; init; } = true;
    public string[] Sources { get; init; } = ["local", "lyricsplus", "netease", "lrclib", "qq", "kugou", "kuwo", "ovh"];
    public string[] DisabledSources { get; init; } = [];
    public string? LyricsFolder { get; init; }
    public int Fps { get; init; } = 30;
    public bool PauseCovered { get; init; } = true;
    public bool LowPower { get; init; }
    public bool RespectBattery { get; init; } = true;
    public bool Calm { get; init; }
    public bool Startup { get; init; }
}

public static class Store
{
    public static readonly JsonSerializerOptions Json = new(JsonSerializerDefaults.Web)
    {
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
        WriteIndented = false
    };

    public static T Read<T>(string path, T fallback)
    {
        if (!File.Exists(path)) return fallback;
        try { return JsonSerializer.Deserialize<T>(File.ReadAllText(path), Json) ?? fallback; }
        catch (JsonException) { return fallback; }
        catch (IOException) { return fallback; }
        catch (UnauthorizedAccessException) { return fallback; }
    }

    public static void Write<T>(string path, T value)
    {
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        string temporary = path + "." + Guid.NewGuid().ToString("N") + ".tmp";
        try
        {
            File.WriteAllText(temporary, JsonSerializer.Serialize(value, Json));
            File.Move(temporary, path, overwrite: true);
        }
        finally
        {
            if (File.Exists(temporary)) File.Delete(temporary);
        }
    }
}
