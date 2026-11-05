//! The tray menu: a small custom-drawn popup instead of a native Win32 menu, so it
//! can have icons, a rounded panel with a soft shadow, hover highlights,
//! checkboxes and a fade/slide animation. Icons come from the Windows icon font
//! (Segoe Fluent Icons on Windows 11, Segoe MDL2 Assets on Windows 10).
//!
//! Drawing: the panel's contents are drawn with GDI into an opaque buffer, then
//! composited into a per-pixel-alpha image with anti-aliased rounded corners, a
//! border and a shadow, and shown with UpdateLayeredWindow. The fade only changes
//! the overall alpha and position of that image, so it's cheap.
//!
//! It opens centred above the cursor, closes when you pick an action, press Esc or
//! click anywhere else, and toggles flip in place. Chosen items are posted to the
//! owner window as `WM_MENU_COMMAND` with the item id in `wParam`.

use std::cell::RefCell;
use std::time::{Duration, Instant};
use std::{mem, ptr};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::wide;

pub const WM_MENU_COMMAND: u32 = WM_APP + 5;
const WM_MOUSELEAVE: u32 = 0x02A3;
const CLASS: &str = "HyperlightTrayMenu";
const TIMER_ANIM: usize = 1;
const OPEN_MS: f32 = 150.0;
const CLOSE_MS: f32 = 110.0;
/// How far the menu rises while fading in, at 96 DPI.
const SLIDE_PX: f32 = 8.0;

// Icon font code points.
pub const ICON_MOUSE: u16 = 0xE962;
pub const ICON_REFRESH: u16 = 0xE72C;
pub const ICON_SLIDERS: u16 = 0xE9E9;
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
    /// Status: a main line and a quieter second line, with an optional small icon
    /// button on the right (id, icon) that runs without closing the menu.
    Status { icon: u16, text: String, detail: String, button: Option<(usize, u16)> },
    Action { id: usize, icon: u16, text: String },
    Toggle { id: usize, icon: u16, text: String, on: bool },
    Separator,
}

impl Item {
    fn interactive(&self) -> bool {
        matches!(self, Item::Action { .. } | Item::Toggle { .. } | Item::Status { button: Some(_), .. })
    }
    /// Height at 96 DPI.
    fn height(&self) -> i32 {
        match self {
            Item::Title { .. } => 26,
            Item::Status { detail, .. } if !detail.is_empty() => 40,
            Item::Status { .. } => 28,
            Item::Action { .. } | Item::Toggle { .. } => 30,
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
    shadow: f32,
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
    shadow: 0.38,
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
    shadow: 0.20,
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
    /// The battery glyph is wide and flat, so the status row uses a bigger size.
    icon_large: HFONT,
}

impl Drop for Fonts {
    fn drop(&mut self) {
        unsafe {
            for f in [self.text, self.bold, self.small, self.icon, self.icon_small, self.icon_large] {
                DeleteObject(f);
            }
        }
    }
}

/// A 32-bit top-down DIB selected into its own memory DC.
struct Surface {
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
    w: i32,
    h: i32,
}

impl Surface {
    unsafe fn new(w: i32, h: i32) -> Option<Surface> {
        let screen = GetDC(ptr::null_mut());
        let dc = CreateCompatibleDC(screen);
        ReleaseDC(ptr::null_mut(), screen);
        let mut bmi: BITMAPINFO = mem::zeroed();
        bmi.bmiHeader.biSize = mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = w;
        bmi.bmiHeader.biHeight = -h;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB as _;
        let mut bits: *mut core::ffi::c_void = ptr::null_mut();
        let bmp = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
        if bmp.is_null() || bits.is_null() {
            DeleteDC(dc);
            return None;
        }
        let old = SelectObject(dc, bmp);
        Some(Surface { dc, bmp, old, bits: bits as *mut u32, w, h })
    }
    fn pixels(&mut self) -> &mut [u32] {
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.w * self.h) as usize) }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bmp);
            DeleteDC(self.dc);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Opening(Instant),
    Open,
    Closing(Instant),
}

struct Menu {
    hwnd: HWND,
    owner: HWND,
    items: Vec<Item>,
    hover: Option<usize>,
    tracking: bool,
    scale: f32,
    /// Panel size, and the shadow margin around it.
    width: i32,
    height: i32,
    margin: i32,
    /// Where the panel's top-left corner sits once fully open (screen pixels).
    x: i32,
    y: i32,
    phase: Phase,
    palette: &'static Palette,
    fonts: Fonts,
    /// The finished, alpha-composited image (panel plus shadow).
    image: Option<Surface>,
}

thread_local! {
    static MENU: RefCell<Option<Menu>> = const { RefCell::new(None) };
    static LAST_CLOSED: RefCell<Option<Instant>> = const { RefCell::new(None) };
}

fn with_menu<R>(f: impl FnOnce(&mut Menu) -> R) -> Option<R> {
    MENU.with(|m| m.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

impl Menu {
    fn px(&self, v: f32) -> i32 {
        (v * self.scale).round() as i32
    }
    fn pad(&self) -> i32 {
        self.px(5.0)
    }
    /// Top and bottom of each item, in panel pixels.
    fn rows(&self) -> Vec<(i32, i32)> {
        let mut y = self.pad();
        self.items
            .iter()
            .map(|it| {
                let h = self.px(it.height() as f32);
                let r = (y, y + h);
                y += h;
                r
            })
            .collect()
    }
    fn content_height(&self) -> i32 {
        self.rows().last().map(|r| r.1).unwrap_or(0) + self.pad()
    }
    /// The interactive item under a point in window coordinates.
    fn hit(&self, x: i32, y: i32) -> Option<usize> {
        let (x, y) = (x - self.margin, y - self.margin);
        if x < 0 || x >= self.width {
            return None;
        }
        let i = self.rows().iter().position(|&(t, b)| y >= t && y < b).filter(|&i| self.items[i].interactive())?;
        if matches!(self.items[i], Item::Status { .. }) {
            let (l, _, r, _) = self.button_rect(self.rows()[i]);
            return (x >= l - self.px(2.0) && x < r + self.px(2.0)).then_some(i);
        }
        Some(i)
    }
    /// The small square button at the right of a status row (panel pixels).
    fn button_rect(&self, (top, bottom): (i32, i32)) -> (i32, i32, i32, i32) {
        let s = self.px(26.0);
        let r = self.width - self.pad() - self.px(5.0);
        let t = (top + bottom - s) / 2;
        (r - s, t, r, t + s)
    }
}

/// Opens the menu centred above the cursor (below it if there's no room).
/// Clicking the tray icon again while it's open just closes it.
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

        let mut menu = Menu {
            hwnd: ptr::null_mut(),
            owner,
            items,
            hover: None,
            tracking: false,
            scale,
            width: 0,
            height: 0,
            margin: 0,
            x: 0,
            y: 0,
            phase: Phase::Opening(Instant::now()),
            palette,
            fonts: make_fonts(scale),
            image: None,
        };
        menu.margin = menu.px(16.0);
        menu.width = measure_width(&menu);
        menu.height = menu.content_height();

        let mut mi: MONITORINFO = mem::zeroed();
        mi.cbSize = mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(mon, &mut mi);
        let work = mi.rcWork;
        let gap = menu.px(10.0);
        let (w, h) = (menu.width, menu.height);
        let x = (pt.x - w / 2).clamp(work.left + gap, (work.right - w - gap).max(work.left));
        let y = if pt.y - gap - h >= work.top { pt.y - gap - h } else { pt.y + gap };
        menu.x = x;
        menu.y = y.clamp(work.top, (work.bottom - h).max(work.top));

        let m = menu.margin;
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            class.as_ptr(),
            wide("Hyperlight Tray menu").as_ptr(),
            WS_POPUP,
            menu.x - m,
            menu.y - m,
            w + 2 * m,
            h + 2 * m,
            owner,
            ptr::null_mut(),
            hinst,
            ptr::null(),
        );
        if hwnd.is_null() {
            return;
        }
        menu.hwnd = hwnd;
        render(&mut menu);
        present(&menu, 0.0, menu.px(SLIDE_PX));
        MENU.with(|c| *c.borrow_mut() = Some(menu));
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        SetTimer(hwnd, TIMER_ANIM, 10, None);
    }
}

unsafe fn make_fonts(scale: f32) -> Fonts {
    // Text uses ClearType; icons use plain anti-aliasing, which keeps thin icon
    // strokes smooth instead of jagged.
    let font = |px: f32, weight: i32, face: &str| {
        let quality = if face.starts_with("Segoe UI") { CLEARTYPE_QUALITY } else { ANTIALIASED_QUALITY };
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
            quality as _,
            (DEFAULT_PITCH | FF_DONTCARE) as _,
            f.as_ptr(),
        )
    };
    let icon_face = if font_exists("Segoe Fluent Icons") { "Segoe Fluent Icons" } else { "Segoe MDL2 Assets" };
    Fonts {
        text: font(13.0, FW_NORMAL as i32, "Segoe UI"),
        bold: font(13.0, FW_SEMIBOLD as i32, "Segoe UI"),
        small: font(11.5, FW_NORMAL as i32, "Segoe UI"),
        icon: font(14.0, FW_NORMAL as i32, icon_face),
        icon_small: font(11.0, FW_NORMAL as i32, icon_face),
        icon_large: font(19.0, FW_NORMAL as i32, icon_face),
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
            Item::Title { text, .. } => text_width(dc, m.fonts.text, text),
            Item::Status { text, detail, button, .. } => {
                let extra = if button.is_some() { m.px(30.0) } else { 0 };
                text_width(dc, m.fonts.text, text).max(text_width(dc, m.fonts.small, detail)) + extra
            }
            Item::Action { text, .. } => text_width(dc, m.fonts.text, text),
            Item::Toggle { text, .. } => text_width(dc, m.fonts.text, text) + m.px(34.0),
            Item::Separator => 0,
        };
        widest = widest.max(w);
    }
    ReleaseDC(ptr::null_mut(), dc);
    // padding + icon column + text + right margin
    (m.pad() + m.px(38.0) + widest + m.px(18.0) + m.pad()).max(m.px(212.0))
}

// ---------------------------------------------------------------------------
// software shapes: anti-aliased rounded rectangles via a signed distance

/// Signed distance from a pixel centre to a rounded rectangle (negative inside).
fn sdf(px: f32, py: f32, l: f32, t: f32, r: f32, b: f32, radius: f32) -> f32 {
    let (cx, cy) = ((l + r) / 2.0, (t + b) / 2.0);
    let (hx, hy) = ((r - l) / 2.0 - radius, (b - t) / 2.0 - radius);
    let qx = (px - cx).abs() - hx;
    let qy = (py - cy).abs() - hy;
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - radius
}

fn coverage(d: f32) -> f32 {
    (0.5 - d).clamp(0.0, 1.0)
}

fn mix(dst: u32, rgb: u32, a: f32) -> u32 {
    let ch = |s: u32| -> u32 {
        let d = ((dst >> s) & 0xFF) as f32;
        let c = ((rgb >> s) & 0xFF) as f32;
        ((d + (c - d) * a).round() as u32) << s
    };
    ch(16) | ch(8) | ch(0)
}

/// Fills a rounded rect into an opaque buffer, optionally as an outline ring.
fn shape(s: &mut Surface, r: RECT, radius: f32, rgb: u32, ring: Option<f32>) {
    let (w, h) = (s.w, s.h);
    let px = s.pixels();
    let (l, t, rr, b) = (r.left as f32, r.top as f32, r.right as f32, r.bottom as f32);
    for y in r.top.max(0)..r.bottom.min(h) {
        for x in r.left.max(0)..r.right.min(w) {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let outer = coverage(sdf(fx, fy, l, t, rr, b, radius));
            let a = match ring {
                Some(lw) => outer - coverage(sdf(fx, fy, l + lw, t + lw, rr - lw, b - lw, (radius - lw).max(0.0))),
                None => outer,
            };
            if a > 0.0 {
                let i = (y * w + x) as usize;
                px[i] = mix(px[i], rgb, a);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// drawing

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

/// Draws the panel's contents (opaque) and composites the final image.
unsafe fn render(m: &mut Menu) {
    let (w, h, margin) = (m.width, m.height, m.margin);
    let Some(mut panel) = Surface::new(w, h) else { return };
    let p = m.palette;
    let bg = p.bg;
    panel.pixels().fill(bg);

    let pad = m.pad();
    let icon_x = pad + m.px(7.0);
    let icon_w = m.px(20.0);
    let text_x = pad + m.px(36.0);
    let right = w - pad;
    let rows = m.rows();
    let radius = m.px(5.0) as f32;

    // Shapes first (they write pixels directly), then GDI text on top.
    for (i, (it, &(top, bottom))) in m.items.iter().zip(rows.iter()).enumerate() {
        if m.hover == Some(i) {
            if let Item::Status { .. } = it {
                let (l, t, r, b) = m.button_rect((top, bottom));
                shape(&mut panel, RECT { left: l, top: t, right: r, bottom: b }, radius, p.hover, None);
            } else {
                shape(&mut panel, RECT { left: pad, top, right, bottom }, radius, p.hover, None);
            }
        }
        match it {
            Item::Separator => {
                let y = (top + bottom) / 2;
                shape(&mut panel, RECT { left: pad + m.px(8.0), top: y, right: right - m.px(8.0), bottom: y + 1 }, 0.0, p.line, None);
            }
            Item::Toggle { on, .. } => {
                let s = m.px(15.0);
                let bx = right - m.px(10.0) - s;
                let by = (top + bottom - s) / 2;
                let b = RECT { left: bx, top: by, right: bx + s, bottom: by + s };
                let lw = (1.2 * m.scale).max(1.0);
                if *on {
                    shape(&mut panel, b, m.px(4.0) as f32, p.box_on, None);
                } else {
                    shape(&mut panel, b, m.px(4.0) as f32, p.box_off, Some(lw));
                }
            }
            _ => {}
        }
    }
    GdiFlush();

    let dc = panel.dc;
    SetBkMode(dc, TRANSPARENT as _);
    for (it, &(top, bottom)) in m.items.iter().zip(rows.iter()) {
        let icon_box = RECT { left: icon_x, top, right: icon_x + icon_w, bottom };
        let mut text_box = RECT { left: text_x, top, right: right - m.px(10.0), bottom };
        match it {
            Item::Separator => {}
            Item::Title { icon, text: t } => {
                glyph(dc, m.fonts.icon_small, p.muted, *icon, icon_box);
                text(dc, m.fonts.text, p.muted, t, text_box, DT_VCENTER | DT_END_ELLIPSIS);
            }
            Item::Status { icon, text: t, detail, button } => {
                let wide_box = RECT { left: icon_box.left - m.px(2.0), right: icon_box.right + m.px(2.0), ..icon_box };
                glyph(dc, m.fonts.icon_large, p.text, *icon, wide_box);
                if let Some((_, b_icon)) = button {
                    let (l, bt, r, bb) = m.button_rect((top, bottom));
                    glyph(dc, m.fonts.icon, p.text, *b_icon, RECT { left: l, top: bt, right: r, bottom: bb });
                    text_box.right = l - m.px(6.0);
                }
                if detail.is_empty() {
                    text(dc, m.fonts.text, p.text, t, text_box, DT_VCENTER | DT_END_ELLIPSIS);
                } else {
                    let mid = top + (bottom - top) / 2;
                    let upper = RECT { top: top + m.px(3.0), bottom: mid + m.px(1.0), ..text_box };
                    let lower = RECT { top: mid, bottom: bottom - m.px(4.0), ..text_box };
                    text(dc, m.fonts.text, p.text, t, upper, DT_VCENTER | DT_END_ELLIPSIS);
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
                if *on {
                    let s = m.px(15.0);
                    let bx = right - m.px(10.0) - s;
                    let by = (top + bottom - s) / 2;
                    glyph(dc, m.fonts.icon_small, p.check, ICON_CHECK, RECT { left: bx, top: by, right: bx + s, bottom: by + s });
                }
            }
        }
    }
    GdiFlush();
    // Border last, over everything, along the panel's edge.
    let edge = (1.0 * m.scale).max(1.0);
    let corner = m.px(9.0) as f32;
    shape(&mut panel, RECT { left: 0, top: 0, right: w, bottom: h }, corner, p.border, Some(edge));

    // Composite: rounded panel over a soft drop shadow, premultiplied alpha.
    let (iw, ih) = (w + 2 * margin, h + 2 * margin);
    let Some(mut image) = Surface::new(iw, ih) else { return };
    let src: Vec<u32> = panel.pixels().to_vec();
    let out = image.pixels();
    let (pw, ph) = (w as f32, h as f32);
    let soft = margin as f32;
    let drop = m.px(3.0) as f32;
    for y in 0..ih {
        for x in 0..iw {
            let (fx, fy) = ((x - margin) as f32 + 0.5, (y - margin) as f32 + 0.5);
            let cov = coverage(sdf(fx, fy, 0.0, 0.0, pw, ph, corner));
            let d = sdf(fx, fy, 0.0, drop, pw, ph + drop, corner);
            let shadow = if d > 0.0 {
                let k = (1.0 - d / soft).clamp(0.0, 1.0);
                p.shadow * k * k * k
            } else {
                p.shadow
            };
            let a = cov + shadow * (1.0 - cov);
            let mut px = 0u32;
            if cov > 0.0 {
                let (sx, sy) = (x - margin, y - margin);
                if sx >= 0 && sy >= 0 && sx < w && sy < h {
                    let c = src[(sy * w + sx) as usize];
                    let ch = |s: u32| ((((c >> s) & 0xFF) as f32 * cov).round() as u32) << s;
                    px = ch(16) | ch(8) | ch(0);
                }
            }
            out[(y * iw + x) as usize] = (((a * 255.0).round() as u32) << 24) | px;
        }
    }
    m.image = Some(image);
}

/// Shows the composited image at an overall opacity and a downward offset.
unsafe fn present(m: &Menu, opacity: f32, offset: i32) {
    let Some(img) = &m.image else { return };
    let pos = POINT { x: m.x - m.margin, y: m.y - m.margin + offset };
    let size = SIZE { cx: img.w, cy: img.h };
    let src = POINT { x: 0, y: 0 };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: (opacity.clamp(0.0, 1.0) * 255.0).round() as u8,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    let screen = GetDC(ptr::null_mut());
    UpdateLayeredWindow(m.hwnd, screen, &pos, &size, img.dc, &src, 0, &blend, ULW_ALPHA);
    ReleaseDC(ptr::null_mut(), screen);
}

fn repaint(m: &mut Menu) {
    unsafe {
        render(m);
        if m.phase == Phase::Open {
            present(m, 1.0, 0);
        }
    }
}

// ---------------------------------------------------------------------------
// animation and input

fn begin_close(m: &mut Menu) {
    if matches!(m.phase, Phase::Closing(_)) {
        return;
    }
    // Fade out from wherever an opening fade had got to.
    m.phase = Phase::Closing(Instant::now());
    LAST_CLOSED.with(|c| *c.borrow_mut() = Some(Instant::now()));
    unsafe { SetTimer(m.hwnd, TIMER_ANIM, 10, None) };
}

/// One animation frame. Opening: ease-out fade while rising into place.
/// Closing: quicker fade while settling down a couple of pixels.
fn animate(m: &mut Menu) {
    unsafe {
        match m.phase {
            Phase::Opening(start) => {
                let t = (start.elapsed().as_secs_f32() * 1000.0 / OPEN_MS).min(1.0);
                let e = 1.0 - (1.0 - t).powi(3);
                present(m, e, ((1.0 - e) * SLIDE_PX * m.scale).round() as i32);
                if t >= 1.0 {
                    m.phase = Phase::Open;
                    KillTimer(m.hwnd, TIMER_ANIM);
                }
            }
            Phase::Closing(start) => {
                let t = (start.elapsed().as_secs_f32() * 1000.0 / CLOSE_MS).min(1.0);
                present(m, 1.0 - t * t, (t * 3.0 * m.scale).round() as i32);
                if t >= 1.0 {
                    KillTimer(m.hwnd, TIMER_ANIM);
                    PostMessageW(m.hwnd, WM_CLOSE, 0, 0);
                }
            }
            Phase::Open => {
                KillTimer(m.hwnd, TIMER_ANIM);
            }
        }
    }
}

/// Updates the status row in place, e.g. after a refresh while the menu is open.
pub fn update_status(icon: u16, text: String, detail: String) {
    with_menu(|m| {
        for it in &mut m.items {
            if let Item::Status { icon: i, text: t, detail: d, .. } = it {
                (*i, *t, *d) = (icon, text, detail);
                repaint(m);
                return;
            }
        }
    });
}

/// Runs the item: actions close the menu; toggles and the status button stay open.
fn activate(index: usize) {
    let Some((owner, id)) = with_menu(|m| {
        if matches!(m.phase, Phase::Closing(_)) {
            return None;
        }
        let (id, toggle) = match &mut m.items[index] {
            Item::Action { id, .. } => (*id, false),
            Item::Status { button: Some((id, _)), .. } => (*id, true),
            Item::Toggle { id, on, .. } => {
                *on = !*on;
                (*id, true)
            }
            _ => return None,
        };
        if toggle {
            repaint(m);
        } else {
            begin_close(m);
        }
        Some((m.owner, id))
    })
    .flatten() else {
        return;
    };
    unsafe { PostMessageW(owner, WM_MENU_COMMAND, id, 0) };
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
        repaint(m);
    });
}

fn mouse_pos(lp: LPARAM) -> (i32, i32) {
    ((lp & 0xFFFF) as i16 as i32, ((lp >> 16) & 0xFFFF) as i16 as i32)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            // Layered window: the image is pushed by UpdateLayeredWindow.
            let mut ps: PAINTSTRUCT = mem::zeroed();
            BeginPaint(hwnd, &mut ps);
            EndPaint(hwnd, &ps);
            0
        }
        WM_TIMER if wp == TIMER_ANIM => {
            with_menu(animate);
            0
        }
        WM_MOUSEMOVE => {
            let (x, y) = mouse_pos(lp);
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
                let hit = m.hit(x, y);
                if hit != m.hover {
                    m.hover = hit;
                    repaint(m);
                }
            });
            0
        }
        WM_MOUSELEAVE => {
            with_menu(|m| {
                m.tracking = false;
                if m.hover.take().is_some() {
                    repaint(m);
                }
            });
            0
        }
        WM_LBUTTONUP => {
            let (x, y) = mouse_pos(lp);
            if let Some(i) = with_menu(|m| m.hit(x, y)).flatten() {
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
                0x1B => {
                    with_menu(begin_close); // Esc
                }
                _ => {}
            }
            0
        }
        WM_ACTIVATE => {
            if (wp & 0xFFFF) as u32 == WA_INACTIVE {
                with_menu(begin_close);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounded_rect_distance() {
        // Centre of a 10x10 box is 5 inside; a point on a straight edge is on it.
        assert!((sdf(5.0, 5.0, 0.0, 0.0, 10.0, 10.0, 2.0) + 5.0).abs() < 1e-4);
        assert!(sdf(5.0, 0.0, 0.0, 0.0, 10.0, 10.0, 2.0).abs() < 1e-4);
        // The very corner is outside once rounded.
        assert!(sdf(0.0, 0.0, 0.0, 0.0, 10.0, 10.0, 2.0) > 0.5);
        assert_eq!(battery_icon(100), 0xE83F);
        assert_eq!(battery_icon(42), 0xE854);
    }
}
