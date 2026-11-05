//! Text for the tray menu, drawn with DirectWrite through Direct2D: the same
//! engine Windows 11's own menus use. Compared with GDI it positions glyphs to
//! fractions of a pixel and smooths them, so small text and icon glyphs look even
//! and clean instead of thin and jagged.
//!
//! Text is drawn into an existing GDI memory DC (the menu's opaque panel buffer)
//! through a DC render target, with grayscale anti-aliasing so there are no colour
//! fringes once the panel is composited with per-pixel alpha.

use std::cell::RefCell;
use std::collections::HashMap;

use windows::core::{BOOL, HSTRING};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_IGNORE, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::HDC;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Face {
    Text,
    Icons,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Leading,
    Center,
}

struct Engine {
    dw: IDWriteFactory,
    target: ID2D1DCRenderTarget,
    text_family: &'static str,
    icon_family: &'static str,
    formats: HashMap<(Face, i32, u32, bool), IDWriteTextFormat>,
}

thread_local! {
    static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
}

fn with_engine<R>(f: impl FnOnce(&mut Engine) -> R) -> Option<R> {
    ENGINE.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = unsafe { Engine::new() };
        }
        slot.as_mut().map(f)
    })
}

impl Engine {
    unsafe fn new() -> Option<Engine> {
        let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).ok()?;
        let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).ok()?;
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            // Software: the CPU rasteriser is plenty for a few lines of text, and avoids
            // loading the Direct3D/GPU driver stack (tens of MB) into a tray app.
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_IGNORE },
            // 96 DPI: one unit is one pixel; the menu scales sizes itself.
            dpiX: 96.0,
            dpiY: 96.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        let target = d2d.CreateDCRenderTarget(&props).ok()?;
        target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        // Natural symmetric: smooth outlines with even spacing, the modern Windows look.
        if let Ok(params) = dw.CreateCustomRenderingParams(
            1.8,
            0.6,
            0.0,
            DWRITE_PIXEL_GEOMETRY_FLAT,
            DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC,
        ) {
            target.SetTextRenderingParams(&params);
        }

        let mut collection = None;
        let _ = dw.GetSystemFontCollection(&mut collection, false);
        let has = |name: &str| -> bool {
            let Some(c) = &collection else { return false };
            let (mut index, mut exists) = (0u32, BOOL(0));
            c.FindFamilyName(&HSTRING::from(name), &mut index, &mut exists).is_ok() && exists.as_bool()
        };
        // Windows 11 fonts, falling back to their Windows 10 equivalents.
        let text_family = if has("Segoe UI Variable Text") { "Segoe UI Variable Text" } else { "Segoe UI" };
        let icon_family = if has("Segoe Fluent Icons") { "Segoe Fluent Icons" } else { "Segoe MDL2 Assets" };
        Some(Engine { dw, target, text_family, icon_family, formats: HashMap::new() })
    }

    unsafe fn format(&mut self, face: Face, weight: i32, size: f32, center: bool) -> Option<IDWriteTextFormat> {
        let key = (face, weight, size.to_bits(), center);
        if let Some(f) = self.formats.get(&key) {
            return Some(f.clone());
        }
        let family = match face {
            Face::Text => self.text_family,
            Face::Icons => self.icon_family,
        };
        let f = self
            .dw
            .CreateTextFormat(
                &HSTRING::from(family),
                None,
                DWRITE_FONT_WEIGHT(weight),
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                &HSTRING::from("en-us"),
            )
            .ok()?;
        let _ = f.SetTextAlignment(if center { DWRITE_TEXT_ALIGNMENT_CENTER } else { DWRITE_TEXT_ALIGNMENT_LEADING });
        let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
        let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
        self.formats.insert(key, f.clone());
        Some(f)
    }
}

/// Width in pixels of `s` at the given style.
pub fn measure(s: &str, face: Face, weight: i32, size: f32) -> f32 {
    with_engine(|e| unsafe {
        let Some(format) = e.format(face, weight, size, false) else { return 0.0 };
        let wide: Vec<u16> = s.encode_utf16().collect();
        let Ok(layout) = e.dw.CreateTextLayout(&wide, &format, 10_000.0, 1_000.0) else { return 0.0 };
        let mut m = DWRITE_TEXT_METRICS::default();
        if layout.GetMetrics(&mut m).is_ok() {
            m.widthIncludingTrailingWhitespace
        } else {
            0.0
        }
    })
    .unwrap_or(0.0)
}

pub struct Painter<'a> {
    engine: &'a mut Engine,
}

impl Painter<'_> {
    /// Draws `s` inside the rectangle (pixels), vertically centred.
    pub fn text(&mut self, s: &str, face: Face, weight: i32, size: f32, rgb: u32, r: (i32, i32, i32, i32), align: Align) {
        unsafe {
            let Some(format) = self.engine.format(face, weight, size, align == Align::Center) else { return };
            let color = D2D1_COLOR_F {
                r: ((rgb >> 16) & 0xFF) as f32 / 255.0,
                g: ((rgb >> 8) & 0xFF) as f32 / 255.0,
                b: (rgb & 0xFF) as f32 / 255.0,
                a: 1.0,
            };
            let Ok(brush) = self.engine.target.CreateSolidColorBrush(&color, None) else { return };
            let wide: Vec<u16> = s.encode_utf16().collect();
            let rect = D2D_RECT_F { left: r.0 as f32, top: r.1 as f32, right: r.2 as f32, bottom: r.3 as f32 };
            self.engine.target.DrawText(
                &wide,
                &format,
                &rect,
                &brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }
}

/// Binds the render target to `hdc` (a `w`×`h` memory DC), runs `f` to draw text,
/// and flushes the result into the DC's bitmap.
pub fn paint(hdc: *mut core::ffi::c_void, w: i32, h: i32, f: impl FnOnce(&mut Painter)) {
    with_engine(|e| unsafe {
        let rect = RECT { left: 0, top: 0, right: w, bottom: h };
        if e.target.BindDC(HDC(hdc), &rect).is_err() {
            return;
        }
        e.target.BeginDraw();
        f(&mut Painter { engine: e });
        let _ = e.target.EndDraw(None, None);
    });
}

/// Frees the Direct2D/DirectWrite objects; they're recreated the next time the
/// menu opens. Keeps the tray app small while the menu is closed.
pub fn release() {
    ENGINE.with(|cell| cell.borrow_mut().take());
}
