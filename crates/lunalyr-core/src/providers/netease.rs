//! NetEase Cloud Music (`NetEaseLyricsProvider.swift`): public search API,
//! then the lyric API, preferring word-timed YRC over LRC.

use std::sync::LazyLock;
use std::time::Duration;

use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;

use super::{Provider, ProviderError};
use crate::http::{build_url, HttpClient};
use crate::model::{Candidate, Line, LyricQuery, LyricResult, Provenance, SourceId};
use crate::parse::{parse_lrc, parse_yrc};
use crate::text::is_cs_alphanumeric;
use crate::timing::precision;

const SEARCH_URL: &str = "https://music.163.com/api/search/get";
const LYRIC_URL: &str = "https://music.163.com/api/song/lyric";
const BROWSER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64)";
const REFERER: &str = "https://music.163.com/";
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(6);

static STRIP: LazyLock<[Regex; 4]> =
    LazyLock::new(|| [r"\([^)]*\)", r"\[[^\]]*\]", r"feat\..*", r"ft\..*"].map(|p| Regex::new(p).unwrap()));

#[derive(Deserialize)]
struct SearchResponse {
    result: Option<SearchResult>,
}

#[derive(Deserialize)]
struct SearchResult {
    songs: Option<Vec<Song>>,
}

#[derive(Deserialize)]
struct Song {
    id: i64,
    name: String,
    artists: Option<Vec<Artist>>,
    /// Milliseconds.
    duration: Option<f64>,
}

#[derive(Deserialize)]
struct Artist {
    name: String,
}

#[derive(Deserialize)]
struct LyricResponse {
    yrc: Option<Lyric>,
    lrc: Option<Lyric>,
}

#[derive(Deserialize)]
struct Lyric {
    lyric: Option<String>,
}

/// NetEase's own title/artist folding: lower-case, drop `(...)`, `[...]`
/// and everything from `feat.` / `ft.`, keep only letters, marks and digits.
pub(crate) fn normalize(value: &str) -> String {
    let mut text = value.to_lowercase();
    for pattern in STRIP.iter() {
        text = pattern.replace_all(&text, "").into_owned();
    }
    text.chars().filter(|c| is_cs_alphanumeric(*c)).collect()
}

fn matches(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && (a == b || a.contains(b) || b.contains(a))
}

async fn get(http: &dyn HttpClient, url: &str) -> Option<Vec<u8>> {
    match http.get(url, &[("User-Agent", BROWSER_AGENT), ("Referer", REFERER)], REQUEST_TIMEOUT).await {
        Ok(response) if response.status == 200 => Some(response.body),
        _ => None,
    }
}

/// Picks the best edition from a search response body. Search returns many
/// editions (radio edits, live cuts, remixes) with matching names, so
/// runtime weighs most: within 2 s scores 5, within 5 s scores 4, more than
/// 8 s off is rejected (when the duration is known); the artist must match.
/// Ties go to the smaller duration difference, then the earlier hit.
fn best_song(body: &[u8], title: &str, artist: &str, duration: f64) -> Option<i64> {
    let payload: SearchResponse = serde_json::from_slice(body).ok()?;
    let songs = payload.result?.songs?;
    let wanted_title = normalize(title);
    let wanted_artist = normalize(artist);
    let mut best: Option<(i64, i32, f64)> = None;
    for song in songs {
        if !matches(&normalize(&song.name), &wanted_title) {
            continue;
        }
        let artist_matches =
            song.artists.as_ref().is_some_and(|list| list.iter().any(|a| matches(&normalize(&a.name), &wanted_artist)));
        if !artist_matches {
            continue;
        }
        let song_duration = song.duration.unwrap_or(0.0);
        let delta =
            if duration > 0.0 && song_duration > 0.0 { (song_duration / 1000.0 - duration).abs() } else { f64::MAX };
        if !(duration <= 0.0 || delta < 8.0) {
            continue;
        }
        let mut score = 0;
        if delta < 2.0 {
            score += 5;
        } else if delta < 5.0 {
            score += 4;
        }
        if artist_matches {
            score += 2;
        }
        if score <= 0 {
            continue;
        }
        let better = match best {
            None => true,
            Some((_, best_score, best_delta)) => score > best_score || (score == best_score && delta < best_delta),
        };
        if better {
            best = Some((song.id, score, delta));
        }
    }
    best.map(|(id, _, _)| id)
}

/// Lines for the best matching song: YRC when it parses, else LRC.
async fn timed_lines(http: &dyn HttpClient, query: &LyricQuery) -> Option<Vec<Line>> {
    let search =
        build_url(SEARCH_URL, &[("s", &format!("{} {}", query.title, query.artist)), ("type", "1"), ("limit", "8")]);
    let body = get(http, &search).await?;
    let id = best_song(&body, &query.title, &query.artist, query.duration)?;
    let url = format!("{LYRIC_URL}?id={id}&lv=1&kv=1&tv=-1&yv=1");
    let payload: LyricResponse = serde_json::from_slice(&get(http, &url).await?).ok()?;
    if let Some(yrc) = payload.yrc.and_then(|l| l.lyric) {
        let parsed = parse_yrc(&yrc);
        if !parsed.is_empty() {
            return Some(parsed);
        }
    }
    if let Some(lrc) = payload.lrc.and_then(|l| l.lyric) {
        let parsed = parse_lrc(&lrc, query.duration);
        if !parsed.is_empty() {
            return Some(parsed);
        }
    }
    None
}

/// NetEase Cloud Music. Network and service failures read as "no match"
/// (as on macOS). The candidate carries the query's title, artist and
/// duration; its score is 110 with native word timing, else 96.
#[derive(Debug, Clone, Copy, Default)]
pub struct NetEase;

#[async_trait]
impl Provider for NetEase {
    fn id(&self) -> SourceId {
        SourceId::NetEase
    }

    async fn fetch(&self, query: &LyricQuery, http: &dyn HttpClient) -> Result<Option<Candidate>, ProviderError> {
        let Some(lines) = timed_lines(http, query).await else { return Ok(None) };
        let native = lines.iter().any(|l| l.word_timed);
        Ok(Some(Candidate {
            source: SourceId::NetEase,
            result: LyricResult::Synced {
                provenance: Provenance { precision: precision(&lines).into(), source: "NetEase".into() },
                lines,
            },
            title: query.title.clone(),
            artist: query.artist.clone(),
            duration: Some(query.duration),
            score: if native { 110.0 } else { 96.0 },
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_versions_and_features() {
        assert_eq!(normalize("Paper Lanterns (Radio Edit) [2019]"), "paperlanterns");
        assert_eq!(normalize("Song feat. Someone"), "song");
        assert_eq!(normalize("Song ft. Someone"), "song");
        assert_eq!(normalize("Beyoncé"), "beyoncé");
        assert_eq!(normalize("夜に駆ける"), "夜に駆ける");
        assert!(matches("paperlanterns", "paperlanternsremix"));
        assert!(!matches("", "x"));
    }

    #[test]
    fn edition_selection_prefers_matching_runtime() {
        let body = br#"{"result":{"songs":[
            {"id":1,"name":"Paper Lanterns (Radio Edit)","artists":[{"name":"Glass Harbour"}],"duration":185000},
            {"id":2,"name":"Paper Lanterns","artists":[{"name":"Glass Harbour"}],"duration":214000},
            {"id":3,"name":"Paper Lanterns","artists":[{"name":"Glass Harbour"}],"duration":210500},
            {"id":4,"name":"Paper Lanterns","artists":[{"name":"Somebody Else"}],"duration":210000},
            {"id":5,"name":"Other Song","artists":[{"name":"Glass Harbour"}],"duration":210000}
        ]}}"#;
        assert_eq!(best_song(body, "Paper Lanterns", "Glass Harbour", 210.0), Some(3));
        // 214 s is within 5 s of 211 but 210.5 is within 2 s.
        assert_eq!(best_song(body, "Paper Lanterns", "Glass Harbour", 211.0), Some(3));
        // The radio edit wins for its own runtime.
        assert_eq!(best_song(body, "Paper Lanterns", "Glass Harbour", 185.0), Some(1));
        // Nothing within 8 s.
        assert_eq!(best_song(body, "Paper Lanterns", "Glass Harbour", 300.0), None);
        // Unknown duration: artist match only, first hit wins ties.
        assert_eq!(best_song(body, "Paper Lanterns", "Glass Harbour", 0.0), Some(1));
        assert_eq!(best_song(body, "Paper Lanterns", "Nobody", 210.0), None);
        assert_eq!(best_song(b"{}", "a", "b", 0.0), None);
        assert_eq!(best_song(b"not json", "a", "b", 0.0), None);
    }
}
