//! LRC with optional enhanced word stamps (`LyricsFetcher.parseLRC`).

use std::sync::LazyLock;

use regex::{Captures, Regex};

use crate::model::{Line, Word};
use crate::text::{split_newlines, trim_all, trim_spaces};
use crate::timing::make_line;

// ICU's `\d` also matches non-ASCII digits, which Foundation's `Double()`
// then fails to parse; ASCII digits are the only ones that carry a time.
static LINE_STAMP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([0-9]{1,3}):([0-5][0-9])(?:[.:]([0-9]{1,3}))?\]").unwrap());
static WORD_STAMP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<([0-9]{1,3}):([0-5][0-9])(?:[.:]([0-9]{1,3}))?>").unwrap());
static OFFSET_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?im)^\s*\[offset:([+-]?[0-9]+)\]").unwrap());

/// Largest LRC accepted, in bytes.
pub const MAX_LRC_BYTES: usize = 1_000_000;

fn stamp_time(caps: &Captures) -> f64 {
    let minutes: f64 = caps[1].parse().unwrap_or(0.0);
    let seconds: f64 = caps[2].parse().unwrap_or(0.0);
    let fraction = caps.get(3).map_or("", |m| m.as_str());
    let value: f64 = fraction.parse().unwrap_or(0.0);
    minutes * 60.0 + seconds + value / 10f64.powi(fraction.len() as i32)
}

struct Marker {
    start: usize,
    end: usize,
    time: f64,
}

/// Parses LRC into lines (`LyricsFetcher.parseLRC`).
///
/// * `[offset:±ms]` shifts every stamp (positive = earlier file, later lyrics:
///   the value is *added*, as on macOS); times clamp at 0.
/// * A row may start with several `[mm:ss.xx]` stamps; each produces a line.
/// * `<mm:ss.xx>` word stamps give real word timing when there is no text
///   before the first stamp, no word starts before its line, and starts never
///   go backwards; a word's end is the next stamp (the last word ends with
///   the line). If any word would start at or after the next line, the line
///   falls back to estimated timing.
/// * A line ends where the next begins; the last ends at
///   `max(track_duration, start + 4)`.
/// * Rows without a leading stamp are ignored; input over 1 MB gives nothing.
pub fn parse_lrc(text: &str, track_duration: f64) -> Vec<Line> {
    if text.len() > MAX_LRC_BYTES {
        return Vec::new();
    }
    let mut offset = 0.0;
    if let Some(caps) = OFFSET_TAG.captures(text) {
        let value: f64 = caps[1].parse().unwrap_or(0.0);
        offset = if value.is_finite() { value / 1000.0 } else { 0.0 };
    }
    let mut parsed: Vec<(f64, String, Vec<Word>)> = Vec::new();
    for raw in split_newlines(text) {
        let row = trim_spaces(raw);
        let mut leading: Vec<f64> = Vec::new();
        let mut end = 0;
        for caps in LINE_STAMP.captures_iter(row) {
            let whole = caps.get(0).unwrap();
            if whole.start() != end {
                break;
            }
            leading.push(stamp_time(&caps));
            end = whole.end();
        }
        let Some(&first) = leading.first() else { continue };
        let body = trim_spaces(&row[end..]);
        let markers: Vec<Marker> = WORD_STAMP
            .captures_iter(body)
            .map(|caps| {
                let whole = caps.get(0).unwrap();
                Marker { start: whole.start(), end: whole.end(), time: stamp_time(&caps) }
            })
            .collect();
        let plain = WORD_STAMP.replace_all(body, "").into_owned();
        let prefix = markers.first().map_or("", |m| trim_spaces(&body[..m.start]));
        for &tag in &leading {
            let begin = (tag + offset).max(0.0);
            let shift = tag - first + offset;
            let mut words: Vec<Word> = Vec::new();
            for (index, marker) in markers.iter().enumerate() {
                let to = markers.get(index + 1).map_or(body.len(), |next| next.start);
                let label = trim_all(&body[marker.end..to]);
                if label.is_empty() {
                    continue;
                }
                let start = (marker.time + shift).max(0.0);
                let finish = markers.get(index + 1).map_or(f64::MAX, |next| start.max(next.time + shift));
                words.push(Word { text: label.to_string(), start, end: finish });
            }
            let valid = prefix.is_empty()
                && words.iter().enumerate().all(|(i, w)| w.start >= begin && (i == 0 || w.start >= words[i - 1].start));
            parsed.push((begin, plain.clone(), if valid { words } else { Vec::new() }));
        }
    }
    parsed.sort_by(|a, b| a.0.total_cmp(&b.0));
    let starts: Vec<f64> = parsed.iter().map(|p| p.0).collect();
    parsed
        .into_iter()
        .enumerate()
        .map(|(index, (start, text, words))| {
            let end = starts.get(index + 1).copied().unwrap_or_else(|| track_duration.max(start + 4.0));
            let mut line = make_line(&text, start, end);
            if !words.is_empty() && words.iter().all(|w| w.start < end) {
                line.words = words
                    .into_iter()
                    .map(|w| {
                        let clipped = if w.end < end { w.end } else { end };
                        Word { start: w.start, end: w.start.max(clipped), text: w.text }
                    })
                    .collect();
                line.word_timed = true;
            }
            line
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timing::{precision, LINE_SYNCED, MIXED_TIMING};

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    // LyricParsingTests.swift
    #[test]
    fn enhanced_lrc_uses_actual_word_starts_and_ends() {
        let lines = parse_lrc("[00:01.00]<00:01.00>Hold <00:02.00>on<00:04.00>\n[00:06.00]Next line", 12.0);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].word_timed);
        assert_eq!(lines[0].text, "Hold on");
        assert!(approx(lines[0].words[0].start, 1.0));
        assert!(approx(lines[0].words[0].end, 2.0));
        assert!(approx(lines[0].words[1].end, 4.0));
        assert!(!lines[1].word_timed);
        assert!(approx(lines[1].end, 12.0));
        assert_eq!(precision(&lines), MIXED_TIMING);
    }

    #[test]
    fn line_only_data_is_explicitly_estimated() {
        let lines = parse_lrc("[00:01]A small line\n[00:05]Another line", 10.0);
        assert_eq!(lines.len(), 2);
        assert!(!lines[0].word_timed);
        assert_eq!(precision(&lines), LINE_SYNCED);
        assert!(lines[0].words.iter().all(|w| w.start >= 1.0 && w.end <= 5.0));
    }

    #[test]
    fn offsets_and_repeated_timestamps() {
        let lines = parse_lrc("[offset:250]\n[00:01.00][00:10.00]Again\n[00:05.00]", 15.0);
        let starts: Vec<f64> = lines.iter().map(|l| l.start).collect();
        assert_eq!(starts, vec![1.25, 5.25, 10.25]);
        assert!(lines[1].is_instrumental);
        assert_eq!(lines[2].text, "Again");
        assert!(approx(lines[2].end, 15.0));
    }

    #[test]
    fn malformed_word_timing_falls_back_safely() {
        let lines = parse_lrc("[00:01]<00:04>Backwards <00:02>words", 8.0);
        assert_eq!(lines.len(), 1);
        assert!(!lines[0].word_timed);
        assert_eq!(lines[0].text, "Backwards words");
    }

    #[test]
    fn word_stamps_beyond_the_next_line_fall_back_to_estimated_timing() {
        let lines = parse_lrc("[00:01]<00:01>one <00:10>two\n[00:04]next", 8.0);
        assert!(!lines[0].word_timed);
        assert!(lines[0].words.iter().all(|w| w.end <= 4.0));
    }

    #[test]
    fn negative_offset_clamps_at_zero() {
        let lines = parse_lrc("[offset:-2000]\n[00:01]A line", 8.0);
        assert_eq!(lines[0].start, 0.0);
    }

    #[test]
    fn untimed_and_oversized_files_are_rejected() {
        assert!(parse_lrc("Plain untimed text", 10.0).is_empty());
        assert!(parse_lrc(&"x".repeat(1_000_001), 10.0).is_empty());
        assert!(parse_lrc("", 10.0).is_empty());
    }

    #[test]
    fn stamp_formats_and_fractions() {
        let lines = parse_lrc("[01:02.5]a\n[01:03.50]b\n[01:04.500]c\n[01:05:05]d\n[100:00]e", 0.0);
        let starts: Vec<f64> = lines.iter().map(|l| l.start).collect();
        assert!(approx(starts[0], 62.5));
        assert!(approx(starts[1], 63.5));
        assert!(approx(starts[2], 64.5));
        assert!(approx(starts[3], 65.05));
        assert!(approx(starts[4], 6000.0));
        // last line: max(0, start + 4)
        assert!(approx(lines[4].end, 6004.0));
    }

    #[test]
    fn metadata_and_bad_stamps_are_ignored() {
        let text = "[ti:Title]\n[ar:Artist]\n[00:61]bad seconds\n[0:05] spaced \n  [00:07]indented\nno stamp\n[00:09]x [00:10]not leading";
        let lines = parse_lrc(text, 20.0);
        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["spaced", "indented", "x [00:10]not leading"]);
    }

    #[test]
    fn multiple_stamps_shift_word_times() {
        let lines = parse_lrc("[00:01][00:21]<00:01>la <00:02>la\n[00:10]x\n[00:30]y", 40.0);
        assert_eq!(lines.len(), 4);
        assert!(lines[0].word_timed && lines[2].word_timed);
        assert!(approx(lines[2].start, 21.0));
        assert!(approx(lines[2].words[0].start, 21.0));
        assert!(approx(lines[2].words[1].start, 22.0));
        assert!(approx(lines[2].words[1].end, 30.0), "last word ends with the line");
    }

    #[test]
    fn text_before_first_word_stamp_invalidates_word_timing() {
        let lines = parse_lrc("[00:01]Hey <00:02>you", 10.0);
        assert!(!lines[0].word_timed);
        assert_eq!(lines[0].text, "Hey you");
    }

    #[test]
    fn word_before_line_start_is_rejected() {
        let lines = parse_lrc("[00:05]<00:04>early <00:06>word", 10.0);
        assert!(!lines[0].word_timed);
    }

    #[test]
    fn offset_applies_to_word_stamps() {
        let lines = parse_lrc("[offset:+500]\n[00:01]<00:01>a <00:02>b", 10.0);
        assert!(lines[0].word_timed);
        assert!(approx(lines[0].start, 1.5));
        assert!(approx(lines[0].words[1].start, 2.5));
        let first_tag_offset = parse_lrc("[OFFSET:100]\n[offset:900]\n[00:01]a", 10.0);
        assert!(approx(first_tag_offset[0].start, 1.1), "first offset tag wins, case-insensitive");
    }

    #[test]
    fn crlf_and_unicode_text() {
        let lines = parse_lrc("[00:01.00]夜に駆ける\r\n[00:03.00]♪\r\n[00:05.00]Ünïcødé", 9.0);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].text, "夜に駆ける");
        assert!(lines[1].is_instrumental);
        assert_eq!(lines[2].text, "Ünïcødé");
    }

    #[test]
    fn empty_trailing_word_stamp_closes_the_previous_word() {
        let lines = parse_lrc("[00:01]<00:01>a <00:01.5> <00:02>b<00:03>", 10.0);
        assert!(lines[0].word_timed);
        let texts: Vec<&str> = lines[0].words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["a", "b"]);
        assert!(approx(lines[0].words[0].end, 1.5));
        assert!(approx(lines[0].words[1].end, 3.0));
    }
}
