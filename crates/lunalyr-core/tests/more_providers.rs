//! QQ Music, Kugou, Kuwo, lyrics.ovh and Lyrics+ against canned responses.

use std::io::Write;

use base64::Engine;
use lunalyr_core::http::FixtureHttp;
use lunalyr_core::model::{LyricQuery, LyricResult, SourceId};
use lunalyr_core::providers::additional::PLAIN_NO_TIMING;
use lunalyr_core::providers::{Kugou, Kuwo, LyricsOvh, LyricsPlus, Provider, ProviderError, QqMusic};
use lunalyr_core::timing::{prepare, timing_tier, LINE_SYNCED, WORD_BY_WORD};

fn query() -> LyricQuery {
    LyricQuery::new("Our Song", "Our Artist", None, 180.0)
}

fn header(http: &FixtureHttp, url_part: &str, name: &str) -> Option<String> {
    http.requests()
        .into_iter()
        .find(|r| r.url.contains(url_part))
        .and_then(|r| r.headers.into_iter().find(|(k, _)| k == name).map(|(_, v)| v))
}

// LyricSelectionTests.testNewProvidersKeepTimingProvenanceHonest
#[tokio::test]
async fn kuwo_rebuilds_line_lyrics_and_stays_line_timed() {
    let http = FixtureHttp::new()
        .route(
            "search.kuwo.cn",
            200,
            r#"{"abslist":[{"MUSICRID":"MUSIC_123","SONGNAME":"Our Song","ARTIST":"Our Artist","DURATION":"180"}]}"#,
        )
        .route(
            "www.kuwo.cn",
            200,
            r#"{"code":200,"data":{"lrclist":[{"time":"1.0","lineLyric":"Sing along"},{"time":"4.0","lineLyric":"Next line"}]}}"#,
        );
    let kuwo = Kuwo.fetch(&query(), &http).await.unwrap().unwrap();
    assert_eq!(kuwo.source, SourceId::Kuwo);
    assert_eq!(kuwo.result.line_count(), 2);
    assert_eq!(timing_tier(&kuwo.result), 1);
    assert_eq!(kuwo.result.provenance().source, "Kuwo Music");
    assert_eq!(kuwo.result.provenance().precision, LINE_SYNCED);
    assert_eq!(kuwo.duration, Some(180.0));
    let lyric_url = http.urls().into_iter().find(|u| u.contains("www.kuwo.cn")).unwrap();
    assert!(lyric_url.ends_with("getlyric?musicId=123"), "{lyric_url}");
    assert_eq!(header(&http, "search.kuwo.cn", "Referer").as_deref(), Some("https://www.kuwo.cn/"));
}

#[tokio::test]
async fn lyrics_ovh_is_plain_text() {
    let http = FixtureHttp::new().route("api.lyrics.ovh", 200, r#"{"lyrics":"Sing along\r\n\r\nNext line "}"#);
    let plain = LyricsOvh.fetch(&query(), &http).await.unwrap().unwrap();
    assert_eq!(plain.source, SourceId::LyricsOvh);
    assert_eq!(timing_tier(&plain.result), 0);
    match &plain.result {
        LyricResult::Plain { lines, provenance } => {
            assert_eq!(lines, &["Sing along", "", "Next line"]);
            assert_eq!(provenance.precision, PLAIN_NO_TIMING);
        }
        _ => panic!("expected plain"),
    }
    assert_eq!(http.urls()[0], "https://api.lyrics.ovh/v1/Our%20Artist/Our%20Song");
    let missing = FixtureHttp::new().route("api.lyrics.ovh", 404, r#"{"error":"No lyrics found"}"#);
    let error = LyricsOvh.fetch(&query(), &missing).await.unwrap_err();
    assert_eq!(error, ProviderError::Service("Service response: HTTP 404".into()));
    let blank = FixtureHttp::new().route("api.lyrics.ovh", 200, r#"{"lyrics":"  \n "}"#);
    assert_eq!(LyricsOvh.fetch(&query(), &blank).await, Ok(None));
}

#[tokio::test]
async fn qq_music_decodes_base64_and_entities_and_gets_gap_repair() {
    let lrc = "[00:01.00]Don&apos;t hold on\n[00:40.00]The song returns\n";
    let encoded = base64::engine::general_purpose::STANDARD.encode(lrc);
    let search = serde_json::json!({"data": {"song": {"list": [
        {"songname": "Our Song", "singer": [{"name": "Somebody Else"}], "interval": 180, "songmid": "wrong"},
        {"songname": "Our Song", "singer": [{"name": "Our Artist"}, {"name": "Guest"}], "interval": 181, "songmid": "mid1"}
    ]}}});
    let http = FixtureHttp::new().route("client_search_cp", 200, search.to_string()).route(
        "fcg_query_lyric_new",
        200,
        serde_json::json!({"lyric": encoded}).to_string(),
    );
    let qq = QqMusic.fetch(&query(), &http).await.unwrap().unwrap();
    assert_eq!(qq.source, SourceId::QqMusic);
    assert_eq!(qq.artist, "Our Artist & Guest");
    assert_eq!(qq.duration, Some(181.0));
    let LyricResult::Synced { lines, provenance } = &qq.result else { panic!() };
    assert_eq!(lines[0].text, "Don't hold on");
    assert_eq!(provenance.source, "QQ Music");
    let lyric_url = http.urls().into_iter().find(|u| u.contains("fcg_query_lyric_new")).unwrap();
    assert!(lyric_url.contains("songmid=mid1"), "{lyric_url}");
    assert_eq!(header(&http, "client_search_cp", "Referer").as_deref(), Some("https://y.qq.com/"));
    // Both long windows (to 0:40 and to the 3:00 track end) gain an
    // instrumental line from the QQ repair.
    assert_eq!(prepare(qq.result.clone(), true).line_count(), 4);
    assert_eq!(prepare(qq.result, false).line_count(), 2);
}

#[tokio::test]
async fn qq_music_enhanced_lrc_scores_higher_and_errors_propagate() {
    let lrc = "[00:01.00]<00:01.00>Sing <00:01.50>along<00:02.00>\n[00:03.00]Next";
    let search = serde_json::json!({"data": {"song": {"list": [
        {"songname": "Our Song", "singer": [{"name": "Our Artist"}], "interval": 180, "songmid": "m"}
    ]}}});
    let http = FixtureHttp::new().route("client_search_cp", 200, search.to_string()).route(
        "fcg_query_lyric_new",
        200,
        serde_json::json!({"lyric": lrc}).to_string(),
    );
    let qq = QqMusic.fetch(&query(), &http).await.unwrap().unwrap();
    // 48 + 27 + 14 + 7 + 13 (enhanced)
    assert!((qq.score - 109.0).abs() < 1e-9);
    let broken = FixtureHttp::new().route("client_search_cp", 500, "");
    assert_eq!(QqMusic.fetch(&query(), &broken).await.unwrap_err().to_string(), "Service response: HTTP 500");
    let html = FixtureHttp::new().route("client_search_cp", 200, "<html>");
    assert_eq!(QqMusic.fetch(&query(), &html).await.unwrap_err().to_string(), "Unexpected response format");
}

#[tokio::test]
async fn kugou_searches_by_hash_and_downloads_base64_lrc() {
    let lrc = "[00:01.00]Sing along\n[00:04.00]Next line\n";
    let encoded = base64::engine::general_purpose::STANDARD.encode(lrc);
    let http = FixtureHttp::new()
        .route(
            "mobileservice.kugou.com",
            200,
            r#"{"data":{"info":[{"filename":"Our Artist - Our Song","hash":"ABC123","duration":180}]}}"#,
        )
        .route("lyrics.kugou.com/search", 200, r#"{"candidates":[{"id":42,"accesskey":"KEY"}]}"#)
        .route("lyrics.kugou.com/download", 200, serde_json::json!({"content": encoded}).to_string());
    let kugou = Kugou.fetch(&query(), &http).await.unwrap().unwrap();
    assert_eq!(kugou.source, SourceId::Kugou);
    assert_eq!((kugou.title.as_str(), kugou.artist.as_str()), ("Our Song", "Our Artist"), "from the file name");
    assert_eq!(kugou.result.provenance().source, "Kugou Music");
    let urls = http.urls();
    let search = urls.iter().find(|u| u.contains("lyrics.kugou.com/search")).unwrap();
    assert!(search.contains("duration=180000") && search.contains("hash=ABC123"), "{search}");
    let download = urls.iter().find(|u| u.contains("download")).unwrap();
    assert!(download.contains("id=42") && download.contains("accesskey=KEY"), "{download}");
}

#[tokio::test]
async fn kugou_without_lyric_candidates_is_no_match() {
    let http = FixtureHttp::new()
        .route(
            "mobileservice.kugou.com",
            200,
            r#"{"data":{"info":[{"songname":"Our Song","singername":"Our Artist","hash":"H","duration":180}]}}"#,
        )
        .route("lyrics.kugou.com/search", 200, r#"{"candidates":[]}"#);
    assert_eq!(Kugou.fetch(&query(), &http).await, Ok(None));
}

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

const KPOE_BODY: &str = r#"{"type":"Word","metadata":{"source":"Apple (via Lyrics+)"},"lyrics":[
  {"time":1000,"duration":2000,"text":"Sing along","syllabus":[
    {"time":1000,"duration":400,"text":"Sing "},{"time":1500,"duration":200,"text":"a"},{"time":1700,"duration":800,"text":"long"}]}]}"#;

#[tokio::test]
async fn lyrics_plus_falls_through_mirrors_and_inflates_gzip() {
    let http = FixtureHttp::new().route("lyricsplus.prjktla.workers.dev", 503, "").route(
        "lyricsplus.binimum.org",
        200,
        gzip(KPOE_BODY.as_bytes()),
    );
    let q = LyricQuery::new("Our Song", "Our Artist", Some("Our Album".into()), 180.9);
    let plus = LyricsPlus.fetch(&q, &http).await.unwrap().unwrap();
    assert_eq!(plus.source, SourceId::LyricsPlus);
    assert_eq!(plus.score, 113.0);
    assert_eq!(plus.result.provenance().source, "Apple via Lyrics+");
    assert_eq!(plus.result.provenance().precision, WORD_BY_WORD);
    let LyricResult::Synced { lines, .. } = &plus.result else { panic!() };
    let words: Vec<&str> = lines[0].words.iter().map(|w| w.text.as_str()).collect();
    assert_eq!(words, ["Sing", "along"]);
    let urls = http.urls();
    assert_eq!(urls.len(), 2);
    assert_eq!(
        urls[1],
        "https://lyricsplus.binimum.org/v2/lyrics/get?title=Our%20Song&artist=Our%20Artist&duration=180&album=Our%20Album"
    );
}

#[tokio::test]
async fn lyrics_plus_reads_failures_as_no_match() {
    let http = FixtureHttp::new().route("lyricsplus", 200, "{\"lyrics\":[]}");
    assert_eq!(LyricsPlus.fetch(&query(), &http).await, Ok(None));
    assert_eq!(http.requests().len(), 3, "every mirror is tried");
}
