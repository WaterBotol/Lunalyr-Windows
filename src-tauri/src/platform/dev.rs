//! Development stand-in for non-Windows hosts (macOS/Linux).
//!
//! Lets the Tauri shell, IPC, lyric lookup and renderer run on a Mac:
//! a fake media session "plays" one track on a loop, wallpaper windows are
//! ordinary windows, and audio is a synthetic signal. Configure the track
//! with LUNALYR_DEV_TITLE, LUNALYR_DEV_ARTIST, LUNALYR_DEV_ALBUM and
//! LUNALYR_DEV_DURATION (seconds) to exercise real lyric providers.
//! Nothing here ships in the Windows build.

use std::thread;
use std::time::Duration;

use lunalyr_core::contract::AudioFrame;

use super::{
    now_ms, AudioHandle, DesktopEvent, EventSink, MediaEvent, MediaInfo, MonitorInfo, PlatformEvent, PlatformResult,
    TimelineInfo,
};

fn env_or(name: &str, fallback: &str) -> String {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| fallback.to_string())
}

pub fn start_media(sink: EventSink) -> PlatformResult<()> {
    let info = MediaInfo {
        title: env_or("LUNALYR_DEV_TITLE", "Lunalyr development track"),
        artist: env_or("LUNALYR_DEV_ARTIST", "Lunalyr"),
        album: std::env::var("LUNALYR_DEV_ALBUM").ok().filter(|v| !v.is_empty()),
        duration: env_or("LUNALYR_DEV_DURATION", "180").parse().unwrap_or(180.0),
        source_app: "dev".into(),
    };
    thread::Builder::new()
        .name("dev-media".into())
        .spawn(move || {
            let started = now_ms();
            let _ = sink.send(PlatformEvent::Media(MediaEvent::Track(Some(info.clone()))));
            let _ = sink.send(PlatformEvent::Media(MediaEvent::Artwork {
                title: info.title.clone(),
                artist: info.artist.clone(),
                bytes: None,
            }));
            loop {
                let elapsed = (now_ms() - started) / 1000.0;
                let duration = info.duration.max(1.0);
                let position = elapsed % duration;
                let event = PlatformEvent::Media(MediaEvent::Timeline(TimelineInfo {
                    position,
                    sampled_at_ms: now_ms(),
                    playing: true,
                    rate: 1.0,
                }));
                if sink.send(event).is_err() {
                    return;
                }
                thread::sleep(Duration::from_secs(5));
            }
        })
        .map_err(|e| super::PlatformError::msg(e.to_string()))?;
    Ok(())
}

pub fn start_desktop_watcher(sink: EventSink) -> PlatformResult<()> {
    let _ = sink.send(PlatformEvent::Desktop(DesktopEvent::Coverage(vec![("dev".into(), false)])));
    Ok(())
}

pub fn monitors() -> Vec<MonitorInfo> {
    vec![MonitorInfo { id: "dev".into(), x: 80, y: 80, width: 1280, height: 720, scale: 1.0, primary: true }]
}

/// On non-Windows hosts wallpaper windows stay ordinary windows.
pub struct WallpaperHost;

impl WallpaperHost {
    pub fn new() -> PlatformResult<Self> {
        Ok(WallpaperHost)
    }
    pub fn refresh(&mut self) -> PlatformResult<()> {
        Ok(())
    }
    pub fn attach(&mut self, _hwnd: isize, _monitor: &MonitorInfo) -> PlatformResult<()> {
        Ok(())
    }
    pub fn detach(&mut self, _hwnd: isize) -> PlatformResult<()> {
        Ok(())
    }
    pub fn is_attached(&self, _hwnd: isize) -> bool {
        true
    }
}

/// Synthetic spectrum so visualisers can be developed without capture.
pub fn start_audio(sink: EventSink) -> PlatformResult<AudioHandle> {
    let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let flag = running.clone();
    thread::Builder::new()
        .name("dev-audio".into())
        .spawn(move || {
            let _ = sink.send(PlatformEvent::AudioStatus("Synthetic audio (development build)".into()));
            let start = now_ms();
            while flag.load(std::sync::atomic::Ordering::Relaxed) {
                let t = ((now_ms() - start) / 1000.0) as f32;
                let beat = (t * 2.0 * std::f32::consts::PI * 2.0).sin().max(0.0).powi(8);
                let bands: Vec<f32> = (0..32)
                    .map(|i| {
                        let f = i as f32 / 31.0;
                        (0.55 - 0.4 * f + 0.25 * (t * (1.0 + f * 3.0)).sin() + 0.3 * beat * (1.0 - f)).clamp(0.0, 1.0)
                    })
                    .collect();
                let frame = AudioFrame {
                    level: 0.5 + 0.3 * beat,
                    bass: bands[0..6].iter().sum::<f32>() / 6.0,
                    mid: bands[6..18].iter().sum::<f32>() / 12.0,
                    treble: bands[18..32].iter().sum::<f32>() / 14.0,
                    beat,
                    flux: beat,
                    centroid: 0.35 + 0.1 * (t * 0.5).sin(),
                    bands,
                    pcm: None,
                    sample_rate: 48_000,
                };
                if sink.send(PlatformEvent::Audio(frame)).is_err() {
                    return;
                }
                thread::sleep(Duration::from_millis(33));
            }
        })
        .map_err(|e| super::PlatformError::msg(e.to_string()))?;
    Ok(AudioHandle::new(move || running.store(false, std::sync::atomic::Ordering::Relaxed)))
}

pub fn set_launch_at_login(_enabled: bool) -> PlatformResult<()> {
    Ok(())
}

/// Unknown on development hosts: the persisted setting is kept.
pub fn launch_at_login_state() -> Option<bool> {
    None
}

/// Japanese readings need the Windows phonetic analyser; none here.
pub fn japanese_readings(_text: &str) -> Option<Vec<(String, String)>> {
    None
}
