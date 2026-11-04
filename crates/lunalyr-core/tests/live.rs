//! Live service checks. Ignored by default (network, third-party uptime):
//!
//! ```text
//! cargo test -p lunalyr-core --test live -- --ignored --nocapture
//! ```
//!
//! They print only metadata (source, precision, line count), never lyric text.

use lunalyr_core::http::ReqwestClient;
use lunalyr_core::model::{Candidate, LyricQuery, LyricResult};
use lunalyr_core::providers::{Lrclib, NetEase, Provider};
use lunalyr_core::timing::prepare;

fn query() -> LyricQuery {
    LyricQuery::new("Bohemian Rhapsody", "Queen", Some("A Night at the Opera".into()), 354.0)
}

fn describe(name: &str, candidate: &Candidate) {
    let prepared = prepare(candidate.result.clone(), false);
    let lines = prepared.line_count();
    let timed = match &prepared {
        LyricResult::Synced { lines, .. } => lines.iter().filter(|l| l.word_timed).count(),
        LyricResult::Plain { .. } => 0,
    };
    println!(
        "{name}: precision = {}, source = {}, lines = {lines}, word-timed lines = {timed}, matched = {} / {} ({:?} s), score = {}",
        prepared.provenance().precision,
        prepared.provenance().source,
        candidate.title,
        candidate.artist,
        candidate.duration,
        candidate.score
    );
}

async fn run(provider: &dyn Provider, name: &str) {
    let http = ReqwestClient::new().expect("client");
    let candidate = provider.fetch(&query(), &http).await.expect("request").expect("a match");
    assert!(candidate.result.line_count() > 0);
    describe(name, &candidate);
}

#[tokio::test]
#[ignore = "network"]
async fn live_lrclib() {
    run(&Lrclib, "LRCLIB").await;
}

#[tokio::test]
#[ignore = "network"]
async fn live_netease() {
    run(&NetEase, "NetEase").await;
}

#[tokio::test]
#[ignore = "network"]
async fn live_lyrics_plus() {
    run(&lunalyr_core::providers::LyricsPlus, "Lyrics+").await;
}

#[tokio::test]
#[ignore = "network"]
async fn live_qq_music() {
    run(&lunalyr_core::providers::QqMusic, "QQ Music").await;
}

#[tokio::test]
#[ignore = "network"]
async fn live_kugou() {
    run(&lunalyr_core::providers::Kugou, "Kugou").await;
}

#[tokio::test]
#[ignore = "network"]
async fn live_kuwo() {
    run(&lunalyr_core::providers::Kuwo, "Kuwo").await;
}

#[tokio::test]
#[ignore = "network"]
async fn live_lyrics_ovh() {
    run(&lunalyr_core::providers::LyricsOvh, "lyrics.ovh").await;
}
