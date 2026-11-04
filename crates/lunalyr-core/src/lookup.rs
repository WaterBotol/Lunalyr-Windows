//! The lyric lookup (`LyricsFetcher.lookup` + `bounded`): saved choice,
//! automatic cache, then every enabled provider in parallel with a time
//! budget, ranking, the final choice, and caching of that choice.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use tokio::task::JoinSet;

use crate::cache::LyricCache;
use crate::contract::{CandidateSummary, LookupInfo};
use crate::http::{HttpClient, HttpError, ReqwestClient};
use crate::model::{Candidate, LyricQuery, LyricResult, SourceId};
use crate::providers::{provider_for, states, Provider};
use crate::rank::{choose, ranked};
use crate::settings::SourceSettings;
use crate::timing::precision;

/// Time budget for LRCLIB (slow search endpoint).
pub const LRCLIB_BUDGET: Duration = Duration::from_secs(20);
/// Time budget for every other source.
pub const SOURCE_BUDGET: Duration = Duration::from_secs(8);

/// Budget for one source.
pub fn budget(id: SourceId) -> Duration {
    if id == SourceId::Lrclib {
        LRCLIB_BUDGET
    } else {
        SOURCE_BUDGET
    }
}

/// What a lookup found. `result` is the raw result; run
/// [`crate::timing::prepare`] before showing it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LookupReport {
    pub result: Option<LyricResult>,
    /// Plausible candidates in ranked order (empty for cache hits).
    pub candidates: Vec<Candidate>,
    /// Per enabled source, in the user's order: "Match found",
    /// "No usable match / unavailable", "Timed out", an error message, or
    /// "Not available in this version".
    pub states: Vec<(SourceId, String)>,
    /// `result` is the user's saved choice for this recording.
    pub pinned: bool,
    /// Served from disk without asking any provider.
    pub cached: bool,
}

impl LookupReport {
    /// Candidate summaries with `chosen` set on candidates equal to `result`.
    pub fn summaries(&self) -> Vec<CandidateSummary> {
        summaries(&self.candidates, self.result.as_ref())
    }

    /// The settings-window view of this report.
    pub fn info(&self) -> LookupInfo {
        LookupInfo {
            candidates: self.summaries(),
            states: self.states.clone(),
            pinned: self.pinned,
            cached: self.cached,
        }
    }
}

/// `contract::CandidateSummary` for one candidate. Synced results report
/// their recomputed precision; `has_readings` is true when any line carries
/// a source-supplied reading.
pub fn summary(candidate: &Candidate, chosen: bool) -> CandidateSummary {
    let (precision, has_readings) = match &candidate.result {
        LyricResult::Synced { lines, .. } => (
            precision(lines).to_string(),
            lines.iter().any(|l| l.romanization.as_deref().is_some_and(|r| !r.is_empty())),
        ),
        LyricResult::Plain { provenance, .. } => (provenance.precision.clone(), false),
    };
    CandidateSummary {
        source: candidate.source,
        source_title: candidate.source.title().to_string(),
        title: candidate.title.clone(),
        artist: candidate.artist.clone(),
        duration: candidate.duration,
        precision,
        line_count: candidate.result.line_count(),
        has_readings,
        chosen,
    }
}

/// Summaries for a candidate list; `chosen` marks candidates whose result
/// equals the shown one.
pub fn summaries(candidates: &[Candidate], shown: Option<&LyricResult>) -> Vec<CandidateSummary> {
    candidates.iter().map(|c| summary(c, shown == Some(&c.result))).collect()
}

/// Key of the automatic cache entry: recording key + settings fingerprint,
/// plus the local folder's modification time when local files are enabled
/// (macOS `LocalLyricOptions.fingerprint`), so new exports are picked up.
pub fn automatic_key(query: &LyricQuery, sources: &SourceSettings) -> String {
    let mut key = format!("{}|{}", query.recording_key(), sources.fingerprint());
    if sources.enabled.contains(&SourceId::LocalFiles) && !sources.local_folder.is_empty() {
        let modified = std::fs::metadata(&sources.local_folder)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0.0, |d| d.as_secs_f64());
        key.push_str(&format!("|{modified}"));
    }
    key
}

/// Runs one provider within its budget and turns the outcome into a state.
pub async fn bounded(
    provider: &dyn Provider,
    query: &LyricQuery,
    http: &dyn HttpClient,
) -> (Option<Candidate>, String) {
    match tokio::time::timeout(budget(provider.id()), provider.fetch(query, http)).await {
        Err(_) => (None, states::TIMED_OUT.to_string()),
        Ok(Ok(Some(candidate))) => (Some(candidate), states::MATCH_FOUND.to_string()),
        Ok(Ok(None)) => (None, states::NO_MATCH.to_string()),
        Ok(Err(error)) => (None, error.to_string()),
    }
}

/// Lyric lookups with the on-disk cache of choices.
#[derive(Clone)]
pub struct LyricsService {
    pub http: Arc<dyn HttpClient>,
    /// Directory for `cache-*.json` / `pin-*.json`.
    pub cache_dir: PathBuf,
}

impl LyricsService {
    pub fn new(http: Arc<dyn HttpClient>, cache_dir: impl Into<PathBuf>) -> Self {
        LyricsService { http, cache_dir: cache_dir.into() }
    }

    /// A service using [`ReqwestClient`].
    pub fn with_reqwest(cache_dir: impl Into<PathBuf>) -> Result<Self, HttpError> {
        Ok(Self::new(Arc::new(ReqwestClient::new()?), cache_dir))
    }

    pub fn cache(&self) -> LyricCache {
        LyricCache::new(self.cache_dir.clone())
    }

    /// Looks up lyrics for a recording.
    ///
    /// Unless `force`, the user's saved choice is returned first, then a
    /// fresh (< 14 days) automatic cache entry for the same settings. Then
    /// every enabled source runs in parallel (LRCLIB 20 s, others 8 s),
    /// candidates are ranked, one is chosen and cached. A saved choice still
    /// wins over a forced refresh's choice (`pinned` stays true).
    pub async fn lookup(&self, query: &LyricQuery, sources: &SourceSettings, force: bool) -> LookupReport {
        let cache = self.cache();
        let key = automatic_key(query, sources);
        let pinned = cache.read(&query.recording_key(), true);
        if !force {
            if let Some(result) = pinned {
                return LookupReport { result: Some(result), pinned: true, cached: true, ..Default::default() };
            }
            if let Some(result) = cache.read(&key, false) {
                return LookupReport { result: Some(result), cached: true, ..Default::default() };
            }
        }
        let enabled = sources.enabled_in_order();
        let mut tasks = JoinSet::new();
        let mut task_sources = HashMap::new();
        for id in &enabled {
            let provider = provider_for(*id, sources);
            let http = Arc::clone(&self.http);
            let query = query.clone();
            let handle = tasks.spawn(async move { bounded(provider.as_ref(), &query, http.as_ref()).await });
            task_sources.insert(handle.id(), *id);
        }
        let mut outcomes: HashMap<SourceId, (Option<Candidate>, String)> = HashMap::new();
        while let Some(joined) = tasks.join_next_with_id().await {
            match joined {
                Ok((task, outcome)) => {
                    if let Some(id) = task_sources.get(&task) {
                        outcomes.insert(*id, outcome);
                    }
                }
                Err(error) => {
                    if let Some(id) = task_sources.get(&error.id()) {
                        let state = if error.is_cancelled() { states::CANCELLED } else { "Unavailable" };
                        outcomes.insert(*id, (None, state.to_string()));
                    }
                }
            }
        }
        let mut report_states = Vec::with_capacity(enabled.len());
        let mut found = Vec::new();
        for id in &enabled {
            let (candidate, state) = outcomes.remove(id).unwrap_or((None, "Unavailable".to_string()));
            report_states.push((*id, state));
            found.extend(candidate);
        }
        let candidates = ranked(found, query, sources);
        let chosen = choose(&candidates, sources).map(|c| c.result.clone());
        if let Some(result) = &chosen {
            cache.save(result, &key, false);
        }
        LookupReport {
            pinned: pinned.is_some(),
            result: pinned.or(chosen),
            candidates,
            states: report_states,
            cached: false,
        }
    }

    /// Saves the user's choice for this recording (`LyricsViewModel.select`).
    pub fn pin(&self, query: &LyricQuery, result: &LyricResult) -> bool {
        self.cache().save(result, &query.recording_key(), true)
    }

    /// Forgets the user's choice for this recording.
    pub fn unpin(&self, query: &LyricQuery) {
        self.cache().unpin(&query.recording_key());
    }
}
