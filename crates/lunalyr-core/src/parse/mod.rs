//! Lyric file formats: LRC (with enhanced word stamps), NetEase YRC, LRCLIB's
//! `lyricsfile` YAML and exported TTML, plus user imports and saved
//! readings (`.roma.lrc`).

mod lrc;
mod lyricsfile;
mod ttml;
mod xml;
mod yaml;
mod yrc;

pub use lrc::{parse_lrc, MAX_LRC_BYTES};
pub use lyricsfile::{parse_lyricsfile, MAX_LYRICSFILE_BYTES};
pub use ttml::{parse_ttml, ttml_seconds, MAX_TTML_BYTES};
pub use yrc::{parse_yrc, MAX_WORD_DURATION, MIN_WORD_DURATION};

use crate::model::{Line, LyricResult, Provenance};
use crate::text::trim_all;
use crate::timing::precision;

/// Source label of imported lyrics.
pub const IMPORTED_SOURCE: &str = "Imported lyrics";

/// Parses a user-imported lyric file (`LyricsViewModel.importLyrics`):
/// TTML when it starts with `<`, LRC otherwise. None when it has no usable
/// timestamps.
pub fn parse_import(text: &str, duration: f64) -> Option<LyricResult> {
    let lines = if trim_all(text).starts_with('<') { parse_ttml(text, duration) } else { parse_lrc(text, duration) };
    if lines.is_empty() {
        return None;
    }
    Some(LyricResult::Synced {
        provenance: Provenance { precision: precision(&lines).to_string(), source: IMPORTED_SOURCE.to_string() },
        lines,
    })
}

/// Attaches readings from a timed reading file (e.g. `.roma.lrc`) to the
/// lines they belong to (`LocalLyricBridge.attachingReadings`): the reading
/// line whose start is nearest (within 0.18 s) and not instrumental.
pub fn attach_readings(text: &str, lines: &[Line], duration: f64) -> Vec<Line> {
    let readings = parse_lrc(text, duration);
    lines
        .iter()
        .map(|line| {
            let mut result = line.clone();
            let mut nearest: Option<&Line> = None;
            for reading in &readings {
                let closer = match nearest {
                    None => true,
                    Some(best) => (reading.start - line.start).abs() < (best.start - line.start).abs(),
                };
                if closer {
                    nearest = Some(reading);
                }
            }
            if let Some(reading) = nearest {
                if (reading.start - line.start).abs() <= 0.18 && !reading.is_instrumental {
                    result.romanization = Some(reading.text.clone());
                }
            }
            result
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timing::{LINE_SYNCED, WORD_BY_WORD};

    #[test]
    fn readings_attach_by_timestamp_rather_than_line_index() {
        let lines = parse_lrc("[00:01]one\n[00:03]two\n[00:05]three", 8.0);
        let attached = attach_readings("[00:03.10]second reading\n[00:05]third reading", &lines, 8.0);
        assert_eq!(attached[0].romanization, None);
        assert_eq!(attached[1].romanization.as_deref(), Some("second reading"));
        assert_eq!(attached[2].romanization.as_deref(), Some("third reading"));
        let far = attach_readings("[00:03.20]too far", &lines, 8.0);
        assert_eq!(far[1].romanization, None);
        let instrumental = attach_readings("[00:03.00]♪", &lines, 8.0);
        assert_eq!(instrumental[1].romanization, None);
    }

    #[test]
    fn imports_detect_ttml_or_lrc() {
        let lrc = parse_import("[00:01]Hello there\n[00:04]Again", 10.0).unwrap();
        assert_eq!(lrc.provenance().source, IMPORTED_SOURCE);
        assert_eq!(lrc.provenance().precision, LINE_SYNCED);
        let ttml = parse_import(
            "  <tt><body><p begin='1s' end='2s'><span begin='1s' end='2s'>Hi</span></p></body></tt>",
            10.0,
        )
        .unwrap();
        assert_eq!(ttml.provenance().precision, WORD_BY_WORD);
        assert!(parse_import("no timestamps here", 10.0).is_none());
        assert!(parse_import("<not ttml", 10.0).is_none());
    }
}
