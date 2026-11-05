using System.Text;
using System.Text.RegularExpressions;

namespace Lunalyr.Core;

public static partial class Pronunciation
{
    private static readonly string[] Initial = ["g", "kk", "n", "d", "tt", "r", "m", "b", "pp", "s", "ss", "", "j", "jj", "ch", "k", "t", "p", "h"];
    private static readonly string[] Vowel = ["a", "ae", "ya", "yae", "eo", "e", "yeo", "ye", "o", "wa", "wae", "oe", "yo", "u", "wo", "we", "wi", "yu", "eu", "ui", "i"];
    private static readonly string[] Head = ["", "g", "kk", "g", "n", "n", "n", "d", "r", "r", "r", "r", "r", "r", "r", "r", "m", "b", "b", "s", "ss", "ng", "j", "ch", "k", "t", "p", "h"];
    private static readonly string[] Tail = ["", "", "", "s", "", "j", "h", "", "", "g", "m", "b", "s", "t", "p", "h", "", "", "s", "", "", "", "", "", "", "", "", ""];
    [GeneratedRegex("[가-힣]+")] private static partial Regex Hangul();

    public static bool HasKorean(string text) => Hangul().IsMatch(text.Normalize());
    public static string Korean(string text) => Hangul().Replace(text.Normalize(), match => SpellKorean(match.Value));

    private static string SpellKorean(string word)
    {
        var result = new StringBuilder();
        var syllables = word.Select(character => character - '가').ToArray();
        string onset = Initial[syllables[0] / 588];
        string prior = "";
        for (int i = 0; i < syllables.Length; i++)
        {
            int current = syllables[i];
            int next = i + 1 < syllables.Length ? syllables[i + 1] : -1;
            string head = Head[current % 28], tail = Tail[current % 28];
            var (coda, nextOnset) = next >= 0
                ? Join(head, tail, Initial[next / 588], next % 588 / 28, word[i], word[i + 1])
                : (Final(head, tail, word[i]), "");
            result.Append(onset == "r" && prior == "l" ? "l" : onset).Append(Vowel[current % 588 / 28]).Append(coda);
            onset = nextOnset;
            prior = coda;
        }
        return result.ToString();
    }

    private static string Neutral(string value) => value switch
    {
        "g" or "kk" or "k" => "k", "r" => "l", "b" or "p" => "p",
        "n" or "m" or "ng" or "" => value, _ => "t"
    };

    private static string Aspirate(string value) => value switch
    {
        "g" or "kk" or "k" => "k", "b" or "p" => "p", "j" or "ch" => "ch", _ => "t"
    };

    private static string Final(string head, string tail, char syllable) => (head, tail) switch
    {
        ("r", "g" or "m" or "p") => Neutral(tail), ("r", "b") when syllable == '밟' => "p", _ => Neutral(head)
    };

    private static (string Coda, string Onset) Join(string head, string tail, string onset, int vowel, char current, char next)
    {
        if (head.Length == 0) return ("", onset);
        if (onset.Length == 0 && next == '잎') onset = "n";
        if (onset.Length == 0)
        {
            if (head == "ng") return ("ng", "");
            if (tail == "h") return ("", head);
            if (head == "r" && tail == "t" && vowel is 6 or 20) return ("l", "ch");
            if (tail.Length > 0) return (Neutral(head), tail);
            if (head == "h") return ("", "");
            if (vowel == 20 && head == "d") return ("", "j");
            if (vowel is 6 or 20 && head == "t") return ("", "ch");
            return ("", head);
        }
        if (head == "h" || tail == "h")
        {
            string basic = tail.Length == 0 ? "" : Neutral(head);
            return onset switch
            {
                "g" or "d" or "j" => (basic, Aspirate(onset)), "s" or "h" => (basic, onset),
                "n" => basic == "l" ? ("l", "r") : ("n", "n"),
                _ => Assimilate(basic.Length == 0 ? "t" : basic, onset)
            };
        }
        if (tail.Length > 0 && onset == "h") return tail == "m" ? ("m", "h") : (Neutral(head), Aspirate(tail));
        string single = (head, tail) switch
        {
            ("r", "g") => onset == "g" ? "r" : "g", ("r", "m" or "p") => tail,
            ("r", "b") when current == '밟' => "b", _ => head
        };
        string neutral = Neutral(single);
        if (onset == "h" && neutral is "k" or "p" or "t")
            return ("", (single is "d" or "t") && (vowel is 6 or 20) ? "ch" : Aspirate(single));
        return Assimilate(neutral, onset);
    }

    private static (string, string) Assimilate(string coda, string onset) => (coda, onset) switch
    {
        ("k", "m" or "n") => ("ng", onset), ("t", "m" or "n") => ("n", onset), ("p", "m" or "n") => ("m", onset),
        ("l", "n") or ("n" or "l", "r") => ("l", "r"), ("m" or "ng", "r") => (coda, "n"),
        ("k", "r") => ("ng", "n"), ("p", "r") => ("m", "n"), ("t", "r") => ("n", "n"), _ => (coda, onset)
    };

    private static readonly string KanaLetters = "ぁあぃいぅうぇえぉおかがきぎくぐけげこごさざしじすずせぜそぞただちぢっつづてでとどなにぬねのはばぱひびぴふぶぷへべぺほぼぽまみむめもゃやゅゆょよらりるれろゎわゐゑをんゔ";
    private static readonly string[] KanaSounds = ("a a i i u u e e o o ka ga ki gi ku gu ke ge ko go sa za shi ji su zu se ze so zo " +
        "ta da chi ji q tsu zu te de to do na ni nu ne no ha ba pa hi bi pi fu bu pu he be pe ho bo po " +
        "ma mi mu me mo ya ya yu yu yo yo ra ri ru re ro wa wa wi we o n vu").Split(' ');

    public static bool HasKana(string text) => text.Any(c => c is >= '\u3041' and <= '\u30fa');

    public static string Kana(string text)
    {
        var source = text.Normalize(NormalizationForm.FormKC).Select(c => c is >= 'ァ' and <= 'ヶ' ? (char)(c - 0x60) : c).ToArray();
        var output = new StringBuilder();
        bool doubleOnset = false;
        for (int i = 0; i < source.Length; i++)
        {
            char c = source[i];
            if (c == 'っ') { doubleOnset = true; continue; }
            if (c == 'ー' && output.Length > 0 && "aeiou".Contains(output[^1])) { output.Append(output[^1]); continue; }
            int index = KanaLetters.IndexOf(c);
            if (index < 0) { output.Append(c); doubleOnset = false; continue; }
            string letters = KanaSounds[index];
            if (i + 1 < source.Length && source[i + 1] is 'ゃ' or 'ゅ' or 'ょ' && letters.EndsWith('i'))
            {
                char vowel = source[++i] == 'ゃ' ? 'a' : source[i] == 'ゅ' ? 'u' : 'o';
                letters = letters[..^1] + (letters is "shi" or "chi" or "ji" ? "" : "y") + vowel;
            }
            if (doubleOnset) { output.Append(letters.StartsWith("ch", StringComparison.Ordinal) ? 't' : letters[0]); doubleOnset = false; }
            output.Append(letters);
            if (c == 'ん' && i + 1 < source.Length && "あいうえおやゆよ".Contains(source[i + 1])) output.Append('\'');
        }
        return output.ToString();
    }
}
