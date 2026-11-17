//! Turn 7TV images into textures WoW can load.
//!
//! WoW wants power-of-two textures (TGA or BLP). Static emotes are padded onto a
//! power-of-two canvas; animated emotes become a sprite sheet the addon steps through.

use crate::seventv::{ApiError, Emote, Http};
use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;
use image::imageops::FilterType;
use image::{AnimationDecoder, ImageBuffer, ImageFormat, Rgba, RgbaImage};
use serde_json::Value;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const FORMAT_PREFERENCE: [&str; 4] = ["WEBP", "GIF", "PNG", "AVIF"];

/// What the addon needs to draw a texture (also stored per emote in helper_state.json).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub w: u32,
    pub h: u32,
    pub tw: u32,
    pub th: u32,
    pub frames: u32,
    pub cols: u32,
    pub ms: u32,
}

fn height(f: &Value) -> i64 {
    f.get("height").and_then(Value::as_i64).unwrap_or(0)
}

pub fn pick_file(host: &Value, size: i64) -> Option<Value> {
    let files = host.get("files")?.as_array()?;
    for fmt in FORMAT_PREFERENCE {
        let mut cands: Vec<&Value> = files
            .iter()
            .filter(|f| f.get("format").and_then(Value::as_str) == Some(fmt))
            .collect();
        cands.sort_by_key(|f| height(f)); // stable, like Python's sorted()
        if let Some(f) = cands.iter().find(|f| height(f) >= size) {
            return Some((*f).clone());
        }
        if let Some(f) = cands.last() {
            return Some((*f).clone());
        }
    }
    files.last().cloned()
}

/// Download (or reuse from the cache) the file the texture is built from.
pub fn download(
    http: &dyn Http,
    emote: &Emote,
    size: i64,
    cache: &Path,
) -> Result<PathBuf, String> {
    let f = pick_file(&emote.host, size).ok_or("emote has no files")?;
    let fname = f
        .get("name")
        .and_then(Value::as_str)
        .ok_or("emote file has no name")?;
    std::fs::create_dir_all(cache).map_err(|e| e.to_string())?;
    let path = cache.join(format!("{}_{}", emote.id, fname));
    let have = std::fs::metadata(&path)
        .map(|m| m.len() > 0)
        .unwrap_or(false);
    if !have {
        let base = emote.host.get("url").and_then(Value::as_str).unwrap_or("");
        let base = if base.starts_with("//") {
            format!("https:{base}")
        } else {
            base.to_string()
        };
        let url = format!("{base}/{fname}");
        let data = http.get_bytes(&url).map_err(|e| match e {
            ApiError::NotFound(u) => format!("not found: {u}"),
            other => other.to_string(),
        })?;
        let tmp = with_suffix(&path, ".part");
        std::fs::write(&tmp, &data).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    }
    Ok(path)
}

/// `path` + suffix appended to the whole file name (`a.tga` -> `a.tga.part`).
pub fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// All frames (composited, full canvas) and their durations in ms.
pub fn load_frames(path: &Path) -> Result<(Vec<RgbaImage>, Vec<u32>), String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let fmt = image::guess_format(&data).map_err(|e| e.to_string())?;
    let err = |e: image::ImageError| e.to_string();
    let frames: Vec<image::Frame> = match fmt {
        ImageFormat::Gif => GifDecoder::new(Cursor::new(&data))
            .map_err(err)?
            .into_frames()
            .collect_frames()
            .map_err(err)?,
        ImageFormat::WebP => {
            let mut dec = WebPDecoder::new(Cursor::new(&data)).map_err(err)?;
            if dec.has_animation() {
                // Frames marked "dispose to background" must be cleared to transparent before the
                // next one is drawn (what libwebp, browsers and Pillow do). image-webp skips the
                // clearing entirely unless a background colour is set, which leaves trails.
                dec.set_background_color(Rgba([0, 0, 0, 0])).map_err(err)?;
                dec.into_frames().collect_frames().map_err(err)?
            } else {
                let img = image::DynamicImage::from_decoder(dec)
                    .map_err(err)?
                    .to_rgba8();
                vec![image::Frame::new(img)]
            }
        }
        ImageFormat::Png => {
            let dec = PngDecoder::new(Cursor::new(&data)).map_err(err)?;
            if dec.is_apng().map_err(err)? {
                dec.apng()
                    .map_err(err)?
                    .into_frames()
                    .collect_frames()
                    .map_err(err)?
            } else {
                let img = image::DynamicImage::from_decoder(dec)
                    .map_err(err)?
                    .to_rgba8();
                vec![image::Frame::new(img)]
            }
        }
        other => return Err(format!("unsupported image format {other:?}")),
    };
    if frames.is_empty() {
        return Err("image has no frames".into());
    }
    let mut imgs = Vec::with_capacity(frames.len());
    let mut durations = Vec::with_capacity(frames.len());
    for fr in frames {
        let (n, d) = fr.delay().numer_denom_ms();
        let ms = if d == 0 { 0.0 } else { n as f64 / d as f64 };
        let ms = if ms == 0.0 { 100.0 } else { ms };
        durations.push(if ms >= 20.0 { ms as u32 } else { 100 }); // browsers treat tiny delays as 100ms
        imgs.push(fr.into_buffer());
    }
    Ok((imgs, durations))
}

pub fn pow2(n: u32) -> u32 {
    n.max(1).next_power_of_two()
}

/// Python's round(): halves go to the even neighbour.
fn py_round(x: f64) -> i64 {
    x.round_ties_even() as i64
}

/// Scale so height == size; very wide emotes are capped at size*max_ratio wide.
pub fn fit(w: u32, h: u32, size: u32, max_ratio: f64) -> (u32, u32) {
    let (w, h) = (w.max(1) as f64, h.max(1) as f64);
    let mut ww = py_round(w * size as f64 / h).max(1) as u32;
    let mut hh = size;
    let max_w = (size as f64 * max_ratio) as u32;
    if ww > max_w {
        ww = max_w;
        hh = py_round(h * max_w as f64 / w).max(1) as u32;
    }
    (ww, hh)
}

pub fn decimate<T: Clone>(
    frames: &[T],
    durations: &[u32],
    max_frames: usize,
) -> (Vec<T>, Vec<u32>) {
    let n = frames.len();
    if n <= max_frames {
        return (frames.to_vec(), durations.to_vec());
    }
    let step = n as f64 / max_frames as f64;
    let mut out_f = Vec::with_capacity(max_frames);
    let mut out_d = Vec::with_capacity(max_frames);
    for i in 0..max_frames {
        let a = (i as f64 * step) as usize;
        let b = ((i + 1) as f64 * step) as usize;
        out_f.push(frames[a].clone());
        out_d.push(durations[a..b.max(a + 1).min(n)].iter().sum());
    }
    (out_f, out_d)
}

/// Columns and power-of-two sheet size with the smallest area (then the squarest).
pub fn sheet_layout(n: u32, w: u32, h: u32, max_sheet: u32) -> Option<(u32, u32, u32)> {
    let mut best: Option<((u64, f64), u32, u32, u32)> = None;
    for cols in 1..=n {
        if cols * w > max_sheet {
            break;
        }
        let rows = n.div_ceil(cols);
        let (sw, sh) = (pow2(cols * w), pow2(rows * h));
        if sw > max_sheet || sh > max_sheet {
            continue;
        }
        let key = (
            sw as u64 * sh as u64,
            ((sw as f64).log2() - (sh as f64).log2()).abs(),
        );
        if best
            .as_ref()
            .is_none_or(|b| key.0 < b.0.0 || (key.0 == b.0.0 && key.1 < b.0.1))
        {
            best = Some((key, cols, sw, sh));
        }
    }
    best.map(|(_, c, sw, sh)| (c, sw, sh))
}

// --------------------------------------------------------------------------- resampling

type Rgba32F = ImageBuffer<Rgba<f32>, Vec<f32>>;

// Float pixels in 0..1 (the image crate clamps f32 samples to that range while resampling).
fn premultiply(img: &RgbaImage) -> Rgba32F {
    ImageBuffer::from_fn(img.width(), img.height(), |x, y| {
        let p = img.get_pixel(x, y).0;
        let a = p[3] as f32 / 255.0;
        Rgba([
            p[0] as f32 / 255.0 * a,
            p[1] as f32 / 255.0 * a,
            p[2] as f32 / 255.0 * a,
            a,
        ])
    })
}

fn unpremultiply(img: &Rgba32F) -> RgbaImage {
    ImageBuffer::from_fn(img.width(), img.height(), |x, y| {
        let p = img.get_pixel(x, y).0;
        let a8 = (p[3].clamp(0.0, 1.0) * 255.0).round();
        if a8 < 1.0 {
            return Rgba([0, 0, 0, 0]);
        }
        let a = p[3].clamp(1.0 / 255.0, 1.0);
        let c = |v: f32| (v / a * 255.0).round().clamp(0.0, 255.0) as u8;
        Rgba([c(p[0]), c(p[1]), c(p[2]), a8 as u8])
    })
}

/// Lanczos resize in premultiplied alpha (like Pillow's RGBA resize), so transparent
/// pixels don't bleed dark fringes into the edges.
pub fn resize_lanczos(img: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    if img.dimensions() == (w, h) {
        return img.clone();
    }
    let pm = premultiply(img);
    let out = image::imageops::resize(&pm, w.max(1), h.max(1), FilterType::Lanczos3);
    unpremultiply(&out)
}

/// Halve with a 2x2 box filter (premultiplied). Dimensions of 1 stay 1.
fn box_half(img: &RgbaImage) -> RgbaImage {
    let (w, h) = img.dimensions();
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let (fx, fy) = (if w > 1 { 2 } else { 1 }, if h > 1 { 2 } else { 1 });
    ImageBuffer::from_fn(nw, nh, |x, y| {
        let mut acc = [0f32; 4];
        for dy in 0..fy {
            for dx in 0..fx {
                let p = img
                    .get_pixel((x * fx + dx).min(w - 1), (y * fy + dy).min(h - 1))
                    .0;
                let a = p[3] as f32;
                acc[0] += p[0] as f32 * a;
                acc[1] += p[1] as f32 * a;
                acc[2] += p[2] as f32 * a;
                acc[3] += a;
            }
        }
        let n = (fx * fy) as f32;
        let a = acc[3] / n;
        if acc[3] <= 0.0 {
            return Rgba([0, 0, 0, 0]);
        }
        let c = |v: f32| (v / acc[3]).round().clamp(0.0, 255.0) as u8;
        Rgba([
            c(acc[0]),
            c(acc[1]),
            c(acc[2]),
            a.round().clamp(0.0, 255.0) as u8,
        ])
    })
}

/// Copy `src` onto a transparent canvas at (0, 0).
pub fn pad(src: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    let mut canvas = RgbaImage::new(w, h);
    image::imageops::replace(&mut canvas, src, 0, 0);
    canvas
}

// --------------------------------------------------------------------------- BLP (DXT5)
//
// BLP2 is WoW's own texture format. With DXT5 compression a texture takes 1 byte per pixel
// on disk and in video memory, versus 4 for an uncompressed TGA.

pub const BLP_HEADER_SIZE: usize = 1172; // magic, type, 4 flag bytes, w, h, 16 mip offsets, 16 mip sizes, palette

/// Compressed textures are always available in the Rust helper (built-in encoder).
pub fn compression_available() -> bool {
    true
}

fn to_565(c: [i32; 3]) -> u16 {
    (((c[0] as u16) >> 3) << 11) | (((c[1] as u16) >> 2) << 5) | ((c[2] as u16) >> 3)
}

fn from_565(c: u16) -> [i32; 3] {
    let r = ((c >> 11) & 31) as i32;
    let g = ((c >> 5) & 63) as i32;
    let b = (c & 31) as i32;
    [
        (r << 3) | (r >> 2),
        (g << 2) | (g >> 4),
        (b << 3) | (b >> 2),
    ]
}

/// One 4x4 block (16 RGBA pixels, row-major) to 16 bytes of DXT5/BC3.
/// Port of the helper's numpy encoder: alpha min/max ramp; colour endpoints along the
/// block's principal axis (visible pixels only).
pub fn encode_block(px: &[[u8; 4]; 16]) -> [u8; 16] {
    // Alpha: two endpoints, 8-value ramp, 3-bit index per pixel.
    let alpha: Vec<i32> = px.iter().map(|p| p[3] as i32).collect();
    let a0 = *alpha.iter().max().unwrap();
    let a1 = *alpha.iter().min().unwrap();
    let mut ramp = [0i32; 8];
    ramp[0] = a0;
    ramp[1] = a1;
    for i in 1..7 {
        ramp[i + 1] = ((7 - i as i32) * a0 + i as i32 * a1) / 7;
    }
    let mut abits: u64 = 0;
    for (p, &a) in alpha.iter().enumerate() {
        let mut best = 0;
        for k in 1..8 {
            if (a - ramp[k]).abs() < (a - ramp[best]).abs() {
                best = k;
            }
        }
        abits |= (best as u64) << (3 * p);
    }

    // Colour.
    let rgb: Vec<[f64; 3]> = px
        .iter()
        .map(|p| [p[0] as f64, p[1] as f64, p[2] as f64])
        .collect();
    let mut wv: Vec<f64> = alpha
        .iter()
        .map(|&a| if a > 0 { 1.0 } else { 0.0 })
        .collect();
    if wv.iter().all(|&w| w == 0.0) {
        wv.iter_mut().for_each(|w| *w = 1.0);
    }
    let wsum: f64 = wv.iter().sum();
    let mut mean = [0f64; 3];
    for (c, w) in rgb.iter().zip(&wv) {
        for k in 0..3 {
            mean[k] += c[k] * w;
        }
    }
    mean.iter_mut().for_each(|m| *m /= wsum);
    let mut cov = [[0f64; 3]; 3];
    for (c, w) in rgb.iter().zip(&wv) {
        let sw = w.sqrt();
        let d = [
            (c[0] - mean[0]) * sw,
            (c[1] - mean[1]) * sw,
            (c[2] - mean[2]) * sw,
        ];
        for i in 0..3 {
            for j in 0..3 {
                cov[i][j] += d[i] * d[j];
            }
        }
    }
    let unit = 1.0 / 3f64.sqrt();
    let mut axis = [unit; 3];
    for _ in 0..8 {
        let n = [
            cov[0][0] * axis[0] + cov[0][1] * axis[1] + cov[0][2] * axis[2],
            cov[1][0] * axis[0] + cov[1][1] * axis[1] + cov[1][2] * axis[2],
            cov[2][0] * axis[0] + cov[2][1] * axis[1] + cov[2][2] * axis[2],
        ];
        let norm = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        axis = if norm > 1e-9 {
            [n[0] / norm, n[1] / norm, n[2] / norm]
        } else {
            [unit; 3]
        };
    }
    let (mut tmin, mut tmax) = (f64::INFINITY, f64::NEG_INFINITY);
    for (c, w) in rgb.iter().zip(&wv) {
        if *w > 0.0 {
            let t = (c[0] - mean[0]) * axis[0]
                + (c[1] - mean[1]) * axis[1]
                + (c[2] - mean[2]) * axis[2];
            tmin = tmin.min(t);
            tmax = tmax.max(t);
        }
    }
    let endpoint = |t: f64| -> [i32; 3] {
        let f = |k: usize| (mean[k] + axis[k] * t).round_ties_even().clamp(0.0, 255.0) as i32;
        [f(0), f(1), f(2)]
    };
    let (cmin, cmax) = (endpoint(tmin), endpoint(tmax));
    let (mut c0, mut c1) = (to_565(cmax), to_565(cmin));
    if c0 < c1 {
        std::mem::swap(&mut c0, &mut c1);
    }
    let (e0, e1) = (from_565(c0), from_565(c1));
    let pal = [
        e0,
        e1,
        [
            (2 * e0[0] + e1[0]) / 3,
            (2 * e0[1] + e1[1]) / 3,
            (2 * e0[2] + e1[2]) / 3,
        ],
        [
            (e0[0] + 2 * e1[0]) / 3,
            (e0[1] + 2 * e1[1]) / 3,
            (e0[2] + 2 * e1[2]) / 3,
        ],
    ];
    let mut cbits: u32 = 0;
    if c0 != c1 {
        for (p, q) in px.iter().enumerate() {
            let q = [q[0] as i32, q[1] as i32, q[2] as i32];
            let d =
                |e: &[i32; 3]| (q[0] - e[0]).pow(2) + (q[1] - e[1]).pow(2) + (q[2] - e[2]).pow(2);
            let mut best = 0;
            for k in 1..4 {
                if d(&pal[k]) < d(&pal[best]) {
                    best = k;
                }
            }
            cbits |= (best as u32) << (2 * p);
        }
    }

    let mut out = [0u8; 16];
    out[0] = a0 as u8;
    out[1] = a1 as u8;
    for i in 0..6 {
        out[2 + i] = ((abits >> (8 * i)) & 0xFF) as u8;
    }
    out[8..10].copy_from_slice(&c0.to_le_bytes());
    out[10..12].copy_from_slice(&c1.to_le_bytes());
    out[12..16].copy_from_slice(&cbits.to_le_bytes());
    out
}

/// Compress an RGBA image (dimensions multiples of 4) to DXT5/BC3 blocks.
pub fn encode_dxt5(img: &RgbaImage) -> Vec<u8> {
    let (w, h) = img.dimensions();
    let mut out = Vec::with_capacity((w as usize / 4) * (h as usize / 4) * 16);
    for by in (0..h).step_by(4) {
        for bx in (0..w).step_by(4) {
            let mut px = [[0u8; 4]; 16];
            for y in 0..4 {
                for x in 0..4 {
                    px[(y * 4 + x) as usize] = img.get_pixel(bx + x, by + y).0;
                }
            }
            out.extend_from_slice(&encode_block(&px));
        }
    }
    out
}

/// Full mipmap chain down to 1x1. Box-filtering keeps sprite-sheet cells separate for the
/// first few levels (cell sizes are multiples of 2^n), which are the only ones chat ever uses.
fn mip_chain(img: &RgbaImage) -> Vec<RgbaImage> {
    let mut levels = vec![img.clone()];
    while levels.len() < 16 {
        let last = levels.last().unwrap();
        if last.width() <= 1 && last.height() <= 1 {
            break;
        }
        let next = box_half(last);
        levels.push(next);
    }
    levels
}

fn encode_level(img: &RgbaImage) -> Vec<u8> {
    if !img.width().is_multiple_of(4) || !img.height().is_multiple_of(4) {
        // DXT works on 4x4 blocks; pad tiny levels
        let canvas = pad(
            img,
            img.width().div_ceil(4).max(1) * 4,
            img.height().div_ceil(4).max(1) * 4,
        );
        return encode_dxt5(&canvas);
    }
    encode_dxt5(img)
}

#[allow(clippy::too_many_arguments)]
fn blp_header(
    encoding: u8,
    alpha_depth: u8,
    alpha_type: u8,
    mips: u8,
    w: u32,
    h: u32,
    offsets: &[u32],
    sizes: &[u32],
) -> Vec<u8> {
    let mut hd = Vec::with_capacity(BLP_HEADER_SIZE);
    hd.extend_from_slice(b"BLP2");
    hd.extend_from_slice(&1u32.to_le_bytes());
    hd.extend_from_slice(&[encoding, alpha_depth, alpha_type, mips]);
    hd.extend_from_slice(&w.to_le_bytes());
    hd.extend_from_slice(&h.to_le_bytes());
    for i in 0..16 {
        hd.extend_from_slice(&offsets.get(i).copied().unwrap_or(0).to_le_bytes());
    }
    for i in 0..16 {
        hd.extend_from_slice(&sizes.get(i).copied().unwrap_or(0).to_le_bytes());
    }
    hd
}

/// BLP2, DXT5 (colour encoding 2, alpha depth 8, pixel format 7), optionally with a full mip chain.
pub fn blp_bytes(img: &RgbaImage, mips: bool) -> Vec<u8> {
    let (w, h) = img.dimensions();
    let levels = if mips {
        mip_chain(img)
    } else {
        vec![img.clone()]
    };
    let datas: Vec<Vec<u8>> = levels.iter().map(encode_level).collect();
    let mut offsets = Vec::new();
    let mut sizes = Vec::new();
    let mut pos = BLP_HEADER_SIZE as u32;
    for d in &datas {
        offsets.push(pos);
        sizes.push(d.len() as u32);
        pos += d.len() as u32;
    }
    let mut out = blp_header(2, 8, 7, mips as u8, w, h, &offsets, &sizes);
    out.extend_from_slice(&[0u8; 1024]);
    debug_assert_eq!(out.len(), BLP_HEADER_SIZE);
    for d in datas {
        out.extend_from_slice(&d);
    }
    out
}

/// BLP2 with uncompressed BGRA pixels (colour encoding 3). Only used for the texture test.
pub fn blp_raw_bytes(img: &RgbaImage) -> Vec<u8> {
    let (w, h) = img.dimensions();
    let data: Vec<u8> = img
        .pixels()
        .flat_map(|p| [p[2], p[1], p[0], p[3]])
        .collect();
    let mut out = blp_header(
        3,
        8,
        8,
        0,
        w,
        h,
        &[BLP_HEADER_SIZE as u32],
        &[data.len() as u32],
    );
    out.extend_from_slice(&[0u8; 1024]);
    out.extend_from_slice(&data);
    out
}

/// BLP2 with a 256-colour palette and no alpha (encoding 1), laid out like Pillow's BLP writer.
/// Only used for the texture test (WoW doesn't show this variant on Connor's client).
pub fn blp_palette_bytes(img: &RgbaImage) -> Vec<u8> {
    let (w, h) = img.dimensions();
    // Fixed 6x7x6 colour cube; good enough for a labelled test image.
    let (lr, lg, lb) = (6u32, 7u32, 6u32);
    let mut palette = Vec::with_capacity(1024);
    for r in 0..lr {
        for g in 0..lg {
            for b in 0..lb {
                let v = |i: u32, n: u32| (i * 255 / (n - 1)) as u8;
                palette.extend_from_slice(&[v(b, lb), v(g, lg), v(r, lr), 255]);
            }
        }
    }
    palette.resize(1024, 0);
    let q = |c: u8, n: u32| ((c as u32 * (n - 1) + 127) / 255).min(n - 1);
    let data: Vec<u8> = img
        .pixels()
        .map(|p| (q(p[0], lr) * lg * lb + q(p[1], lg) * lb + q(p[2], lb)) as u8)
        .collect();
    let mut out = blp_header(1, 0, 0, 0, w, h, &[(20 + 128 + 1024) as u32], &[w * h]);
    out.extend_from_slice(&palette);
    out.extend_from_slice(&data);
    out
}

pub fn blp_size(tw: u32, th: u32) -> u64 {
    let (mut total, mut w, mut h) = (BLP_HEADER_SIZE as u64, tw, th);
    for _ in 0..16 {
        total += (w.div_ceil(4).max(1) * h.div_ceil(4).max(1) * 16) as u64;
        if w == 1 && h == 1 {
            break;
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    total
}

pub fn tga_size(tw: u32, th: u32) -> u64 {
    18 + tw as u64 * th as u64 * 4 + 26
}

/// 32-bit uncompressed TGA, bottom-up, as Pillow writes it (including the TGA 2.0 footer).
pub fn tga_bytes(img: &RgbaImage) -> Vec<u8> {
    let (w, h) = img.dimensions();
    let mut out = Vec::with_capacity(tga_size(w, h) as usize);
    out.extend_from_slice(&[0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    out.extend_from_slice(&(w as u16).to_le_bytes());
    out.extend_from_slice(&(h as u16).to_le_bytes());
    out.extend_from_slice(&[32, 8]);
    for y in (0..h).rev() {
        for x in 0..w {
            let p = img.get_pixel(x, y).0;
            out.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
    }
    out.extend_from_slice(&[0u8; 8]);
    out.extend_from_slice(b"TRUEVISION-XFILE.\0");
    out
}

fn is_blp(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("blp"))
}

/// Save as .tga or .blp depending on the extension, atomically (WoW never sees half a file).
pub fn save_texture(img: &RgbaImage, out: &Path) -> std::io::Result<()> {
    let tmp = with_suffix(out, ".part");
    let bytes = if is_blp(out) {
        if img.width() < 4 || img.height() < 4 {
            blp_bytes(&pad(img, img.width().max(4), img.height().max(4)), true)
        } else {
            blp_bytes(img, true)
        }
    } else {
        tga_bytes(img)
    };
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, out)
}

pub struct RenderOptions {
    pub size: u32,
    pub anim_size: u32,
    pub max_ratio: f64,
    pub max_frames: usize,
    pub max_sheet: u32,
    pub animate: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            size: 64,
            anim_size: 48,
            max_ratio: 4.0,
            max_frames: 48,
            max_sheet: 1024,
            animate: true,
        }
    }
}

/// Write `src` as a WoW texture at `out`. Returns the layout the addon needs.
pub fn render(src: &Path, out: &Path, opt: &RenderOptions) -> Result<Layout, String> {
    let (mut frames, mut durations) = load_frames(src)?;
    if !opt.animate {
        frames.truncate(1);
        durations.truncate(1);
    }
    let size = if frames.len() == 1 {
        opt.size
    } else {
        opt.anim_size
    };
    let (w, h) = fit(frames[0].width(), frames[0].height(), size, opt.max_ratio);

    if frames.len() == 1 {
        let (tw, th) = (pow2(w), pow2(h));
        let canvas = pad(&resize_lanczos(&frames[0], w, h), tw, th);
        save_texture(&canvas, out).map_err(|e| e.to_string())?;
        return Ok(Layout {
            w,
            h,
            tw,
            th,
            frames: 1,
            cols: 1,
            ms: 0,
        });
    }

    let (mut frames, mut durations) = decimate(&frames, &durations, opt.max_frames);
    let mut layout = sheet_layout(frames.len() as u32, w, h, opt.max_sheet);
    while layout.is_none() {
        // too many frames for the sheet: halve until it fits
        let n = (frames.len() / 2).max(1);
        (frames, durations) = decimate(&frames, &durations, n);
        layout = sheet_layout(frames.len() as u32, w, h, opt.max_sheet);
        if layout.is_none() && frames.len() == 1 {
            return Err("emote too large for a texture".into());
        }
    }
    let (cols, tw, th) = layout.unwrap();
    let mut canvas = RgbaImage::new(tw, th);
    for (i, fr) in frames.iter().enumerate() {
        let i = i as u32;
        let small = resize_lanczos(fr, w, h);
        image::imageops::replace(
            &mut canvas,
            &small,
            ((i % cols) * w) as i64,
            ((i / cols) * h) as i64,
        );
    }
    save_texture(&canvas, out).map_err(|e| e.to_string())?;
    let total: u32 = durations.iter().sum();
    let ms = py_round(total as f64 / frames.len() as f64).max(10) as u32;
    Ok(Layout {
        w,
        h,
        tw,
        th,
        frames: frames.len() as u32,
        cols,
        ms,
    })
}

/// Tiny transparent placeholder used to reserve a texture slot (.tga or .blp).
pub fn write_blank(out: &Path) -> std::io::Result<()> {
    static BLANK_BLP: OnceLock<Vec<u8>> = OnceLock::new();
    static BLANK_TGA: OnceLock<Vec<u8>> = OnceLock::new();
    if is_blp(out) {
        std::fs::write(
            out,
            BLANK_BLP.get_or_init(|| blp_bytes(&RgbaImage::new(8, 8), true)),
        )
    } else {
        std::fs::write(
            out,
            BLANK_TGA.get_or_init(|| tga_bytes(&RgbaImage::new(8, 8))),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_helpers() {
        assert_eq!(pow2(0), 1);
        assert_eq!(pow2(48), 64);
        assert_eq!(pow2(64), 64);
        assert_eq!(fit(128, 32, 64, 4.0), (256, 64));
        assert_eq!(fit(1000, 10, 64, 4.0), (256, 3));
        assert_eq!(fit(64, 64, 48, 4.0), (48, 48));
        // values checked against the Python helper
        let (cols, sw, sh) = sheet_layout(30, 48, 48, 1024).unwrap();
        assert_eq!((cols, sw, sh), (3, 256, 512));
        let (f, d) = decimate(&(0..100).collect::<Vec<_>>(), &[10; 100], 48);
        assert_eq!(f.len(), 48);
        assert_eq!(d.iter().sum::<u32>(), 1000);
    }

    #[test]
    fn sizes_match_encoders() {
        for (w, h) in [(8, 8), (64, 64), (256, 64), (512, 256), (4, 1024)] {
            let img = RgbaImage::from_pixel(w, h, Rgba([10, 200, 30, 128]));
            assert_eq!(
                blp_bytes(&img, true).len() as u64,
                blp_size(w, h),
                "{w}x{h}"
            );
            assert_eq!(tga_bytes(&img).len() as u64, tga_size(w, h));
        }
        assert_eq!(tga_bytes(&RgbaImage::new(8, 8)).len(), 300);
    }

    #[test]
    fn resize_keeps_colour_and_alpha() {
        // Regression: float resampling once clamped alpha to 1/255.
        let img = RgbaImage::from_pixel(128, 32, Rgba([255, 255, 0, 255]));
        for (w, h) in [(256, 64), (64, 16), (48, 12)] {
            let out = resize_lanczos(&img, w, h);
            assert!(out.pixels().all(|p| p.0 == [255, 255, 0, 255]), "{w}x{h}");
        }
        let mut half = RgbaImage::new(64, 64);
        for y in 0..64 {
            for x in 0..32 {
                half.put_pixel(x, y, Rgba([0, 200, 255, 255]));
            }
        }
        let out = resize_lanczos(&half, 48, 48);
        assert_eq!(out.get_pixel(5, 20).0, [0, 200, 255, 255]);
        assert_eq!(out.get_pixel(40, 20).0[3], 0);
        // No dark fringe: partly transparent edge pixels keep the colour.
        let edge = out.get_pixel(24, 20).0;
        assert!(edge[3] > 0 && edge[3] < 255 && edge[2] > 200, "{edge:?}");
    }

    #[test]
    fn dxt5_solid_block_is_exact() {
        let px = [[255, 0, 0, 255]; 16];
        let b = encode_block(&px);
        assert_eq!(&b[0..2], &[255, 255]);
        assert_eq!(u16::from_le_bytes([b[8], b[9]]), 0xF800);
        let px = [[0, 0, 0, 0]; 16];
        assert_eq!(encode_block(&px), [0u8; 16]);
    }
}
