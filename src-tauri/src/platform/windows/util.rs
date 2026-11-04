//! Small helpers shared by the Windows modules.

use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ::windows::core::{Error, Result, RuntimeType, HRESULT};
use ::windows::Win32::Foundation::{ERROR_TIMEOUT, HWND, RPC_E_CHANGED_MODE};
use ::windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use ::windows::Win32::UI::HiDpi::{
    SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use ::windows::Win32::UI::WindowsAndMessaging::GetClassNameW;
use windows_future::{AsyncStatus, IAsyncOperation};

/// `HWND` from the `isize` the shell passes around (`hwnd.0 as isize`).
pub(super) fn hwnd(raw: isize) -> HWND {
    HWND(raw as *mut c_void)
}

/// The `isize` form of a window handle.
pub(super) fn raw(window: HWND) -> isize {
    window.0 as isize
}

/// COM for the current thread in the multithreaded apartment, balanced on
/// drop. WinRT (GSMTC, the phonetic analyser) and WASAPI need it.
pub(super) struct ComApartment {
    initialized: bool,
}

impl ComApartment {
    pub(super) fn multithreaded() -> Result<Self> {
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if hr.is_ok() {
            // S_OK or S_FALSE (already initialised): both must be balanced.
            Ok(ComApartment { initialized: true })
        } else if hr == RPC_E_CHANGED_MODE {
            // The thread is already single-threaded: usable as it is, and not
            // ours to uninitialise.
            Ok(ComApartment { initialized: false })
        } else {
            Err(Error::from(hr))
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { CoUninitialize() };
        }
    }
}

/// Per-monitor-v2 DPI awareness for the current thread while alive, so
/// monitor and window rectangles are physical pixels regardless of what the
/// calling thread was set to. (tao already makes the whole process PMv2 via
/// SetProcessDpiAwarenessContext when the event loop is created, and Tauri's
/// default manifest declares no DPI awareness that could override that; this
/// is belt and braces for threads we do not own.)
pub(super) struct PerMonitorDpi {
    previous: DPI_AWARENESS_CONTEXT,
}

impl PerMonitorDpi {
    pub(super) fn enter() -> Self {
        let previous = unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        PerMonitorDpi { previous }
    }
}

impl Drop for PerMonitorDpi {
    fn drop(&mut self) {
        if !self.previous.0.is_null() {
            unsafe { SetThreadDpiAwarenessContext(self.previous) };
        }
    }
}

/// Waits for a WinRT async operation, giving up (and cancelling it) after
/// `timeout`. `IAsyncOperation::join` would block forever on a hung player
/// (thumbnail streams are served by the player process).
pub(super) fn wait<T: RuntimeType + 'static>(operation: &IAsyncOperation<T>, timeout: Duration) -> Result<T> {
    let deadline = Instant::now() + timeout;
    let mut nap = Duration::from_millis(1);
    loop {
        if operation.Status()? != AsyncStatus::Started {
            // Completed → the value; Error/Canceled → the operation's error.
            return operation.GetResults();
        }
        if Instant::now() >= deadline {
            let _ = operation.Cancel();
            return Err(Error::new(HRESULT::from_win32(ERROR_TIMEOUT.0), "WinRT call timed out"));
        }
        std::thread::sleep(nap);
        nap = (nap * 2).min(Duration::from_millis(16));
    }
}

/// Runs `f`, turning a panic into `None`. Used at the top of every
/// `extern "system"` callback: unwinding across the FFI boundary aborts.
/// (Release builds use panic = "abort", so callbacks are written not to
/// panic in the first place; this keeps debug builds alive too.)
pub(super) fn guard<R>(f: impl FnOnce() -> R) -> Option<R> {
    catch_unwind(AssertUnwindSafe(f)).ok()
}

/// The window's class name ("" if it cannot be read).
pub(super) fn class_name(window: HWND) -> String {
    let mut buffer = [0u16; 256];
    let length = unsafe { GetClassNameW(window, &mut buffer) };
    let length = usize::try_from(length).unwrap_or(0).min(buffer.len());
    String::from_utf16_lossy(&buffer[..length])
}

/// Explorer windows whose destruction means the wallpaper lost its parent
/// (published by the wallpaper host, watched by the desktop watcher).
static SHELL_WINDOWS: Mutex<Vec<isize>> = Mutex::new(Vec::new());

pub(super) fn watch_shell_windows(windows: &[isize]) {
    let mut list = SHELL_WINDOWS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    list.clear();
    list.extend(windows.iter().copied().filter(|window| *window != 0));
}

pub(super) fn is_watched_shell_window(window: isize) -> bool {
    window != 0 && SHELL_WINDOWS.lock().map(|list| list.contains(&window)).unwrap_or(false)
}
