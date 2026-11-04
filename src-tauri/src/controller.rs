//! The app's state machine: platform events, lookups and user actions in;
//! state changes and side effects out. It has no Tauri or OS dependency, so
//! its behaviour is unit-tested on any host; `lib.rs` executes the effects.
//!
//! macOS counterparts: NowPlayingMonitor (track + anchor), LyricsViewModel
//! (lookup lifecycle, saved choices, imports), NowPlayingView (palette
//! changes) and WindowRenderActivity (pausing).

use std::collections::HashMap;

use lunalyr_core::contract::{
    events, CandidateSummary, LookupInfo, LyricsView, Palette, PlaybackClock, RenderState, Snapshot, Track,
    TrackPayload,
};
use lunalyr_core::model::{Candidate, LyricQuery, LyricResult, SourceId, Word};
use lunalyr_core::settings::{BackgroundMode, ReadingDisplay, Settings};
use serde_json::Value;

use crate::platform::{DesktopEvent, MediaEvent, MediaInfo, PlatformEvent, TimelineInfo};

/// Lyrics are shown slightly ahead of the reported position: the audio the
/// listener hears is buffered behind what the player reports (macOS uses
/// 0.16 s for AppleScript latency + buffering; GSMTC timestamps its samples,
/// so less is needed). Users fine-tune with the global and per-song offsets.
pub const SYNC_LEAD: f64 = 0.10;

/// Where an event goes.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    All,
    /// Every `wallpaper-*` window.
    Wallpapers,
    Window(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Emit {
        event: &'static str,
        payload: Value,
        target: Target,
    },
    Lookup {
        generation: u64,
        query: LyricQuery,
        force: bool,
    },
    Pin {
        query: LyricQuery,
        result: LyricResult,
    },
    Unpin {
        query: LyricQuery,
    },
    ExtractPalette {
        generation: u64,
        bytes: Vec<u8>,
    },
    ComputeReadings {
        generation: u64,
        lines: Vec<lunalyr_core::model::Line>,
    },
    SaveSettings,
    /// Match wallpaper windows to the monitors; `recreate` replaces every
    /// window (after an Explorer restart the attached ones are destroyed).
    RebuildWallpapers {
        recreate: bool,
    },
    SetLaunchAtLogin(bool),
    SetAudio(bool),
    Tray {
        now_playing: String,
        paused: bool,
    },
}

/// Result of a lookup, as produced by `lyrics::lookup`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LookupOutcome {
    pub result: Option<LyricResult>,
    pub candidates: Vec<Candidate>,
    pub states: Vec<(SourceId, String)>,
    pub pinned: bool,
    pub cached: bool,
}

/// Pure hooks into lunalyr-core used while handling events.
pub trait LyricOps {
    /// LyricsViewModel.apply processing (QQ gap repair, word-timing validation).
    fn prepare(&self, result: LyricResult, repair_qq_gaps: bool) -> LyricResult;
    fn summary(&self, candidate: &Candidate, chosen: bool) -> CandidateSummary;
}

pub struct Controller {
    settings: Settings,
    version: String,
    media: Option<MediaInfo>,
    query: Option<LyricQuery>,
    track: Option<Track>,
    palette: Palette,
    idle_since: Option<f64>,
    /// The chosen result before `prepare` (kept to re-prepare on setting changes).
    raw: Option<LyricResult>,
    lyrics: LyricsView,
    candidates: Vec<Candidate>,
    lookup: LookupInfo,
    timeline: Option<TimelineInfo>,
    generation: u64,
    artwork_generation: u64,
    readings_generation: u64,
    user_paused: bool,
    locked: bool,
    coverage: HashMap<String, bool>,
    /// (window label, monitor id)
    windows: Vec<(String, String)>,
    sent_render: HashMap<String, RenderState>,
    audio_on: bool,
}

/// ColorExtractor.fallbackPalette, used before artwork resolves.
pub fn fallback_palette() -> Palette {
    lunalyr_core::palette::fallback_palette()
}

fn to_value<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

fn same_recording(a: &MediaInfo, b: &MediaInfo) -> bool {
    a.title == b.title && a.artist == b.artist && a.album == b.album
}

impl Controller {
    pub fn new(settings: Settings, version: impl Into<String>) -> Self {
        Controller {
            settings: settings.sanitized(),
            version: version.into(),
            media: None,
            query: None,
            track: None,
            palette: fallback_palette(),
            idle_since: None,
            raw: None,
            lyrics: LyricsView::Idle,
            candidates: Vec::new(),
            lookup: LookupInfo::default(),
            timeline: None,
            generation: 0,
            artwork_generation: 0,
            readings_generation: 0,
            user_paused: false,
            locked: false,
            coverage: HashMap::new(),
            windows: Vec::new(),
            sent_render: HashMap::new(),
            audio_on: false,
        }
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn query(&self) -> Option<&LyricQuery> {
        self.query.as_ref()
    }

    pub fn user_paused(&self) -> bool {
        self.user_paused
    }

    pub fn windows(&self) -> &[(String, String)] {
        &self.windows
    }

    // ---- outgoing state -------------------------------------------------

    pub fn track_payload(&self) -> TrackPayload {
        TrackPayload { track: self.track.clone(), palette: self.palette.clone(), idle_since: self.idle_since }
    }

    pub fn clock(&self, now_ms: f64) -> PlaybackClock {
        let song = self.query.as_ref().map(|q| self.settings.song_offset(q)).unwrap_or(0.0);
        let offset = self.settings.lyric_offset + song + SYNC_LEAD;
        match self.timeline {
            Some(t) => PlaybackClock {
                position: t.position,
                sampled_at: t.sampled_at_ms,
                playing: t.playing,
                rate: if t.rate.is_finite() && t.rate > 0.0 { t.rate } else { 1.0 },
                offset,
            },
            None => PlaybackClock { position: 0.0, sampled_at: now_ms, playing: false, rate: 1.0, offset },
        }
    }

    pub fn render_state(&self, label: &str) -> RenderState {
        let monitor = self.windows.iter().find(|(l, _)| l == label).map(|(_, m)| m.as_str());
        let covered = monitor.and_then(|m| self.coverage.get(m)).copied().unwrap_or(false);
        let (paused, reason) = if self.user_paused {
            (true, "user")
        } else if self.locked {
            (true, "session-locked")
        } else if covered && self.settings.pause_when_covered {
            (true, "covered")
        } else {
            (false, "")
        };
        RenderState { paused, reason: reason.into() }
    }

    pub fn snapshot(&self, label: Option<&str>, now_ms: f64) -> Snapshot {
        Snapshot {
            track: self.track_payload(),
            lyrics: self.lyrics.clone(),
            clock: self.clock(now_ms),
            settings: self.settings.clone(),
            render: label.map(|l| self.render_state(l)).unwrap_or_default(),
            lookup: self.lookup.clone(),
            version: self.version.clone(),
        }
    }

    fn emit<T: serde::Serialize>(event: &'static str, payload: &T, target: Target) -> Effect {
        Effect::Emit { event, payload: to_value(payload), target }
    }

    fn emit_track(&self) -> Effect {
        Self::emit(events::TRACK, &self.track_payload(), Target::All)
    }

    fn emit_lyrics(&self) -> Effect {
        Self::emit(events::LYRICS, &self.lyrics, Target::All)
    }

    fn emit_clock(&self, now_ms: f64) -> Effect {
        Self::emit(events::CLOCK, &self.clock(now_ms), Target::All)
    }

    fn emit_lookup(&self) -> Effect {
        Self::emit(events::LOOKUP, &self.lookup, Target::All)
    }

    fn emit_settings(&self) -> Effect {
        Self::emit(events::SETTINGS, &self.settings, Target::All)
    }

    fn tray(&self) -> Effect {
        let now_playing = match &self.track {
            Some(t) if t.artist.is_empty() => t.title.clone(),
            Some(t) => format!("{} — {}", t.title, t.artist),
            None => "Nothing playing".into(),
        };
        Effect::Tray { now_playing, paused: self.user_paused }
    }

    /// Emits `RenderState` to every wallpaper whose state changed.
    fn render_effects(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        for (label, _) in self.windows.clone() {
            let state = self.render_state(&label);
            if self.sent_render.get(&label) != Some(&state) {
                self.sent_render.insert(label.clone(), state.clone());
                effects.push(Self::emit(events::RENDER, &state, Target::Window(label)));
            }
        }
        effects
    }

    fn wants_audio(&self) -> bool {
        self.settings.background == BackgroundMode::Visualizer || self.settings.audio_reactive
    }

    fn audio_effects(&mut self) -> Vec<Effect> {
        let wanted = self.wants_audio() && !self.user_paused && !self.locked;
        if wanted != self.audio_on {
            self.audio_on = wanted;
            vec![Effect::SetAudio(wanted)]
        } else {
            Vec::new()
        }
    }

    // ---- platform events --------------------------------------------------

    pub fn on_platform(&mut self, event: PlatformEvent, now_ms: f64) -> Vec<Effect> {
        match event {
            PlatformEvent::Media(MediaEvent::Track(info)) => self.on_track(info, now_ms),
            PlatformEvent::Media(MediaEvent::Timeline(timeline)) => {
                self.timeline = Some(timeline);
                vec![self.emit_clock(now_ms)]
            }
            PlatformEvent::Media(MediaEvent::Artwork { title, artist, bytes }) => {
                let current = self.media.as_ref().map(|m| m.title == title && m.artist == artist).unwrap_or(false);
                if !current {
                    return Vec::new();
                }
                self.artwork_generation += 1;
                match bytes {
                    Some(bytes) if !bytes.is_empty() => {
                        vec![Effect::ExtractPalette { generation: self.artwork_generation, bytes }]
                    }
                    _ => {
                        self.palette = fallback_palette();
                        vec![self.emit_track()]
                    }
                }
            }
            PlatformEvent::Desktop(DesktopEvent::Coverage(list)) => {
                for (monitor, covered) in list {
                    self.coverage.insert(monitor, covered);
                }
                self.render_effects()
            }
            PlatformEvent::Desktop(DesktopEvent::SessionLocked(locked)) => {
                self.locked = locked;
                let mut effects = self.render_effects();
                effects.extend(self.audio_effects());
                effects
            }
            PlatformEvent::Desktop(DesktopEvent::DisplaysChanged) => {
                vec![Effect::RebuildWallpapers { recreate: false }]
            }
            PlatformEvent::Desktop(DesktopEvent::ShellRestarted) => vec![Effect::RebuildWallpapers { recreate: true }],
            PlatformEvent::Audio(frame) => vec![Self::emit(events::AUDIO, &frame, Target::Wallpapers)],
            PlatformEvent::AudioStatus(status) => {
                log::info!("audio: {status}");
                vec![Self::emit("lunalyr://audio-status", &status, Target::All)]
            }
        }
    }

    fn on_track(&mut self, info: Option<MediaInfo>, now_ms: f64) -> Vec<Effect> {
        let Some(info) = info.filter(|i| !i.title.trim().is_empty()) else {
            if self.media.is_none() && self.track.is_none() {
                return Vec::new();
            }
            self.media = None;
            self.query = None;
            self.track = None;
            self.raw = None;
            self.candidates.clear();
            self.lookup = LookupInfo::default();
            self.timeline = None;
            self.lyrics = LyricsView::Idle;
            self.idle_since = Some(now_ms);
            self.generation += 1;
            return vec![
                self.emit_track(),
                self.emit_lyrics(),
                self.emit_lookup(),
                self.emit_clock(now_ms),
                self.tray(),
            ];
        };

        let previous = self.media.replace(info.clone());
        let same = previous.as_ref().map(|p| same_recording(p, &info)).unwrap_or(false);
        let query = LyricQuery::new(info.title.clone(), info.artist.clone(), info.album.clone(), info.duration);
        let track = Track {
            title: info.title.clone(),
            artist: info.artist.clone(),
            album: info.album.clone(),
            duration: query.duration,
            source_app: info.source_app.clone(),
            recording_key: query.recording_key(),
            seed: Track::seed_for(&info.title, &info.artist),
        };
        self.idle_since = None;

        if same {
            // A late duration (players often report 0 first) refines the
            // query; retry only if the first lookup found nothing.
            let duration_arrived = previous.map(|p| p.duration <= 0.0 && info.duration > 0.0).unwrap_or(false);
            self.query = Some(query.clone());
            let changed = self.track.as_ref() != Some(&track);
            self.track = Some(track);
            let mut effects = Vec::new();
            if changed {
                effects.push(self.emit_track());
                effects.push(self.emit_clock(now_ms));
            }
            if duration_arrived && matches!(self.lyrics, LyricsView::NotFound { .. }) {
                effects.extend(self.start_lookup(false));
            }
            return effects;
        }

        // A new recording: keep the outgoing colours until the new cover
        // resolves (NowPlayingMonitor.refresh), reset everything else.
        self.query = Some(query);
        self.track = Some(track);
        self.raw = None;
        self.candidates.clear();
        self.lookup = LookupInfo::default();
        self.timeline = None;
        let mut effects = vec![self.emit_track(), self.emit_clock(now_ms), self.tray()];
        effects.extend(self.start_lookup(false));
        effects
    }

    fn start_lookup(&mut self, force: bool) -> Vec<Effect> {
        let Some(query) = self.query.clone() else { return Vec::new() };
        self.generation += 1;
        self.lyrics = LyricsView::Loading;
        vec![self.emit_lyrics(), Effect::Lookup { generation: self.generation, query, force }]
    }

    // ---- async completions ------------------------------------------------

    pub fn on_lookup_finished(
        &mut self,
        generation: u64,
        outcome: LookupOutcome,
        ops: &dyn LyricOps,
        now_ms: f64,
    ) -> Vec<Effect> {
        if generation != self.generation {
            return Vec::new();
        }
        self.candidates = outcome.candidates.clone();
        let chosen_source =
            outcome.result.as_ref().and_then(|r| outcome.candidates.iter().find(|c| &c.result == r).map(|c| c.source));
        self.lookup = LookupInfo {
            candidates: outcome.candidates.iter().map(|c| ops.summary(c, Some(c.source) == chosen_source)).collect(),
            states: outcome.states.clone(),
            pinned: outcome.pinned,
            cached: outcome.cached,
        };
        self.raw = outcome.result;
        let mut effects = self.apply_raw(ops, now_ms);
        effects.push(self.emit_lookup());
        effects
    }

    /// LyricsViewModel.apply: turn the raw result into what is displayed.
    fn apply_raw(&mut self, ops: &dyn LyricOps, now_ms: f64) -> Vec<Effect> {
        self.readings_generation += 1;
        let mut effects = Vec::new();
        self.lyrics = match self.raw.clone() {
            Some(result) => match ops.prepare(result, self.settings.sources.repair_qq_gaps) {
                LyricResult::Synced { lines, provenance } if !lines.is_empty() => {
                    if self.settings.readings != ReadingDisplay::Off {
                        effects.push(Effect::ComputeReadings {
                            generation: self.readings_generation,
                            lines: lines.clone(),
                        });
                    }
                    LyricsView::Synced { lines, provenance, readings: Vec::new() }
                }
                LyricResult::Plain { lines, provenance } if !lines.is_empty() => {
                    LyricsView::Plain { lines, provenance }
                }
                _ => LyricsView::NotFound { reported_at: now_ms },
            },
            None => LyricsView::NotFound { reported_at: now_ms },
        };
        effects.insert(0, self.emit_lyrics());
        effects
    }

    pub fn on_readings(&mut self, generation: u64, readings: Vec<Option<Vec<Word>>>) -> Vec<Effect> {
        if generation != self.readings_generation {
            return Vec::new();
        }
        if let LyricsView::Synced { readings: current, lines, .. } = &mut self.lyrics {
            if readings.len() == lines.len() || readings.is_empty() {
                *current = readings;
                return vec![self.emit_lyrics()];
            }
        }
        Vec::new()
    }

    pub fn on_palette(&mut self, generation: u64, palette: Palette) -> Vec<Effect> {
        if generation != self.artwork_generation {
            return Vec::new();
        }
        self.palette = palette;
        vec![self.emit_track()]
    }

    // ---- user actions -----------------------------------------------------

    /// Shallow-merges `patch` (a JSON object of Settings fields) into the
    /// settings. Returns the new settings and the side effects.
    pub fn update_settings(
        &mut self,
        patch: &Value,
        ops: &dyn LyricOps,
        now_ms: f64,
    ) -> Result<(Settings, Vec<Effect>), String> {
        let Value::Object(fields) = patch else { return Err("settings patch must be an object".into()) };
        let mut merged = to_value(&self.settings);
        if let Value::Object(target) = &mut merged {
            for (key, value) in fields {
                target.insert(key.clone(), value.clone());
            }
        }
        let next: Settings =
            serde_json::from_value::<Settings>(merged).map_err(|e| format!("invalid settings: {e}"))?.sanitized();
        let previous = std::mem::replace(&mut self.settings, next.clone());
        let mut effects = vec![self.emit_settings(), Effect::SaveSettings];
        if previous.lyric_offset != next.lyric_offset || previous.song_offsets != next.song_offsets {
            effects.push(self.emit_clock(now_ms));
        }
        if previous.displays != next.displays {
            effects.push(Effect::RebuildWallpapers { recreate: false });
        }
        if previous.launch_at_login != next.launch_at_login {
            effects.push(Effect::SetLaunchAtLogin(next.launch_at_login));
        }
        if previous.pause_when_covered != next.pause_when_covered {
            effects.extend(self.render_effects());
        }
        effects.extend(self.audio_effects());
        let sources_changed = {
            let mut a = previous.sources.clone();
            let mut b = next.sources.clone();
            a.repair_qq_gaps = false;
            b.repair_qq_gaps = false;
            a != b
        };
        if sources_changed && self.query.is_some() {
            effects.extend(self.start_lookup(false));
        } else if (previous.sources.repair_qq_gaps != next.sources.repair_qq_gaps
            || previous.readings != next.readings
            || previous.tone_marks != next.tone_marks)
            && self.raw.is_some()
        {
            effects.extend(self.apply_raw(ops, now_ms));
        }
        Ok((next, effects))
    }

    pub fn reload_lyrics(&mut self) -> Vec<Effect> {
        self.start_lookup(true)
    }

    pub fn select_candidate(
        &mut self,
        source: SourceId,
        ops: &dyn LyricOps,
        now_ms: f64,
    ) -> Result<Vec<Effect>, String> {
        let query = self.query.clone().ok_or("Nothing is playing.")?;
        let candidate = self
            .candidates
            .iter()
            .find(|c| c.source == source)
            .cloned()
            .ok_or("That source has no match for this song.")?;
        self.generation += 1;
        self.raw = Some(candidate.result.clone());
        self.lookup.pinned = true;
        for summary in &mut self.lookup.candidates {
            summary.chosen = summary.source == source;
        }
        let mut effects = vec![Effect::Pin { query, result: candidate.result }];
        effects.extend(self.apply_raw(ops, now_ms));
        effects.push(self.emit_lookup());
        Ok(effects)
    }

    pub fn forget_selection(&mut self) -> Result<Vec<Effect>, String> {
        let query = self.query.clone().ok_or("Nothing is playing.")?;
        self.lookup.pinned = false;
        let mut effects = vec![Effect::Unpin { query }];
        effects.extend(self.start_lookup(true));
        Ok(effects)
    }

    pub fn set_song_offset(&mut self, seconds: f64, now_ms: f64) -> Result<Vec<Effect>, String> {
        let query = self.query.clone().ok_or("Nothing is playing.")?;
        self.settings.set_song_offset(&query, seconds);
        Ok(vec![self.emit_settings(), Effect::SaveSettings, self.emit_clock(now_ms)])
    }

    /// `parsed` is the imported file already parsed by lunalyr-core.
    pub fn import_lyrics(
        &mut self,
        parsed: Option<LyricResult>,
        ops: &dyn LyricOps,
        now_ms: f64,
    ) -> Result<Vec<Effect>, String> {
        let query = self.query.clone().ok_or("Play a song first, then import its lyrics.")?;
        let result = parsed.ok_or("This file has no usable LRC or TTML timestamps.")?;
        self.generation += 1;
        self.raw = Some(result.clone());
        self.lookup.pinned = true;
        let mut effects = vec![Effect::Pin { query, result }];
        effects.extend(self.apply_raw(ops, now_ms));
        effects.push(self.emit_lookup());
        Ok(effects)
    }

    pub fn set_user_paused(&mut self, paused: bool) -> Vec<Effect> {
        self.user_paused = paused;
        let mut effects = self.render_effects();
        effects.extend(self.audio_effects());
        effects.push(self.tray());
        effects
    }

    /// The wallpaper windows that now exist (after a rebuild).
    pub fn set_windows(&mut self, windows: Vec<(String, String)>) -> Vec<Effect> {
        self.windows = windows;
        // Re-send everything: new or re-attached windows need their state now.
        self.sent_render.clear();
        let mut effects = self.render_effects();
        effects.extend(self.audio_effects());
        effects
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lunalyr_core::model::{Line, Provenance};

    struct Ops;
    impl LyricOps for Ops {
        fn prepare(&self, result: LyricResult, _repair: bool) -> LyricResult {
            result
        }
        fn summary(&self, c: &Candidate, chosen: bool) -> CandidateSummary {
            CandidateSummary {
                source: c.source,
                source_title: c.source.title().into(),
                title: c.title.clone(),
                artist: c.artist.clone(),
                duration: c.duration,
                precision: c.result.provenance().precision.clone(),
                line_count: c.result.line_count(),
                has_readings: false,
                chosen,
            }
        }
    }

    fn info(title: &str, duration: f64) -> MediaInfo {
        MediaInfo { title: title.into(), artist: "Artist".into(), album: None, duration, source_app: "test".into() }
    }

    fn synced() -> LyricResult {
        LyricResult::Synced {
            lines: vec![Line {
                start: 1.0,
                end: 3.0,
                text: "hello world".into(),
                words: vec![],
                is_instrumental: false,
                word_timed: false,
                romanization: None,
            }],
            provenance: Provenance { precision: "Line-synced".into(), source: "LRCLIB".into() },
        }
    }

    fn candidate(source: SourceId) -> Candidate {
        Candidate {
            source,
            result: synced(),
            title: "Song".into(),
            artist: "Artist".into(),
            duration: Some(200.0),
            score: 1.0,
        }
    }

    fn lookups(effects: &[Effect]) -> Vec<(u64, bool)> {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::Lookup { generation, force, .. } => Some((*generation, *force)),
                _ => None,
            })
            .collect()
    }

    fn emitted(effects: &[Effect], name: &str) -> Vec<Value> {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::Emit { event, payload, .. } if *event == name => Some(payload.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn new_track_starts_one_lookup_and_shows_loading() {
        let mut c = Controller::new(Settings::default(), "test");
        let effects = c.on_platform(PlatformEvent::Media(MediaEvent::Track(Some(info("Song", 200.0)))), 0.0);
        assert_eq!(lookups(&effects), vec![(1, false)]);
        assert_eq!(emitted(&effects, events::LYRICS)[0]["status"], "loading");
        assert_eq!(emitted(&effects, events::TRACK)[0]["track"]["title"], "Song");
        // The same recording again (e.g. a property burst) does not re-query.
        let again = c.on_platform(PlatformEvent::Media(MediaEvent::Track(Some(info("Song", 200.0)))), 1.0);
        assert!(lookups(&again).is_empty());
    }

    #[test]
    fn stale_lookup_results_are_ignored() {
        let mut c = Controller::new(Settings::default(), "test");
        c.on_platform(PlatformEvent::Media(MediaEvent::Track(Some(info("One", 200.0)))), 0.0);
        c.on_platform(PlatformEvent::Media(MediaEvent::Track(Some(info("Two", 200.0)))), 0.0);
        let outcome = LookupOutcome { result: Some(synced()), ..Default::default() };
        assert!(c.on_lookup_finished(1, outcome.clone(), &Ops, 0.0).is_empty());
        let effects = c.on_lookup_finished(2, outcome, &Ops, 0.0);
        assert_eq!(emitted(&effects, events::LYRICS)[0]["status"], "synced");
    }

    #[test]
    fn missing_result_reports_not_found_and_late_duration_retries() {
        let mut c = Controller::new(Settings::default(), "test");
        c.on_platform(PlatformEvent::Media(MediaEvent::Track(Some(info("Song", 0.0)))), 0.0);
        let effects = c.on_lookup_finished(1, LookupOutcome::default(), &Ops, 42.0);
        assert_eq!(emitted(&effects, events::LYRICS)[0], serde_json::json!({"status": "notFound", "reportedAt": 42.0}));
        let retry = c.on_platform(PlatformEvent::Media(MediaEvent::Track(Some(info("Song", 200.0)))), 50.0);
        assert_eq!(lookups(&retry), vec![(2, false)]);
    }

    #[test]
    fn session_end_goes_idle() {
        let mut c = Controller::new(Settings::default(), "test");
        c.on_platform(PlatformEvent::Media(MediaEvent::Track(Some(info("Song", 200.0)))), 0.0);
        let effects = c.on_platform(PlatformEvent::Media(MediaEvent::Track(None)), 99.0);
        assert_eq!(emitted(&effects, events::LYRICS)[0]["status"], "idle");
        assert_eq!(emitted(&effects, events::TRACK)[0]["idleSince"], 99.0);
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::Tray { now_playing, .. } if now_playing == "Nothing playing")));
    }

    #[test]
    fn clock_includes_global_song_and_sync_offsets() {
        let mut c = Controller::new(Settings::default(), "test");
        c.on_platform(PlatformEvent::Media(MediaEvent::Track(Some(info("Song", 200.0)))), 0.0);
        c.update_settings(&serde_json::json!({"lyricOffset": 0.5}), &Ops, 0.0).unwrap();
        c.set_song_offset(-0.25, 0.0).unwrap();
        let effects = c.on_platform(
            PlatformEvent::Media(MediaEvent::Timeline(TimelineInfo {
                position: 10.0,
                sampled_at_ms: 1000.0,
                playing: true,
                rate: 1.0,
            })),
            1000.0,
        );
        let clock = &emitted(&effects, events::CLOCK)[0];
        assert_eq!(clock["position"], 10.0);
        assert!((clock["offset"].as_f64().unwrap() - (0.25 + SYNC_LEAD)).abs() < 1e-9);
    }

    #[test]
    fn coverage_pauses_only_the_covered_monitor_and_respects_setting() {
        let mut c = Controller::new(Settings::default(), "test");
        c.set_windows(vec![("wallpaper-a".into(), "A".into()), ("wallpaper-b".into(), "B".into())]);
        let effects = c.on_platform(PlatformEvent::Desktop(DesktopEvent::Coverage(vec![("A".into(), true)])), 0.0);
        let renders: Vec<_> = effects
            .iter()
            .filter_map(|e| match e {
                Effect::Emit { event, payload, target: Target::Window(label) } if *event == events::RENDER => {
                    Some((label.clone(), payload["paused"].as_bool().unwrap()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(renders, vec![("wallpaper-a".to_string(), true)]);
        let (_, effects) = c.update_settings(&serde_json::json!({"pauseWhenCovered": false}), &Ops, 0.0).unwrap();
        assert!(effects.iter().any(|e| matches!(e, Effect::Emit { event, target: Target::Window(l), .. } if *event == events::RENDER && l == "wallpaper-a")));
        assert!(!c.render_state("wallpaper-a").paused);
    }

    #[test]
    fn explorer_restart_recreates_and_display_change_reattaches() {
        let mut c = Controller::new(Settings::default(), "t");
        assert_eq!(
            c.on_platform(PlatformEvent::Desktop(DesktopEvent::ShellRestarted), 0.0),
            vec![Effect::RebuildWallpapers { recreate: true }]
        );
        assert_eq!(
            c.on_platform(PlatformEvent::Desktop(DesktopEvent::DisplaysChanged), 0.0),
            vec![Effect::RebuildWallpapers { recreate: false }]
        );
    }

    #[test]
    fn user_pause_wins_and_stops_audio() {
        let mut c = Controller::new(Settings { background: BackgroundMode::Visualizer, ..Settings::default() }, "t");
        let start = c.set_windows(vec![("wallpaper-a".into(), "A".into())]);
        assert!(start.contains(&Effect::SetAudio(true)));
        let effects = c.set_user_paused(true);
        assert!(effects.contains(&Effect::SetAudio(false)));
        assert_eq!(c.render_state("wallpaper-a").reason, "user");
    }

    #[test]
    fn settings_patch_merges_validates_and_reacts() {
        let mut c = Controller::new(Settings::default(), "test");
        c.on_platform(PlatformEvent::Media(MediaEvent::Track(Some(info("Song", 200.0)))), 0.0);
        let (settings, effects) =
            c.update_settings(&serde_json::json!({"lyricScale": 7, "displays": "primary"}), &Ops, 0.0).unwrap();
        assert_eq!(settings.lyric_scale, 2.0);
        assert!(effects.contains(&Effect::SaveSettings));
        assert!(effects.contains(&Effect::RebuildWallpapers { recreate: false }));
        assert!(c.update_settings(&serde_json::json!({"frameRate": "fast"}), &Ops, 0.0).is_err());
        assert!(c.update_settings(&serde_json::json!(3), &Ops, 0.0).is_err());
        let mut sources = c.settings().sources.clone();
        sources.enabled.retain(|s| *s != SourceId::NetEase);
        let (_, effects) = c.update_settings(&serde_json::json!({"sources": to_value(&sources)}), &Ops, 0.0).unwrap();
        assert_eq!(lookups(&effects).len(), 1, "changing sources re-runs the lookup");
    }

    #[test]
    fn selecting_a_candidate_pins_it() {
        let mut c = Controller::new(Settings::default(), "test");
        c.on_platform(PlatformEvent::Media(MediaEvent::Track(Some(info("Song", 200.0)))), 0.0);
        let outcome = LookupOutcome {
            result: Some(synced()),
            candidates: vec![candidate(SourceId::Lrclib), candidate(SourceId::NetEase)],
            ..Default::default()
        };
        c.on_lookup_finished(1, outcome, &Ops, 0.0);
        let effects = c.select_candidate(SourceId::NetEase, &Ops, 0.0).unwrap();
        assert!(matches!(&effects[0], Effect::Pin { .. }));
        let lookup = &emitted(&effects, events::LOOKUP)[0];
        assert_eq!(lookup["pinned"], true);
        assert!(c.select_candidate(SourceId::Kuwo, &Ops, 0.0).is_err());
        let forget = c.forget_selection().unwrap();
        assert!(matches!(&forget[0], Effect::Unpin { .. }));
        assert_eq!(lookups(&forget), vec![(3, true)]);
    }

    #[test]
    fn readings_requested_only_when_enabled_and_matched_by_generation() {
        let mut c = Controller::new(Settings { readings: ReadingDisplay::Below, ..Settings::default() }, "t");
        c.on_platform(PlatformEvent::Media(MediaEvent::Track(Some(info("Song", 200.0)))), 0.0);
        let effects =
            c.on_lookup_finished(1, LookupOutcome { result: Some(synced()), ..Default::default() }, &Ops, 0.0);
        let generation = effects
            .iter()
            .find_map(|e| match e {
                Effect::ComputeReadings { generation, .. } => Some(*generation),
                _ => None,
            })
            .expect("readings requested");
        assert!(c.on_readings(generation + 1, vec![None]).is_empty());
        let applied = c.on_readings(generation, vec![None]);
        assert_eq!(emitted(&applied, events::LYRICS)[0]["readings"], serde_json::json!([null]));
    }

    #[test]
    fn fallback_palette_has_nine_hsb_cells() {
        let p = fallback_palette();
        assert_eq!(p.hsb.len(), 9);
        assert!(p.hsb.iter().all(|c| c.iter().all(|v| (0.0..=1.0).contains(v))));
        assert!(!p.from_artwork);
    }
}
