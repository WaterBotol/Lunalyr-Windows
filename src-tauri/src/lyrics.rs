//! Adapter between the shell and lunalyr-core's lyric service.
//!
//! TEMPORARY: lunalyr-core's providers/lookup/palette modules are being
//! written; until they land this adapter reports "not found" so the rest of
//! the pipeline (events, windows, renderer) can be exercised end to end.

use std::path::PathBuf;

use lunalyr_core::contract::{CandidateSummary, Palette};
use lunalyr_core::model::{Candidate, LyricQuery, LyricResult, Word};
use lunalyr_core::settings::{ReadingDisplay, SourceSettings};

use crate::controller::{LookupOutcome, LyricOps};

pub struct Lyrics {
    #[allow(dead_code)]
    cache_dir: PathBuf,
}

impl Lyrics {
    pub fn new(cache_dir: PathBuf) -> Self {
        Lyrics { cache_dir }
    }

    pub async fn lookup(&self, _query: &LyricQuery, _sources: &SourceSettings, _force: bool) -> LookupOutcome {
        LookupOutcome::default()
    }

    pub fn pin(&self, _query: &LyricQuery, _result: &LyricResult) -> bool {
        false
    }

    pub fn unpin(&self, _query: &LyricQuery) {}

    pub fn parse_import(&self, _text: &str, _duration: f64) -> Option<LyricResult> {
        None
    }

    pub fn palette(&self, _bytes: &[u8]) -> Option<Palette> {
        None
    }

    pub fn readings(
        &self,
        lines: &[lunalyr_core::model::Line],
        _display: ReadingDisplay,
        _tone_marks: bool,
    ) -> Vec<Option<Vec<Word>>> {
        vec![None; lines.len()]
    }
}

impl LyricOps for Lyrics {
    fn prepare(&self, result: LyricResult, _repair_qq_gaps: bool) -> LyricResult {
        result
    }

    fn summary(&self, candidate: &Candidate, chosen: bool) -> CandidateSummary {
        CandidateSummary {
            source: candidate.source,
            source_title: candidate.source.title().into(),
            title: candidate.title.clone(),
            artist: candidate.artist.clone(),
            duration: candidate.duration,
            precision: candidate.result.provenance().precision.clone(),
            line_count: candidate.result.line_count(),
            has_readings: false,
            chosen,
        }
    }
}
