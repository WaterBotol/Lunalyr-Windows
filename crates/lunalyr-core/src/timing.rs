//! Line construction, word-timing validation, timing tiers, the active-line
//! index, vocal gaps (instrumental interludes) and QQ gap repair.
//!
//! Ported from Sources/Lyrics/LyricsFetcher.swift (`makeLine`,
//! `distributeWords`, `validWordTiming`, `hasWordTiming`, `timingTier`,
//! `precision`), LyricLineIndex.swift, LyricGapTiming.swift,
//! QQGapRepair.swift, LyricsViewModel.swift (`apply`) and
//! Sources/Support/ConstellationGeometry.c (`vc_vocal_gap`,
//! `vc_estimated_line_end`).

use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

use crate::model::{Line, LyricResult, Provenance, Word};
use crate::text::{
    canonical_eq, grapheme_count, grapheme_is_whitespace, letters_and_numbers, swift_max, swift_min, trim_all,
    trim_spaces,
};

/// `precision` strings (same as macOS).
pub const WORD_BY_WORD: &str = "Word-by-word";
pub const MIXED_TIMING: &str = "Mixed timing";
pub const LINE_SYNCED: &str = "Line-synced";
pub const NOT_SYNCED: &str = "Not synced";

/// Glyphs that, alone, mark an instrumental break rather than a lyric.
const INSTRUMENTAL_GLYPHS: [&str; 7] = ["♪", "♫", "•", "·", "-", "–", "—"];

/// Builds a line with estimated word timing (`LyricsFetcher.makeLine`).
///
/// The text is trimmed of spaces/tabs. An empty line, or one made only of
/// note glyphs, bullets and dashes, becomes an instrumental line with empty
/// text and no words.
pub fn make_line(text: &str, start: f64, end: f64) -> Line {
    let cleaned = trim_spaces(text);
    let instrumental = cleaned.is_empty()
        || cleaned.graphemes(true).all(|g| INSTRUMENTAL_GLYPHS.contains(&g) || grapheme_is_whitespace(g));
    Line {
        start,
        end,
        text: if instrumental { String::new() } else { cleaned.to_string() },
        words: if instrumental { Vec::new() } else { distribute_words(cleaned, start, end) },
        is_instrumental: instrumental,
        word_timed: false,
        romanization: None,
    }
}

/// Word tokens as `NSString.enumerateSubstrings(options: .byWords)` reports
/// them through its *enclosing* ranges: each word keeps the punctuation and
/// spacing that follows it (the first also keeps anything before it), then
/// whitespace is trimmed. Word boundaries come from UAX #29.
///
/// Deliberate difference: macOS segments CJK with a dictionary
/// ("夜に駆ける" → 夜 / に / 駆ける); UAX #29 has no dictionary, so each
/// ideograph and kana becomes its own token here.
pub fn word_tokens(text: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut leading = String::new();
    for segment in text.split_word_bounds() {
        if segment.chars().any(char::is_alphanumeric) {
            let mut token = if tokens.is_empty() { std::mem::take(&mut leading) } else { String::new() };
            token.push_str(segment);
            tokens.push(token);
        } else if let Some(last) = tokens.last_mut() {
            last.push_str(segment);
        } else {
            leading.push_str(segment);
        }
    }
    tokens.into_iter().map(|t| trim_all(&t).to_string()).filter(|t| !t.is_empty()).collect()
}

/// Splits a line window across its words in proportion to their length
/// (`LyricsFetcher.distributeWords`): weight = characters + 1.5, window =
/// max(end − start, 0.3), and only `min(window, max(n × 0.42, window × 0.88))`
/// of it is used because a sung line usually ends before the next begins.
pub fn distribute_words(text: &str, start: f64, end: f64) -> Vec<Word> {
    let tokens = word_tokens(text);
    if tokens.is_empty() {
        return Vec::new();
    }
    let window = (end - start).max(0.3);
    let usable = window.min((tokens.len() as f64 * 0.42).max(window * 0.88));
    let weights: Vec<f64> = tokens.iter().map(|t| grapheme_count(t) as f64 + 1.5).collect();
    let total: f64 = weights.iter().sum();
    let mut cursor = start;
    tokens
        .into_iter()
        .zip(weights)
        .map(|(text, weight)| {
            let slice = usable * (weight / total);
            let word = Word { text, start: cursor, end: cursor + slice };
            cursor += slice;
            word
        })
        .collect()
}

/// True when a line's words are real, usable per-word timing
/// (`LyricsFetcher.validWordTiming`): the line is flagged word-timed, its
/// window is valid, the words spell the line's letters and digits
/// (punctuation and spacing may differ), every word sits inside the line
/// (±50 ms), has positive length, and starts never go backwards.
pub fn valid_word_timing(line: &Line) -> bool {
    if !line.word_timed
        || line.words.is_empty()
        || !line.start.is_finite()
        || !line.end.is_finite()
        || line.end <= line.start
    {
        return false;
    }
    let joined: String = line.words.iter().map(|w| w.text.as_str()).collect();
    if !canonical_eq(&letters_and_numbers(&joined), &letters_and_numbers(&line.text)) {
        return false;
    }
    line.words.iter().enumerate().all(|(i, word)| {
        word.start.is_finite()
            && word.end.is_finite()
            && word.start >= line.start - 0.05
            && word.end > word.start
            && word.end <= line.end + 0.05
            && (i == 0 || word.start >= line.words[i - 1].start)
    })
}

/// Every vocal line has valid word timing (`LyricsFetcher.hasWordTiming`).
pub fn has_word_timing(result: &LyricResult) -> bool {
    match result {
        LyricResult::Synced { lines, .. } => {
            let mut vocal = lines.iter().filter(|l| !l.is_instrumental).peekable();
            vocal.peek().is_some() && vocal.all(valid_word_timing)
        }
        LyricResult::Plain { .. } => false,
    }
}

/// 3 = word-by-word, 2 = mixed, 1 = line-synced, 0 = plain / no vocal lines
/// (`LyricsFetcher.timingTier`).
pub fn timing_tier(result: &LyricResult) -> u8 {
    if has_word_timing(result) {
        return 3;
    }
    match result {
        LyricResult::Synced { lines, .. } => {
            let vocal: Vec<&Line> = lines.iter().filter(|l| !l.is_instrumental).collect();
            if vocal.iter().any(|l| valid_word_timing(l)) {
                2
            } else if vocal.is_empty() {
                0
            } else {
                1
            }
        }
        LyricResult::Plain { .. } => 0,
    }
}

/// "Word-by-word", "Mixed timing" or "Line-synced" (`LyricsFetcher.precision`).
pub fn precision(lines: &[Line]) -> &'static str {
    let vocal: Vec<&Line> = lines.iter().filter(|l| !l.is_instrumental).collect();
    if !vocal.is_empty() && vocal.iter().all(|l| valid_word_timing(l)) {
        WORD_BY_WORD
    } else if vocal.iter().any(|l| valid_word_timing(l)) {
        MIXED_TIMING
    } else {
        LINE_SYNCED
    }
}

/// `LyricsViewModel.apply`: QQ gap repair (only when enabled and the
/// provenance source contains "qq", case-insensitively), then `word_timed`
/// re-validated per line and the precision recomputed. Plain results and
/// empty results are returned unchanged (callers treat an empty result as
/// "not found").
pub fn prepare(result: LyricResult, repair_qq_gaps: bool) -> LyricResult {
    match result {
        LyricResult::Synced { lines, provenance } if !lines.is_empty() => {
            let repaired = if repair_qq_gaps && provenance.source.to_lowercase().contains("qq") {
                repair_qq(&lines)
            } else {
                lines
            };
            let lines: Vec<Line> = repaired
                .into_iter()
                .map(|mut line| {
                    line.word_timed = valid_word_timing(&line);
                    line
                })
                .collect();
            let provenance = Provenance { precision: precision(&lines).to_string(), source: provenance.source };
            LyricResult::Synced { lines, provenance }
        }
        other => other,
    }
}

/// Finds the active line: the last line whose start is at or before `time`
/// (`LyricLineIndex`). Ordered timelines use a binary search; malformed
/// (unordered or non-finite) ones keep the original early-stop forward scan.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LineIndex {
    starts: Vec<f64>,
    ordered: bool,
}

impl LineIndex {
    pub fn new(lines: &[Line]) -> Self {
        let starts: Vec<f64> = lines.iter().map(|l| l.start).collect();
        let ordered = starts.iter().all(|s| s.is_finite()) && starts.windows(2).all(|pair| pair[0] <= pair[1]);
        LineIndex { starts, ordered }
    }

    pub fn len(&self) -> usize {
        self.starts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.starts.is_empty()
    }

    /// True when the binary search is used.
    pub fn is_ordered(&self) -> bool {
        self.ordered
    }

    pub fn line_at(&self, time: f64) -> Option<usize> {
        if !self.ordered {
            let mut found = None;
            for (index, start) in self.starts.iter().enumerate() {
                if *start <= time {
                    found = Some(index);
                } else {
                    break;
                }
            }
            return found;
        }
        let (mut low, mut high) = (0usize, self.starts.len());
        while low < high {
            let middle = low + (high - low) / 2;
            if self.starts[middle] <= time {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        if low == 0 {
            None
        } else {
            Some(low - 1)
        }
    }
}

/// When someone is singing (`VCVocalWindow`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VocalWindow {
    pub start: f64,
    pub end: f64,
}

/// A stretch without vocals (`VCVocalGap`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VocalGap {
    pub start: f64,
    /// None when the gap runs to the end of a track of unknown length
    /// (C: `end == -1`).
    pub end: Option<f64>,
    /// Index of the window that ends the gap; None for the trailing gap
    /// (C: `nextVocal == -1`).
    pub next_vocal: Option<usize>,
}

/// The minimum gap the macOS wallpaper asks for (NowPlayingView).
pub const DEFAULT_MINIMUM_GAP: f64 = 2.4;

/// Vocal windows sorted by start (`LyricGapTiming.windows`). Word-timed
/// lines use their first word start and last word end; estimated words do
/// not supply an authoritative vocal end, so other lines use the line window.
pub fn vocal_windows(lines: &[Line]) -> Vec<VocalWindow> {
    let mut windows: Vec<VocalWindow> = lines
        .iter()
        .filter_map(|line| {
            if line.is_instrumental || line.words.is_empty() {
                return None;
            }
            let start = if line.word_timed {
                swift_min(line.words.iter().map(|w| w.start)).unwrap_or(line.start)
            } else {
                line.start
            };
            let end = if line.word_timed {
                swift_max(line.words.iter().map(|w| w.end)).unwrap_or(line.end)
            } else {
                line.end
            };
            (start.is_finite() && end.is_finite() && start >= 0.0 && end > start).then_some(VocalWindow { start, end })
        })
        .collect();
    windows.sort_by(|a, b| a.start.total_cmp(&b.start));
    windows
}

/// The vocal gap containing `time`, if it is at least `minimum` seconds long
/// (`vc_vocal_gap`). `duration` ≤ 0 or non-finite means unknown. Overlapping
/// windows merge; malformed input (unsorted, negative, non-finite, more than
/// 20 000 windows) yields None.
pub fn vocal_gap(windows: &[VocalWindow], time: f64, duration: f64, minimum: f64) -> Option<VocalGap> {
    if windows.len() > 20_000 || !time.is_finite() || time < 0.0 {
        return None;
    }
    let minimum = if minimum.is_finite() { minimum } else { DEFAULT_MINIMUM_GAP };
    let minimum = minimum.clamp(0.5, 10.0);
    let known_duration = duration.is_finite() && duration > 0.0;
    if known_duration && time >= duration {
        return None;
    }
    for (i, window) in windows.iter().enumerate() {
        if !window.start.is_finite()
            || !window.end.is_finite()
            || window.start < 0.0
            || window.end < window.start
            || (i > 0 && window.start < windows[i - 1].start)
        {
            return None;
        }
    }
    let mut previous = 0.0f64;
    let mut i = 0;
    while i < windows.len() {
        let start = windows[i].start;
        let mut end = windows[i].end;
        if time >= previous && time < start && start - previous >= minimum {
            return Some(VocalGap { start: previous, end: Some(start), next_vocal: Some(i) });
        }
        while i + 1 < windows.len() && windows[i + 1].start <= end {
            i += 1;
            end = end.max(windows[i].end);
        }
        if time >= start && time < end {
            return None;
        }
        previous = previous.max(end);
        i += 1;
    }
    if time >= previous && (!known_duration || duration - previous >= minimum) {
        return Some(VocalGap { start: previous, end: known_duration.then_some(duration), next_vocal: None });
    }
    None
}

/// Conservative vocal end for a line-only lyric that spans a long
/// instrumental (`vc_estimated_line_end`). Returns `next` unless the window
/// is much longer than the words could take to sing.
pub fn estimated_line_end(start: f64, next: f64, words: usize, per_word: f64) -> f64 {
    if !start.is_finite() || !next.is_finite() || next <= start || !(1..=10_000).contains(&words) {
        return next;
    }
    let per_word = if per_word.is_finite() { per_word } else { 0.45 };
    let per_word = per_word.clamp(0.18, 1.2);
    let sung = (words as f64 * per_word * 1.35 + 1.25).clamp(4.0, 22.0);
    if next - start > 12f64.max(sung + 6.0) {
        start + sung
    } else {
        next
    }
}

/// `QQGapRepair.repair`. LRC carries starts, not vocal ends, so a line
/// before a long instrumental would otherwise hold its words across the
/// break. Compact neighbouring lines give a singing pace; only unusually
/// long windows are split into the line plus an instrumental line. Real
/// word timing is never moved: a word-timed line only gains a gap when its
/// last word ends more than 2 s before the line does.
pub fn repair_qq(lines: &[Line]) -> Vec<Line> {
    let mut samples: Vec<f64> = lines
        .iter()
        .filter_map(|line| {
            let window = line.end - line.start;
            if line.is_instrumental || line.word_timed || line.words.len() < 2 || !(1.0..=10.0).contains(&window) {
                return None;
            }
            let pace = window / line.words.len() as f64;
            (0.18..=1.2).contains(&pace).then_some(pace)
        })
        .collect();
    samples.sort_by(f64::total_cmp);
    let pace = if samples.is_empty() { 0.45 } else { samples[samples.len() / 2] };
    let mut result = Vec::with_capacity(lines.len());
    for line in lines {
        if line.is_instrumental || line.words.is_empty() {
            result.push(line.clone());
            continue;
        }
        let end = if line.word_timed {
            let vocal_end = swift_max(line.words.iter().map(|w| w.end)).unwrap_or(line.end);
            if line.end - vocal_end > 2.0 {
                line.end.min(vocal_end + 0.3)
            } else {
                line.end
            }
        } else {
            estimated_line_end(line.start, line.end, line.words.len(), pace)
        };
        if !(end < line.end && end > line.start) {
            result.push(line.clone());
            continue;
        }
        let mut shortened = line.clone();
        shortened.end = end;
        if !line.word_timed {
            shortened.words = distribute_words(&line.text, line.start, end);
        }
        result.push(shortened);
        result.push(Line {
            start: end,
            end: line.end,
            text: String::new(),
            words: Vec::new(),
            is_instrumental: true,
            word_timed: false,
            romanization: None,
        });
    }
    result
}

/// Alias matching the module list in the porting brief.
pub fn repair_qq_gaps(lines: &[Line]) -> Vec<Line> {
    repair_qq(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_lrc;

    fn word(text: &str, start: f64, end: f64) -> Word {
        Word { text: text.into(), start, end }
    }

    fn timed(text: &str, start: f64, end: f64, words: Vec<Word>) -> Line {
        Line { start, end, text: text.into(), words, is_instrumental: false, word_timed: true, romanization: None }
    }

    fn synced(lines: Vec<Line>) -> LyricResult {
        LyricResult::Synced { lines, provenance: Provenance { precision: "fixture".into(), source: "Test".into() } }
    }

    #[test]
    fn make_line_marks_glyph_only_lines_instrumental() {
        for text in ["", "   ", "♪", "♪ ♫", "— – -", "•·"] {
            let line = make_line(text, 1.0, 2.0);
            assert!(line.is_instrumental, "{text:?}");
            assert!(line.text.is_empty() && line.words.is_empty());
        }
        let line = make_line("  ♪ la la  ", 1.0, 2.0);
        assert!(!line.is_instrumental);
        assert_eq!(line.text, "♪ la la");
        assert!(!line.word_timed);
    }

    #[test]
    fn distribute_words_weights_by_length_and_trims_the_window() {
        let words = distribute_words("Hello, wide world!", 10.0, 14.0);
        let texts: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["Hello,", "wide", "world!"]);
        // window 4, usable = min(4, max(1.26, 3.52)) = 3.52
        let total = 7.5 + 5.5 + 7.5;
        assert!((words[0].start - 10.0).abs() < 1e-12);
        assert!((words[0].end - (10.0 + 3.52 * 7.5 / total)).abs() < 1e-9);
        assert!((words[2].end - 13.52).abs() < 1e-9);
        for pair in words.windows(2) {
            assert!((pair[0].end - pair[1].start).abs() < 1e-12);
        }
    }

    #[test]
    fn distribute_words_short_window_uses_minimum() {
        // window max(0, 0.3) = 0.3; usable = min(0.3, max(0.84, 0.264)) = 0.3
        let words = distribute_words("a b", 5.0, 5.0);
        assert_eq!(words.len(), 2);
        assert!((words[1].end - 5.3).abs() < 1e-9);
        assert!(distribute_words("!!! ...", 0.0, 3.0).is_empty());
        assert!(distribute_words("", 0.0, 3.0).is_empty());
    }

    #[test]
    fn distribute_words_keeps_leading_punctuation_and_contractions() {
        let words = distribute_words("\"Don't stop\" - now", 0.0, 3.0);
        let texts: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["\"Don't", "stop\" -", "now"]);
    }

    #[test]
    fn distribute_words_cjk_splits_per_character() {
        let words = distribute_words("夜に駆ける", 0.0, 5.0);
        let texts: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["夜", "に", "駆", "け", "る"]);
        // equal weights -> equal slices; usable = min(5, max(2.1, 4.4)) = 4.4
        for w in &words {
            assert!((w.end - w.start - 0.88).abs() < 1e-9);
        }
        let hangul = distribute_words("안녕 하세요", 0.0, 2.0);
        assert_eq!(hangul.len(), 2);
        let katakana = distribute_words("カタカナ です", 0.0, 2.0);
        assert_eq!(katakana[0].text, "カタカナ");
    }

    #[test]
    fn valid_word_timing_edge_cases() {
        let good = timed("Sing along", 1.0, 3.0, vec![word("Sing", 1.0, 2.0), word("along", 2.0, 3.0)]);
        assert!(valid_word_timing(&good));
        let mut line = good.clone();
        line.word_timed = false;
        assert!(!valid_word_timing(&line), "flag required");
        let mut line = good.clone();
        line.words.clear();
        assert!(!valid_word_timing(&line), "words required");
        let mut line = good.clone();
        line.end = line.start;
        assert!(!valid_word_timing(&line), "empty window");
        let mut line = good.clone();
        line.start = f64::NAN;
        assert!(!valid_word_timing(&line));
        let mut line = good.clone();
        line.words[0].start = 0.94;
        assert!(!valid_word_timing(&line), "more than 50 ms early");
        line.words[0].start = 0.96;
        assert!(valid_word_timing(&line), "within tolerance");
        let mut line = good.clone();
        line.words[1].end = 3.06;
        assert!(!valid_word_timing(&line), "past the line end");
        let mut line = good.clone();
        line.words[1].end = line.words[1].start;
        assert!(!valid_word_timing(&line), "zero-length word");
        let mut line = good.clone();
        line.words[1].start = 0.99;
        assert!(!valid_word_timing(&line), "starts go backwards");
        let mut line = good.clone();
        line.words[1].text = "alone".into();
        assert!(!valid_word_timing(&line), "words must spell the line");
        let mut line = good.clone();
        line.text = "Sing, along!".into();
        assert!(valid_word_timing(&line), "punctuation may differ");
        let mut line = good;
        line.text = "Sing along".into();
        line.words[1].start = 1.0; // equal starts are allowed
        assert!(valid_word_timing(&line));
    }

    #[test]
    fn punctuation_differences_do_not_discard_real_word_timing() {
        // LyricSelectionTests.testPunctuationDifferencesDoNotDiscardRealWordTiming
        let mut line = make_line("Sing along", 1.0, 3.0);
        line.word_timed = true;
        line.words = vec![word("Sing", 1.0, 2.0), word("along", 2.0, 3.0)];
        line.text = "Sing, along!".into();
        assert!(has_word_timing(&synced(vec![line.clone()])));
        line.words.pop();
        assert!(!valid_word_timing(&line));
    }

    #[test]
    fn tiers_and_precision() {
        let word_line = timed("a b", 0.0, 2.0, vec![word("a", 0.0, 1.0), word("b", 1.0, 2.0)]);
        let estimated = make_line("c d", 2.0, 4.0);
        let gap = make_line("♪", 4.0, 6.0);
        assert_eq!(timing_tier(&synced(vec![word_line.clone(), gap.clone()])), 3);
        assert_eq!(precision(&[word_line.clone(), gap.clone()]), WORD_BY_WORD);
        assert_eq!(timing_tier(&synced(vec![word_line.clone(), estimated.clone()])), 2);
        assert_eq!(precision(&[word_line, estimated.clone()]), MIXED_TIMING);
        assert_eq!(timing_tier(&synced(vec![estimated.clone()])), 1);
        assert_eq!(precision(&[estimated]), LINE_SYNCED);
        assert_eq!(timing_tier(&synced(vec![gap.clone()])), 0);
        assert_eq!(precision(&[gap]), LINE_SYNCED);
        assert_eq!(timing_tier(&synced(vec![])), 0);
        let plain = LyricResult::Plain {
            lines: vec!["x".into()],
            provenance: Provenance { precision: NOT_SYNCED.into(), source: "x".into() },
        };
        assert_eq!(timing_tier(&plain), 0);
        assert!(!has_word_timing(&plain));
    }

    #[test]
    fn line_index_ordered_uses_last_start_at_or_before() {
        let lines: Vec<Line> = [1.0, 2.0, 2.0, 5.0].iter().map(|s| make_line("x", *s, s + 1.0)).collect();
        let index = LineIndex::new(&lines);
        assert!(index.is_ordered());
        assert_eq!(index.line_at(0.5), None);
        assert_eq!(index.line_at(1.0), Some(0));
        assert_eq!(index.line_at(2.0), Some(2), "repeated timestamps pick the last");
        assert_eq!(index.line_at(4.9), Some(2));
        assert_eq!(index.line_at(100.0), Some(3));
        assert_eq!(index.line_at(f64::NAN), None);
        assert_eq!(LineIndex::new(&[]).line_at(3.0), None);
    }

    #[test]
    fn line_index_unordered_keeps_early_stop_scan() {
        let lines: Vec<Line> = [1.0, 5.0, 3.0, 4.0].iter().map(|s| make_line("x", *s, s + 1.0)).collect();
        let index = LineIndex::new(&lines);
        assert!(!index.is_ordered());
        assert_eq!(index.line_at(4.5), Some(0), "stops at the first later start");
        assert_eq!(index.line_at(6.0), Some(3));
        let nan: Vec<Line> = [1.0, f64::NAN, 3.0].iter().map(|s| make_line("x", *s, 9.0)).collect();
        let index = LineIndex::new(&nan);
        assert!(!index.is_ordered());
        assert_eq!(index.line_at(5.0), Some(0));
    }

    // LyricGapTimingTests.swift
    #[test]
    fn native_word_end_marks_actual_vocal_window() {
        let line = timed("Hold", 1.0, 30.0, vec![word("Hold", 1.2, 4.0)]);
        let windows = vocal_windows(&[line]);
        assert_eq!(windows, vec![VocalWindow { start: 1.2, end: 4.0 }]);
    }

    #[test]
    fn estimated_words_do_not_invent_an_earlier_vocal_end() {
        let mut line = timed("Hold", 1.0, 30.0, vec![word("Hold", 1.0, 25.0)]);
        line.word_timed = false;
        assert_eq!(vocal_windows(&[line])[0].end, 30.0);
    }

    #[test]
    fn instrumental_and_invalid_lines_are_excluded() {
        let gap = Line {
            start: 5.0,
            end: 20.0,
            text: String::new(),
            words: vec![],
            is_instrumental: true,
            word_timed: false,
            romanization: None,
        };
        assert!(vocal_windows(&[gap]).is_empty());
        let mut nan = timed("Hold", f64::NAN, 20.0, vec![word("Hold", 1.0, 3.0)]);
        nan.word_timed = false;
        assert!(vocal_windows(&[nan]).is_empty());
        let sorted = vocal_windows(&[make_line("b", 9.0, 10.0), make_line("a", 1.0, 2.0)]);
        assert_eq!(sorted[0].start, 1.0);
    }

    // Tests/geometry_test.c (vocal gap + estimated line end)
    #[test]
    fn vocal_gap_matches_c_invariants() {
        let vocals = [VocalWindow { start: 10.0, end: 14.0 }, VocalWindow { start: 22.0, end: 28.0 }];
        assert_eq!(
            vocal_gap(&vocals, 5.0, 40.0, 2.4),
            Some(VocalGap { start: 0.0, end: Some(10.0), next_vocal: Some(0) })
        );
        assert_eq!(vocal_gap(&vocals, 12.0, 40.0, 2.4), None);
        assert_eq!(
            vocal_gap(&vocals, 18.0, 40.0, 2.4),
            Some(VocalGap { start: 14.0, end: Some(22.0), next_vocal: Some(1) })
        );
        assert_eq!(
            vocal_gap(&vocals, 32.0, 40.0, 2.4),
            Some(VocalGap { start: 28.0, end: Some(40.0), next_vocal: None })
        );
        assert_eq!(vocal_gap(&vocals, 40.0, 40.0, 2.4), None);
        let breath = [VocalWindow { start: 0.0, end: 5.0 }, VocalWindow { start: 6.0, end: 10.0 }];
        assert_eq!(vocal_gap(&breath, 5.5, 20.0, 2.4), None);
        let overlapping = [
            VocalWindow { start: 0.0, end: 8.0 },
            VocalWindow { start: 4.0, end: 10.0 },
            VocalWindow { start: 14.0, end: 18.0 },
        ];
        assert_eq!(vocal_gap(&overlapping, 6.0, 22.0, 2.4), None);
        let gap = vocal_gap(&overlapping, 12.0, 22.0, 2.4).unwrap();
        assert_eq!((gap.start, gap.end), (10.0, Some(14.0)));
        assert_eq!(vocal_gap(&[], 5.0, 20.0, 2.4).unwrap().next_vocal, None);
        assert_eq!(vocal_gap(&vocals, f64::NAN, 40.0, 2.4), None);
        // unknown duration: trailing gap has no end
        assert_eq!(vocal_gap(&vocals, 32.0, 0.0, 2.4), Some(VocalGap { start: 28.0, end: None, next_vocal: None }));
        // unsorted windows are rejected
        assert_eq!(vocal_gap(&[vocals[1], vocals[0]], 5.0, 40.0, 2.4), None);
        // minimum is clamped to 0.5..=10 and NaN means 2.4
        assert!(vocal_gap(&vocals, 5.0, 40.0, f64::NAN).is_some());
        assert!(vocal_gap(&vocals, 5.0, 40.0, 50.0).is_some(), "minimum clamps to 10");
    }

    #[test]
    fn estimated_line_end_matches_c_invariants() {
        assert_eq!(estimated_line_end(10.0, 14.0, 4, 0.45), 14.0);
        assert!(estimated_line_end(10.0, 50.0, 4, 0.45) < 17.0);
        assert_eq!(estimated_line_end(10.0, 30.0, 20, 0.50), 30.0);
        assert!(estimated_line_end(10.0, 90.0, 4, f64::NAN) < 17.0);
        assert_eq!(estimated_line_end(10.0, 90.0, 0, 0.45), 90.0);
    }

    // QQGapTests.swift
    #[test]
    fn long_music_gap_does_not_extend_the_estimated_words() {
        let original = parse_lrc("[00:01]Hold on\n[00:35]The song returns", 40.0);
        let fixed = repair_qq(&original);
        assert_eq!(fixed.len(), 3);
        assert!(fixed[0].end < 10.0);
        assert!(fixed[0].words.last().unwrap().end <= fixed[0].end);
        assert!(fixed[1].is_instrumental);
        assert_eq!(fixed[1].end, 35.0);
        assert_eq!(fixed[2].start, 35.0);
    }

    #[test]
    fn normal_verse_does_not_gain_instrumental_breaks() {
        let original = parse_lrc("[00:01]Keep it moving\n[00:04]One more phrase\n[00:07]Hold the light", 10.0);
        let fixed = repair_qq(&original);
        let starts = |lines: &[Line]| lines.iter().map(|l| l.start).collect::<Vec<_>>();
        let ends = |lines: &[Line]| lines.iter().map(|l| l.end).collect::<Vec<_>>();
        assert_eq!(starts(&fixed), starts(&original));
        assert_eq!(ends(&fixed), ends(&original));
    }

    #[test]
    fn real_held_word_is_not_shortened() {
        let held = timed("Stay", 1.0, 35.0, vec![word("Stay", 1.0, 34.0)]);
        let fixed = repair_qq(&[held]);
        assert_eq!(fixed.len(), 1);
        assert_eq!(fixed[0].words[0].end, 34.0);
    }

    #[test]
    fn known_word_end_creates_gap_without_retiming_words() {
        let line = timed("Stay", 1.0, 35.0, vec![word("Stay", 1.0, 4.0)]);
        let fixed = repair_qq(&[line]);
        assert_eq!(fixed.len(), 2);
        assert_eq!(fixed[0].words[0].end, 4.0);
        assert!((fixed[0].end - 4.3).abs() < 1e-9);
        assert!(fixed[1].is_instrumental);
    }

    #[test]
    fn prepare_repairs_only_qq_and_revalidates() {
        let lines = parse_lrc("[00:01]Hold on\n[00:35]The song returns", 40.0);
        let qq = LyricResult::Synced {
            lines: lines.clone(),
            provenance: Provenance { precision: "whatever".into(), source: "QQ Music".into() },
        };
        match prepare(qq.clone(), true) {
            LyricResult::Synced { lines, provenance } => {
                assert_eq!(lines.len(), 3);
                assert_eq!(provenance.precision, LINE_SYNCED);
                assert_eq!(provenance.source, "QQ Music");
            }
            _ => panic!(),
        }
        match prepare(qq, false) {
            LyricResult::Synced { lines, .. } => assert_eq!(lines.len(), 2),
            _ => panic!(),
        }
        let other =
            LyricResult::Synced { lines, provenance: Provenance { precision: "x".into(), source: "LRCLIB".into() } };
        match prepare(other, true) {
            LyricResult::Synced { lines, .. } => assert_eq!(lines.len(), 2),
            _ => panic!(),
        }
        // A line flagged word-timed with words that do not spell it is demoted.
        let mut bad = timed("Sing along", 1.0, 3.0, vec![word("Sing", 1.0, 2.0)]);
        bad.word_timed = true;
        let good = timed("Next", 3.0, 4.0, vec![word("Next", 3.0, 3.5)]);
        match prepare(synced(vec![bad, good]), true) {
            LyricResult::Synced { lines, provenance } => {
                assert!(!lines[0].word_timed);
                assert!(lines[1].word_timed);
                assert_eq!(provenance.precision, MIXED_TIMING);
            }
            _ => panic!(),
        }
        let empty = synced(vec![]);
        assert_eq!(prepare(empty.clone(), true), empty);
    }
}
