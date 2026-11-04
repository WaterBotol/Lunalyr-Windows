//! Independent adapters for public lyric endpoints: QQ Music, Kugou, Kuwo
//! and lyrics.ovh (`AdditionalLyricsProviders.swift`). No cookies,
//! credentials, audio or listening history are requested.

use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use serde_json::{Map, Value};

use super::{Provider, ProviderError, USER_AGENT};
use crate::http::{build_url, encode_component, HttpClient, MAX_BODY_BYTES};
use crate::model::{Candidate, LyricQuery, LyricResult, Provenance, SourceId};
use crate::parse::parse_lrc;
use crate::rank::match_score;
use crate::text::{decode_entities, split_newlines, trim_spaces};
use crate::timing::precision;

/// Per request (`LyricHTTP.json`).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(6);

const QQ_SEARCH: &str = "https://c.y.qq.com/soso/fcgi-bin/client_search_cp";
const QQ_LYRIC: &str = "https://c.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg";
const QQ_REFERER: &str = "https://y.qq.com/";
/// Same API as macOS's `mobilecdn.kugou.com`, whose TLS certificate does not
/// cover that host name (every HTTPS request fails); this host's does.
const KUGOU_SEARCH: &str = "https://mobileservice.kugou.com/api/v3/search/song";
const KUGOU_LYRIC_SEARCH: &str = "https://lyrics.kugou.com/search";
const KUGOU_DOWNLOAD: &str = "https://lyrics.kugou.com/download";
const KUWO_SEARCH: &str = "https://search.kuwo.cn/r.s";
const KUWO_LYRIC: &str = "https://www.kuwo.cn/openapi/v1/www/lyric/getlyric";
const KUWO_REFERER: &str = "https://www.kuwo.cn/";
const LYRICS_OVH: &str = "https://api.lyrics.ovh/v1/";

/// GET returning a JSON object (`LyricHTTP.json`): parameters sorted by key,
/// HTTP 200 required, 2 MB cap.
pub(crate) async fn json(
    http: &dyn HttpClient,
    base: &str,
    parameters: &[(&str, &str)],
    referer: Option<&str>,
) -> Result<Map<String, Value>, ProviderError> {
    let mut sorted = parameters.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    let url = build_url(base, &sorted);
    let mut headers = vec![("User-Agent", USER_AGENT)];
    if let Some(referer) = referer {
        headers.push(("Referer", referer));
    }
    let response = http.get(&url, &headers, REQUEST_TIMEOUT).await?;
    if response.status != 200 {
        return Err(ProviderError::Service(format!("Service response: HTTP {}", response.status)));
    }
    if response.body.len() > MAX_BODY_BYTES {
        return Err(ProviderError::Service("Response too large".into()));
    }
    match serde_json::from_slice::<Value>(&response.body) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Err(ProviderError::Service("Unexpected response format".into())),
    }
}

/// `LyricHTTP.string`: a string, or a number's text.
fn string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn text(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str)
}

fn objects(value: Option<&Value>) -> Vec<Map<String, Value>> {
    value
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(|i| i.as_object().cloned()).collect())
        .unwrap_or_default()
}

/// Lenient base64 (whitespace ignored, padding optional) to UTF-8.
fn base64_text(encoded: &str) -> Option<String> {
    let engine = base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::GeneralPurposeConfig::new()
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
    );
    let compact: String = encoded.chars().filter(|c| !c.is_whitespace()).collect();
    String::from_utf8(engine.decode(compact).ok()?).ok()
}

/// Metadata score for these sources (line timing assumed, no album).
fn score(query: &LyricQuery, title: &str, artist: &str, duration: Option<f64>) -> f64 {
    match_score(
        LyricQuery::similarity(&query.title, title),
        LyricQuery::similarity(&query.artist, artist),
        duration.map_or(0.0, |d| (d - query.duration).abs()),
        duration.is_some() && query.duration > 0.0,
        false,
        true,
        false,
    )
}

/// A candidate from LRC text; enhanced (word-stamped) LRC scores +13.
fn candidate(
    source: SourceId,
    lrc: &str,
    query: &LyricQuery,
    title: &str,
    artist: &str,
    duration: Option<f64>,
) -> Option<Candidate> {
    let lines = parse_lrc(&decode_entities(lrc), query.duration);
    if lines.is_empty() {
        return None;
    }
    let enhanced = lines.iter().any(|l| l.word_timed);
    let value = score(query, title, artist, duration);
    if value <= 0.0 {
        return None;
    }
    Some(Candidate {
        source,
        result: LyricResult::Synced {
            provenance: Provenance { precision: precision(&lines).to_string(), source: source.title().to_string() },
            lines,
        },
        title: title.to_string(),
        artist: artist.to_string(),
        duration,
        score: value + if enhanced { 13.0 } else { 0.0 },
    })
}

struct Hit {
    song: Map<String, Value>,
    title: String,
    artist: String,
    duration: Option<f64>,
    score: f64,
}

fn rank_hits(mut hits: Vec<Hit>) -> Vec<Hit> {
    hits.retain(|h| h.score > 0.0);
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    hits
}

/// QQ Music: search, then the line lyric of the top three hits.
#[derive(Debug, Clone, Copy, Default)]
pub struct QqMusic;

#[async_trait]
impl Provider for QqMusic {
    fn id(&self) -> SourceId {
        SourceId::QqMusic
    }

    async fn fetch(&self, query: &LyricQuery, http: &dyn HttpClient) -> Result<Option<Candidate>, ProviderError> {
        let keyword = format!("{} {}", query.title, query.artist);
        let found =
            json(http, QQ_SEARCH, &[("format", "json"), ("p", "1"), ("n", "8"), ("w", &keyword)], Some(QQ_REFERER))
                .await?;
        let songs = objects(found.get("data").and_then(|d| d.get("song")).and_then(|s| s.get("list")));
        let hits = songs
            .into_iter()
            .filter_map(|song| {
                let title = text(song.get("songname"))?.to_string();
                let artist = objects(song.get("singer"))
                    .iter()
                    .filter_map(|s| text(s.get("name")).map(str::to_string))
                    .collect::<Vec<_>>()
                    .join(" & ");
                let duration = song.get("interval").and_then(Value::as_f64);
                let score = score(query, &title, &artist, duration);
                Some(Hit { song, title, artist, duration, score })
            })
            .collect();
        for hit in rank_hits(hits).into_iter().take(3) {
            let Some(mid) = text(hit.song.get("songmid")) else { continue };
            let payload = json(
                http,
                QQ_LYRIC,
                &[("songmid", mid), ("format", "json"), ("nobase64", "1"), ("g_tk", "5381")],
                Some(QQ_REFERER),
            )
            .await?;
            let Some(mut lyric) = text(payload.get("lyric")).filter(|t| !t.is_empty()).map(str::to_string) else {
                continue;
            };
            if !lyric.contains('[') {
                if let Some(decoded) = base64_text(&lyric) {
                    lyric = decoded;
                }
            }
            if let Some(found) = candidate(SourceId::QqMusic, &lyric, query, &hit.title, &hit.artist, hit.duration) {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }
}

/// Kugou: song search, lyric search by hash, then the LRC download.
#[derive(Debug, Clone, Copy, Default)]
pub struct Kugou;

#[async_trait]
impl Provider for Kugou {
    fn id(&self) -> SourceId {
        SourceId::Kugou
    }

    async fn fetch(&self, query: &LyricQuery, http: &dyn HttpClient) -> Result<Option<Candidate>, ProviderError> {
        let keyword = format!("{} {}", query.title, query.artist);
        let payload = json(
            http,
            KUGOU_SEARCH,
            &[("keyword", &keyword), ("page", "1"), ("pagesize", "8"), ("format", "json")],
            None,
        )
        .await?;
        let songs = objects(payload.get("data").and_then(|d| d.get("info")));
        let hits = songs
            .into_iter()
            .map(|song| {
                let filename = text(song.get("filename")).unwrap_or("").to_string();
                let parts: Vec<&str> = filename.split(" - ").collect();
                let title = text(song.get("songname")).or(parts.last().copied()).unwrap_or("").to_string();
                let artist = text(song.get("singername")).or(parts.first().copied()).unwrap_or("").to_string();
                let duration = song.get("duration").and_then(Value::as_f64);
                let score = score(query, &title, &artist, duration);
                Hit { song, title, artist, duration, score }
            })
            .collect();
        let duration_ms = format!("{}", (query.duration * 1000.0) as i64);
        for hit in rank_hits(hits).into_iter().take(2) {
            let Some(hash) = text(hit.song.get("hash")) else { continue };
            let search = json(
                http,
                KUGOU_LYRIC_SEARCH,
                &[
                    ("ver", "1"),
                    ("man", "yes"),
                    ("client", "pc"),
                    ("keyword", &keyword),
                    ("duration", &duration_ms),
                    ("hash", hash),
                ],
                None,
            )
            .await?;
            for item in objects(search.get("candidates")).into_iter().take(2) {
                let (Some(id), Some(key)) = (string(item.get("id")), text(item.get("accesskey"))) else { continue };
                let response = json(
                    http,
                    KUGOU_DOWNLOAD,
                    &[
                        ("ver", "1"),
                        ("client", "pc"),
                        ("id", &id),
                        ("accesskey", key),
                        ("fmt", "lrc"),
                        ("charset", "utf8"),
                    ],
                    None,
                )
                .await?;
                let Some(lrc) = text(response.get("content")).and_then(base64_text) else { continue };
                if let Some(found) = candidate(SourceId::Kugou, &lrc, query, &hit.title, &hit.artist, hit.duration) {
                    return Ok(Some(found));
                }
            }
        }
        Ok(None)
    }
}

/// Kuwo: web search, then the line-lyric list rebuilt as LRC.
#[derive(Debug, Clone, Copy, Default)]
pub struct Kuwo;

/// `[mm:ss.xx]` as Kuwo's rows are rebuilt (`%02d:%05.2f`).
fn kuwo_stamp(time: f64) -> String {
    format!("[{:02}:{:05.2}]", (time / 60.0) as i64, time % 60.0)
}

#[async_trait]
impl Provider for Kuwo {
    fn id(&self) -> SourceId {
        SourceId::Kuwo
    }

    async fn fetch(&self, query: &LyricQuery, http: &dyn HttpClient) -> Result<Option<Candidate>, ProviderError> {
        let keyword = format!("{} {}", query.title, query.artist);
        let search = json(
            http,
            KUWO_SEARCH,
            &[
                ("all", &keyword),
                ("ft", "music"),
                ("itemset", "web_2013"),
                ("client", "kt"),
                ("pn", "0"),
                ("rn", "8"),
                ("rformat", "json"),
                ("encoding", "utf8"),
                ("pcjson", "1"),
                ("vipver", "1"),
            ],
            Some(KUWO_REFERER),
        )
        .await?;
        let hits = objects(search.get("abslist"))
            .into_iter()
            .filter_map(|song| {
                let rid = text(song.get("MUSICRID"))?;
                let title = text(song.get("SONGNAME"))?.to_string();
                let artist = text(song.get("ARTIST"))?.to_string();
                let id = rid.split('_').rfind(|p| !p.is_empty())?.to_string();
                let duration = string(song.get("DURATION")).and_then(|d| d.parse::<f64>().ok());
                let score = score(query, &title, &artist, duration);
                let mut song = Map::new();
                song.insert("id".into(), Value::String(id));
                Some(Hit { song, title, artist, duration, score })
            })
            .collect();
        for hit in rank_hits(hits).into_iter().take(2) {
            let id = text(hit.song.get("id")).unwrap_or_default();
            let payload = json(http, KUWO_LYRIC, &[("musicId", id)], Some(KUWO_REFERER)).await?;
            let Some(rows) = payload.get("data").and_then(|d| d.get("lrclist")).and_then(Value::as_array) else {
                continue;
            };
            let lrc = rows
                .iter()
                .filter_map(|row| {
                    let time = string(row.get("time"))?.parse::<f64>().ok()?;
                    if !time.is_finite() || time < 0.0 {
                        return None;
                    }
                    let line = text(row.get("lineLyric"))?;
                    Some(format!("{}{line}", kuwo_stamp(time)))
                })
                .collect::<Vec<_>>()
                .join("\n");
            if let Some(found) = candidate(SourceId::Kuwo, &lrc, query, &hit.title, &hit.artist, hit.duration) {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }
}

/// lyrics.ovh: plain text only (timing tier 0, score 0).
#[derive(Debug, Clone, Copy, Default)]
pub struct LyricsOvh;

/// Provenance precision of lyrics.ovh results.
pub const PLAIN_NO_TIMING: &str = "Plain text · no timing";

#[async_trait]
impl Provider for LyricsOvh {
    fn id(&self) -> SourceId {
        SourceId::LyricsOvh
    }

    async fn fetch(&self, query: &LyricQuery, http: &dyn HttpClient) -> Result<Option<Candidate>, ProviderError> {
        let url = format!("{LYRICS_OVH}{}/{}", encode_component(&query.artist), encode_component(&query.title));
        let payload = json(http, &url, &[], None).await?;
        let Some(lyrics) = text(payload.get("lyrics")) else { return Ok(None) };
        let lines: Vec<String> = split_newlines(lyrics).into_iter().map(|l| trim_spaces(l).to_string()).collect();
        if lines.iter().all(|l| l.is_empty()) {
            return Ok(None);
        }
        Ok(Some(Candidate {
            source: SourceId::LyricsOvh,
            result: LyricResult::Plain {
                lines,
                provenance: Provenance { precision: PLAIN_NO_TIMING.into(), source: "lyrics.ovh".into() },
            },
            title: query.title.clone(),
            artist: query.artist.clone(),
            duration: None,
            score: 0.0,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kuwo_stamps_round_trip_through_the_lrc_parser() {
        assert_eq!(kuwo_stamp(1.0), "[00:01.00]");
        assert_eq!(kuwo_stamp(75.5), "[01:15.50]");
        assert_eq!(kuwo_stamp(600.25), "[10:00.25]");
    }

    #[test]
    fn base64_is_lenient() {
        assert_eq!(base64_text("WzAwOjAxXWhp").as_deref(), Some("[00:01]hi"));
        assert_eq!(base64_text("WzAw\nOjAxXWhp").as_deref(), Some("[00:01]hi"));
        assert_eq!(base64_text("aGk").as_deref(), Some("hi"));
        assert_eq!(base64_text("!!"), None);
    }
}
