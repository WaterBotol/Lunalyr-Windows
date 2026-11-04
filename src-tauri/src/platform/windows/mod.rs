//! Windows platform layer: GSMTC now playing, Progman/WorkerW wallpaper
//! attachment, desktop watcher (display changes, Explorer restarts,
//! full-screen coverage, session lock), WASAPI loopback, autostart and
//! Japanese readings.
//!
//! Exports exactly the API of `dev.rs` (see `platform/mod.rs`). Decisions
//! that do not need the OS live in `platform/logic.rs` (unit-tested on every
//! host).

mod audio;
mod autostart;
mod desktop;
mod japanese;
mod media;
mod util;
mod wallpaper;

pub use audio::start_audio;
pub use autostart::{launch_at_login_state, set_launch_at_login};
pub use desktop::start_desktop_watcher;
pub use japanese::japanese_readings;
pub use media::start_media;
pub use wallpaper::{monitors, WallpaperHost};
