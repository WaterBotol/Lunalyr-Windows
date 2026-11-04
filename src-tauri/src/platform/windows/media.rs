//! Now playing from the Global System Media Transport Controls (GSMTC,
//! `Windows.Media.Control`), the macOS NowPlayingMonitor counterpart.
//!
//! One dedicated MTA thread owns the session manager. WinRT event handlers
//! run on thread-pool threads and only post a [`Signal`] to that thread; it
//! coalesces bursts (~150 ms), re-reads the current session and emits
//! de-duplicated [`MediaEvent`]s. Every 2.5 s it re-samples the timeline
//! (some players only update it on seek) and re-checks which session is
//! current. The anchor ([`TimelineAnchor`]) is reset when the session
//! changes and re-derived when the recording changes. Sessions can vanish
//! mid-call: every failure is logged and retried on the next signal or tick,
//! never unwrapped.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use ::windows::core::{Result, HSTRING};
use ::windows::Foundation::TypedEventHandler;
use ::windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as SessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};
use ::windows::Storage::Streams::{DataReader, IRandomAccessStreamReference};

use super::util::{wait, ComApartment};
use crate::platform::logic::{self, RawTimeline, TimelineAnchor};
use crate::platform::{now_ms, EventSink, MediaEvent, MediaInfo, PlatformError, PlatformEvent, PlatformResult};

/// Events arriving within this window are handled together.
const COALESCE: Duration = Duration::from_millis(150);
/// Timeline re-sample and session re-check period (spec: ≤ 3 s).
const RESAMPLE: Duration = Duration::from_millis(2_500);
/// Players often attach the cover a moment after the title: keep the old
/// colours this long before reporting "no artwork" for a new track.
const ARTWORK_GRACE: Duration = Duration::from_millis(1_500);
/// A title that disappears for less than this (players clearing metadata
/// between tracks, a browser tab re-creating its session) is not reported
/// as "nothing playing".
const NOTHING_GRACE: Duration = Duration::from_millis(1_000);
/// Upper bound for any single WinRT call into a player.
const CALL_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_ARTWORK_BYTES: usize = 10 * 1024 * 1024;
/// Failed thumbnail reads per track before giving up on its cover.
const MAX_ARTWORK_ATTEMPTS: u8 = 3;

pub fn start_media(sink: EventSink) -> PlatformResult<()> {
    thread::Builder::new()
        .name("lunalyr-media".into())
        .spawn(move || run(sink))
        .map_err(|error| PlatformError::msg(format!("could not start the media thread: {error}")))?;
    Ok(())
}

fn run(sink: EventSink) {
    let _com = match ComApartment::multithreaded() {
        Ok(com) => com,
        Err(error) => {
            log::error!("media: COM initialisation failed: {error}");
            return;
        }
    };
    let Some(manager) = request_manager(&sink) else { return };
    let (tx, rx) = mpsc::channel();
    let mut media = Media::new(sink, tx, manager);
    media.run(&rx);
    media.shutdown();
}

/// The session manager can be unavailable for a moment at login (the app
/// autostarts before the media broker); retry with backoff.
fn request_manager(sink: &EventSink) -> Option<SessionManager> {
    let mut delay = Duration::from_secs(1);
    loop {
        match SessionManager::RequestAsync().and_then(|operation| wait(&operation, Duration::from_secs(10))) {
            Ok(manager) => return Some(manager),
            Err(error) => log::warn!("media: GSMTC unavailable ({error}); retrying in {} s", delay.as_secs()),
        }
        let until = Instant::now() + delay;
        while Instant::now() < until {
            if sink.is_closed() {
                return None;
            }
            thread::sleep(Duration::from_millis(250));
        }
        delay = (delay * 2).min(Duration::from_secs(30));
    }
}

#[derive(Debug, Clone, Copy)]
enum Signal {
    /// Current session changed or sessions were added/removed.
    Sessions,
    Media,
    Timeline,
    Playback,
}

#[derive(Debug, Clone, Copy, Default)]
struct Dirty {
    media: bool,
    timeline: bool,
    resample: bool,
}

impl Dirty {
    fn add(&mut self, signal: Signal) {
        match signal {
            Signal::Sessions => {
                self.media = true;
                self.timeline = true;
            }
            Signal::Media => self.media = true,
            Signal::Timeline | Signal::Playback => self.timeline = true,
        }
    }
}

/// The session we follow and our handler tokens on it.
struct Subscribed {
    session: Session,
    source_app: String,
    media: Option<i64>,
    timeline: Option<i64>,
    playback: Option<i64>,
}

/// The last media properties read from the session.
struct Props {
    title: String,
    artist: String,
    album: Option<String>,
    thumbnail: Option<IRandomAccessStreamReference>,
}

#[derive(Default)]
struct Artwork {
    /// (title, artist) of the track this state belongs to.
    key: Option<(String, String)>,
    /// The thumbnail object last read (COM identity).
    thumbnail: Option<IRandomAccessStreamReference>,
    /// What was sent for `key`: `Some(Some(hash))` bytes, `Some(None)` "no
    /// artwork", `None` nothing yet.
    sent: Option<Option<u64>>,
    /// When to give up waiting for a cover and send "no artwork".
    none_at: Option<Instant>,
    failures: u8,
}

struct Media {
    sink: EventSink,
    tx: Sender<Signal>,
    manager: SessionManager,
    current_token: Option<i64>,
    sessions_token: Option<i64>,
    subscribed: Option<Subscribed>,
    props: Option<Props>,
    duration: f64,
    /// Last `Track` sent: `Some(None)` = "nothing playing", `None` = none yet.
    sent_track: Option<Option<MediaInfo>>,
    /// While the title is missing: when "nothing playing" becomes official.
    nothing_at: Option<Instant>,
    anchor: TimelineAnchor,
    /// Arrival time (Unix ms) of the last PlaybackInfoChanged event.
    playback_event_ms: Option<f64>,
    artwork: Artwork,
}

impl Media {
    fn new(sink: EventSink, tx: Sender<Signal>, manager: SessionManager) -> Self {
        let current_token = {
            let tx = tx.clone();
            manager.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
                let _ = tx.send(Signal::Sessions);
                Ok(())
            }))
        };
        let sessions_token = {
            let tx = tx.clone();
            manager.SessionsChanged(&TypedEventHandler::new(move |_, _| {
                let _ = tx.send(Signal::Sessions);
                Ok(())
            }))
        };
        if let Err(error) = &current_token {
            log::warn!("media: could not subscribe to CurrentSessionChanged: {error}");
        }
        if let Err(error) = &sessions_token {
            log::warn!("media: could not subscribe to SessionsChanged: {error}");
        }
        Media {
            sink,
            tx,
            manager,
            current_token: current_token.ok(),
            sessions_token: sessions_token.ok(),
            subscribed: None,
            props: None,
            duration: 0.0,
            sent_track: None,
            nothing_at: None,
            anchor: TimelineAnchor::new(),
            playback_event_ms: None,
            artwork: Artwork::default(),
        }
    }

    fn run(&mut self, rx: &Receiver<Signal>) {
        self.flush(Dirty { media: true, timeline: true, resample: false });
        let mut pending: Option<(Instant, Dirty)> = None;
        let mut next_resample = Instant::now() + RESAMPLE;
        loop {
            let mut deadline = next_resample;
            for at in [pending.map(|(at, _)| at), self.artwork.none_at, self.nothing_at].into_iter().flatten() {
                deadline = deadline.min(at);
            }
            match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(signal) => {
                    if matches!(signal, Signal::Playback) {
                        self.playback_event_ms = Some(now_ms());
                    }
                    // The deadline is set by the first signal of a burst and
                    // not extended, so a chatty player cannot starve us.
                    let (_, dirty) = pending.get_or_insert_with(|| (Instant::now() + COALESCE, Dirty::default()));
                    dirty.add(signal);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if self.sink.is_closed() {
                return;
            }
            let now = Instant::now();
            if pending.is_some_and(|(at, _)| now >= at) {
                if let Some((_, dirty)) = pending.take() {
                    self.flush(dirty);
                }
            }
            if now >= next_resample {
                next_resample = now + RESAMPLE;
                self.flush(Dirty { resample: true, ..Dirty::default() });
            }
            if self.nothing_at.is_some_and(|at| Instant::now() >= at) {
                self.flush(Dirty { media: true, ..Dirty::default() });
            }
            if self.artwork.none_at.is_some_and(|at| Instant::now() >= at) {
                self.artwork_grace_over();
            }
        }
    }

    fn shutdown(&mut self) {
        self.unsubscribe();
        if let Some(token) = self.current_token.take() {
            let _ = self.manager.RemoveCurrentSessionChanged(token);
        }
        if let Some(token) = self.sessions_token.take() {
            let _ = self.manager.RemoveSessionsChanged(token);
        }
    }

    fn send(&self, event: MediaEvent) {
        let _ = self.sink.send(PlatformEvent::Media(event));
    }

    /// Re-reads the current session and reports what changed.
    fn flush(&mut self, dirty: Dirty) {
        let resolved = self.resolve_session();
        let switched = match (&self.subscribed, &resolved) {
            (Some(current), Some(next)) => current.session != *next,
            (None, None) => false,
            _ => true,
        };
        if switched {
            self.unsubscribe();
            if let Some(session) = resolved {
                self.subscribe(session);
            } else {
                log::info!("media: no media session");
            }
            self.props = None;
            self.duration = 0.0;
            self.anchor.reset();
        }
        let Some((session, source_app)) = self.subscribed.as_ref().map(|s| (s.session.clone(), s.source_app.clone()))
        else {
            self.settle_nothing();
            return;
        };

        let mut media_changed = false;
        if switched || dirty.media || dirty.resample || self.props.is_none() {
            match read_props(&session) {
                Ok(props) => {
                    media_changed = switched || dirty.media || self.props.is_none();
                    self.props = Some(props);
                }
                Err(error) => log::debug!("media: could not read media properties: {error}"),
            }
        }
        let timeline = match read_timeline(&session) {
            Ok(raw) => {
                self.duration = raw.duration;
                Some(raw)
            }
            Err(error) => {
                log::debug!("media: could not read the timeline: {error}");
                None
            }
        };

        let Some(info) = self.props.as_ref().and_then(|props| media_info(props, self.duration, &source_app)) else {
            self.settle_nothing();
            return;
        };
        self.nothing_at = None;
        let now = now_ms();
        // Only a different song re-anchors the timeline: players often fill in
        // the album or artist after the title, and that must not reset a valid
        // stamped position to 0.
        let new_song = logic::is_new_song(self.sent_track.as_ref(), &info);
        self.send_track(Some(info.clone()));
        if new_song {
            // Also when the timeline could not be read this time: a session
            // without a timeline (or one updated only on seek) reports the
            // same sample for every track, which must not keep the previous
            // track's anchor.
            self.anchor.forget_sample(now);
        }
        if let Some(raw) = timeline {
            let sample = self.anchor.update(raw, now, self.playback_event_ms);
            self.send(MediaEvent::Timeline(sample));
        }
        let thumbnail = self.props.as_ref().and_then(|props| props.thumbnail.clone());
        self.update_artwork(&info, thumbnail, media_changed);
    }

    /// No session or no title. Reported after [`NOTHING_GRACE`] if a track
    /// is showing, immediately otherwise.
    fn settle_nothing(&mut self) {
        let showing = matches!(self.sent_track, Some(Some(_)));
        if showing {
            match self.nothing_at {
                None => {
                    self.nothing_at = Some(Instant::now() + NOTHING_GRACE);
                    return;
                }
                Some(at) if Instant::now() < at => return,
                Some(_) => {}
            }
        }
        self.nothing_at = None;
        self.artwork = Artwork::default();
        self.send_track(None);
    }

    fn send_track(&mut self, info: Option<MediaInfo>) {
        let unchanged = match (&self.sent_track, &info) {
            (Some(Some(sent)), Some(next)) => logic::same_media(sent, next),
            (Some(None), None) => true,
            _ => false,
        };
        if unchanged {
            return;
        }
        match &info {
            Some(info) => log::debug!(
                "media: track \"{}\" by \"{}\" ({:.1} s) from {}",
                info.title,
                info.artist,
                info.duration,
                info.source_app
            ),
            None => log::debug!("media: nothing playing"),
        }
        self.sent_track = Some(info.clone());
        self.send(MediaEvent::Track(info));
    }

    /// `GetCurrentSession()`, or the first playing session when the system
    /// has no current one.
    fn resolve_session(&self) -> Option<Session> {
        match self.manager.GetCurrentSession() {
            Ok(session) => return Some(session),
            // A null result surfaces as an error with S_OK: no current session.
            Err(error) if error.code().is_ok() => {}
            Err(error) => log::debug!("media: GetCurrentSession failed: {error}"),
        }
        let sessions = match self.manager.GetSessions() {
            Ok(sessions) => sessions,
            Err(error) => {
                log::debug!("media: GetSessions failed: {error}");
                return None;
            }
        };
        sessions.into_iter().find(is_playing)
    }

    fn subscribe(&mut self, session: Session) {
        let source_app = session.SourceAppUserModelId().map(|id| id.to_string_lossy()).unwrap_or_default();
        log::info!("media: following the media session of {source_app}");
        let media = {
            let tx = self.tx.clone();
            session.MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                let _ = tx.send(Signal::Media);
                Ok(())
            }))
        };
        let timeline = {
            let tx = self.tx.clone();
            session.TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
                let _ = tx.send(Signal::Timeline);
                Ok(())
            }))
        };
        let playback = {
            let tx = self.tx.clone();
            session.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                let _ = tx.send(Signal::Playback);
                Ok(())
            }))
        };
        for (what, result) in [("media", &media), ("timeline", &timeline), ("playback", &playback)] {
            if let Err(error) = result {
                log::warn!("media: could not subscribe to {what} changes of {source_app}: {error}");
            }
        }
        self.subscribed = Some(Subscribed {
            session,
            source_app,
            media: media.ok(),
            timeline: timeline.ok(),
            playback: playback.ok(),
        });
    }

    fn unsubscribe(&mut self) {
        let Some(subscribed) = self.subscribed.take() else { return };
        // The session may already be gone; failures do not matter then.
        if let Some(token) = subscribed.media {
            let _ = subscribed.session.RemoveMediaPropertiesChanged(token);
        }
        if let Some(token) = subscribed.timeline {
            let _ = subscribed.session.RemoveTimelinePropertiesChanged(token);
        }
        if let Some(token) = subscribed.playback {
            let _ = subscribed.session.RemovePlaybackInfoChanged(token);
        }
    }

    /// Sends the cover when the track or its thumbnail changes. `media_event`:
    /// the properties were re-read because the player said they changed (a
    /// periodic re-read only fills in a missing cover).
    fn update_artwork(&mut self, info: &MediaInfo, thumbnail: Option<IRandomAccessStreamReference>, media_event: bool) {
        let key = (info.title.clone(), info.artist.clone());
        if self.artwork.key.as_ref() != Some(&key) {
            self.artwork = Artwork { key: Some(key), ..Artwork::default() };
        }
        let Some(thumbnail) = thumbnail else {
            self.await_artwork();
            return;
        };
        if self.artwork.thumbnail.as_ref() == Some(&thumbnail) {
            return;
        }
        let have_cover = matches!(self.artwork.sent, Some(Some(_)));
        if (have_cover && !media_event) || self.artwork.failures >= MAX_ARTWORK_ATTEMPTS {
            return;
        }
        match read_thumbnail(&thumbnail) {
            Ok(Some(bytes)) => {
                self.artwork.thumbnail = Some(thumbnail);
                self.artwork.none_at = None;
                let hash = hash_bytes(&bytes);
                if self.artwork.sent != Some(Some(hash)) {
                    self.artwork.sent = Some(Some(hash));
                    self.send(MediaEvent::Artwork {
                        title: info.title.clone(),
                        artist: info.artist.clone(),
                        bytes: Some(bytes),
                    });
                }
            }
            Ok(None) => {
                self.artwork.thumbnail = Some(thumbnail);
                self.await_artwork();
            }
            Err(error) => {
                self.artwork.failures += 1;
                log::debug!("media: could not read the thumbnail (attempt {}): {error}", self.artwork.failures);
                self.await_artwork();
            }
        }
    }

    fn await_artwork(&mut self) {
        if self.artwork.sent.is_none() && self.artwork.none_at.is_none() {
            self.artwork.none_at = Some(Instant::now() + ARTWORK_GRACE);
        }
    }

    fn artwork_grace_over(&mut self) {
        self.artwork.none_at = None;
        if self.artwork.sent.is_some() {
            return;
        }
        if let Some((title, artist)) = self.artwork.key.clone() {
            self.artwork.sent = Some(None);
            self.send(MediaEvent::Artwork { title, artist, bytes: None });
        }
    }
}

fn is_playing(session: &Session) -> bool {
    session
        .GetPlaybackInfo()
        .and_then(|info| info.PlaybackStatus())
        .map(|status| status == PlaybackStatus::Playing)
        .unwrap_or(false)
}

fn text(value: Result<HSTRING>) -> String {
    value.map(|value| value.to_string_lossy().trim().to_string()).unwrap_or_default()
}

fn read_props(session: &Session) -> Result<Props> {
    let props = wait(&session.TryGetMediaPropertiesAsync()?, CALL_TIMEOUT)?;
    let title = text(props.Title());
    let mut artist = text(props.Artist());
    if artist.is_empty() {
        artist = text(props.AlbumArtist());
    }
    let album = Some(text(props.AlbumTitle())).filter(|album| !album.is_empty());
    // A null thumbnail surfaces as an error: no artwork.
    let thumbnail = props.Thumbnail().ok();
    Ok(Props { title, artist, album, thumbnail })
}

fn read_timeline(session: &Session) -> Result<RawTimeline> {
    let timeline = session.GetTimelineProperties()?;
    let playback = session.GetPlaybackInfo()?;
    let start = timeline.StartTime()?.Duration;
    let end = timeline.EndTime()?.Duration;
    let position = timeline.Position()?.Duration;
    let last_updated_ms =
        timeline.LastUpdatedTime().ok().and_then(|time| logic::winrt_datetime_to_unix_ms(time.UniversalTime));
    let playing = playback.PlaybackStatus()? == PlaybackStatus::Playing;
    // IReference<f64>: null (an error here) when the player does not say.
    let rate = playback.PlaybackRate().and_then(|rate| rate.Value()).unwrap_or(1.0);
    Ok(RawTimeline {
        position: logic::timespan_to_secs(position),
        last_updated_ms,
        playing,
        rate,
        duration: logic::duration_secs(start, end),
    })
}

fn media_info(props: &Props, duration: f64, source_app: &str) -> Option<MediaInfo> {
    if props.title.is_empty() {
        return None;
    }
    Some(MediaInfo {
        title: props.title.clone(),
        artist: props.artist.clone(),
        album: props.album.clone(),
        duration,
        source_app: source_app.to_string(),
    })
}

/// Reads the encoded cover (PNG/JPEG/BMP as the player supplies it).
/// `Ok(None)` for an empty or oversized stream.
fn read_thumbnail(reference: &IRandomAccessStreamReference) -> Result<Option<Vec<u8>>> {
    let stream = wait(&reference.OpenReadAsync()?, CALL_TIMEOUT)?;
    let size = stream.Size().unwrap_or(0);
    if size > MAX_ARTWORK_BYTES as u64 {
        log::warn!("media: ignoring a {size}-byte thumbnail");
        let _ = stream.Close();
        return Ok(None);
    }
    let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
    let mut bytes: Vec<u8> = Vec::with_capacity(size as usize);
    loop {
        // Known size: read exactly that; unknown (0): read until the end.
        let want = if size > 0 { (size as usize).saturating_sub(bytes.len()).min(1 << 20) } else { 1 << 16 };
        if want == 0 {
            break;
        }
        let loaded = wait(&reader.LoadAsync(want as u32)?, CALL_TIMEOUT)? as usize;
        if loaded == 0 {
            break;
        }
        let start = bytes.len();
        if start + loaded > MAX_ARTWORK_BYTES {
            log::warn!("media: thumbnail stream exceeds {MAX_ARTWORK_BYTES} bytes; ignoring it");
            let _ = reader.Close();
            return Ok(None);
        }
        bytes.resize(start + loaded, 0);
        reader.ReadBytes(&mut bytes[start..])?;
    }
    // Closes the input stream as well.
    let _ = reader.Close();
    let _ = stream.Close();
    Ok(if bytes.is_empty() { None } else { Some(bytes) })
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}
