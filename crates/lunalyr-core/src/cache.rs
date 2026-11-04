//! Saved lyric results (`SavedLyrics` in Sources/Lyrics/LyricSources.swift).
//!
//! A directory of `cache-<sha256(key)>.json` (automatic choices, expire
//! after 14 days, at most 150 files, oldest evicted by modification time)
//! and `pin-<sha256(key)>.json` (the user's explicit choice, never expires).
//! Files over 2 MB are neither written nor read.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::model::LyricResult;
use crate::text::sha256_hex;

/// Automatic entries older than this are ignored.
pub const MAX_AGE: Duration = Duration::from_secs(14 * 86_400);
/// Automatic entries kept on disk.
pub const MAX_AUTOMATIC_FILES: usize = 150;
/// Largest entry written or read.
pub const MAX_ENTRY_BYTES: u64 = 2_000_000;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    result: LyricResult,
    /// Unix epoch seconds.
    stored_at: f64,
}

#[derive(Debug, Clone)]
pub struct LyricCache {
    dir: PathBuf,
}

fn now_seconds() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

impl LyricCache {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        LyricCache { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// File for a key (`cache-` or `pin-` + SHA-256 hex of the key).
    pub fn path(&self, key: &str, pin: bool) -> PathBuf {
        self.dir.join(format!("{}{}.json", if pin { "pin-" } else { "cache-" }, sha256_hex(key)))
    }

    /// The saved result, if present, readable, within size and (for
    /// automatic entries) younger than 14 days.
    pub fn read(&self, key: &str, pin: bool) -> Option<LyricResult> {
        let path = self.path(key, pin);
        let size = std::fs::metadata(&path).ok()?.len();
        if size > MAX_ENTRY_BYTES {
            return None;
        }
        let bytes = std::fs::read(&path).ok()?;
        if bytes.len() as u64 > MAX_ENTRY_BYTES {
            return None;
        }
        let entry: Entry = serde_json::from_slice(&bytes).ok()?;
        if !pin && now_seconds() - entry.stored_at >= MAX_AGE.as_secs_f64() {
            return None;
        }
        Some(entry.result)
    }

    /// Saves atomically; then bounds the automatic entries. Returns false
    /// when the entry is too large or cannot be written.
    pub fn save(&self, result: &LyricResult, key: &str, pin: bool) -> bool {
        self.try_save(result, key, pin).is_ok()
    }

    fn try_save(&self, result: &LyricResult, key: &str, pin: bool) -> io::Result<()> {
        let data = serde_json::to_vec(&Entry { result: result.clone(), stored_at: now_seconds() })
            .map_err(io::Error::other)?;
        if data.len() as u64 > MAX_ENTRY_BYTES {
            return Err(io::Error::other("entry too large"));
        }
        std::fs::create_dir_all(&self.dir)?;
        let target = self.path(key, pin);
        write_atomic(&target, &data)?;
        self.evict(&target);
        Ok(())
    }

    /// Keeps at most [`MAX_AUTOMATIC_FILES`] automatic entries, removing the
    /// oldest by modification time (the file just written sorts last on ties).
    fn evict(&self, just_written: &Path) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return };
        let mut automatic: Vec<(SystemTime, bool, PathBuf)> = entries
            .filter_map(Result::ok)
            .filter(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                name.starts_with("cache-") && name.ends_with(".json")
            })
            .map(|e| {
                let modified = e.metadata().and_then(|m| m.modified()).unwrap_or(UNIX_EPOCH);
                let path = e.path();
                (modified, path == just_written, path)
            })
            .collect();
        if automatic.len() <= MAX_AUTOMATIC_FILES {
            return;
        }
        automatic.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        let excess = automatic.len() - MAX_AUTOMATIC_FILES;
        for (_, _, path) in automatic.into_iter().take(excess) {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Removes the user's saved choice for a key.
    pub fn unpin(&self, key: &str) {
        let _ = std::fs::remove_file(self.path(key, true));
    }
}

fn write_atomic(target: &Path, data: &[u8]) -> io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let temporary = target.with_file_name(format!(".{name}.{}-{unique}.tmp", std::process::id()));
    std::fs::write(&temporary, data)?;
    std::fs::rename(&temporary, target).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Provenance;

    fn result(text: &str) -> LyricResult {
        LyricResult::Plain {
            lines: vec![text.into()],
            provenance: Provenance { precision: "Not synced".into(), source: "Test".into() },
        }
    }

    fn backdate(path: &Path, seconds: f64) {
        let bytes = std::fs::read(path).unwrap();
        let mut entry: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        entry["storedAt"] = serde_json::json!(now_seconds() - seconds);
        std::fs::write(path, serde_json::to_vec(&entry).unwrap()).unwrap();
    }

    #[test]
    fn save_read_and_file_names() {
        let dir = tempfile::tempdir().unwrap();
        let cache = LyricCache::new(dir.path().join("Lyrics"));
        assert!(cache.read("k", false).is_none());
        assert!(cache.save(&result("a"), "k", false));
        assert_eq!(cache.read("k", false), Some(result("a")));
        assert!(cache.read("k", true).is_none(), "pins are separate");
        let name = cache.path("k", false).file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(name, format!("cache-{}.json", sha256_hex("k")));
        assert!(cache.path("k", true).file_name().unwrap().to_string_lossy().starts_with("pin-"));
    }

    #[test]
    fn automatic_entries_expire_but_pins_do_not() {
        let dir = tempfile::tempdir().unwrap();
        let cache = LyricCache::new(dir.path());
        cache.save(&result("auto"), "k", false);
        cache.save(&result("pinned"), "k", true);
        backdate(&cache.path("k", false), 13.0 * 86_400.0);
        assert!(cache.read("k", false).is_some());
        backdate(&cache.path("k", false), 14.0 * 86_400.0 + 1.0);
        assert!(cache.read("k", false).is_none());
        backdate(&cache.path("k", true), 400.0 * 86_400.0);
        assert_eq!(cache.read("k", true), Some(result("pinned")));
        cache.unpin("k");
        assert!(cache.read("k", true).is_none());
        cache.unpin("k"); // idempotent
    }

    #[test]
    fn oversized_and_corrupt_entries_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let cache = LyricCache::new(dir.path());
        let huge = result(&"x".repeat(2_000_001));
        assert!(!cache.save(&huge, "big", false));
        assert!(!cache.path("big", false).exists());
        std::fs::write(cache.path("bad", false), b"{ nope").unwrap();
        assert!(cache.read("bad", false).is_none());
        std::fs::write(cache.path("big2", true), vec![b' '; 2_000_001]).unwrap();
        assert!(cache.read("big2", true).is_none());
    }

    #[test]
    fn automatic_files_are_capped_oldest_first_and_pins_kept() {
        let dir = tempfile::tempdir().unwrap();
        let cache = LyricCache::new(dir.path());
        cache.save(&result("pin"), "pinned", true);
        let old = SystemTime::now() - Duration::from_secs(3600);
        for i in 0..MAX_AUTOMATIC_FILES {
            let key = format!("k{i}");
            cache.save(&result(&key), &key, false);
            // Spread modification times: k0 oldest.
            let file = std::fs::File::options().write(true).open(cache.path(&key, false)).unwrap();
            file.set_modified(old + Duration::from_secs(i as u64)).unwrap();
        }
        cache.save(&result("new"), "new", false);
        let count = std::fs::read_dir(dir.path())
            .unwrap()
            .filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with("cache-"))
            .count();
        assert_eq!(count, MAX_AUTOMATIC_FILES);
        assert!(cache.read("k0", false).is_none(), "oldest evicted");
        assert!(cache.read("k1", false).is_some());
        assert!(cache.read("new", false).is_some());
        assert!(cache.read("pinned", true).is_some());
        let leftovers = std::fs::read_dir(dir.path())
            .unwrap()
            .filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);
    }
}
