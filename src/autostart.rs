//! "Start with Windows" via the per-user Run key (no admin rights needed).
use std::{ffi::c_void, ptr};

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::*;

use crate::wide;

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE: &str = "HyperlightTray";
/// The Run value used when the app was called Hitscan Tray.
const OLD_VALUE: &str = "HitscanTray";
/// Windows' record of which Run entries are on or off (Task Manager's Startup tab).
const APPROVED_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run";
/// The record for "enabled": first byte 02, then an empty timestamp.
const APPROVED_ON: [u8; 12] = [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

pub fn is_enabled() -> bool {
    has_value(VALUE)
}

fn has_value(value: &str) -> bool {
    exists(RUN_KEY, value, RRF_RT_REG_SZ)
}

fn exists(key: &str, value: &str, kind: u32) -> bool {
    let key = wide(key);
    let name = wide(value);
    let mut size = 0u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            kind,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut size,
        ) == ERROR_SUCCESS
    }
}

/// Adds the "enabled" record for a Run entry that has none. Windows 11 build 26300
/// skipped our Run entry at sign-in while running every entry that had a record. An
/// existing record is left alone: it may mean the user turned us off in Windows.
fn ensure_approved(key: &str, value: &str) {
    if exists(key, value, RRF_RT_REG_BINARY) {
        return;
    }
    let (key, name) = (wide(key), wide(value));
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            REG_BINARY,
            APPROVED_ON.as_ptr() as *const c_void,
            APPROVED_ON.len() as u32,
        );
    }
}

pub fn set(enabled: bool) {
    let key = wide(RUN_KEY);
    let name = wide(VALUE);
    unsafe {
        if !enabled {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr());
            RegDeleteKeyValueW(HKEY_CURRENT_USER, wide(APPROVED_KEY).as_ptr(), name.as_ptr());
            return;
        }
        let Ok(exe) = std::env::current_exe() else { return };
        let cmd = wide(&format!("\"{}\"", exe.display()));
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            REG_SZ,
            cmd.as_ptr() as *const c_void,
            (cmd.len() * 2) as u32,
        );
    }
    ensure_approved(APPROVED_KEY, VALUE);
}

/// Points the Run entry at this exe, in case it was moved since it was enabled,
/// and carries over an entry left by the old Hitscan Tray name.
pub fn refresh() {
    if has_value(OLD_VALUE) {
        let key = wide(RUN_KEY);
        let name = wide(OLD_VALUE);
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) };
        set(true);
    } else if is_enabled() {
        set(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(key: &str, value: &str) -> Option<Vec<u8>> {
        let (key, name) = (wide(key), wide(value));
        let mut data = [0u8; 16];
        let mut size = data.len() as u32;
        let rc = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_BINARY,
                ptr::null_mut(),
                data.as_mut_ptr() as *mut c_void,
                &mut size,
            )
        };
        (rc == ERROR_SUCCESS).then(|| data[..size as usize].to_vec())
    }

    /// Uses a scratch key, not the real startup list.
    #[test]
    fn adds_a_missing_record_but_keeps_an_existing_one() {
        let key = "Software\\HyperlightTray.Test\\StartupApproved";
        let (k, off) = (wide(key), wide("Off"));

        ensure_approved(key, "New");
        assert_eq!(read(key, "New").as_deref(), Some(&APPROVED_ON[..]));

        // Turned off in Task Manager: first byte 03 and a timestamp.
        let disabled = [3u8, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8];
        unsafe {
            RegSetKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), off.as_ptr(), REG_BINARY, disabled.as_ptr() as *const c_void, 12);
        }
        ensure_approved(key, "Off");
        assert_eq!(read(key, "Off").as_deref(), Some(&disabled[..]));

        unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, wide("Software\\HyperlightTray.Test").as_ptr()) };
    }
}
