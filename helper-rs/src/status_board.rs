//! Live status shown inside WoW without a reload.
//!
//! WoW reads a texture file's contents the first time the file is displayed, as long as the file
//! existed when the game started. So the helper keeps a ring of placeholder images,
//! status/st0000.tga ... status/stNNNN.tga, and writes its current status (as a picture of text) into
//! the few files the addon will display next. The addon picks the file for the current time:
//!
//! ```text
//! index = floor(unix_time / PERIOD) % COUNT
//! ```
//!
//! and shows a new one every PERIOD seconds, so what you see in game is at most a few seconds old.
//! Files the clock has passed are blanked again so the folder stays small.

use crate::draw;
use crate::images;
use chrono::{Local, TimeZone};
use image::RgbaImage;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const PERIOD: u64 = 3; // seconds each status image is shown
pub const COUNT: u64 = 4800; // ring size: 4 hours before an index repeats
pub const AHEAD: u64 = 6; // images written ahead of the clock
pub const SIZE: (u32, u32) = (384, 48);

pub fn color(kind: &str) -> [u8; 3] {
    match kind {
        "ready" => [64, 220, 96],
        "warn" => [255, 176, 64],
        "error" => [255, 96, 96],
        "idle" => [150, 150, 160],
        _ => [41, 182, 246], // busy
    }
}

pub fn render(text: &str, sub: &str, kind: &str) -> RgbaImage {
    let (w, h) = SIZE;
    let mut img = RgbaImage::new(w, h);
    draw::rounded_rect(&mut img, 0, 0, w - 1, h - 1, 8, [14, 14, 18, 235]);
    let c = color(kind);
    draw::rounded_rect(&mut img, 0, 0, 6, h - 1, 3, [c[0], c[1], c[2], 255]);
    let bold = draw::font(true);
    let mut size = 19.0;
    while size > 12.0 && draw::text_length(text, bold, size) > (w - 24) as f32 {
        size -= 1.0;
    }
    let y = 5.0 + ((19.0 - size) / 2.0).floor();
    draw::draw_text(&mut img, 16.0, y, text, bold, size, [255, 255, 255, 255]);
    draw::draw_text(
        &mut img,
        16.0,
        28.0,
        sub,
        draw::font(false),
        13.0,
        [c[0], c[1], c[2], 255],
    );
    img
}

pub fn index_at(t: f64) -> u64 {
    ((t / PERIOD as f64).floor() as u64) % COUNT
}

fn local_hms(unix: i64) -> String {
    match Local.timestamp_opt(unix, 0).single() {
        Some(t) => t.format("%H:%M:%S").to_string(),
        None => String::new(),
    }
}

pub struct StatusBoard {
    pub dir: PathBuf,
    /// index -> (text, kind) written there
    content: HashMap<u64, (String, String)>,
    pub current: Option<(String, String)>,
}

impl StatusBoard {
    pub fn new(folder: &Path) -> Self {
        Self {
            dir: folder.to_path_buf(),
            content: HashMap::new(),
            current: None,
        }
    }

    pub fn path(&self, i: u64) -> PathBuf {
        self.dir.join(format!("st{i:04}.tga"))
    }

    /// Create any missing placeholder files. Returns how many were created
    /// (new files only become visible to WoW after it restarts).
    pub fn ensure_files(&self) -> std::io::Result<usize> {
        std::fs::create_dir_all(&self.dir)?;
        let mut made = 0;
        for i in 0..COUNT {
            let p = self.path(i);
            if !p.exists() {
                images::write_blank(&p)?;
                made += 1;
            }
        }
        Ok(made)
    }

    /// Show `text` from the next status image on. Cheap to call often.
    pub fn update(&mut self, text: &str, kind: &str, now: Option<f64>) {
        let now = now.unwrap_or_else(crate::now);
        self.current = Some((text.to_string(), kind.to_string()));
        let k = index_at(now);
        let base = (now / PERIOD as f64).floor() as i64;
        for step in 0..=AHEAD {
            let i = (k + step) % COUNT;
            let want = (text.to_string(), kind.to_string());
            if self.content.get(&i) == Some(&want) {
                continue;
            }
            let stamp = local_hms((base + step as i64) * PERIOD as i64);
            let img = render(text, &format!("7TV Emotes Helper  -  live {stamp}"), kind);
            match images::save_texture(&img, &self.path(i)) {
                Ok(()) => {
                    self.content.insert(i, want);
                }
                Err(e) => log::info!("status image {i}: {e}"),
            }
        }
        // Blank the images the clock has passed (keeps the folder small).
        let behind: Vec<u64> = self
            .content
            .keys()
            .copied()
            .filter(|&i| {
                let d = (k as i64 - i as i64).rem_euclid(COUNT as i64) as u64;
                d > 2 && d < COUNT - AHEAD - 1
            })
            .collect();
        for i in behind {
            let _ = images::write_blank(&self.path(i));
            self.content.remove(&i);
        }
    }

    pub fn info(&self) -> crate::luasv::Value {
        crate::luasv::TableBuilder::new()
            .set("dir", "status")
            .set("count", COUNT as i64)
            .set("period", PERIOD as i64)
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blanks_old_images() {
        let tmp = crate::testutil::TempDir::new("board");
        let mut b = StatusBoard::new(&tmp.path().join("status"));
        b.ensure_files().unwrap();
        let t0 = 1_800_000_000.0;
        b.update("A", "busy", Some(t0));
        let first = index_at(t0);
        b.update("A", "busy", Some(t0 + PERIOD as f64 * 20.0));
        let size = |i: u64| std::fs::metadata(b.path(i)).unwrap().len();
        assert!(size(first) < 400);
        let k = index_at(t0 + PERIOD as f64 * 20.0);
        assert!(size(k) > 1000);
    }

    #[test]
    fn long_text_shrinks_and_renders() {
        let img = render(
            &"Downloading emotes 1234/5678 and a very long tail".repeat(2),
            "sub",
            "warn",
        );
        assert_eq!(img.dimensions(), SIZE);
        // some white text pixels exist
        assert!(img.pixels().any(|p| p[0] > 240 && p[1] > 240 && p[2] > 240));
    }
}
