//! Launch at login through the per-user Run key:
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\Lunalyr =
//! "<current exe>" --autostart`. Per-user, so no elevation is needed (the
//! NSIS installer is per-user too).

use std::ffi::c_void;

use ::windows::core::w;
use ::windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use ::windows::Win32::System::Registry::{RegDeleteKeyValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};

use crate::platform::{PlatformError, PlatformResult};

pub fn set_launch_at_login(enabled: bool) -> PlatformResult<()> {
    if enabled {
        let exe = std::env::current_exe()
            .map_err(|error| PlatformError::msg(format!("could not locate the Lunalyr executable: {error}")))?;
        let command = format!("\"{}\" --autostart", exe.display());
        let data: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();
        // REG_SZ: the size includes the terminating NUL. Creates the Run key
        // if a fresh profile does not have one yet.
        let status = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
                w!("Lunalyr"),
                REG_SZ.0,
                Some(data.as_ptr() as *const c_void),
                std::mem::size_of_val(data.as_slice()) as u32,
            )
        };
        check(status, "could not register Lunalyr to start at login")?;
        // "Startup apps" in Settings / Task Manager stores a separate
        // enabled/disabled flag. The user has just asked for launch at login,
        // so drop a stale "disabled" flag (the default is enabled).
        let status = unsafe {
            RegDeleteKeyValueW(
                HKEY_CURRENT_USER,
                w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run"),
                w!("Lunalyr"),
            )
        };
        if status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND {
            log::debug!("autostart: could not reset the StartupApproved flag: {}", describe(status));
        }
        log::info!("autostart: enabled ({command})");
    } else {
        let status = unsafe {
            RegDeleteKeyValueW(
                HKEY_CURRENT_USER,
                w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
                w!("Lunalyr"),
            )
        };
        if status != ERROR_FILE_NOT_FOUND {
            check(status, "could not remove Lunalyr from the programs started at login")?;
        }
        log::info!("autostart: disabled");
    }
    Ok(())
}

fn check(status: WIN32_ERROR, what: &str) -> PlatformResult<()> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(PlatformError::msg(format!("{what}: {}", describe(status))))
    }
}

fn describe(status: WIN32_ERROR) -> String {
    ::windows::core::Error::from(status.to_hresult()).to_string()
}
