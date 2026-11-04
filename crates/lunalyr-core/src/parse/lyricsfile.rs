//! LRCLIB's `lyricsfile` YAML (`LyricsFetcher.parseLyricsFile`):
//!
//! ```yaml
//! metadata:
//!   offset_ms: 0
//! lines:
//! - text: Every word in place
//!   start_ms: 27160
//!   end_ms: 29960
//!   words:
//!   - text: Every
//!     start_ms: 27160
//!     end_ms: 27600
//! ```

use super::yaml::{self, Yaml};
use crate::model::{Line, Word};
use crate::text::{canonical_eq, trim_all, without_whitespace};
use crate::timing::make_line;

/// Largest document accepted, in bytes.
pub const MAX_LYRICSFILE_BYTES: usize = 1_000_000;

struct Token {
    text: String,
    start_ms: f64,
    end_ms: Option<f64>,
}

struct Row {
    text: String,
    start_ms: f64,
    end_ms: Option<f64>,
    words: Vec<Token>,
}

fn required_number(node: &Yaml, key: &str) -> Result<f64, ()> {
    node.get(key).ok_or(())?.as_f64()?.ok_or(())
}

fn optional_number(node: &Yaml, key: &str) -> Result<Option<f64>, ()> {
    node.get(key).map_or(Ok(None), Yaml::as_f64)
}

fn required_text(node: &Yaml, key: &str) -> Result<String, ()> {
    node.get(key).ok_or(())?.as_text().ok_or(())
}

/// Like the Swift `Decodable` model: a missing or mistyped required field
/// anywhere rejects the whole document.
fn decode(doc: &Yaml) -> Result<(f64, Vec<Row>), ()> {
    if !matches!(doc, Yaml::Map(_)) {
        return Err(());
    }
    let offset_ms = match doc.get("metadata") {
        None => 0.0,
        Some(m) if m.is_null() => 0.0,
        Some(m @ Yaml::Map(_)) => optional_number(m, "offset_ms")?.unwrap_or(0.0),
        Some(_) => return Err(()),
    };
    let rows = match doc.get("lines") {
        None => Vec::new(),
        Some(l) if l.is_null() => Vec::new(),
        Some(Yaml::Seq(items)) => items
            .iter()
            .map(|item| {
                if !matches!(item, Yaml::Map(_)) {
                    return Err(());
                }
                let words = match item.get("words") {
                    None => Vec::new(),
                    Some(w) if w.is_null() => Vec::new(),
                    Some(Yaml::Seq(tokens)) => tokens
                        .iter()
                        .map(|t| {
                            if !matches!(t, Yaml::Map(_)) {
                                return Err(());
                            }
                            Ok(Token {
                                text: required_text(t, "text")?,
                                start_ms: required_number(t, "start_ms")?,
                                end_ms: optional_number(t, "end_ms")?,
                            })
                        })
                        .collect::<Result<_, _>>()?,
                    Some(_) => return Err(()),
                };
                Ok(Row {
                    text: required_text(item, "text")?,
                    start_ms: required_number(item, "start_ms")?,
                    end_ms: optional_number(item, "end_ms")?,
                    words,
                })
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err(()),
    };
    Ok((offset_ms, rows))
}

/// Parses an LRCLIB lyricsfile. Rows are sorted by start; a row without
/// `end_ms` ends where the next starts (the last after 4 s). Word timing is
/// used only when the words spell the row (ignoring whitespace), sit inside
/// it, and do not overlap; otherwise the line keeps estimated timing.
/// Invalid documents give no lines.
pub fn parse_lyricsfile(text: &str) -> Vec<Line> {
    if text.len() > MAX_LYRICSFILE_BYTES {
        return Vec::new();
    }
    let Ok((offset_ms, rows)) = decode(&yaml::parse(text)) else { return Vec::new() };
    let offset = offset_ms / 1000.0;
    if !offset.is_finite() {
        return Vec::new();
    }
    let mut rows: Vec<Row> = rows.into_iter().filter(|r| r.start_ms.is_finite() && r.start_ms >= 0.0).collect();
    rows.sort_by(|a, b| a.start_ms.total_cmp(&b.start_ms));
    let mut lines = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let start = (row.start_ms / 1000.0 + offset).max(0.0);
        let end = match row.end_ms {
            Some(ms) => ms / 1000.0 + offset,
            None => rows.get(index + 1).map_or(start + 4.0, |next| next.start_ms / 1000.0 + offset),
        };
        if !end.is_finite() || end <= start {
            continue;
        }
        let mut line = make_line(&row.text, start, end);
        let words: Vec<Word> = row
            .words
            .iter()
            .enumerate()
            .map(|(i, token)| Word {
                text: trim_all(&token.text).to_string(),
                start: token.start_ms / 1000.0 + offset,
                end: match token.end_ms {
                    Some(ms) => ms / 1000.0 + offset,
                    None => row.words.get(i + 1).map_or(end, |next| next.start_ms / 1000.0 + offset),
                },
            })
            .collect();
        let covered: String = without_whitespace(&words.iter().map(|w| w.text.as_str()).collect::<String>());
        let spelled = canonical_eq(&covered, &without_whitespace(&row.text));
        let fits = words.iter().enumerate().all(|(i, w)| {
            w.start.is_finite()
                && w.end.is_finite()
                && w.start >= start
                && w.end > w.start
                && w.end <= end
                && (i == 0 || w.start >= words[i - 1].end)
        });
        if !words.is_empty() && spelled && fits {
            line.words = words;
            line.word_timed = true;
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timing::{precision, valid_word_timing, MIXED_TIMING};

    const SAMPLE: &str = r#"version: '1.0'
metadata:
  title: Paper Lanterns
  offset_ms: 0
lines:
- text: Every word in place
  start_ms: 27160
  end_ms: 29960
  words:
  - text: 'Every '
    start_ms: 27160
    end_ms: 27600
  - text: 'word '
    start_ms: 27600
    end_ms: 28000
  - text: 'in '
    start_ms: 28000
    end_ms: 28300
  - text: place
    start_ms: 28300
    end_ms: 29900
- text: "Only the line is \"timed\" here"
  start_ms: 30500
- text: ♪
  start_ms: 33000
  end_ms: 36000
"#;

    #[test]
    fn parses_words_lines_and_instrumentals() {
        let lines = parse_lyricsfile(SAMPLE);
        assert_eq!(lines.len(), 3);
        assert!(lines[0].word_timed);
        assert!(valid_word_timing(&lines[0]));
        let texts: Vec<&str> = lines[0].words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["Every", "word", "in", "place"]);
        assert!((lines[0].start - 27.16).abs() < 1e-9);
        assert!((lines[0].end - 29.96).abs() < 1e-9);
        assert_eq!(lines[1].text, "Only the line is \"timed\" here");
        assert!(!lines[1].word_timed);
        assert!((lines[1].end - 33.0).abs() < 1e-9, "no end_ms: next start");
        assert!(lines[2].is_instrumental);
        assert_eq!(precision(&lines), MIXED_TIMING);
    }

    #[test]
    fn offset_and_missing_word_ends() {
        let doc = "metadata:\n  offset_ms: -500\nlines:\n  - text: a b\n    start_ms: 1000\n    words:\n      - {text: a, start_ms: 1000}\n      - {text: b, start_ms: 1500}\n";
        let lines = parse_lyricsfile(doc);
        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        assert!((line.start - 0.5).abs() < 1e-9);
        assert!((line.end - 4.5).abs() < 1e-9, "last row: start + 4");
        assert!(line.word_timed);
        assert!((line.words[0].end - 1.0).abs() < 1e-9, "next token start");
        assert!((line.words[1].end - 4.5).abs() < 1e-9, "line end");
    }

    #[test]
    fn overlapping_or_partial_words_fall_back_to_estimates() {
        let overlapping = "lines:\n- text: a b\n  start_ms: 0\n  end_ms: 2000\n  words:\n  - text: a\n    start_ms: 0\n    end_ms: 1200\n  - text: b\n    start_ms: 1000\n    end_ms: 1800\n";
        assert!(!parse_lyricsfile(overlapping)[0].word_timed);
        let partial = "lines:\n- text: a b c\n  start_ms: 0\n  end_ms: 2000\n  words:\n  - text: a\n    start_ms: 0\n    end_ms: 500\n";
        let line = &parse_lyricsfile(partial)[0];
        assert!(!line.word_timed);
        assert_eq!(line.words.len(), 3);
        let punctuation = "lines:\n- text: 'a, b'\n  start_ms: 0\n  end_ms: 2000\n  words:\n  - text: a\n    start_ms: 0\n    end_ms: 500\n  - text: b\n    start_ms: 600\n    end_ms: 900\n";
        assert!(!parse_lyricsfile(punctuation)[0].word_timed, "unlike LRC, punctuation must match");
    }

    #[test]
    fn rows_are_sorted_and_invalid_rows_dropped() {
        let doc = "lines:\n- text: second\n  start_ms: 5000\n- text: first\n  start_ms: 1000\n- text: negative\n  start_ms: -5\n- text: backwards\n  start_ms: 9000\n  end_ms: 8000\n";
        let lines = parse_lyricsfile(doc);
        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["first", "second"]);
        assert!((lines[1].end - 9.0).abs() < 1e-9);
    }

    #[test]
    fn invalid_documents_give_nothing() {
        assert!(parse_lyricsfile("").is_empty());
        assert!(parse_lyricsfile("just text").is_empty());
        assert!(parse_lyricsfile("lines:\n- text: a\n").is_empty(), "start_ms is required");
        assert!(parse_lyricsfile("lines:\n- start_ms: 5\n").is_empty(), "text is required");
        assert!(parse_lyricsfile("lines:\n- text: a\n  start_ms: soon\n").is_empty());
        assert!(parse_lyricsfile("metadata: 5\nlines:\n- text: a\n  start_ms: 1\n").is_empty());
        assert!(parse_lyricsfile("metadata:\n  offset_ms: .inf\nlines:\n- text: a\n  start_ms: 1\n").is_empty());
        let big = format!("lines:\n- text: '{}'\n  start_ms: 1\n", "x".repeat(1_000_000));
        assert!(parse_lyricsfile(&big).is_empty());
    }

    #[test]
    fn folded_long_text_and_escapes() {
        let doc = "lines:\n- text: \"A long line that an emitter\n    folded across two lines \\u2014 with an escape\"\n  start_ms: 1000\n  end_ms: 4000\n- text: plain words that also\n    continue here\n  start_ms: 5000\n";
        let lines = parse_lyricsfile(doc);
        assert_eq!(lines[0].text, "A long line that an emitter folded across two lines — with an escape");
        assert_eq!(lines[1].text, "plain words that also continue here");
    }
}
