//! Lyric providers. Each returns its single best candidate for a query
//! (`LyricsFetcher.fetch` and the per-source adapters on macOS).

use async_trait::async_trait;

use crate::http::{HttpClient, HttpError};
use crate::model::{Candidate, LyricQuery, SourceId};
use crate::settings::SourceSettings;

pub mod additional;
pub mod kpoe;
pub mod local_files;
pub mod lrclib;
pub mod netease;

pub use additional::{Kugou, Kuwo, LyricsOvh, QqMusic};
pub use kpoe::LyricsPlus;
pub use local_files::LocalFiles;
pub use lrclib::Lrclib;
pub use netease::NetEase;

/// `User-Agent` sent to LRCLIB (its API asks clients to identify themselves).
pub const LRCLIB_USER_AGENT: &str = "Lunalyr-Windows/0.1 (https://github.com/WaterBotol/Lunalyr)";
/// `User-Agent` for the other public lyric endpoints (macOS `LyricHTTP`).
pub const USER_AGENT: &str = "Lunalyr-Windows/0.1 (personal desktop lyrics)";

/// State strings reported per source (same wording as macOS).
pub mod states {
    pub const MATCH_FOUND: &str = "Match found";
    pub const NO_MATCH: &str = "No usable match / unavailable";
    pub const TIMED_OUT: &str = "Timed out";
    pub const CANCELLED: &str = "Cancelled";
    pub const NOT_AVAILABLE: &str = "Not available in this version";
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ProviderError {
    #[error("{0}")]
    Http(#[from] HttpError),
    /// A service-specific failure with a user-facing message.
    #[error("{0}")]
    Service(String),
    /// The source has no implementation in this build.
    #[error("Not available in this version")]
    NotAvailable,
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> SourceId;
    /// The provider's best candidate, `Ok(None)` when nothing usable matched.
    async fn fetch(&self, query: &LyricQuery, http: &dyn HttpClient) -> Result<Option<Candidate>, ProviderError>;
}

/// Stand-in for a source that is not implemented in a build; reports
/// [`ProviderError::NotAvailable`] instead of failing silently. Every
/// [`SourceId`] currently has a provider.
#[derive(Debug, Clone, Copy)]
pub struct Unavailable(pub SourceId);

#[async_trait]
impl Provider for Unavailable {
    fn id(&self) -> SourceId {
        self.0
    }
    async fn fetch(&self, _query: &LyricQuery, _http: &dyn HttpClient) -> Result<Option<Candidate>, ProviderError> {
        Err(ProviderError::NotAvailable)
    }
}

/// The provider for a source under the given settings.
pub fn provider_for(id: SourceId, sources: &SourceSettings) -> Box<dyn Provider> {
    match id {
        SourceId::LocalFiles => Box::new(LocalFiles { folder: sources.local_folder.clone() }),
        SourceId::LyricsPlus => Box::new(LyricsPlus),
        SourceId::NetEase => Box::new(NetEase),
        SourceId::Lrclib => Box::new(Lrclib),
        SourceId::QqMusic => Box::new(QqMusic),
        SourceId::Kugou => Box::new(Kugou),
        SourceId::Kuwo => Box::new(Kuwo),
        SourceId::LyricsOvh => Box::new(LyricsOvh),
    }
}

/// Swift `max { $0.score < $1.score }`: the *first* candidate with the
/// highest score.
pub(crate) fn first_best(candidates: Vec<Candidate>) -> Option<Candidate> {
    let mut best: Option<Candidate> = None;
    for candidate in candidates {
        let better = match &best {
            None => true,
            Some(current) => current.score < candidate.score,
        };
        if better {
            best = Some(candidate);
        }
    }
    best
}
