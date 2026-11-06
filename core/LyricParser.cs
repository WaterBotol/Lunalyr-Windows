using System.Globalization;
using System.Text;
using System.Text.RegularExpressions;
using System.Xml;
using System.Xml.Linq;

namespace Lunalyr.Core;

public static partial class LyricParser
{
    [GeneratedRegex(@"\[(\d{1,4}:\d{2}(?:[.:]\d{1,3})?)\]")]
    private static partial Regex LineStamp();
    [GeneratedRegex(@"<(\d{1,4}:\d{2}(?:[.:]\d{1,3})?)>")]
    private static partial Regex WordStamp();
    [GeneratedRegex(@"(?im)^\[offset:\s*([+-]?\d+)\s*\]")]
    private static partial Regex Offset();
    [GeneratedRegex(@"^\[(\d+),(\d+)\](.*)$")]
    private static partial Regex YrcLine();
    [GeneratedRegex(@"\((\d+),(\d+),\d+\)(.*?)(?=\(\d+,\d+,\d+\)|$)")]
    private static partial Regex YrcWord();
    [GeneratedRegex(@"\s+")]
    private static partial Regex Space();
    [GeneratedRegex(@"^\[([a-z]+)\s*:\s*(.*?)\]", RegexOptions.IgnoreCase)]
    private static partial Regex Tag();
    [GeneratedRegex(@"^\s*[（(][^）)]*[）)]\s+|\s+[（(][^）)]*[）)]\s*$")]
    private static partial Regex ParentheticalVoice();

    private static double Number(string? value) =>
        double.TryParse(value, NumberStyles.Float, CultureInfo.InvariantCulture, out double number) && double.IsFinite(number)
            ? number : 0;
    private static double Clock(string value)
    {
        string[] parts = value.Split(':');
        return parts.Length switch
        {
            2 => 60 * Number(parts[0]) + Number(parts[1]),
            3 when parts[1].Contains('.') => 3600 * Number(parts[0]) + 60 * Number(parts[1]) + Number(parts[2]),
            3 => 60 * Number(parts[0]) + Number(parts[1] + "." + parts[2]),
            _ => 0
        };
    }

    public static Dictionary<string, string> Tags(string content)
    {
        var tags = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        foreach (string row in content.TrimStart('\uFEFF').Split('\n').Take(35))
        {
            Match tag = Tag().Match(row.Trim());
            if (tag.Success) tags.TryAdd(tag.Groups[1].Value, tag.Groups[2].Value);
        }
        return tags;
    }

    public static Line[] Parse(string content, string extension, double duration) => extension.ToLowerInvariant() switch
    {
        ".ttml" or ".xml" => Ttml(content, duration),
        ".yrc" => Yrc(content),
        ".txt" => Plain(content),
        ".yaml" or ".yml" => Yaml(content, duration),
        _ => Lrc(content, duration)
    };

    public static Line[] Plain(string content) => content.Split('\n')
        .Select(s => s.Trim()).Where(s => s.Length > 0)
        .Select(s => new Line(s, null, null, [])).ToArray();

    public static Line[] Lrc(string content, double duration)
    {
        double offset = Number(Offset().Match(content).Groups[1].Value) / 1000;
        var lines = new List<Line>();
        long expansion = 0;
        foreach (string row in content.TrimStart('\uFEFF').Split('\n'))
        {
            var stamps = LineStamp().Matches(row);
            if (stamps.Count == 0) continue;
            string text = row[(stamps[^1].Index + stamps[^1].Length)..].Trim();
            expansion += (long)stamps.Count * (text.Length + 1);
            if (expansion > 2_000_000) break;
            var wordStamps = WordStamp().Matches(text);
            string plain = WordStamp().Replace(text, "").Trim();
            double original = Clock(stamps[0].Groups[1].Value);
            foreach (Match stamp in stamps)
            {
                double time = Clock(stamp.Groups[1].Value);
                double shift = offset + time - original;
                var words = new List<Word>();
                string prefix = wordStamps.Count == 0 ? "" : text[..wordStamps[0].Index].Trim();
                if (prefix.Length > 0) words.Add(new Word(prefix, Math.Max(0, time + offset), Math.Max(0, Clock(wordStamps[0].Groups[1].Value) + shift)));
                for (int i = 0; i < wordStamps.Count; ++i)
                {
                    Match word = wordStamps[i];
                    Match? next = i + 1 < wordStamps.Count ? wordStamps[i + 1] : null;
                    string segment = text[(word.Index + word.Length)..(next?.Index ?? text.Length)].Trim();
                    if (segment.Length == 0) continue;
                    double start = Math.Max(0, Clock(word.Groups[1].Value) + shift);
                    double end = next is null ? start : Math.Max(start, Clock(next.Groups[1].Value) + shift);
                    words.Add(new Word(segment, start, end));
                }
                lines.Add(new Line(plain, Math.Max(0, time + offset), null, words.ToArray()));
            }
        }
        return FinalizeLines(lines, duration);
    }

    public static Line[] Yrc(string content)
    {
        var lines = new List<Line>();
        foreach (string row in content.TrimStart('\uFEFF').Split('\n'))
        {
            Match match = YrcLine().Match(row.Trim());
            if (!match.Success) continue;
            double start = Number(match.Groups[1].Value) / 1000;
            double end = start + Number(match.Groups[2].Value) / 1000;
            var segments = YrcWord().Matches(match.Groups[3].Value);
            var words = segments.Select(word =>
            {
                double time = Number(word.Groups[1].Value) / 1000;
                return new Word(word.Groups[3].Value.Trim(), time, time + Number(word.Groups[2].Value) / 1000);
            }).Where(word => word.Text.Length > 0 && word.End > word.Start).ToArray();
            string text = Space().Replace(string.Concat(segments.Select(word => word.Groups[3].Value)).Trim(), " ");
            if (text.Length > 0) lines.Add(new Line(text, start, end, words));
        }
        return FinalizeLines(lines, 0);
    }

    public static Line[] Ttml(string content, double duration)
    {
        using var input = new StringReader(content);
        using var reader = XmlReader.Create(input, new XmlReaderSettings
        {
            DtdProcessing = DtdProcessing.Prohibit,
            XmlResolver = null,
            MaxCharactersInDocument = 4_000_000
        });
        var xml = XDocument.Load(reader, LoadOptions.PreserveWhitespace);
        double frameRate = Number(Attribute(xml.Root!, "frameRate"));
        if (frameRate <= 0) frameRate = 30;
        double tickRate = Number(Attribute(xml.Root!, "tickRate"));
        if (tickRate <= 0) tickRate = 1;
        double? Time(string? raw)
        {
            if (string.IsNullOrWhiteSpace(raw)) return null;
            string[] parts = raw.Split(':');
            return parts.Length switch
            {
                4 => 3600 * Number(parts[0]) + 60 * Number(parts[1]) + Number(parts[2]) + Number(parts[3]) / frameRate,
                3 => 3600 * Number(parts[0]) + 60 * Number(parts[1]) + Number(parts[2]),
                2 => Clock(raw),
                _ when raw.EndsWith("ms", StringComparison.Ordinal) => Number(raw[..^2]) / 1000,
                _ when raw.EndsWith('s') => Number(raw[..^1]),
                _ when raw.EndsWith('m') => Number(raw[..^1]) * 60,
                _ when raw.EndsWith('h') => Number(raw[..^1]) * 3600,
                _ when raw.EndsWith('f') => Number(raw[..^1]) / frameRate,
                _ when raw.EndsWith('t') => Number(raw[..^1]) / tickRate,
                _ => null
            };
        }

        var lines = new List<Line>();
        foreach (var paragraph in xml.Descendants().Where(e => e.Name.LocalName == "p"))
        {
            double? start = Time(Attribute(paragraph, "begin"));
            double? end = Time(Attribute(paragraph, "end")) ?? (start + Time(Attribute(paragraph, "dur")));
            var tokens = new List<Word>();
            var reading = new List<string>();
            var text = new System.Text.StringBuilder();
            void Visit(XElement parent)
            {
                foreach (var node in parent.Nodes())
                {
                    if (node is XText literal) text.Append(literal.Value);
                    if (node is not XElement child) continue;
                    string? role = Attribute(child, "role");
                    if (role is "x-bg" or "x-translation") continue;
                    if (role is "x-roman" or "x-romanization") { reading.Add(child.Value.Trim()); continue; }
                    if (child.Name.LocalName == "br") { text.Append(' '); continue; }
                    double? begin = Time(Attribute(child, "begin"));
                    double? stop = Time(Attribute(child, "end")) ?? (begin + Time(Attribute(child, "dur")));
                    if (begin.HasValue && stop > begin && child.Value.Trim().Length > 0)
                        tokens.Add(new Word(child.Value.Trim(), begin.Value, stop.Value));
                    Visit(child);
                }
            }
            Visit(paragraph);
            string plain = Space().Replace(text.ToString().Trim(), " ");
            if (plain.Length > 0)
                lines.Add(new Line(plain, start ?? tokens.FirstOrDefault()?.Start, end, tokens.ToArray(),
                    reading.Count == 0 ? null : string.Join(" ", reading)));
        }
        return FinalizeLines(lines, duration);
    }

    private static string? Attribute(XElement element, string name) =>
        element.Attributes().FirstOrDefault(attribute => attribute.Name.LocalName == name)?.Value;

    internal static Line[] FinalizeLines(List<Line> source, double duration)
    {
        var rows = source.OrderBy(row => row.Start ?? double.PositiveInfinity).ToArray();
        var result = new List<Line>();
        var nextStarts = new double?[rows.Length];
        for (int i = rows.Length - 2; i >= 0; i--)
            nextStarts[i] = rows[i + 1].Start > rows[i].Start ? rows[i + 1].Start : nextStarts[i + 1];
        for (int i = 0; i < rows.Length; i++)
        {
            Line line = rows[i];
            if (line.Text.Length == 0) continue;
            if (!line.Start.HasValue) { result.Add(line); continue; }
            double start = line.Start.Value;
            double? next = nextStarts[i];
            bool inferredEnd = !(line.End > start) && next.HasValue;
            double end = line.End > start ? line.End.Value : next ?? (duration > start ? duration : start + 5);
            Word[] words = line.Words.Where(word => word.Start >= start - .25 && word.Start < end)
                .OrderBy(word => word.Start).ToArray();
            for (int w = 0; w < words.Length; w++)
            {
                Word word = words[w];
                double wordEnd = word.End > word.Start ? word.End : w + 1 < words.Length ? words[w + 1].Start : end;
                words[w] = word with { End = Math.Clamp(wordEnd, word.Start, end) };
            }
            var current = line with { End = end, Words = words, EndFromNextLine = inferredEnd || line.EndFromNextLine };
            if (result.LastOrDefault()?.Start == line.Start)
            {
                Line previous = result[^1];
                if (previous.Words.Length == 0 && words.Length > 0 && TimingKey(previous.Text) == TimingKey(line.Text))
                    result[^1] = current with { Reading = current.Reading ?? previous.Reading };
                else if (previous.Text != line.Text && previous.Reading is null)
                    result[^1] = previous with { Reading = line.Text };
            }
            else result.Add(current);
        }
        return RemoveMixedTimingCopies(result);
    }

    private static Line[] RemoveMixedTimingCopies(List<Line> lines)
    {
        if (!lines.Any(line => line.Words.Length > 0) || !lines.Any(line => line.Words.Length == 0)) return lines.ToArray();
        var removed = new HashSet<int>();
        for (int i = 0; i < lines.Count; i++)
        {
            Line timed = lines[i];
            if (timed.Start is not { } start || timed.Words.Length == 0) continue;
            string key = TimingKey(timed.Text);
            string main = TimingKey(ParentheticalVoice().Replace(timed.Text, ""));
            double vocalEnd = timed.Words.Max(word => word.End);
            int duplicate = -1;
            double distance = double.PositiveInfinity;
            foreach (int j in new[] { i - 1, i + 1 })
            {
                if (j < 0 || j >= lines.Count || removed.Contains(j)) continue;
                Line fallback = lines[j];
                if (fallback.Start is not { } a || fallback.End is not { } b || fallback.Words.Length != 0) continue;
                string other = TimingKey(fallback.Text);
                if (other != key && other != main) continue;
                double difference = Math.Abs(a - start);
                bool multiple = Space().IsMatch(fallback.Text) || fallback.Text.EnumerateRunes().Count() > 1
                    && fallback.Text.Any(character => character is >= '\u3040' and <= '\u30ff' or >= '\u3400' and <= '\u9fff');
                if (difference < distance && LyricRules.MixedTimingCopy(a, b, fallback.EndFromNextLine ? 1 : 0,
                    multiple ? 1 : 0, start, vocalEnd) != 0)
                {
                    distance = difference;
                    duplicate = j;
                }
            }
            if (duplicate < 0) continue;
            removed.Add(duplicate);
            if (timed.Reading is null) lines[i] = timed with { Reading = lines[duplicate].Reading };
        }
        if (removed.Count == 0) return lines.ToArray();
        var result = new List<Line>(lines.Count - removed.Count);
        int prior = -1;
        for (int i = 0; i < lines.Count; i++)
        {
            if (removed.Contains(i)) continue;
            if (prior >= 0 && i > prior + 1 && lines[prior].EndFromNextLine && lines[i].Start is { } next
                && Math.Abs(lines[prior].End!.Value - lines[prior + 1].Start!.Value) < .05)
                result[^1] = lines[prior] with { End = next };
            result.Add(lines[i]);
            prior = i;
        }
        return result.ToArray();
    }

    private static string TimingKey(string value)
    {
        string key = string.Concat(value.Normalize(NormalizationForm.FormKC).ToLowerInvariant()
            .EnumerateRunes().Where(Rune.IsLetterOrDigit).Select(rune => rune.ToString()));
        return key.Length > 0 ? key : value;
    }
}
