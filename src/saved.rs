//! Remembers the displayed battery value between runs, so a restart doesn't jump.
//!
//! Our own copy lives in %APPDATA%\HyperlightTray\battery.ini. The official utility
//! keeps the same thing in HKCU\Software\DeviceBatteryOptimize as
//! `BatteryValue_<mouse id> = "<unix time>-<percent>"`. We read that too (never write
//! it) and start from whichever is newer, so both apps show the same number.
use std::{ffi::c_void, path::PathBuf, ptr};

use hyperlight::estimate::Saved;
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::*;

use crate::wide;

fn id_hex(id: [u8; 3]) -> String {
    format!("{:02x}{:02x}{:02x}", id[0], id[1], id[2])
}

fn parse(v: &str) -> Option<Saved> {
    let (t, p) = v.trim().split_once('-')?;
    Some(Saved { unix: t.trim().parse().ok()?, percent: p.trim().parse::<u8>().ok()?.min(100) })
}

fn file() -> Option<PathBuf> {
    crate::config::data_dir().map(|d| d.join("battery.ini"))
}

fn read_file() -> Vec<(String, Saved)> {
    let Some(text) = file().and_then(|p| std::fs::read_to_string(p).ok()) else { return vec![] };
    text.lines()
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            Some((k.trim().to_string(), parse(v)?))
        })
        .collect()
}

fn from_utility(id: [u8; 3]) -> Option<Saved> {
    let key = wide("Software\\DeviceBatteryOptimize");
    let name = wide(&format!("BatteryValue_{}", id_hex(id)));
    let mut buf = [0u16; 64];
    let mut size = (buf.len() * 2) as u32;
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            ptr::null_mut(),
            buf.as_mut_ptr() as *mut c_void,
            &mut size,
        )
    };
    if rc != ERROR_SUCCESS {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    parse(&String::from_utf16_lossy(&buf[..len]))
}

/// The most recently saved displayed value for this mouse, from either app.
pub fn load(id: [u8; 3]) -> Option<Saved> {
    let key = id_hex(id);
    let ours = read_file().into_iter().find(|(k, _)| *k == key).map(|(_, s)| s);
    match (ours, from_utility(id)) {
        (Some(a), Some(b)) => Some(if b.unix > a.unix { b } else { a }),
        (a, b) => a.or(b),
    }
}

/// Discharge history for the time-remaining estimate (discharge.txt).
pub fn load_discharge() -> hyperlight::remaining::Discharge {
    crate::config::data_dir()
        .and_then(|d| std::fs::read_to_string(d.join("discharge.txt")).ok())
        .map(|t| hyperlight::remaining::Discharge::from_text(&t))
        .unwrap_or_default()
}

pub fn store_discharge(d: &hyperlight::remaining::Discharge) {
    if let Some(dir) = crate::config::data_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("discharge.txt"), d.to_text());
    }
}

pub fn store(id: [u8; 3], s: Saved) {
    let Some(path) = file() else { return };
    let key = id_hex(id);
    let mut entries = read_file();
    entries.retain(|(k, _)| *k != key);
    entries.push((key, s));
    let text: String =
        entries.iter().map(|(k, s)| format!("{k}={}-{}\n", s.unix, s.percent)).collect();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, text);
}
