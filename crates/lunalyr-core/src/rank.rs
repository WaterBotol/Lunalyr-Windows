//! Cross-provider ranking and the final choice (`LyricsFetcher.ranked` /
//! `LyricsFetcher.choose`) plus the metadata scorer providers use
//! (`vc_match_score` in Sources/Support/ConstellationGeometry.c).

use crate::model::{Candidate, LyricQuery, SourceId};
use crate::settings::{MatchMode, SourceSettings};
use crate::timing::{has_word_timing, timing_tier};

/// Deterministic metadata score (`vc_match_score`). Title similarity below
/// 0.62, artist below 0.45, a known duration off by more than 25 s, or a
/// non-finite input scores -1000. Otherwise
/// `48·title + 27·artist + duration term + (20 word | 7 line) + 4 album`,
/// where the duration term is `max(-22, 14 - 2·delta)` when known.
pub fn match_score(
    title: f64,
    artist: f64,
    delta: f64,
    duration_known: bool,
    word_timed: bool,
    line_timed: bool,
    album_match: bool,
) -> f64 {
    if !title.is_finite() || !artist.is_finite() || !delta.is_finite() {
        return -1000.0;
    }
    if title < 0.62 || artist < 0.45 {
        return -1000.0;
    }
    if duration_known && delta > 25f64.max(0.0) {
        return -1000.0;
    }
    let duration = if duration_known { (14.0 - delta * 2.0).max(-22.0) } else { 0.0 };
    let timing = if word_timed {
        20.0
    } else if line_timed {
        7.0
    } else {
        0.0
    };
    title * 48.0 + artist * 27.0 + duration + timing + if album_match { 4.0 } else { 0.0 }
}

fn confidence(query: &LyricQuery, candidate: &Candidate) -> f64 {
    LyricQuery::similarity(&query.title, &candidate.title) + LyricQuery::similarity(&query.artist, &candidate.artist)
}

/// Whether a candidate plausibly is this recording: its source is enabled,
/// title similarity ≥ 0.75, artist ≥ 0.65, and (when both are known) the
/// duration is within max(3 s, 2 %).
pub fn is_plausible(candidate: &Candidate, query: &LyricQuery, sources: &SourceSettings) -> bool {
    if !sources.enabled.contains(&candidate.source)
        || LyricQuery::similarity(&query.title, &candidate.title) < 0.75
        || LyricQuery::similarity(&query.artist, &candidate.artist) < 0.65
    {
        return false;
    }
    match candidate.duration {
        Some(duration) if duration > 0.0 && query.duration > 0.0 => {
            (duration - query.duration).abs() <= 3f64.max(query.duration * 0.02)
        }
        _ => true,
    }
}

/// Filters implausible candidates and sorts the rest: timing tier (word >
/// mixed > line > plain), then confidence (title + artist similarity), then
/// the user's source order. Provider scores and catalogue size play no part.
pub fn ranked(candidates: Vec<Candidate>, query: &LyricQuery, sources: &SourceSettings) -> Vec<Candidate> {
    let order = sources.enabled_in_order();
    let position = |id: SourceId| order.iter().position(|s| *s == id).unwrap_or(999);
    let mut kept: Vec<(u8, f64, usize, Candidate)> = candidates
        .into_iter()
        .filter(|c| is_plausible(c, query, sources))
        .map(|c| (timing_tier(&c.result), confidence(query, &c), position(c.source), c))
        .collect();
    kept.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| a.2.cmp(&b.2))
    });
    kept.into_iter().map(|(_, _, _, c)| c).collect()
}

/// Index of the candidate to show (`LyricsFetcher.choose`), given candidates
/// already in [`ranked`] order.
///
/// * Only enabled sources count.
/// * In "Accuracy first" mode, or whenever word timing is preferred, only
///   candidates of the best timing tier remain.
/// * With "prefer word-timed LRCLIB", a word-timed LRCLIB result wins.
/// * "Source order" mode then takes the first remaining candidate in the
///   user's order; otherwise the first remaining (best ranked) one.
pub fn choose_index(candidates: &[Candidate], sources: &SourceSettings) -> Option<usize> {
    let mut remaining: Vec<usize> =
        (0..candidates.len()).filter(|&i| sources.enabled.contains(&candidates[i].source)).collect();
    if sources.mode == MatchMode::Smart || sources.prefer_word_timing {
        let best = remaining.iter().map(|&i| timing_tier(&candidates[i].result)).max().unwrap_or(0);
        remaining.retain(|&i| timing_tier(&candidates[i].result) == best);
    }
    if sources.prefer_word_timed_lrclib {
        if let Some(&native) = remaining
            .iter()
            .find(|&&i| candidates[i].source == SourceId::Lrclib && has_word_timing(&candidates[i].result))
        {
            return Some(native);
        }
    }
    if sources.mode == MatchMode::Priority {
        return sources
            .enabled_in_order()
            .into_iter()
            .find_map(|id| remaining.iter().copied().find(|&i| candidates[i].source == id));
    }
    remaining.first().copied()
}

/// The candidate to show; see [`choose_index`].
pub fn choose<'a>(candidates: &'a [Candidate], sources: &SourceSettings) -> Option<&'a Candidate> {
    choose_index(candidates, sources).map(|i| &candidates[i])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{LyricResult, Provenance, Word};
    use crate::timing::make_line;

    fn query() -> LyricQuery {
        LyricQuery::new("Our Song", "Our Artist", None, 180.0)
    }

    fn options(enabled: &[SourceId], mode: MatchMode) -> SourceSettings {
        SourceSettings { enabled: enabled.to_vec(), order: enabled.to_vec(), mode, ..SourceSettings::default() }
    }

    fn candidate(source: SourceId, word: bool, score: f64, title: &str, duration: f64) -> Candidate {
        let mut line = make_line("Sing along", 1.0, 3.0);
        if word {
            line.word_timed = true;
            line.words = vec![
                Word { text: "Sing".into(), start: 1.0, end: 2.0 },
                Word { text: "along".into(), start: 2.0, end: 3.0 },
            ];
        }
        Candidate {
            source,
            result: LyricResult::Synced {
                lines: vec![line],
                provenance: Provenance { precision: "fixture".into(), source: source.title().into() },
            },
            title: title.into(),
            artist: "Our Artist".into(),
            duration: Some(duration),
            score,
        }
    }

    fn simple(source: SourceId, word: bool) -> Candidate {
        candidate(source, word, 100.0, "Our Song", 180.0)
    }

    fn plain(source: SourceId) -> Candidate {
        Candidate {
            result: LyricResult::Plain {
                lines: vec!["Sing along".into()],
                provenance: Provenance { precision: "Not synced".into(), source: "x".into() },
            },
            ..simple(source, false)
        }
    }

    // Tests/geometry_test.c
    #[test]
    fn match_score_invariants() {
        assert!(
            match_score(1.0, 1.0, 1.0, true, true, true, true) > match_score(1.0, 1.0, 1.0, true, false, true, true)
        );
        assert!(match_score(1.0, 0.2, 0.0, true, true, true, true) < 0.0);
        assert!(match_score(1.0, 1.0, 80.0, true, true, true, true) < 0.0);
        assert!(match_score(f64::NAN, 1.0, 0.0, true, true, true, true) < 0.0);
        assert_eq!(match_score(1.0, 1.0, 0.0, false, false, true, false), 48.0 + 27.0 + 7.0);
        assert_eq!(match_score(1.0, 1.0, 20.0, true, false, false, true), 48.0 + 27.0 - 22.0 + 4.0);
        assert_eq!(match_score(1.0, 1.0, 80.0, false, false, false, false), 75.0, "unknown duration ignores delta");
        assert_eq!(match_score(0.61, 1.0, 0.0, false, false, false, false), -1000.0);
    }

    // LyricSelectionTests.swift
    #[test]
    fn line_only_priority_does_not_hide_available_real_words() {
        let o = options(&[SourceId::Lrclib, SourceId::NetEase], MatchMode::Priority);
        let values = vec![
            candidate(SourceId::Lrclib, false, 999.0, "Our Song", 180.0),
            candidate(SourceId::NetEase, true, 1.0, "Our Song", 180.0),
        ];
        let ranked = ranked(values, &query(), &o);
        assert_eq!(choose(&ranked, &o).unwrap().source, SourceId::NetEase);
    }

    #[test]
    fn explicit_strict_priority_is_still_respected() {
        let o = SourceSettings {
            prefer_word_timed_lrclib: false,
            prefer_word_timing: false,
            ..options(&[SourceId::Lrclib, SourceId::NetEase], MatchMode::Priority)
        };
        let values = vec![simple(SourceId::Lrclib, false), simple(SourceId::NetEase, true)];
        assert_eq!(choose(&values, &o).unwrap().source, SourceId::Lrclib);
        // even when the list is ranked (word timing first)
        let ranked = ranked(values, &query(), &o);
        assert_eq!(ranked[0].source, SourceId::NetEase);
        assert_eq!(choose(&ranked, &o).unwrap().source, SourceId::Lrclib);
    }

    #[test]
    fn accuracy_mode_uses_timing_rather_than_provider_score_or_catalogue() {
        let o = options(&[SourceId::QqMusic, SourceId::NetEase], MatchMode::Smart);
        let values = vec![
            candidate(SourceId::QqMusic, false, 999.0, "Our Song", 180.0),
            candidate(SourceId::NetEase, true, 1.0, "Our Song", 180.0),
        ];
        assert_eq!(ranked(values, &query(), &o)[0].source, SourceId::NetEase);
    }

    #[test]
    fn wrong_recording_cannot_win_with_perfect_word_timing() {
        let o = options(&[SourceId::LyricsPlus, SourceId::Lrclib], MatchMode::Smart);
        let values = vec![
            candidate(SourceId::LyricsPlus, true, 100.0, "Unrelated Song", 230.0),
            simple(SourceId::Lrclib, false),
        ];
        let sources: Vec<SourceId> = ranked(values, &query(), &o).iter().map(|c| c.source).collect();
        assert_eq!(sources, vec![SourceId::Lrclib]);
    }

    #[test]
    fn source_order_breaks_equal_quality_ties() {
        let o = options(&[SourceId::NetEase, SourceId::LyricsPlus], MatchMode::Smart);
        let values = vec![
            candidate(SourceId::LyricsPlus, true, 999.0, "Our Song", 180.0),
            candidate(SourceId::NetEase, true, 1.0, "Our Song", 180.0),
        ];
        assert_eq!(ranked(values, &query(), &o)[0].source, SourceId::NetEase);
    }

    #[test]
    fn plausibility_filters() {
        let o = options(&[SourceId::Lrclib, SourceId::NetEase], MatchMode::Smart);
        let q = query();
        assert!(is_plausible(&simple(SourceId::Lrclib, false), &q, &o));
        assert!(!is_plausible(&simple(SourceId::Kugou, false), &q, &o), "disabled source");
        let short = LyricQuery::new("Our Song", "Our Artist", None, 100.0);
        assert!(is_plausible(&candidate(SourceId::Lrclib, false, 0.0, "Our Song", 103.0), &short, &o), "3 s");
        assert!(!is_plausible(&candidate(SourceId::Lrclib, false, 0.0, "Our Song", 103.5), &short, &o));
        assert!(is_plausible(&candidate(SourceId::Lrclib, false, 0.0, "Our Song", 183.5), &q, &o), "2 % of 180");
        assert!(!is_plausible(&candidate(SourceId::Lrclib, false, 0.0, "Our Song", 183.7), &q, &o));
        let long = LyricQuery::new("Our Song", "Our Artist", None, 600.0);
        assert!(is_plausible(&candidate(SourceId::Lrclib, false, 0.0, "Our Song", 611.0), &long, &o), "2 %");
        assert!(is_plausible(&candidate(SourceId::Lrclib, false, 0.0, "Our Song", 0.0), &q, &o), "unknown");
        let unknown = LyricQuery::new("Our Song", "Our Artist", None, 0.0);
        assert!(is_plausible(&candidate(SourceId::Lrclib, false, 0.0, "Our Song", 999.0), &unknown, &o));
        let mut other_artist = simple(SourceId::Lrclib, false);
        other_artist.artist = "Somebody Else".into();
        assert!(!is_plausible(&other_artist, &q, &o));
        assert!(is_plausible(&candidate(SourceId::Lrclib, false, 0.0, "Our Song (Remastered)", 180.0), &q, &o));
    }

    #[test]
    fn ranking_orders_tiers_then_confidence_then_source_order() {
        let o = options(&[SourceId::Kugou, SourceId::Lrclib, SourceId::NetEase, SourceId::LyricsOvh], MatchMode::Smart);
        let mut loose = simple(SourceId::Kugou, false);
        loose.title = "Our Song (Live)".into(); // 0.85 containment
        let values =
            vec![plain(SourceId::LyricsOvh), loose, simple(SourceId::Lrclib, false), simple(SourceId::NetEase, true)];
        let order: Vec<SourceId> = ranked(values, &query(), &o).iter().map(|c| c.source).collect();
        assert_eq!(order, vec![SourceId::NetEase, SourceId::Lrclib, SourceId::Kugou, SourceId::LyricsOvh]);
    }

    #[test]
    fn choose_matrix() {
        let both = [SourceId::NetEase, SourceId::Lrclib];
        // Smart: best tier wins, ranked order breaks ties.
        let o = options(&both, MatchMode::Smart);
        let values = ranked(vec![simple(SourceId::Lrclib, false), simple(SourceId::NetEase, true)], &query(), &o);
        assert_eq!(choose(&values, &o).unwrap().source, SourceId::NetEase);
        // Word-timed LRCLIB beats an equally word-timed earlier source.
        let values = ranked(vec![simple(SourceId::Lrclib, true), simple(SourceId::NetEase, true)], &query(), &o);
        assert_eq!(values[0].source, SourceId::NetEase);
        assert_eq!(choose(&values, &o).unwrap().source, SourceId::Lrclib);
        // ...unless that preference is off.
        let off = SourceSettings { prefer_word_timed_lrclib: false, ..o.clone() };
        assert_eq!(choose(&values, &off).unwrap().source, SourceId::NetEase);
        // Priority + prefer word timing: tier filter first, then source order.
        let p = options(&both, MatchMode::Priority);
        let values = vec![simple(SourceId::Lrclib, false), simple(SourceId::NetEase, false)];
        let p_no_lrclib = SourceSettings { prefer_word_timed_lrclib: false, ..p.clone() };
        assert_eq!(choose(&values, &p_no_lrclib).unwrap().source, SourceId::NetEase);
        let p_reordered = SourceSettings { order: vec![SourceId::Lrclib, SourceId::NetEase], ..p_no_lrclib };
        assert_eq!(choose(&values, &p_reordered).unwrap().source, SourceId::Lrclib);
        // Plain text only when nothing synced exists.
        let values = vec![plain(SourceId::NetEase), simple(SourceId::Lrclib, false)];
        assert_eq!(choose(&values, &o).unwrap().source, SourceId::Lrclib);
        let values = vec![plain(SourceId::NetEase)];
        assert_eq!(choose(&values, &o).unwrap().source, SourceId::NetEase);
        // Disabled sources never win; nothing enabled -> None.
        let only_netease = options(&[SourceId::NetEase], MatchMode::Smart);
        assert!(choose(&[simple(SourceId::Lrclib, true)], &only_netease).is_none());
        assert!(choose(&[], &o).is_none());
    }
}
