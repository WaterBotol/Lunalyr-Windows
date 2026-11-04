//! LRCLIB (`LyricsFetcher.lrclib` / `LyricsFetcher.requestLRCLIB`).

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use super::{first_best, Provider, ProviderError, LRCLIB_USER_AGENT};
use crate::http::{build_url, HttpClient, MAX_BODY_BYTES};
use crate::model::{Candidate, LyricQuery, LyricResult, Provenance, SourceId};
use crate::parse::{parse_lrc, parse_lyricsfile};
use crate::rank::match_score;
use crate::text::split_newlines;
use crate::timing::{precision, NOT_SYNCED};

const BASE: &str = "https://lrclib.net/api/";
/// Per request. macOS used a 4 s *idle* timeout; reqwest's timeout covers the
/// whole response, so it is longer here (the source budget is 20 s).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
/// Search results considered.
const MAX_SEARCH_RESULTS: usize = 30;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    track_name: Option<String>,
    artist_name: Option<String>,
    album_name: Option<String>,
    duration: Option<f64>,
    synced_lyrics: Option<String>,
    lyricsfile: Option<String>,
    plain_lyrics: Option<String>,
}

fn non_empty(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|v| !v.is_empty())
}

/// GET `https://lrclib.net/api/<path>` with parameters sorted by key.
/// 404 → `Ok(None)`; 429/503 retried once when `Retry-After` ≤ 3 s
/// (missing or non-numeric counts as 1 s), otherwise an error.
async fn request(
    http: &dyn HttpClient,
    path: &str,
    parameters: &[(&str, &str)],
) -> Result<Option<Vec<u8>>, ProviderError> {
    let mut sorted = parameters.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    let url = build_url(&format!("{BASE}{path}"), &sorted);
    for attempt in 0..2 {
        let response = http.get(&url, &[("User-Agent", LRCLIB_USER_AGENT)], REQUEST_TIMEOUT).await?;
        let status = response.status;
        if status == 404 {
            return Ok(None);
        }
        if (status == 503 || status == 429) && attempt == 0 {
            let delay = response.header("Retry-After").unwrap_or("1").trim().parse::<f64>().unwrap_or(1.0);
            // Long server backoffs are reported rather than keeping a lookup alive.
            if !delay.is_finite() || delay > 3.0 {
                return Err(ProviderError::Service(format!("LRCLIB is busy. Try again later (HTTP {status}).")));
            }
            tokio::time::sleep(Duration::from_secs_f64(delay.max(0.0))).await;
            continue;
        }
        if status != 200 {
            return Err(ProviderError::Service(format!(
                "LRCLIB: HTTP {status}. The service may be temporarily unavailable."
            )));
        }
        if response.body.len() > MAX_BODY_BYTES {
            return Err(ProviderError::Service("LRCLIB response too large".into()));
        }
        return Ok(Some(response.body));
    }
    Ok(None)
}

fn candidate(payload: Response, query: &LyricQuery) -> Option<Candidate> {
    let title = payload.track_name.clone().unwrap_or_else(|| query.title.clone());
    let artist = payload.artist_name.clone().unwrap_or_else(|| query.artist.clone());
    let delta = payload.duration.map_or(0.0, |d| (d - query.duration).abs());
    let album_match = LyricQuery::normalize(payload.album_name.as_deref().unwrap_or(""))
        == LyricQuery::normalize(query.album.as_deref().unwrap_or(""));
    let score = match_score(
        LyricQuery::similarity(&query.title, &title),
        LyricQuery::similarity(&query.artist, &artist),
        delta,
        payload.duration.is_some() && query.duration > 0.0,
        false,
        true,
        album_match,
    );
    if score <= 0.0 {
        return None;
    }
    let mut lines = non_empty(&payload.lyricsfile).map(parse_lyricsfile).unwrap_or_default();
    if lines.is_empty() {
        if let Some(lrc) = non_empty(&payload.synced_lyrics) {
            lines = parse_lrc(lrc, query.duration);
        }
    }
    let timed = !lines.is_empty();
    let result = if timed {
        LyricResult::Synced {
            provenance: Provenance { precision: precision(&lines).into(), source: "LRCLIB".into() },
            lines,
        }
    } else {
        let plain = non_empty(&payload.plain_lyrics)?;
        LyricResult::Plain {
            lines: split_newlines(plain).into_iter().map(str::to_string).collect(),
            provenance: Provenance { precision: NOT_SYNCED.into(), source: "LRCLIB".into() },
        }
    };
    Some(Candidate {
        source: SourceId::Lrclib,
        result,
        title,
        artist,
        duration: payload.duration,
        score: score - if timed { 0.0 } else { 12.0 },
    })
}

/// LRCLIB: `/api/get` for the exact recording (title, artist, album,
/// rounded duration); if that has no timed lyrics, `/api/search` by title
/// and artist (first 30 hits). Each payload is scored with
/// [`match_score`]; the `lyricsfile` (word timing) is preferred, then
/// `syncedLyrics`, then plain text (−12).
#[derive(Debug, Clone, Copy, Default)]
pub struct Lrclib;

#[async_trait]
impl Provider for Lrclib {
    fn id(&self) -> SourceId {
        SourceId::Lrclib
    }

    async fn fetch(&self, query: &LyricQuery, http: &dyn HttpClient) -> Result<Option<Candidate>, ProviderError> {
        let duration = format!("{}", query.duration.round() as i64);
        let mut parameters: Vec<(&str, &str)> = vec![("track_name", &query.title), ("artist_name", &query.artist)];
        if let Some(album) = query.album.as_deref().filter(|a| !a.is_empty()) {
            parameters.push(("album_name", album));
        }
        if query.duration > 0.0 {
            parameters.push(("duration", &duration));
        }
        let mut payloads: Vec<Response> = Vec::new();
        let mut exact_error = None;
        match request(http, "get", &parameters).await {
            Ok(Some(data)) => {
                if let Ok(exact) = serde_json::from_slice::<Response>(&data) {
                    payloads.push(exact);
                }
            }
            Ok(None) => {}
            Err(error) => exact_error = Some(error),
        }
        let exact_timed =
            payloads.iter().any(|p| non_empty(&p.synced_lyrics).is_some() || non_empty(&p.lyricsfile).is_some());
        if !exact_timed {
            let search = [("track_name", query.title.as_str()), ("artist_name", query.artist.as_str())];
            match request(http, "search", &search).await {
                Ok(Some(data)) => {
                    if let Ok(found) = serde_json::from_slice::<Vec<Response>>(&data) {
                        payloads.extend(found.into_iter().take(MAX_SEARCH_RESULTS));
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    if payloads.is_empty() {
                        return Err(error);
                    }
                }
            }
        }
        if payloads.is_empty() {
            if let Some(error) = exact_error {
                return Err(error);
            }
        }
        Ok(first_best(payloads.into_iter().filter_map(|p| candidate(p, query)).collect()))
    }
}
