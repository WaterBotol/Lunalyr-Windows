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

var track = new Track("Under the stars", "Tides & Clouds", "", 190, "fixture");
Assert(LyricLookup.Match(track, "Under the Stars", "Tides", 192), "Equivalent metadata matches.");
Assert(!LyricLookup.Match(track, "Under the Stars (live)", "Tides", 190), "Reject mismatched titles.");
Assert(!LyricLookup.Match(track, "Under the Stars", "Tides", 245), "Reject mismatched durations.");
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
}
finally { Directory.Delete(temp, true); }
Console.WriteLine("Core checks passed (LRC, YRC, TTML, ranking, lookup, preferences).");

sealed class FixtureHttp : HttpMessageHandler
{
    protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
    {
        if (request.RequestUri?.Host != "lrclib.net") throw new InvalidOperationException("Unexpected provider.");
        return Task.FromResult(new HttpResponseMessage(HttpStatusCode.OK)
        {
            Content = new StringContent("""
                [{"trackName":"Under the stars","artistName":"Tides","duration":245,"syncedLyrics":"[00:02]Wrong duration"},
                 {"trackName":"Under the stars","artistName":"Tides","duration":191,"syncedLyrics":"[00:02]<00:02>Word <00:03>timing"}]
                """)
        });
    }
}
