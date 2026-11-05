//! Draws the tray icon: the battery percentage as a number, tinted by state.
use std::{ffi::c_void, mem, ptr};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::UI::HiDpi::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::wide;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    /// 50% and up.
    Healthy,
    /// 20–49%.
    Moderate,
    /// Under 20%: time to charge.
    Low,
    /// Mouse asleep or off: last known reading.
    Stale,
}

impl Tone {
    pub fn for_percent(p: u8) -> Tone {
        match p {
            50.. => Tone::Healthy,
            20..=49 => Tone::Moderate,
            _ => Tone::Low,
        }
    }
}

/// 0xRRGGBB for a tone. Each colour has a light-taskbar and a dark-taskbar
/// version, picked so the digits keep at least about 4.5:1 contrast against the
/// taskbar Windows draws in that mode.
pub fn color(tone: Tone, light_taskbar: bool) -> u32 {
    match (tone, light_taskbar) {
        (Tone::Healthy, false) => 0x6CCB5F,
        (Tone::Healthy, true) => 0x0E700E,
        (Tone::Moderate, false) => 0xFFA94D,
        (Tone::Moderate, true) => 0xA85200,
        (Tone::Low, false) => 0xFF6B61,
        (Tone::Low, true) => 0xC42B1C,
        (Tone::Stale, false) => 0x8A8A8A,
        (Tone::Stale, true) => 0x6E6E6E,
    }
}

pub fn taskbar_is_light() -> bool {
    let mut value = 0u32;
    let mut size = 4u32;
    let key = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    let name = wide("SystemUsesLightTheme");
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            ptr::null_mut(),
            &mut value as *mut u32 as *mut c_void,
            &mut size,
        )
    };
    rc == ERROR_SUCCESS && value == 1
}

pub fn tray_icon_size() -> i32 {
    unsafe { GetSystemMetricsForDpi(SM_CXSMICON as _, GetDpiForSystem()).max(16) }
}

/// Renders `text` centred in a `size`×`size` square. Returns premultiplied BGRA
/// pixels, top row first.
///
/// The font size is found by drawing on a roomier scratch canvas and measuring the
/// pixels actually drawn, so nothing is ever clipped: font metrics alone put
/// Bahnschrift's digits too high and sized a lone digit so big its top was cut off.
/// Every number uses the digit height of "88", so 8 and 85 look the same size.
pub fn render_pixels(text: &str, rgb: u32, size: i32) -> Vec<u32> {
    let em = fit_em("88", size).min(fit_em(text, size));
    let canvas = size * 3;
    let cov = coverage(text, em, size);
    let mut out = vec![0u32; (size * size) as usize];
    let Some((l, t, r, b)) = ink_bounds(&cov, canvas) else { return out };
    // Centre the drawn pixels; a spare odd pixel goes above so it never looks raised.
    let (w, h) = (r - l + 1, b - t + 1);
    let ox = (size - w) / 2 - l;
    let oy = (size - h + 1) / 2 - t;
    let (cr, cg, cb) = ((rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF);
    for y in t..=b {
        for x in l..=r {
            let (nx, ny) = (x + ox, y + oy);
            if (0..size).contains(&nx) && (0..size).contains(&ny) {
                let a = cov[(y * canvas + x) as usize] as u32;
                out[(ny * size + nx) as usize] =
                    (a << 24) | ((cr * a / 255) << 16) | ((cg * a / 255) << 8) | (cb * a / 255);
            }
        }
    }
    out
}

/// Largest font size whose drawn pixels fit the icon: full width, and a pixel or
/// two short of full height so the digits don't touch the edges.
fn fit_em(text: &str, size: i32) -> i32 {
    let canvas = size * 3;
    let max_h = size - (size / 8).max(2);
    for em in (6..=size * 3 / 2).rev() {
        if let Some((l, t, r, b)) = ink_bounds(&coverage(text, em, size), canvas) {
            if r - l < size && b - t < max_h {
                return em;
            }
        }
    }
    6
}

/// Draws white text on black into a (3*size)² canvas, offset by `size` so nothing
/// clips, and returns per-pixel coverage (0..255).
fn coverage(text: &str, em: i32, size: i32) -> Vec<u8> {
    let canvas = size * 3;
    let n = (canvas * canvas) as usize;
    let mut out = vec![0u8; n];
    unsafe {
        let screen = GetDC(ptr::null_mut());
        let dc = CreateCompatibleDC(screen);
        ReleaseDC(ptr::null_mut(), screen);
        let mut bmi: BITMAPINFO = mem::zeroed();
        bmi.bmiHeader.biSize = mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = canvas;
        bmi.bmiHeader.biHeight = -canvas;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB as _;
        let mut bits: *mut c_void = ptr::null_mut();
        let dib = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
        if dib.is_null() || bits.is_null() {
            DeleteDC(dc);
            return out;
        }
        let old_bmp = SelectObject(dc, dib);
        ptr::write_bytes(bits as *mut u32, 0, n);
        let face = wide("Bahnschrift SemiBold SemiConden");
        let font = CreateFontW(
            -em, 0, 0, 0, FW_NORMAL as _, 0, 0, 0,
            DEFAULT_CHARSET as _, OUT_TT_PRECIS as _, CLIP_DEFAULT_PRECIS as _,
            ANTIALIASED_QUALITY as _, (DEFAULT_PITCH | FF_SWISS) as _, face.as_ptr(),
        );
        let old_font = SelectObject(dc, font);
        SetBkMode(dc, TRANSPARENT as _);
        SetTextColor(dc, 0x00FFFFFF);
        let w: Vec<u16> = text.encode_utf16().collect();
        TextOutW(dc, size / 2, size / 2, w.as_ptr(), w.len() as i32);
        GdiFlush();
        let src = std::slice::from_raw_parts(bits as *const u32, n);
        for (o, p) in out.iter_mut().zip(src) {
            *o = ((p >> 16) & 0xFF).max((p >> 8) & 0xFF).max(p & 0xFF) as u8;
        }
        SelectObject(dc, old_font);
        DeleteObject(font);
        SelectObject(dc, old_bmp);
        DeleteObject(dib);
        DeleteDC(dc);
    }
    out
}

/// Left, top, right, bottom of the clearly drawn pixels.
fn ink_bounds(cov: &[u8], canvas: i32) -> Option<(i32, i32, i32, i32)> {
    let (mut l, mut t, mut r, mut b) = (canvas, canvas, -1, -1);
    for y in 0..canvas {
        for x in 0..canvas {
            if cov[(y * canvas + x) as usize] > 40 {
                l = l.min(x);
                r = r.max(x);
                t = t.min(y);
                b = b.max(y);
            }
        }
    }
    (r >= 0).then_some((l, t, r, b))
}

pub fn make_icon(text: &str, rgb: u32, size: i32) -> HICON {
    let px = render_pixels(text, rgb, size);
    unsafe {
        let mut bmi: BITMAPINFO = mem::zeroed();
        bmi.bmiHeader.biSize = mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = size;
        bmi.bmiHeader.biHeight = -size;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB as _;
        let mut bits: *mut c_void = ptr::null_mut();
        let color = CreateDIBSection(ptr::null_mut(), &bmi, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
        if color.is_null() || bits.is_null() {
            return ptr::null_mut();
        }
        ptr::copy_nonoverlapping(px.as_ptr(), bits as *mut u32, px.len());
        let stride = ((size + 15) / 16 * 2) as usize;
        let mask_bits = vec![0u8; stride * size as usize];
        let mask = CreateBitmap(size, size, 1, 1, mask_bits.as_ptr() as *const c_void);
        let info = ICONINFO { fIcon: TRUE, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info);
        DeleteObject(mask);
        DeleteObject(color);
        icon
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rows that have visible pixels: (first, last).
    fn ink_rows(px: &[u32], size: i32) -> (i32, i32) {
        let rows: Vec<i32> = (0..size)
            .filter(|&y| (0..size).any(|x| (px[(y * size + x) as usize] >> 24) > 40))
            .collect();
        (rows[0], *rows.last().unwrap())
    }

    #[test]
    fn every_level_fits_without_clipping() {
        for size in [16, 20, 24, 32] {
            let (_, ref_h) = {
                let (t, b) = ink_rows(&render_pixels("88", 0xFFFFFF, size), size);
                (t, b - t)
            };
            for n in 0..=100 {
                let px = render_pixels(&n.to_string(), 0xFFFFFF, size);
                let (top, bottom) = ink_rows(&px, size);
                // A clear row above and below: nothing touches (or is cut by) the edge.
                assert!(top >= 1 && bottom <= size - 2, "{n} at {size}px: rows {top}..{bottom}");
                // Never taller than "88" (wide numbers may be a touch smaller to fit the
                // width), and single digits are exactly as tall as two-digit ones.
                assert!(bottom - top <= ref_h + 1, "{n} at {size}px is taller than 88");
                if n < 10 {
                    assert!(((bottom - top) - ref_h).abs() <= 1, "{n} at {size}px isn't as tall as 88");
                }
            }
        }
    }

    /// Writes sample icons as BMPs for eyeballing: `cargo test --bin hyperlight-tray -- --ignored`
    #[test]
    #[ignore]
    fn dump_samples() {
        let dir = std::env::temp_dir().join("hyperlight-tray-icons");
        std::fs::create_dir_all(&dir).unwrap();
        for size in [16, 20, 24, 32] {
            for (text, tone) in [("85", Tone::Healthy), ("35", Tone::Moderate), ("8", Tone::Low), ("–", Tone::Stale)] {
                for light in [false, true] {
                    let px = render_pixels(text, color(tone, light), size);
                    // Composite onto the taskbar colour so the file shows what users see.
                    let bg: u32 = if light { 0xEEEEEE } else { 0x202020 };
                    let mut data = Vec::new();
                    for p in &px {
                        let a = p >> 24;
                        for sh in [0, 8, 16] {
                            let fg = (p >> sh) & 0xFF;
                            let b = (bg >> sh) & 0xFF;
                            data.push((fg + b * (255 - a) / 255) as u8);
                        }
                        data.push(255);
                    }
                    let mut f = Vec::new();
                    let img = data.len() as u32;
                    f.extend_from_slice(b"BM");
                    f.extend_from_slice(&(54 + img).to_le_bytes());
                    f.extend_from_slice(&[0; 4]);
                    f.extend_from_slice(&54u32.to_le_bytes());
                    f.extend_from_slice(&40u32.to_le_bytes());
                    f.extend_from_slice(&size.to_le_bytes());
                    f.extend_from_slice(&(-size).to_le_bytes());
                    f.extend_from_slice(&1u16.to_le_bytes());
                    f.extend_from_slice(&32u16.to_le_bytes());
                    f.extend_from_slice(&[0; 24]);
                    f.extend_from_slice(&data);
                    let name = format!("{size}_{}_{:?}_{}.bmp", text.len(), tone, if light { "light" } else { "dark" });
                    std::fs::write(dir.join(name), f).unwrap();
                }
            }
        }
        println!("wrote samples to {}", dir.display());
    }
}
