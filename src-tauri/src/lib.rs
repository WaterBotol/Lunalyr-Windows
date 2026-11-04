//! Lunalyr for Windows — Tauri application shell.
//!
//! * `controller` — the app's state machine (pure, unit-tested).
//! * `platform`   — GSMTC, desktop wallpaper layer, WASAPI (Windows) or a
//!   development stand-in (other hosts).
//! * `wallpapers` — one webview per monitor behind the desktop icons.
//! * `lyrics`     — adapter to lunalyr-core's providers, cache and palette.
//! * `tray`       — notification-area icon and menu.
//!
//! This file wires them together: it executes the controller's effects
//! (emit events, run lookups, save settings, rebuild windows) and exposes
//! the commands listed in docs/CONTRACT.md.

pub mod controller;
mod dsp;
mod lyrics;
pub mod platform;
mod tray;
mod wallpapers;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use lunalyr_core::contract::Snapshot;
use lunalyr_core::model::SourceId;
use lunalyr_core::settings::Settings;
use serde_json::Value;
use tauri::{AppHandle, Emitter, EventTarget, Manager, RunEvent, State, WebviewUrl, WebviewWindowBuilder};

use controller::{Controller, Effect, Target};
use platform::{now_ms, AudioHandle, EventSink, PlatformEvent};

pub struct AppState {
    controller: Mutex<Controller>,
    lyrics: Arc<lyrics::Lyrics>,
    settings_path: PathBuf,
    wallpapers: Mutex<wallpapers::Wallpapers>,
    audio: Mutex<Option<AudioHandle>>,
    tray: Mutex<Option<tray::Tray>>,
    sink: EventSink,
    /// A queued wallpaper rebuild (`Some(recreate)`); requests coalesce.
    rebuild_pending: Mutex<Option<bool>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic elsewhere must not take the whole app down with poisoned locks.
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Like [`lock`] but gives up instead of waiting (used by periodic checks so
/// they can never re-enter a long main-thread operation holding the lock).
fn try_lock<T>(mutex: &Mutex<T>) -> Option<MutexGuard<'_, T>> {
    match mutex.try_lock() {
        Ok(guard) => Some(guard),
        Err(std::sync::TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
        Err(std::sync::TryLockError::WouldBlock) => None,
    }
}

fn state(app: &AppHandle) -> Arc<AppState> {
    app.state::<Arc<AppState>>().inner().clone()
}

/// Executes controller effects. Safe to call from any thread.
fn run_effects(app: &AppHandle, effects: Vec<Effect>) {
    let shared = state(app);
    for effect in effects {
        match effect {
            Effect::Emit { event, payload, target } => {
                let result = match target {
                    Target::All => app.emit(event, payload),
                    Target::Window(label) => app.emit_to(EventTarget::webview_window(label), event, payload),
                    Target::Wallpapers => app.emit_filter(event, payload, |target| {
                        matches!(target, EventTarget::WebviewWindow { label } if label.starts_with(wallpapers::LABEL_PREFIX))
                    }),
                };
                if let Err(error) = result {
                    log::warn!("emit {event} failed: {error}");
                }
            }
            Effect::Lookup { generation, query, force } => {
                let app = app.clone();
                let shared = shared.clone();
                tauri::async_runtime::spawn(async move {
                    let sources = lock(&shared.controller).settings().sources.clone();
                    let outcome = shared.lyrics.lookup(&query, &sources, force).await;
                    match &outcome.result {
                        Some(result) => log::info!(
                            "lyrics for \"{}\" by {}: {} ({} lines; {} candidates{}{})",
                            query.title,
                            query.artist,
                            result.provenance().label(),
                            result.line_count(),
                            outcome.candidates.len(),
                            if outcome.cached { ", cached" } else { "" },
                            if outcome.pinned { ", saved choice" } else { "" },
                        ),
                        None => {
                            log::info!("no lyrics for \"{}\" by {} ({:?})", query.title, query.artist, outcome.states)
                        }
                    }
                    let effects = lock(&shared.controller).on_lookup_finished(
                        generation,
                        outcome,
                        shared.lyrics.as_ref(),
                        now_ms(),
                    );
                    run_effects(&app, effects);
                });
            }
            Effect::Unpin { query } => shared.lyrics.unpin(&query),
            Effect::ExtractPalette { generation, bytes } => {
                let app = app.clone();
                let shared = shared.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    // Undecodable artwork gives the fallback palette, not the
                    // previous song's colours.
                    let palette = shared.lyrics.palette(&bytes);
                    let effects = lock(&shared.controller).on_palette(generation, palette);
                    run_effects(&app, effects);
                });
            }
            Effect::ComputeReadings { generation, lines } => {
                let app = app.clone();
                let shared = shared.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    let (display, tones) = {
                        let controller = lock(&shared.controller);
                        (controller.settings().readings, controller.settings().tone_marks)
                    };
                    let readings = shared.lyrics.readings(&lines, display, tones);
                    let effects = lock(&shared.controller).on_readings(generation, readings);
                    run_effects(&app, effects);
                });
            }
            Effect::SaveSettings => {
                let settings = lock(&shared.controller).settings().clone();
                if let Err(error) = settings.save(&shared.settings_path) {
                    log::error!("could not save settings: {error}");
                }
            }
            Effect::RebuildWallpapers { recreate } => schedule_rebuild(app, recreate),
            Effect::SetLaunchAtLogin(enabled) => {
                if let Err(error) = platform::set_launch_at_login(enabled) {
                    log::error!("launch at login: {error}");
                    // Show what is really configured, as macOS reverts the toggle.
                    let actual = platform::launch_at_login_state().unwrap_or(!enabled);
                    let effects = lock(&shared.controller)
                        .launch_at_login_failed(actual, format!("Launch at login could not be changed: {error}"));
                    run_effects(app, effects);
                }
            }
            Effect::SetAudio(enabled) => {
                let mut audio = lock(&shared.audio);
                if enabled && audio.is_none() {
                    match platform::start_audio(shared.sink.clone()) {
                        Ok(handle) => *audio = Some(handle),
                        Err(error) => {
                            log::error!("audio capture: {error}");
                            let _ = app.emit("lunalyr://audio-status", format!("Audio capture unavailable: {error}"));
                        }
                    }
                } else if !enabled {
                    *audio = None;
                }
            }
            Effect::Tray { now_playing, paused } => {
                let app = app.clone();
                let _ = app.clone().run_on_main_thread(move || {
                    if let Some(tray) = lock(&state(&app).tray).as_ref() {
                        tray.update(&now_playing, paused);
                    }
                });
            }
        }
    }
}

/// Queues `task` on the event loop and never runs it inline.
///
/// `run_on_main_thread` runs its closure immediately when called on the main
/// thread, and synchronous commands run on the main thread inside a WebView2
/// callback: creating a webview there deadlocks (WebView2 cannot complete
/// its creation while one of its own handlers is on the stack). Posting from
/// another thread always goes through tao's event loop instead.
fn defer_to_main(app: &AppHandle, what: &'static str, task: impl FnOnce(&AppHandle) + Send + 'static) {
    let app = app.clone();
    let spawned = std::thread::Builder::new().name("lunalyr-defer".into()).spawn(move || {
        let handle = app.clone();
        if let Err(error) = app.run_on_main_thread(move || task(&handle)) {
            log::error!("could not schedule {what}: {error}");
        }
    });
    if let Err(error) = spawned {
        log::error!("could not schedule {what}: {error}");
    }
}

/// Rebuilds wallpaper windows on the main thread, then tells the controller.
/// Requests made while one is queued are merged (`recreate` wins).
fn schedule_rebuild(app: &AppHandle, recreate: bool) {
    let shared = state(app);
    {
        let mut pending = lock(&shared.rebuild_pending);
        if let Some(queued) = pending.as_mut() {
            *queued |= recreate;
            return;
        }
        *pending = Some(recreate);
    }
    defer_to_main(app, "wallpaper rebuild", |handle| {
        let shared = state(handle);
        let Some(recreate) = lock(&shared.rebuild_pending).take() else { return };
        let selection = lock(&shared.controller).settings().displays;
        let windows = lock(&shared.wallpapers).rebuild(handle, selection, recreate);
        let effects = lock(&shared.controller).set_windows(windows);
        run_effects(handle, effects);
    });
}

/// Shows (or creates) the settings window. Always deferred: see
/// [`defer_to_main`].
pub(crate) fn show_settings(app: &AppHandle) {
    defer_to_main(app, "the settings window", show_settings_now);
}

fn show_settings_now(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let builder = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("settings.html".into()))
        .title("Lunalyr Settings")
        .inner_size(980.0, 720.0)
        .min_inner_size(720.0, 520.0)
        .center();
    // Must match the wallpapers' arguments (one shared WebView2 browser process).
    #[cfg(windows)]
    let builder = builder.additional_browser_args(wallpapers::BROWSER_ARGS);
    let built = builder.build();
    if let Err(error) = built {
        log::error!("could not open settings: {error}");
    }
}

pub(crate) fn toggle_user_pause(app: &AppHandle) {
    let shared = state(app);
    let effects = {
        let mut controller = lock(&shared.controller);
        let next = !controller.user_paused();
        controller.set_user_paused(next)
    };
    run_effects(app, effects);
}

pub(crate) fn reload_lyrics_now(app: &AppHandle) {
    let effects = lock(&state(app).controller).reload_lyrics();
    run_effects(app, effects);
}

pub(crate) fn quit(app: &AppHandle) {
    let shared = state(app);
    *lock(&shared.audio) = None;
    lock(&shared.wallpapers).shutdown(app);
    app.exit(0);
}

// ---- commands ---------------------------------------------------------------

#[tauri::command]
fn get_snapshot(window: tauri::WebviewWindow, state: State<'_, Arc<AppState>>) -> Snapshot {
    // Logged: proves the page loaded and IPC works (CI smoke evidence).
    log::info!("{} loaded and requested a snapshot", window.label());
    lock(&state.controller).snapshot(Some(window.label()), now_ms())
}

#[tauri::command]
fn update_settings(app: AppHandle, state: State<'_, Arc<AppState>>, patch: Value) -> Result<Settings, String> {
    let (settings, effects) = lock(&state.controller).update_settings(&patch, state.lyrics.as_ref(), now_ms())?;
    run_effects(&app, effects);
    Ok(settings)
}

#[tauri::command]
fn reload_lyrics(app: AppHandle) {
    reload_lyrics_now(&app);
}

#[tauri::command]
fn select_candidate(app: AppHandle, state: State<'_, Arc<AppState>>, source: SourceId) -> Result<(), String> {
    let effects = lock(&state.controller).select_candidate(source, state.lyrics.as_ref(), now_ms())?;
    run_effects(&app, effects);
    Ok(())
}

#[tauri::command]
fn forget_selection(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let effects = lock(&state.controller).forget_selection()?;
    run_effects(&app, effects);
    Ok(())
}

#[tauri::command]
fn set_song_offset(app: AppHandle, state: State<'_, Arc<AppState>>, seconds: f64) -> Result<(), String> {
    let effects = lock(&state.controller).set_song_offset(seconds, now_ms())?;
    run_effects(&app, effects);
    Ok(())
}

#[tauri::command]
fn import_lyrics(app: AppHandle, state: State<'_, Arc<AppState>>, text: String) -> Result<String, String> {
    if text.len() > 2_000_000 {
        return Err("That file is too large to be a lyric file.".into());
    }
    let duration = lock(&state.controller).query().map(|q| q.duration).unwrap_or(0.0);
    let parsed = state.lyrics.parse_import(&text, duration);
    let (message, effects) = lock(&state.controller).import_lyrics(parsed, state.lyrics.as_ref(), now_ms())?;
    run_effects(&app, effects);
    Ok(message)
}

#[tauri::command]
fn open_settings(app: AppHandle) {
    show_settings(&app);
}

/// Opens one of a few fixed links in the default browser, or an installed
/// licence file in its default app. Webviews get no general "open URL"
/// permission: only these targets exist.
#[tauri::command]
fn open_link(app: AppHandle, target: String) -> Result<(), String> {
    let destination = match target.as_str() {
        "project" => "https://github.com/WaterBotol/Lunalyr-Windows".to_string(),
        "mac" => "https://github.com/WaterBotol/Lunalyr".to_string(),
        "upstream" => "https://github.com/AkkiCode06/verso".to_string(),
        other => {
            let file = other.strip_prefix("license:").ok_or("Unknown link.")?;
            let allowed = [
                "LICENSE.txt",
                "THIRD_PARTY_NOTICES.md",
                "Manrope-OFL.txt",
                "Wavefield-LICENSE.txt",
                "Wavefield-Web-Dependencies-LICENSE.txt",
            ];
            if !allowed.contains(&file) {
                return Err("Unknown licence file.".into());
            }
            let path = app.path().resource_dir().map_err(|e| e.to_string())?.join("licenses").join(file);
            if !path.exists() {
                return Err(format!("{file} is not installed with this build."));
            }
            path.to_string_lossy().into_owned()
        }
    };
    open::that_detached(&destination).map_err(|error| format!("Could not open {destination}: {error}"))
}

#[tauri::command]
fn set_user_paused(app: AppHandle, state: State<'_, Arc<AppState>>, paused: bool) {
    let effects = lock(&state.controller).set_user_paused(paused);
    run_effects(&app, effects);
}

// ---- startup ------------------------------------------------------------------

fn init_logging(dir: Option<PathBuf>) {
    let mut builder = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    if let Some(dir) = dir {
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("lunalyr.log");
        if std::fs::metadata(&path).map(|m| m.len() > 2_000_000).unwrap_or(false) {
            let _ = std::fs::rename(&path, dir.join("lunalyr.old.log"));
        }
        if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            builder.target(env_logger::Target::Pipe(Box::new(file)));
        }
    }
    let _ = builder.try_init();
    // Release builds abort on panic; record where first (the log is the only
    // diagnostic a user can send).
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("panicked: {info}");
        default_hook(info);
    }));
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();
    init_logging(app.path().app_log_dir().ok());
    let config_dir = app.path().app_config_dir()?;
    let cache_dir = app.path().app_cache_dir()?.join("lyrics");
    let settings_path = config_dir.join("settings.json");
    let first_run = !settings_path.exists();
    let mut settings = Settings::load(&settings_path);
    log::info!("Lunalyr {} starting (settings: {})", app.package_info().version, settings_path.display());
    // The Run key is the truth for launch at login (the user may have changed
    // it in Task Manager's Startup apps).
    if let Some(actual) = platform::launch_at_login_state() {
        if actual != settings.launch_at_login {
            log::info!("launch at login is {} in Windows; updating the setting", if actual { "on" } else { "off" });
            settings.launch_at_login = actual;
            let _ = settings.save(&settings_path);
        } else if actual {
            // Re-write the command: a reinstall may have moved the executable.
            if let Err(error) = platform::set_launch_at_login(true) {
                log::warn!("could not refresh the launch-at-login command: {error}");
            }
        }
    }

    let (sink, mut events) = tokio::sync::mpsc::unbounded_channel::<PlatformEvent>();
    let shared = Arc::new(AppState {
        controller: Mutex::new(Controller::new(settings.clone(), app.package_info().version.to_string())),
        lyrics: Arc::new(lyrics::Lyrics::new(cache_dir)),
        settings_path: settings_path.clone(),
        wallpapers: Mutex::new(wallpapers::Wallpapers::default()),
        audio: Mutex::new(None),
        tray: Mutex::new(None),
        sink: sink.clone(),
        rebuild_pending: Mutex::new(None),
    });
    app.manage(shared.clone());
    if first_run {
        let _ = settings.save(&settings_path);
    }

    match tray::create(&handle) {
        Ok(tray) => *lock(&shared.tray) = Some(tray),
        Err(error) => log::error!("tray icon unavailable: {error}"),
    }

    // Platform events → controller → effects.
    let events_app = handle.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = events.recv().await {
            let shared = state(&events_app);
            match &event {
                PlatformEvent::Media(platform::MediaEvent::Track(Some(info))) => log::info!(
                    "now playing: \"{}\" by {} ({:.0} s, {})",
                    info.title,
                    info.artist,
                    info.duration,
                    info.source_app
                ),
                PlatformEvent::Media(platform::MediaEvent::Track(None)) => log::info!("no media session"),
                PlatformEvent::Desktop(desktop) => log::info!("desktop: {desktop:?}"),
                _ => {}
            }
            let effects = lock(&shared.controller).on_platform(event, now_ms());
            run_effects(&events_app, effects);
        }
    });

    if let Err(error) = platform::start_desktop_watcher(sink.clone()) {
        log::error!("desktop watcher unavailable: {error}");
    }
    if let Err(error) = platform::start_media(sink) {
        log::error!("now-playing source unavailable: {error}");
    }

    schedule_rebuild(&handle, false);

    // Watchdog: Explorer can recreate its desktop windows without telling us.
    let watchdog = handle.clone();
    std::thread::Builder::new().name("wallpaper-watchdog".into()).spawn(move || loop {
        std::thread::sleep(Duration::from_secs(2));
        let app = watchdog.clone();
        let _ = watchdog.run_on_main_thread(move || {
            let shared = state(&app);
            // Skip this tick if a rebuild holds the lock (never wait for it).
            let Some(mut wallpapers) = try_lock(&shared.wallpapers) else { return };
            let health = wallpapers.verify();
            drop(wallpapers);
            match health {
                wallpapers::Health::Fine | wallpapers::Health::Unavailable => {}
                wallpapers::Health::Repaired => {
                    // Re-send render state: a re-attached webview may have reloaded.
                    let windows = lock(&shared.controller).windows().to_vec();
                    let effects = lock(&shared.controller).set_windows(windows);
                    run_effects(&app, effects);
                }
                wallpapers::Health::Broken => schedule_rebuild(&app, true),
            }
        });
    })?;

    let autostart = std::env::args().any(|a| a == "--autostart");
    if first_run && !autostart {
        show_settings(&handle);
    }
    Ok(())
}

pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(windows)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        show_settings(app);
    }));
    let app = builder
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            update_settings,
            reload_lyrics,
            select_candidate,
            forget_selection,
            set_song_offset,
            import_lyrics,
            open_settings,
            set_user_paused,
            open_link,
        ])
        .setup(setup)
        .build(tauri::generate_context!())
        .expect("error while building Lunalyr");

    app.run(|app, event| {
        if let RunEvent::ExitRequested { api, code, .. } = event {
            // Closing the settings window must not quit a tray app; only an
            // explicit exit (tray Quit → app.exit(0)) carries a code.
            if code.is_none() {
                api.prevent_exit();
            } else {
                lock(&state(app).wallpapers).shutdown(app);
            }
        }
    });
}
