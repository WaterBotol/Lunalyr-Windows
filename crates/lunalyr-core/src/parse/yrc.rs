//! NetEase's word-level karaoke format (`NetEaseLyricsProvider.parseYRC`):
//!
//! ```text
//! [32820,4800](32820,540,0)Maybe (33360,120,0)you (33480,180,0)can
//! ```
//!
//! `[lineStartMs,lineDurationMs]` followed by `(wordStartMs,wordDurationMs,0)word`.

use std::sync::LazyLock;

use regex::Regex;
use unicode_segmentation::UnicodeSegmentation;

use crate::model::{Line, Word};
use crate::text::{grapheme_is_alnum, trim_spaces};

static HEADER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[([0-9]+),([0-9]+)\]").unwrap());
static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\(([0-9]+),([0-9]+),[0-9]+\)([^(]*)").unwrap());

/// Longest a single sung word may hold. Real sustains top out around 3 s;
/// beyond 5 s is bad data (a lone "!" parked across an instrumental).
pub const MAX_WORD_DURATION: f64 = 5.0;
/// Shortest word duration.
pub const MIN_WORD_DURATION: f64 = 0.06;

/// NetEase prefixes most tracks with production credits in Chinese; they
/// are timed like lyrics but are not lyrics.
const CREDIT_MARKERS: [&str; 16] = [
    "制作人",
    "作词",
    "作曲",
    "编曲",
    "混音",
    "录音",
    "母带",
    "出品",
    "发行",
    "监制",
    "吉他",
    "贝斯",
    "鼓",
    "键盘",
    "和声",
    "制作",
];

fn is_credits(text: &str) -> bool {
    CREDIT_MARKERS.iter().any(|marker| text.contains(marker))
}

/// Parses YRC (`NetEaseLyricsProvider.parseYRC`). Bare punctuation tokens
/// fold into the previous word (keeping that word's end); word durations
/// are clamped to 0.06–5 s; credit lines and rows without a header are
/// dropped. Lines with no words are marked instrumental.
///
/// Rows are split on LF with a trailing CR removed (the Swift split on "\n"
/// would leave CRLF text as one row; NetEase sends LF).
pub fn parse_yrc(yrc: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    for raw in yrc.split('\n') {
        let s = raw.strip_suffix('\r').unwrap_or(raw);
        if s.is_empty() {
            continue;
        }
        let Some(header) = HEADER.captures(s) else { continue };
        let (Ok(start_ms), Ok(duration_ms)) = (header[1].parse::<f64>(), header[2].parse::<f64>()) else { continue };
        let mut words: Vec<Word> = Vec::new();
        let mut full_text = String::new();
        for caps in WORD.captures_iter(s) {
            let (Ok(word_start), Ok(word_duration)) = (caps[1].parse::<f64>(), caps[2].parse::<f64>()) else {
                continue;
            };
            let piece = &caps[3];
            full_text.push_str(piece);
            let trimmed = trim_spaces(piece);
            if trimmed.is_empty() {
                continue;
            }
            let start = word_start / 1000.0;
            let end = start + (word_duration / 1000.0).clamp(MIN_WORD_DURATION, MAX_WORD_DURATION);
            let punctuation_only = trimmed.graphemes(true).all(|g| !grapheme_is_alnum(g));
            if punctuation_only {
                if let Some(previous) = words.last_mut() {
                    previous.text.push_str(trimmed);
                    continue;
                }
            }
            words.push(Word { text: trimmed.to_string(), start, end });
        }
        let cleaned = trim_spaces(&full_text);
        if cleaned.is_empty() || is_credits(cleaned) {
            continue;
        }
        let has_words = !words.is_empty();
        lines.push(Line {
            start: start_ms / 1000.0,
            end: (start_ms + duration_ms) / 1000.0,
            text: cleaned.to_string(),
            words,
            is_instrumental: !has_words,
            word_timed: has_words,
            romanization: None,
        });
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timing::valid_word_timing;

    #[test]
    fn parses_words_and_line_window() {
        let lines = parse_yrc("[32820,4800](32820,540,0)Maybe (33360,120,0)you (33480,180,0)can\n");
        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        assert_eq!(line.text, "Maybe you can");
        assert!((line.start - 32.82).abs() < 1e-9);
        assert!((line.end - 37.62).abs() < 1e-9);
        let texts: Vec<&str> = line.words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["Maybe", "you", "can"]);
        assert!((line.words[1].end - 33.48).abs() < 1e-9);
        assert!(line.word_timed && !line.is_instrumental);
        assert!(valid_word_timing(line));
    }

    #[test]
    fn punctuation_folds_into_previous_word() {
        let lines = parse_yrc("[1000,3000](1000,500,0)Wait(1500,400,0), (1900,30000,0)stop(31900,300,0)!");
        let words = &lines[0].words;
        let texts: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["Wait,", "stop!"]);
        assert!((words[0].end - 1.5).abs() < 1e-9, "the folded word keeps its own end");
        assert!((words[1].end - (1.9 + MAX_WORD_DURATION)).abs() < 1e-9, "5 s cap");
        assert_eq!(lines[0].text, "Wait, stop!");
    }

    #[test]
    fn minimum_duration_and_leading_punctuation() {
        let lines = parse_yrc("[0,2000](0,0,0)\"(10,0,0)Hi");
        let words = &lines[0].words;
        assert_eq!(words[0].text, "\"", "no previous word to fold into");
        assert!((words[0].end - MIN_WORD_DURATION).abs() < 1e-9);
        assert_eq!(words[1].text, "Hi");
    }

    #[test]
    fn credits_headerless_and_empty_rows_are_dropped() {
        let text = "{\"t\":0,\"c\":[{\"tx\":\"作词: \"}]}\n[0,1000](0,500,0)作词 : 某人\n[1000,1000](1000,500,0)制作人：某人\nno header\n[2000,1000](2000,500,0)   \n[3000,1500](3000,700,0)Real (3700,800,0)line\r\n";
        let lines = parse_yrc(text);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Real line");
    }

    #[test]
    fn zero_width_space_pieces_are_blank() {
        // CharacterSet.whitespaces includes U+200B: macOS drops such a row.
        assert!(parse_yrc("[1000,2000](1000,500,0)\u{200B}").is_empty());
        let lines = parse_yrc("[0,1000](0,500,0)Hi(500,200,0)\u{200B}\n[1000,1000](1000,500,0)\u{200B} ");
        assert_eq!(lines.len(), 1);
        let texts: Vec<&str> = lines[0].words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["Hi"]);
        assert_eq!(lines[0].text, "Hi");
    }

    #[test]
    fn line_without_words_is_instrumental() {
        let lines = parse_yrc("[0,1000]Text without word stamps");
        assert!(lines.is_empty(), "the text only counts inside word stamps");
        let lines = parse_yrc("[0,1000](0,200,0)   (200,200,0)♪");
        assert_eq!(lines.len(), 1);
        assert!(!lines[0].is_instrumental, "a glyph token is still a word");
    }
}
