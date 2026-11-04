//! One wallpaper webview per monitor, attached behind the desktop icons by
//! the platform layer (macOS: Sources/Desktop/DesktopWallpaperController.swift).
//! Everything here runs on the main (event-loop) thread.
//!
//! Rules from the platform layer: once a window is attached, no Tauri/tao API
//! that changes window state (show/hide, focus, decorations, size/position,
//! skip-taskbar...) may be called on it — tao rewrites the whole style when
//! its own flags change. Attached windows also die with Explorer's desktop
//! windows, so after an Explorer restart they are recreated, not re-attached.

use lunalyr_core::settings::DisplaySelection;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::platform::{self, MonitorInfo, WallpaperHost};

pub const LABEL_PREFIX: &str = "wallpaper-";

/// Identical for every webview in the app (WebView2 shares one browser
/// process per environment, which requires identical arguments). Tauri's
/// defaults plus CalculateNativeWinOcclusion: Chromium must not throttle a
/// wallpaper it considers occluded — Lunalyr pauses rendering itself.
#[cfg_attr(not(windows), allow(dead_code))]
pub const BROWSER_ARGS: &str =
    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,CalculateNativeWinOcclusion";

struct Wallpaper {
    label: String,
    monitor: MonitorInfo,
    hwnd: isize,
}

#[derive(Default)]
pub struct Wallpapers {
    host: Option<WallpaperHost>,
    windows: Vec<Wallpaper>,
    /// Labels get a fresh suffix whenever a window is created, so a window
    /// that is still being torn down never collides with its replacement.
    created: u64,
    /// The desktop layer could not be found on the last check (logged once).
    layer_missing: bool,
    /// When the last recreate was requested, and the current back-off.
    last_recreate: Option<std::time::Instant>,
    backoff: std::time::Duration,
}

/// Outcome of the periodic attachment check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Fine,
    /// Something had come loose and was re-attached.
    Repaired,
    /// Re-attaching failed (e.g. the window died with Explorer): recreate.
    Broken,
    /// The desktop layer is missing (Explorer not running or restarting) or
    /// a recreate happened too recently: keep the windows, check again later.
    Unavailable,
}

fn sanitize(id: &str) -> String {
    let id: String = id.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
    if id.is_empty() {
        "display".into()
    } else {
        id
    }
}

fn selected(selection: DisplaySelection) -> Vec<MonitorInfo> {
    let all = platform::monitors();
    match selection {
        DisplaySelection::All => all,
        DisplaySelection::Primary => {
            let primary: Vec<MonitorInfo> = all.iter().filter(|m| m.primary).cloned().collect();
            if primary.is_empty() {
                all.into_iter().take(1).collect()
            } else {
                primary
            }
        }
    }
}

#[cfg(windows)]
fn native_handle(window: &tauri::WebviewWindow) -> Option<isize> {
    window.hwnd().ok().map(|hwnd| hwnd.0 as isize)
}

#[cfg(not(windows))]
fn native_handle(_window: &tauri::WebviewWindow) -> Option<isize> {
    Some(0)
}

impl Wallpapers {
    fn attach(&mut self, index: usize) -> bool {
        let Some(host) = self.host.as_mut() else { return false };
        let wallpaper = &self.windows[index];
        let monitor = &wallpaper.monitor;
        match host.attach(wallpaper.hwnd, monitor) {
            Ok(()) => {
                log::info!(
                    "{} attached behind the desktop icons on {} ({}x{} at {},{}; scale {})",
                    wallpaper.label,
                    monitor.id,
                    monitor.width,
                    monitor.height,
                    monitor.x,
                    monitor.y,
                    monitor.scale
                );
                true
            }
            Err(error) => {
                log::error!("could not attach {} to the desktop: {error}", wallpaper.label);
                false
            }
        }
    }

    fn close(&mut self, app: &AppHandle, wallpaper: Wallpaper) {
        if let Some(host) = self.host.as_mut() {
            let _ = host.detach(wallpaper.hwnd);
        }
        if let Some(window) = app.get_webview_window(&wallpaper.label) {
            let _ = window.destroy();
        }
    }

    /// Matches windows to the current monitors: re-attaches existing ones
    /// (positions may have changed), closes windows of vanished monitors and
    /// creates windows for new ones. With `recreate`, every window is
    /// replaced (after an Explorer restart the old ones are gone).
    /// Returns (label, monitor id) for every live wallpaper.
    pub fn rebuild(&mut self, app: &AppHandle, selection: DisplaySelection, recreate: bool) -> Vec<(String, String)> {
        if self.host.is_none() {
            match WallpaperHost::new() {
                Ok(host) => self.host = Some(host),
                Err(error) => log::error!("wallpaper host unavailable: {error}"),
            }
        }
        if let Some(host) = self.host.as_mut() {
            if let Err(error) = host.refresh() {
                log::warn!("could not locate the desktop window layer: {error}");
            }
        }
        let monitors = selected(selection);
        log::info!(
            "rebuilding wallpapers for {} monitor(s){}",
            monitors.len(),
            if recreate { " (recreate)" } else { "" }
        );

        let old = std::mem::take(&mut self.windows);
        for wallpaper in old {
            if recreate || !monitors.iter().any(|m| m.id == wallpaper.monitor.id) {
                self.close(app, wallpaper);
            } else {
                self.windows.push(wallpaper);
            }
        }

        for monitor in monitors {
            if let Some(index) = self.windows.iter().position(|w| w.monitor.id == monitor.id) {
                self.windows[index].monitor = monitor;
                self.attach(index);
                continue;
            }
            self.created += 1;
            let label = format!("{LABEL_PREFIX}{}-{}", sanitize(&monitor.id), self.created);
            let window = match create_window(app, &label, &monitor) {
                Ok(window) => window,
                Err(error) => {
                    log::error!("could not create {label}: {error}");
                    continue;
                }
            };
            let Some(hwnd) = native_handle(&window) else {
                log::error!("{label} has no native handle");
                let _ = window.destroy();
                continue;
            };
            self.windows.push(Wallpaper { label, monitor, hwnd });
            let index = self.windows.len() - 1;
            self.attach(index);
        }
        self.windows.iter().map(|w| (w.label.clone(), w.monitor.id.clone())).collect()
    }

    /// Re-attaches any window that came loose (Explorer can recreate its
    /// desktop windows without telling us). Asks for a recreate only when
    /// the desktop layer exists but our windows cannot be attached to it,
    /// and then no more often than every 10 s (doubling to 60 s).
    pub fn verify(&mut self) -> Health {
        let Some(host) = self.host.as_mut() else { return Health::Fine };
        let loose: Vec<usize> =
            self.windows.iter().enumerate().filter(|(_, w)| !host.is_attached(w.hwnd)).map(|(i, _)| i).collect();
        if loose.is_empty() {
            self.layer_missing = false;
            self.backoff = std::time::Duration::ZERO;
            return Health::Fine;
        }
        if let Err(error) = host.refresh() {
            if !self.layer_missing {
                log::warn!("desktop layer unavailable ({error}); wallpapers wait for Explorer");
                self.layer_missing = true;
            }
            return Health::Unavailable;
        }
        self.layer_missing = false;
        log::warn!("{} wallpaper(s) came loose from the desktop; re-attaching", loose.len());
        let mut repaired = true;
        for index in loose {
            repaired &= self.attach(index);
        }
        if repaired {
            return Health::Repaired;
        }
        let now = std::time::Instant::now();
        let due = self.last_recreate.map(|last| now.duration_since(last) >= self.backoff).unwrap_or(true);
        if !due {
            return Health::Unavailable;
        }
        self.last_recreate = Some(now);
        self.backoff = (self.backoff * 2).clamp(std::time::Duration::from_secs(10), std::time::Duration::from_secs(60));
        Health::Broken
    }

    /// Detaches and closes every wallpaper before the app exits.
    pub fn shutdown(&mut self, app: &AppHandle) {
        let all = std::mem::take(&mut self.windows);
        for wallpaper in all {
            self.close(app, wallpaper);
        }
    }
}

fn create_window(app: &AppHandle, label: &str, monitor: &MonitorInfo) -> tauri::Result<tauri::WebviewWindow> {
    let scale = if monitor.scale > 0.0 { monitor.scale } else { 1.0 };
    let builder = WebviewWindowBuilder::new(app, label, WebviewUrl::App("wallpaper.html".into()))
        .title("Lunalyr wallpaper")
        .position(monitor.x as f64 / scale, monitor.y as f64 / scale)
        .inner_size(monitor.width as f64 / scale, monitor.height as f64 / scale)
        .resizable(false)
        .focused(false)
        .shadow(false)
        .background_color(tauri::window::Color(0, 0, 0, 255));
    // On Windows the platform layer shows the window once it is behind the
    // icons; elsewhere (development) it is an ordinary visible window.
    #[cfg(windows)]
    let builder = builder.decorations(false).skip_taskbar(true).visible(false).additional_browser_args(BROWSER_ARGS);
    #[cfg(not(windows))]
    let builder = builder.decorations(true).visible(true).title("Lunalyr wallpaper (development window)");
    builder.build()
}
