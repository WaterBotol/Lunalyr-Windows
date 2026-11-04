//! Pure helpers for the Windows platform layer.
//!
//! Plain data in, plain data out — no Win32 types — so every decision the
//! Windows code makes (timeline anchoring, full-screen coverage, session
//! lock, which desktop windows to trust and watch, wallpaper window styles,
//! track de-duplication, text chunking for the phonetic analyser) is
//! unit-tested on any host. `platform/windows/*` gathers the facts from the
//! OS and calls these.

use super::{MediaInfo, TimelineInfo};

// ---- WinRT time ---------------------------------------------------------------

/// 100 ns ticks from 1601-01-01 (the WinRT `DateTime` / FILETIME epoch) to
/// 1970-01-01.
pub const UNIX_EPOCH_AS_WINRT_TICKS: i64 = 116_444_736_000_000_000;

/// WinRT `DateTime.UniversalTime` (100 ns ticks since 1601-01-01 UTC) as Unix
/// epoch milliseconds. `None` for unset (zero) or pre-1970 values.
pub fn winrt_datetime_to_unix_ms(ticks: i64) -> Option<f64> {
    if ticks <= UNIX_EPOCH_AS_WINRT_TICKS {
        return None;
    }
    Some((ticks - UNIX_EPOCH_AS_WINRT_TICKS) as f64 / 10_000.0)
}

/// WinRT `TimeSpan.Duration` (100 ns ticks) as seconds.
pub fn timespan_to_secs(ticks: i64) -> f64 {
    ticks as f64 / 10_000_000.0
}

/// Track length from GSMTC `EndTime - StartTime`, in seconds; 0 when the
/// player does not report it.
pub fn duration_secs(start_ticks: i64, end_ticks: i64) -> f64 {
    let duration = timespan_to_secs(end_ticks.saturating_sub(start_ticks));
    if duration.is_finite() && duration > 0.0 {
        duration
    } else {
        0.0
    }
}

// ---- Track identity -------------------------------------------------------------

/// True when two now-playing reports describe the same thing, so no
/// `MediaEvent::Track` needs to be sent. Durations within half a second are
/// equal (players re-report EndTime with jitter), as on macOS.
pub fn same_media(a: &MediaInfo, b: &MediaInfo) -> bool {
    same_recording(a, b) && (a.duration - b.duration).abs() < 0.5
}

/// Same title/artist/album/player, ignoring the duration (which players often
/// report a moment after the title).
/// A different song (title changed, or nothing was showing): the timeline
/// anchor starts over. Album/artist fill-ins for the same title are not.
pub fn is_new_song(sent: Option<&Option<MediaInfo>>, next: &MediaInfo) -> bool {
    !matches!(sent, Some(Some(sent)) if sent.title == next.title)
}

pub fn same_recording(a: &MediaInfo, b: &MediaInfo) -> bool {
    a.title == b.title && a.artist == b.artist && a.album == b.album && a.source_app == b.source_app
}

/// Does `next` start a new recording, compared with the last `Track` sent?
/// `sent`: `None` before the first one, `Some(None)` after "nothing playing"
/// (a title that returns after that is new again; one that returns within
/// the grace period, before "nothing playing" was sent, is not).
pub fn is_new_recording(sent: Option<&Option<MediaInfo>>, next: &MediaInfo) -> bool {
    !matches!(sent, Some(Some(sent)) if same_recording(sent, next))
}

// ---- Timeline anchoring --------------------------------------------------------

/// One read of GSMTC `GetTimelineProperties()` + `GetPlaybackInfo()`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawTimeline {
    /// `Position`, seconds.
    pub position: f64,
    /// `LastUpdatedTime` as Unix ms; `None` when unset.
    pub last_updated_ms: Option<f64>,
    pub playing: bool,
    /// `PlaybackRate` (1.0 when the player does not report it).
    pub rate: f64,
    /// `EndTime - StartTime`, seconds; 0 when unknown.
    pub duration: f64,
}

/// `LastUpdatedTime` further in the future than this is not trusted.
const FUTURE_TOLERANCE_MS: f64 = 1_000.0;
/// ...nor anything older than this.
const MAX_SAMPLE_AGE_MS: f64 = 24.0 * 3_600_000.0;
/// A sample stamped this long before playback (re)started belongs to the
/// previous run: the position froze while paused, so it was true when
/// playback resumed, not when it was stamped.
const RESUME_TOLERANCE_MS: f64 = 750.0;
/// Extrapolating further than this past the end of the track means the
/// anchor is stale.
const OVERRUN_TOLERANCE_S: f64 = 2.0;
/// How long after a PlaybackInfoChanged event its time still counts as the
/// moment playback resumed (or paused).
const RESUME_EVENT_WINDOW_MS: f64 = 5_000.0;
/// When the recording changes, a sample that is still the one from before
/// and older than this belongs to the previous recording: the player has not
/// published a timeline for the new one (some only do on seek, some never).
/// Well above the event coalescing window and the usual lag between a
/// player's metadata and timeline updates, which arrive in either order.
const STALE_SAMPLE_MS: f64 = 3_000.0;

/// (`Position`, `LastUpdatedTime`) bits: identifies one published sample.
type SampleKey = (u64, Option<u64>);

/// Turns GSMTC timeline reads into [`TimelineInfo`] anchors the renderer can
/// extrapolate from (`position + (now - sampled_at) * rate` while playing).
///
/// Players behave differently, and the anchor must stay put across the
/// periodic re-samples or lyrics would jump:
/// * Most stamp `LastUpdatedTime` whenever they set the timeline (play,
///   pause, seek, track change): `position` was true at that instant.
/// * Some do not update the timeline when they resume: the position held
///   during the pause, so it is anchored at the resume instant instead.
/// * Some never stamp it: the anchor is the first time this exact sample was
///   seen, and repeated identical reads keep that time.
/// * Some publish no timeline at all (Position 0, never stamped), or update
///   it only on seek, so the sample stays the same across pauses and tracks.
///   A pause the player does not publish freezes the position where the
///   extrapolation had it (the renderer would otherwise jump back to the
///   stale sample), a resume continues from there, and a new recording
///   ([`TimelineAnchor::forget_sample`]) counts from when it was first seen.
#[derive(Debug, Clone)]
pub struct TimelineAnchor {
    last_raw: Option<SampleKey>,
    /// When the last sample was true by its own account: its
    /// `LastUpdatedTime`, or when it was first seen.
    sample_ms: f64,
    /// The position reported as true at `anchor_ms`: the sample's, or one
    /// derived from it (frozen at a pause, counted from a new recording).
    position: f64,
    anchor_ms: f64,
    /// Rate of the last playing read.
    rate: f64,
    /// The sample still belongs to the previous recording, so its duration
    /// says nothing about this one.
    assumed: bool,
    was_playing: Option<bool>,
    playing_since_ms: Option<f64>,
    /// Left by `forget_sample` for the next read.
    stale: Option<StaleSample>,
}

/// The sample that was current when the recording changed.
#[derive(Debug, Clone, Copy)]
struct StaleSample {
    key: SampleKey,
    /// Its `TimelineAnchor::sample_ms`.
    sample_ms: f64,
    /// When the new recording was first seen.
    seen_ms: f64,
}

impl Default for TimelineAnchor {
    fn default() -> Self {
        TimelineAnchor {
            last_raw: None,
            sample_ms: 0.0,
            position: 0.0,
            anchor_ms: 0.0,
            rate: 1.0,
            assumed: false,
            was_playing: None,
            playing_since_ms: None,
            stale: None,
        }
    }
}

impl TimelineAnchor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget everything (the media session changed).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The reported recording changed at `now_ms`: a sample equal to the
    /// last one must not keep the old anchor. The play/pause history is
    /// kept, so a resume is still recognised. If the next read still returns
    /// the previous recording's sample (stamped, or first seen, more than 3 s
    /// earlier), the new recording counts from `now_ms`; a sample the player
    /// published just before its metadata keeps its own stamp.
    pub fn forget_sample(&mut self, now_ms: f64) {
        self.stale = match self.last_raw.take() {
            Some(key) => Some(StaleSample { key, sample_ms: self.sample_ms, seen_ms: now_ms }),
            // Changed again before the next read: still the same stale sample.
            None => self.stale.map(|stale| StaleSample { seen_ms: now_ms, ..stale }),
        };
        self.assumed = false;
    }

    /// `now_ms`: wall clock of this read. `playback_event_ms`: when the most
    /// recent PlaybackInfoChanged event arrived (used as the moment playback
    /// resumed or paused when this read is the first to see it).
    pub fn update(&mut self, raw: RawTimeline, now_ms: f64, playback_event_ms: Option<f64>) -> TimelineInfo {
        let position = if raw.position.is_finite() { raw.position.max(0.0) } else { 0.0 };
        let rate = if raw.rate.is_finite() && raw.rate > 0.0 && raw.rate <= 16.0 { raw.rate } else { 1.0 };
        let event = playback_event_ms
            .filter(|t| t.is_finite() && *t <= now_ms && *t >= now_ms - RESUME_EVENT_WINDOW_MS)
            .unwrap_or(now_ms);
        let key = (position.to_bits(), raw.last_updated_ms.map(f64::to_bits));
        let unchanged = self.last_raw == Some(key);
        let stale = self.stale.take();

        if !raw.playing && self.was_playing == Some(true) && unchanged {
            // Paused without publishing where: the position froze where the
            // extrapolation had it, and holds until playback resumes.
            let paused_at = event.max(self.anchor_ms);
            let mut frozen = self.position + (paused_at - self.anchor_ms) / 1000.0 * self.rate;
            let duration = self.known_duration(&raw);
            if duration > 0.0 {
                frozen = frozen.min(duration);
            }
            self.position = frozen;
            self.anchor_ms = paused_at;
            self.was_playing = Some(false);
            self.playing_since_ms = None;
            return TimelineInfo { position: frozen, sampled_at_ms: paused_at, playing: false, rate };
        }

        let resumed = raw.playing && self.was_playing == Some(false);
        if resumed {
            self.playing_since_ms = Some(event);
        } else if !raw.playing {
            self.playing_since_ms = None;
        }
        self.was_playing = Some(raw.playing);

        let (mut shown, mut anchor) = if unchanged {
            (self.position, self.anchor_ms)
        } else {
            self.assumed = false;
            self.sample_ms = raw.last_updated_ms.filter(|t| t.is_finite()).unwrap_or(now_ms);
            match stale {
                Some(stale) if stale.key == key && stale.sample_ms < stale.seen_ms - STALE_SAMPLE_MS => {
                    // The player has not published the new recording's
                    // timeline: it started when it was first seen.
                    self.assumed = true;
                    (0.0, stale.seen_ms.min(now_ms))
                }
                _ => (position, sample_anchor(raw.last_updated_ms, now_ms)),
            }
        };
        if raw.playing {
            if let Some(since) = self.playing_since_ms {
                // Resumed without a new sample: the position held during the
                // pause (however short), so it was true when playback resumed.
                if anchor < since - RESUME_TOLERANCE_MS || (resumed && unchanged) {
                    anchor = since;
                }
            }
            let duration = self.known_duration(&raw);
            if duration > 0.0 {
                let projected = shown + (now_ms - anchor) / 1000.0 * rate;
                if projected > duration + OVERRUN_TOLERANCE_S {
                    shown = position;
                    anchor = now_ms;
                }
            }
            self.rate = rate;
        }
        let anchor = anchor.min(now_ms);
        self.last_raw = Some(key);
        self.position = shown;
        self.anchor_ms = anchor;
        TimelineInfo { position: shown, sampled_at_ms: anchor, playing: raw.playing, rate }
    }

    /// The track length to bound extrapolation by; 0 (unknown) while the
    /// sample, and so its duration, still belongs to the previous recording.
    fn known_duration(&self, raw: &RawTimeline) -> f64 {
        if self.assumed || !raw.duration.is_finite() {
            0.0
        } else {
            raw.duration
        }
    }
}

/// Where a new sample is anchored: its `LastUpdatedTime` when plausible
/// (clamped to now), otherwise when it was first seen (this read).
fn sample_anchor(last_updated_ms: Option<f64>, now_ms: f64) -> f64 {
    match last_updated_ms {
        Some(t) if t.is_finite() && t <= now_ms + FUTURE_TOLERANCE_MS && t >= now_ms - MAX_SAMPLE_AGE_MS => {
            t.min(now_ms)
        }
        _ => now_ms,
    }
}

// ---- Full-screen / maximised coverage ------------------------------------------

/// A rectangle in physical virtual-screen pixels (Win32 `RECT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Rect { left, top, right, bottom }
    }

    pub fn from_origin_size(x: i32, y: i32, width: i32, height: i32) -> Self {
        Rect { left: x, top: y, right: x.saturating_add(width), bottom: y.saturating_add(height) }
    }

    pub fn is_empty(&self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }

    pub fn contains(&self, other: &Rect) -> bool {
        self.left <= other.left && self.top <= other.top && self.right >= other.right && self.bottom >= other.bottom
    }
}

// Win32 window style bits (winuser.h). Duplicated here so the style logic is
// testable without the `windows` crate.
pub mod style {
    pub const WS_POPUP: u32 = 0x8000_0000;
    pub const WS_CHILD: u32 = 0x4000_0000;
    pub const WS_MINIMIZE: u32 = 0x2000_0000;
    pub const WS_VISIBLE: u32 = 0x1000_0000;
    pub const WS_DISABLED: u32 = 0x0800_0000;
    pub const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
    pub const WS_CLIPCHILDREN: u32 = 0x0200_0000;
    pub const WS_MAXIMIZE: u32 = 0x0100_0000;
    /// WS_BORDER | WS_DLGFRAME.
    pub const WS_CAPTION: u32 = 0x00C0_0000;
    pub const WS_VSCROLL: u32 = 0x0020_0000;
    pub const WS_HSCROLL: u32 = 0x0010_0000;
    pub const WS_SYSMENU: u32 = 0x0008_0000;
    pub const WS_THICKFRAME: u32 = 0x0004_0000;
    pub const WS_MINIMIZEBOX: u32 = 0x0002_0000;
    pub const WS_MAXIMIZEBOX: u32 = 0x0001_0000;

    pub const WS_EX_DLGMODALFRAME: u32 = 0x0000_0001;
    pub const WS_EX_TOPMOST: u32 = 0x0000_0008;
    pub const WS_EX_TRANSPARENT: u32 = 0x0000_0020;
    pub const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
    pub const WS_EX_WINDOWEDGE: u32 = 0x0000_0100;
    pub const WS_EX_CLIENTEDGE: u32 = 0x0000_0200;
    pub const WS_EX_STATICEDGE: u32 = 0x0002_0000;
    pub const WS_EX_APPWINDOW: u32 = 0x0004_0000;
    pub const WS_EX_LAYERED: u32 = 0x0008_0000;
    pub const WS_EX_NOREDIRECTIONBITMAP: u32 = 0x0020_0000;
    pub const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
}

/// Window classes that belong to the shell and never count as covering.
pub const SHELL_WINDOW_CLASSES: &[&str] = &["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"];

/// What the coverage check needs to know about one top-level window.
#[derive(Debug, Clone, Default)]
pub struct WindowFacts {
    pub class_name: String,
    /// `GetShellWindow()` / `GetDesktopWindow()`.
    pub is_shell_window: bool,
    pub visible: bool,
    /// DWMWA_CLOAKED (other virtual desktop, suspended UWP app, ...).
    pub cloaked: bool,
    pub minimized: bool,
    pub maximized: bool,
    pub own_process: bool,
    pub ex_style: u32,
    /// For WS_EX_LAYERED windows: `LWA_ALPHA` 255 without a colour key.
    pub opaque_layered: bool,
    /// `GetWindowRect`, physical pixels.
    pub rect: Rect,
    /// Index into the monitor list of `MonitorFromWindow`.
    pub monitor: Option<usize>,
}

/// Can this window hide the wallpaper? Deliberately conservative: pausing a
/// wallpaper the user can see (a frozen desktop) is worse than rendering one
/// nobody sees, so overlays — click-through, tool, no-activate or
/// translucent layered windows (game/GPU overlays, screen recorders) — never
/// count.
pub fn can_cover(window: &WindowFacts) -> bool {
    use style::*;
    if !window.visible || window.cloaked || window.minimized || window.own_process || window.is_shell_window {
        return false;
    }
    if SHELL_WINDOW_CLASSES.contains(&window.class_name.as_str()) {
        return false;
    }
    if window.ex_style & (WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) != 0 {
        return false;
    }
    if window.ex_style & WS_EX_LAYERED != 0 && !window.opaque_layered {
        return false;
    }
    !window.rect.is_empty()
}

/// `QUERY_USER_NOTIFICATION_STATE` values (shellapi.h) the coverage check
/// distinguishes.
pub mod notification_state {
    pub const RUNNING_D3D_FULL_SCREEN: i32 = 3;
    pub const PRESENTATION_MODE: i32 = 4;
}

/// Does this `SHQueryUserNotificationState` result mean the foreground
/// window's monitor is covered? Only exclusive full-screen Direct3D.
/// Presentation mode is a user toggle (Mobility Center, `presentationsettings
/// /start`, or automatic for a remembered display) that says nothing about
/// any window's size; a real full-screen slideshow is caught by the
/// maximised / contains-the-monitor window check in [`coverage`].
pub fn notification_state_covers(state: i32) -> bool {
    state == notification_state::RUNNING_D3D_FULL_SCREEN
}

/// Per monitor: is it covered by a maximised window on it, by a window whose
/// rectangle contains the whole monitor, or (exclusive full-screen Direct3D,
/// see [`notification_state_covers`]) is it the foreground window's monitor?
pub fn coverage(monitors: &[Rect], windows: &[WindowFacts], fullscreen_monitor: Option<usize>) -> Vec<bool> {
    monitors
        .iter()
        .enumerate()
        .map(|(index, monitor)| {
            fullscreen_monitor == Some(index)
                || windows.iter().any(|window| {
                    can_cover(window)
                        && ((window.maximized && window.monitor == Some(index)) || window.rect.contains(monitor))
                })
        })
        .collect()
}

// ---- Session lock -----------------------------------------------------------------

/// `WTSINFOEX_LEVEL1_W::SessionFlags` for a locked session.
pub const WTS_SESSIONSTATE_LOCK: i32 = 0;

/// Is the session locked, by `WTSQuerySessionInformationW(WTSSessionInfoEx)`
/// level-1 `SessionFlags`? Only an explicit WTS_SESSIONSTATE_LOCK counts:
/// WTS_SESSIONSTATE_UNKNOWN (-1) and anything else read as unlocked, so an
/// odd answer never pauses the wallpapers.
pub fn session_flags_locked(session_flags: i32) -> bool {
    session_flags == WTS_SESSIONSTATE_LOCK
}

// ---- Desktop windows ----------------------------------------------------------------

/// The desktop window the wallpaper windows are children of: Progman on the
/// raised desktop (24H2+), the empty WorkerW behind the icons otherwise.
pub fn wallpaper_parent(progman: isize, workerw: isize, raised: bool) -> isize {
    if raised {
        progman
    } else {
        workerw
    }
}

/// Explorer windows whose destruction takes the wallpaper windows with them
/// (they must then be recreated): Progman and the wallpapers' parent. Not the
/// raised desktop's WorkerW: the wallpapers are its siblings there, it does
/// not take them along, and Explorer replaces it whenever the desktop
/// background changes; re-attaching below the new one (the shell's watchdog)
/// repairs that without recreating any webview.
pub fn watched_shell_windows(progman: isize, workerw: isize, raised: bool) -> Vec<isize> {
    let parent = wallpaper_parent(progman, workerw, raised);
    if parent == progman {
        vec![progman]
    } else {
        vec![progman, parent]
    }
}

/// Legacy layout: can this top-level window, which has a SHELLDLL_DefView
/// child, be Explorer's desktop icon layer? Only Progman itself or a WorkerW
/// of Progman's process. Other apps host shell views in their own top-level
/// windows too (the legacy Explorer-style Open/Save dialog, IShellBrowser
/// frames), and those sit above the desktop in z-order.
pub fn is_desktop_icon_host(is_progman: bool, class_name: &str, pid: u32, explorer_pid: u32) -> bool {
    is_progman || (explorer_pid != 0 && pid == explorer_pid && class_name == "WorkerW")
}

// ---- Wallpaper window styles ------------------------------------------------------

/// GWL_STYLE for a wallpaper window parented into the desktop: a plain child
/// (no caption, frame, system menu, min/max state). WS_VISIBLE and
/// WS_DISABLED are left alone: visibility goes through ShowWindow and the
/// enabled state through EnableWindow.
pub fn attached_style(current: u32) -> u32 {
    use style::*;
    let strip = WS_POPUP
        | WS_CAPTION
        | WS_THICKFRAME
        | WS_SYSMENU
        | WS_MINIMIZEBOX
        | WS_MAXIMIZEBOX
        | WS_MINIMIZE
        | WS_MAXIMIZE
        | WS_VSCROLL
        | WS_HSCROLL;
    (current & !strip) | WS_CHILD | WS_CLIPSIBLINGS | WS_CLIPCHILDREN
}

/// GWL_EXSTYLE for an attached wallpaper. Never activated (NOACTIVATE), no
/// taskbar/Alt-Tab presence (TOOLWINDOW, no APPWINDOW), no frame edges. In
/// the raised-desktop layout it must be a layered child, and LAYERED +
/// TRANSPARENT also makes it invisible to hit-testing, so desktop input
/// falls through to the shell exactly as if the wallpaper were not there.
pub fn attached_ex_style(current: u32, raised: bool) -> u32 {
    use style::*;
    let strip = WS_EX_APPWINDOW
        | WS_EX_TOPMOST
        | WS_EX_DLGMODALFRAME
        | WS_EX_WINDOWEDGE
        | WS_EX_CLIENTEDGE
        | WS_EX_STATICEDGE
        | WS_EX_LAYERED
        | WS_EX_TRANSPARENT;
    let mut ex = (current & !strip) | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
    if raised {
        ex |= WS_EX_LAYERED | WS_EX_TRANSPARENT;
    }
    ex
}

/// GWL_STYLE to restore on detach: the original style, keeping the current
/// visibility/enabled bits (set beforehand with ShowWindow/EnableWindow).
pub fn restored_style(original: u32, current: u32) -> u32 {
    use style::*;
    let live = WS_VISIBLE | WS_DISABLED;
    (original & !live) | (current & live)
}

/// Does an attached wallpaper still carry the styles `attach` gave it?
/// (tao rewrites GWL_STYLE/GWL_EXSTYLE wholesale whenever its own window
/// flags change, e.g. on `show()` or `set_skip_taskbar()`.)
pub fn styles_intact(style: u32, ex_style: u32, raised: bool) -> bool {
    use style::*;
    if style & WS_CHILD == 0 || style & WS_POPUP != 0 || style & WS_VISIBLE == 0 {
        return false;
    }
    if ex_style & WS_EX_NOACTIVATE == 0 {
        return false;
    }
    !raised || ex_style & (WS_EX_LAYERED | WS_EX_TRANSPARENT) == (WS_EX_LAYERED | WS_EX_TRANSPARENT)
}

// ---- Japanese phonetic analysis -------------------------------------------------

/// Splits `text` into pieces of at most `max_utf16` UTF-16 code units
/// (`JapanesePhoneticAnalyzer.GetWords` accepts 100 characters), preferring
/// to cut after whitespace or punctuation and never inside a character.
/// Concatenating the pieces gives back `text`.
pub fn phonetic_chunks(text: &str, max_utf16: usize) -> Vec<&str> {
    let max = max_utf16.max(2);
    let mut chunks = Vec::new();
    let mut start = 0usize;
    let mut units = 0usize;
    // Byte index just after the last break opportunity inside the chunk.
    let mut last_break: Option<usize> = None;
    for (index, ch) in text.char_indices() {
        let width = ch.len_utf16();
        if units + width > max && index > start {
            let cut = match last_break {
                Some(cut) if cut > start => cut,
                _ => index,
            };
            chunks.push(&text[start..cut]);
            start = cut;
            // The text between the cut and this character moves on to the
            // next chunk, together with its own break opportunities.
            let carried = &text[cut..index];
            units = carried.encode_utf16().count();
            last_break =
                carried.char_indices().rev().find(|(_, c)| is_break_after(*c)).map(|(i, c)| cut + i + c.len_utf8());
            if units + width > max && index > start {
                chunks.push(carried);
                start = index;
                units = 0;
                last_break = None;
            }
        }
        units += width;
        if is_break_after(ch) {
            last_break = Some(index + ch.len_utf8());
        }
    }
    if start < text.len() {
        chunks.push(&text[start..]);
    }
    chunks
}

fn is_break_after(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            '、' | '。'
                | '，'
                | '．'
                | '！'
                | '？'
                | '・'
                | '…'
                | '」'
                | '』'
                | '）'
                | ')'
                | ','
                | '.'
                | '!'
                | '?'
                | ';'
                | ':'
                | '～'
                | '〜'
        )
}

#[cfg(test)]
mod tests {
    use super::style::*;
    use super::*;

    fn info(title: &str, duration: f64) -> MediaInfo {
        MediaInfo {
            title: title.into(),
            artist: "Artist".into(),
            album: Some("Album".into()),
            duration,
            source_app: "Spotify.exe".into(),
        }
    }

    #[test]
    fn winrt_time_converts_to_unix_ms() {
        assert_eq!(winrt_datetime_to_unix_ms(0), None);
        assert_eq!(winrt_datetime_to_unix_ms(UNIX_EPOCH_AS_WINRT_TICKS), None);
        // 2024-01-01T00:00:00Z = 1_704_067_200_000 ms.
        let ticks = UNIX_EPOCH_AS_WINRT_TICKS + 1_704_067_200_000 * 10_000;
        assert_eq!(winrt_datetime_to_unix_ms(ticks), Some(1_704_067_200_000.0));
        assert_eq!(timespan_to_secs(2_500_000), 0.25);
        assert_eq!(duration_secs(0, 2_130_000_000), 213.0);
        assert_eq!(duration_secs(5, 5), 0.0);
        assert_eq!(duration_secs(10, 0), 0.0);
    }

    #[test]
    fn duration_jitter_is_not_a_new_track() {
        assert!(same_media(&info("A", 200.0), &info("A", 200.3)));
        assert!(!same_media(&info("A", 0.0), &info("A", 200.0)));
        assert!(same_recording(&info("A", 0.0), &info("A", 200.0)));
        assert!(!same_recording(&info("A", 200.0), &info("B", 200.0)));
    }

    fn raw(position: f64, last_updated_ms: Option<f64>, playing: bool) -> RawTimeline {
        RawTimeline { position, last_updated_ms, playing, rate: 1.0, duration: 300.0 }
    }

    #[test]
    fn anchor_uses_last_updated_time() {
        let mut anchor = TimelineAnchor::new();
        let t = anchor.update(raw(10.0, Some(1_000_000.0), true), 1_005_000.0, None);
        assert_eq!(t.position, 10.0);
        assert_eq!(t.sampled_at_ms, 1_000_000.0);
        assert!(t.playing);
        // Re-sampling the unchanged timeline keeps the anchor.
        let t = anchor.update(raw(10.0, Some(1_000_000.0), true), 1_008_000.0, None);
        assert_eq!(t.sampled_at_ms, 1_000_000.0);
    }

    #[test]
    fn anchor_without_stamp_is_first_sighting() {
        let mut anchor = TimelineAnchor::new();
        let t = anchor.update(raw(42.0, None, true), 2_000_000.0, None);
        assert_eq!(t.sampled_at_ms, 2_000_000.0);
        // Same frozen sample 3 s later: still anchored at first sighting.
        let t = anchor.update(raw(42.0, None, true), 2_003_000.0, None);
        assert_eq!(t.sampled_at_ms, 2_000_000.0);
        // A live position (player updates it without a stamp) re-anchors.
        let t = anchor.update(raw(48.0, None, true), 2_006_000.0, None);
        assert_eq!(t.sampled_at_ms, 2_006_000.0);
    }

    #[test]
    fn anchor_rejects_future_and_ancient_stamps() {
        let mut anchor = TimelineAnchor::new();
        let t = anchor.update(raw(1.0, Some(5_000_000.0), true), 1_000_000.0, None);
        assert_eq!(t.sampled_at_ms, 1_000_000.0);
        anchor.reset();
        let t = anchor.update(raw(1.0, Some(1_000_400.0), false), 1_000_000.0, None);
        assert_eq!(t.sampled_at_ms, 1_000_000.0, "slightly-future stamps clamp to now");
        anchor.reset();
        let now = 200.0 * 3_600_000.0;
        let t = anchor.update(raw(1.0, Some(now - 30.0 * 3_600_000.0), false), now, None);
        assert_eq!(t.sampled_at_ms, now);
    }

    #[test]
    fn resume_without_timeline_update_anchors_at_resume() {
        let mut anchor = TimelineAnchor::new();
        // Paused at 30 s, stamped at t=1000 s.
        let t = anchor.update(raw(30.0, Some(1_000_000.0), false), 1_001_000.0, None);
        assert!(!t.playing);
        // Resumed at t=1060 s (event), read at 1060.15 s; the player did not
        // touch the timeline, so the stamp is a minute old.
        let t = anchor.update(raw(30.0, Some(1_000_000.0), true), 1_060_150.0, Some(1_060_000.0));
        assert_eq!(t.sampled_at_ms, 1_060_000.0);
        // Periodic re-sample keeps that anchor.
        let t = anchor.update(raw(30.0, Some(1_000_000.0), true), 1_063_000.0, None);
        assert_eq!(t.sampled_at_ms, 1_060_000.0);
    }

    #[test]
    fn resume_with_fresh_stamp_keeps_the_stamp() {
        let mut anchor = TimelineAnchor::new();
        anchor.update(raw(30.0, Some(1_000_000.0), false), 1_001_000.0, None);
        // Player stamped the resume itself, 200 ms before we saw the event.
        let t = anchor.update(raw(30.0, Some(1_059_800.0), true), 1_060_150.0, Some(1_060_000.0));
        assert_eq!(t.sampled_at_ms, 1_059_800.0);
    }

    #[test]
    fn anchor_never_extrapolates_far_past_the_end() {
        let mut anchor = TimelineAnchor::new();
        // First read after launch: playing, stamped an hour ago.
        let t = anchor.update(raw(100.0, Some(1_000_000.0), true), 1_000_000.0 + 3_600_000.0, None);
        assert_eq!(t.sampled_at_ms, 1_000_000.0 + 3_600_000.0);
    }

    #[test]
    fn rate_is_sanitised() {
        let mut anchor = TimelineAnchor::new();
        let mut sample = raw(0.0, None, true);
        sample.rate = f64::NAN;
        assert_eq!(anchor.update(sample, 1.0, None).rate, 1.0);
        sample.rate = 0.0;
        assert_eq!(anchor.update(sample, 2.0, None).rate, 1.0);
        sample.rate = 1.5;
        assert_eq!(anchor.update(sample, 3.0, None).rate, 1.5);
        sample.position = -4.0;
        assert_eq!(anchor.update(sample, 4.0, None).position, 0.0);
    }

    /// What the renderer shows at `now_ms` for this anchor.
    fn shown(t: TimelineInfo, now_ms: f64) -> f64 {
        t.position + if t.playing { (now_ms - t.sampled_at_ms) / 1000.0 * t.rate } else { 0.0 }
    }

    /// A session that publishes no timeline: Position 0, never stamped, no
    /// duration (Firefox before 125, for one).
    fn untimed(playing: bool) -> RawTimeline {
        RawTimeline { position: 0.0, last_updated_ms: None, playing, rate: 1.0, duration: 0.0 }
    }

    #[test]
    fn only_a_new_title_re_anchors_the_timeline() {
        let mut a = info("A", 200.0);
        assert!(is_new_song(None, &a));
        assert!(is_new_song(Some(&None), &a));
        let sent = Some(a.clone());
        a.album = Some("Late album".into());
        assert!(!is_new_song(Some(&sent), &a), "album filled in later: same song");
        assert!(is_new_song(Some(&sent), &info("B", 200.0)));
    }

    #[test]
    fn new_recording_decisions() {
        let a = info("A", 200.0);
        assert!(is_new_recording(None, &a), "nothing sent yet");
        assert!(is_new_recording(Some(&None), &a), "after a Track(None) gap, even the same title");
        assert!(is_new_recording(Some(&Some(info("B", 200.0))), &a));
        assert!(!is_new_recording(Some(&Some(info("A", 0.0))), &a), "a duration update is the same recording");
    }

    #[test]
    fn a_new_recording_without_a_timeline_counts_from_its_first_sighting() {
        let t0 = 1_000_000.0;
        let mut anchor = TimelineAnchor::new();
        assert_eq!(anchor.update(untimed(true), t0, None).sampled_at_ms, t0);
        let t = anchor.update(untimed(true), t0 + 207_500.0, None);
        assert_eq!(shown(t, t0 + 210_000.0), 210.0);
        // The next track in the same session, gapless (no paused read in
        // between); the reads look exactly as before.
        let t1 = t0 + 210_000.0;
        anchor.forget_sample(t1);
        let t = anchor.update(untimed(true), t1 + 150.0, None);
        assert_eq!((t.position, t.sampled_at_ms, t.playing), (0.0, t1, true));
        assert_eq!(shown(t, t1 + 1_000.0), 1.0);
        // Re-samples keep counting from there.
        assert_eq!(anchor.update(untimed(true), t1 + 2_650.0, None).sampled_at_ms, t1);
        // After a Track(None) gap (also a new recording) and when re-sampled
        // right away, it is the read time.
        let t2 = t1 + 300_000.0;
        anchor.forget_sample(t2);
        assert_eq!(anchor.update(untimed(true), t2, None).sampled_at_ms, t2);
    }

    #[test]
    fn a_timeline_published_before_the_metadata_keeps_its_stamp() {
        let tb = 2_000_000.0;
        let mut anchor = TimelineAnchor::new();
        anchor.update(raw(95.0, Some(tb - 120_000.0), true), tb - 2_000.0, None);
        // The player stamps the next track's start, and its title follows
        // 600 ms later.
        anchor.update(raw(0.0, Some(tb), true), tb + 100.0, None);
        anchor.forget_sample(tb + 600.0);
        let t = anchor.update(raw(0.0, Some(tb), true), tb + 600.0, None);
        assert_eq!((t.position, t.sampled_at_ms), (0.0, tb));
    }

    #[test]
    fn a_new_recording_ignores_the_previous_tracks_stale_sample() {
        // A player that updates its timeline only on seek: the last seek in
        // the previous track (95 s, two minutes ago) is still what it reports.
        let seek = 3_000_000.0;
        let mut anchor = TimelineAnchor::new();
        anchor.update(raw(95.0, Some(seek), true), seek + 50.0, None);
        let next = seek + 120_000.0;
        anchor.forget_sample(next);
        let t = anchor.update(raw(95.0, Some(seek), true), next + 150.0, None);
        assert_eq!((t.position, t.sampled_at_ms), (0.0, next));
        // The stale sample's duration (300 s, the previous track's) does not
        // cut the new one short.
        let t = anchor.update(raw(95.0, Some(seek), true), next + 400_000.0, None);
        assert_eq!((t.position, t.sampled_at_ms), (0.0, next));
        // Another track change before any fresh sample: counts from that one.
        let third = next + 450_000.0;
        anchor.forget_sample(third);
        let t = anchor.update(raw(95.0, Some(seek), true), third + 150.0, None);
        assert_eq!((t.position, t.sampled_at_ms), (0.0, third));
        // A seek publishes a fresh sample, which is used as is.
        let t = anchor.update(raw(30.0, Some(third + 9_000.0), true), third + 9_100.0, None);
        assert_eq!((t.position, t.sampled_at_ms), (30.0, third + 9_000.0));
    }

    #[test]
    fn pause_and_resume_without_a_timeline_keep_the_position() {
        let t0 = 1_000_000.0;
        let mut anchor = TimelineAnchor::new();
        anchor.update(untimed(true), t0, None);
        anchor.update(untimed(true), t0 + 57_500.0, None);
        // Paused at +60 s (event), read 150 ms later: the sample is
        // unchanged, so the position freezes where it had got to.
        let t = anchor.update(untimed(false), t0 + 60_150.0, Some(t0 + 60_000.0));
        assert!(!t.playing);
        assert_eq!(shown(t, t0 + 60_150.0), 60.0);
        let t = anchor.update(untimed(false), t0 + 62_650.0, None);
        assert_eq!(shown(t, t0 + 62_650.0), 60.0, "re-sampled while paused");
        // Resumed at +90 s: continues from 60 s.
        let t = anchor.update(untimed(true), t0 + 90_150.0, Some(t0 + 90_000.0));
        assert!((shown(t, t0 + 90_150.0) - 60.15).abs() < 1e-9);
        assert!((shown(t, t0 + 100_000.0) - 70.0).abs() < 1e-9);
        // A second pause, shorter than the resume tolerance.
        let t = anchor.update(untimed(false), t0 + 100_150.0, Some(t0 + 100_000.0));
        assert!((shown(t, t0 + 100_150.0) - 70.0).abs() < 1e-9);
        let t = anchor.update(untimed(true), t0 + 100_550.0, Some(t0 + 100_400.0));
        assert!((shown(t, t0 + 100_550.0) - 70.15).abs() < 1e-9);
        let t = anchor.update(untimed(true), t0 + 103_000.0, None);
        assert!((shown(t, t0 + 110_400.0) - 80.0).abs() < 1e-9);
    }

    #[test]
    fn a_pause_read_before_the_player_publishes_it_is_extrapolated_then_replaced() {
        let t0 = 1_000_000.0;
        let mut anchor = TimelineAnchor::new();
        anchor.update(raw(10.0, Some(t0), true), t0 + 100.0, None);
        // The pause event arrives first; the timeline still says 10 s at t0.
        let t = anchor.update(raw(10.0, Some(t0), false), t0 + 5_150.0, Some(t0 + 5_000.0));
        assert_eq!((t.position, t.sampled_at_ms, t.playing), (15.0, t0 + 5_000.0, false));
        // Then the player publishes where it paused: that wins.
        let t = anchor.update(raw(15.02, Some(t0 + 5_010.0), false), t0 + 5_300.0, None);
        assert_eq!((t.position, t.sampled_at_ms, t.playing), (15.02, t0 + 5_010.0, false));
        // Resumed without a new sample: from the published position.
        let t = anchor.update(raw(15.02, Some(t0 + 5_010.0), true), t0 + 8_150.0, Some(t0 + 8_000.0));
        assert_eq!((t.position, t.sampled_at_ms), (15.02, t0 + 8_000.0));
        // A pause the extrapolation runs past the end of is held at the end.
        anchor.update(raw(290.0, Some(t0 + 9_000.0), true), t0 + 9_100.0, None);
        let t = anchor.update(raw(290.0, Some(t0 + 9_000.0), false), t0 + 21_000.0, Some(t0 + 20_000.0));
        assert_eq!(t.position, 300.0);
    }

    #[test]
    fn a_short_pause_resumes_from_the_published_position() {
        let t0 = 1_000_000.0;
        let mut anchor = TimelineAnchor::new();
        anchor.update(raw(40.0, Some(t0), true), t0 + 50.0, None);
        anchor.update(raw(40.5, Some(t0 + 500.0), false), t0 + 650.0, Some(t0 + 500.0));
        // Resumed 300 ms later without a new stamp: 40.5 s at the resume.
        let t = anchor.update(raw(40.5, Some(t0 + 500.0), true), t0 + 950.0, Some(t0 + 800.0));
        assert_eq!((t.position, t.sampled_at_ms), (40.5, t0 + 800.0));
    }

    fn window(rect: Rect) -> WindowFacts {
        WindowFacts {
            class_name: "Chrome_WidgetWin_1".into(),
            visible: true,
            rect,
            monitor: Some(0),
            ..Default::default()
        }
    }

    #[test]
    fn coverage_by_fullscreen_or_maximised_windows() {
        let monitors = [Rect::from_origin_size(0, 0, 1920, 1080), Rect::from_origin_size(1920, 0, 2560, 1440)];
        // Nothing open.
        assert_eq!(coverage(&monitors, &[], None), vec![false, false]);
        // Borderless full-screen on the second monitor.
        let mut fullscreen = window(Rect::from_origin_size(1920, 0, 2560, 1440));
        fullscreen.monitor = Some(1);
        assert_eq!(coverage(&monitors, &[fullscreen.clone()], None), vec![false, true]);
        // Maximised (rect is the work area plus invisible borders) on the first.
        let mut maximised = window(Rect::new(-8, -8, 1928, 1040));
        maximised.maximized = true;
        assert_eq!(coverage(&monitors, &[maximised.clone()], None), vec![true, false]);
        // An ordinary window does not cover.
        assert_eq!(coverage(&monitors, &[window(Rect::new(100, 100, 900, 700))], None), vec![false, false]);
        // Exclusive full-screen reported by the shell.
        assert_eq!(coverage(&monitors, &[], Some(1)), vec![false, true]);
    }

    #[test]
    fn only_exclusive_fullscreen_covers_the_foreground_monitor() {
        assert!(notification_state_covers(notification_state::RUNNING_D3D_FULL_SCREEN));
        // Presentation settings are a user toggle, not a full-screen window.
        assert!(!notification_state_covers(notification_state::PRESENTATION_MODE));
        // QUNS_NOT_PRESENT, QUNS_BUSY, QUNS_ACCEPTS_NOTIFICATIONS, QUNS_QUIET_TIME, QUNS_APP.
        for state in [1, 2, 5, 6, 7] {
            assert!(!notification_state_covers(state), "{state}");
        }
    }

    #[test]
    fn session_lock_flags() {
        assert!(session_flags_locked(WTS_SESSIONSTATE_LOCK));
        assert!(!session_flags_locked(1), "WTS_SESSIONSTATE_UNLOCK");
        assert!(!session_flags_locked(-1), "WTS_SESSIONSTATE_UNKNOWN is not a lock");
    }

    #[test]
    fn raised_desktop_watches_only_progman() {
        let (progman, workerw) = (0x100, 0x200);
        // Raised (24H2+): the wallpapers are Progman's children; Explorer's
        // WorkerW is a sibling it replaces on every background change.
        assert_eq!(wallpaper_parent(progman, workerw, true), progman);
        assert_eq!(watched_shell_windows(progman, workerw, true), vec![progman]);
        // Legacy: the WorkerW is their parent.
        assert_eq!(wallpaper_parent(progman, workerw, false), workerw);
        assert_eq!(watched_shell_windows(progman, workerw, false), vec![progman, workerw]);
    }

    #[test]
    fn only_explorers_desktop_hosts_the_icon_layer() {
        let explorer = 4_242;
        assert!(is_desktop_icon_host(true, "Progman", explorer, explorer));
        assert!(is_desktop_icon_host(false, "WorkerW", explorer, explorer));
        // The legacy Explorer-style Open/Save dialog of another app.
        assert!(!is_desktop_icon_host(false, "#32770", 777, explorer));
        assert!(!is_desktop_icon_host(false, "WorkerW", 777, explorer), "another process's WorkerW");
        assert!(!is_desktop_icon_host(false, "CabinetWClass", explorer, explorer), "a File Explorer window");
        assert!(!is_desktop_icon_host(false, "WorkerW", 0, 0), "Explorer's process unknown");
    }

    #[test]
    fn overlays_and_shell_windows_never_cover() {
        let monitors = [Rect::from_origin_size(0, 0, 1920, 1080)];
        let full = Rect::from_origin_size(0, 0, 1920, 1080);
        let mut cases = Vec::new();
        let mut w = window(full);
        w.cloaked = true;
        cases.push(w);
        let mut w = window(full);
        w.minimized = true;
        w.maximized = true;
        cases.push(w);
        let mut w = window(full);
        w.visible = false;
        cases.push(w);
        let mut w = window(full);
        w.own_process = true;
        cases.push(w);
        let mut w = window(full);
        w.class_name = "WorkerW".into();
        cases.push(w);
        let mut w = window(full);
        w.is_shell_window = true;
        cases.push(w);
        let mut w = window(full);
        w.ex_style = WS_EX_LAYERED | WS_EX_TRANSPARENT;
        w.opaque_layered = true;
        cases.push(w);
        let mut w = window(full);
        w.ex_style = WS_EX_TOOLWINDOW;
        cases.push(w);
        let mut w = window(full);
        w.ex_style = WS_EX_LAYERED;
        w.opaque_layered = false;
        cases.push(w);
        for case in &cases {
            assert_eq!(coverage(&monitors, std::slice::from_ref(case), None), vec![false], "{case:?}");
        }
        // An opaque layered window does cover.
        let mut w = window(full);
        w.ex_style = WS_EX_LAYERED;
        w.opaque_layered = true;
        assert_eq!(coverage(&monitors, &[w], None), vec![true]);
    }

    #[test]
    fn attach_styles() {
        // A typical undecorated, hidden tao window.
        let popup = WS_POPUP | WS_CLIPSIBLINGS | WS_CLIPCHILDREN | WS_SYSMENU | WS_MINIMIZEBOX;
        let style = attached_style(popup);
        assert_eq!(style & WS_POPUP, 0);
        assert_eq!(style & WS_SYSMENU, 0);
        assert_ne!(style & WS_CHILD, 0);
        assert_eq!(style & WS_VISIBLE, 0, "visibility is ShowWindow's job");
        let decorated = WS_CAPTION | WS_THICKFRAME | WS_MAXIMIZEBOX | WS_VISIBLE | WS_MAXIMIZE;
        let style = attached_style(decorated);
        assert_eq!(style & (WS_CAPTION | WS_THICKFRAME | WS_MAXIMIZEBOX | WS_MAXIMIZE), 0);
        assert_ne!(style & WS_VISIBLE, 0);

        let ex = attached_ex_style(WS_EX_APPWINDOW | WS_EX_WINDOWEDGE, false);
        assert_eq!(ex & (WS_EX_APPWINDOW | WS_EX_WINDOWEDGE | WS_EX_LAYERED | WS_EX_TRANSPARENT), 0);
        assert_ne!(ex & WS_EX_NOACTIVATE, 0);
        assert_ne!(ex & WS_EX_TOOLWINDOW, 0);
        let ex = attached_ex_style(0, true);
        assert_eq!(ex & (WS_EX_LAYERED | WS_EX_TRANSPARENT), WS_EX_LAYERED | WS_EX_TRANSPARENT);

        assert!(styles_intact(attached_style(popup) | WS_VISIBLE, attached_ex_style(0, true), true));
        assert!(styles_intact(attached_style(popup) | WS_VISIBLE, attached_ex_style(0, false), false));
        assert!(!styles_intact(attached_style(popup) | WS_VISIBLE, attached_ex_style(0, false), true));
        assert!(!styles_intact(attached_style(popup), attached_ex_style(0, true), true), "hidden");
        assert!(!styles_intact(popup | WS_VISIBLE, attached_ex_style(0, true), true), "tao rewrote it");

        let original = WS_POPUP | WS_CLIPCHILDREN | WS_VISIBLE;
        let restored = restored_style(original, WS_CHILD | WS_DISABLED);
        assert_eq!(restored, WS_POPUP | WS_CLIPCHILDREN | WS_DISABLED);
    }

    #[test]
    fn rect_helpers() {
        let monitor = Rect::from_origin_size(-1920, 0, 1920, 1080);
        assert_eq!(monitor, Rect::new(-1920, 0, 0, 1080));
        assert!(monitor.contains(&monitor));
        assert!(Rect::new(-1928, -8, 8, 1088).contains(&monitor));
        assert!(!Rect::new(-1900, 0, 0, 1080).contains(&monitor));
        assert!(Rect::new(5, 5, 5, 10).is_empty());
    }

    #[test]
    fn phonetic_chunks_respect_the_limit() {
        assert!(phonetic_chunks("", 100).is_empty());
        assert_eq!(phonetic_chunks("今日は", 100), vec!["今日は"]);

        let line = "君の名前を呼んだ、遠い空の向こうで。".repeat(12);
        let chunks = phonetic_chunks(&line, 100);
        assert!(chunks.len() > 1);
        assert_eq!(chunks.concat(), line);
        for chunk in &chunks {
            assert!(chunk.encode_utf16().count() <= 100, "{chunk}");
            assert!(!chunk.is_empty());
        }
        // Cuts land after punctuation when there is some.
        assert!(chunks[0].ends_with('、') || chunks[0].ends_with('。'), "{}", chunks[0]);

        // No break opportunities: hard cut, but never inside a surrogate pair.
        let rare = "𠮷".repeat(120);
        let chunks = phonetic_chunks(&rare, 101);
        assert_eq!(chunks.concat(), rare);
        for chunk in &chunks {
            assert!(chunk.encode_utf16().count() <= 101);
            assert!(chunk.chars().all(|c| c == '𠮷'));
        }
    }
}
