//! Puts the wallpaper webviews behind the desktop icons, and lists monitors.
//!
//! Explorer has two desktop layouts:
//!
//! * **Legacy** (Windows 10, Windows 11 before 24H2). Sending Progman the
//!   undocumented message 0x052C makes Explorer split the desktop into a
//!   top-level WorkerW that holds SHELLDLL_DefView (the icons) and, right
//!   behind it in z-order, an empty top-level WorkerW that paints the
//!   wallpaper. Our window becomes a child of that empty WorkerW.
//!   *Input:* the icon layer is a separate top-level window in front of the
//!   wallpaper WorkerW and covers the whole virtual screen, so every click,
//!   rubber-band drag, icon drag, right-click and key goes to the shell. The
//!   wallpaper is not layered here (layered children need a Windows 8
//!   manifest and buy nothing); it relies on that z-order, plus being
//!   disabled (below).
//! * **Raised desktop** (Windows 11 24H2+). Progman is created with
//!   WS_EX_NOREDIRECTIONBITMAP; SHELLDLL_DefView is a WS_EX_LAYERED *child* of
//!   Progman that draws mostly transparent, and a WorkerW child below it
//!   paints the wallpaper. Microsoft's guidance for apps that raise the
//!   desktop: insert your own WS_EX_LAYERED child (alpha 255) below DefView
//!   and above that WorkerW, which must stay the last child.
//!   *Input:* DefView is above us but layered, so hit-testing may fall
//!   through its transparent pixels. Our window is LAYERED **and**
//!   TRANSPARENT, which hit-testing skips entirely (with its children), so a
//!   click reaches exactly what it would reach without a wallpaper: DefView,
//!   or Explorer's own WorkerW below.
//!
//! In both layouts the window is WS_EX_NOACTIVATE + WS_EX_TOOLWINDOW and is
//! **disabled** with EnableWindow: a disabled window gets no keyboard or
//! mouse input, and SetFocus fails for it and every descendant (including
//! WebView2's child HWNDs), so the webview can never take keyboard focus away
//! from the desktop. We set every style ourselves; tao's own style APIs
//! (`set_ignore_cursor_events`, `show`, `set_skip_taskbar`, ...) rewrite
//! GWL_STYLE/GWL_EXSTYLE wholesale and must not be called on an attached
//! window (`is_attached` notices if they were, and the shell re-attaches).
//!
//! Only a real Windows desktop can validate: the 0x052C behaviour and the
//! window tree on each Windows build, that WebView2 renders inside a
//! (layered) child of another process's window, the z-order on 24H2, that
//! icons/right-click/rubber-band keep working, and what happens to the
//! attached windows when Explorer restarts (children of a destroyed parent
//! are destroyed with it; the shell recreates them on `ShellRestarted`).
//! Note also that SetParent across processes attaches the two threads' input
//! queues (every wallpaper engine lives with this): a hung Explorer can stall
//! our UI thread and vice versa.

use std::collections::HashMap;
use std::mem::size_of;

use ::windows::core::{w, BOOL, PCWSTR};
use ::windows::Win32::Foundation::{COLORREF, FALSE, HWND, LPARAM, POINT, RECT, TRUE, WPARAM};
use ::windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MapWindowPoints, RedrawWindow, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
    RDW_ALLCHILDREN, RDW_ERASE, RDW_INVALIDATE,
};
use ::windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use ::windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, IsWindowEnabled};
use ::windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, FindWindowW, GetAncestor, GetWindow, GetWindowLongPtrW, IsWindow, IsWindowVisible,
    SendMessageTimeoutW, SetLayeredWindowAttributes, SetParent, SetWindowLongPtrW, SetWindowPos, ShowWindow, GA_PARENT,
    GWL_EXSTYLE, GWL_STYLE, GW_HWNDLAST, GW_HWNDNEXT, HWND_BOTTOM, LWA_ALPHA, MONITORINFOF_PRIMARY, SMTO_NORMAL,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOWNA, WINDOW_LONG_PTR_INDEX,
};

use super::util::{self, hwnd, raw, PerMonitorDpi};
use crate::platform::logic::{self, style};
use crate::platform::{MonitorInfo, PlatformError, PlatformResult};

/// Progman: "create the wallpaper WorkerW" (legacy) / "raise the desktop"
/// (24H2). Undocumented; wParam 0xD, lParam 1 as used by Lively Wallpaper.
const SPAWN_WORKERW: u32 = 0x052C;

/// Explorer's desktop windows as last discovered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Layer {
    progman: isize,
    /// SHELLDLL_DefView (child of Progman when raised, of the icon WorkerW
    /// otherwise).
    defview: isize,
    /// Raised: Progman's WorkerW child (must stay last). Legacy: the empty
    /// top-level WorkerW behind the icons (our parent).
    workerw: isize,
    raised: bool,
}

impl Layer {
    fn parent(&self) -> isize {
        if self.raised {
            self.progman
        } else {
            self.workerw
        }
    }

    fn alive(&self) -> bool {
        [self.progman, self.defview, self.workerw]
            .iter()
            .all(|window| unsafe { IsWindow(Some(hwnd(*window))).as_bool() })
    }
}

/// What `attach` changed, to undo it in `detach`.
#[derive(Debug, Clone, Copy)]
struct Attached {
    original_style: u32,
    original_ex_style: u32,
    was_enabled: bool,
    parent: isize,
}

/// Owns the attachment of wallpaper windows to the desktop. Lives on the
/// main (UI) thread; holds only plain handles (`isize`), so it is `Send`
/// and `Sync` like the macOS stand-in.
pub struct WallpaperHost {
    layer: Option<Layer>,
    attached: HashMap<isize, Attached>,
}

impl WallpaperHost {
    /// Cheap: the desktop windows are discovered on `refresh`/`attach`.
    pub fn new() -> PlatformResult<Self> {
        Ok(WallpaperHost { layer: None, attached: HashMap::new() })
    }

    /// Re-discovers Progman / DefView / WorkerW (after Explorer restarts or
    /// recreates its desktop windows).
    pub fn refresh(&mut self) -> PlatformResult<()> {
        match discover() {
            Ok(layer) => {
                if self.layer != Some(layer) {
                    log::info!(
                        "wallpaper: {} desktop (Progman {:#x}, DefView {:#x}, WorkerW {:#x})",
                        if layer.raised { "raised (24H2+)" } else { "legacy WorkerW" },
                        layer.progman,
                        layer.defview,
                        layer.workerw
                    );
                }
                self.layer = Some(layer);
                util::watch_shell_windows(&[layer.progman, layer.workerw]);
                if layer.raised {
                    unsafe { keep_workerw_last(&layer) };
                }
                Ok(())
            }
            Err(error) => {
                self.layer = None;
                util::watch_shell_windows(&[]);
                Err(error)
            }
        }
    }

    fn layer(&mut self) -> PlatformResult<Layer> {
        if let Some(layer) = self.layer {
            if layer.alive() {
                return Ok(layer);
            }
        }
        self.refresh()?;
        self.layer.ok_or_else(|| PlatformError::msg("the desktop window layer is unavailable"))
    }

    /// Puts `window` behind the desktop icons, covering `monitor` (physical
    /// pixels). Main thread. Safe to call again on an attached window (that
    /// is how the shell repairs one).
    pub fn attach(&mut self, window: isize, monitor: &MonitorInfo) -> PlatformResult<()> {
        let target = hwnd(window);
        if !unsafe { IsWindow(Some(target)) }.as_bool() {
            self.attached.remove(&window);
            return Err(PlatformError::msg("the wallpaper window no longer exists"));
        }
        if monitor.width <= 0 || monitor.height <= 0 {
            return Err(PlatformError::msg(format!("monitor {} has no area", monitor.id)));
        }
        let layer = self.layer()?;
        let parent = hwnd(layer.parent());
        unsafe {
            let style = window_long(target, GWL_STYLE);
            let ex_style = window_long(target, GWL_EXSTYLE);
            // Remember the top-level state once, on first attach.
            let was_enabled = IsWindowEnabled(target).as_bool();
            let entry = self.attached.entry(window).or_insert(Attached {
                original_style: style,
                original_ex_style: ex_style,
                was_enabled,
                parent: 0,
            });

            // Styles go on while the window is still top-level: SetParent
            // expects WS_CHILD already set, and WS_EX_LAYERED (+ its alpha)
            // must be in place before the window becomes a child.
            set_window_long(target, GWL_STYLE, logic::attached_style(style));
            set_window_long(target, GWL_EXSTYLE, logic::attached_ex_style(ex_style, layer.raised));
            if layer.raised {
                if let Err(error) = SetLayeredWindowAttributes(target, COLORREF(0), 255, LWA_ALPHA) {
                    log::warn!(
                        "wallpaper: SetLayeredWindowAttributes failed ({error}); layered child windows need a \
                         Windows 8+ <supportedOS> entry in the application manifest"
                    );
                }
            }
            if GetAncestor(target, GA_PARENT) != parent {
                SetParent(target, Some(parent))
                    .map_err(|error| PlatformError::msg(format!("SetParent to the desktop failed: {error}")))?;
            }
            entry.parent = layer.parent();

            // Monitor rectangle (screen) → parent client coordinates. Both
            // processes are per-monitor-v2 DPI aware, so this is 1:1 physical
            // pixels; two points are treated as a RECT (RTL-safe).
            let mut corners = [
                POINT { x: monitor.x, y: monitor.y },
                POINT { x: monitor.x.saturating_add(monitor.width), y: monitor.y.saturating_add(monitor.height) },
            ];
            MapWindowPoints(None, Some(parent), &mut corners);
            let x = corners[0].x.min(corners[1].x);
            let y = corners[0].y.min(corners[1].y);
            // Raised: directly below DefView. Legacy: z-order among our own
            // siblings in the WorkerW does not matter.
            let (insert_after, flags) = if layer.raised {
                (Some(hwnd(layer.defview)), SWP_NOACTIVATE | SWP_FRAMECHANGED)
            } else {
                (None, SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_NOZORDER)
            };
            SetWindowPos(target, insert_after, x, y, monitor.width, monitor.height, flags)
                .map_err(|error| PlatformError::msg(format!("positioning the wallpaper failed: {error}")))?;
            if layer.raised {
                keep_workerw_last(&layer);
            }
            let _ = EnableWindow(target, false);
            let _ = ShowWindow(target, SW_SHOWNA);
        }
        log::info!(
            "wallpaper: attached {window:#x} to {} at {}x{}+{}+{} ({})",
            monitor.id,
            monitor.width,
            monitor.height,
            monitor.x,
            monitor.y,
            if layer.raised { "raised desktop" } else { "WorkerW" }
        );
        Ok(())
    }

    /// Undoes `attach`: hidden, enabled, top-level again with its original
    /// styles. Main thread.
    pub fn detach(&mut self, window: isize) -> PlatformResult<()> {
        let Some(info) = self.attached.remove(&window) else { return Ok(()) };
        let target = hwnd(window);
        unsafe {
            if !IsWindow(Some(target)).as_bool() {
                return Ok(());
            }
            let former_parent = GetAncestor(target, GA_PARENT);
            let _ = ShowWindow(target, SW_HIDE);
            let _ = EnableWindow(target, info.was_enabled);
            SetParent(target, None)
                .map_err(|error| PlatformError::msg(format!("SetParent back to the desktop failed: {error}")))?;
            // After SetParent(NULL) the docs say to swap WS_CHILD for the
            // original top-level styles.
            let current = window_long(target, GWL_STYLE);
            set_window_long(target, GWL_STYLE, logic::restored_style(info.original_style, current));
            set_window_long(target, GWL_EXSTYLE, info.original_ex_style);
            let _ = SetWindowPos(
                target,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
            // Legacy WorkerW keeps our last frame until it repaints.
            if raw(former_parent) == info.parent && info.parent != 0 {
                let _ = RedrawWindow(Some(former_parent), None, None, RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN);
            }
        }
        Ok(())
    }

    /// True if `window` is still attached the way `attach` left it: alive,
    /// parented to the current desktop layer, styles intact (tao has not
    /// rewritten them), visible, and — raised layout — below DefView with
    /// the WorkerW still last. Cheap: no messages are sent.
    pub fn is_attached(&self, window: isize) -> bool {
        let (Some(layer), Some(info)) = (self.layer, self.attached.get(&window)) else { return false };
        if info.parent != layer.parent() || !layer.alive() {
            return false;
        }
        let target = hwnd(window);
        unsafe {
            if !IsWindow(Some(target)).as_bool() || GetAncestor(target, GA_PARENT) != hwnd(info.parent) {
                return false;
            }
            if !logic::styles_intact(window_long(target, GWL_STYLE), window_long(target, GWL_EXSTYLE), layer.raised) {
                return false;
            }
            !layer.raised || (below_defview(&layer, target) && workerw_is_last(&layer))
        }
    }
}

unsafe fn window_long(window: HWND, index: WINDOW_LONG_PTR_INDEX) -> u32 {
    // Styles live in the low 32 bits.
    GetWindowLongPtrW(window, index) as u32
}

unsafe fn set_window_long(window: HWND, index: WINDOW_LONG_PTR_INDEX, value: u32) {
    SetWindowLongPtrW(window, index, value as isize);
}

/// Raised layout: Explorer's WorkerW must remain Progman's last child.
unsafe fn keep_workerw_last(layer: &Layer) {
    if !workerw_is_last(layer) {
        log::debug!("wallpaper: moving the desktop WorkerW back to the bottom");
        let _ =
            SetWindowPos(hwnd(layer.workerw), Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

unsafe fn workerw_is_last(layer: &Layer) -> bool {
    let workerw = hwnd(layer.workerw);
    GetWindow(workerw, GW_HWNDLAST).map(|last| last == workerw).unwrap_or(false)
}

/// Is `target` somewhere below DefView among Progman's children?
unsafe fn below_defview(layer: &Layer, target: HWND) -> bool {
    let mut current = hwnd(layer.defview);
    for _ in 0..512 {
        match GetWindow(current, GW_HWNDNEXT) {
            Ok(next) if next == target => return true,
            Ok(next) => current = next,
            Err(_) => return false,
        }
    }
    false
}

/// Finds Explorer's desktop windows, asking Progman to create the wallpaper
/// WorkerW / raise the desktop (0x052C) only when they are not there yet:
/// the shell's watchdog calls `refresh` whenever something looks off, and
/// re-raising a working desktop every time would be needless churn.
fn discover() -> PlatformResult<Layer> {
    unsafe {
        let progman = FindWindowW(w!("Progman"), PCWSTR::null()).map_err(|_| {
            PlatformError::msg("Explorer's desktop window (Progman) was not found; is Explorer running?")
        })?;
        // Without asking first, only trust a WorkerW that is on screen (a
        // hidden one may be some other shell WorkerW).
        if let Ok(layer) = find_layer(progman) {
            if IsWindowVisible(hwnd(layer.workerw)).as_bool() {
                return Ok(layer);
            }
        }
        // Bounded so a hung Explorer cannot hang the UI thread.
        let mut result = 0usize;
        let _ =
            SendMessageTimeoutW(progman, SPAWN_WORKERW, WPARAM(0xD), LPARAM(0x1), SMTO_NORMAL, 1000, Some(&mut result));
        let layer = find_layer(progman)?;
        if !IsWindowVisible(hwnd(layer.workerw)).as_bool() {
            log::warn!("wallpaper: the desktop WorkerW {:#x} is hidden; the wallpaper may not show", layer.workerw);
        }
        Ok(layer)
    }
}

unsafe fn find_layer(progman: HWND) -> PlatformResult<Layer> {
    // 24H2+: Progman is created with WS_EX_NOREDIRECTIONBITMAP and, once
    // raised, holds DefView and a WorkerW as children.
    let raised = window_long(progman, GWL_EXSTYLE) & style::WS_EX_NOREDIRECTIONBITMAP != 0;
    if raised {
        if let Ok(defview) = FindWindowExW(Some(progman), None, w!("SHELLDLL_DefView"), PCWSTR::null()) {
            let workerw = FindWindowExW(Some(progman), None, w!("WorkerW"), PCWSTR::null())
                .map_err(|_| PlatformError::msg("the raised desktop has no WorkerW child (yet)"))?;
            return Ok(Layer { progman: raw(progman), defview: raw(defview), workerw: raw(workerw), raised: true });
        }
        log::debug!("wallpaper: Progman looks raised but SHELLDLL_DefView is not its child; trying the legacy layout");
    }

    // Legacy: the top-level window that holds SHELLDLL_DefView, then the next
    // WorkerW after it in z-order.
    let mut found = IconLayer::default();
    let _ = EnumWindows(Some(find_icon_layer), LPARAM(&mut found as *mut IconLayer as isize));
    if found.top == 0 {
        return Err(PlatformError::msg("the desktop icon layer (SHELLDLL_DefView) was not found"));
    }
    if found.top == raw(progman) {
        // The icons still live in Progman: Explorer has not split off a
        // wallpaper WorkerW (before 0x052C, or it had no effect).
        return Err(PlatformError::msg("Explorer has not created a wallpaper WorkerW behind the icons yet"));
    }
    let workerw = FindWindowExW(None, Some(hwnd(found.top)), w!("WorkerW"), PCWSTR::null())
        .map_err(|_| PlatformError::msg("the wallpaper WorkerW behind the desktop icons was not found"))?;
    Ok(Layer { progman: raw(progman), defview: found.defview, workerw: raw(workerw), raised: false })
}

#[derive(Default)]
struct IconLayer {
    top: isize,
    defview: isize,
}

unsafe extern "system" fn find_icon_layer(window: HWND, data: LPARAM) -> BOOL {
    let Some(found) = (data.0 as *mut IconLayer).as_mut() else { return FALSE };
    match FindWindowExW(Some(window), None, w!("SHELLDLL_DefView"), PCWSTR::null()) {
        Ok(defview) => {
            found.top = raw(window);
            found.defview = raw(defview);
            FALSE
        }
        Err(_) => TRUE,
    }
}

/// Every monitor in physical pixels, primary first, then left to right.
pub fn monitors() -> Vec<MonitorInfo> {
    monitor_handles().into_iter().map(|(_, monitor)| monitor).collect()
}

/// `monitors()` plus each monitor's HMONITOR (as `isize`).
pub(super) fn monitor_handles() -> Vec<(isize, MonitorInfo)> {
    let _dpi = PerMonitorDpi::enter();
    let mut list: Vec<(isize, MonitorInfo)> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(add_monitor),
            LPARAM(&mut list as *mut Vec<(isize, MonitorInfo)> as isize),
        );
    }
    list.sort_by_key(|(_, monitor)| (!monitor.primary, monitor.x, monitor.y));
    list
}

unsafe extern "system" fn add_monitor(monitor: HMONITOR, _dc: HDC, _clip: *mut RECT, data: LPARAM) -> BOOL {
    let Some(list) = (data.0 as *mut Vec<(isize, MonitorInfo)>).as_mut() else { return FALSE };
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(monitor, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO).as_bool() {
        let rect = info.monitorInfo.rcMonitor;
        let name_length = info.szDevice.iter().position(|unit| *unit == 0).unwrap_or(info.szDevice.len());
        let (mut dpi_x, mut dpi_y) = (0u32, 0u32);
        let scale = match GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) {
            Ok(()) if dpi_x > 0 => f64::from(dpi_x) / 96.0,
            _ => 1.0,
        };
        list.push((
            monitor.0 as isize,
            MonitorInfo {
                id: String::from_utf16_lossy(&info.szDevice[..name_length]),
                x: rect.left,
                y: rect.top,
                width: rect.right - rect.left,
                height: rect.bottom - rect.top,
                scale,
                primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
            },
        ));
    }
    TRUE
}

#[cfg(test)]
mod tests {
    // Compile-time check: the shell keeps the host in a Mutex inside managed
    // state, which requires Send (the macOS stand-in is a unit struct).
    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn host_is_send_and_sync() {
        assert_send_sync::<super::WallpaperHost>();
    }
}
