using Lunalyr.Core;
using Windows.Globalization;

namespace Lunalyr;

internal static class WindowsReadings
{
    internal static Lyrics Add(Lyrics lyrics)
    {
        bool japanese = lyrics.Lines.Any(line => Pronunciation.HasKana(line.Text));
        return lyrics with { Lines = lyrics.Lines.Select(line => Add(line, japanese)).ToArray() };
    }

    private static Line Add(Line line, bool japanese)
    {
        if (line.Reading is not null || line.Text.Length > 1000) return line;
        if (Pronunciation.HasKorean(line.Text)) return line with { Reading = Pronunciation.Korean(line.Text) };
        if (!japanese || !line.Text.Any(c => c is >= '\u3041' and <= '\u9fff')) return line;
        try
        {
            var words = JapanesePhoneticAnalyzer.GetWords(line.Text);
            string result = string.Concat(words.Select(word => (word.IsPhraseStart ? " " : "") +
                (word.DisplayText == "は" && !word.IsPhraseStart ? "wa" : Pronunciation.Kana(word.YomiText ?? word.DisplayText)))).Trim();
            return result.Length == 0 || result == line.Text ? line : line with { Reading = result };
        }
        catch (Exception error) when (error is System.Runtime.InteropServices.COMException or ArgumentException or TypeLoadException)
        {
            string result = Pronunciation.Kana(line.Text);
            return result == line.Text ? line : line with { Reading = result };
        }
    }
}
