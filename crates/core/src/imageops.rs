//! Whole-image operations used by the post-capture chain: trim empty margins, downscale, watermarks.

use image::{Rgba, RgbaImage, imageops};
use serde::{Deserialize, Serialize};

use crate::editor::render::{measure_text, overlay};
use crate::editor::{Document, Pt, Shape};
use crate::geometry::Rect;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Corner {
    TopLeft,
    TopRight,
    #[default]
    BottomRight,
    BottomLeft,
    Center,
}

impl Corner {
    pub const ALL: [Corner; 5] = [
        Corner::TopLeft,
        Corner::TopRight,
        Corner::BottomRight,
        Corner::BottomLeft,
        Corner::Center,
    ];

    /// Top-left position of a `w`×`h` box inside a `bw`×`bh` area with `margin` pixels of padding.
    pub fn place(self, bw: u32, bh: u32, w: u32, h: u32, margin: u32) -> (i64, i64) {
        let (bw, bh, w, h, m) = (bw as i64, bh as i64, w as i64, h as i64, margin as i64);
        match self {
            Corner::TopLeft => (m, m),
            Corner::TopRight => (bw - w - m, m),
            Corner::BottomLeft => (m, bh - h - m),
            Corner::BottomRight => (bw - w - m, bh - h - m),
            Corner::Center => ((bw - w) / 2, (bh - h) / 2),
        }
    }
}

fn close(a: &Rgba<u8>, b: &Rgba<u8>, tol: u8) -> bool {
    (0..3).all(|i| a[i].abs_diff(b[i]) <= tol)
}

/// Bounding box of everything that differs from the background colour (taken from the top-left pixel), plus `pad`
/// pixels. `None` if the picture is empty-looking (all background) or there is nothing to trim.
pub fn content_bounds(img: &RgbaImage, tolerance: u8, pad: u32) -> Option<Rect> {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    let bg = *img.get_pixel(0, 0);
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0u32, 0u32);
    for (x, y, p) in img.enumerate_pixels() {
        if !close(p, &bg, tolerance) {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    if x1 < x0 {
        return None;
    }
    let r = Rect::new(
        x0.saturating_sub(pad) as i32,
        y0.saturating_sub(pad) as i32,
        (x1 + 1 + pad).min(w) as i32 - x0.saturating_sub(pad) as i32,
        (y1 + 1 + pad).min(h) as i32 - y0.saturating_sub(pad) as i32,
    );
    (r.w < w as i32 || r.h < h as i32).then_some(r)
}

/// Crops uniform margins. Returns the picture unchanged when there is nothing to trim.
pub fn trim_margins(img: &RgbaImage, tolerance: u8, pad: u32) -> RgbaImage {
    match content_bounds(img, tolerance, pad) {
        Some(r) => imageops::crop_imm(img, r.x as u32, r.y as u32, r.w as u32, r.h as u32).to_image(),
        None => img.clone(),
    }
}

/// Fits the picture into `max_w`×`max_h` keeping the aspect ratio; never enlarges.
pub fn downscale_to_fit(img: &RgbaImage, max_w: u32, max_h: u32) -> RgbaImage {
    let (w, h) = img.dimensions();
    let k = (max_w.max(1) as f64 / w as f64)
        .min(max_h.max(1) as f64 / h as f64)
        .min(1.0);
    if k >= 1.0 {
        return img.clone();
    }
    imageops::resize(
        img,
        ((w as f64 * k).round() as u32).max(1),
        ((h as f64 * k).round() as u32).max(1),
        imageops::FilterType::Lanczos3,
    )
}

pub fn scale_percent(img: &RgbaImage, percent: u32) -> RgbaImage {
    let k = percent.clamp(1, 100) as f64 / 100.0;
    if percent >= 100 {
        return img.clone();
    }
    let (w, h) = img.dimensions();
    imageops::resize(
        img,
        ((w as f64 * k).round() as u32).max(1),
        ((h as f64 * k).round() as u32).max(1),
        imageops::FilterType::Lanczos3,
    )
}

/// Text watermark in a corner. `opacity` 0.0–1.0; the size follows the picture width unless `size` is given.
pub fn watermark_text(img: &RgbaImage, text: &str, corner: Corner, opacity: f32, size: Option<f32>) -> RgbaImage {
    if text.trim().is_empty() {
        return img.clone();
    }
    let size = size.unwrap_or((img.width() as f32 / 48.0).clamp(14.0, 64.0));
    let (tw, th) = measure_text(text, size);
    let margin = (size * 0.6) as u32;
    let (x, y) = corner.place(img.width(), img.height(), tw as u32, th as u32, margin);
    let mut d = Document::new(img.clone());
    let alpha = (opacity.clamp(0.0, 1.0) * 255.0) as u8;
    // a faint dark shadow keeps the mark readable on both light and dark backgrounds
    d.add(
        Shape::Text {
            at: Pt::new(x as f32 + 1.5, y as f32 + 1.5),
            text: text.to_owned(),
            size,
        },
        [0, 0, 0, alpha / 2],
        1.0,
    );
    d.add(
        Shape::Text {
            at: Pt::new(x as f32, y as f32),
            text: text.to_owned(),
            size,
        },
        [255, 255, 255, alpha],
        1.0,
    );
    d.render()
}

/// Logo watermark: `mark` is scaled to `width_percent` of the picture width (never enlarged).
pub fn watermark_image(
    img: &RgbaImage,
    mark: &RgbaImage,
    corner: Corner,
    opacity: f32,
    width_percent: u32,
) -> RgbaImage {
    let target_w = ((img.width() as f64 * width_percent.clamp(1, 100) as f64 / 100.0) as u32)
        .max(1)
        .min(mark.width());
    let k = target_w as f64 / mark.width().max(1) as f64;
    let target_h = ((mark.height() as f64 * k) as u32).max(1);
    let scaled = imageops::resize(mark, target_w, target_h, imageops::FilterType::Lanczos3);
    let (x, y) = corner.place(img.width(), img.height(), target_w, target_h, (img.width() / 60).max(6));
    let mut out = img.clone();
    overlay(&mut out, &scaled, x, y, opacity);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bg(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba([255, 255, 255, 255]))
    }

    #[test]
    fn trim_removes_uniform_margins() {
        let mut img = bg(100, 80);
        for y in 20..50 {
            for x in 30..70 {
                img.put_pixel(x, y, Rgba([10, 10, 10, 255]));
            }
        }
        let r = content_bounds(&img, 4, 0).unwrap();
        assert_eq!(r, Rect::new(30, 20, 40, 30));
        let t = trim_margins(&img, 4, 3);
        assert_eq!(t.dimensions(), (46, 36), "content plus 3 px padding on each side");
    }

    #[test]
    fn trim_is_a_noop_when_content_touches_every_edge_or_is_blank() {
        assert!(
            trim_margins(&bg(10, 10), 4, 0).dimensions() == (10, 10),
            "blank picture stays"
        );
        let mut img = bg(10, 10);
        for (i, p) in img.pixels_mut().enumerate() {
            if i % 3 == 0 {
                *p = Rgba([0, 0, 0, 255]);
            }
        }
        // background (0,0) is black here, so content is the white pixels, which span the whole picture
        assert_eq!(trim_margins(&img, 4, 0).dimensions(), (10, 10));
    }

    #[test]
    fn trim_tolerates_noise_within_tolerance() {
        let mut img = bg(40, 40);
        img.put_pixel(5, 5, Rgba([252, 252, 252, 255])); // compression noise
        img.put_pixel(20, 20, Rgba([0, 0, 0, 255]));
        let r = content_bounds(&img, 6, 0).unwrap();
        assert_eq!(r, Rect::new(20, 20, 1, 1));
    }

    #[test]
    fn downscale_keeps_aspect_and_never_enlarges() {
        let img = bg(400, 200);
        assert_eq!(downscale_to_fit(&img, 100, 100).dimensions(), (100, 50));
        assert_eq!(downscale_to_fit(&img, 1000, 1000).dimensions(), (400, 200));
        assert_eq!(scale_percent(&img, 50).dimensions(), (200, 100));
        assert_eq!(scale_percent(&img, 100).dimensions(), (400, 200));
    }

    #[test]
    fn corners_place_boxes_inside_the_area() {
        assert_eq!(Corner::TopLeft.place(100, 50, 20, 10, 5), (5, 5));
        assert_eq!(Corner::BottomRight.place(100, 50, 20, 10, 5), (75, 35));
        assert_eq!(Corner::Center.place(100, 50, 20, 10, 5), (40, 20));
    }

    #[test]
    fn text_watermark_changes_only_its_corner() {
        let img = bg(400, 200);
        let out = watermark_text(&img, "SnapForge", Corner::BottomRight, 1.0, Some(24.0));
        assert_eq!(out.dimensions(), img.dimensions());
        assert_eq!(out.get_pixel(5, 5), img.get_pixel(5, 5), "top-left untouched");
        if crate::editor::render::has_font() {
            let changed = (200..400).any(|x| (120..200).any(|y| out.get_pixel(x, y) != img.get_pixel(x, y)));
            assert!(changed, "mark drawn in the bottom-right quarter");
        }
        assert_eq!(
            watermark_text(&img, "  ", Corner::BottomRight, 1.0, None),
            img,
            "empty text is a no-op"
        );
    }

    #[test]
    fn image_watermark_blends() {
        let img = bg(200, 100);
        let mark = RgbaImage::from_pixel(40, 20, Rgba([0, 0, 0, 255]));
        let out = watermark_image(&img, &mark, Corner::TopLeft, 0.5, 20);
        let p = out.get_pixel(10, 10).0[0];
        assert!(p > 100 && p < 160, "half-transparent black over white: {p}");
        assert_eq!(out.get_pixel(190, 90).0[0], 255);
    }
}
