//! Provider behaviour against canned HTTP responses (no network).

use lunalyr_core::http::{FixtureHttp, HttpError, HttpResponse};
use lunalyr_core::model::{LyricQuery, LyricResult, SourceId};
use lunalyr_core::providers::{Lrclib, NetEase, Provider, ProviderError, Unavailable, LRCLIB_USER_AGENT};
use lunalyr_core::timing::{precision, LINE_SYNCED, WORD_BY_WORD};

const LRCLIB_GET: &str = "lrclib.net/api/get";
const LRCLIB_SEARCH: &str = "lrclib.net/api/search";

fn query() -> LyricQuery {
    LyricQuery::new("Paper Lanterns", "Glass Harbour", Some("Night Ferry".into()), 214.6)
}

fn lines_of(result: &LyricResult) -> &[lunalyr_core::model::Line] {
    match result {
        LyricResult::Synced { lines, .. } => lines,
        LyricResult::Plain { .. } => panic!("expected synced"),
    }
}

fn record(track: &str, artist: &str, duration: f64, synced: Option<&str>, plain: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "id": 1, "trackName": track, "artistName": artist, "albumName": "Night Ferry",
        "duration": duration, "instrumental": false,
        "syncedLyrics": synced, "plainLyrics": plain
    })
}

const SYNCED: &str = "[00:12.00]Lanterns drift over the water\n[00:16.50]Carry the light to the shore\n";
const LYRICSFILE: &str = "lines:\n- text: Lanterns drift\n  start_ms: 12000\n  end_ms: 14000\n  words:\n  - text: Lanterns\n    start_ms: 12000\n    end_ms: 12800\n  - text: drift\n    start_ms: 12800\n    end_ms: 13900\n";

#[tokio::test]
async fn lrclib_exact_hit_skips_search() {
    let body = record("Paper Lanterns", "Glass Harbour", 214.0, Some(SYNCED), Some("x"));
    let http = FixtureHttp::new().route(LRCLIB_GET, 200, body.to_string());
    let candidate = Lrclib.fetch(&query(), &http).await.unwrap().unwrap();
    assert_eq!(candidate.source, SourceId::Lrclib);
    assert_eq!(lines_of(&candidate.result).len(), 2);
    assert_eq!(candidate.result.provenance().precision, LINE_SYNCED);
    assert_eq!(candidate.result.provenance().source, "LRCLIB");
    assert_eq!(candidate.duration, Some(214.0));
    assert_eq!(http.count(LRCLIB_SEARCH), 0);
    let request = &http.requests()[0];
    assert_eq!(
        request.url,
        "https://lrclib.net/api/get?album_name=Night%20Ferry&artist_name=Glass%20Harbour&duration=215&track_name=Paper%20Lanterns"
    );
    assert!(request.headers.contains(&("User-Agent".to_string(), LRCLIB_USER_AGENT.to_string())));
    // title 48 + artist 27 + duration (14 - 1.2) + line 7 + album 4
    assert!((candidate.score - (48.0 + 27.0 + 12.8 + 7.0 + 4.0)).abs() < 1e-9);
}

#[tokio::test]
async fn lrclib_prefers_lyricsfile_word_timing() {
    let mut body = record("Paper Lanterns", "Glass Harbour", 214.0, Some(SYNCED), None);
    body["lyricsfile"] = serde_json::json!(LYRICSFILE);
    let http = FixtureHttp::new().route(LRCLIB_GET, 200, body.to_string());
    let candidate = Lrclib.fetch(&query(), &http).await.unwrap().unwrap();
    let lines = lines_of(&candidate.result);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].word_timed);
    assert_eq!(precision(lines), WORD_BY_WORD);
}

#[tokio::test]
async fn lrclib_get_miss_falls_back_to_search_and_scores_hits() {
    let search = serde_json::json!([
        record("Paper Lanterns", "Someone Else", 214.0, Some(SYNCED), None),
        record("Paper Lanterns", "Glass Harbour", 214.0, None, Some("Lanterns drift\nCarry the light")),
        record("Paper Lanterns (Live)", "Glass Harbour", 260.0, Some(SYNCED), None),
        record("Paper Lanterns", "Glass Harbour", 215.0, Some(SYNCED), None),
    ]);
    let http = FixtureHttp::new().route(LRCLIB_GET, 404, "").route(LRCLIB_SEARCH, 200, search.to_string());
    let candidate = Lrclib.fetch(&query(), &http).await.unwrap().unwrap();
    assert_eq!(candidate.duration, Some(215.0), "the synced studio edition wins");
    assert_eq!(candidate.artist, "Glass Harbour");
    assert!(matches!(candidate.result, LyricResult::Synced { .. }));
    let search_url = http.urls().into_iter().find(|u| u.contains(LRCLIB_SEARCH)).unwrap();
    assert_eq!(search_url, "https://lrclib.net/api/search?artist_name=Glass%20Harbour&track_name=Paper%20Lanterns");
}

#[tokio::test]
async fn lrclib_plain_exact_hit_still_searches_and_plain_scores_lower() {
    let exact = record("Paper Lanterns", "Glass Harbour", 214.0, None, Some("Lanterns drift\r\nCarry the light"));
    let http = FixtureHttp::new().route(LRCLIB_GET, 200, exact.to_string()).route(LRCLIB_SEARCH, 200, "[]");
    let candidate = Lrclib.fetch(&query(), &http).await.unwrap().unwrap();
    assert_eq!(http.count(LRCLIB_SEARCH), 1);
    match &candidate.result {
        LyricResult::Plain { lines, provenance } => {
            assert_eq!(lines, &vec!["Lanterns drift".to_string(), "Carry the light".to_string()]);
            assert_eq!(provenance.precision, "Not synced");
        }
        _ => panic!("expected plain"),
    }
    assert!((candidate.score - (48.0 + 27.0 + 12.8 + 7.0 + 4.0 - 12.0)).abs() < 1e-9);
}

#[tokio::test]
async fn lrclib_404_everywhere_is_no_match() {
    let http = FixtureHttp::new();
    assert_eq!(Lrclib.fetch(&query(), &http).await, Ok(None));
    assert_eq!(http.count(LRCLIB_GET), 1);
    assert_eq!(http.count(LRCLIB_SEARCH), 1);
}

#[tokio::test(start_paused = true)]
async fn lrclib_429_retries_once_after_short_retry_after() {
    let body = record("Paper Lanterns", "Glass Harbour", 214.0, Some(SYNCED), None);
    let http = FixtureHttp::new().route_sequence(
        LRCLIB_GET,
        vec![
            Ok(HttpResponse::new(429, "").with_header("Retry-After", "2")),
            Ok(HttpResponse::new(200, body.to_string())),
        ],
    );
    let started = tokio::time::Instant::now();
    let candidate = Lrclib.fetch(&query(), &http).await.unwrap();
    assert!(candidate.is_some());
    assert_eq!(http.count(LRCLIB_GET), 2);
    assert!(started.elapsed() >= std::time::Duration::from_secs(2));
}

#[tokio::test(start_paused = true)]
async fn lrclib_long_retry_after_is_reported() {
    let http = FixtureHttp::new()
        .route_response(LRCLIB_GET, HttpResponse::new(429, "").with_header("Retry-After", "30"))
        .route(LRCLIB_SEARCH, 404, "");
    let error = Lrclib.fetch(&query(), &http).await.unwrap_err();
    assert_eq!(error.to_string(), "LRCLIB is busy. Try again later (HTTP 429).");
    assert_eq!(http.count(LRCLIB_GET), 1, "no retry for long backoffs");
}

#[tokio::test(start_paused = true)]
async fn lrclib_503_twice_is_an_error_and_search_rescues() {
    let search = serde_json::json!([record("Paper Lanterns", "Glass Harbour", 214.0, Some(SYNCED), None)]);
    let http = FixtureHttp::new().route(LRCLIB_GET, 503, "").route(LRCLIB_SEARCH, 200, search.to_string());
    let candidate = Lrclib.fetch(&query(), &http).await.unwrap();
    assert!(candidate.is_some(), "search results are used when /get fails");
    assert_eq!(http.count(LRCLIB_GET), 2, "one retry");
}

#[tokio::test]
async fn lrclib_errors_surface_when_nothing_was_found() {
    let http = FixtureHttp::new().route(LRCLIB_GET, 500, "").route(LRCLIB_SEARCH, 500, "");
    let error = Lrclib.fetch(&query(), &http).await.unwrap_err();
    assert_eq!(error.to_string(), "LRCLIB: HTTP 500. The service may be temporarily unavailable.");
    let http = FixtureHttp::new().route_error(LRCLIB_GET, HttpError::Timeout).route(LRCLIB_SEARCH, 404, "");
    assert_eq!(Lrclib.fetch(&query(), &http).await, Err(ProviderError::Http(HttpError::Timeout)));
}

#[tokio::test]
async fn lrclib_omits_unknown_album_and_duration_and_caps_search() {
    let many: Vec<serde_json::Value> =
        (0..40).map(|i| record("Paper Lanterns", "Glass Harbour", 200.0 + i as f64, Some(SYNCED), None)).collect();
    let http = FixtureHttp::new().route(LRCLIB_SEARCH, 200, serde_json::Value::Array(many).to_string());
    let q = LyricQuery::new("Paper Lanterns", "Glass Harbour", None, 0.0);
    let candidate = Lrclib.fetch(&q, &http).await.unwrap().unwrap();
    assert_eq!(http.urls()[0], "https://lrclib.net/api/get?artist_name=Glass%20Harbour&track_name=Paper%20Lanterns");
    assert_eq!(candidate.duration, Some(200.0), "equal scores keep the first hit");
    // A malformed search body is ignored rather than failing the source.
    let http = FixtureHttp::new().route(LRCLIB_SEARCH, 200, "{\"not\": \"a list\"}");
    assert_eq!(Lrclib.fetch(&q, &http).await, Ok(None));
}

const NETEASE_SEARCH: &str = "music.163.com/api/search/get";
const NETEASE_LYRIC: &str = "music.163.com/api/song/lyric";

fn netease_search() -> String {
    serde_json::json!({"result": {"songs": [
        {"id": 11, "name": "Paper Lanterns (Radio Edit)", "artists": [{"name": "Glass Harbour"}], "duration": 186000},
        {"id": 22, "name": "Paper Lanterns", "artists": [{"name": "Glass Harbour"}, {"name": "Guest"}], "duration": 214500},
        {"id": 33, "name": "Paper Lanterns", "artists": [{"name": "Tribute Band"}], "duration": 214600}
    ]}})
    .to_string()
}

#[tokio::test]
async fn netease_picks_edition_by_duration_and_prefers_yrc() {
    let yrc = "[12000,4000](12000,600,0)Lanterns (12600,500,0)drift\n[16500,3000](16500,700,0)Carry (17200,400,0)the (17600,900,0)light";
    let lyric = serde_json::json!({"yrc": {"lyric": yrc}, "lrc": {"lyric": SYNCED}}).to_string();
    let http = FixtureHttp::new().route(NETEASE_SEARCH, 200, netease_search()).route(NETEASE_LYRIC, 200, lyric);
    let candidate = NetEase.fetch(&query(), &http).await.unwrap().unwrap();
    assert_eq!(candidate.source, SourceId::NetEase);
    assert_eq!(candidate.score, 110.0);
    assert_eq!(candidate.result.provenance().precision, WORD_BY_WORD);
    assert_eq!(candidate.result.provenance().source, "NetEase");
    assert_eq!(candidate.title, "Paper Lanterns");
    assert_eq!(candidate.duration, Some(214.6));
    let urls = http.urls();
    assert_eq!(urls[0], "https://music.163.com/api/search/get?s=Paper%20Lanterns%20Glass%20Harbour&type=1&limit=8");
    assert_eq!(urls[1], "https://music.163.com/api/song/lyric?id=22&lv=1&kv=1&tv=-1&yv=1");
    let headers = &http.requests()[0].headers;
    assert!(headers.contains(&("Referer".to_string(), "https://music.163.com/".to_string())));
}

#[tokio::test]
async fn netease_falls_back_to_lrc_when_yrc_is_missing() {
    let lyric = serde_json::json!({"lrc": {"lyric": SYNCED}, "yrc": {"lyric": ""}}).to_string();
    let http = FixtureHttp::new().route(NETEASE_SEARCH, 200, netease_search()).route(NETEASE_LYRIC, 200, lyric);
    let candidate = NetEase.fetch(&query(), &http).await.unwrap().unwrap();
    assert_eq!(candidate.score, 96.0);
    assert_eq!(candidate.result.provenance().precision, LINE_SYNCED);
}

#[tokio::test]
async fn netease_failures_read_as_no_match() {
    let http = FixtureHttp::new().route(NETEASE_SEARCH, 503, "");
    assert_eq!(NetEase.fetch(&query(), &http).await, Ok(None));
    let http = FixtureHttp::new().route(NETEASE_SEARCH, 200, netease_search()).route(NETEASE_LYRIC, 200, "{}");
    assert_eq!(NetEase.fetch(&query(), &http).await, Ok(None));
    let http = FixtureHttp::new().route_error(NETEASE_SEARCH, HttpError::Network("offline".into()));
    assert_eq!(NetEase.fetch(&query(), &http).await, Ok(None));
    let far = LyricQuery::new("Paper Lanterns", "Glass Harbour", None, 300.0);
    let http = FixtureHttp::new().route(NETEASE_SEARCH, 200, netease_search());
    assert_eq!(NetEase.fetch(&far, &http).await, Ok(None));
    assert_eq!(http.count(NETEASE_LYRIC), 0);
}

#[tokio::test]
async fn unimplemented_sources_say_so() {
    let provider = Unavailable(SourceId::Kuwo);
    assert_eq!(provider.id(), SourceId::Kuwo);
    let http = FixtureHttp::new();
    let error = provider.fetch(&query(), &http).await.unwrap_err();
    assert_eq!(error, ProviderError::NotAvailable);
    assert_eq!(error.to_string(), "Not available in this version");
    assert!(http.requests().is_empty());
}
