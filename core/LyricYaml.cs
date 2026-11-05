using System.Globalization;
using YamlDotNet.Core;
using YamlDotNet.Core.Events;
using YamlDotNet.RepresentationModel;

namespace Lunalyr.Core;

public static partial class LyricParser
{
    private static Line[] Yaml(string content, double duration)
    {
        if (content.Length > 1_000_000) throw new FormatException("YAML lyrics may be up to 1 MB.");
        using var input = new StringReader(content);
        var parser = new Parser(input);
        int nesting = 0, nodes = 0;
        while (parser.MoveNext())
        {
            if (parser.Current is AnchorAlias) throw new FormatException("YAML lyric aliases are unsupported.");
            if (parser.Current is MappingStart or SequenceStart) nesting++;
            if (parser.Current is MappingEnd or SequenceEnd) nesting--;
            if (++nodes > 80_000 || nesting > 12) throw new FormatException("YAML lyric structure is too large.");
        }
        using var validated = new StringReader(content);
        var stream = new YamlStream();
        stream.Load(validated);
        if (stream.Documents.Count != 1) return [];
        var root = stream.Documents[0].RootNode;
        double offset = (Seconds(Field(root, "metadata"), "offset_ms") ?? 0);
        var rows = Items(Field(root, "lines")).Select(node => (
            Node: node, Start: Seconds(node, "start_ms"), End: Seconds(node, "end_ms"), Text: Text(node, "text").Trim()
        )).Where(row => row.Start is >= 0 && row.Text.Length > 0).OrderBy(row => row.Start).ToArray();

        var result = new List<Line>();
        for (int i = 0; i < rows.Length; i++)
        {
            var row = rows[i];
            double start = Math.Max(0, row.Start!.Value + offset);
            double end = (row.End ?? (i + 1 < rows.Length ? rows[i + 1].Start!.Value : row.Start.Value + 4)) + offset;
            if (end <= start) continue;
            var tokens = Items(Field(row.Node, "words")).Select(node => (
                Text: Text(node, "text").Trim(), Start: Seconds(node, "start_ms"), End: Seconds(node, "end_ms")
            )).ToArray();
            var words = new List<Word>();
            double prior = start;
            for (int w = 0; w < tokens.Length; w++)
            {
                var token = tokens[w];
                double? a = token.Start + offset;
                double? b = (token.End ?? (w + 1 < tokens.Length ? tokens[w + 1].Start : end - offset)) + offset;
                if (a is null || b is null || a < prior || b <= a || b > end || token.Text.Length == 0)
                {
                    words.Clear();
                    break;
                }
                words.Add(new Word(token.Text, a.Value, b.Value));
                prior = b.Value;
            }
            if (string.Concat(words.Select(word => WithoutSpaces(word.Text))) != WithoutSpaces(row.Text)) words.Clear();
            result.Add(new Line(row.Text, start, end, words.ToArray()));
        }
        return FinalizeLines(result, duration);
    }

    private static YamlNode? Field(YamlNode? node, string key) =>
        node is YamlMappingNode map && map.Children.TryGetValue(new YamlScalarNode(key), out var value) ? value : null;
    private static IEnumerable<YamlNode> Items(YamlNode? node) => node is YamlSequenceNode seq ? seq.Children : [];
    private static string Text(YamlNode? node, string key) => (Field(node, key) as YamlScalarNode)?.Value ?? "";
    private static double? Seconds(YamlNode? node, string key) =>
        double.TryParse(Text(node, key), NumberStyles.Float, CultureInfo.InvariantCulture, out double n) && double.IsFinite(n)
            ? n / 1000 : null;
    private static string WithoutSpaces(string value) => string.Concat(value.Where(c => !char.IsWhiteSpace(c)));
}
