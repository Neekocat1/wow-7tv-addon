//! Just enough 2D drawing for the status images and the tray icon: rounded rectangles and text.
//!
//! Fonts: Segoe UI / Arial from the Windows font folder when available (what the Python helper
//! used), otherwise the DejaVu Sans copies embedded in the exe.

use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use image::{Rgba, RgbaImage};
use std::sync::OnceLock;

static EMBEDDED_REGULAR: &[u8] = include_bytes!("../assets/DejaVuSans.ttf");
static EMBEDDED_BOLD: &[u8] = include_bytes!("../assets/DejaVuSans-Bold.ttf");

fn system_font(names: &[&str]) -> Option<FontArc> {
    if !cfg!(windows) {
        return None;
    }
    let windir = std::env::var_os("WINDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "C:\\Windows".into());
    for n in names {
        if let Ok(data) = std::fs::read(windir.join("Fonts").join(n))
            && let Ok(f) = FontArc::try_from_vec(data)
        {
            return Some(f);
        }
    }
    None
}

pub fn font(bold: bool) -> &'static FontArc {
    static REGULAR: OnceLock<FontArc> = OnceLock::new();
    static BOLD: OnceLock<FontArc> = OnceLock::new();
    if bold {
        BOLD.get_or_init(|| {
            system_font(&["segoeuib.ttf", "arialbd.ttf"])
                .unwrap_or_else(|| FontArc::try_from_slice(EMBEDDED_BOLD).expect("embedded font"))
        })
    } else {
        REGULAR.get_or_init(|| {
            system_font(&["segoeui.ttf", "arial.ttf"]).unwrap_or_else(|| {
                FontArc::try_from_slice(EMBEDDED_REGULAR).expect("embedded font")
            })
        })
    }
}

/// Pillow's font size is the em size in pixels; ab_glyph's scale is ascent - descent.
fn scale_for(font: &FontArc, size: f32) -> PxScale {
    let upem = font.units_per_em().unwrap_or(1000.0);
    PxScale::from(size * font.height_unscaled() / upem)
}

/// Advance width of `text` in pixels (Pillow's textlength).
pub fn text_length(text: &str, font: &FontArc, size: f32) -> f32 {
    let sf = font.as_scaled(scale_for(font, size));
    let mut w = 0.0;
    let mut prev = None;
    for c in text.chars() {
        let id = sf.glyph_id(c);
        if let Some(p) = prev {
            w += sf.kern(p, id);
        }
        w += sf.h_advance(id);
        prev = Some(id);
    }
    w
}

/// Blend `color` into `dst` with coverage `m` on every channel (Pillow's ink-through-mask paste).
fn blend(dst: &mut Rgba<u8>, color: [u8; 4], m: f32) {
    let m = m.clamp(0.0, 1.0);
    for (d, c) in dst.0.iter_mut().zip(color) {
        *d = (c as f32 * m + *d as f32 * (1.0 - m)).round() as u8;
    }
}

/// Draw text with its top-left at (x, y) ("la" anchor: y is the ascender line).
pub fn draw_text(
    img: &mut RgbaImage,
    x: f32,
    y: f32,
    text: &str,
    font: &FontArc,
    size: f32,
    color: [u8; 4],
) {
    let sf = font.as_scaled(scale_for(font, size));
    let baseline = y + sf.ascent();
    let mut caret = x;
    let mut prev = None;
    let (w, h) = img.dimensions();
    for c in text.chars() {
        let id = sf.glyph_id(c);
        if let Some(p) = prev {
            caret += sf.kern(p, id);
        }
        let glyph = id.with_scale_and_position(sf.scale(), ab_glyph::point(caret, baseline));
        caret += sf.h_advance(id);
        prev = Some(id);
        if let Some(outline) = font.outline_glyph(glyph) {
            let b = outline.px_bounds();
            outline.draw(|gx, gy, cov| {
                let px = b.min.x as i32 + gx as i32;
                let py = b.min.y as i32 + gy as i32;
                if px >= 0 && py >= 0 && (px as u32) < w && (py as u32) < h {
                    blend(img.get_pixel_mut(px as u32, py as u32), color, cov);
                }
            });
        }
    }
}

/// Draw text centred on (cx, cy) using the glyph ink box ("mm" anchor, near enough).
pub fn draw_text_centered(
    img: &mut RgbaImage,
    cx: f32,
    cy: f32,
    text: &str,
    font: &FontArc,
    size: f32,
    color: [u8; 4],
) {
    let sf = font.as_scaled(scale_for(font, size));
    let width = text_length(text, font, size);
    // Centre vertically between the ascender and descender lines like Pillow's "mm".
    // (ab_glyph's descent is negative.)
    let top = cy + (sf.descent() - sf.ascent()) / 2.0;
    draw_text(img, cx - width / 2.0, top, text, font, size, color);
}

/// Pillow's rounded_rectangle((x0, y0, x1, y1), radius, fill): inclusive box, solid fill (no blending).
pub fn rounded_rect(
    img: &mut RgbaImage,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    r: u32,
    color: [u8; 4],
) {
    let (w, h) = img.dimensions();
    let r = r as f32;
    let (fx0, fy0, fx1, fy1) = (x0 as f32, y0 as f32, x1 as f32 + 1.0, y1 as f32 + 1.0);
    for y in y0..=y1.min(h - 1) {
        for x in x0..=x1.min(w - 1) {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let cx = if px < fx0 + r {
                fx0 + r
            } else if px > fx1 - r {
                fx1 - r
            } else {
                px
            };
            let cy = if py < fy0 + r {
                fy0 + r
            } else if py > fy1 - r {
                fy1 - r
            } else {
                py
            };
            let (dx, dy) = (px - cx, py - cy);
            if dx * dx + dy * dy <= r * r {
                img.put_pixel(x, y, Rgba(color));
            }
        }
    }
}

/// The tray icon: a blue rounded square with a white "7".
pub fn make_icon(size: u32) -> RgbaImage {
    let mut img = RgbaImage::new(size, size);
    let m = (size / 32).max(1);
    rounded_rect(
        &mut img,
        m,
        m,
        size - 1 - m,
        size - 1 - m,
        size / 5,
        [41, 182, 246, 255],
    );
    let font = font(true);
    draw_text_centered(
        &mut img,
        size as f32 / 2.0,
        size as f32 / 2.0,
        "7",
        font,
        size as f32 * 0.62,
        [255, 255, 255, 255],
    );
    img
}
