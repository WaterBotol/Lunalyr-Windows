//! Kana to (modified) Hepburn romaji: digraphs (きゃ kya, しゃ sha), small
//! っ doubling the next consonant (っち tchi), ん as `n'` before vowels and
//! y, ー as a macron on the previous vowel, katakana extensions (ファ fa,
//! ティ ti, ヴ vu). Characters that are not kana pass through unchanged.

use unicode_normalization::UnicodeNormalization;

fn base(c: char) -> Option<&'static str> {
    Some(match c {
        'あ' => "a",
        'い' => "i",
        'う' => "u",
        'え' => "e",
        'お' => "o",
        'か' => "ka",
        'き' => "ki",
        'く' => "ku",
        'け' => "ke",
        'こ' => "ko",
        'さ' => "sa",
        'し' => "shi",
        'す' => "su",
        'せ' => "se",
        'そ' => "so",
        'た' => "ta",
        'ち' => "chi",
        'つ' => "tsu",
        'て' => "te",
        'と' => "to",
        'な' => "na",
        'に' => "ni",
        'ぬ' => "nu",
        'ね' => "ne",
        'の' => "no",
        'は' => "ha",
        'ひ' => "hi",
        'ふ' => "fu",
        'へ' => "he",
        'ほ' => "ho",
        'ま' => "ma",
        'み' => "mi",
        'む' => "mu",
        'め' => "me",
        'も' => "mo",
        'や' => "ya",
        'ゆ' => "yu",
        'よ' => "yo",
        'ら' => "ra",
        'り' => "ri",
        'る' => "ru",
        'れ' => "re",
        'ろ' => "ro",
        'わ' => "wa",
        'ゐ' => "i",
        'ゑ' => "e",
        'を' => "o",
        'ん' => "n",
        'が' => "ga",
        'ぎ' => "gi",
        'ぐ' => "gu",
        'げ' => "ge",
        'ご' => "go",
        'ざ' => "za",
        'じ' => "ji",
        'ず' => "zu",
        'ぜ' => "ze",
        'ぞ' => "zo",
        'だ' => "da",
        'ぢ' => "ji",
        'づ' => "zu",
        'で' => "de",
        'ど' => "do",
        'ば' => "ba",
        'び' => "bi",
        'ぶ' => "bu",
        'べ' => "be",
        'ぼ' => "bo",
        'ぱ' => "pa",
        'ぴ' => "pi",
        'ぷ' => "pu",
        'ぺ' => "pe",
        'ぽ' => "po",
        'ゔ' => "vu",
        'ぁ' => "a",
        'ぃ' => "i",
        'ぅ' => "u",
        'ぇ' => "e",
        'ぉ' => "o",
        'ゃ' => "ya",
        'ゅ' => "yu",
        'ょ' => "yo",
        'ゎ' => "wa",
        'ゕ' => "ka",
        'ゖ' => "ke",
        // Katakana-only letters (no hiragana counterpart in this table).
        'ヷ' => "va",
        'ヸ' => "vi",
        'ヹ' => "ve",
        'ヺ' => "vo",
        _ => return None,
    })
}

/// Katakana to hiragana (other characters unchanged).
fn to_hiragana(c: char) -> char {
    match c {
        '\u{30A1}'..='\u{30F6}' => char::from_u32(c as u32 - 0x60).unwrap_or(c),
        '\u{30FD}' => 'ゝ',
        '\u{30FE}' => 'ゞ',
        _ => c,
    }
}

fn is_small_y(c: char) -> bool {
    matches!(c, 'ゃ' | 'ゅ' | 'ょ')
}

fn is_small_vowel(c: char) -> bool {
    matches!(c, 'ぁ' | 'ぃ' | 'ぅ' | 'ぇ' | 'ぉ')
}

/// Katakana extension digraphs written with a small vowel.
fn extension(first: char, small: char) -> Option<&'static str> {
    Some(match (first, small) {
        ('ふ', 'ぁ') => "fa",
        ('ふ', 'ぃ') => "fi",
        ('ふ', 'ぇ') => "fe",
        ('ふ', 'ぉ') => "fo",
        ('て', 'ぃ') => "ti",
        ('で', 'ぃ') => "di",
        ('と', 'ぅ') => "tu",
        ('ど', 'ぅ') => "du",
        ('う', 'ぃ') => "wi",
        ('う', 'ぇ') => "we",
        ('う', 'ぉ') => "wo",
        ('ゔ', 'ぁ') => "va",
        ('ゔ', 'ぃ') => "vi",
        ('ゔ', 'ぇ') => "ve",
        ('ゔ', 'ぉ') => "vo",
        ('し', 'ぇ') => "she",
        ('じ', 'ぇ') => "je",
        ('ち', 'ぇ') => "che",
        ('つ', 'ぁ') => "tsa",
        ('つ', 'ぃ') => "tsi",
        ('つ', 'ぇ') => "tse",
        ('つ', 'ぉ') => "tso",
        ('い', 'ぇ') => "ye",
        ('く', 'ぁ') => "kwa",
        ('ぐ', 'ぁ') => "gwa",
        _ => return None,
    })
}

fn macron(vowel: char) -> Option<char> {
    Some(match vowel {
        'a' => 'ā',
        'i' => 'ī',
        'u' => 'ū',
        'e' => 'ē',
        'o' => 'ō',
        _ => return None,
    })
}

/// Romaji for one syllable starting at `chars[i]`; returns it and how many
/// characters it used.
fn syllable(chars: &[char], i: usize) -> Option<(String, usize)> {
    let first = chars[i];
    let next = chars.get(i + 1).copied();
    if let Some(small) = next.filter(|n| is_small_vowel(*n)) {
        if let Some(special) = extension(first, small) {
            return Some((special.to_string(), 2));
        }
    }
    let romaji = base(first)?;
    if let Some(small) = next.filter(|n| is_small_y(*n)) {
        if romaji.len() >= 2 && romaji.ends_with('i') && first != 'い' {
            let stem = &romaji[..romaji.len() - 1];
            let vowel = &base(small)?[1..];
            let joined = if matches!(romaji, "shi" | "chi" | "ji") {
                format!("{stem}{vowel}")
            } else {
                format!("{stem}y{vowel}")
            };
            return Some((joined, 2));
        }
    }
    Some((romaji.to_string(), 1))
}

/// Hepburn romaji for kana; other characters are kept.
pub fn kana_to_romaji(text: &str) -> String {
    let chars: Vec<char> = text.nfkc().map(to_hiragana).collect();
    let mut out = String::new();
    let mut previous = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            'っ' => {
                // Double the next consonant; "tch" before ch.
                if let Some((next, used)) = (i + 1 < chars.len()).then(|| syllable(&chars, i + 1)).flatten() {
                    let lead = if next.starts_with("ch") {
                        "t".to_string()
                    } else {
                        next.chars().next().filter(|c| !"aiueo".contains(*c)).map(String::from).unwrap_or_default()
                    };
                    let piece = format!("{lead}{next}");
                    out.push_str(&piece);
                    previous = piece;
                    i += 1 + used;
                    continue;
                }
                i += 1;
            }
            'ん' => {
                let follows_vowel = (i + 1 < chars.len())
                    .then(|| syllable(&chars, i + 1))
                    .flatten()
                    .is_some_and(|(next, _)| next.starts_with(['a', 'i', 'u', 'e', 'o', 'y']));
                let piece = if follows_vowel { "n'" } else { "n" };
                out.push_str(piece);
                previous = piece.to_string();
                i += 1;
            }
            'ー' => {
                match out.pop() {
                    Some(last) => match macron(last) {
                        Some(long) => out.push(long),
                        None => {
                            out.push(last);
                            out.push('-');
                        }
                    },
                    None => out.push('-'),
                }
                i += 1;
            }
            'ゝ' | 'ゞ' => {
                out.push_str(&previous);
                i += 1;
            }
            _ => match syllable(&chars, i) {
                Some((piece, used)) => {
                    out.push_str(&piece);
                    previous = piece;
                    i += used;
                }
                None => {
                    out.push(c);
                    previous.clear();
                    i += 1;
                }
            },
        }
    }
    out
}

/// True for hiragana / katakana letters (including ー).
pub fn is_kana(c: char) -> bool {
    matches!(c, '\u{3041}'..='\u{3096}' | '\u{309D}'..='\u{309F}' | '\u{30A1}'..='\u{30FA}' | '\u{30FC}'..='\u{30FF}' | '\u{FF66}'..='\u{FF9F}')
}

/// Hiragana form for comparing readings with surface kana.
pub fn hiragana(text: &str) -> String {
    text.nfkc().map(to_hiragana).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_hepburn() {
        assert_eq!(kana_to_romaji("ありがとう"), "arigatou");
        assert_eq!(kana_to_romaji("しんじゅく"), "shinjuku");
        assert_eq!(kana_to_romaji("ちゃんと"), "chanto");
        assert_eq!(kana_to_romaji("きょう"), "kyou");
        assert_eq!(kana_to_romaji("ふじさん"), "fujisan");
        assert_eq!(kana_to_romaji("つづく"), "tsuzuku");
    }

    #[test]
    fn sokuon_hatsuon_and_long_vowels() {
        assert_eq!(kana_to_romaji("きって"), "kitte");
        assert_eq!(kana_to_romaji("まっちゃ"), "matcha");
        assert_eq!(kana_to_romaji("げんいん"), "gen'in");
        assert_eq!(kana_to_romaji("こんや"), "kon'ya");
        assert_eq!(kana_to_romaji("さんぽ"), "sanpo");
        assert_eq!(kana_to_romaji("ラーメン"), "rāmen");
        assert_eq!(kana_to_romaji("コーヒー"), "kōhī");
    }

    #[test]
    fn katakana_extensions_and_passthrough() {
        assert_eq!(kana_to_romaji("ファイト"), "faito");
        assert_eq!(kana_to_romaji("パーティー"), "pātī");
        assert_eq!(kana_to_romaji("ヴァイオリン"), "vaiorin");
        assert_eq!(kana_to_romaji("ｶﾀｶﾅ"), "katakana");
        assert_eq!(kana_to_romaji("夜に駆ける!"), "夜ni駆keru!");
        assert_eq!(kana_to_romaji("いぇい"), "yei");
    }
}
