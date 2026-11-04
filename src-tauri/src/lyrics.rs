//! Adapter between the shell and lunalyr-core: lookups, saved choices,
//! imports, palette extraction and readings. Japanese readings come from the
//! platform layer (Windows' phonetic analyser), everything else is portable.

use std::path::PathBuf;

use lunalyr_core::contract::{CandidateSummary, Palette};
use lunalyr_core::lookup::{self, LyricsService};
use lunalyr_core::model::{Candidate, Line, LyricQuery, LyricResult, Word};
use lunalyr_core::settings::{ReadingDisplay, SourceSettings};
use lunalyr_core::{palette, parse, romanize, timing};

use crate::controller::{LookupOutcome, LyricOps};
use crate::platform;

pub struct Lyrics {
    service: Option<LyricsService>,
}

impl Lyrics {
    pub fn new(cache_dir: PathBuf) -> Self {
        let service = match LyricsService::with_reqwest(cache_dir) {
            Ok(service) => Some(service),
            Err(error) => {
                // Only fails if the OS TLS backend cannot initialise.
                log::error!("lyric service unavailable: {error}");
                None
            }
        };
        Lyrics { service }
    }

    pub async fn lookup(&self, query: &LyricQuery, sources: &SourceSettings, force: bool) -> LookupOutcome {
        let Some(service) = self.service.as_ref() else { return LookupOutcome::default() };
        let report = service.lookup(query, sources, force).await;
        LookupOutcome {
            result: report.result,
            candidates: report.candidates,
            states: report.states,
            pinned: report.pinned,
            cached: report.cached,
        }
    }

    pub fn pin(&self, query: &LyricQuery, result: &LyricResult) -> bool {
        self.service.as_ref().map(|s| s.pin(query, result)).unwrap_or(false)
    }

    pub fn unpin(&self, query: &LyricQuery) {
        if let Some(service) = self.service.as_ref() {
            service.unpin(query);
        }
    }

    /// LRC or TTML text from the settings window's import.
    pub fn parse_import(&self, text: &str, duration: f64) -> Option<LyricResult> {
        parse::parse_import(text, duration)
    }

    pub fn palette(&self, bytes: &[u8]) -> Option<Palette> {
        palette::try_extract_palette(bytes)
    }

    pub fn readings(&self, lines: &[Line], display: ReadingDisplay, tone_marks: bool) -> Vec<Option<Vec<Word>>> {
        let japanese = |text: &str| platform::japanese_readings(text);
        romanize::readings(lines, display, tone_marks, &japanese)
    }
}

impl LyricOps for Lyrics {
    fn prepare(&self, result: LyricResult, repair_qq_gaps: bool) -> LyricResult {
        timing::prepare(result, repair_qq_gaps)
    }

    fn summary(&self, candidate: &Candidate, chosen: bool) -> CandidateSummary {
        lookup::summary(candidate, chosen)
    }
}
