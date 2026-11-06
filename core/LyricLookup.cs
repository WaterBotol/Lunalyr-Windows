using System.Globalization;
using System.IO.Compression;
using System.Net;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;

namespace Lunalyr.Core;

public sealed class LyricLookup(HttpClient http)
{
    public static readonly string[] Formats = [".lrc", ".yrc", ".ttml", ".xml", ".txt", ".yaml", ".yml"];
    public static readonly Dictionary<string, string> Names = new()
    {
        ["local"] = "Local lyrics", ["lyricsplus"] = "Lyrics+", ["netease"] = "NetEase",
        ["lrclib"] = "LRCLIB", ["qq"] = "QQ Music", ["kugou"] = "Kugou", ["kuwo"] = "Kuwo", ["ovh"] = "lyrics.ovh"
    };

    public async Task<Lyrics[]> Find(Track track, Preferences preferences, string localFolder, CancellationToken token,
        Action<Lyrics[]>? progress = null)
    {
        string[] sources = preferences.Sources.Where(source => !preferences.DisabledSources.Contains(source)).Distinct().ToArray();
        if (preferences.LookupPolicy == "order" && !preferences.PreferWords)
        {
            foreach (string source in sources)
            {
                var match = await FindOne(source, track, preferences.LyricsFolder ?? localFolder, token);
                if (match is not null) return [match];
            }
            return [];
        }
        var folder = preferences.LyricsFolder ?? localFolder;
        var pending = sources.Select((source, order) => FindRanked(source, order)).ToList();
        var matches = new List<(Lyrics Match, int Order)>();
        Lyrics[] ordered = [];
        while (pending.Count > 0)
        {
            var completed = await Task.WhenAny(pending);
            pending.Remove(completed);
            var (match, order) = await completed;
            if (match is null) continue;
            matches.Add((match, order));
            ordered = matches.OrderByDescending(row => row.Match.Quality).ThenBy(row => row.Order)
                .Select(row => row.Match).ToArray();
            progress?.Invoke(ordered);
        }
        return ordered;

        async Task<(Lyrics? Match, int Order)> FindRanked(string source, int order) =>
            (await FindOne(source, track, folder, token), order);
    }

    private async Task<Lyrics?> FindOne(string source, Track track, string localFolder, CancellationToken token)
    {
        using var deadline = CancellationTokenSource.CreateLinkedTokenSource(token);
        deadline.CancelAfter(TimeSpan.FromSeconds(9));
        var ct = deadline.Token;
        try
        {
            return source switch
            {
                "local" => await Task.Run(() => Local(track, localFolder, ct), ct),
                "lyricsplus" => await LyricsPlus(track, ct),
                "netease" => await NetEase(track, ct),
                "lrclib" => await LrcLib(track, ct),
                "qq" => await Qq(track, ct),
                "kugou" => await Kugou(track, ct),
                "kuwo" => await Kuwo(track, ct),
                "ovh" when track.Artist.Length > 0 => await Ovh(track, ct),
                _ => null
            };
        }
        catch (OperationCanceledException) when (!token.IsCancellationRequested) { return null; }
        catch (Exception error) when (error is HttpRequestException or JsonException or IOException
                                      or FormatException or System.Xml.XmlException or UnauthorizedAccessException)
        {
            return null;
        }
    }

    public static Lyrics? ReadFile(string path, double duration, string source = "local")
    {
        if (!Formats.Contains(Path.GetExtension(path), StringComparer.OrdinalIgnoreCase))
            throw new FormatException("Choose an LRC, YRC, TTML, YAML or text lyric file.");
        using var stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite);
        if (stream.Length > 4_000_000) throw new FormatException("Lyric files may be up to 4 MB.");
        using var reader = new StreamReader(stream, Encoding.UTF8, detectEncodingFromByteOrderMarks: true);
        try { return Make(source, LyricParser.Parse(reader.ReadToEnd(), Path.GetExtension(path), duration)); }
        catch (YamlDotNet.Core.YamlException error) { throw new FormatException("Invalid YAML lyrics.", error); }
    }

    private static Lyrics? Local(Track track, string path, CancellationToken token)
    {
        if (!Directory.Exists(path)) return null;
        var files = Directory.EnumerateFiles(path, "*", new EnumerationOptions
        {
            RecurseSubdirectories = true, MaxRecursionDepth = 2, IgnoreInaccessible = true,
            AttributesToSkip = FileAttributes.ReparsePoint | FileAttributes.Hidden
        }).Take(3000);
        string title = Normalize(track.Title), artist = Normalize(track.Artist);
        var candidates = new List<Lyrics>();
        foreach (string file in files)
        {
            token.ThrowIfCancellationRequested();
            if (!Formats.Contains(Path.GetExtension(file), StringComparer.OrdinalIgnoreCase)
                || file.EndsWith(".roma.lrc", StringComparison.OrdinalIgnoreCase)) continue;
            string name = Normalize(Path.GetFileNameWithoutExtension(file));
            if (name != title && name != artist + title && name != title + artist) continue;
            try
            {
                var match = ReadFile(file, track.Duration);
                if (match is null) continue;
                string readingFile = Path.ChangeExtension(file, ".roma.lrc");
                if (File.Exists(readingFile) && ReadFile(readingFile, track.Duration) is { } reading)
                    match = match with { Lines = WithReadings(match.Lines, reading.Lines) };
                candidates.Add(match);
            }
            catch (Exception error) when (error is IOException or FormatException or System.Xml.XmlException or UnauthorizedAccessException) { }
        }
        return candidates.OrderByDescending(match => match.Quality).FirstOrDefault();
    }

    private async Task<Lyrics?> LrcLib(Track track, CancellationToken ct)
    {
        const string api = "https://lrclib.net/api/";
        string title = LookupTitle(track.Title, track.Artist);
        var fields = new List<(string, string)> { ("track_name", title) };
        if (track.Artist.Length > 0) fields.Add(("artist_name", track.Artist));
        var rows = new List<JsonNode?>();
        if (track.Artist.Length > 0 && track.Duration > 0)
        {
            var exactFields = new List<(string, string)>(fields) { ("duration", Invariant(track.Duration)) };
            if (track.Album.Length > 0) exactFields.Add(("album_name", track.Album));
            using var exactDeadline = CancellationTokenSource.CreateLinkedTokenSource(ct);
            exactDeadline.CancelAfter(TimeSpan.FromSeconds(2));
            try { rows.Add(await Get(api + "get", exactDeadline.Token, null, exactFields.ToArray())); }
            catch (OperationCanceledException) when (!ct.IsCancellationRequested) { }
            catch (Exception error) when (error is HttpRequestException or JsonException or IOException) { }
        }
        var exact = Best(rows);
        if (exact?.Quality == 4) return exact;
        await Search(fields.ToArray());
        var best = Best(rows);
        if (best?.Quality >= 2 || track.Artist.Length == 0) return best;
        await Search([("q", Query(track))]);
        return Best(rows);

        async Task Search((string, string)[] parameters)
        {
            using var searchDeadline = CancellationTokenSource.CreateLinkedTokenSource(ct);
            searchDeadline.CancelAfter(TimeSpan.FromSeconds(3));
            try { rows.AddRange(Array(await Get(api + "search", searchDeadline.Token, null, parameters))); }
            catch (OperationCanceledException) when (!ct.IsCancellationRequested) { }
            catch (Exception error) when (error is HttpRequestException or JsonException or IOException) { }
        }

        Lyrics? Best(IEnumerable<JsonNode?> entries) => entries
            .Where(row => Match(track, String(row, "trackName"), String(row, "artistName"), Number(row, "duration")))
            .OrderBy(row => Difference(track.Duration, Number(row, "duration")))
            .ThenByDescending(row => Normalize(String(row, "albumName")) == Normalize(track.Album))
            .Select(Read).OfType<Lyrics>().OrderByDescending(row => row.Quality).FirstOrDefault();

        Lyrics? Read(JsonNode? row)
        {
            if (Flag(row, "instrumental")) return new Lyrics("lrclib", [], true);
            if (String(row, "lyricsfile") is { Length: > 0 } file)
            {
                try
                {
                    if (Make("lrclib", LyricParser.Parse(file, ".yaml", track.Duration)) is { } timed) return timed;
                }
                catch (Exception error) when (error is YamlDotNet.Core.YamlException or FormatException) { }
            }
            return Make("lrclib", LyricParser.Lrc(String(row, "syncedLyrics"), track.Duration))
                ?? Make("lrclib", LyricParser.Plain(String(row, "plainLyrics")));
        }
    }

    private async Task<Lyrics?> LyricsPlus(Track track, CancellationToken ct)
    {
        string[] hosts = ["https://lyricsplus.binimum.org", "https://lyricsplus.prjktla.workers.dev", "https://lyricsplus.prjktla.my.id"];
        var fields = new List<(string, string)> { ("title", LookupTitle(track.Title, track.Artist)), ("artist", track.Artist) };
        if (track.Album.Length > 0) fields.Add(("album", track.Album));
        if (track.Duration > 0) fields.Add(("duration", track.Duration.ToString("0.###", CultureInfo.InvariantCulture)));
        foreach (string host in hosts)
        {
            using var mirrorDeadline = CancellationTokenSource.CreateLinkedTokenSource(ct);
            mirrorDeadline.CancelAfter(TimeSpan.FromSeconds(3));
            JsonNode? result;
            try
            {
                result = await Get(host + "/v2/lyrics/get", mirrorDeadline.Token, null, fields.ToArray());
            }
            catch (OperationCanceledException) when (!ct.IsCancellationRequested) { continue; }
            catch (Exception error) when (error is HttpRequestException or JsonException or IOException) { continue; }
            var meta = Value(result, "metadata");
            if (String(meta, "title") is { Length: > 0 } title && !Match(track, title, String(meta, "artist"), 0)) continue;
            var lines = Array(Value(result, "lyrics")).Where(row => Value(row, "time") is not null).Select(row =>
            {
                double start = Number(row, "time") / 1000;
                double end = start + Number(row, "duration") / 1000;
                var words = new List<Word>();
                bool priorSpace = true;
                foreach (var syllable in Array(Value(row, "syllabus")))
                {
                    string raw = String(syllable, "text");
                    if (string.IsNullOrWhiteSpace(raw)) { priorSpace = true; continue; }
                    double time = Number(syllable, "time") / 1000;
                    var word = new Word(raw.Trim(), time, Math.Max(time + .05, time + Number(syllable, "duration") / 1000));
                    bool latin = raw.Trim().All(c => c <= 0x024f);
                    if (!priorSpace && latin && !char.IsWhiteSpace(raw[0]) && words.Count > 0)
                        words[^1] = words[^1] with { Text = words[^1].Text + word.Text, End = word.End };
                    else words.Add(word);
                    priorSpace = char.IsWhiteSpace(raw[^1]);
                }
                return new Line(String(row, "text").Trim(), start, end, words.ToArray());
            }).Where(row => row.Text.Length > 0).ToList();
            if (Make("lyricsplus", LyricParser.FinalizeLines(lines, track.Duration)) is { } match) return match;
        }
        return null;
    }

    private async Task<Lyrics?> NetEase(Track track, CancellationToken ct)
    {
        const string referer = "https://music.163.com/";
        var search = await Get(referer + "api/search/get", ct, referer, ("s", Query(track)), ("type", "1"), ("limit", "8"));
        var songs = Array(Value(Value(search, "result"), "songs"))
            .Where(row => Array(Value(row, "alias")).Select(alias => alias?.ToString() ?? "").Prepend(String(row, "name"))
                .Any(title => Match(track, title, Artists(row, "artists"), Number(row, "duration") / 1000)))
            .OrderBy(row => Difference(track.Duration, Number(row, "duration") / 1000)).Take(2);
        foreach (var row in songs)
        {
            var result = await Get(referer + "api/song/lyric", ct, referer, ("id", String(row, "id")), ("lv", "1"),
                ("kv", "1"), ("yv", "1"), ("rv", "-1"));
            Line[] words = LyricParser.Yrc(String(Value(result, "yrc"), "lyric"));
            Line[] lines = LyricParser.Lrc(String(Value(result, "lrc"), "lyric"), track.Duration);
            // Preserve line-timed gaps when YRC covers only parts of the track.
            if (words.Length > 0)
                lines = words.Concat(lines.Where(line => words.All(word => Math.Abs(word.Start!.Value - line.Start!.Value) > .32
                    && (line.Start < word.Start || line.Start >= word.End)))).OrderBy(line => line.Start).ToArray();
            Line[] readings = LyricParser.Lrc(String(Value(result, "romalrc"), "lyric"), track.Duration);
            if (Make("netease", WithReadings(lines, readings)) is { } match) return match;
        }
        return null;
    }

    private async Task<Lyrics?> Qq(Track track, CancellationToken ct)
    {
        const string referer = "https://y.qq.com/";
        var search = await Get("https://c.y.qq.com/soso/fcgi-bin/client_search_cp", ct, referer, ("format", "json"),
            ("p", "1"), ("n", "8"), ("w", Query(track)));
        var songs = Array(Value(Value(Value(search, "data"), "song"), "list"))
            .Where(row => Match(track, String(row, "songname"), Artists(row, "singer"), Number(row, "interval")))
            .OrderBy(row => Difference(track.Duration, Number(row, "interval"))).Take(3);
        foreach (var row in songs)
        {
            var result = await Get("https://c.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg", ct, referer,
                ("songmid", String(row, "songmid")), ("format", "json"), ("nobase64", "1"), ("g_tk", "5381"));
            string content = String(result, "lyric");
            if (content.Length > 0 && !content.Contains('[')) content = FromBase64(content);
            if (Make("qq", LyricParser.Lrc(WebUtility.HtmlDecode(content), track.Duration)) is { } match) return match;
        }
        return null;
    }

    private async Task<Lyrics?> Kugou(Track track, CancellationToken ct)
    {
        var search = await Get("https://mobileservice.kugou.com/api/v3/search/song", ct, null, ("keyword", Query(track)),
            ("page", "1"), ("pagesize", "8"), ("format", "json"));
        var songs = Array(Value(Value(search, "data"), "info")).Where(row =>
            {
                string[] fileName = String(row, "filename").Split(" - ", 2);
                string title = String(row, "songname"), artist = String(row, "singername");
                return Match(track, title.Length > 0 ? title : fileName[^1], artist.Length > 0 ? artist : fileName[0], Number(row, "duration"));
            }).OrderBy(row => Difference(track.Duration, Number(row, "duration"))).Take(2);
        foreach (var row in songs)
        {
            var result = await Get("https://lyrics.kugou.com/search", ct, null, ("ver", "1"), ("man", "yes"), ("client", "pc"),
                ("keyword", Query(track)), ("duration", Invariant(track.Duration * 1000)), ("hash", String(row, "hash")));
            foreach (var item in Array(Value(result, "candidates")).Take(2))
            {
                var payload = await Get("https://lyrics.kugou.com/download", ct, null, ("ver", "1"), ("client", "pc"),
                    ("id", String(item, "id")), ("accesskey", String(item, "accesskey")), ("fmt", "lrc"), ("charset", "utf8"));
                if (Make("kugou", LyricParser.Lrc(FromBase64(String(payload, "content")), track.Duration)) is { } match) return match;
            }
        }
        return null;
    }

    private async Task<Lyrics?> Kuwo(Track track, CancellationToken ct)
    {
        const string referer = "https://www.kuwo.cn/";
        var search = await Get("https://search.kuwo.cn/r.s", ct, referer, ("all", Query(track)), ("ft", "music"),
            ("itemset", "web_2013"), ("client", "kt"), ("pn", "0"), ("rn", "8"), ("rformat", "json"),
            ("encoding", "utf8"), ("pcjson", "1"), ("vipver", "1"));
        var songs = Array(Value(search, "abslist"))
            .Where(row => Match(track, String(row, "SONGNAME"), String(row, "ARTIST"), Number(row, "DURATION")))
            .OrderBy(row => Difference(track.Duration, Number(row, "DURATION"))).Take(2);
        foreach (var row in songs)
        {
            var result = await Get(referer + "openapi/v1/www/lyric/getlyric", ct, referer,
                ("musicId", String(row, "MUSICRID").Split('_')[^1]));
            var rows = Array(Value(Value(result, "data"), "lrclist")).Select(line =>
            {
                double time = Number(line, "time");
                return FormattableString.Invariant($"[{(int)(time / 60):00}:{time % 60:00.00}]") + String(line, "lineLyric");
            });
            if (Make("kuwo", LyricParser.Lrc(string.Join("\n", rows), track.Duration)) is { } match) return match;
        }
        return null;
    }

    private async Task<Lyrics?> Ovh(Track track, CancellationToken ct)
    {
        string title = LookupTitle(track.Title, track.Artist);
        foreach (string artist in new[] { track.Artist, PrimaryArtist(track.Artist) }.Distinct())
        {
            var result = await Get("https://api.lyrics.ovh/v1/" + Escape(artist) + "/" + Escape(title), ct);
            if (Make("ovh", LyricParser.Plain(String(result, "lyrics"))) is { } match) return match;
        }
        return null;
    }

    private async Task<JsonNode?> Get(string url, CancellationToken ct, string? referer = null,
        params (string Name, string Value)[] parameters)
    {
        const int limit = 4_000_000;
        if (parameters.Length > 0) url += "?" + string.Join("&", parameters.Select(pair => Escape(pair.Name) + "=" + Escape(pair.Value)));
        using var request = new HttpRequestMessage(HttpMethod.Get, url);
        if (referer is not null) request.Headers.Referrer = new Uri(referer);
        request.Headers.UserAgent.ParseAdd("Lunalyr/0.2 (+https://github.com/WaterBotol/Lunalyr-Windows)");
        using var response = await http.SendAsync(request, HttpCompletionOption.ResponseHeadersRead, ct);
        if (!response.IsSuccessStatusCode || response.Content.Headers.ContentLength > limit) return null;
        await using var input = await response.Content.ReadAsStreamAsync(ct);
        using var buffer = new MemoryStream();
        await CopyBounded(input, buffer, limit, ct);
        byte[] bytes = buffer.ToArray();
        if (bytes is [0x1f, 0x8b, ..])
        {
            buffer.Position = 0;
            await using var gzip = new GZipStream(buffer, CompressionMode.Decompress, leaveOpen: true);
            using var decoded = new MemoryStream();
            await CopyBounded(gzip, decoded, limit, ct);
            bytes = decoded.ToArray();
        }
        return JsonNode.Parse(bytes);
    }

    private static async Task CopyBounded(Stream input, Stream output, int limit, CancellationToken ct)
    {
        byte[] block = new byte[8192];
        int count;
        while ((count = await input.ReadAsync(block, ct)) > 0)
        {
            if (output.Length + count > limit) throw new IOException("Lyric response too large.");
            await output.WriteAsync(block.AsMemory(0, count), ct);
        }
    }

    private static Line[] WithReadings(Line[] lines, Line[] readings) => lines.Select(line =>
    {
        if (line.Start is null || line.Reading is not null) return line;
        var reading = readings.FirstOrDefault(row => row.Start.HasValue && Math.Abs(row.Start.Value - line.Start.Value) < .3);
        return reading is null ? line : line with { Reading = reading.Text };
    }).ToArray();

    public static bool Match(Track track, string title, string artist, double duration)
    {
        string expected = Normalize(LookupTitle(track.Title, track.Artist));
        string candidate = Normalize(LookupTitle(title, artist));
        string expectedArtist = Normalize(track.Artist), candidateArtist = Normalize(artist);
        bool compareArtist = expectedArtist.Length > 0 && candidateArtist.Length > 0;
        bool artistMatches = expectedArtist == candidateArtist || compareArtist && ArtistParts(track.Artist).Intersect(ArtistParts(artist)).Any();
        return LyricRules.RecordingMatch(expected.Length > 0 && expected == candidate ? 1 : 0, artistMatches ? 1 : 0,
            compareArtist ? 1 : 0, 0, 1, track.Duration, duration) == 0;
    }

    private static string LookupTitle(string value, string artist)
    {
        string[] parts = value.Split(" - ", 2);
        if (parts.Length == 2 && artist.Length > 0 && ArtistParts(artist).Append(Normalize(artist)).Contains(Normalize(parts[0])))
            value = parts[1];
        // Keep version qualifiers such as Live, Remix and Radio Edit.
        return Regex.Replace(value,
            @"(?:\s*(?:[\[(](?:(?:feat\.?|ft\.?|featuring)\s+[^\])]+|(?:\d{4}\s+)?remaster(?:ed)?(?:\s+\d{4})?)[\])]|\s-\s(?:\d{4}\s+)?remaster(?:ed)?(?:\s+\d{4})?)\s*)+$",
            "", RegexOptions.IgnoreCase | RegexOptions.NonBacktracking).Trim();
    }
    private static string[] SplitArtists(string value) => Regex.Split(value,
        @"\s+(?:feat\.?|ft\.?|featuring|and|&|×|x|/)\s+|[,;，、]", RegexOptions.IgnoreCase);
    private static IEnumerable<string> ArtistParts(string value) => SplitArtists(value).Select(Normalize).Where(s => s.Length > 0);
    private static string PrimaryArtist(string value) => SplitArtists(value).FirstOrDefault(part => !string.IsNullOrWhiteSpace(part))?.Trim() ?? value;
    private static string Normalize(string value) => string.Concat(value.Normalize(NormalizationForm.FormKC).ToLowerInvariant()
        .EnumerateRunes().Where(Rune.IsLetterOrDigit).Select(rune => rune.ToString()));
    private static double Difference(double a, double b) => a <= 0 || b <= 0 ? double.MaxValue : Math.Abs(a - b);
    private static string Query(Track track) => (LookupTitle(track.Title, track.Artist) + " " + PrimaryArtist(track.Artist)).Trim();
    private static string Escape(string value) => Uri.EscapeDataString(value);
    private static string Invariant(double value) => Math.Round(value).ToString(CultureInfo.InvariantCulture);
    private static JsonNode? Value(JsonNode? node, string key) => node is JsonObject item ? item[key] : null;
    private static IEnumerable<JsonNode?> Array(JsonNode? node) => node is JsonArray list ? list : [];
    private static string String(JsonNode? node, string key) => Value(node, key) is JsonValue item ? item.ToString() : "";
    private static bool Flag(JsonNode? node, string key) => Value(node, key) is JsonValue item && item.TryGetValue(out bool value) && value;
    private static double Number(JsonNode? node, string key) => double.TryParse(String(node, key), CultureInfo.InvariantCulture, out double n)
        && double.IsFinite(n) ? n : 0;
    private static string Artists(JsonNode? node, string key) => string.Join(" & ", Array(Value(node, key)).Select(row => String(row, "name")));
    private static string FromBase64(string value) => Encoding.UTF8.GetString(Convert.FromBase64String(value));
    private static Lyrics? Make(string source, Line[] lines) => lines.Length == 0 ? null : new Lyrics(source, lines);
}
