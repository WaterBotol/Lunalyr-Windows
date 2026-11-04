//! Exported lyric files in a folder the user chooses
//! (`LocalLyricBridge.candidate` in Sources/Lyrics/LocalLyricBridge.swift).
//!
//! `<name>.lrc` files carry `[ti:]` / `[ar:]` / `[al:]` tags and line timing;
//! a sibling `<name>.ttml` (with word timing) or `<name>.yrc` replaces the
//! LRC timing, and `<name>.roma.lrc` attaches readings. Read-only: nothing
//! is written, launched or installed.

use std::path::{Path, PathBuf};

use async_trait::async_trait;

use super::{Provider, ProviderError};
use crate::http::HttpClient;
use crate::model::{Candidate, LyricQuery, LyricResult, Provenance, SourceId};
use crate::parse::{attach_readings, parse_lrc, parse_ttml, parse_yrc};
use crate::rank::match_score;
use crate::text::{split_newlines, trim_all};
use crate::timing::precision;

/// Largest lyric file read.
pub const MAX_FILE_BYTES: u64 = 2_000_000;
/// Folders with more entries are refused.
pub const MAX_FOLDER_ENTRIES: usize = 30_000;
/// Matching `.lrc` files examined per lookup.
pub const MAX_CANDIDATE_FILES: usize = 150;
/// Provenance source for local files.
pub const SOURCE_LABEL: &str = "Local lyric files";

/// Decodes UTF-8 (with or without BOM) or UTF-16 (BOM, or little-endian).
pub(crate) fn decode_text(bytes: &[u8]) -> Option<String> {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(rest.to_vec()).ok();
    }
    let utf16 = |data: &[u8], little: bool| -> Option<String> {
        if data.len() % 2 != 0 {
            return None;
        }
        let units: Vec<u16> = data
            .chunks_exact(2)
            .map(|p| if little { u16::from_le_bytes([p[0], p[1]]) } else { u16::from_be_bytes([p[0], p[1]]) })
            .collect();
        String::from_utf16(&units).ok()
    };
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, true);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, false);
    }
    String::from_utf8(bytes.to_vec()).ok().or_else(|| utf16(bytes, true))
}

fn read_text(path: &Path) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return None;
    }
    decode_text(&bytes)
}

/// An LRC header tag value (`[ti:Title]`), searched in the first 12 000
/// characters; case-insensitive key, trimmed, None when empty.
pub fn lrc_tag(key: &str, text: &str) -> Option<String> {
    let prefix = format!("[{}:", key.to_lowercase());
    let head: String = text.chars().take(12_000).collect();
    for raw in split_newlines(&head) {
        let line = trim_all(raw);
        if line.to_lowercase().starts_with(&prefix) {
            if let Some(close) = line.find(']') {
                let value = trim_all(line.get(prefix.len()..close).unwrap_or(""));
                return (!value.is_empty()).then(|| value.to_string());
            }
        }
    }
    None
}

fn is_hidden(entry: &std::fs::DirEntry) -> bool {
    if entry.file_name().to_string_lossy().starts_with('.') {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        if entry.metadata().is_ok_and(|m| m.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0) {
            return true;
        }
    }
    false
}

fn with_extension(base: &Path, extension: &str) -> PathBuf {
    let mut name = base.as_os_str().to_os_string();
    name.push(".");
    name.push(extension);
    PathBuf::from(name)
}

/// The best local candidate for a query (blocking file IO).
pub fn find_candidate(query: &LyricQuery, folder: &str) -> Result<Option<Candidate>, ProviderError> {
    if folder.is_empty() {
        return Err(ProviderError::Service("Choose the local lyrics folder below".into()));
    }
    let entries: Vec<std::fs::DirEntry> = std::fs::read_dir(folder)
        .map_err(|e| ProviderError::Service(format!("The lyrics folder can't be read ({e}).")))?
        .filter_map(Result::ok)
        .filter(|e| !is_hidden(e))
        .collect();
    if entries.len() > MAX_FOLDER_ENTRIES {
        return Err(ProviderError::Service("Choose a lyrics folder with fewer than 30,000 files".into()));
    }
    let title_key = LyricQuery::normalize(&query.title);
    let mut files: Vec<(String, PathBuf)> = entries
        .into_iter()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let extension = path.extension()?.to_string_lossy().to_lowercase();
            let stem = path.file_stem()?.to_string_lossy().into_owned();
            let wanted = extension == "lrc"
                && !name.ends_with(".tr.lrc")
                && !name.ends_with(".roma.lrc")
                && LyricQuery::normalize(&stem).contains(&title_key);
            wanted.then_some((name, path))
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut best: Option<Candidate> = None;
    for (_, file) in files.into_iter().take(MAX_CANDIDATE_FILES) {
        let Some(text) = read_text(&file) else { continue };
        let (Some(title), Some(artist)) = (lrc_tag("ti", &text), lrc_tag("ar", &text)) else { continue };
        let title_match = LyricQuery::similarity(&query.title, &title);
        let artist_match = LyricQuery::similarity(&query.artist, &artist);
        if title_match < 0.92 || artist_match < 0.85 {
            continue;
        }
        let album = lrc_tag("al", &text);
        if let (Some(expected), Some(found)) = (query.album.as_deref(), album.as_deref()) {
            if !expected.is_empty() && !found.is_empty() && LyricQuery::similarity(expected, found) < 0.80 {
                continue;
            }
        }
        let base = file.with_extension("");
        let mut lines = parse_lrc(&text, query.duration);
        if let Some(ttml) = read_text(&with_extension(&base, "ttml")) {
            let native = parse_ttml(&ttml, query.duration);
            if !native.is_empty() && native.iter().any(|l| l.word_timed) {
                lines = native;
            }
        } else if let Some(yrc) = read_text(&with_extension(&base, "yrc")) {
            let native = parse_yrc(&yrc);
            if !native.is_empty() {
                lines = native;
            }
        }
        let Some(last) = lines.last() else { continue };
        if query.duration > 0.0 && last.start > query.duration + 8.0 {
            continue;
        }
        if let Some(saved) = read_text(&with_extension(&base, "roma.lrc")) {
            lines = attach_readings(&saved, &lines, query.duration);
        }
        let native = lines.iter().any(|l| l.word_timed);
        let album_match = album
            .as_deref()
            .is_some_and(|a| LyricQuery::normalize(a) == LyricQuery::normalize(query.album.as_deref().unwrap_or("")));
        let score = match_score(title_match, artist_match, 0.0, false, native, true, album_match) + 32.0;
        let provider = lrc_tag("source", &text).map(|s| format!(" · {s}")).unwrap_or_default();
        let candidate = Candidate {
            source: SourceId::LocalFiles,
            result: LyricResult::Synced {
                provenance: Provenance {
                    precision: precision(&lines).to_string(),
                    source: format!("{SOURCE_LABEL}{provider}"),
                },
                lines,
            },
            title,
            artist,
            duration: None,
            score,
        };
        let better = match &best {
            None => true,
            Some(current) => candidate.score > current.score,
        };
        if better {
            best = Some(candidate);
        }
    }
    Ok(best)
}

/// The local-files source for one folder.
#[derive(Debug, Clone, Default)]
pub struct LocalFiles {
    pub folder: String,
}

#[async_trait]
impl Provider for LocalFiles {
    fn id(&self) -> SourceId {
        SourceId::LocalFiles
    }

    async fn fetch(&self, query: &LyricQuery, _http: &dyn HttpClient) -> Result<Option<Candidate>, ProviderError> {
        let query = query.clone();
        let folder = self.folder.clone();
        tokio::task::spawn_blocking(move || find_candidate(&query, &folder))
            .await
            .map_err(|_| ProviderError::Service("Reading local lyrics failed".into()))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timing::WORD_BY_WORD;

    fn write(dir: &Path, name: &str, text: &str) {
        std::fs::write(dir.join(name), text).unwrap();
    }

    fn folder_with_sample() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "Singer - Sample - Album.lrc",
            "[ti:Sample]\n[ar:Singer]\n[al:Album]\n[00:01]Keep moving\n[00:03]Hold on",
        );
        dir
    }

    fn folder(dir: &tempfile::TempDir) -> String {
        dir.path().to_string_lossy().into_owned()
    }

    // ReadingAndTTMLTests.testLocalFilesRequireMatchingArtistAndAlbum
    #[test]
    fn local_files_require_matching_artist_and_album() {
        let dir = folder_with_sample();
        let found = find_candidate(&LyricQuery::new("Sample", "Singer", Some("Album".into()), 10.0), &folder(&dir));
        let candidate = found.unwrap().unwrap();
        assert_eq!(candidate.source, SourceId::LocalFiles);
        assert_eq!(candidate.result.provenance().source, SOURCE_LABEL);
        assert_eq!(candidate.duration, None);
        assert!(find_candidate(&LyricQuery::new("Sample", "Someone else", Some("Album".into()), 10.0), &folder(&dir))
            .unwrap()
            .is_none());
        assert!(find_candidate(&LyricQuery::new("Sample", "Singer", Some("Live".into()), 10.0), &folder(&dir))
            .unwrap()
            .is_none());
        // No album on the query: any album is fine.
        assert!(find_candidate(&LyricQuery::new("Sample", "Singer", None, 10.0), &folder(&dir)).unwrap().is_some());
    }

    #[test]
    fn ttml_word_timing_replaces_lrc_and_readings_attach() {
        let dir = folder_with_sample();
        write(
            dir.path(),
            "Singer - Sample - Album.ttml",
            "<tt><body><p begin='1s' end='3s'><span begin='1s' end='2s'>Keep </span><span begin='2s' end='3s'>moving</span></p></body></tt>",
        );
        write(dir.path(), "Singer - Sample - Album.roma.lrc", "[00:01]keep reading");
        write(dir.path(), "Singer - Sample - Album.tr.lrc", "[ti:Sample]\n[ar:Singer]\n[00:01]translation");
        let candidate =
            find_candidate(&LyricQuery::new("Sample", "Singer", None, 10.0), &folder(&dir)).unwrap().unwrap();
        let LyricResult::Synced { lines, provenance } = &candidate.result else { panic!() };
        assert_eq!(provenance.precision, WORD_BY_WORD);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].romanization.as_deref(), Some("keep reading"));
        // native word timing adds to the score: 48 + 27 + 20 + 32
        assert!((candidate.score - 127.0).abs() < 1e-9);
    }

    #[test]
    fn yrc_is_used_when_there_is_no_ttml() {
        let dir = folder_with_sample();
        write(dir.path(), "Singer - Sample - Album.yrc", "[1000,2000](1000,500,0)Keep (1500,500,0)moving");
        let candidate =
            find_candidate(&LyricQuery::new("Sample", "Singer", None, 10.0), &folder(&dir)).unwrap().unwrap();
        assert_eq!(candidate.result.provenance().precision, WORD_BY_WORD);
    }

    #[test]
    fn errors_tags_and_limits() {
        assert_eq!(
            find_candidate(&LyricQuery::new("a", "b", None, 0.0), "").unwrap_err().to_string(),
            "Choose the local lyrics folder below"
        );
        assert!(find_candidate(&LyricQuery::new("a", "b", None, 0.0), "/definitely/not/here").is_err());
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "Untagged Sample.lrc", "[00:01]x");
        write(dir.path(), ".Hidden Sample.lrc", "[ti:Sample]\n[ar:Singer]\n[00:01]x");
        write(dir.path(), "Late Sample.lrc", "[ti:Sample]\n[ar:Singer]\n[source:Somewhere]\n[05:00]late");
        let query = LyricQuery::new("Sample", "Singer", None, 100.0);
        assert!(find_candidate(&query, &folder(&dir)).unwrap().is_none(), "untagged, hidden and too-late files");
        let unknown_length = LyricQuery::new("Sample", "Singer", None, 0.0);
        let found = find_candidate(&unknown_length, &folder(&dir)).unwrap().unwrap();
        assert_eq!(found.result.provenance().source, "Local lyric files · Somewhere");
        assert_eq!(lrc_tag("TI", "  [ti: Spaced ]\n"), Some("Spaced".into()));
        assert_eq!(lrc_tag("ti", "[ti:]\n"), None);
        assert_eq!(lrc_tag("ar", "[ti:x]"), None);
    }

    #[test]
    fn decodes_utf16_and_bom() {
        let text = "[ti:Sample]";
        let mut le = vec![0xFF, 0xFE];
        le.extend(text.encode_utf16().flat_map(|u| u.to_le_bytes()));
        assert_eq!(decode_text(&le).as_deref(), Some(text));
        let mut be = vec![0xFE, 0xFF];
        be.extend(text.encode_utf16().flat_map(|u| u.to_be_bytes()));
        assert_eq!(decode_text(&be).as_deref(), Some(text));
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend(text.as_bytes());
        assert_eq!(decode_text(&bom).as_deref(), Some(text));
    }

    #[tokio::test]
    async fn provider_runs_off_the_async_thread() {
        let dir = folder_with_sample();
        let provider = LocalFiles { folder: folder(&dir) };
        let http = crate::http::FixtureHttp::new();
        let found = provider.fetch(&LyricQuery::new("Sample", "Singer", None, 10.0), &http).await.unwrap();
        assert!(found.is_some());
        assert!(http.requests().is_empty());
    }
}
