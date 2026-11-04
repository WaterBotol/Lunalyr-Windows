//! Exported TTML (`TTMLLyricParser.swift`). Timed leaf `<span>`s become
//! words; paragraphs whose spans do not cover all their text keep estimated
//! timing. Translation / romanisation spans and `<metadata>` are skipped;
//! DTDs and entity declarations are refused.

use super::xml::{self, Event};
use crate::model::{Line, Word};
use crate::text::{trim_all, trim_spaces, without_whitespace};
use crate::timing::make_line;

/// Largest document accepted, in bytes.
pub const MAX_TTML_BYTES: usize = 2_000_000;
const MAX_DEPTH: usize = 64;
const MAX_LINES: usize = 15_000;

/// A TTML clock value in seconds (`TTMLLyricParser.seconds`): `1250ms`,
/// `1.25s`, or `[[hh:]mm:]ss(.fff)`; finite and within 0..=864000.
pub fn ttml_seconds(value: &str) -> Option<f64> {
    let text = trim_spaces(value);
    let result = if let Some(ms) = text.strip_suffix("ms") {
        ms.parse::<f64>().ok().map(|v| v / 1000.0)
    } else if let Some(s) = text.strip_suffix('s') {
        s.parse::<f64>().ok()
    } else {
        let parts: Vec<&str> = text.split(':').collect();
        let values: Option<Vec<f64>> = parts.iter().map(|p| p.parse::<f64>().ok()).collect();
        match values {
            Some(values) if (1..=3).contains(&values.len()) => Some(values.iter().fold(0.0, |acc, v| acc * 60.0 + v)),
            _ => None,
        }
    }?;
    (result.is_finite() && (0.0..=864_000.0).contains(&result)).then_some(result)
}

struct Span {
    start: Option<f64>,
    end: Option<f64>,
    text: String,
    timed_child: bool,
}

#[derive(Default)]
struct State {
    lines: Vec<Line>,
    paragraph: Option<(f64, f64)>,
    text: String,
    words: Vec<Word>,
    spans: Vec<Span>,
    skip_depth: usize,
    depth: usize,
}

fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn attribute<'a>(attributes: &'a [(&str, String)], key: &str) -> Option<&'a str> {
    attributes.iter().find(|(k, _)| *k == key).map(|(_, v)| v.as_str())
}

impl State {
    fn start(&mut self, name: &str, attributes: &[(&str, String)], duration: f64) -> bool {
        self.depth += 1;
        if self.depth > MAX_DEPTH || self.lines.len() >= MAX_LINES {
            return false;
        }
        if self.skip_depth > 0 {
            self.skip_depth += 1;
            return true;
        }
        let name = local_name(name);
        let role =
            attributes.iter().find(|(k, _)| *k == "role" || k.ends_with(":role")).map_or("", |(_, v)| v.as_str());
        if name == "metadata" || matches!(role, "x-translation" | "x-roman" | "x-romanization") {
            self.skip_depth = 1;
            return true;
        }
        match name {
            "p" => {
                let Some(start) = attribute(attributes, "begin").and_then(ttml_seconds) else { return true };
                let end = attribute(attributes, "end")
                    .and_then(ttml_seconds)
                    .or_else(|| attribute(attributes, "dur").and_then(ttml_seconds).map(|d| start + d));
                let Some(end) = end else { return true };
                if end > start && (duration <= 0.0 || start <= duration + 8.0) {
                    self.paragraph = Some((start, end));
                    self.text.clear();
                    self.words.clear();
                    self.spans.clear();
                }
            }
            "span" => {
                if let Some((paragraph_start, _)) = self.paragraph {
                    let start = attribute(attributes, "begin").and_then(ttml_seconds);
                    let end = attribute(attributes, "end").and_then(ttml_seconds).or_else(|| {
                        attribute(attributes, "dur")
                            .and_then(ttml_seconds)
                            .map(|d| start.unwrap_or(paragraph_start) + d)
                    });
                    self.spans.push(Span { start, end, text: String::new(), timed_child: false });
                }
            }
            "br" if self.paragraph.is_some() => self.text.push(' '),
            _ => {}
        }
        true
    }

    fn characters(&mut self, text: &str) {
        if self.skip_depth > 0 || self.paragraph.is_none() {
            return;
        }
        self.text.push_str(text);
        for span in &mut self.spans {
            span.text.push_str(text);
        }
    }

    fn end(&mut self, name: &str) {
        self.depth = self.depth.saturating_sub(1);
        if self.skip_depth > 0 {
            self.skip_depth -= 1;
            return;
        }
        match local_name(name) {
            "span" => {
                let Some(span) = self.spans.pop() else { return };
                let Some((p_start, p_end)) = self.paragraph else { return };
                let (Some(start), Some(end)) = (span.start, span.end) else { return };
                if span.timed_child || end <= start || start < p_start - 0.02 || end > p_end + 0.02 {
                    return;
                }
                let label = trim_all(&span.text);
                if !label.is_empty() {
                    self.words.push(Word { text: label.to_string(), start: start.max(p_start), end: end.min(p_end) });
                    for open in &mut self.spans {
                        open.timed_child = true;
                    }
                }
            }
            "p" => {
                let Some((start, end)) = self.paragraph.take() else { return };
                let label = self.text.split_whitespace().collect::<Vec<_>>().join(" ");
                if !label.is_empty() {
                    let mut line = make_line(&label, start, end);
                    let covered: String = self.words.iter().map(|w| w.text.as_str()).collect();
                    if !self.words.is_empty() && without_whitespace(&covered) == without_whitespace(&label) {
                        let mut words = std::mem::take(&mut self.words);
                        words.sort_by(|a, b| a.start.total_cmp(&b.start));
                        line.words = words;
                        line.word_timed = true;
                    }
                    self.lines.push(line);
                }
                self.spans.clear();
                self.words.clear();
                self.text.clear();
            }
            _ => {}
        }
    }
}

/// Parses exported TTML (`TTMLLyricParser.parse`). `duration` (seconds,
/// 0 = unknown) drops paragraphs starting more than 8 s past the end.
/// Malformed XML, DTDs, over 2 MB, nesting deeper than 64 or more than
/// 15 000 lines give no lines at all.
pub fn parse_ttml(text: &str, duration: f64) -> Vec<Line> {
    let lower = text.to_lowercase();
    if text.len() > MAX_TTML_BYTES || lower.contains("<!doctype") || lower.contains("<!entity") {
        return Vec::new();
    }
    let mut state = State::default();
    let parsed = xml::read(text, |event| match event {
        Event::Start { name, attributes } => state.start(name, &attributes, duration),
        Event::End { name } => {
            state.end(name);
            true
        }
        Event::Text(text) => {
            state.characters(&text);
            true
        }
    });
    if parsed.is_err() {
        return Vec::new();
    }
    let mut lines = state.lines;
    lines.sort_by(|a, b| a.start.total_cmp(&b.start));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    // ReadingAndTTMLTests.swift
    #[test]
    fn leaf_spans_keep_word_times() {
        let xml = r#"<tt xmlns="http://www.w3.org/ns/ttml"><body><div>
        <p begin="00:01.000" end="00:03.000"><span begin="1s" end="1.4s">Hold </span><span begin="1.5s" end="3s">on</span></p>
        </div></body></tt>"#;
        let lines = parse_ttml(xml, 10.0);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].word_timed);
        let texts: Vec<&str> = lines[0].words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["Hold", "on"]);
        assert!((lines[0].words[1].start - 1.5).abs() < 1e-9);
        assert!((lines[0].words[1].end - 3.0).abs() < 1e-9);
        assert_eq!(lines[0].text, "Hold on");
    }

    #[test]
    fn partial_word_coverage_uses_estimated_timing() {
        let xml = "<tt><body><p begin='1s' end='4s'>Keep <span begin='2s' end='3s'>moving</span> now</p></body></tt>";
        let line = &parse_ttml(xml, 10.0)[0];
        assert!(!line.word_timed);
        assert_eq!(line.text, "Keep moving now");
        assert_eq!(line.words.len(), 3);
    }

    #[test]
    fn translation_spans_are_not_lyrics() {
        let xml = r#"<tt xmlns:ttm="http://www.w3.org/ns/ttml#metadata"><body><p begin="1s" end="3s"><span begin="1s" end="3s">Hello</span><span ttm:role="x-translation">Translation</span></p></body></tt>"#;
        let line = &parse_ttml(xml, 10.0)[0];
        assert_eq!(line.text, "Hello");
        assert!(line.word_timed);
    }

    #[test]
    fn rejects_entities_and_invalid_times() {
        let xml =
            "<!DOCTYPE tt [<!ENTITY secret SYSTEM 'file:///etc/passwd'>]><tt><p begin='1s' end='2s'>&secret;</p></tt>";
        assert!(parse_ttml(xml, 10.0).is_empty());
        assert_eq!(ttml_seconds("nan"), None);
        assert_eq!(ttml_seconds("-1s"), None);
        assert_eq!(ttml_seconds("1250ms"), Some(1.25));
        assert_eq!(ttml_seconds(" 1:02:03.5 "), Some(3723.5));
        assert_eq!(ttml_seconds("00:01.000"), Some(1.0));
        assert_eq!(ttml_seconds("1:2:3:4"), None);
        assert_eq!(ttml_seconds("864001"), None);
        assert_eq!(ttml_seconds("1.5h"), None);
    }

    #[test]
    fn nested_spans_dur_br_metadata_and_order() {
        let xml = r#"<tt><head><metadata><ttm:title>Ignored</ttm:title></metadata></head><body>
            <p begin="5s" dur="2s">Second<br/>line</p>
            <p begin="1s" end="4s"><span begin="1s" end="4s"><span begin="1s" dur="1s">Two</span> <span begin="2s" end="3.5s">words</span></span></p>
            <p begin="9s" end="8s">backwards</p>
            <p begin="30s" end="31s">past the end</p>
        </body></tt>"#;
        let lines = parse_ttml(xml, 20.0);
        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["Two words", "Second line"]);
        assert!(lines[0].word_timed, "the outer span holds timed children, so it is not a word");
        assert_eq!(lines[0].words.len(), 2);
        assert!((lines[1].end - 7.0).abs() < 1e-9);
        assert!(!lines[1].word_timed);
    }

    #[test]
    fn zero_width_space_trims_like_foundation_whitespace() {
        // A paragraph of only U+200B is an instrumental break, as on macOS.
        let xml = "<tt><body><p begin='1s' end='3s'><span begin='1s' end='2s'>Hold</span> <span begin='2s' end='3s'>on</span></p><p begin='3s' end='5s'>\u{200B}</p></body></tt>";
        let lines = parse_ttml(xml, 10.0);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].is_instrumental);
        assert_eq!(crate::timing::precision(&lines), crate::timing::WORD_BY_WORD);
        // Span labels lose it, but the coverage check (Character.isWhitespace)
        // keeps it in the paragraph text, so as on macOS the timing is estimated.
        let xml = "<tt><body><p begin='5s' end='6s'><span begin='5s' end='6s'>\u{200B}go</span></p></body></tt>";
        let line = &parse_ttml(xml, 10.0)[0];
        assert_eq!(line.text, "go");
        assert!(!line.word_timed);
    }

    #[test]
    fn malformed_xml_gives_nothing() {
        assert!(parse_ttml("<tt><p begin='1s' end='2s'>x</tt>", 0.0).is_empty());
        assert!(parse_ttml("<tt><p begin='1s' end='2s'>&bogus;</p></tt>", 0.0).is_empty());
        let deep = format!("<tt>{}{}</tt>", "<div>".repeat(70), "</div>".repeat(70));
        assert!(parse_ttml(&deep, 0.0).is_empty());
    }
}
