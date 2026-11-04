//! Notification-area icon and menu (macOS: the menu-bar panel, MenuBarView.swift).

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Wry};

pub struct Tray {
    now_playing: MenuItem<Wry>,
    pause: CheckMenuItem<Wry>,
    icon: TrayIcon<Wry>,
}

pub fn create(app: &AppHandle) -> tauri::Result<Tray> {
    let now_playing = MenuItem::with_id(app, "now-playing", "Nothing playing", false, None::<&str>)?;
    let pause = CheckMenuItem::with_id(app, "pause", "Pause wallpaper", true, false, None::<&str>)?;
    let reload = MenuItem::with_id(app, "reload", "Reload lyrics", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Lunalyr", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &now_playing,
            &PredefinedMenuItem::separator(app)?,
            &pause,
            &reload,
            &settings,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    let mut builder = TrayIconBuilder::with_id("lunalyr")
        .tooltip("Lunalyr")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "pause" => crate::toggle_user_pause(app),
            "reload" => crate::reload_lyrics_now(app),
            "settings" => crate::show_settings(app),
            "quit" => crate::quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                crate::show_settings(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    let icon = builder.build(app)?;
    Ok(Tray { now_playing, pause, icon })
}

impl Tray {
    pub fn update(&self, now_playing: &str, paused: bool) {
        let label: String = if now_playing.chars().count() > 60 {
            now_playing.chars().take(59).chain(std::iter::once('…')).collect()
        } else {
            now_playing.to_string()
        };
        let _ = self.now_playing.set_text(&label);
        let _ = self.pause.set_checked(paused);
        let _ = self.icon.set_tooltip(Some(format!("Lunalyr — {label}")));
    }
}
