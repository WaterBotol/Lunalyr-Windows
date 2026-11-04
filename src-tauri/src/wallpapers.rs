//! One wallpaper webview per monitor, attached behind the desktop icons by
//! the platform layer (macOS: Sources/Desktop/DesktopWallpaperController.swift).
//! Everything here runs on the main (event-loop) thread.

use lunalyr_core::settings::DisplaySelection;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::platform::{self, MonitorInfo, WallpaperHost};

pub const LABEL_PREFIX: &str = "wallpaper-";

struct Wallpaper {
    label: String,
    monitor: MonitorInfo,
    hwnd: isize,
}

#[derive(Default)]
pub struct Wallpapers {
    host: Option<WallpaperHost>,
    windows: Vec<Wallpaper>,
}

fn label_for(monitor: &MonitorInfo) -> String {
    let id: String = monitor
        .id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    format!("{LABEL_PREFIX}{}", if id.is_empty() { "display".into() } else { id })
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
    /// Creates, re-attaches or closes windows to match the current monitors.
    /// Returns (label, monitor id) for every live wallpaper.
    pub fn rebuild(&mut self, app: &AppHandle, selection: DisplaySelection) -> Vec<(String, String)> {
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
        log::info!("rebuilding wallpapers for {} monitor(s)", monitors.len());

        // Close windows whose monitor went away.
        let wanted: Vec<String> = monitors.iter().map(label_for).collect();
        let (keep, gone): (Vec<_>, Vec<_>) = self.windows.drain(..).partition(|w| wanted.contains(&w.label));
        for old in gone {
            if let Some(host) = self.host.as_mut() {
                let _ = host.detach(old.hwnd);
            }
            if let Some(window) = app.get_webview_window(&old.label) {
                let _ = window.destroy();
            }
        }
        self.windows = keep;

        for monitor in monitors {
            let label = label_for(&monitor);
            let existing = self.windows.iter().position(|w| w.label == label);
            let window = match app.get_webview_window(&label) {
                Some(window) => window,
                None => match create_window(app, &label, &monitor) {
                    Ok(window) => window,
                    Err(error) => {
                        log::error!("could not create {label}: {error}");
                        continue;
                    }
                },
            };
            let Some(hwnd) = native_handle(&window) else {
                log::error!("{label} has no native handle");
                continue;
            };
            if let Some(host) = self.host.as_mut() {
                match host.attach(hwnd, &monitor) {
                    Ok(()) => log::info!(
                        "{label} attached behind the desktop icons on {} ({}x{} at {},{}; scale {})",
                        monitor.id,
                        monitor.width,
                        monitor.height,
                        monitor.x,
                        monitor.y,
                        monitor.scale
                    ),
                    Err(error) => log::error!("could not attach {label} to the desktop: {error}"),
                }
            }
            match existing {
                Some(index) => {
                    self.windows[index].monitor = monitor;
                    self.windows[index].hwnd = hwnd;
                }
                None => self.windows.push(Wallpaper { label, monitor, hwnd }),
            }
        }
        self.windows.iter().map(|w| (w.label.clone(), w.monitor.id.clone())).collect()
    }

    /// Re-attaches any window Explorer detached (e.g. after it recreated its
    /// desktop windows without us noticing). Returns true if one was fixed.
    pub fn verify(&mut self) -> bool {
        let Some(host) = self.host.as_mut() else { return false };
        let broken: Vec<usize> =
            self.windows.iter().enumerate().filter(|(_, w)| !host.is_attached(w.hwnd)).map(|(i, _)| i).collect();
        if broken.is_empty() {
            return false;
        }
        log::warn!("{} wallpaper(s) lost their desktop parent; re-attaching", broken.len());
        if let Err(error) = host.refresh() {
            log::warn!("desktop layer refresh failed: {error}");
        }
        for index in broken {
            let wallpaper = &self.windows[index];
            if let Err(error) = host.attach(wallpaper.hwnd, &wallpaper.monitor) {
                log::error!("re-attach of {} failed: {error}", wallpaper.label);
            }
        }
        true
    }

    /// Detaches every wallpaper before the app exits.
    pub fn shutdown(&mut self, app: &AppHandle) {
        for wallpaper in self.windows.drain(..) {
            if let Some(host) = self.host.as_mut() {
                let _ = host.detach(wallpaper.hwnd);
            }
            if let Some(window) = app.get_webview_window(&wallpaper.label) {
                let _ = window.destroy();
            }
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
    let builder = builder.decorations(false).skip_taskbar(true).visible(false);
    #[cfg(not(windows))]
    let builder = builder.decorations(true).visible(true).title("Lunalyr wallpaper (development window)");
    builder.build()
}
