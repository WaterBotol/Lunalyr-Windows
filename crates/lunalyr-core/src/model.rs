//! Lyric data model. Mirrors `LyricsFetcher.Word/Line/Result` and
//! `LyricQuery` from the macOS sources (Sources/Lyrics/LyricsFetcher.swift,
//! Sources/Lyrics/LyricSources.swift).
//!
//! All times are **seconds** (f64) on the track timeline, exactly as on macOS.
//! The JSON form (camelCase) is the renderer contract — see docs/CONTRACT.md
//! and ui/src/contract.ts. Change all three together.

use serde::{Deserialize, Serialize};
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// One sung word (or syllable / CJK character) with its own timing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Word {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

/// One lyric line. `words` is either real per-word timing (`word_timed`) or
/// an estimate distributed across the line window (see
/// `distribute_words` in the timing module).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub words: Vec<Word>,
    pub is_instrumental: bool,
    #[serde(default)]
    pub word_timed: bool,
    /// A pronunciation reading supplied by the source (e.g. `.roma.lrc`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub romanization: Option<String>,
}

/// Where a lyric result came from and how precisely it is timed.
/// `precision` is one of "Word-by-word", "Mixed timing", "Line-synced",
/// "Not synced" (same strings as macOS).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    pub precision: String,
    pub source: String,
}

impl Provenance {
    pub fn label(&self) -> String {
        format!("{} · {}", self.precision, self.source)
    }
}

/// A complete lyric result for one recording.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LyricResult {
    Synced { lines: Vec<Line>, provenance: Provenance },
    Plain { lines: Vec<String>, provenance: Provenance },
}

impl LyricResult {
    pub fn provenance(&self) -> &Provenance {
        match self {
            LyricResult::Synced { provenance, .. } | LyricResult::Plain { provenance, .. } => provenance,
        }
    }
    pub fn line_count(&self) -> usize {
        match self {
            LyricResult::Synced { lines, .. } => lines.len(),
            LyricResult::Plain { lines, .. } => lines.len(),
        }
    }
}

/// Lyric sources. macOS-only sources (Apple Music's local cache) are absent.
/// `LocalFiles` is the portable form of the macOS "Lyrimuse · local files"
/// bridge (exported LRC / TTML / YRC files in a folder the user chooses).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceId {
    LocalFiles,
    LyricsPlus,
    NetEase,
    Lrclib,
    QqMusic,
    Kugou,
    Kuwo,
    LyricsOvh,
}

impl SourceId {
    pub const ALL: [SourceId; 8] = [
        SourceId::LocalFiles,
        SourceId::LyricsPlus,
        SourceId::NetEase,
        SourceId::Lrclib,
        SourceId::QqMusic,
        SourceId::Kugou,
        SourceId::Kuwo,
        SourceId::LyricsOvh,
    ];

    /// Same order as `LyricSourceOptions.recommendedOrder` on macOS, minus
    /// the Apple Music cache.
    pub const RECOMMENDED_ORDER: [SourceId; 8] = Self::ALL;

    pub fn title(self) -> &'static str {
        match self {
            SourceId::LocalFiles => "Local lyric files",
            SourceId::LyricsPlus => "Lyrics+ / KPoe",
            SourceId::NetEase => "NetEase Cloud Music",
            SourceId::Lrclib => "LRCLIB",
            SourceId::QqMusic => "QQ Music",
            SourceId::Kugou => "Kugou Music",
            SourceId::Kuwo => "Kuwo Music",
            SourceId::LyricsOvh => "lyrics.ovh",
        }
    }

    pub fn timing(self) -> &'static str {
        match self {
            SourceId::LocalFiles => "LRC / TTML / YRC + readings",
            SourceId::LyricsPlus => "Word / syllable timing",
            SourceId::NetEase | SourceId::Lrclib => "Word or line timing",
            SourceId::QqMusic | SourceId::Kugou | SourceId::Kuwo => "Line timing",
            SourceId::LyricsOvh => "Plain text fallback",
        }
    }

    /// Stable string id (matches the serde form).
    pub fn as_str(self) -> &'static str {
        match self {
            SourceId::LocalFiles => "localFiles",
            SourceId::LyricsPlus => "lyricsPlus",
            SourceId::NetEase => "netEase",
            SourceId::Lrclib => "lrclib",
            SourceId::QqMusic => "qqMusic",
            SourceId::Kugou => "kugou",
            SourceId::Kuwo => "kuwo",
            SourceId::LyricsOvh => "lyricsOvh",
        }
    }
}

/// What to look up. Mirrors `LyricQuery` on macOS.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricQuery {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    /// Seconds; 0 when unknown. Clamped to 0..=864000.
    pub duration: f64,
}

impl LyricQuery {
    pub fn new(title: impl Into<String>, artist: impl Into<String>, album: Option<String>, duration: f64) -> Self {
        let duration = if duration.is_finite() { duration.clamp(0.0, 864_000.0) } else { 0.0 };
        LyricQuery { title: title.into(), artist: artist.into(), album, duration }
    }

    /// Identifies one recording: normalised title | artist | album | rounded
    /// duration. Used for caches, saved choices and per-song offsets.
    pub fn recording_key(&self) -> String {
        [
            Self::normalize(&self.title),
            Self::normalize(&self.artist),
            Self::normalize(self.album.as_deref().unwrap_or("")),
            format!("{}", self.duration.round() as i64),
        ]
        .join("|")
    }

    /// Diacritic- and case-insensitive folding; every run of
    /// non-alphanumeric characters becomes one space.
    pub fn normalize(text: &str) -> String {
        let folded: String = text
            .nfd()
            .filter(|c| !is_combining_mark(*c))
            .flat_map(|c| c.to_lowercase())
            .collect();
        folded
            .split(|c: char| !c.is_alphanumeric())
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// 1 for equal normalised strings; otherwise the larger of token overlap
    /// (shared tokens / larger token count) and 0.85 for containment.
    pub fn similarity(lhs: &str, rhs: &str) -> f64 {
        let a = Self::normalize(lhs);
        let b = Self::normalize(rhs);
        if a.is_empty() || b.is_empty() {
            return 0.0;
        }
        if a == b {
            return 1.0;
        }
        let aa: std::collections::HashSet<&str> = a.split(' ').collect();
        let bb: std::collections::HashSet<&str> = b.split(' ').collect();
        let shared = aa.intersection(&bb).count() as f64;
        let token = shared / aa.len().max(bb.len()) as f64;
        let containment = if a.contains(&b) || b.contains(&a) { 0.85 } else { 0.0 };
        token.max(containment)
    }
}

/// One provider's best result for a query, before ranking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub source: SourceId,
    pub result: LyricResult,
    /// Title/artist as reported by the provider (used by the ranking filter).
    pub title: String,
    pub artist: String,
    pub duration: Option<f64>,
    /// Provider-internal score; not used across providers.
    pub score: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_folds_case_diacritics_and_punctuation() {
        assert_eq!(LyricQuery::normalize("Beyoncé — Halo (Live!)"), "beyonce halo live");
        assert_eq!(LyricQuery::normalize("  "), "");
        assert_eq!(LyricQuery::normalize("夜に駆ける"), "夜に駆ける");
    }

    #[test]
    fn similarity_matches_macos_rules() {
        assert_eq!(LyricQuery::similarity("Halo", "halo"), 1.0);
        assert_eq!(LyricQuery::similarity("", "x"), 0.0);
        // containment
        assert_eq!(LyricQuery::similarity("Halo", "Halo Remastered 2011"), 0.85);
        // token overlap: {a,b,c} vs {a,b,d} -> 2/3
        let s = LyricQuery::similarity("a b c", "a b d");
        assert!((s - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn recording_key_is_stable() {
        let q = LyricQuery::new("Halo", "Beyoncé", Some("I Am... Sasha Fierce".into()), 261.4);
        assert_eq!(q.recording_key(), "halo|beyonce|i am sasha fierce|261");
        let unknown = LyricQuery::new("x", "y", None, f64::NAN);
        assert_eq!(unknown.recording_key(), "x|y||0");
    }

    #[test]
    fn result_json_shape_is_the_renderer_contract() {
        let result = LyricResult::Synced {
            lines: vec![Line {
                start: 1.0,
                end: 2.5,
                text: "hi there".into(),
                words: vec![Word { text: "hi".into(), start: 1.0, end: 1.4 }],
                is_instrumental: false,
                word_timed: true,
                romanization: None,
            }],
            provenance: Provenance { precision: "Word-by-word".into(), source: "LRCLIB".into() },
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["kind"], "synced");
        assert_eq!(json["lines"][0]["isInstrumental"], false);
        assert_eq!(json["lines"][0]["wordTimed"], true);
        assert_eq!(json["lines"][0]["words"][0]["start"], 1.0);
        assert!(json["lines"][0].get("romanization").is_none());
        assert_eq!(serde_json::to_value(SourceId::NetEase).unwrap(), "netEase");
        assert_eq!(serde_json::to_value(SourceId::LyricsOvh).unwrap(), "lyricsOvh");
    }
}
