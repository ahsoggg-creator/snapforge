//! Offline face detection with a small ONNX model run by `tract` (pure Rust, no native libraries).
//!
//! The model is NOT bundled in the executable. It is downloaded on request, its SHA-256 is verified against the
//! value pinned below, and it lives in the data folder (`models\`). Without it, detection reports "model not
//! installed" instead of pretending to work.
//!
//! Model: Ultra-Light-Fast-Generic-Face-Detector-1MB, `version-RFB-320` (MIT licence).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use image::{RgbaImage, imageops};
use sha2::{Digest, Sha256};
use tract_onnx::prelude::*;

use crate::geometry::Rect;
use crate::paths;

pub const MODEL_URL: &str =
    "https://github.com/Linzaer/Ultra-Light-Fast-Generic-Face-Detector-1MB/raw/master/models/onnx/version-RFB-320.onnx";
pub const MODEL_SHA256: &str = "34cd7e60aeff28744c657de7a3dc64e872d506741de66987f3426f2b79f88017";
pub const MODEL_FILE: &str = "face-detector-rfb-320.onnx";
pub const MODEL_SIZE_HINT: &str = "1.2 MB";

const IN_W: usize = 320;
const IN_H: usize = 240;
const TILE_W: u32 = 640;
const TILE_H: u32 = 480;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelState {
    /// Not downloaded yet.
    Missing,
    Ready,
    /// A file exists but its hash is wrong (damaged or replaced).
    Corrupt,
}

pub fn model_path() -> PathBuf {
    paths::models_dir().join(MODEL_FILE)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

pub fn model_state_at(path: &Path) -> ModelState {
    match std::fs::read(path) {
        Err(_) => ModelState::Missing,
        Ok(b) if sha256_hex(&b) == MODEL_SHA256 => ModelState::Ready,
        Ok(_) => ModelState::Corrupt,
    }
}

pub fn model_state() -> ModelState {
    model_state_at(&model_path())
}

/// Downloads the model to `dest`, verifying the pinned SHA-256 before the file gets its final name.
pub fn download_model_to(
    dest: &Path,
    url: &str,
    expected_sha256: &str,
    progress: &dyn Fn(u64, u64),
) -> Result<(), String> {
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let agent = crate::net::agent(180);
    let mut resp = agent
        .get(url)
        .header("User-Agent", &crate::net::user_agent())
        .call()
        .map_err(|e| format!("download failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("download failed: HTTP {}", resp.status().as_u16()));
    }
    let total = resp.body().content_length().unwrap_or(0);
    let mut reader = resp.body_mut().as_reader();
    let mut data = Vec::new();
    let mut buf = vec![0u8; 1 << 15];
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
        progress(data.len() as u64, total.max(data.len() as u64));
        if data.len() > 64 << 20 {
            return Err("the file is unexpectedly large; aborted".into());
        }
    }
    if sha256_hex(&data) != expected_sha256 {
        return Err("SHA-256 of the downloaded model does not match the expected value; the file was discarded".into());
    }
    let tmp = dest.with_extension("onnx.part");
    std::fs::File::create(&tmp)
        .and_then(|mut f| f.write_all(&data))
        .map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, dest).map_err(|e| e.to_string())
}

pub fn download_model(progress: &dyn Fn(u64, u64)) -> Result<(), String> {
    download_model_to(&model_path(), MODEL_URL, MODEL_SHA256, progress)
}

#[derive(Debug, thiserror::Error)]
pub enum FaceError {
    #[error("the face detection model is not installed (download it in the settings or in the post-capture panel)")]
    ModelMissing,
    #[error("the model file is damaged (SHA-256 mismatch); download it again")]
    ModelCorrupt,
    #[error("face detection failed: {0}")]
    Failed(String),
}

type Runnable = std::sync::Arc<TypedRunnableModel>;

pub struct FaceDetector {
    model: Runnable,
}

fn tf<E: std::fmt::Display>(e: E) -> FaceError {
    FaceError::Failed(e.to_string())
}

impl FaceDetector {
    /// Loads the verified model from the data folder.
    pub fn load() -> Result<FaceDetector, FaceError> {
        FaceDetector::load_from(&model_path())
    }

    pub fn load_from(path: &Path) -> Result<FaceDetector, FaceError> {
        match model_state_at(path) {
            ModelState::Missing => return Err(FaceError::ModelMissing),
            ModelState::Corrupt => return Err(FaceError::ModelCorrupt),
            ModelState::Ready => {}
        }
        let model = tract_onnx::onnx()
            .model_for_path(path)
            .map_err(tf)?
            .with_input_fact(0, InferenceFact::dt_shape(f32::datum_type(), tvec![1, 3, IN_H, IN_W]))
            .map_err(tf)?
            .into_optimized()
            .map_err(tf)?
            .into_runnable()
            .map_err(tf)?;
        Ok(FaceDetector { model })
    }

    /// Faces in `img` as pixel rectangles (already padded a little so a blur covers the whole face).
    pub fn detect(&self, img: &RgbaImage, threshold: f32) -> Result<Vec<Rect>, FaceError> {
        let mut cands = Vec::new();
        for tile in tiles(img.width(), img.height()) {
            cands.extend(self.run_tile(img, tile, threshold)?);
        }
        let kept = nms(cands, 0.35);
        Ok(kept
            .into_iter()
            .map(|c| pad_box(c.rect, img.width(), img.height()))
            .collect())
    }

    fn run_tile(&self, img: &RgbaImage, tile: Rect, threshold: f32) -> Result<Vec<Candidate>, FaceError> {
        let crop = imageops::crop_imm(img, tile.x as u32, tile.y as u32, tile.w as u32, tile.h as u32).to_image();
        let small = imageops::resize(&crop, IN_W as u32, IN_H as u32, imageops::FilterType::Triangle);
        let input: Tensor = tract_ndarray::Array4::from_shape_fn((1, 3, IN_H, IN_W), |(_, c, y, x)| {
            (small.get_pixel(x as u32, y as u32)[c] as f32 - 127.0) / 128.0
        })
        .into();
        let out = self.model.run(tvec!(input.into())).map_err(tf)?;
        let (mut scores, mut boxes) = (None, None);
        for t in out.iter() {
            let a = t.to_plain_array_view::<f32>().map_err(tf)?;
            match a.shape().last() {
                Some(2) => scores = Some(a.iter().copied().collect::<Vec<f32>>()),
                Some(4) => boxes = Some(a.iter().copied().collect::<Vec<f32>>()),
                _ => {}
            }
        }
        let (scores, boxes) = scores
            .zip(boxes)
            .ok_or_else(|| FaceError::Failed("unexpected model outputs".into()))?;
        Ok(decode(&scores, &boxes, tile, threshold))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    pub rect: Rect,
    pub score: f32,
}

/// Turns raw model outputs (`N×2` scores, `N×4` normalised corner boxes) into pixel candidates inside `tile`.
pub fn decode(scores: &[f32], boxes: &[f32], tile: Rect, threshold: f32) -> Vec<Candidate> {
    scores
        .chunks_exact(2)
        .zip(boxes.chunks_exact(4))
        .filter(|(s, _)| s[1] >= threshold)
        .filter_map(|(s, b)| {
            let (x1, y1, x2, y2) = (
                b[0].clamp(0.0, 1.0),
                b[1].clamp(0.0, 1.0),
                b[2].clamp(0.0, 1.0),
                b[3].clamp(0.0, 1.0),
            );
            let (tw, th) = (tile.w as f32, tile.h as f32);
            let rect = Rect::new(
                tile.x + (x1 * tw).round() as i32,
                tile.y + (y1 * th).round() as i32,
                ((x2 - x1) * tw).round() as i32,
                ((y2 - y1) * th).round() as i32,
            );
            (rect.w >= 8 && rect.h >= 8).then_some(Candidate { rect, score: s[1] })
        })
        .collect()
}

fn iou(a: &Rect, b: &Rect) -> f32 {
    match a.intersect(b) {
        None => 0.0,
        Some(i) => i.area() as f32 / (a.area() + b.area() - i.area()) as f32,
    }
}

/// Non-maximum suppression: keeps the best box of every overlapping group.
pub fn nms(mut c: Vec<Candidate>, iou_limit: f32) -> Vec<Candidate> {
    c.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut kept: Vec<Candidate> = Vec::new();
    for cand in c {
        if kept.iter().all(|k| iou(&k.rect, &cand.rect) <= iou_limit) {
            kept.push(cand);
        }
    }
    kept
}

/// Grows a face box by 15 % each way and clamps it to the picture.
pub fn pad_box(r: Rect, w: u32, h: u32) -> Rect {
    let (dx, dy) = ((r.w as f32 * 0.15) as i32, (r.h as f32 * 0.15) as i32);
    Rect::new(r.x - dx, r.y - dy, r.w + 2 * dx, r.h + 2 * dy)
        .intersect(&Rect::new(0, 0, w as i32, h as i32))
        .unwrap_or(r)
}

/// Overlapping 4:3 windows covering the picture (plus the whole picture when it is larger than one window), so faces
/// that are small relative to a big screenshot are still seen by the small-input model.
pub fn tiles(w: u32, h: u32) -> Vec<Rect> {
    let axis = |len: u32, tile: u32| -> Vec<u32> {
        let step = tile * 3 / 4;
        let mut v = Vec::new();
        let mut p = 0u32;
        loop {
            v.push(p.min(len.saturating_sub(tile)));
            if p + tile >= len {
                break;
            }
            p += step;
        }
        v
    };
    let (tw, th) = (TILE_W.min(w), TILE_H.min(h));
    let mut out = Vec::new();
    if w > TILE_W || h > TILE_H {
        out.push(Rect::new(0, 0, w as i32, h as i32));
    }
    for y in axis(h, th) {
        for x in axis(w, tw) {
            out.push(Rect::new(x as i32, y as i32, tw as i32, th as i32));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_scales_boxes_into_the_tile() {
        let scores = [0.9, 0.1, 0.2, 0.8, 0.4, 0.6];
        let boxes = [0.1, 0.2, 0.3, 0.6, 0.5, 0.5, 0.7, 0.9, 0.0, 0.0, 0.01, 0.01];
        let c = decode(&scores, &boxes, Rect::new(100, 50, 640, 480), 0.7);
        assert_eq!(
            c.len(),
            1,
            "only the 0.8 face passes; the third box is too small anyway"
        );
        assert_eq!(c[0].rect, Rect::new(100 + 320, 50 + 240, 128, 192));
        assert!((c[0].score - 0.8).abs() < 1e-6);
    }

    #[test]
    fn nms_keeps_the_best_of_overlapping_boxes_only() {
        let c = |x, y, s| Candidate {
            rect: Rect::new(x, y, 100, 100),
            score: s,
        };
        let kept = nms(vec![c(0, 0, 0.7), c(5, 5, 0.9), c(400, 0, 0.8)], 0.35);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].rect.x, 5, "best score first");
        assert_eq!(kept[1].rect.x, 400);
    }

    #[test]
    fn padding_stays_inside_the_picture() {
        assert_eq!(pad_box(Rect::new(0, 0, 100, 100), 500, 500), Rect::new(0, 0, 115, 115));
        let p = pad_box(Rect::new(450, 450, 50, 50), 500, 500);
        assert!(p.right() <= 500 && p.bottom() <= 500 && p.x < 450);
    }

    #[test]
    fn tiles_cover_every_pixel() {
        for (w, h) in [(320u32, 200u32), (640, 480), (1920, 1080), (5760, 1080), (700, 500)] {
            let t = tiles(w, h);
            let windows: Vec<&Rect> = t.iter().skip(usize::from(w > TILE_W || h > TILE_H)).collect();
            for r in &windows {
                assert!(
                    r.x >= 0 && r.y >= 0 && r.right() <= w as i32 && r.bottom() <= h as i32,
                    "{r:?} inside {w}x{h}"
                );
            }
            for y in (0..h as i32).step_by(7).chain([h as i32 - 1]) {
                for x in (0..w as i32).step_by(7).chain([w as i32 - 1]) {
                    assert!(
                        windows.iter().any(|r| r.contains(x, y)),
                        "({x},{y}) is not covered in {w}x{h}"
                    );
                }
            }
        }
    }

    #[test]
    fn small_pictures_use_a_single_tile() {
        assert_eq!(tiles(300, 200), vec![Rect::new(0, 0, 300, 200)]);
    }

    #[test]
    fn missing_and_corrupt_models_are_reported_honestly() {
        let d = std::env::temp_dir().join(format!("sf-face-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("m.onnx");
        assert_eq!(model_state_at(&p), ModelState::Missing);
        assert!(matches!(FaceDetector::load_from(&p), Err(FaceError::ModelMissing)));
        std::fs::write(&p, b"not the model").unwrap();
        assert_eq!(model_state_at(&p), ModelState::Corrupt);
        assert!(matches!(FaceDetector::load_from(&p), Err(FaceError::ModelCorrupt)));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn sha256_helper_matches_a_known_value() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    /// Needs the network: downloads the real model (verifying the pinned hash) and finds faces in the model
    /// author's sample picture. `cargo test -p snapforge-core -- --ignored face_detection_end_to_end`
    #[test]
    #[ignore = "needs the network"]
    fn face_detection_end_to_end() {
        let d = std::env::temp_dir().join(format!("sf-face-e2e-{}", std::process::id()));
        let model = d.join(MODEL_FILE);
        download_model_to(&model, MODEL_URL, MODEL_SHA256, &|_, _| {}).expect("download + SHA-256 check");
        assert_eq!(model_state_at(&model), ModelState::Ready);
        // a wrong pinned hash must refuse the file
        let bad = d.join("bad.onnx");
        assert!(download_model_to(&bad, MODEL_URL, &"0".repeat(64), &|_, _| {}).is_err());
        assert!(!bad.exists());

        let sample = d.join("sample.jpg");
        let bytes = crate::net::agent(60)
            .get("https://github.com/Linzaer/Ultra-Light-Fast-Generic-Face-Detector-1MB/raw/master/imgs/1.jpg")
            .header("User-Agent", &crate::net::user_agent())
            .call()
            .unwrap()
            .body_mut()
            .read_to_vec()
            .unwrap();
        std::fs::write(&sample, bytes).unwrap();
        let img = image::open(&sample).unwrap().to_rgba8();
        let det = FaceDetector::load_from(&model).expect("load");
        let faces = det.detect(&img, 0.7).expect("detect");
        eprintln!("{}x{} picture, faces: {faces:?}", img.width(), img.height());
        assert!(!faces.is_empty(), "at least one face in the sample picture");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
