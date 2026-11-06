using System.Net;
using System.Text.Json;
using Lunalyr.Core;

if (args is ["--preferences"])
{
    Console.Write(JsonSerializer.Serialize(new Preferences(), Store.Json));
    return;
}

static void Assert(bool condition, string message)
{
    if (!condition) throw new Exception(message);
}

var mixed = LyricParser.Lrc("""
    [offset:100]
    [00:01.00]<00:01.00>First <00:01.50>words
    [00:04.00]Plain fallback line
    [00:06.00]<00:06.00>Last <00:07.00>words
    """, 10);
Assert(mixed.Length == 3 && mixed[0].Words.Length == 2 && mixed[1].Words.Length == 0, "LRC retains mixed timing.");
Assert(Math.Abs(mixed[0].Words[0].End - 1.6) < .001 && mixed[0].End == 4.1, "LRC offset and ends.");
Assert(new Lyrics("fixture", mixed).Quality == 3, "Mixed precision is labelled accurately.");
var repeats = LyricParser.Lrc("[00:01][00:05]<00:01>Same <00:02>line", 10);
Assert(repeats[1].Words[1].Start == 6, "Repeated LRC shifts enhanced markers.");
var ttml = LyricParser.Ttml("""
    <tt xmlns="http://www.w3.org/ns/ttml"><body><div><p begin="00:00:01.250" end="00:00:04.000">
    <span begin="1.25s" end="2s">Shine</span> <span begin="2s" end="3s">on</span><span role="x-roman">reading</span>
    </p></div></body></tt>
    """, 12);
Assert(ttml[0].Start == 1.25 && ttml[0].Text == "Shine on" && ttml[0].Reading == "reading", "TTML text, timing and readings.");
Assert(LyricParser.Yrc("[1000,2000](1000,500,0)One (1500,1500,0)more")[0].Words[1].End == 3, "YRC timing.");
try
{
    LyricParser.Ttml("<!DOCTYPE doc [<!ENTITY data SYSTEM 'file:///test'>]><doc>&data;</doc>", 10);
    throw new Exception("External TTML entity accepted.");
}
catch (System.Xml.XmlException) { }

Assert(Pronunciation.Korean("한국어 같이 국물 신라") == "hangugeo gachi gungmul silla", "Hangul liaison and assimilation.");
Assert(Pronunciation.Kana("きょう スーパー まっちゃ") == "kyou suupaa matcha", "Kana combinations, long vowels and gemination.");

var yaml = LyricParser.Parse("""
    metadata: {offset_ms: 500}
    lines:
      - text: Hold on
        start_ms: 1000
        end_ms: 2000
        words:
          - {text: Hold, start_ms: 1000}
          - {text: on, start_ms: 1500}
    """, ".yaml", 10);
Assert(yaml[0].Start == 1.5 && yaml[0].Words[0].End == 2 && yaml[0].Words[1].End == 2.5,
    "YAML retains verified lyric timing and file offset.");

var track = new Track("Under the stars", "Tides & Clouds", "", 190, "fixture");
Assert(LyricLookup.Match(track, "Under the Stars", "Tides", 192), "Equivalent metadata matches.");
Assert(LyricLookup.Match(track, "Tides - Under the Stars (feat. Clouds) - 2011 Remaster", "Tides", 192),
    "Credits and remaster tags do not hide the same recording.");
Assert(!LyricLookup.Match(track, "Under the Stars (live)", "Tides", 190), "Reject mismatched titles.");
Assert(!LyricLookup.Match(track, "Under the Stars", "Other Artist", 190), "Reject unrelated artists.");
Assert(!LyricLookup.Match(track, "Under the Stars", "Tides", 245), "Reject mismatched durations.");

var clock = new PlaybackClock();
double rate = PlaybackClock.Rate(0);
Assert(rate == 1 && PlaybackClock.Rate(double.NaN) == 1, "Invalid media rates use normal speed.");
Assert(clock.Update(true, 5, 0, 0, true, rate) == 5, "Clock starts from the reported position.");
Assert(clock.Update(false, 5, 0, 500, true, rate) == 5.5, "Repeated reports keep advancing.");
Assert(clock.Update(false, 5, 0, 1000, false, rate) == 6, "Pausing keeps the extrapolated position.");
Assert(clock.Update(false, 5, 0, 1500, false, rate) == 6, "Paused time does not advance.");
Assert(clock.Update(false, 5, 0, 2500, true, 2) == 6, "Resuming keeps the paused position.");
Assert(clock.Update(false, 5, 0, 3000, true, 2) == 7, "Playback uses the real rate.");
Assert(clock.Update(false, 2, .25, 3500, true, 2) == 2.5, "A fresh seek corrects the clock.");
Assert(clock.Update(true, 0, 0, 4000, true, rate) == 0, "A new track resets the clock.");
string temp = Path.Combine(Path.GetTempPath(), "lunalyr-test-" + Guid.NewGuid().ToString("N"));
Directory.CreateDirectory(temp);
try
{
    string path = Path.Combine(temp, "settings.json");
    Store.Write(path, new Preferences { Fps = 60 });
    Store.Write(path, new Preferences { Fps = 15 });
    Assert(Store.Read(path, new Preferences()).Fps == 15, "Preferences replace atomically.");
    File.WriteAllText(Path.Combine(temp, "Tides & Clouds - Under the stars.lrc"), "[00:01]Keep this line");
    using var http = new HttpClient(new FixtureHttp());
    var lyrics = new LyricLookup(http);
    var sources = await lyrics.Find(track, new Preferences { Sources = ["local", "lrclib"] }, temp, default);
    Assert(sources.Length == 2 && sources[0].Source == "lrclib" && sources[0].Quality == 4,
        "Prefer verified word timing to line timing.");
    sources = await lyrics.Find(track, new Preferences { Sources = ["local", "lrclib"], LookupPolicy = "order", PreferWords = false }, temp, default);
    Assert(sources.Length == 1 && sources[0].Source == "local", "Strict source order respects the first source.");
    var decorated = new Track("Tides - Back Again (feat. Clouds)", "Tides, Clouds", "Evening", 0, "fixture");
    sources = await lyrics.Find(decorated, new Preferences { Sources = ["lrclib", "lyricsplus"] }, temp, default);
    Assert(sources is [{ Source: "lrclib", Quality: 4 }, { Source: "lyricsplus", Quality: 2 }],
        "Find enhanced LRCLIB lyrics via a broader query and keep Lyrics+ lines without explicit durations.");
    Assert(sources[1].Lines[0].End == 5 && sources[1].Lines[1].Start == 5, "Lyrics+ uses the next timestamp for missing durations.");
}
finally { Directory.Delete(temp, true); }
Console.WriteLine("Core checks passed (LRC, YRC, TTML, ranking, lookup, preferences).");

sealed class FixtureHttp : HttpMessageHandler
{
    protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
    {
        var uri = request.RequestUri!;
        string query = Uri.UnescapeDataString(uri.Query);
        string body;
        if (uri.Host == "lyricsplus.binimum.org" && query == "?title=Back Again&artist=Tides, Clouds&album=Evening")
            body = """
                {"metadata":{"title":"Back Again","artist":"Tides"},"lyrics":[
                  {"time":1000,"text":"A line without duration"},
                  {"time":5000,"duration":2000,"text":"Another line"}]}
                """;
        else if (uri.Host != "lrclib.net") throw new InvalidOperationException("Unexpected provider.");
        else if (uri.AbsolutePath == "/api/get")
        {
            if (!query.Contains("duration=190")) throw new InvalidOperationException("Expected exact recording lookup.");
            body = "{}";
        }
        else if (query == "?q=Back Again Tides")
            body = JsonSerializer.Serialize(new[] { new
            {
                trackName = "Back Again", artistName = "Tides", duration = 190,
                lyricsfile = """
                    lines:
                      - text: Back again
                        start_ms: 1000
                        end_ms: 4000
                        words:
                          - {text: Back, start_ms: 1000}
                          - {text: again, start_ms: 2000}
                    """
            } });
        else if (query == "?track_name=Back Again&artist_name=Tides, Clouds") body = "[]";
        else
            body = """
                [{"trackName":"Under the stars","artistName":"Tides","duration":245,"syncedLyrics":"[00:02]Wrong duration"},
                 {"trackName":"Under the stars","artistName":"Tides","duration":191,"syncedLyrics":"[00:02]<00:02>Word <00:03>timing"}]
                """;
        return Task.FromResult(new HttpResponseMessage(HttpStatusCode.OK) { Content = new StringContent(body) });
    }
}
