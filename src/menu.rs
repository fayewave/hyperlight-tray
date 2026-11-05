//! The tray menu: a small custom-drawn popup instead of a native Win32 menu, so it
//! can have icons, a rounded dark panel, soft hover highlights and checkboxes,
//! matching the configurator's look. Icons come from the Windows icon font
//! (Segoe Fluent Icons on Windows 11, Segoe MDL2 Assets on Windows 10).
//!
//! It closes when you click an action, press Esc or click anywhere else. Toggles
//! flip in place and leave it open. Chosen items are posted to the owner window as
//! `WM_MENU_COMMAND` with the item id in `wParam`.

use std::cell::RefCell;
use std::time::{Duration, Instant};
use std::{mem, ptr};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::wide;

pub const WM_MENU_COMMAND: u32 = WM_APP + 5;
const WM_MOUSELEAVE: u32 = 0x02A3;
const CLASS: &str = "HyperlightTrayMenu";

// Icon font code points.
pub const ICON_MOUSE: u16 = 0xE962;
pub const ICON_REFRESH: u16 = 0xE72C;
pub const ICON_SLIDERS: u16 = 0xE9E9;
pub const ICON_CLOCK: u16 = 0xE823;
pub const ICON_STARTUP: u16 = 0xE7B5;
pub const ICON_POWER: u16 = 0xE7E8;
pub const ICON_CHARGING: u16 = 0xEA93;
const ICON_CHECK: u16 = 0xE73E;

/// Battery glyph for a level: Battery0..Battery9 are E850..E859, Battery10 is E83F.
pub fn battery_icon(percent: u8) -> u16 {
    match percent / 10 {
        10.. => 0xE83F,
        n => 0xE850 + n as u16,
    }
}

pub enum Item {
    /// Small heading at the top.
    Title { icon: u16, text: String },
    /// Read-only status: a main line and a quieter second line.
    Status { icon: u16, text: String, detail: String },
    Action { id: usize, icon: u16, text: String },
    Toggle { id: usize, icon: u16, text: String, on: bool },
    Separator,
}

impl Item {
    fn interactive(&self) -> bool {
        matches!(self, Item::Action { .. } | Item::Toggle { .. })
    }
    /// Height at 96 DPI.
    fn height(&self) -> i32 {
        match self {
            Item::Title { .. } => 30,
            Item::Status { detail, .. } if !detail.is_empty() => 46,
            Item::Status { .. } => 32,
            Item::Action { .. } | Item::Toggle { .. } => 34,
            Item::Separator => 9,
        }
    }
}

struct Palette {
    bg: u32,
    border: u32,
    hover: u32,
    text: u32,
    muted: u32,
    line: u32,
    box_off: u32,
    box_on: u32,
    check: u32,
}

const DARK: Palette = Palette {
    bg: 0x16191B,
    border: 0x353D41,
    hover: 0x282E30,
    text: 0xF4F5F6,
    muted: 0x9BA3A7,
    line: 0x2C3032,
    box_off: 0x5A6468,
    box_on: 0xE5E7E9,
    check: 0x171B1E,
};

const LIGHT: Palette = Palette {
    bg: 0xFBFBFC,
    border: 0xD3D7DB,
    hover: 0xE9ECEE,
    text: 0x121517,
    muted: 0x626A6F,
    line: 0xE1E4E7,
    box_off: 0x8A949A,
    box_on: 0x171B1E,
    check: 0xF4F5F6,
};

/// 0xRRGGBB to a GDI COLORREF (0x00BBGGRR).
fn cref(rgb: u32) -> u32 {
    ((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF)
}

struct Fonts {
    text: HFONT,
    bold: HFONT,
    small: HFONT,
    icon: HFONT,
    icon_small: HFONT,
}

impl Drop for Fonts {
    fn drop(&mut self) {
        unsafe {
            for f in [self.text, self.bold, self.small, self.icon, self.icon_small] {
                DeleteObject(f);
            }
        }
    }
}

struct Menu {
    hwnd: HWND,
    owner: HWND,
    items: Vec<Item>,
    hover: Option<usize>,
    tracking: bool,
    scale: f32,
    width: i32,
    palette: &'static Palette,
    fonts: Fonts,
}

thread_local! {
    static MENU: RefCell<Option<Menu>> = const { RefCell::new(None) };
    static LAST_CLOSED: RefCell<Option<Instant>> = const { RefCell::new(None) };
}

fn with_menu<R>(f: impl FnOnce(&mut Menu) -> R) -> Option<R> {
    MENU.with(|m| m.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

impl Menu {
    fn px(&self, v: i32) -> i32 {
        (v as f32 * self.scale).round() as i32
    }
    fn pad(&self) -> i32 {
        self.px(6)
    }
    /// Top and bottom of each item, in client pixels.
    fn rows(&self) -> Vec<(i32, i32)> {
        let mut y = self.pad();
        self.items
            .iter()
            .map(|it| {
                let h = self.px(it.height());
                let r = (y, y + h);
                y += h;
                r
            })
            .collect()
    }
    fn height(&self) -> i32 {
        self.rows().last().map(|r| r.1).unwrap_or(0) + self.pad()
    }
    fn hit(&self, y: i32) -> Option<usize> {
        self.rows().iter().position(|&(t, b)| y >= t && y < b).filter(|&i| self.items[i].interactive())
    }
}

/// Opens the menu with its bottom-right corner at the cursor (flipping to stay on
/// screen). Clicking the tray icon again while it's open just closes it.
pub fn show(owner: HWND, items: Vec<Item>) {
    let just_closed = LAST_CLOSED.with(|c| c.borrow().is_some_and(|t| t.elapsed() < Duration::from_millis(250)));
    if just_closed || MENU.with(|m| m.borrow().is_some()) {
        return;
    }
    unsafe {
        let hinst = GetModuleHandleW(ptr::null());
        let class = wide(CLASS);
        let mut wc: WNDCLASSW = mem::zeroed();
        wc.lpfnWndProc = Some(wndproc);
        wc.hInstance = hinst;
        wc.lpszClassName = class.as_ptr();
        wc.hCursor = LoadCursorW(ptr::null_mut(), IDC_ARROW);
        RegisterClassW(&wc); // fails harmlessly once registered

        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let (mut dx, mut dy) = (96u32, 96u32);
        GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        let scale = dx as f32 / 96.0;
        let palette = if crate::icon::taskbar_is_light() { &LIGHT } else { &DARK };
        let fonts = make_fonts(scale);

        let mut menu = Menu {
            hwnd: ptr::null_mut(),
            owner,
            items,
            hover: None,
            tracking: false,
            scale,
            width: 0,
            palette,
            fonts,
        };
        menu.width = measure_width(&menu);
        let (w, h) = (menu.width, menu.height());

        let mut mi: MONITORINFO = mem::zeroed();
        mi.cbSize = mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(mon, &mut mi);
        let work = mi.rcWork;
        let mut x = if pt.x - w >= work.left { pt.x - w } else { pt.x };
        let mut y = if pt.y - h >= work.top { pt.y - h } else { pt.y };
        x = x.clamp(work.left, (work.right - w).max(work.left));
        y = y.clamp(work.top, (work.bottom - h).max(work.top));

        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            class.as_ptr(),
            wide("Hyperlight Tray menu").as_ptr(),
            WS_POPUP,
            x,
            y,
            w,
            h,
            owner,
            ptr::null_mut(),
            hinst,
            ptr::null(),
        );
        if hwnd.is_null() {
            return;
        }
        // Windows 11: rounded corners (which also brings the system shadow) and a
        // border in our palette. Ignored on Windows 10.
        let round: i32 = 2; // DWMWCP_ROUND
        DwmSetWindowAttribute(hwnd, 33, &round as *const i32 as _, 4);
        let border = cref(palette.border);
        DwmSetWindowAttribute(hwnd, 34, &border as *const u32 as _, 4);

        menu.hwnd = hwnd;
        MENU.with(|m| *m.borrow_mut() = Some(menu));
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
    }
}

unsafe fn make_fonts(scale: f32) -> Fonts {
    let font = |px: f32, weight: i32, face: &str| {
        let f = wide(face);
        CreateFontW(
            -(px * scale).round() as i32,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET as _,
            OUT_TT_PRECIS as _,
            CLIP_DEFAULT_PRECIS as _,
            CLEARTYPE_QUALITY as _,
            (DEFAULT_PITCH | FF_DONTCARE) as _,
            f.as_ptr(),
        )
    };
    let icon_face = if font_exists("Segoe Fluent Icons") { "Segoe Fluent Icons" } else { "Segoe MDL2 Assets" };
    Fonts {
        text: font(14.0, FW_NORMAL as i32, "Segoe UI"),
        bold: font(14.0, FW_SEMIBOLD as i32, "Segoe UI"),
        small: font(12.0, FW_NORMAL as i32, "Segoe UI"),
        icon: font(16.0, FW_NORMAL as i32, icon_face),
        icon_small: font(12.0, FW_NORMAL as i32, icon_face),
    }
}

/// Whether GDI really gives us this face (it silently substitutes otherwise).
unsafe fn font_exists(face: &str) -> bool {
    let f = wide(face);
    let font = CreateFontW(-16, 0, 0, 0, 400, 0, 0, 0, DEFAULT_CHARSET as _, 0, 0, 0, 0, f.as_ptr());
    let dc = GetDC(ptr::null_mut());
    let old = SelectObject(dc, font);
    let mut buf = [0u16; 64];
    let n = GetTextFaceW(dc, buf.len() as i32, buf.as_mut_ptr());
    SelectObject(dc, old);
    ReleaseDC(ptr::null_mut(), dc);
    DeleteObject(font);
    String::from_utf16_lossy(&buf[..n.max(1) as usize - 1]).eq_ignore_ascii_case(face)
}

unsafe fn text_width(dc: HDC, font: HFONT, s: &str) -> i32 {
    let w: Vec<u16> = s.encode_utf16().collect();
    let old = SelectObject(dc, font);
    let mut size: SIZE = mem::zeroed();
    GetTextExtentPoint32W(dc, w.as_ptr(), w.len() as i32, &mut size);
    SelectObject(dc, old);
    size.cx
}

unsafe fn measure_width(m: &Menu) -> i32 {
    let dc = GetDC(ptr::null_mut());
    let mut widest = 0;
    for it in &m.items {
        let w = match it {
            Item::Title { text, .. } => text_width(dc, m.fonts.bold, text),
            Item::Status { text, detail, .. } => {
                text_width(dc, m.fonts.bold, text).max(text_width(dc, m.fonts.small, detail))
            }
            Item::Action { text, .. } => text_width(dc, m.fonts.text, text),
            Item::Toggle { text, .. } => text_width(dc, m.fonts.text, text) + m.px(40),
            Item::Separator => 0,
        };
        widest = widest.max(w);
    }
    ReleaseDC(ptr::null_mut(), dc);
    // padding + icon column + text + right margin
    (m.pad() + m.px(44) + widest + m.px(22) + m.pad()).max(m.px(250))
}

unsafe fn fill(dc: HDC, r: RECT, rgb: u32) {
    let b = CreateSolidBrush(cref(rgb));
    FillRect(dc, &r, b);
    DeleteObject(b);
}

unsafe fn round_rect(dc: HDC, r: RECT, radius: i32, fill_rgb: Option<u32>, line_rgb: Option<u32>, line_w: i32) {
    let brush = fill_rgb.map(|c| CreateSolidBrush(cref(c)));
    let pen = line_rgb.map(|c| CreatePen(PS_SOLID, line_w, cref(c)));
    let old_b = SelectObject(dc, brush.unwrap_or(GetStockObject(NULL_BRUSH)));
    let old_p = SelectObject(dc, pen.unwrap_or(GetStockObject(NULL_PEN)));
    RoundRect(dc, r.left, r.top, r.right, r.bottom, radius * 2, radius * 2);
    SelectObject(dc, old_b);
    SelectObject(dc, old_p);
    if let Some(b) = brush {
        DeleteObject(b);
    }
    if let Some(p) = pen {
        DeleteObject(p);
    }
}

unsafe fn text(dc: HDC, font: HFONT, rgb: u32, s: &str, mut r: RECT, flags: u32) {
    let mut w: Vec<u16> = s.encode_utf16().collect();
    SelectObject(dc, font);
    SetTextColor(dc, cref(rgb));
    DrawTextW(dc, w.as_mut_ptr(), w.len() as i32, &mut r, flags | DT_SINGLELINE | DT_NOPREFIX);
}

unsafe fn glyph(dc: HDC, font: HFONT, rgb: u32, cp: u16, r: RECT) {
    let s = String::from_utf16_lossy(&[cp]);
    text(dc, font, rgb, &s, r, DT_CENTER | DT_VCENTER);
}

unsafe fn paint(m: &Menu, dc: HDC) {
    let p = m.palette;
    let (w, h) = (m.width, m.height());
    fill(dc, RECT { left: 0, top: 0, right: w, bottom: h }, p.bg);
    SetBkMode(dc, TRANSPARENT as _);

    let pad = m.pad();
    let icon_x = pad + m.px(8);
    let icon_w = m.px(22);
    let text_x = pad + m.px(44);
    let right = w - pad;
    for (i, (it, &(top, bottom))) in m.items.iter().zip(m.rows().iter()).enumerate() {
        let row = RECT { left: pad, top, right, bottom };
        let icon_box = RECT { left: icon_x, top, right: icon_x + icon_w, bottom };
        let text_box = RECT { left: text_x, top, right: right - m.px(12), bottom };
        if m.hover == Some(i) {
            round_rect(dc, row, m.px(6), Some(p.hover), None, 0);
        }
        match it {
            Item::Separator => {
                let y = (top + bottom) / 2;
                fill(dc, RECT { left: pad + m.px(8), top: y, right: right - m.px(8), bottom: y + m.px(1).max(1) }, p.line);
            }
            Item::Title { icon, text: t } => {
                glyph(dc, m.fonts.icon_small, p.muted, *icon, icon_box);
                text(dc, m.fonts.bold, p.muted, t, text_box, DT_VCENTER | DT_END_ELLIPSIS);
            }
            Item::Status { icon, text: t, detail } => {
                glyph(dc, m.fonts.icon, p.text, *icon, icon_box);
                if detail.is_empty() {
                    text(dc, m.fonts.bold, p.text, t, text_box, DT_VCENTER | DT_END_ELLIPSIS);
                } else {
                    let mid = top + (bottom - top) / 2;
                    let upper = RECT { top: top + m.px(4), bottom: mid + m.px(2), ..text_box };
                    let lower = RECT { top: mid, bottom: bottom - m.px(4), ..text_box };
                    text(dc, m.fonts.bold, p.text, t, upper, DT_VCENTER | DT_END_ELLIPSIS);
                    text(dc, m.fonts.small, p.muted, detail, lower, DT_VCENTER | DT_END_ELLIPSIS);
                }
            }
            Item::Action { icon, text: t, .. } => {
                glyph(dc, m.fonts.icon, p.text, *icon, icon_box);
                text(dc, m.fonts.text, p.text, t, text_box, DT_VCENTER | DT_END_ELLIPSIS);
            }
            Item::Toggle { icon, text: t, on, .. } => {
                glyph(dc, m.fonts.icon, p.text, *icon, icon_box);
                text(dc, m.fonts.text, p.text, t, text_box, DT_VCENTER | DT_END_ELLIPSIS);
                // Checkbox on the right, like the configurator's switches: filled when on.
                let s = m.px(18);
                let bx = right - m.px(12) - s;
                let by = (top + bottom - s) / 2;
                let b = RECT { left: bx, top: by, right: bx + s, bottom: by + s };
                if *on {
                    round_rect(dc, b, m.px(4), Some(p.box_on), None, 0);
                    glyph(dc, m.fonts.icon_small, p.check, ICON_CHECK, b);
                } else {
                    round_rect(dc, b, m.px(4), None, Some(p.box_off), m.px(1).max(1));
                }
            }
        }
    }
}

fn close(hwnd: HWND) {
    unsafe { DestroyWindow(hwnd) };
}

/// Runs the item: actions close the menu, toggles flip and stay open.
fn activate(index: usize) {
    let Some((hwnd, owner, id, is_toggle)) = with_menu(|m| {
        let (id, toggle) = match &mut m.items[index] {
            Item::Action { id, .. } => (*id, false),
            Item::Toggle { id, on, .. } => {
                *on = !*on;
                (*id, true)
            }
            _ => return None,
        };
        Some((m.hwnd, m.owner, id, toggle))
    })
    .flatten() else {
        return;
    };
    unsafe {
        if is_toggle {
            InvalidateRect(hwnd, ptr::null(), 0);
        } else {
            close(hwnd);
        }
        PostMessageW(owner, WM_MENU_COMMAND, id, 0);
    }
}

fn move_hover(step: isize) {
    with_menu(|m| {
        let n = m.items.len() as isize;
        let mut i = m.hover.map(|h| h as isize).unwrap_or(if step > 0 { -1 } else { n });
        for _ in 0..n {
            i = (i + step).rem_euclid(n);
            if m.items[i as usize].interactive() {
                m.hover = Some(i as usize);
                break;
            }
        }
        unsafe { InvalidateRect(m.hwnd, ptr::null(), 0) };
    });
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = mem::zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            with_menu(|m| {
                // Draw off-screen, then copy, so hover changes don't flicker.
                let (w, h) = (m.width, m.height());
                let mem_dc = CreateCompatibleDC(dc);
                let bmp = CreateCompatibleBitmap(dc, w, h);
                let old = SelectObject(mem_dc, bmp);
                paint(m, mem_dc);
                BitBlt(dc, 0, 0, w, h, mem_dc, 0, 0, SRCCOPY);
                SelectObject(mem_dc, old);
                DeleteObject(bmp);
                DeleteDC(mem_dc);
            });
            EndPaint(hwnd, &ps);
            0
        }
        WM_ERASEBKGND => 1,
        WM_MOUSEMOVE => {
            let y = ((lp >> 16) & 0xFFFF) as i16 as i32;
            with_menu(|m| {
                if !m.tracking {
                    let mut t = TRACKMOUSEEVENT {
                        cbSize: mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    TrackMouseEvent(&mut t);
                    m.tracking = true;
                }
                let hit = m.hit(y);
                if hit != m.hover {
                    m.hover = hit;
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
            });
            0
        }
        WM_MOUSELEAVE => {
            with_menu(|m| {
                m.tracking = false;
                if m.hover.take().is_some() {
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
            });
            0
        }
        WM_LBUTTONUP => {
            let y = ((lp >> 16) & 0xFFFF) as i16 as i32;
            if let Some(i) = with_menu(|m| m.hit(y)).flatten() {
                activate(i);
            }
            0
        }
        WM_KEYDOWN => {
            match wp as u16 {
                0x26 => move_hover(-1), // Up
                0x28 => move_hover(1),  // Down
                0x0D | 0x20 => {
                    // Enter / Space
                    if let Some(i) = with_menu(|m| m.hover).flatten() {
                        activate(i);
                    }
                }
                0x1B => close(hwnd), // Esc
                _ => {}
            }
            0
        }
        WM_ACTIVATE => {
            if (wp & 0xFFFF) as u32 == WA_INACTIVE {
                close(hwnd);
            }
            0
        }
        WM_DESTROY => {
            MENU.with(|m| m.borrow_mut().take());
            LAST_CLOSED.with(|c| *c.borrow_mut() = Some(Instant::now()));
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
