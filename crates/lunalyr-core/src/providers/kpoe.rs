//! Lyrics+ / KPoe community nodes (`KPoeLyricsProvider.swift`): syllable
//! timed lyrics (often Apple Music's own) with no key. Mirrors are tried in
//! order; failures fall through to the next mirror and finally read as
//! "no match".

use std::io::Read;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use super::{Provider, ProviderError};
use crate::http::{build_url, HttpClient};
use crate::model::{Candidate, Line, LyricQuery, LyricResult, Provenance, SourceId, Word};
use crate::text::trim_spaces;
use crate::timing::precision;

/// Community mirrors, tried in order.
pub const HOSTS: [&str; 3] =
    ["https://lyricsplus.prjktla.workers.dev", "https://lyricsplus.binimum.org", "https://lyricsplus.prjktla.my.id"];
/// Per mirror (macOS used a 4 s idle timeout).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(4);
const USER_AGENT: &str = "Lunalyr-Windows/0.1 (Windows now-playing lyrics)";
/// Inflated bodies larger than this are discarded.
const MAX_INFLATED_BYTES: u64 = 16_000_000;

#[derive(Deserialize)]
struct Response {
    lyrics: Option<Vec<Entry>>,
    metadata: Option<Metadata>,
}

#[derive(Deserialize)]
struct Metadata {
    source: Option<String>,
}

#[derive(Deserialize)]
struct Entry {
    time: f64,
    duration: f64,
    text: String,
    syllabus: Option<Vec<Syllable>>,
}

#[derive(Deserialize)]
struct Syllable {
    time: f64,
    duration: f64,
    text: String,
}

/// These nodes may serve a gzip body without `Content-Encoding`.
fn gunzip_if_needed(data: Vec<u8>) -> Vec<u8> {
    if data.len() <= 18 || data[0] != 0x1f || data[1] != 0x8b {
        return data;
    }
    let mut inflated = Vec::new();
    let decoder = flate2::read::GzDecoder::new(&data[..]);
    match decoder.take(MAX_INFLATED_BYTES).read_to_end(&mut inflated) {
        Ok(_) => inflated,
        Err(_) => data,
    }
}

/// Joins syllables back into words: a trailing space ends a word, which
/// keeps its first syllable's start and its last end (≥ 60 ms long).
fn merge_syllables(syllabus: &[Syllable]) -> Vec<Word> {
    let mut words = Vec::new();
    let mut buffer = String::new();
    let mut word_start: Option<f64> = None;
    let mut word_end = 0.0f64;
    let mut flush = |buffer: &mut String, word_start: &mut Option<f64>, word_end: &mut f64| {
        let trimmed = trim_spaces(buffer);
        if let (false, Some(begin)) = (trimmed.is_empty(), *word_start) {
            words.push(Word { text: trimmed.to_string(), start: begin, end: word_end.max(begin + 0.06) });
        }
        buffer.clear();
        *word_start = None;
        *word_end = 0.0;
    };
    for syllable in syllabus {
        let start = syllable.time / 1000.0;
        let end = (syllable.time + syllable.duration.max(1.0)) / 1000.0;
        if word_start.is_none() {
            word_start = Some(start);
        }
        word_end = word_end.max(end);
        buffer.push_str(&syllable.text);
        if syllable.text.ends_with(' ') {
            flush(&mut buffer, &mut word_start, &mut word_end);
        }
    }
    flush(&mut buffer, &mut word_start, &mut word_end);
    words
}

fn convert(entry: &Entry) -> Option<Line> {
    let start = entry.time / 1000.0;
    let end = (entry.time + entry.duration.max(1.0)) / 1000.0;
    let text = trim_spaces(&entry.text);
    if text.is_empty() {
        return None;
    }
    let words = entry.syllabus.as_deref().map(merge_syllables).unwrap_or_default();
    if words.is_empty() {
        return None;
    }
    Some(Line {
        start,
        end,
        text: text.to_string(),
        words,
        is_instrumental: false,
        word_timed: true,
        romanization: None,
    })
}

/// Parses one node response into lines and the upstream source name
/// ("Apple" from "Apple (via ...)"; "Lyrics+" when absent).
fn parse_response(body: Vec<u8>) -> Option<(Vec<Line>, String)> {
    let payload: Response = serde_json::from_slice(&gunzip_if_needed(body)).ok()?;
    let entries = payload.lyrics.filter(|e| !e.is_empty())?;
    let lines: Vec<Line> = entries.iter().filter_map(convert).collect();
    if lines.is_empty() {
        return None;
    }
    let raw = payload.metadata.and_then(|m| m.source).unwrap_or_else(|| "Lyrics+".to_string());
    let source = raw.split(" (").next().unwrap_or(&raw).to_string();
    Some((lines, source))
}

/// Lyrics+ / KPoe. The candidate carries the query's metadata and scores
/// 113; its provenance reads "`<upstream>` via Lyrics+".
#[derive(Debug, Clone, Copy, Default)]
pub struct LyricsPlus;

#[async_trait]
impl Provider for LyricsPlus {
    fn id(&self) -> SourceId {
        SourceId::LyricsPlus
    }

    async fn fetch(&self, query: &LyricQuery, http: &dyn HttpClient) -> Result<Option<Candidate>, ProviderError> {
        let duration = format!("{}", query.duration as i64);
        let mut parameters: Vec<(&str, &str)> = vec![("title", &query.title), ("artist", &query.artist)];
        if query.duration > 0.0 {
            parameters.push(("duration", &duration));
        }
        if let Some(album) = query.album.as_deref().filter(|a| !a.is_empty()) {
            parameters.push(("album", album));
        }
        for host in HOSTS {
            let url = build_url(&format!("{host}/v2/lyrics/get"), &parameters);
            let Ok(response) = http.get(&url, &[("User-Agent", USER_AGENT)], REQUEST_TIMEOUT).await else {
                continue;
            };
            if response.status != 200 {
                continue;
            }
            let Some((lines, source)) = parse_response(response.body) else { continue };
            return Ok(Some(Candidate {
                source: SourceId::LyricsPlus,
                result: LyricResult::Synced {
                    provenance: Provenance {
                        precision: precision(&lines).to_string(),
                        source: format!("{source} via Lyrics+"),
                    },
                    lines,
                },
                title: query.title.clone(),
                artist: query.artist.clone(),
                duration: Some(query.duration),
                score: 113.0,
            }));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn syllable(time: f64, duration: f64, text: &str) -> Syllable {
        Syllable { time, duration, text: text.into() }
    }

    #[test]
    fn syllables_merge_into_words() {
        let words = merge_syllables(&[
            syllable(1000.0, 200.0, "con"),
            syllable(1200.0, 300.0, "trol "),
            syllable(1600.0, 0.0, "me"),
        ]);
        let texts: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["control", "me"]);
        assert!((words[0].start - 1.0).abs() < 1e-9 && (words[0].end - 1.5).abs() < 1e-9);
        assert!((words[1].end - 1.66).abs() < 1e-9, "minimum 60 ms");
    }

    #[test]
    fn gzip_bodies_are_inflated() {
        let json = br#"{"lyrics":[{"time":1000,"duration":2000,"text":"Hold on","syllabus":[{"time":1000,"duration":500,"text":"Hold "},{"time":1500,"duration":500,"text":"on"}]}],"metadata":{"source":"Apple (via Lyrics+)"}}"#;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(json).unwrap();
        let gz = encoder.finish().unwrap();
        let (lines, source) = parse_response(gz).unwrap();
        assert_eq!(source, "Apple");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].word_timed);
        let (_, plain_source) = parse_response(
            br#"{"lyrics":[{"time":0,"duration":1,"text":"a","syllabus":[{"time":0,"duration":1,"text":"a"}]}]}"#
                .to_vec(),
        )
        .unwrap();
        assert_eq!(plain_source, "Lyrics+");
    }

    #[test]
    fn unusable_payloads_are_skipped() {
        assert!(parse_response(b"{}".to_vec()).is_none());
        assert!(parse_response(br#"{"lyrics":[]}"#.to_vec()).is_none());
        assert!(parse_response(br#"{"lyrics":[{"time":0,"duration":1,"text":"  "}]}"#.to_vec()).is_none());
        assert!(parse_response(br#"{"lyrics":[{"time":0,"duration":1,"text":"no syllables"}]}"#.to_vec()).is_none());
        assert!(parse_response(br#"{"lyrics":[{"text":"missing times"}]}"#.to_vec()).is_none());
        assert!(parse_response(vec![0x1f, 0x8b, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).is_none());
    }
}
