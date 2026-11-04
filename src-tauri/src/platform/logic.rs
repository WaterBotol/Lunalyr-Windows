//! Pure helpers for the Windows platform layer.
//!
//! Plain data in, plain data out — no Win32 types — so every decision the
//! Windows code makes (timeline anchoring, full-screen coverage, wallpaper
//! window styles, track de-duplication, text chunking for the phonetic
//! analyser) is unit-tested on any host. `platform/windows/*` gathers the
//! facts from the OS and calls these.

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
pub fn same_recording(a: &MediaInfo, b: &MediaInfo) -> bool {
    a.title == b.title && a.artist == b.artist && a.album == b.album && a.source_app == b.source_app
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
/// moment playback resumed.
const RESUME_EVENT_WINDOW_MS: f64 = 5_000.0;

/// Turns GSMTC timeline reads into [`TimelineInfo`] anchors the renderer can
/// extrapolate from (`position + (now - sampled_at) * rate` while playing).
///
/// Players behave differently, and the anchor must stay put across the
/// periodic re-samples or lyrics would jump:
/// * Most stamp `LastUpdatedTime` whenever they set the timeline (play,
///   pause, seek, track change): `position` was true at that instant.
/// * Some do not update the timeline when they resume: the stamp predates
///   the current play run, so the position is anchored at the resume
///   instant instead.
/// * Some never stamp it: the anchor is the first time this exact sample was
///   seen, and repeated identical reads keep that time.
#[derive(Debug, Clone, Default)]
pub struct TimelineAnchor {
    last_raw: Option<(u64, Option<u64>)>,
    anchor_ms: f64,
    was_playing: Option<bool>,
    playing_since_ms: Option<f64>,
}

impl TimelineAnchor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget everything (the media session changed).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// `now_ms`: wall clock of this read. `playback_event_ms`: when the most
    /// recent PlaybackInfoChanged event arrived (used as the resume instant
    /// when this read is the first to see the player playing again).
    pub fn update(&mut self, raw: RawTimeline, now_ms: f64, playback_event_ms: Option<f64>) -> TimelineInfo {
        let position = if raw.position.is_finite() { raw.position.max(0.0) } else { 0.0 };
        let rate = if raw.rate.is_finite() && raw.rate > 0.0 && raw.rate <= 16.0 { raw.rate } else { 1.0 };

        if raw.playing {
            if self.was_playing == Some(false) {
                let resumed = playback_event_ms
                    .filter(|t| t.is_finite() && *t <= now_ms && *t >= now_ms - RESUME_EVENT_WINDOW_MS)
                    .unwrap_or(now_ms);
                self.playing_since_ms = Some(resumed);
            }
        } else {
            self.playing_since_ms = None;
        }
        self.was_playing = Some(raw.playing);

        let key = (position.to_bits(), raw.last_updated_ms.map(f64::to_bits));
        let mut anchor = if self.last_raw == Some(key) {
            self.anchor_ms
        } else {
            match raw.last_updated_ms {
                Some(t) if t.is_finite() && t <= now_ms + FUTURE_TOLERANCE_MS && t >= now_ms - MAX_SAMPLE_AGE_MS => {
                    t.min(now_ms)
                }
                _ => now_ms,
            }
        };
        if raw.playing {
            if let Some(since) = self.playing_since_ms {
                if anchor < since - RESUME_TOLERANCE_MS {
                    anchor = since;
                }
            }
            if raw.duration > 0.0 {
                let projected = position + (now_ms - anchor) / 1000.0 * rate;
                if projected > raw.duration + OVERRUN_TOLERANCE_S {
                    anchor = now_ms;
                }
            }
        }
        let anchor = anchor.min(now_ms);
        self.last_raw = Some(key);
        self.anchor_ms = anchor;
        TimelineInfo { position, sampled_at_ms: anchor, playing: raw.playing, rate }
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

/// Per monitor: is it covered by a maximised window on it, by a window whose
/// rectangle contains the whole monitor, or (exclusive full-screen D3D /
/// presentation mode) is it the foreground window's monitor?
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
