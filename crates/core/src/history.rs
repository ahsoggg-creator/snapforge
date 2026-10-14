//! Local screenshot history: PNG files + thumbnails + a JSON index. Everything stays on this computer.

use std::fs;
use std::path::{Path, PathBuf};

use image::RgbaImage;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::imageops::FilterType as Resize;
use serde::{Deserialize, Serialize};

pub const THUMB_W: u32 = 240;
pub const THUMB_H: u32 = 150;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    /// Unix seconds (UTC).
    pub created: u64,
    pub app: String,
    pub width: u32,
    pub height: u32,
}

pub struct History {
    dir: PathBuf,
    entries: Vec<Entry>,
}

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("image: {0}")]
    Image(#[from] image::ImageError),
    #[error("no such entry")]
    Missing,
}

pub fn encode_png_fast(img: &RgbaImage) -> Result<Vec<u8>, image::ImageError> {
    let mut out = Vec::new();
    let enc = PngEncoder::new_with_quality(&mut out, CompressionType::Fast, FilterType::Adaptive);
    img.write_with_encoder(enc)?;
    Ok(out)
}

impl History {
    pub fn open(dir: &Path) -> History {
        let _ = fs::create_dir_all(dir);
        let entries = fs::read_to_string(dir.join("index.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Vec<Entry>>(&t).ok())
            .unwrap_or_default();
        let mut h = History {
            dir: dir.to_path_buf(),
            entries,
        };
        // forget entries whose image was removed by hand
        let dir = h.dir.clone();
        h.entries.retain(|e| dir.join(format!("{}.png", e.id)).exists());
        h
    }

    pub fn image_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.png"))
    }

    pub fn thumb_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.thumb.png"))
    }

    /// Newest first.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    fn save_index(&self) -> Result<(), HistoryError> {
        let tmp = self.dir.join("index.json.tmp");
        fs::write(&tmp, serde_json::to_vec(&self.entries).map_err(std::io::Error::other)?)?;
        fs::rename(&tmp, self.dir.join("index.json"))?;
        Ok(())
    }

    pub fn add(&mut self, img: &RgbaImage, app: &str, now_unix: u64) -> Result<Entry, HistoryError> {
        let mut id = format!("{now_unix}-{:04}", self.entries.len() % 10_000);
        while self.entries.iter().any(|e| e.id == id) {
            id.push('x');
        }
        fs::write(self.image_path(&id), encode_png_fast(img)?)?;
        let thumb = fit_thumbnail(img);
        fs::write(self.thumb_path(&id), encode_png_fast(&thumb)?)?;
        let entry = Entry {
            id,
            created: now_unix,
            app: app.to_owned(),
            width: img.width(),
            height: img.height(),
        };
        self.entries.insert(0, entry.clone());
        self.save_index()?;
        Ok(entry)
    }

    /// Replaces the stored picture of an entry (after the user edited and saved it).
    pub fn replace_image(&mut self, id: &str, img: &RgbaImage) -> Result<(), HistoryError> {
        let e = self
            .entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or(HistoryError::Missing)?;
        e.width = img.width();
        e.height = img.height();
        fs::write(self.dir.join(format!("{id}.png")), encode_png_fast(img)?)?;
        fs::write(
            self.dir.join(format!("{id}.thumb.png")),
            encode_png_fast(&fit_thumbnail(img))?,
        )?;
        self.save_index()
    }

    pub fn load_image(&self, id: &str) -> Result<RgbaImage, HistoryError> {
        if !self.entries.iter().any(|e| e.id == id) {
            return Err(HistoryError::Missing);
        }
        Ok(image::open(self.image_path(id))?.to_rgba8())
    }

    pub fn delete(&mut self, id: &str) -> Result<(), HistoryError> {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        if self.entries.len() == before {
            return Err(HistoryError::Missing);
        }
        let _ = fs::remove_file(self.image_path(id));
        let _ = fs::remove_file(self.thumb_path(id));
        self.save_index()
    }

    /// Deletes every screenshot. Returns how many were removed.
    pub fn clear(&mut self) -> Result<usize, HistoryError> {
        let n = self.entries.len();
        for e in std::mem::take(&mut self.entries) {
            let _ = fs::remove_file(self.image_path(&e.id));
            let _ = fs::remove_file(self.thumb_path(&e.id));
        }
        self.save_index()?;
        Ok(n)
    }

    /// Removes entries older than `days` (0 = keep everything). Returns how many were removed.
    pub fn purge_older_than(&mut self, days: u32, now_unix: u64) -> Result<usize, HistoryError> {
        if days == 0 {
            return Ok(0);
        }
        let limit = now_unix.saturating_sub(days as u64 * 86_400);
        let old: Vec<String> = self
            .entries
            .iter()
            .filter(|e| e.created < limit)
            .map(|e| e.id.clone())
            .collect();
        for id in &old {
            self.delete(id)?;
        }
        Ok(old.len())
    }

    /// Case-insensitive match on the application name, the date (`2026-10-15`) or the size (`1920x1080`).
    pub fn search(&self, query: &str) -> Vec<&Entry> {
        let q = query.trim().to_lowercase();
        self.entries
            .iter()
            .filter(|e| {
                q.is_empty()
                    || e.app.to_lowercase().contains(&q)
                    || date_string(e.created).contains(&q)
                    || format!("{}x{}", e.width, e.height).contains(&q)
            })
            .collect()
    }
}

pub fn date_string(unix: u64) -> String {
    chrono::DateTime::from_timestamp(unix as i64, 0)
        .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

/// Letter-boxes a scaled copy of `src` onto a fixed-size canvas, keeping the aspect ratio.
fn fit_thumbnail(src: &RgbaImage) -> RgbaImage {
    let (sw, sh) = (src.width().max(1), src.height().max(1));
    let scale = (THUMB_W as f32 / sw as f32).min(THUMB_H as f32 / sh as f32).min(1.0);
    let (w, h) = (((sw as f32 * scale) as u32).max(1), ((sh as f32 * scale) as u32).max(1));
    let scaled = image::imageops::resize(src, w, h, Resize::Triangle);
    let mut canvas = RgbaImage::from_pixel(THUMB_W, THUMB_H, image::Rgba([24, 24, 32, 255]));
    image::imageops::overlay(
        &mut canvas,
        &scaled,
        ((THUMB_W - w) / 2) as i64,
        ((THUMB_H - h) / 2) as i64,
    );
    canvas
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sf-hist-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    fn img(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 30, 255]))
    }

    #[test]
    fn add_list_load_delete() {
        let d = dir("basic");
        let mut h = History::open(&d);
        let e = h.add(&img(64, 32), "chrome", 1_800_000_000).unwrap();
        assert_eq!(h.entries().len(), 1);
        assert_eq!(h.load_image(&e.id).unwrap().dimensions(), (64, 32));
        assert!(h.thumb_path(&e.id).exists());
        // survives a reopen
        let h2 = History::open(&d);
        assert_eq!(h2.entries(), h.entries());
        let mut h = h2;
        h.delete(&e.id).unwrap();
        assert!(h.entries().is_empty() && !h.image_path(&e.id).exists() && !h.thumb_path(&e.id).exists());
        assert!(matches!(h.delete(&e.id), Err(HistoryError::Missing)));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn newest_first_and_unique_ids_in_same_second() {
        let d = dir("order");
        let mut h = History::open(&d);
        let a = h.add(&img(8, 8), "a", 1000).unwrap();
        let b = h.add(&img(8, 8), "b", 1000).unwrap();
        assert_ne!(a.id, b.id);
        assert_eq!(h.entries()[0].id, b.id);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn clear_removes_files() {
        let d = dir("clear");
        let mut h = History::open(&d);
        for i in 0..3 {
            h.add(&img(8, 8), "x", 1000 + i).unwrap();
        }
        assert_eq!(h.clear().unwrap(), 3);
        let pngs = fs::read_dir(&d)
            .unwrap()
            .filter(|e| e.as_ref().unwrap().path().extension().is_some_and(|x| x == "png"))
            .count();
        assert_eq!(pngs, 0);
        assert!(History::open(&d).entries().is_empty());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn retention_purges_only_old() {
        let d = dir("ret");
        let mut h = History::open(&d);
        let now = 100 * 86_400;
        h.add(&img(8, 8), "old", now - 40 * 86_400).unwrap();
        h.add(&img(8, 8), "new", now - 86_400).unwrap();
        assert_eq!(h.purge_older_than(0, now).unwrap(), 0, "0 days keeps everything");
        assert_eq!(h.purge_older_than(30, now).unwrap(), 1);
        assert_eq!(h.entries().len(), 1);
        assert_eq!(h.entries()[0].app, "new");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn search_by_app_and_size() {
        let d = dir("search");
        let mut h = History::open(&d);
        h.add(&img(10, 20), "Chrome", 1000).unwrap();
        h.add(&img(30, 40), "Code", 2000).unwrap();
        assert_eq!(h.search("chro").len(), 1);
        assert_eq!(h.search("30x40").len(), 1);
        assert_eq!(h.search("").len(), 2);
        assert_eq!(h.search("zzz").len(), 0);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn index_drops_entries_with_missing_image() {
        let d = dir("missing");
        let mut h = History::open(&d);
        let e = h.add(&img(8, 8), "x", 1000).unwrap();
        fs::remove_file(h.image_path(&e.id)).unwrap();
        assert!(History::open(&d).entries().is_empty());
        fs::remove_dir_all(&d).unwrap();
    }
}
