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

use crate::text::{self, Align, Face};
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
pub const ICON_STARTUP: u16 = 0xE768; // outline play triangle
pub const ICON_POWER: u16 = 0xE7E8;
pub const ICON_CHARGING: u16 = 0xEA93;
pub const ICON_DOWNLOAD: u16 = 0xE896;

/// Battery glyph for a level: Battery0..Battery9 are E850..E859, Battery10 is E83F.
pub fn battery_icon(percent: u8) -> u16 {
    match percent / 10 {
        10.. => 0xE83F,
        n => 0xE850 + n as u16,
    }
}

pub enum Item {
    /// Small heading at the top, with a quieter note after it (the version).
    Title { icon: u16, text: String, note: String },
    /// Status: a main line and a quieter second line, with an optional small icon
    /// button on the right (id, icon) that runs without closing the menu.
    Status { icon: u16, text: String, detail: String, button: Option<(usize, u16)> },
    Action { id: usize, icon: u16, text: String },
    /// An action in the accent colour with a dot on the right ("Update available").
    Link { id: usize, icon: u16, text: String },
    Toggle { id: usize, icon: u16, text: String, on: bool },
    Separator,
}

impl Item {
    fn interactive(&self) -> bool {
        matches!(
            self,
            Item::Action { .. } | Item::Link { .. } | Item::Toggle { .. } | Item::Status { button: Some(_), .. }
        )
    }
    /// Height at 96 DPI.
    fn height(&self) -> i32 {
        match self {
            Item::Title { .. } => 26,
            Item::Status { detail, .. } if !detail.is_empty() => 40,
            Item::Status { .. } => 28,
            Item::Action { .. } | Item::Link { .. } | Item::Toggle { .. } => 30,
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
    /// Quieter than muted: the version next to the title.
    faint: u32,
    line: u32,
    box_off: u32,
    box_on: u32,
    check: u32,
    /// Links and the update dot (same blues as the tray badge).
    accent: u32,
    shadow: f32,
}

const DARK: Palette = Palette {
    bg: 0x16191B,
    border: 0x353D41,
    hover: 0x282E30,
    text: 0xF4F5F6,
    muted: 0x9BA3A7,
    faint: 0x6B7378,
    line: 0x2C3032,
    box_off: 0x5A6468,
    box_on: 0xE5E7E9,
    check: 0x171B1E,
    accent: 0x4CC2FF,
    shadow: 0.38,
};

const LIGHT: Palette = Palette {
    bg: 0xFBFBFC,
    border: 0xD3D7DB,
    hover: 0xE9ECEE,
    text: 0x121517,
    muted: 0x626A6F,
    faint: 0x8A949A,
    line: 0xE1E4E7,
    box_off: 0x8A949A,
    box_on: 0x171B1E,
    check: 0xF4F5F6,
    accent: 0x0067C0,
    shadow: 0.20,
};

/// A text style: typeface, weight and size at 96 DPI. Drawn with DirectWrite (text.rs).
#[derive(Clone, Copy)]
struct Style(Face, i32, f32);

const TEXT: Style = Style(Face::Text, 400, 13.0);
const SMALL: Style = Style(Face::Text, 400, 11.5);
const ICON: Style = Style(Face::Icons, 400, 14.0);
const ICON_SMALL: Style = Style(Face::Icons, 400, 11.0);
/// The battery glyph is wide and flat, so the status row uses a bigger size.
const ICON_LARGE: Style = Style(Face::Icons, 400, 19.0);

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

fn text_width(m: &Menu, s: &str, st: Style) -> i32 {
    text::measure(s, st.0, st.1, st.2 * m.scale).ceil() as i32
}

unsafe fn measure_width(m: &Menu) -> i32 {
    let mut widest = 0;
    for it in &m.items {
        let w = match it {
            Item::Title { text, note, .. } => text_width(m, text, TEXT) + m.px(6.0) + text_width(m, note, SMALL),
            Item::Status { text, detail, button, .. } => {
                let extra = if button.is_some() { m.px(30.0) } else { 0 };
                text_width(m, text, TEXT).max(text_width(m, detail, SMALL)) + extra
            }
            Item::Action { text, .. } => text_width(m, text, TEXT),
            Item::Link { text, .. } => text_width(m, text, TEXT) + m.px(24.0),
            Item::Toggle { text, .. } => text_width(m, text, TEXT) + m.px(34.0),
            Item::Separator => 0,
        };
        widest = widest.max(w);
    }
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

/// Draws an anti-aliased line with round ends into an opaque buffer.
fn stroke(s: &mut Surface, (ax, ay): (f32, f32), (bx, by): (f32, f32), width: f32, rgb: u32) {
    let (w, h) = (s.w, s.h);
    let px = s.pixels();
    let r = width / 2.0;
    let (x0, x1) = ((ax.min(bx) - r).floor() as i32, (ax.max(bx) + r).ceil() as i32);
    let (y0, y1) = ((ay.min(by) - r).floor() as i32, (ay.max(by) + r).ceil() as i32);
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = (dx * dx + dy * dy).max(1e-6);
    for y in y0.max(0)..y1.min(h) {
        for x in x0.max(0)..x1.min(w) {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // Distance from the pixel centre to the segment.
            let t = (((fx - ax) * dx + (fy - ay) * dy) / len2).clamp(0.0, 1.0);
            let (qx, qy) = (ax + t * dx - fx, ay + t * dy - fy);
            let a = coverage((qx * qx + qy * qy).sqrt() - r);
            if a > 0.0 {
                let i = (y * w + x) as usize;
                px[i] = mix(px[i], rgb, a);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// drawing

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

    // Shapes first (they write pixels directly), then text on top.
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
                    // A drawn tick: the icon font's checkmark is too fine at this size.
                    let (fx, fy, fs) = (bx as f32, by as f32, s as f32);
                    let pt = |u: f32, v: f32| (fx + u * fs, fy + v * fs);
                    let width = (1.9 * m.scale).max(1.5);
                    stroke(&mut panel, pt(0.25, 0.53), pt(0.43, 0.70), width, p.check);
                    stroke(&mut panel, pt(0.43, 0.70), pt(0.76, 0.33), width, p.check);
                } else {
                    shape(&mut panel, b, m.px(4.0) as f32, p.box_off, Some(lw));
                }
            }
            Item::Link { .. } => {
                let s = m.px(7.0);
                let dx = right - m.px(14.0) - s;
                let dy = (top + bottom - s) / 2;
                shape(&mut panel, RECT { left: dx, top: dy, right: dx + s, bottom: dy + s }, s as f32 / 2.0, p.accent, None);
            }
            _ => {}
        }
    }
    GdiFlush();

    // Text and icon glyphs, drawn with DirectWrite into the panel buffer. (Measuring
    // isn't possible while drawing, so the title's width is taken first.)
    let title_w = m.items.iter().find_map(|it| match it {
        Item::Title { text: t, .. } => Some(text_width(m, t, TEXT)),
        _ => None,
    });
    let scale = m.scale;
    text::paint(panel.dc, w, h, |pt| {
        let mut draw = |s: &str, st: Style, rgb: u32, r: RECT, align: Align| {
            pt.text(s, st.0, st.1, st.2 * scale, rgb, (r.left, r.top, r.right, r.bottom), align);
        };
        let g = |cp: u16| String::from_utf16_lossy(&[cp]);
        for (it, &(top, bottom)) in m.items.iter().zip(rows.iter()) {
            let icon_box = RECT { left: icon_x, top, right: icon_x + icon_w, bottom };
            let mut text_box = RECT { left: text_x, top, right: right - m.px(10.0), bottom };
            match it {
                Item::Separator => {}
                Item::Title { icon, text: t, note } => {
                    draw(&g(*icon), ICON_SMALL, p.muted, icon_box, Align::Center);
                    draw(t, TEXT, p.muted, text_box, Align::Leading);
                    if let Some(w) = title_w {
                        // Nudged down a touch so the smaller text sits on the same baseline.
                        let b = RECT { left: text_x + w + m.px(6.0), top: top + m.px(1.0), bottom: bottom + m.px(1.0), ..text_box };
                        draw(note, SMALL, p.faint, b, Align::Leading);
                    }
                }
                Item::Status { icon, text: t, detail, button } => {
                    let wide_box = RECT { left: icon_box.left - m.px(2.0), right: icon_box.right + m.px(2.0), ..icon_box };
                    draw(&g(*icon), ICON_LARGE, p.text, wide_box, Align::Center);
                    if let Some((_, b_icon)) = button {
                        let (l, bt, r, bb) = m.button_rect((top, bottom));
                        draw(&g(*b_icon), ICON, p.text, RECT { left: l, top: bt, right: r, bottom: bb }, Align::Center);
                        text_box.right = l - m.px(6.0);
                    }
                    if detail.is_empty() {
                        draw(t, TEXT, p.text, text_box, Align::Leading);
                    } else {
                        let mid = top + (bottom - top) / 2;
                        let upper = RECT { top: top + m.px(3.0), bottom: mid + m.px(1.0), ..text_box };
                        let lower = RECT { top: mid, bottom: bottom - m.px(4.0), ..text_box };
                        draw(t, TEXT, p.text, upper, Align::Leading);
                        draw(detail, SMALL, p.muted, lower, Align::Leading);
                    }
                }
                Item::Action { icon, text: t, .. } | Item::Toggle { icon, text: t, .. } => {
                    draw(&g(*icon), ICON, p.text, icon_box, Align::Center);
                    draw(t, TEXT, p.text, text_box, Align::Leading);
                }
                Item::Link { icon, text: t, .. } => {
                    draw(&g(*icon), ICON, p.accent, icon_box, Align::Center);
                    draw(t, TEXT, p.text, text_box, Align::Leading);
                }
            }
        }
    });
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
            Item::Action { id, .. } | Item::Link { id, .. } => (*id, false),
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
            text::release();
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

    /// Renders a sample menu at 2x to %TEMP%\hyperlight-menu-<theme>.rgba (width
    /// and height as little-endian u32s, then premultiplied BGRA rows) for the
    /// README screenshot: `cargo test --bin hyperlight-tray -- --ignored`
    #[test]
    #[ignore]
    fn dump_menu_image() {
        for (name, palette) in [("dark", &DARK), ("light", &LIGHT)] {
            let items = vec![
                Item::Title { icon: ICON_MOUSE, text: "Hyperlight Tray".into(), note: "v0.2.0".into() },
                Item::Separator,
                Item::Status {
                    icon: battery_icon(85),
                    text: "Battery 85% (~12h)".into(),
                    detail: "Not charging · 3.95 V".into(),
                    button: Some((1, ICON_REFRESH)),
                },
                Item::Separator,
                Item::Link { id: 5, icon: ICON_DOWNLOAD, text: "Update available: v0.2.0".into() },
                Item::Action { id: 2, icon: ICON_SLIDERS, text: "Open Configurator...".into() },
                Item::Toggle { id: 3, icon: ICON_STARTUP, text: "Start with Windows".into(), on: true },
                Item::Separator,
                Item::Action { id: 4, icon: ICON_POWER, text: "Exit".into() },
            ];
            let scale = 2.0;
            let mut m = Menu {
                hwnd: ptr::null_mut(),
                owner: ptr::null_mut(),
                items,
                hover: Some(4),
                tracking: false,
                scale,
                width: 0,
                height: 0,
                margin: 0,
                x: 0,
                y: 0,
                phase: Phase::Open,
                palette,
                image: None,
            };
            m.margin = m.px(16.0);
            m.width = unsafe { measure_width(&m) };
            m.height = m.content_height();
            unsafe { render(&mut m) };
            let img = m.image.as_mut().expect("rendered");
            let mut out = Vec::new();
            out.extend_from_slice(&(img.w as u32).to_le_bytes());
            out.extend_from_slice(&(img.h as u32).to_le_bytes());
            for px in img.pixels().iter() {
                out.extend_from_slice(&px.to_le_bytes());
            }
            let path = std::env::temp_dir().join(format!("hyperlight-menu-{name}.rgba"));
            std::fs::write(&path, out).unwrap();
            println!("wrote {}", path.display());
        }
    }
}
