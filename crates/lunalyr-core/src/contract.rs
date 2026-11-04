//! JSON shapes exchanged with the TypeScript renderer and settings UI.
//!
//! This file, `docs/CONTRACT.md` and `ui/src/contract.ts` describe the same
//! thing; change them together. Field names are camelCase on the wire.
//! Times on the track timeline are seconds; wall-clock instants are Unix
//! epoch milliseconds (the renderer compares them with `Date.now()`).

use serde::{Deserialize, Serialize};

use crate::model::{Line, Provenance, SourceId, Word};
use crate::settings::Settings;

/// Event names (Tauri `emit`). Payload types are noted per event.
pub mod events {
    /// [`super::Snapshot`] — full state; sent after a wallpaper reloads.
    pub const SNAPSHOT: &str = "lunalyr://snapshot";
    /// `Option<Track>` plus palette: [`super::TrackPayload`].
    pub const TRACK: &str = "lunalyr://track";
    /// [`super::LyricsView`].
    pub const LYRICS: &str = "lunalyr://lyrics";
    /// [`super::PlaybackClock`].
    pub const CLOCK: &str = "lunalyr://clock";
    /// [`crate::settings::Settings`].
    pub const SETTINGS: &str = "lunalyr://settings";
    /// [`super::RenderState`] — sent to one wallpaper window (emit_to).
    pub const RENDER: &str = "lunalyr://render";
    /// [`super::LookupInfo`] — candidates for the settings window.
    pub const LOOKUP: &str = "lunalyr://lookup";
    /// [`super::AudioFrame`] — only while a visualiser is active.
    pub const AUDIO: &str = "lunalyr://audio";
}

/// The song that is playing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    /// Seconds; 0 when the player does not report it.
    pub duration: f64,
    /// Player that owns the media session, e.g. "Spotify.exe" (GSMTC
    /// `SourceAppUserModelId`).
    pub source_app: String,
    /// `LyricQuery::recording_key()` of this track.
    pub recording_key: String,
    /// Stable per-track seed (FNV-1a of title + artist, mod 100000), the
    /// macOS `trackSeed`.
    pub seed: u32,
}

impl Track {
    pub fn seed_for(title: &str, artist: &str) -> u32 {
        let mut value: u64 = 1469598103934665603;
        for byte in title.bytes().chain(artist.bytes()) {
            value = (value ^ byte as u64).wrapping_mul(1099511628211);
        }
        (value % 100_000) as u32
    }
}

/// Playback position anchor. The renderer extrapolates:
/// `t = position + (playing ? (Date.now() - sampledAt) / 1000 * rate : 0) + offset`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackClock {
    /// Seconds into the track at `sampled_at`.
    pub position: f64,
    /// Unix epoch milliseconds when `position` was true.
    pub sampled_at: f64,
    pub playing: bool,
    pub rate: f64,
    /// Seconds added to the extrapolated time: global offset + per-song
    /// offset + platform sync lead. Positive shows lyrics earlier.
    pub offset: f64,
}

impl Default for PlaybackClock {
    fn default() -> Self {
        PlaybackClock { position: 0.0, sampled_at: 0.0, playing: false, rate: 1.0, offset: 0.0 }
    }
}

/// 3x3 grid of album colours as [hue, saturation, brightness] in 0..=1,
/// row-major from the top-left (macOS `ColorExtractor.extractPalette`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Palette {
    pub hsb: Vec<[f64; 3]>,
    pub from_artwork: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackPayload {
    pub track: Option<Track>,
    pub palette: Palette,
    /// Unix epoch ms when playback stopped (drives the "Nothing playing"
    /// notice fade); None while something plays.
    pub idle_since: Option<f64>,
}

/// What the lyric layer should show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum LyricsView {
    /// Nothing playing.
    Idle,
    Loading,
    #[serde(rename_all = "camelCase")]
    Synced {
        lines: Vec<Line>,
        provenance: Provenance,
        /// Per-line reading words (same count as the line's words), or null.
        /// Empty when readings are off.
        #[serde(default)]
        readings: Vec<Option<Vec<Word>>>,
    },
    Plain {
        lines: Vec<String>,
        provenance: Provenance,
    },
    #[serde(rename_all = "camelCase")]
    NotFound {
        /// Unix epoch ms.
        reported_at: f64,
    },
}

/// Per-window render control.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RenderState {
    /// Stop the frame loop (covered by a full-screen/maximised window, or
    /// the user paused the wallpaper).
    pub paused: bool,
    /// Why, for diagnostics: "covered", "user", "session-locked", or "".
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateSummary {
    pub source: SourceId,
    pub source_title: String,
    pub title: String,
    pub artist: String,
    pub duration: Option<f64>,
    pub precision: String,
    pub line_count: usize,
    pub has_readings: bool,
    pub chosen: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LookupInfo {
    pub candidates: Vec<CandidateSummary>,
    /// Source id -> human-readable state ("Match found", "Timed out", ...).
    pub states: Vec<(SourceId, String)>,
    pub pinned: bool,
    pub cached: bool,
}

/// Audio analysis for visualisers (Milestone 2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AudioFrame {
    pub level: f32,
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    pub beat: f32,
    pub flux: f32,
    pub centroid: f32,
    /// Log-spaced magnitude bands, 0..=1.
    pub bands: Vec<f32>,
    /// Recent mono PCM window, base64 of little-endian f32 samples.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pcm: Option<String>,
    pub sample_rate: u32,
}

/// Everything a freshly loaded window needs (`get_snapshot` command).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub track: TrackPayload,
    pub lyrics: LyricsView,
    pub clock: PlaybackClock,
    pub settings: Settings,
    pub render: RenderState,
    pub lookup: LookupInfo,
    /// App version, for the settings window.
    pub version: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lyrics_view_is_tagged_by_status() {
        let json = serde_json::to_value(LyricsView::NotFound { reported_at: 5.0 }).unwrap();
        assert_eq!(json, serde_json::json!({"status": "notFound", "reportedAt": 5.0}));
        let json = serde_json::to_value(LyricsView::Idle).unwrap();
        assert_eq!(json, serde_json::json!({"status": "idle"}));
    }

    #[test]
    fn clock_uses_camel_case() {
        let json = serde_json::to_value(PlaybackClock::default()).unwrap();
        assert!(json.get("sampledAt").is_some());
    }

    #[test]
    fn seed_matches_macos_fnv() {
        // FNV-1a 64 over UTF-8 bytes of title + artist, then % 100000.
        assert_eq!(Track::seed_for("", ""), (1469598103934665603u64 % 100_000) as u32);
        assert_eq!(Track::seed_for("a", "b"), Track::seed_for("ab", ""));
    }
}
