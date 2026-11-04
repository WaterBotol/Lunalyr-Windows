//! Readings for non-Latin lyrics (`LyricRomanizer` in
//! Sources/Lyrics/LyricRomanization.swift): Mandarin pinyin (with or without
//! tone marks), Korean Revised Romanization, Japanese kana → Hepburn, and
//! Japanese kanji readings supplied by the platform (Windows'
//! `JapanesePhoneticAnalyzer`) through a callback. Readings a source
//! supplies (`Line::romanization`, e.g. `.roma.lrc` Jyutping) are preferred.
//!
//! Differences from macOS: there is no language recogniser, so a Han-only
//! text is Japanese only when the line or song shows kana (else Mandarin);
//! other scripts (Cyrillic, Greek, Arabic, ...) get no reading.

mod hangul;
mod kana;

use std::collections::HashMap;
use std::sync::LazyLock;

use pinyin::ToPinyin;
use regex::Regex;
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

pub use hangul::hangul_to_latin;
pub use kana::kana_to_romaji;

use crate::model::{Line, Word};
use crate::settings::ReadingDisplay;
use crate::text::trim_all;
use crate::timing::distribute_words;

static KANA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\p{Hiragana}\p{Katakana}]").unwrap());
static HANGUL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\p{Hangul}").unwrap());
static HAN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\p{Han}").unwrap());
static NON_LATIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"[\p{Han}\p{Hiragana}\p{Katakana}\p{Hangul}\p{Cyrillic}\p{Greek}\p{Arabic}\p{Devanagari}\p{Thai}\p{Hebrew}]",
    )
    .unwrap()
});

/// Segments of a Japanese text as `(surface, reading)` pairs, in order, the
/// reading in kana (or already Latin); None when no analyser is available.
/// On Windows: `JapanesePhoneticAnalyzer.GetWords(text)` →
/// `(DisplayText, YomiText)` (the API takes at most 100 characters per call).
pub type JapaneseReadings<'a> = &'a dyn Fn(&str) -> Option<Vec<(String, String)>>;

/// Reading language (`RomanizationOptions.Language` without "automatic").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Japanese,
    Mandarin,
    Korean,
    Other,
}

/// Any Han, kana, Hangul, Cyrillic, Greek, Arabic, Devanagari, Thai or
/// Hebrew letter (`LyricRomanizer.containsNonLatin`).
pub fn contains_non_latin(text: &str) -> bool {
    NON_LATIN.is_match(text)
}

/// Kana → Japanese; Hangul → Korean; no Han → Other; Han-only text keeps a
/// Japanese / Korean `fallback` (a short Han fragment in such a line), else
/// Mandarin.
pub fn detect_language(text: &str, fallback: Language) -> Language {
    if KANA.is_match(text) {
        return Language::Japanese;
    }
    if HANGUL.is_match(text) {
        return Language::Korean;
    }
    if !HAN.is_match(text) {
        return Language::Other;
    }
    if matches!(fallback, Language::Japanese | Language::Korean) {
        fallback
    } else {
        Language::Mandarin
    }
}

/// Removes diacritics (tone marks, macrons, umlauts).
pub fn strip_diacritics(text: &str) -> String {
    text.nfd().filter(|c| !is_combining_mark(*c)).nfc().collect()
}

fn normalized(text: &str, tone_marks: bool) -> String {
    let reading = if tone_marks { text.to_string() } else { strip_diacritics(text) };
    trim_all(&reading).to_string()
}

/// Tone-marked pinyin, one syllable per character, separated by spaces
/// ("你好" → "nǐ hǎo"). Characters without a reading pass through.
pub fn pinyin(text: &str) -> String {
    // Previous character: Some(true) a Han syllable, Some(false) another
    // letter or digit, None anything else.
    let mut previous: Option<bool> = None;
    let mut out = String::new();
    for c in text.chars() {
        match c.to_pinyin() {
            Some(reading) => {
                if previous.is_some() {
                    out.push(' ');
                }
                out.push_str(reading.with_tone());
                previous = Some(true);
            }
            None => {
                if previous == Some(true) && c.is_alphanumeric() {
                    out.push(' ');
                }
                out.push(c);
                previous = c.is_alphanumeric().then_some(false);
            }
        }
    }
    out
}

fn is_particle(surface: &str) -> Option<&'static str> {
    match surface {
        "は" => Some("わ"),
        "へ" => Some("え"),
        _ => None,
    }
}

/// Romaji for one analyser segment.
fn segment_romaji(surface: &str, reading: &str, particle: bool) -> String {
    if particle {
        if let Some(spoken) = is_particle(surface) {
            return kana_to_romaji(spoken);
        }
    }
    let reading = if trim_all(reading).is_empty() { surface } else { reading };
    kana_to_romaji(reading)
}

/// Splits a segment reading across the segment's characters: kana in the
/// surface match the reading literally, kanji runs take what lies between
/// (楽しい / たのしい → 楽 たの, し し, い い). The first character of a
/// kanji run carries the run's whole reading.
fn split_reading(surface: &[char], reading: &str) -> Option<Vec<String>> {
    let reading: Vec<char> = kana::hiragana(reading).chars().collect();
    if surface.len() > 32 || reading.len() > 96 {
        return None;
    }
    // Runs of (is_kana, start, end) over the surface.
    let mut runs: Vec<(bool, usize, usize)> = Vec::new();
    for (i, c) in surface.iter().enumerate() {
        let k = kana::is_kana(*c);
        match runs.last_mut() {
            Some((kind, _, end)) if *kind == k => *end = i + 1,
            _ => runs.push((k, i, i + 1)),
        }
    }
    let surface_hiragana: Vec<char> = kana::hiragana(&surface.iter().collect::<String>()).chars().collect();
    if surface_hiragana.len() != surface.len() {
        return None;
    }
    fn assign(
        runs: &[(bool, usize, usize)],
        surface: &[char],
        reading: &[char],
        position: usize,
        out: &mut Vec<(usize, usize)>,
    ) -> bool {
        let Some(&(is_kana, start, end)) = runs.first() else { return position == reading.len() };
        if is_kana {
            let wanted = &surface[start..end];
            if reading.len() >= position + wanted.len() && &reading[position..position + wanted.len()] == wanted {
                out.push((position, position + wanted.len()));
                if assign(&runs[1..], surface, reading, position + wanted.len(), out) {
                    return true;
                }
                out.pop();
            }
            return false;
        }
        for length in 1..=reading.len().saturating_sub(position) {
            out.push((position, position + length));
            if assign(&runs[1..], surface, reading, position + length, out) {
                return true;
            }
            out.pop();
        }
        false
    }
    let mut spans = Vec::new();
    if !assign(&runs, &surface_hiragana, &reading, 0, &mut spans) {
        return None;
    }
    let mut per_char = vec![String::new(); surface.len()];
    for ((is_kana, start, end), (from, to)) in runs.iter().zip(spans) {
        if *is_kana {
            for (offset, i) in (*start..*end).enumerate() {
                per_char[i] = reading[from + offset].to_string();
            }
        } else {
            per_char[*start] = reading[from..to].iter().collect();
        }
    }
    Some(per_char)
}

/// Finds `needle` in `haystack` at `cursor`, skipping whitespace first.
fn locate(haystack: &[char], cursor: usize, needle: &[char]) -> Option<usize> {
    let mut start = cursor;
    while start < haystack.len() && haystack[start].is_whitespace() {
        start += 1;
    }
    (haystack.len() >= start + needle.len() && &haystack[start..start + needle.len()] == needle).then_some(start)
}

struct Romanizer<'a> {
    japanese: JapaneseReadings<'a>,
    tone_marks: bool,
    memo: HashMap<(Language, String), String>,
    analyses: HashMap<String, Option<Vec<(String, String)>>>,
}

impl Romanizer<'_> {
    fn analyse(&mut self, text: &str) -> Option<Vec<(String, String)>> {
        if let Some(found) = self.analyses.get(text) {
            return found.clone();
        }
        let found = (self.japanese)(text);
        self.analyses.insert(text.to_string(), found.clone());
        found
    }

    /// `LyricRomanizer.romanize` for one word.
    fn romanize(&mut self, text: &str, language: Language) -> String {
        if !contains_non_latin(text) {
            return text.to_string();
        }
        if let Some(found) = self.memo.get(&(language, text.to_string())) {
            return found.clone();
        }
        let reading = match language {
            Language::Japanese => match self.analyse(text) {
                Some(segments) => segments
                    .iter()
                    .enumerate()
                    .filter(|(_, (surface, _))| !trim_all(surface).is_empty())
                    .map(|(i, (surface, reading))| segment_romaji(surface, reading, i > 0))
                    .collect::<Vec<_>>()
                    .join(" "),
                None => kana_to_romaji(text),
            },
            Language::Mandarin => pinyin(text),
            Language::Korean => hangul_to_latin(text),
            Language::Other => text.to_string(),
        };
        let reading = normalized(&reading, self.tone_marks);
        self.memo.insert((language, text.to_string()), reading.clone());
        reading
    }

    /// Japanese readings for every word of a line from one analysis of the
    /// whole line (better kanji readings than word-by-word analysis). None
    /// when the analyser is unavailable or its segments / the words cannot
    /// be located in the line text.
    fn japanese_line(&mut self, line: &Line) -> Option<Vec<String>> {
        let segments = self.analyse(&line.text)?;
        let chars: Vec<char> = line.text.chars().collect();
        // Per character: (segment index, reading piece in kana).
        let mut per_char: Vec<Option<(usize, String)>> = vec![None; chars.len()];
        let mut cursor = 0;
        let mut first_segment = true;
        for (index, (surface, reading)) in segments.iter().enumerate() {
            let surface_chars: Vec<char> = surface.chars().collect();
            if surface_chars.iter().all(|c| c.is_whitespace()) {
                continue;
            }
            let start = locate(&chars, cursor, &surface_chars)?;
            let particle = !first_segment && is_particle(surface).is_some();
            first_segment = false;
            let pieces: Vec<String> = if particle {
                let mut pieces = vec![String::new(); surface_chars.len()];
                pieces[0] = is_particle(surface).unwrap_or_default().to_string();
                pieces
            } else if trim_all(reading).is_empty() || reading.chars().any(|c| c.is_ascii_alphabetic()) {
                let mut pieces = vec![String::new(); surface_chars.len()];
                pieces[0] = if trim_all(reading).is_empty() { surface.clone() } else { reading.clone() };
                pieces
            } else {
                split_reading(&surface_chars, reading).unwrap_or_else(|| {
                    let mut pieces = vec![String::new(); surface_chars.len()];
                    pieces[0] = reading.clone();
                    pieces
                })
            };
            for (offset, piece) in pieces.into_iter().enumerate() {
                per_char[start + offset] = Some((index, piece));
            }
            cursor = start + surface_chars.len();
        }
        let mut readings = Vec::with_capacity(line.words.len());
        let mut cursor = 0;
        for word in &line.words {
            let word_chars: Vec<char> = word.text.chars().collect();
            let start = locate(&chars, cursor, &word_chars)?;
            let mut parts: Vec<String> = Vec::new();
            let mut group: Option<usize> = None;
            let mut buffer = String::new();
            for i in start..start + word_chars.len() {
                match &per_char[i] {
                    Some((segment, piece)) => {
                        if group != Some(*segment) && !buffer.is_empty() {
                            parts.push(kana_to_romaji(&std::mem::take(&mut buffer)));
                        }
                        group = Some(*segment);
                        buffer.push_str(piece);
                    }
                    None => {
                        if !buffer.is_empty() {
                            parts.push(kana_to_romaji(&std::mem::take(&mut buffer)));
                        }
                        group = None;
                        parts.push(chars[i].to_string());
                    }
                }
            }
            if !buffer.is_empty() {
                parts.push(kana_to_romaji(&buffer));
            }
            readings.push(normalized(&parts.join(" "), self.tone_marks));
            cursor = start + word_chars.len();
        }
        Some(readings)
    }
}

/// Reading words per line (`LyricRomanizer.readings`), `None` where a line
/// has no reading. Readings keep each original word's timing and align 1:1
/// with `line.words`, with one macOS exception: in [`ReadingDisplay::Replace`]
/// a line without real word timing whose saved reading splits into a
/// different number of words gets the reading's own words spread over the
/// line (render them instead of the line's words).
///
/// Saved readings (`Line::romanization`) win: matched word-for-word when the
/// counts agree; shown whole under a one-word line ("below"). Otherwise lines
/// with non-Latin text are romanised word by word; a word's language is
/// detected from the word, then the line, then the first 24 lines.
/// `tone_marks` false strips all diacritics from readings.
pub fn readings(
    lines: &[Line],
    display: ReadingDisplay,
    tone_marks: bool,
    japanese: JapaneseReadings,
) -> Vec<Option<Vec<Word>>> {
    let mut result = vec![None; lines.len()];
    if display == ReadingDisplay::Off {
        return result;
    }
    let context = lines.iter().take(24).map(|l| l.text.as_str()).collect::<Vec<_>>().join(" ");
    let song_language = detect_language(&context, Language::Other);
    let mut romanizer = Romanizer { japanese, tone_marks, memo: HashMap::new(), analyses: HashMap::new() };
    for (index, line) in lines.iter().enumerate() {
        if line.is_instrumental {
            continue;
        }
        let language = detect_language(&line.text, song_language);
        if let Some(saved) = &line.romanization {
            let reading = normalized(saved, tone_marks);
            let words = distribute_words(&reading, line.start, line.end);
            if !words.is_empty() && words.len() == line.words.len() {
                result[index] = Some(
                    words
                        .into_iter()
                        .zip(&line.words)
                        .map(|(saved, original)| Word { text: saved.text, start: original.start, end: original.end })
                        .collect(),
                );
                continue;
            }
            if !words.is_empty() && display == ReadingDisplay::Below && line.words.len() == 1 {
                let original = &line.words[0];
                result[index] = Some(vec![Word { text: reading, start: original.start, end: original.end }]);
                continue;
            }
            if !words.is_empty() && !line.word_timed && display == ReadingDisplay::Replace {
                result[index] = Some(words);
                continue;
            }
        }
        if !contains_non_latin(&line.text) {
            continue;
        }
        let whole_line = if language == Language::Japanese { romanizer.japanese_line(line) } else { None };
        let converted: Vec<Word> = line
            .words
            .iter()
            .enumerate()
            .map(|(i, word)| {
                let word_language = detect_language(&word.text, language);
                let text = match (&whole_line, word_language) {
                    (Some(readings), Language::Japanese) if contains_non_latin(&word.text) => readings[i].clone(),
                    _ => romanizer.romanize(&word.text, word_language),
                };
                Word { text, start: word.start, end: word.end }
            })
            .collect();
        if converted.iter().zip(&line.words).any(|(a, b)| a.text != b.text) {
            result[index] = Some(converted);
        }
    }
    result
}

/// Romanises a text in a given language (`LyricRomanizer.romanize`).
pub fn romanize(text: &str, language: Language, tone_marks: bool, japanese: JapaneseReadings) -> String {
    Romanizer { japanese, tone_marks, memo: HashMap::new(), analyses: HashMap::new() }.romanize(text, language)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> Option<Vec<(String, String)>> {
        None
    }

    /// A tiny stand-in for the Windows analyser.
    fn analyser(text: &str) -> Option<Vec<(String, String)>> {
        let dictionary = [
            ("夜", "よる"),
            ("に", "に"),
            ("駆ける", "かける"),
            ("世界", "せかい"),
            ("は", "は"),
            ("楽しい", "たのしい"),
            ("今日", "きょう"),
            (" ", ""),
            ("love", "love"),
        ];
        let mut rest = text;
        let mut out = Vec::new();
        'outer: while !rest.is_empty() {
            for (surface, reading) in dictionary {
                if let Some(next) = rest.strip_prefix(surface) {
                    out.push((surface.to_string(), reading.to_string()));
                    rest = next;
                    continue 'outer;
                }
            }
            let c = rest.chars().next().unwrap();
            out.push((c.to_string(), c.to_string()));
            rest = &rest[c.len_utf8()..];
        }
        Some(out)
    }

    fn line(text: &str, words: &[(&str, f64, f64)], word_timed: bool) -> Line {
        Line {
            start: words.first().map_or(0.0, |w| w.1),
            end: words.last().map_or(1.0, |w| w.2),
            text: text.into(),
            words: words.iter().map(|(t, s, e)| Word { text: (*t).into(), start: *s, end: *e }).collect(),
            is_instrumental: false,
            word_timed,
            romanization: None,
        }
    }

    // ReadingAndTTMLTests.testPinyinKeepsOriginalWordTimes
    #[test]
    fn pinyin_keeps_original_word_times() {
        let lines = [line("你好", &[("你好", 1.2, 2.7)], true)];
        let result = readings(&lines, ReadingDisplay::Below, false, &none);
        let word = &result[0].as_ref().unwrap()[0];
        assert_eq!(word.text.to_lowercase(), "ni hao");
        assert_eq!((word.start, word.end), (1.2, 2.7));
        let toned = readings(&lines, ReadingDisplay::Below, true, &none);
        assert_eq!(toned[0].as_ref().unwrap()[0].text, "nǐ hǎo");
    }

    // ReadingAndTTMLTests.testSavedJyutpingIsUsedInsteadOfGeneratingPinyin
    #[test]
    fn saved_jyutping_is_used_instead_of_generating_pinyin() {
        let mut l = line("你好", &[("你好", 1.0, 3.0)], true);
        l.romanization = Some("nei5 hou2".into());
        let result = readings(&[l], ReadingDisplay::Below, true, &none);
        let words = result[0].as_ref().unwrap();
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].text, "nei5 hou2");
    }

    // ReadingAndTTMLTests.testSavedReadingsReplaceWordsWithoutMovingTheirTiming
    #[test]
    fn saved_readings_replace_words_without_moving_their_timing() {
        let mut l = line("你好", &[("你", 1.2, 1.5), ("好", 2.1, 2.9)], true);
        l.romanization = Some("nei5 hou2".into());
        let result = readings(&[l], ReadingDisplay::Replace, true, &none);
        let words = result[0].as_ref().unwrap();
        let texts: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["nei5", "hou2"]);
        assert_eq!(words.iter().map(|w| w.start).collect::<Vec<_>>(), [1.2, 2.1]);
        assert_eq!(words.iter().map(|w| w.end).collect::<Vec<_>>(), [1.5, 2.9]);
    }

    #[test]
    fn saved_reading_on_line_timed_line_in_replace_mode_brings_its_own_words() {
        let mut l = line("你好吗", &[("你好吗", 1.0, 3.0)], false);
        l.romanization = Some("néih hóu ma".into());
        let result = readings(&[l.clone()], ReadingDisplay::Replace, false, &none);
        let texts: Vec<String> = result[0].as_ref().unwrap().iter().map(|w| w.text.clone()).collect();
        assert_eq!(texts, ["neih", "hou", "ma"], "tone marks stripped from saved readings too");
        assert!(readings(&[l], ReadingDisplay::Off, true, &none)[0].is_none());
    }

    // ReadingAndTTMLTests.testJapaneseKanaDoesNotFallBackToMandarin
    #[test]
    fn japanese_kana_does_not_fall_back_to_mandarin() {
        let reading = romanize("ありがとう", Language::Japanese, false, &none);
        assert!(reading.to_lowercase().contains("arigat"));
    }

    #[test]
    fn japanese_line_analysis_aligns_readings_to_per_character_words() {
        let words = [("夜", 0.0, 0.5), ("に", 0.5, 1.0), ("駆", 1.0, 1.4), ("け", 1.4, 1.7), ("る", 1.7, 2.0)];
        let lines = [line("夜に駆ける", &words, true)];
        let result = readings(&lines, ReadingDisplay::Below, true, &analyser);
        let texts: Vec<String> = result[0].as_ref().unwrap().iter().map(|w| w.text.clone()).collect();
        assert_eq!(texts, ["yoru", "ni", "ka", "ke", "ru"]);
        // Without an analyser: kana are romanised, kanji kept.
        let fallback = readings(&lines, ReadingDisplay::Below, true, &none);
        let texts: Vec<String> = fallback[0].as_ref().unwrap().iter().map(|w| w.text.clone()).collect();
        assert_eq!(texts, ["夜", "ni", "駆", "ke", "ru"]);
    }

    #[test]
    fn japanese_compounds_particles_and_mixed_words() {
        let words = [("世界は", 0.0, 1.0), ("楽しい", 1.0, 2.0), ("love", 2.0, 3.0)];
        // The second line is Han-only; the song's kana make it Japanese. A
        // kanji compound split over two words keeps its reading on the first.
        let lines =
            [line("世界は 楽しい love", &words, true), line("今日", &[("今", 3.0, 3.5), ("日", 3.5, 4.0)], true)];
        let result = readings(&lines, ReadingDisplay::Below, true, &analyser);
        let texts: Vec<String> = result[0].as_ref().unwrap().iter().map(|w| w.text.clone()).collect();
        assert_eq!(texts, ["sekai wa", "tanoshii", "love"]);
        let texts: Vec<String> = result[1].as_ref().unwrap().iter().map(|w| w.text.clone()).collect();
        assert_eq!(texts, ["kyou", ""]);
    }

    #[test]
    fn han_only_lines_follow_the_song_language() {
        let lines =
            [line("夜に駆ける", &[("夜に駆ける", 0.0, 1.0)], false), line("世界", &[("世界", 1.0, 2.0)], false)];
        let result = readings(&lines, ReadingDisplay::Below, true, &analyser);
        assert_eq!(result[1].as_ref().unwrap()[0].text, "sekai", "Japanese song, so not shìjiè");
        let mandarin = readings(&[line("世界", &[("世界", 0.0, 1.0)], false)], ReadingDisplay::Below, true, &none);
        assert_eq!(mandarin[0].as_ref().unwrap()[0].text, "shì jiè");
    }

    #[test]
    fn korean_lines_and_latin_lines() {
        let lines = [
            line("사랑해 baby", &[("사랑해", 0.0, 1.0), ("baby", 1.0, 2.0)], true),
            line("Only Latin here", &[("Only", 2.0, 2.5), ("Latin", 2.5, 3.0), ("here", 3.0, 3.5)], true),
        ];
        let result = readings(&lines, ReadingDisplay::Below, true, &none);
        let texts: Vec<String> = result[0].as_ref().unwrap().iter().map(|w| w.text.clone()).collect();
        assert_eq!(texts, ["saranghae", "baby"]);
        assert!(result[1].is_none());
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn detection_and_helpers() {
        assert_eq!(detect_language("カタカナ", Language::Other), Language::Japanese);
        assert_eq!(detect_language("한국어", Language::Other), Language::Korean);
        assert_eq!(detect_language("漢字", Language::Korean), Language::Korean);
        assert_eq!(detect_language("漢字", Language::Other), Language::Mandarin);
        assert_eq!(detect_language("Latin", Language::Japanese), Language::Other);
        assert!(contains_non_latin("Привет"));
        assert!(!contains_non_latin("Café ♪"));
        assert_eq!(strip_diacritics("nǚ rén lǜ"), "nu ren lu");
        assert_eq!(pinyin("我love你!"), "wǒ love nǐ!");
        assert_eq!(pinyin("“你”"), "“nǐ”");
        assert_eq!(split_reading(&"楽しい".chars().collect::<Vec<_>>(), "たのしい").unwrap(), ["たの", "し", "い"]);
        assert_eq!(
            split_reading(&"見つける".chars().collect::<Vec<_>>(), "ミツケル").unwrap(),
            ["み", "つ", "け", "る"]
        );
        assert!(split_reading(&"走る".chars().collect::<Vec<_>>(), "あるく").is_none());
    }
}
