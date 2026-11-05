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
    Normal,
    Low,
    Charging,
    Stale,
}

/// 0xRRGGBB for a tone on a light or dark taskbar.
pub fn color(tone: Tone, light_taskbar: bool) -> u32 {
    match (tone, light_taskbar) {
        (Tone::Normal, false) => 0xFFFFFF,
        (Tone::Normal, true) => 0x1A1A1A,
        (Tone::Low, false) => 0xFF5F57,
        (Tone::Low, true) => 0xC42B1C,
        (Tone::Charging, false) => 0x6CCB5F,
        (Tone::Charging, true) => 0x0F7B0F,
        (Tone::Stale, false) => 0x8A8A8A,
        (Tone::Stale, true) => 0x767676,
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
pub fn render_pixels(text: &str, rgb: u32, size: i32) -> Vec<u32> {
    let n = (size * size) as usize;
    let mut out = vec![0u32; n];
    unsafe {
        let screen = GetDC(ptr::null_mut());
        let dc = CreateCompatibleDC(screen);
        let mut bmi: BITMAPINFO = mem::zeroed();
        bmi.bmiHeader.biSize = mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = size;
        bmi.bmiHeader.biHeight = -size;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB as _;
        let mut bits: *mut c_void = ptr::null_mut();
        let dib = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
        if dib.is_null() || bits.is_null() {
            DeleteDC(dc);
            ReleaseDC(ptr::null_mut(), screen);
            return out;
        }
        let old_bmp = SelectObject(dc, dib);
        ptr::write_bytes(bits as *mut u32, 0, n);

        let text_w: Vec<u16> = text.encode_utf16().collect();
        let face = wide("Bahnschrift SemiBold SemiConden");
        let mut em = size * 5 / 4;
        let (font, cap, ascent, width) = loop {
            let font = CreateFontW(
                -em, 0, 0, 0, FW_NORMAL as _, 0, 0, 0,
                DEFAULT_CHARSET as _, OUT_TT_PRECIS as _, CLIP_DEFAULT_PRECIS as _,
                ANTIALIASED_QUALITY as _, (DEFAULT_PITCH | FF_SWISS) as _, face.as_ptr(),
            );
            let prev = SelectObject(dc, font);
            let mut ext: SIZE = mem::zeroed();
            GetTextExtentPoint32W(dc, text_w.as_ptr(), text_w.len() as i32, &mut ext);
            let mut tm: TEXTMETRICW = mem::zeroed();
            GetTextMetricsW(dc, &mut tm);
            let cap = cap_height(dc).unwrap_or(em * 7 / 10);
            if (ext.cx <= size && cap <= size - 2) || em <= 6 {
                break (font, cap, tm.tmAscent, ext.cx);
            }
            SelectObject(dc, prev);
            DeleteObject(font);
            em -= 1;
        };
        SetBkMode(dc, TRANSPARENT as _);
        SetTextColor(dc, 0x00FFFFFF);
        let baseline = (size + cap + 1) / 2;
        TextOutW(dc, (size - width) / 2, baseline - ascent, text_w.as_ptr(), text_w.len() as i32);
        GdiFlush();

        // White-on-black coverage becomes alpha; tint with the requested colour.
        let src = std::slice::from_raw_parts(bits as *const u32, n);
        let (r, g, b) = ((rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF);
        for (o, p) in out.iter_mut().zip(src) {
            let a = ((p >> 16) & 0xFF).max((p >> 8) & 0xFF).max(p & 0xFF);
            *o = (a << 24) | ((r * a / 255) << 16) | ((g * a / 255) << 8) | (b * a / 255);
        }

        SelectObject(dc, old_bmp);
        DeleteObject(font);
        DeleteObject(dib);
        DeleteDC(dc);
        ReleaseDC(ptr::null_mut(), screen);
    }
    out
}

unsafe fn cap_height(dc: HDC) -> Option<i32> {
    let need = GetOutlineTextMetricsW(dc, 0, ptr::null_mut());
    if need == 0 {
        return None;
    }
    let mut buf = vec![0u64; (need as usize + 7) / 8];
    let otm = buf.as_mut_ptr() as *mut OUTLINETEXTMETRICW;
    if GetOutlineTextMetricsW(dc, need, otm) == 0 {
        return None;
    }
    let cap = (*otm).otmsCapEmHeight as i32;
    (cap > 0).then_some(cap)
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

    /// Writes sample icons as BMPs for eyeballing: `cargo test --bin hitscantray -- --ignored`
    #[test]
    #[ignore]
    fn dump_samples() {
        let dir = std::env::temp_dir().join("hitscantray-icons");
        std::fs::create_dir_all(&dir).unwrap();
        for size in [16, 20, 24, 32] {
            for (text, tone) in [("25", Tone::Normal), ("8", Tone::Low), ("100", Tone::Charging), ("–", Tone::Stale)] {
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
