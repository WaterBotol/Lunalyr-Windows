//! End-to-end lookups with canned HTTP responses and a temporary cache.

use std::sync::Arc;
use std::time::Duration;

use lunalyr_core::http::FixtureHttp;
use lunalyr_core::lookup::{automatic_key, summaries, LyricsService};
use lunalyr_core::model::{LyricQuery, LyricResult, Provenance, SourceId};
use lunalyr_core::settings::{MatchMode, SourceSettings};
use lunalyr_core::timing::{prepare, LINE_SYNCED, WORD_BY_WORD};

const SYNCED: &str = "[00:12.00]Lanterns drift over the water\n[00:16.50]Carry the light to the shore\n";

fn query() -> LyricQuery {
    LyricQuery::new("Paper Lanterns", "Glass Harbour", Some("Night Ferry".into()), 214.6)
}

fn sources(enabled: &[SourceId]) -> SourceSettings {
    SourceSettings { enabled: enabled.to_vec(), ..SourceSettings::default() }
}

fn lrclib_body() -> String {
    serde_json::json!({
        "trackName": "Paper Lanterns", "artistName": "Glass Harbour", "albumName": "Night Ferry",
        "duration": 214.0, "syncedLyrics": SYNCED
    })
    .to_string()
}

fn netease_search() -> String {
    serde_json::json!({"result": {"songs": [
        {"id": 22, "name": "Paper Lanterns", "artists": [{"name": "Glass Harbour"}], "duration": 214500}
    ]}})
    .to_string()
}

fn netease_lyric() -> String {
    let yrc = "[12000,4000](12000,600,0)Lanterns (12600,500,0)drift\n[16500,3000](16500,700,0)Carry (17200,400,0)on";
    serde_json::json!({"yrc": {"lyric": yrc}}).to_string()
}

fn full_http() -> FixtureHttp {
    FixtureHttp::new()
        .route("lrclib.net/api/get", 200, lrclib_body())
        .route("music.163.com/api/search/get", 200, netease_search())
        .route("music.163.com/api/song/lyric", 200, netease_lyric())
}

fn service(http: Arc<FixtureHttp>, dir: &std::path::Path) -> LyricsService {
    LyricsService::new(http, dir.to_path_buf())
}

#[tokio::test]
async fn parallel_lookup_ranks_chooses_and_caches() {
    let dir = tempfile::tempdir().unwrap();
    let http = Arc::new(full_http());
    let lyrics = service(http.clone(), dir.path());
    let settings = sources(&[SourceId::NetEase, SourceId::Lrclib]);

    let report = lyrics.lookup(&query(), &settings, false).await;
    assert!(!report.cached && !report.pinned);
    assert_eq!(
        report.states,
        vec![(SourceId::NetEase, "Match found".to_string()), (SourceId::Lrclib, "Match found".to_string())]
    );
    let order: Vec<SourceId> = report.candidates.iter().map(|c| c.source).collect();
    assert_eq!(order, vec![SourceId::NetEase, SourceId::Lrclib], "word timing ranks first");
    let result = report.result.clone().unwrap();
    assert_eq!(result.provenance().source, "NetEase");
    assert_eq!(prepare(result, true).provenance().precision, WORD_BY_WORD);

    let info = report.info();
    assert_eq!(info.candidates.len(), 2);
    assert!(info.candidates[0].chosen && !info.candidates[1].chosen);
    assert_eq!(info.candidates[1].precision, LINE_SYNCED);
    assert_eq!(info.candidates[0].source_title, "NetEase Cloud Music");
    assert!(!info.candidates[0].has_readings);

    // Second lookup is served from the automatic cache without network.
    let before = http.requests().len();
    let again = lyrics.lookup(&query(), &settings, false).await;
    assert!(again.cached && !again.pinned);
    assert_eq!(again.result, report.result);
    assert!(again.candidates.is_empty());
    assert_eq!(http.requests().len(), before);

    // A different policy does not reuse that cache entry.
    let priority = SourceSettings { mode: MatchMode::Priority, ..settings.clone() };
    assert_ne!(automatic_key(&query(), &priority), automatic_key(&query(), &settings));
    let fresh = lyrics.lookup(&query(), &priority, false).await;
    assert!(!fresh.cached);
    assert!(http.requests().len() > before);
}

#[tokio::test]
async fn pins_win_and_survive_forced_refresh() {
    let dir = tempfile::tempdir().unwrap();
    let http = Arc::new(full_http());
    let lyrics = service(http.clone(), dir.path());
    let settings = sources(&[SourceId::NetEase, SourceId::Lrclib]);
    let mine = LyricResult::Plain {
        lines: vec!["my own".into()],
        provenance: Provenance { precision: "Not synced".into(), source: "Imported lyrics".into() },
    };
    assert!(lyrics.pin(&query(), &mine));

    let report = lyrics.lookup(&query(), &settings, false).await;
    assert!(report.pinned && report.cached);
    assert_eq!(report.result.as_ref(), Some(&mine));
    assert!(http.requests().is_empty());

    let forced = lyrics.lookup(&query(), &settings, true).await;
    assert!(forced.pinned && !forced.cached);
    assert_eq!(forced.result.as_ref(), Some(&mine), "the saved choice still shows");
    assert_eq!(forced.candidates.len(), 2, "but fresh candidates are listed");
    assert!(summaries(&forced.candidates, forced.result.as_ref()).iter().all(|s| !s.chosen));

    lyrics.unpin(&query());
    let after = lyrics.lookup(&query(), &settings, false).await;
    assert!(!after.pinned);
    assert_eq!(after.result.unwrap().provenance().source, "NetEase", "automatic cache from the forced run");
}

#[tokio::test]
async fn unavailable_and_failing_sources_report_states() {
    let dir = tempfile::tempdir().unwrap();
    let http =
        Arc::new(FixtureHttp::new().route("lrclib.net/api/get", 500, "").route("lrclib.net/api/search", 500, ""));
    let lyrics = service(http, dir.path());
    let settings = sources(&[SourceId::Lrclib, SourceId::NetEase, SourceId::Kuwo]);
    let report = lyrics.lookup(&query(), &settings, false).await;
    assert!(report.result.is_none());
    assert!(report.candidates.is_empty());
    let states: std::collections::HashMap<SourceId, String> = report.states.into_iter().collect();
    assert_eq!(states[&SourceId::Lrclib], "LRCLIB: HTTP 500. The service may be temporarily unavailable.");
    assert_eq!(states[&SourceId::NetEase], "No usable match / unavailable");
    assert!(states.contains_key(&SourceId::Kuwo));
    // Nothing chosen, nothing cached.
    assert_eq!(std::fs::read_dir(dir.path()).map(|d| d.count()).unwrap_or(0), 0);
}

#[tokio::test(start_paused = true)]
async fn per_source_time_budgets() {
    let dir = tempfile::tempdir().unwrap();
    let http = Arc::new(
        FixtureHttp::new()
            // LRCLIB answers after 15 s: inside its 20 s budget.
            .route_delayed("lrclib.net/api/get", Duration::from_secs(15), 200, lrclib_body())
            // NetEase answers after 9 s: outside the 8 s budget.
            .route_delayed("music.163.com/api/search/get", Duration::from_secs(9), 200, netease_search()),
    );
    let lyrics = service(http, dir.path());
    let settings = sources(&[SourceId::NetEase, SourceId::Lrclib]);
    let started = tokio::time::Instant::now();
    let report = lyrics.lookup(&query(), &settings, false).await;
    let states: std::collections::HashMap<SourceId, String> = report.states.into_iter().collect();
    assert_eq!(states[&SourceId::NetEase], "Timed out");
    assert_eq!(states[&SourceId::Lrclib], "Match found");
    assert_eq!(report.result.unwrap().provenance().source, "LRCLIB");
    assert!(started.elapsed() < Duration::from_secs(20));
}

#[tokio::test]
async fn implausible_candidates_are_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let wrong = serde_json::json!({
        "trackName": "Paper Lanterns", "artistName": "Glass Harbour", "duration": 236.0, "syncedLyrics": SYNCED
    });
    // LRCLIB itself accepts a 21 s difference, the cross-source filter does not.
    let http = Arc::new(FixtureHttp::new().route("lrclib.net/api/get", 200, wrong.to_string()));
    let lyrics = service(http, dir.path());
    let report = lyrics.lookup(&query(), &sources(&[SourceId::Lrclib]), false).await;
    assert_eq!(report.states, vec![(SourceId::Lrclib, "Match found".to_string())]);
    assert!(report.candidates.is_empty());
    assert!(report.result.is_none());
}

#[tokio::test]
async fn local_files_are_wired_from_settings() {
    let cache = tempfile::tempdir().unwrap();
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(
        folder.path().join("Glass Harbour - Paper Lanterns.lrc"),
        "[ti:Paper Lanterns]\n[ar:Glass Harbour]\n[00:12.00]Lanterns drift\n[00:16.50]Carry the light",
    )
    .unwrap();
    std::fs::write(folder.path().join("Glass Harbour - Paper Lanterns.roma.lrc"), "[00:12.00]a reading").unwrap();
    let http = Arc::new(FixtureHttp::new());
    let lyrics = service(http, cache.path());
    let settings = SourceSettings {
        enabled: vec![SourceId::LocalFiles, SourceId::Lrclib],
        local_folder: folder.path().to_string_lossy().into_owned(),
        ..SourceSettings::default()
    };
    let report = lyrics.lookup(&query(), &settings, false).await;
    assert_eq!(
        report.states,
        vec![
            (SourceId::LocalFiles, "Match found".to_string()),
            (SourceId::Lrclib, "No usable match / unavailable".to_string())
        ]
    );
    assert_eq!(report.result.as_ref().unwrap().provenance().source, "Local lyric files");
    let info = report.info();
    assert!(info.candidates[0].chosen && info.candidates[0].has_readings);
    assert!(automatic_key(&query(), &settings).contains(&settings.local_folder));

    let unset = SourceSettings { local_folder: String::new(), ..settings };
    let report = lyrics.lookup(&query(), &unset, true).await;
    assert_eq!(report.states[0], (SourceId::LocalFiles, "Choose the local lyrics folder below".to_string()));
}

#[test]
fn service_and_lookup_future_can_cross_threads() {
    fn send<T: Send>(_: &T) {}
    fn send_sync<T: Send + Sync>(_: &T) {}
    let dir = tempfile::tempdir().unwrap();
    let lyrics = service(Arc::new(FixtureHttp::new()), dir.path());
    send_sync(&lyrics);
    let settings = SourceSettings::default();
    let q = query();
    let future = lyrics.lookup(&q, &settings, false);
    send(&future);
}
