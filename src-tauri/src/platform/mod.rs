//! Platform layer.
//!
//! The app shell (lib.rs / controller.rs) talks to the OS only through the
//! items re-exported here. `windows/` implements them with GSMTC, the
//! Progman/WorkerW wallpaper layer and WASAPI; `dev.rs` is a stand-in for
//! macOS/Linux so the shell, IPC and renderer can be developed and smoke
//! tested without Windows. Both modules must export exactly the same API:
//!
//! ```ignore
//! fn start_media(sink: EventSink) -> PlatformResult<()>;
//! fn start_desktop_watcher(sink: EventSink) -> PlatformResult<()>;
//! fn monitors() -> Vec<MonitorInfo>;
//! struct WallpaperHost;
//! impl WallpaperHost {
//!     fn new() -> PlatformResult<Self>;
//!     /// Re-discover Progman/WorkerW (after Explorer restarts).
//!     fn refresh(&mut self) -> PlatformResult<()>;
//!     /// Put `hwnd` behind the desktop icons, covering `monitor`. Main thread.
//!     fn attach(&mut self, hwnd: isize, monitor: &MonitorInfo) -> PlatformResult<()>;
//!     /// Undo attach (make it a hidden top-level window again). Main thread.
//!     fn detach(&mut self, hwnd: isize) -> PlatformResult<()>;
//!     /// True if `hwnd` is still correctly attached (parent alive, z-order OK).
//!     fn is_attached(&self, hwnd: isize) -> bool;
//! }
//! fn start_audio(sink: EventSink) -> PlatformResult<AudioHandle>;  // M2
//! fn set_launch_at_login(enabled: bool) -> PlatformResult<()>;
//! fn launch_at_login_state() -> Option<bool>;  // None: unknown
//! fn japanese_readings(text: &str) -> Option<Vec<(String, String)>>;  // M2
//! ```

use lunalyr_core::contract::AudioFrame;

/// Pure, host-testable decisions used by the Windows layer.
pub mod logic;

#[cfg(windows)]
pub mod windows;
#[cfg(windows)]
pub use self::windows::*;

#[cfg(not(windows))]
pub mod dev;
#[cfg(not(windows))]
pub use self::dev::*;

/// Now-playing metadata as reported by the OS media session.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaInfo {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    /// Seconds; 0 when unknown (GSMTC EndTime - StartTime).
    pub duration: f64,
    /// GSMTC `SourceAppUserModelId`, e.g. "Spotify.exe".
    pub source_app: String,
}

/// A playback position sample. The controller extrapolates from it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimelineInfo {
    /// Seconds into the track at `sampled_at_ms`.
    pub position: f64,
    /// Unix epoch milliseconds when `position` was true (GSMTC LastUpdatedTime
    /// when the player provides it, otherwise the time it was read).
    pub sampled_at_ms: f64,
    pub playing: bool,
    pub rate: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MediaEvent {
    /// The current session's track changed. `None`: no media session.
    Track(Option<MediaInfo>),
    /// Position / play state changed or was re-sampled.
    Timeline(TimelineInfo),
    /// Encoded artwork (PNG/JPEG/BMP bytes) for the track with this title and
    /// artist; `None` when the session has no thumbnail.
    Artwork { title: String, artist: String, bytes: Option<Vec<u8>> },
}

/// A monitor in physical pixels (virtual-screen coordinates).
#[derive(Debug, Clone, PartialEq)]
pub struct MonitorInfo {
    /// Stable id for this session, e.g. `\\.\DISPLAY1`.
    pub id: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// 1.0 = 96 DPI.
    pub scale: f64,
    pub primary: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DesktopEvent {
    /// Monitors added/removed/moved/rescaled, or work area changed. Rebuild.
    DisplaysChanged,
    /// Explorer restarted (TaskbarCreated) or our WorkerW parent was destroyed.
    ShellRestarted,
    /// For each monitor id: is it covered by a full-screen or maximised app?
    Coverage(Vec<(String, bool)>),
    /// Workstation locked (true) / unlocked (false). Pause while locked.
    SessionLocked(bool),
    /// Every display of this session is off (true) or on again (false).
    DisplaysAsleep(bool),
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlatformEvent {
    Media(MediaEvent),
    Desktop(DesktopEvent),
    Audio(AudioFrame),
    /// Audio capture status for the settings window ("Receiving audio", errors).
    AudioStatus(String),
}

pub type EventSink = tokio::sync::mpsc::UnboundedSender<PlatformEvent>;

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("{0}")]
    Message(String),
    #[cfg(windows)]
    #[error("Windows error: {0}")]
    Windows(#[from] ::windows::core::Error),
}

pub type PlatformResult<T> = Result<T, PlatformError>;

impl PlatformError {
    pub fn msg(text: impl Into<String>) -> Self {
        PlatformError::Message(text.into())
    }
}

/// Keeps audio capture alive; dropping it stops capture.
pub struct AudioHandle {
    pub(crate) stop: Option<Box<dyn FnOnce() + Send>>,
}

impl AudioHandle {
    pub fn new(stop: impl FnOnce() + Send + 'static) -> Self {
        AudioHandle { stop: Some(Box::new(stop)) }
    }
}

impl Drop for AudioHandle {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop();
        }
    }
}

/// Wall-clock now in Unix epoch milliseconds.
pub fn now_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
}
