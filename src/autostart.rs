//! "Start with Windows" via the per-user Run key (no admin rights needed).
use std::{ffi::c_void, ptr};

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::*;

use crate::wide;

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE: &str = "HitscanTray";

pub fn is_enabled() -> bool {
    let key = wide(RUN_KEY);
    let name = wide(VALUE);
    let mut size = 0u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut size,
        ) == ERROR_SUCCESS
    }
}

pub fn set(enabled: bool) {
    let key = wide(RUN_KEY);
    let name = wide(VALUE);
    unsafe {
        if !enabled {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr());
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
}

/// Points the Run entry at this exe, in case it was moved since it was enabled.
pub fn refresh() {
    if is_enabled() {
        set(true);
    }
}
