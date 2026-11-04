//! Windows platform layer: GSMTC now playing, Progman/WorkerW wallpaper
//! attachment, desktop watcher (display changes, Explorer restarts,
//! full-screen coverage, session lock), WASAPI loopback and autostart.
//!
//! STUB: the real implementation is being written in submodules
//! (media.rs, wallpaper.rs, desktop.rs, audio.rs, autostart.rs,
//! japanese.rs). Until then these placeholders keep the crate compiling.

use super::{AudioHandle, EventSink, MonitorInfo, PlatformError, PlatformResult};

pub fn start_media(_sink: EventSink) -> PlatformResult<()> {
    Err(PlatformError::msg("GSMTC media source not implemented yet"))
}

pub fn start_desktop_watcher(_sink: EventSink) -> PlatformResult<()> {
    Ok(())
}

pub fn monitors() -> Vec<MonitorInfo> {
    Vec::new()
}

pub struct WallpaperHost;

impl WallpaperHost {
    pub fn new() -> PlatformResult<Self> {
        Ok(WallpaperHost)
    }
    pub fn refresh(&mut self) -> PlatformResult<()> {
        Ok(())
    }
    pub fn attach(&mut self, _hwnd: isize, _monitor: &MonitorInfo) -> PlatformResult<()> {
        Err(PlatformError::msg("wallpaper attach not implemented yet"))
    }
    pub fn detach(&mut self, _hwnd: isize) -> PlatformResult<()> {
        Ok(())
    }
    pub fn is_attached(&self, _hwnd: isize) -> bool {
        false
    }
}

pub fn start_audio(_sink: EventSink) -> PlatformResult<AudioHandle> {
    Err(PlatformError::msg("WASAPI loopback not implemented yet"))
}

pub fn set_launch_at_login(_enabled: bool) -> PlatformResult<()> {
    Ok(())
}

pub fn japanese_readings(_text: &str) -> Option<Vec<(String, String)>> {
    None
}
