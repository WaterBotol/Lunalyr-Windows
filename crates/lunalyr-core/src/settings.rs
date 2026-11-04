//! Persisted user settings (`%APPDATA%\Lunalyr\settings.json` on Windows).
//!
//! Defaults follow the macOS app (Sources/Settings/Settings.swift and
//! LyricSourceOptions) except where Windows calls for a different default,
//! which is noted inline. Unknown fields are ignored and missing fields take
//! their default, so older and newer files both load. Every value is clamped
//! by [`Settings::sanitized`] before use.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::model::{LyricQuery, SourceId};

/// How a lyric line is composed (macOS `LyricComposition`, plus automatic).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Composition {
    Automatic,
    Scattered,
    Constellation,
    ConstellationEditorial,
    Echo,
    Editorial,
    Spoken,
    Cascade,
    Cutout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FontChoice {
    /// Bundled Manrope (SIL OFL 1.1) — the macOS default face.
    Manrope,
    /// Segoe UI Variable / Segoe UI.
    System,
    Serif,
    Slab,
    Soft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackgroundMode {
    /// Animated album-colour gradient.
    Colour,
    /// Audio-reactive visualiser (WASAPI loopback + bundled web engine).
    Visualizer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VisualizerStyle {
    Wavefield,
    Spectrum,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NoLyricsBehaviour {
    Notice,
    TrackName,
    Nothing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReadingDisplay {
    Off,
    Below,
    Replace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DisplaySelection {
    All,
    Primary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MatchMode {
    /// "Accuracy first": best timing tier wins, then confidence, then order.
    Smart,
    /// "Source order".
    Priority,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SourceSettings {
    pub order: Vec<SourceId>,
    pub enabled: Vec<SourceId>,
    pub mode: MatchMode,
    pub prefer_word_timing: bool,
    pub prefer_word_timed_lrclib: bool,
    pub repair_qq_gaps: bool,
    /// Folder of exported LRC / TTML / YRC files (LocalFiles source).
    pub local_folder: String,
}

impl Default for SourceSettings {
    fn default() -> Self {
        SourceSettings {
            order: SourceId::RECOMMENDED_ORDER.to_vec(),
            enabled: SourceId::ALL.iter().copied().filter(|s| *s != SourceId::LocalFiles).collect(),
            mode: MatchMode::Smart,
            prefer_word_timing: true,
            prefer_word_timed_lrclib: true,
            repair_qq_gaps: true,
            local_folder: String::new(),
        }
    }
}

impl SourceSettings {
    /// Enabled sources in the user's order (macOS `Snapshot.enabled`).
    pub fn enabled_in_order(&self) -> Vec<SourceId> {
        self.order.iter().copied().filter(|s| self.enabled.contains(s)).collect()
    }

    /// Changes whenever a setting that affects automatic selection changes,
    /// so automatic caches are not reused across policies.
    pub fn fingerprint(&self) -> String {
        let enabled: Vec<&str> = self.enabled_in_order().iter().map(|s| s.as_str()).collect();
        let local = if self.enabled.contains(&SourceId::LocalFiles) { self.local_folder.as_str() } else { "" };
        format!(
            "w1|{}|{}|{}|{}|{}",
            self.prefer_word_timed_lrclib,
            self.prefer_word_timing,
            match self.mode {
                MatchMode::Smart => "smart",
                MatchMode::Priority => "priority",
            },
            enabled.join(","),
            local
        )
    }

    fn sanitize(&mut self) {
        let mut order: Vec<SourceId> = Vec::new();
        for id in self.order.iter().chain(SourceId::RECOMMENDED_ORDER.iter()) {
            if !order.contains(id) {
                order.push(*id);
            }
        }
        self.order = order;
        let mut enabled: Vec<SourceId> = Vec::new();
        for id in &self.enabled {
            if !enabled.contains(id) {
                enabled.push(*id);
            }
        }
        self.enabled = enabled;
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// macOS defaults to Constellation + Editorial; the Windows MVP ships
    /// Editorial and Spoken first, so Editorial is the default here.
    pub composition: Composition,
    /// Applied after fitting, 0.5..=2.
    pub lyric_scale: f64,
    /// 15, 30 or 60 (macOS "Balanced" = 30).
    pub frame_rate: u32,
    /// Windows default is on (macOS: off) — full-screen games and video are common.
    pub pause_when_covered: bool,
    /// Global lyric offset in seconds, -30..=30.
    pub lyric_offset: f64,
    pub next_word_preview: bool,
    /// Seconds, 0.1..=2.
    pub next_word_preview_lead: f64,
    pub word_reveal: bool,
    pub show_interludes: bool,
    pub no_lyrics: NoLyricsBehaviour,
    pub scrim_strength: f64,
    pub vividness: f64,
    pub effect_speed: f64,
    pub font: FontChoice,
    pub background: BackgroundMode,
    pub visualizer: VisualizerStyle,
    pub audio_reactive: bool,
    pub audio_sensitivity: f64,
    pub readings: ReadingDisplay,
    pub tone_marks: bool,
    pub displays: DisplaySelection,
    pub launch_at_login: bool,
    pub reduce_motion: bool,
    pub sources: SourceSettings,
    /// sha256(recording key) hex -> seconds.
    pub song_offsets: BTreeMap<String, f64>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            composition: Composition::Editorial,
            lyric_scale: 1.0,
            frame_rate: 30,
            pause_when_covered: true,
            lyric_offset: 0.0,
            next_word_preview: true,
            next_word_preview_lead: 0.45,
            word_reveal: true,
            show_interludes: true,
            no_lyrics: NoLyricsBehaviour::Notice,
            scrim_strength: 1.0,
            vividness: 1.0,
            effect_speed: 1.0,
            font: FontChoice::Manrope,
            background: BackgroundMode::Colour,
            visualizer: VisualizerStyle::Wavefield,
            // Windows needs no capture permission; capture still only runs
            // while the visualiser background is selected.
            audio_reactive: true,
            audio_sensitivity: 1.0,
            readings: ReadingDisplay::Off,
            tone_marks: true,
            displays: DisplaySelection::All,
            launch_at_login: false,
            reduce_motion: false,
            sources: SourceSettings::default(),
            song_offsets: BTreeMap::new(),
        }
    }
}

fn clamp(value: f64, low: f64, high: f64, fallback: f64) -> f64 {
    if value.is_finite() {
        value.clamp(low, high)
    } else {
        fallback
    }
}

impl Settings {
    /// Returns a copy with every value inside its valid range.
    pub fn sanitized(mut self) -> Self {
        let d = Settings::default();
        self.lyric_scale = clamp(self.lyric_scale, 0.5, 2.0, d.lyric_scale);
        self.frame_rate = match self.frame_rate {
            0..=22 => 15,
            23..=45 => 30,
            _ => 60,
        };
        self.lyric_offset = clamp(self.lyric_offset, -30.0, 30.0, 0.0);
        self.next_word_preview_lead = clamp(self.next_word_preview_lead, 0.1, 2.0, d.next_word_preview_lead);
        self.scrim_strength = clamp(self.scrim_strength, 0.0, 1.5, d.scrim_strength);
        self.vividness = clamp(self.vividness, 0.0, 1.5, d.vividness);
        self.effect_speed = clamp(self.effect_speed, 0.25, 2.0, d.effect_speed);
        self.audio_sensitivity = clamp(self.audio_sensitivity, 0.1, 3.0, d.audio_sensitivity);
        self.song_offsets.retain(|key, value| key.len() == 64 && value.is_finite() && *value != 0.0);
        for value in self.song_offsets.values_mut() {
            *value = value.clamp(-30.0, 30.0);
        }
        self.sources.sanitize();
        self
    }

    /// Loads settings; a missing file gives defaults, and an unreadable or
    /// corrupt file is moved aside to `settings.json.bak` and defaults used.
    pub fn load(path: &Path) -> Settings {
        match std::fs::read(path) {
            Ok(bytes) => match serde_json::from_slice::<Settings>(&bytes) {
                Ok(settings) => settings.sanitized(),
                Err(error) => {
                    log::warn!("settings at {} are unreadable ({error}); using defaults", path.display());
                    let _ = std::fs::rename(path, path.with_extension("json.bak"));
                    Settings::default()
                }
            },
            Err(_) => Settings::default(),
        }
    }

    /// Atomic save: write a temporary file next to the target, then rename.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, json)?;
        std::fs::rename(&temporary, path)
    }

    fn offset_key(query: &LyricQuery) -> String {
        let digest = Sha256::digest(query.recording_key().as_bytes());
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Per-recording offset in seconds (macOS `SongOffsets`).
    pub fn song_offset(&self, query: &LyricQuery) -> f64 {
        self.song_offsets
            .get(&Self::offset_key(query))
            .copied()
            .filter(|v| v.is_finite())
            .unwrap_or(0.0)
            .clamp(-30.0, 30.0)
    }

    pub fn set_song_offset(&mut self, query: &LyricQuery, seconds: f64) {
        if !seconds.is_finite() {
            return;
        }
        let key = Self::offset_key(query);
        let bounded = seconds.clamp(-30.0, 30.0);
        if bounded == 0.0 {
            self.song_offsets.remove(&key);
        } else {
            self.song_offsets.insert(key, bounded);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip_through_json() {
        let settings = Settings::default();
        let json = serde_json::to_string(&settings).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(settings, back);
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["composition"], "editorial");
        assert_eq!(value["frameRate"], 30);
        assert_eq!(value["sources"]["enabled"][0], "lyricsPlus");
    }

    #[test]
    fn missing_and_unknown_fields_are_tolerated() {
        let settings: Settings = serde_json::from_str(r#"{"lyricScale": 1.5, "somethingNew": true}"#).unwrap();
        assert_eq!(settings.lyric_scale, 1.5);
        assert_eq!(settings.frame_rate, 30);
    }

    #[test]
    fn sanitize_clamps_and_repairs() {
        let mut settings =
            Settings { lyric_scale: 9.0, frame_rate: 144, lyric_offset: f64::NAN, ..Settings::default() };
        settings.sources.order = vec![SourceId::Lrclib, SourceId::Lrclib];
        settings.sources.enabled = vec![SourceId::NetEase, SourceId::NetEase];
        let settings = settings.sanitized();
        assert_eq!(settings.lyric_scale, 2.0);
        assert_eq!(settings.frame_rate, 60);
        assert_eq!(settings.lyric_offset, 0.0);
        assert_eq!(settings.sources.order.len(), SourceId::ALL.len());
        assert_eq!(settings.sources.order[0], SourceId::Lrclib);
        assert_eq!(settings.sources.enabled, vec![SourceId::NetEase]);
    }

    #[test]
    fn load_save_and_corrupt_file_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(Settings::load(&path), Settings::default());
        let settings = Settings { lyric_scale: 1.25, ..Settings::default() };
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path).lyric_scale, 1.25);
        std::fs::write(&path, b"{ not json").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
        assert!(path.with_extension("json.bak").exists());
    }

    #[test]
    fn song_offsets_are_per_recording_and_bounded() {
        let mut settings = Settings::default();
        let a = LyricQuery::new("Song", "Artist", None, 200.0);
        let b = LyricQuery::new("Other", "Artist", None, 200.0);
        settings.set_song_offset(&a, 45.0);
        assert_eq!(settings.song_offset(&a), 30.0);
        assert_eq!(settings.song_offset(&b), 0.0);
        settings.set_song_offset(&a, 0.0);
        assert!(settings.song_offsets.is_empty());
    }

    #[test]
    fn fingerprint_changes_with_policy() {
        let mut sources = SourceSettings::default();
        let before = sources.fingerprint();
        sources.mode = MatchMode::Priority;
        assert_ne!(before, sources.fingerprint());
    }
}
