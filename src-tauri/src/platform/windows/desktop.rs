//! Desktop watcher: display changes, Explorer restarts, full-screen /
//! maximised coverage and session lock, observed from one hidden window on
//! its own thread.
//!
//! The window is a hidden, unowned *top-level* popup: message-only windows
//! (HWND_MESSAGE) do not receive broadcasts such as TaskbarCreated,
//! WM_DISPLAYCHANGE or WM_SETTINGCHANGE. Everything below runs on that
//! thread: the window procedure, the out-of-context WinEvent hook (delivered
//! through the same message loop) and the timers. Thread-local state is only
//! borrowed for short bookkeeping, never across calls that could pump
//! messages, so a re-entrant message cannot hit a `RefCell` borrow.

use std::cell::{OnceCell, RefCell};
use std::ffi::c_void;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use ::windows::core::{w, BOOL, PCWSTR};
use ::windows::Win32::Foundation::HANDLE;
use ::windows::Win32::Foundation::{
    GetLastError, ERROR_CLASS_ALREADY_EXISTS, HWND, LPARAM, LRESULT, RECT, TRUE, WPARAM,
};
use ::windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use ::windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL};
use ::windows::Win32::System::LibraryLoader::GetModuleHandleW;
use ::windows::Win32::System::Power::{RegisterPowerSettingNotification, POWERBROADCAST_SETTING};
use ::windows::Win32::System::RemoteDesktop::{
    WTSRegisterSessionNotification, WTSUnRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION,
};
use ::windows::Win32::System::SystemServices::GUID_SESSION_DISPLAY_STATUS;
use ::windows::Win32::System::Threading::GetCurrentProcessId;
use ::windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use ::windows::Win32::UI::Shell::{SHQueryUserNotificationState, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN};
use ::windows::Win32::UI::WindowsAndMessaging::{
    ChangeWindowMessageFilterEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EnumWindows,
    FindWindowW, GetDesktopWindow, GetForegroundWindow, GetLayeredWindowAttributes, GetMessageW, GetShellWindow,
    GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible, IsZoomed, KillTimer,
    PostMessageW, PostQuitMessage, RegisterClassExW, RegisterWindowMessageW, SetTimer, TranslateMessage, CHILDID_SELF,
    DEVICE_NOTIFY_WINDOW_HANDLE, EVENT_OBJECT_DESTROY, GWL_EXSTYLE, LAYERED_WINDOW_ATTRIBUTES_FLAGS, LWA_ALPHA,
    LWA_COLORKEY, MSG, MSGFLT_ALLOW, OBJID_WINDOW, PBT_POWERSETTINGCHANGE, SPI_SETWORKAREA, WINEVENT_OUTOFCONTEXT,
    WM_APP, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_POWERBROADCAST, WM_SETTINGCHANGE, WM_TIMER,
    WM_WTSSESSION_CHANGE, WNDCLASSEXW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP, WTS_SESSION_LOCK,
    WTS_SESSION_UNLOCK,
};

use super::util::{self, hwnd, raw, PerMonitorDpi};
use super::wallpaper;
use crate::platform::logic::{self, style, Rect, WindowFacts};
use crate::platform::{DesktopEvent, EventSink, MonitorInfo, PlatformError, PlatformEvent, PlatformResult};

const CLASS_NAME: PCWSTR = w!("LunalyrDesktopWatcher");

/// 1 s: coverage, monitor-list and Explorer-PID polling.
const TIMER_POLL: usize = 1;
/// Debounce for DisplaysChanged.
const TIMER_DISPLAYS: usize = 2;
/// Delay before ShellRestarted (lets Explorer finish building its desktop).
const TIMER_SHELL: usize = 3;
const POLL_MS: u32 = 1_000;
const DISPLAY_DEBOUNCE_MS: u32 = 500;
const SHELL_SETTLE_AFTER_RESTART_MS: u32 = 1_000;
const SHELL_SETTLE_AFTER_DESTROY_MS: u32 = 300;
/// TaskbarCreated arrived while the state was busy: handle it again.
const WM_RETRY_TASKBAR_CREATED: u32 = WM_APP + 1;
/// WTSRegisterSessionNotification fails until Remote Desktop Services is up
/// (early autostart); retry once per poll for this long.
const SESSION_REGISTRATION_ATTEMPTS: u32 = 300;

static STARTED: AtomicBool = AtomicBool::new(false);
static WATCHER: AtomicIsize = AtomicIsize::new(0);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
/// Last display power state sent: 0 unknown, 1 on, 2 off.
static DISPLAYS_STATE: AtomicU32 = AtomicU32::new(0);

thread_local! {
    static SINK: OnceCell<EventSink> = const { OnceCell::new() };
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

struct State {
    explorer_pid: u32,
    hook: Option<HWINEVENTHOOK>,
    coverage: Option<Vec<(String, bool)>>,
    monitors: Vec<MonitorInfo>,
    session_registered: bool,
    session_attempts: u32,
}

pub fn start_desktop_watcher(sink: EventSink) -> PlatformResult<()> {
    if STARTED.swap(true, Ordering::SeqCst) {
        return Err(PlatformError::msg("the desktop watcher is already running"));
    }
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    if let Err(error) = thread::Builder::new().name("lunalyr-desktop".into()).spawn(move || run(sink, ready_tx)) {
        STARTED.store(false, Ordering::SeqCst);
        return Err(PlatformError::msg(format!("could not start the desktop watcher: {error}")));
    }
    // Creating the window takes milliseconds; wait so a failure is reported.
    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(message)) => {
            STARTED.store(false, Ordering::SeqCst);
            Err(PlatformError::msg(message))
        }
        Err(_) => {
            log::warn!("desktop: the watcher is slow to start; continuing");
            Ok(())
        }
    }
}

fn run(sink: EventSink, ready: mpsc::Sender<Result<(), String>>) {
    let _dpi = PerMonitorDpi::enter();
    let window = match create_window() {
        Ok(window) => window,
        Err(message) => {
            let _ = ready.send(Err(message));
            return;
        }
    };
    // The caller only waits for the window; the rest (RPC to Remote Desktop
    // Services, hooks) can be slow right after login.
    let _ = ready.send(Ok(()));
    SINK.with(|cell| {
        let _ = cell.set(sink);
    });
    WATCHER.store(raw(window), Ordering::SeqCst);
    unsafe {
        let taskbar_created = RegisterWindowMessageW(w!("TaskbarCreated"));
        TASKBAR_CREATED.store(taskbar_created, Ordering::SeqCst);
        if taskbar_created != 0 {
            // Explorer runs at medium integrity; if we were started elevated,
            // UIPI would drop its broadcast otherwise.
            let _ = ChangeWindowMessageFilterEx(window, taskbar_created, MSGFLT_ALLOW, None);
        }
    }

    // Display power for this session (on / off / dimmed). Windows sends the
    // current state right after registering. The registration lives as long
    // as the watcher window (the whole process).
    match unsafe {
        RegisterPowerSettingNotification(HANDLE(window.0), &GUID_SESSION_DISPLAY_STATUS, DEVICE_NOTIFY_WINDOW_HANDLE)
    } {
        Ok(_) => {}
        Err(error) => log::warn!("desktop: display power notifications unavailable ({error})"),
    }

    let explorer_pid = explorer_pid();
    let session_registered = register_session(window);
    let state = State {
        explorer_pid,
        hook: install_hook(explorer_pid),
        coverage: None,
        monitors: wallpaper::monitors(),
        session_registered,
        session_attempts: 1,
    };
    STATE.with(|cell| *cell.borrow_mut() = Some(state));
    unsafe {
        SetTimer(Some(window), TIMER_POLL, POLL_MS, None);
    }
    log::info!("desktop: watcher running (Explorer pid {explorer_pid})");
    poll(window);

    let mut message = MSG::default();
    loop {
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) };
        // 0: WM_QUIT; -1: error.
        if result.0 == 0 || result.0 == -1 {
            break;
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    if let Some(state) = STATE.with(|cell| cell.borrow_mut().take()) {
        unsafe {
            if let Some(hook) = state.hook {
                let _ = UnhookWinEvent(hook);
            }
            if state.session_registered {
                let _ = WTSUnRegisterSessionNotification(window);
            }
        }
    }
    WATCHER.store(0, Ordering::SeqCst);
    STARTED.store(false, Ordering::SeqCst);
    log::info!("desktop: watcher stopped");
}

fn create_window() -> Result<HWND, String> {
    unsafe {
        let instance = GetModuleHandleW(PCWSTR::null()).map_err(|error| format!("GetModuleHandleW failed: {error}"))?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            let error = GetLastError();
            if error != ERROR_CLASS_ALREADY_EXISTS {
                return Err(format!("RegisterClassExW failed: {}", ::windows::core::Error::from(error.to_hresult())));
            }
        }
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            CLASS_NAME,
            w!("Lunalyr desktop watcher"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .map_err(|error| format!("could not create the desktop watcher window: {error}"))
    }
}

unsafe extern "system" fn window_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if message == WM_POWERBROADCAST {
        if wparam.0 as u32 == PBT_POWERSETTINGCHANGE && lparam.0 != 0 {
            // SAFETY: for PBT_POWERSETTINGCHANGE, lParam points to a
            // POWERBROADCAST_SETTING valid for the duration of the call.
            let setting = unsafe { &*(lparam.0 as *const POWERBROADCAST_SETTING) };
            if setting.PowerSetting == GUID_SESSION_DISPLAY_STATUS && setting.DataLength >= 1 {
                let _ = util::guard(|| display_power(setting.Data[0]));
            }
        }
        return LRESULT(1);
    }
    match util::guard(|| handle_message(window, message, wparam)) {
        Some(Some(result)) => result,
        _ => DefWindowProcW(window, message, wparam, lparam),
    }
}

/// `Some(result)` if handled, `None` for DefWindowProc.
fn handle_message(window: HWND, message: u32, wparam: WPARAM) -> Option<LRESULT> {
    match message {
        WM_DISPLAYCHANGE | WM_DPICHANGED => {
            schedule(window, TIMER_DISPLAYS, DISPLAY_DEBOUNCE_MS);
            Some(LRESULT(0))
        }
        WM_SETTINGCHANGE => {
            if wparam.0 == SPI_SETWORKAREA.0 as usize {
                schedule(window, TIMER_DISPLAYS, DISPLAY_DEBOUNCE_MS);
            }
            None
        }
        WM_WTSSESSION_CHANGE => {
            match wparam.0 as u32 {
                WTS_SESSION_LOCK => send(DesktopEvent::SessionLocked(true)),
                WTS_SESSION_UNLOCK => send(DesktopEvent::SessionLocked(false)),
                _ => {}
            }
            Some(LRESULT(0))
        }
        WM_TIMER => {
            match wparam.0 {
                TIMER_POLL => poll(window),
                TIMER_DISPLAYS => {
                    cancel(window, TIMER_DISPLAYS);
                    displays_changed();
                }
                TIMER_SHELL => {
                    cancel(window, TIMER_SHELL);
                    log::info!("desktop: Explorer's desktop changed; asking for a re-attach");
                    send(DesktopEvent::ShellRestarted);
                }
                _ => return None,
            }
            Some(LRESULT(0))
        }
        WM_RETRY_TASKBAR_CREATED => {
            taskbar_created(window);
            Some(LRESULT(0))
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            Some(LRESULT(0))
        }
        _ => {
            let taskbar_created_message = TASKBAR_CREATED.load(Ordering::Relaxed);
            if taskbar_created_message != 0 && message == taskbar_created_message {
                taskbar_created(window);
                return Some(LRESULT(0));
            }
            None
        }
    }
}

fn send(event: DesktopEvent) {
    SINK.with(|cell| {
        if let Some(sink) = cell.get() {
            let _ = sink.send(PlatformEvent::Desktop(event));
        }
    });
}

fn sink_closed() -> bool {
    SINK.with(|cell| cell.get().map(|sink| sink.is_closed()).unwrap_or(true))
}

/// Runs `f` on the watcher state unless it is already borrowed (re-entrant
/// message) or gone.
fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|cell| match cell.try_borrow_mut() {
        Ok(mut state) => state.as_mut().map(f),
        Err(_) => None,
    })
}

/// (Re)starts timer `id`; re-arming an existing timer restarts its countdown,
/// which is what makes it a debounce.
fn schedule(window: HWND, id: usize, milliseconds: u32) {
    unsafe {
        SetTimer(Some(window), id, milliseconds, None);
    }
}

fn cancel(window: HWND, id: usize) {
    unsafe {
        let _ = KillTimer(Some(window), id);
    }
}

fn displays_changed() {
    let monitors = wallpaper::monitors();
    log::info!("desktop: displays changed ({} monitor(s))", monitors.len());
    with_state(|state| {
        state.monitors = monitors;
        // Monitor ids may have changed: report coverage afresh.
        state.coverage = None;
    });
    send(DesktopEvent::DisplaysChanged);
}

/// TaskbarCreated is broadcast when Explorer (re)creates the taskbar — and
/// also on some DPI changes, so only a new Explorer process counts.
fn taskbar_created(window: HWND) {
    let pid = explorer_pid();
    let outcome = with_state(|state| {
        if pid != 0 && pid != state.explorer_pid {
            state.explorer_pid = pid;
            true
        } else {
            false
        }
    });
    match outcome {
        None => unsafe {
            let _ = PostMessageW(Some(window), WM_RETRY_TASKBAR_CREATED, WPARAM(0), LPARAM(0));
        },
        Some(true) => {
            log::info!("desktop: Explorer restarted (pid {pid})");
            rehook(pid);
            schedule(window, TIMER_SHELL, SHELL_SETTLE_AFTER_RESTART_MS);
        }
        Some(false) => log::debug!("desktop: TaskbarCreated without an Explorer restart; ignored"),
    }
}

/// The 1 s tick: coverage, plus fallbacks for missed notifications (monitor
/// list, Explorer process, session-notification registration).
fn poll(window: HWND) {
    if sink_closed() {
        unsafe {
            let _ = DestroyWindow(window);
        }
        return;
    }
    // Everything that talks to the OS happens outside the state borrow.
    let monitors = wallpaper::monitor_handles();
    let coverage = compute_coverage(&monitors);
    let infos: Vec<MonitorInfo> = monitors.into_iter().map(|(_, monitor)| monitor).collect();
    let pid = explorer_pid();

    let Some((coverage_changed, displays_moved, explorer_moved, register)) = with_state(|state| {
        let coverage_changed = state.coverage.as_ref() != Some(&coverage);
        if coverage_changed {
            state.coverage = Some(coverage.clone());
        }
        let displays_moved = state.monitors != infos;
        let explorer_moved = pid != 0 && pid != state.explorer_pid;
        if explorer_moved {
            state.explorer_pid = pid;
        }
        let register = !state.session_registered && state.session_attempts < SESSION_REGISTRATION_ATTEMPTS;
        if register {
            state.session_attempts += 1;
        }
        (coverage_changed, displays_moved, explorer_moved, register)
    }) else {
        return;
    };

    if coverage_changed {
        log::debug!("desktop: coverage {coverage:?}");
        send(DesktopEvent::Coverage(coverage));
    }
    if displays_moved {
        schedule(window, TIMER_DISPLAYS, DISPLAY_DEBOUNCE_MS);
    }
    if explorer_moved {
        log::info!("desktop: Explorer is now pid {pid}");
        rehook(pid);
        schedule(window, TIMER_SHELL, SHELL_SETTLE_AFTER_RESTART_MS);
    }
    if register && register_session(window) {
        with_state(|state| state.session_registered = true);
        log::info!("desktop: session lock notifications registered");
    }
}

fn register_session(window: HWND) -> bool {
    match unsafe { WTSRegisterSessionNotification(window, NOTIFY_FOR_THIS_SESSION) } {
        Ok(()) => true,
        Err(error) => {
            log::debug!("desktop: WTSRegisterSessionNotification failed ({error}); will retry");
            false
        }
    }
}

/// Explorer's process id (owner of the shell window or the taskbar), 0 if
/// Explorer is not running.
fn explorer_pid() -> u32 {
    unsafe {
        let mut pid = 0u32;
        let shell = GetShellWindow();
        if !shell.is_invalid() {
            GetWindowThreadProcessId(shell, Some(&mut pid));
        }
        if pid == 0 {
            if let Ok(tray) = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) {
                GetWindowThreadProcessId(tray, Some(&mut pid));
            }
        }
        pid
    }
}

/// Watches Explorer for destroyed windows; the callback filters for the
/// Progman/WorkerW the wallpaper host is using.
fn install_hook(pid: u32) -> Option<HWINEVENTHOOK> {
    if pid == 0 {
        return None;
    }
    let hook = unsafe {
        SetWinEventHook(
            EVENT_OBJECT_DESTROY,
            EVENT_OBJECT_DESTROY,
            None,
            Some(win_event),
            pid,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    if hook.is_invalid() {
        log::warn!("desktop: could not watch Explorer's windows (SetWinEventHook failed)");
        None
    } else {
        Some(hook)
    }
}

fn rehook(pid: u32) {
    let old = with_state(|state| state.hook.take()).flatten();
    if let Some(old) = old {
        unsafe {
            let _ = UnhookWinEvent(old);
        }
    }
    let new = install_hook(pid);
    if with_state(|state| state.hook = new).is_none() {
        if let Some(new) = new {
            unsafe {
                let _ = UnhookWinEvent(new);
            }
        }
    }
}

unsafe extern "system" fn win_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    window: HWND,
    object: i32,
    child: i32,
    _thread: u32,
    _time: u32,
) {
    let _ = util::guard(|| {
        if event != EVENT_OBJECT_DESTROY || object != OBJID_WINDOW.0 || child != CHILDID_SELF as i32 {
            return;
        }
        if !util::is_watched_shell_window(raw(window)) {
            return;
        }
        log::info!("desktop: Explorer destroyed desktop window {:#x}", raw(window));
        let watcher = WATCHER.load(Ordering::SeqCst);
        if watcher != 0 {
            schedule(hwnd(watcher), TIMER_SHELL, SHELL_SETTLE_AFTER_DESTROY_MS);
        }
    });
}

// ---- coverage ------------------------------------------------------------------

struct Collect {
    windows: Vec<WindowFacts>,
    monitors: Vec<isize>,
    own_pid: u32,
    shell: isize,
    desktop: isize,
}

fn compute_coverage(monitors: &[(isize, MonitorInfo)]) -> Vec<(String, bool)> {
    let rects: Vec<Rect> = monitors
        .iter()
        .map(|(_, monitor)| Rect::from_origin_size(monitor.x, monitor.y, monitor.width, monitor.height))
        .collect();
    let mut collect = unsafe {
        Collect {
            windows: Vec::new(),
            monitors: monitors.iter().map(|(handle, _)| *handle).collect(),
            own_pid: GetCurrentProcessId(),
            shell: raw(GetShellWindow()),
            desktop: raw(GetDesktopWindow()),
        }
    };
    unsafe {
        let _ = EnumWindows(Some(collect_window), LPARAM(&mut collect as *mut Collect as isize));
    }
    let fullscreen = fullscreen_monitor(&collect);
    let covered = logic::coverage(&rects, &collect.windows, fullscreen);
    monitors.iter().zip(covered).map(|((_, monitor), covered)| (monitor.id.clone(), covered)).collect()
}

unsafe extern "system" fn collect_window(window: HWND, data: LPARAM) -> BOOL {
    if let Some(collect) = (data.0 as *mut Collect).as_mut() {
        let _ = util::guard(|| collect.add(window));
    }
    TRUE
}

impl Collect {
    fn add(&mut self, window: HWND) {
        unsafe {
            // Most top-level windows are hidden; skip them cheaply.
            if !IsWindowVisible(window).as_bool() {
                return;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(window, Some(&mut pid));
            let mut cloaked = 0u32;
            let cloaked = DwmGetWindowAttribute(
                window,
                DWMWA_CLOAKED,
                &mut cloaked as *mut u32 as *mut c_void,
                size_of::<u32>() as u32,
            )
            .is_ok()
                && cloaked != 0;
            let ex_style = GetWindowLongPtrW(window, GWL_EXSTYLE) as u32;
            let opaque_layered = ex_style & style::WS_EX_LAYERED != 0 && {
                let mut alpha = 0u8;
                let mut flags = LAYERED_WINDOW_ATTRIBUTES_FLAGS(0);
                // Fails for UpdateLayeredWindow (per-pixel alpha) windows:
                // treated as see-through.
                GetLayeredWindowAttributes(window, None, Some(&mut alpha), Some(&mut flags)).is_ok()
                    && flags.0 & LWA_COLORKEY.0 == 0
                    && (flags.0 & LWA_ALPHA.0 == 0 || alpha == 255)
            };
            let mut rect = RECT::default();
            let rect = if GetWindowRect(window, &mut rect).is_ok() {
                Rect::new(rect.left, rect.top, rect.right, rect.bottom)
            } else {
                Rect::default()
            };
            let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTONULL);
            self.windows.push(WindowFacts {
                class_name: util::class_name(window),
                is_shell_window: raw(window) == self.shell || raw(window) == self.desktop,
                visible: true,
                cloaked,
                minimized: IsIconic(window).as_bool(),
                maximized: IsZoomed(window).as_bool(),
                own_process: pid == self.own_pid,
                ex_style,
                opaque_layered,
                rect,
                monitor: self.monitors.iter().position(|handle| *handle == monitor.0 as isize),
            });
        }
    }
}

/// Exclusive full-screen Direct3D or presentation mode: the shell says so,
/// and the foreground window's monitor counts as covered (unless the
/// foreground window is the desktop itself or ours).
fn fullscreen_monitor(collect: &Collect) -> Option<usize> {
    let state = unsafe { SHQueryUserNotificationState() }.ok()?;
    if state != QUNS_RUNNING_D3D_FULL_SCREEN && state != QUNS_PRESENTATION_MODE {
        return None;
    }
    let foreground = unsafe { GetForegroundWindow() };
    if foreground.is_invalid() || raw(foreground) == collect.shell {
        return None;
    }
    if logic::SHELL_WINDOW_CLASSES.contains(&util::class_name(foreground).as_str()) {
        return None;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(foreground, Some(&mut pid)) };
    if pid == collect.own_pid {
        return None;
    }
    let monitor = unsafe { MonitorFromWindow(foreground, MONITOR_DEFAULTTONEAREST) };
    collect.monitors.iter().position(|handle| *handle == monitor.0 as isize)
}

/// GUID_SESSION_DISPLAY_STATUS data: 0 = off, 1 = on, 2 = dimmed. Dimmed
/// counts as on (still visible), as on macOS.
fn display_power(state: u8) {
    let next = if state == 0 { 2 } else { 1 };
    let previous = DISPLAYS_STATE.swap(next, Ordering::SeqCst);
    if previous != next {
        log::info!("desktop: displays {}", if next == 2 { "off" } else { "on" });
        send(DesktopEvent::DisplaysAsleep(next == 2));
    }
}
