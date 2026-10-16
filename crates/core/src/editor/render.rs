//! Rasterises the base image plus vector objects with tiny-skia. The editor preview and the export use this one
//! code path, so what you see is what you save.

use std::path::PathBuf;
use std::sync::OnceLock;

use ab_glyph::{Font, FontArc, PxScale, ScaleFont, point};
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use tiny_skia::{
    BlendMode, Color as SkColor, FillRule, LineCap, LineJoin, Mask, Paint, PathBuilder, Pixmap, PixmapPaint, Stroke,
    Transform,
};

use super::model::{Color, Object, Pt, Shape, norm_rect, step_radius};
use crate::geometry::Rect;

static FONT: OnceLock<Option<FontArc>> = OnceLock::new();

fn font() -> Option<&'static FontArc> {
    FONT.get_or_init(|| {
        let win = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\Windows"));
        let mut candidates: Vec<PathBuf> = ["segoeui.ttf", "arial.ttf", "tahoma.ttf", "calibri.ttf"]
            .iter()
            .map(|f| win.join("Fonts").join(f))
            .collect();
        candidates.push(PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"));
        candidates
            .into_iter()
            .find_map(|p| std::fs::read(p).ok().and_then(|b| FontArc::try_from_vec(b).ok()))
    })
    .as_ref()
}

/// True when a system font could be loaded (text objects render nothing otherwise).
pub fn has_font() -> bool {
    font().is_some()
}

/// Size `(width, height)` of `text` at `size` pixels, multi-line aware.
pub fn measure_text(text: &str, size: f32) -> (f32, f32) {
    let lines = text.split('\n').count().max(1) as f32;
    let Some(f) = font() else {
        let w = text.split('\n').map(|l| l.chars().count()).max().unwrap_or(0) as f32;
        return (w * size * 0.55, lines * size * 1.25);
    };
    let sc = f.as_scaled(PxScale::from(size));
    let w = text
        .split('\n')
        .map(|l| l.chars().map(|c| sc.h_advance(sc.glyph_id(c))).sum::<f32>())
        .fold(0.0, f32::max);
    (w, lines * sc.height() * 1.05)
}

fn sk(c: Color) -> SkColor {
    SkColor::from_rgba8(c[0], c[1], c[2], c[3])
}

pub(crate) fn paint(c: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(sk(c));
    p.anti_alias = true;
    p
}

pub(crate) fn stroke(width: f32, cap: LineCap) -> Stroke {
    Stroke {
        width: width.max(1.0),
        line_cap: cap,
        line_join: LineJoin::Round,
        ..Stroke::default()
    }
}

fn to_pixmap(img: &RgbaImage) -> Pixmap {
    let mut pm = Pixmap::new(img.width().max(1), img.height().max(1)).expect("non-zero size");
    for (dst, src) in pm.data_mut().chunks_exact_mut(4).zip(img.as_raw().chunks_exact(4)) {
        let a = src[3] as u32;
        if a == 255 {
            dst.copy_from_slice(src);
        } else {
            dst[0] = (src[0] as u32 * a / 255) as u8;
            dst[1] = (src[1] as u32 * a / 255) as u8;
            dst[2] = (src[2] as u32 * a / 255) as u8;
            dst[3] = a as u8;
        }
    }
    pm
}

fn from_pixmap(pm: &Pixmap) -> RgbaImage {
    let mut out = Vec::with_capacity(pm.data().len());
    for px in pm.data().chunks_exact(4) {
        let a = px[3] as u32;
        if a == 0 || a == 255 {
            out.extend_from_slice(px);
        } else {
            out.extend_from_slice(&[
                (px[0] as u32 * 255 / a).min(255) as u8,
                (px[1] as u32 * 255 / a).min(255) as u8,
                (px[2] as u32 * 255 / a).min(255) as u8,
                a as u8,
            ]);
        }
    }
    RgbaImage::from_raw(pm.width(), pm.height(), out).expect("same size")
}

/// A title bar drawn above the screenshot, so it looks like an app window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Chrome {
    #[default]
    None,
    Dark,
    Light,
}

/// "Frame" around a screenshot: gradient background, rounded corners, drop shadow, optional title bar.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Beautify {
    pub padding: u32,
    pub radius: u32,
    pub shadow: bool,
    pub top: Color,
    pub bottom: Color,
    /// Gradient direction in degrees: 0 runs from `top` down to `bottom`, 90 from left to right.
    pub angle: u16,
    pub chrome: Chrome,
}

impl Default for Beautify {
    fn default() -> Beautify {
        Beautify {
            padding: 48,
            radius: 14,
            shadow: true,
            top: [96, 165, 250, 255],
            bottom: [168, 85, 247, 255],
            angle: 0,
            chrome: Chrome::None,
        }
    }
}

/// Ready-made backgrounds: `(name key, top, bottom, angle)`.
pub const BEAUTIFY_PRESETS: [(&str, Color, Color, u16); 8] = [
    ("bg.sky", [96, 165, 250, 255], [168, 85, 247, 255], 0),
    ("bg.sunset", [251, 146, 60, 255], [219, 39, 119, 255], 35),
    ("bg.ocean", [34, 211, 238, 255], [37, 99, 235, 255], 45),
    ("bg.mint", [167, 243, 208, 255], [52, 211, 153, 255], 20),
    ("bg.night", [30, 41, 59, 255], [88, 28, 135, 255], 60),
    ("bg.peach", [254, 215, 170, 255], [253, 164, 175, 255], 90),
    ("bg.mono", [229, 231, 235, 255], [156, 163, 175, 255], 0),
    ("bg.ink", [24, 24, 27, 255], [63, 63, 70, 255], 0),
];

impl Beautify {
    /// The look of `BEAUTIFY_PRESETS[i]` (colours and angle) with the rest of `self` kept.
    pub fn with_preset(&self, i: usize) -> Beautify {
        let (_, top, bottom, angle) = BEAUTIFY_PRESETS[i % BEAUTIFY_PRESETS.len()];
        Beautify {
            top,
            bottom,
            angle,
            ..self.clone()
        }
    }

    /// Pixels the title bar adds on top of a picture `w` wide (0 without one).
    pub fn bar_height(&self, w: u32) -> u32 {
        match self.chrome {
            Chrome::None => 0,
            _ => (w as f32 / 24.0).clamp(28.0, 44.0) as u32,
        }
    }
}

/// The picture with a title bar (three window buttons) above it.
fn with_chrome(base: &RgbaImage, o: &Beautify) -> RgbaImage {
    let (w, h) = base.dimensions();
    let bar = o.bar_height(w);
    if bar == 0 {
        return base.clone();
    }
    let bar_color: Color = match o.chrome {
        Chrome::Light => [236, 236, 238, 255],
        _ => [38, 38, 42, 255],
    };
    let mut out = RgbaImage::from_pixel(w, h + bar, image::Rgba(bar_color));
    image::imageops::replace(&mut out, base, 0, bar as i64);
    let mut pm = to_pixmap(&out);
    let r = bar as f32 * 0.17;
    for (i, c) in [[255u8, 95, 86, 255], [255, 189, 46, 255], [39, 201, 63, 255]]
        .into_iter()
        .enumerate()
    {
        let cx = bar as f32 * (0.55 + 0.5 * i as f32);
        let mut pb = PathBuilder::new();
        pb.push_circle(cx, bar as f32 / 2.0, r);
        if let Some(p) = pb.finish() {
            pm.fill_path(&p, &paint(c), FillRule::Winding, Transform::identity(), None);
        }
    }
    from_pixmap(&pm)
}

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    let r = r.clamp(0.0, w.min(h) / 2.0);
    let k = 0.552_284_7 * r;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

/// The picture inside `o.padding` pixels of vertical gradient, with rounded corners and an optional shadow.
pub fn beautify(base: &RgbaImage, o: &Beautify) -> RgbaImage {
    let framed = with_chrome(base, o);
    let base = &framed;
    let (w, h) = base.dimensions();
    let pad = o.padding.min(400);
    let (cw, ch) = (w + 2 * pad, h + 2 * pad);
    let mut bg = RgbaImage::new(cw, ch);
    let (sin, cos) = (f32::from(o.angle % 360).to_radians()).sin_cos();
    let (mx, my) = ((cw.max(2) - 1) as f32, (ch.max(2) - 1) as f32);
    let extent = (mx * sin.abs() + my * cos.abs()).max(1.0);
    for (y, row) in bg.rows_mut().enumerate() {
        for (x, p) in row.enumerate() {
            let along = (x as f32 - mx / 2.0) * sin + (y as f32 - my / 2.0) * cos;
            let t = (0.5 + along / extent).clamp(0.0, 1.0);
            let lerp = |i: usize| (o.top[i] as f32 + (o.bottom[i] as f32 - o.top[i] as f32) * t).round() as u8;
            *p = image::Rgba([lerp(0), lerp(1), lerp(2), 255]);
        }
    }
    let mut pm = to_pixmap(&bg);
    let id = Transform::identity();
    let (x, y, fw, fh) = (pad as f32, pad as f32, w as f32, h as f32);
    if o.shadow
        && pad > 0
        && let Some(mut sh) = Pixmap::new(cw, ch)
        && let Some(path) = rounded_rect(x, y + (pad as f32 * 0.25).min(24.0), fw, fh, o.radius as f32)
    {
        sh.fill_path(&path, &paint([0, 0, 0, 150]), FillRule::Winding, id, None);
        blur_region(
            &mut sh,
            Pt::new(0.0, 0.0),
            Pt::new(cw as f32, ch as f32),
            (pad / 3).max(4),
        );
        pm.draw_pixmap(0, 0, sh.as_ref(), &PixmapPaint::default(), id, None);
    }
    let src = to_pixmap(base);
    let mask = Mask::new(cw, ch).and_then(|mut m| {
        let path = rounded_rect(x, y, fw, fh, o.radius as f32)?;
        m.fill_path(&path, FillRule::Winding, true, id);
        Some(m)
    });
    pm.draw_pixmap(
        pad as i32,
        pad as i32,
        src.as_ref(),
        &PixmapPaint::default(),
        id,
        mask.as_ref(),
    );
    from_pixmap(&pm)
}

/// Renders `objects` (in order) over `base`.
pub fn render(base: &RgbaImage, objects: &[Object]) -> RgbaImage {
    let mut pm = to_pixmap(base);
    draw_objects(&mut pm, objects);
    from_pixmap(&pm)
}

fn draw_objects(pm: &mut Pixmap, objects: &[Object]) {
    let any_eraser = objects
        .iter()
        .any(|o| o.visible && matches!(o.shape, Shape::Erase { .. }));
    for (i, o) in objects.iter().enumerate().filter(|(_, o)| o.visible) {
        match &o.shape {
            Shape::Erase { .. } => {}
            sh if any_eraser && sh.is_ink() => {
                let erasers: Vec<&Object> = objects[i + 1..]
                    .iter()
                    .filter(|e| e.visible && matches!(e.shape, Shape::Erase { .. }))
                    .collect();
                if erasers.is_empty() {
                    draw_object(pm, o);
                } else {
                    draw_erased(pm, o, &erasers);
                }
            }
            _ => draw_object(pm, o),
        }
    }
}

/// Draws `o` on a transparent layer, wipes the eraser strokes that come after it out of that layer, and puts the
/// layer on the picture, so the picture underneath is never erased.
fn draw_erased(pm: &mut Pixmap, o: &Object, erasers: &[&Object]) {
    let (lo, hi) = o.shape.bounds(o.width);
    let m = (o.width * 4.0).max(16.0);
    let (x0, y0) = ((lo.x - m).floor().max(0.0) as i32, (lo.y - m).floor().max(0.0) as i32);
    let (x1, y1) = (
        ((hi.x + m).ceil().max(0.0) as i32).min(pm.width() as i32),
        ((hi.y + m).ceil().max(0.0) as i32).min(pm.height() as i32),
    );
    let Some(mut layer) = (x1 > x0 && y1 > y0)
        .then(|| Pixmap::new((x1 - x0) as u32, (y1 - y0) as u32))
        .flatten()
    else {
        return;
    };
    let mut local = o.clone();
    local.shape.translate(-(x0 as f32), -(y0 as f32));
    draw_object(&mut layer, &local);
    let wipe = Paint {
        blend_mode: BlendMode::Clear,
        anti_alias: true,
        ..Paint::default()
    };
    for e in erasers {
        let Shape::Erase { pts } = &e.shape else { continue };
        let shifted: Vec<Pt> = pts.iter().map(|p| Pt::new(p.x - x0 as f32, p.y - y0 as f32)).collect();
        if let Some(path) = smooth_path(&shifted) {
            layer.stroke_path(
                &path,
                &wipe,
                &stroke(e.width, LineCap::Round),
                Transform::identity(),
                None,
            );
        }
    }
    pm.draw_pixmap(
        x0,
        y0,
        layer.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        None,
    );
}

/// Renders only the part of the picture inside `area`, exactly as `render` would draw it there, and returns the area
/// that was really rendered (it grows to take in whole blur, pixelate and chart regions, which read their
/// surroundings) with its pixels. Moving an object only has to redraw the pixels it touched.
pub fn render_region(base: &RgbaImage, objects: &[Object], area: Rect) -> Option<(Rect, RgbaImage)> {
    let full = Rect::new(0, 0, base.width() as i32, base.height() as i32);
    let mut r = area.intersect(&full)?;
    let reads_below = |o: &&Object| {
        o.visible
            && matches!(
                o.shape,
                Shape::Blur { .. } | Shape::Pixelate { .. } | Shape::Chart { .. }
            )
    };
    loop {
        let mut grown = r;
        for o in objects.iter().filter(reads_below) {
            let (lo, hi) = o.shape.bounds(0.0);
            let b = Rect::new(
                lo.x.floor() as i32,
                lo.y.floor() as i32,
                (hi.x.ceil() - lo.x.floor()) as i32 + 1,
                (hi.y.ceil() - lo.y.floor()) as i32 + 1,
            );
            if let Some(b) = b.intersect(&full)
                && b.intersect(&r).is_some()
            {
                grown = grown.union(&b);
            }
        }
        if grown == r {
            break;
        }
        r = grown;
    }
    // most of the picture anyway: one pass over everything is cheaper than cutting it up
    if r.area() * 10 >= full.area() * 6 {
        return Some((full, render(base, objects)));
    }
    let crop = image::imageops::crop_imm(base, r.x as u32, r.y as u32, r.w as u32, r.h as u32).to_image();
    let mut pm = to_pixmap(&crop);
    let near: Vec<Object> = objects
        .iter()
        .filter(|o| o.visible)
        .filter(|o| {
            if matches!(o.shape, Shape::Spotlight { .. }) {
                return true;
            }
            let (lo, hi) = o.shape.bounds(o.width);
            let m = (o.width * 8.0).max(40.0);
            Rect::new(
                (lo.x - m).floor() as i32,
                (lo.y - m).floor() as i32,
                (hi.x - lo.x + 2.0 * m) as i32 + 2,
                (hi.y - lo.y + 2.0 * m) as i32 + 2,
            )
            .intersect(&r)
            .is_some()
        })
        .map(|o| {
            let mut c = o.clone();
            c.shape.translate(-(r.x as f32), -(r.y as f32));
            c
        })
        .collect();
    draw_objects(&mut pm, &near);
    Some((r, from_pixmap(&pm)))
}

fn smooth_path(pts: &[Pt]) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(pts.first()?.x, pts[0].y);
    if pts.len() == 1 {
        pb.line_to(pts[0].x + 0.01, pts[0].y);
    } else if pts.len() == 2 {
        pb.line_to(pts[1].x, pts[1].y);
    } else {
        for w in pts.windows(2) {
            let mid = Pt::new((w[0].x + w[1].x) / 2.0, (w[0].y + w[1].y) / 2.0);
            pb.quad_to(w[0].x, w[0].y, mid.x, mid.y);
        }
        let last = pts[pts.len() - 1];
        pb.line_to(last.x, last.y);
    }
    pb.finish()
}

fn draw_object(pm: &mut Pixmap, o: &Object) {
    let id = Transform::identity();
    match &o.shape {
        Shape::Line { a, b } => {
            let mut pb = PathBuilder::new();
            pb.move_to(a.x, a.y);
            pb.line_to(b.x, b.y);
            if let Some(p) = pb.finish() {
                pm.stroke_path(&p, &paint(o.color), &stroke(o.width, LineCap::Round), id, None);
            }
        }
        Shape::Arrow { a, b } => draw_arrow(pm, *a, *b, o),
        Shape::Rect { a, b, fill } => {
            let (lo, hi) = norm_rect(*a, *b);
            if let Some(r) = tiny_skia::Rect::from_ltrb(lo.x, lo.y, hi.x.max(lo.x + 0.5), hi.y.max(lo.y + 0.5)) {
                let path = PathBuilder::from_rect(r);
                if *fill {
                    pm.fill_path(&path, &paint(o.color), FillRule::Winding, id, None);
                } else {
                    pm.stroke_path(&path, &paint(o.color), &stroke(o.width, LineCap::Butt), id, None);
                }
            }
        }
        Shape::Ellipse { a, b, fill } => {
            let (lo, hi) = norm_rect(*a, *b);
            if let Some(r) = tiny_skia::Rect::from_ltrb(lo.x, lo.y, hi.x.max(lo.x + 0.5), hi.y.max(lo.y + 0.5))
                && let Some(path) = PathBuilder::from_oval(r)
            {
                if *fill {
                    pm.fill_path(&path, &paint(o.color), FillRule::Winding, id, None);
                } else {
                    pm.stroke_path(&path, &paint(o.color), &stroke(o.width, LineCap::Butt), id, None);
                }
            }
        }
        Shape::Pencil { pts } => {
            if let Some(p) = smooth_path(pts) {
                pm.stroke_path(&p, &paint(o.color), &stroke(o.width, LineCap::Round), id, None);
            }
        }
        Shape::Marker { pts } => {
            if let Some(p) = smooth_path(pts) {
                let mut c = o.color;
                c[3] = (c[3] as u32 * 110 / 255) as u8;
                pm.stroke_path(&p, &paint(c), &stroke(o.width * 5.0, LineCap::Butt), id, None);
            }
        }
        Shape::Erase { .. } => {}
        Shape::Text { at, text, size } => draw_text(pm, text, *at, *size, o.color),
        Shape::Blur { a, b, radius } => {
            let (lo, hi) = norm_rect(*a, *b);
            blur_region(pm, lo, hi, *radius);
        }
        Shape::Pixelate { a, b, block } => {
            let (lo, hi) = norm_rect(*a, *b);
            pixelate_region(pm, lo, hi, *block);
        }
        Shape::Spotlight { a, b } => {
            let (lo, hi) = norm_rect(*a, *b);
            let mut pb = PathBuilder::new();
            if let Some(all) = tiny_skia::Rect::from_ltrb(0.0, 0.0, pm.width() as f32, pm.height() as f32) {
                pb.push_rect(all);
            }
            if let Some(hole) = tiny_skia::Rect::from_ltrb(lo.x, lo.y, hi.x.max(lo.x + 0.5), hi.y.max(lo.y + 0.5)) {
                pb.push_rect(hole);
            }
            if let Some(p) = pb.finish() {
                pm.fill_path(&p, &paint([0, 0, 0, 150]), FillRule::EvenOdd, id, None);
            }
        }
        Shape::Chart { a, b, spec } => super::chart::draw(pm, *a, *b, spec, o.color),
        Shape::Step { at, n } => {
            let r = step_radius(o.width);
            let mut pb = PathBuilder::new();
            pb.push_circle(at.x, at.y, r);
            if let Some(p) = pb.finish() {
                pm.fill_path(&p, &paint(o.color), FillRule::Winding, id, None);
                let ring = [255, 255, 255, 230];
                pm.stroke_path(&p, &paint(ring), &stroke((r * 0.12).max(1.5), LineCap::Butt), id, None);
            }
            let lum = 0.299 * o.color[0] as f32 + 0.587 * o.color[1] as f32 + 0.114 * o.color[2] as f32;
            let ink = if lum > 150.0 {
                [20, 20, 20, 255]
            } else {
                [255, 255, 255, 255]
            };
            let label = n.to_string();
            let size = r * 1.15;
            let (tw, th) = measure_text(&label, size);
            draw_text(pm, &label, Pt::new(at.x - tw / 2.0, at.y - th / 2.0), size, ink);
        }
    }
}

fn draw_arrow(pm: &mut Pixmap, a: Pt, b: Pt, o: &Object) {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return;
    }
    let (ux, uy) = (dx / len, dy / len);
    let head = (o.width * 4.5).max(12.0).min(len * 0.9);
    let half = head * 0.42;
    let base = Pt::new(b.x - ux * head, b.y - uy * head);
    let mut shaft = PathBuilder::new();
    shaft.move_to(a.x, a.y);
    // end the shaft inside the head so the join has no gap
    shaft.line_to(base.x + ux * head * 0.3, base.y + uy * head * 0.3);
    if let Some(p) = shaft.finish() {
        pm.stroke_path(
            &p,
            &paint(o.color),
            &stroke(o.width, LineCap::Round),
            Transform::identity(),
            None,
        );
    }
    let mut tri = PathBuilder::new();
    tri.move_to(b.x, b.y);
    tri.line_to(base.x - uy * half, base.y + ux * half);
    tri.line_to(base.x + uy * half, base.y - ux * half);
    tri.close();
    if let Some(p) = tri.finish() {
        pm.fill_path(&p, &paint(o.color), FillRule::Winding, Transform::identity(), None);
    }
}

fn blend(dst: &mut [u8], c: Color, cov: f32) {
    let a = (c[3] as f32 / 255.0) * cov.clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    for i in 0..3 {
        dst[i] = (c[i] as f32 * a + dst[i] as f32 * (1.0 - a)).round() as u8;
    }
    dst[3] = ((a + dst[3] as f32 / 255.0 * (1.0 - a)) * 255.0).round() as u8;
}

pub(crate) fn draw_text(pm: &mut Pixmap, text: &str, at: Pt, size: f32, color: Color) {
    let Some(f) = font() else { return };
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    let sc = f.as_scaled(PxScale::from(size));
    let line_h = sc.height() * 1.05;
    let data = pm.data_mut();
    for (li, line) in text.split('\n').enumerate() {
        let baseline = at.y + sc.ascent() + li as f32 * line_h;
        let mut caret = at.x;
        let mut prev = None;
        for ch in line.chars() {
            let gid = sc.glyph_id(ch);
            if let Some(p) = prev {
                caret += sc.kern(p, gid);
            }
            let glyph = gid.with_scale_and_position(PxScale::from(size), point(caret, baseline));
            caret += sc.h_advance(gid);
            prev = Some(gid);
            if let Some(og) = f.outline_glyph(glyph) {
                let b = og.px_bounds();
                og.draw(|gx, gy, cov| {
                    let (x, y) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                    if x >= 0 && y >= 0 && x < w && y < h {
                        let i = (y as usize * w as usize + x as usize) * 4;
                        blend(&mut data[i..i + 4], color, cov);
                    }
                });
            }
        }
    }
}

fn clamp_region(pm: &Pixmap, lo: Pt, hi: Pt) -> Option<(usize, usize, usize, usize)> {
    let x0 = lo.x.floor().max(0.0) as usize;
    let y0 = lo.y.floor().max(0.0) as usize;
    let x1 = (hi.x.ceil().max(0.0) as usize).min(pm.width() as usize);
    let y1 = (hi.y.ceil().max(0.0) as usize).min(pm.height() as usize);
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
}

/// Three box-blur passes (≈ Gaussian) over the region; edges are clamped to the region.
pub fn blur_region(pm: &mut Pixmap, lo: Pt, hi: Pt, radius: u32) {
    let Some((x0, y0, x1, y1)) = clamp_region(pm, lo, hi) else {
        return;
    };
    let stride = pm.width() as usize * 4;
    let (rw, rh) = (x1 - x0, y1 - y0);
    let r = (radius.max(1) as usize).min(rw.max(rh));
    let data = pm.data_mut();
    let mut buf = vec![0u8; rw * rh * 4];
    for y in 0..rh {
        let s = (y0 + y) * stride + x0 * 4;
        buf[y * rw * 4..(y + 1) * rw * 4].copy_from_slice(&data[s..s + rw * 4]);
    }
    let mut tmp = vec![0u8; buf.len()];
    for _ in 0..3 {
        box_pass(&buf, &mut tmp, rw, rh, r, true);
        box_pass(&tmp, &mut buf, rw, rh, r, false);
    }
    for y in 0..rh {
        let s = (y0 + y) * stride + x0 * 4;
        data[s..s + rw * 4].copy_from_slice(&buf[y * rw * 4..(y + 1) * rw * 4]);
    }
}

fn box_pass(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize, horizontal: bool) {
    let (lines, len) = if horizontal { (h, w) } else { (w, h) };
    let idx = |line: usize, i: usize| {
        if horizontal {
            (line * w + i) * 4
        } else {
            (i * w + line) * 4
        }
    };
    let win = (2 * r + 1) as u32;
    for line in 0..lines {
        let mut sum = [0u32; 4];
        for k in -(r as isize)..=(r as isize) {
            let i = k.clamp(0, len as isize - 1) as usize;
            for c in 0..4 {
                sum[c] += src[idx(line, i) + c] as u32;
            }
        }
        for i in 0..len {
            let o = idx(line, i);
            for c in 0..4 {
                dst[o + c] = (sum[c] / win) as u8;
            }
            let add = (i + r + 1).min(len - 1);
            let sub = (i as isize - r as isize).max(0) as usize;
            for c in 0..4 {
                sum[c] = sum[c] + src[idx(line, add) + c] as u32 - src[idx(line, sub) + c] as u32;
            }
        }
    }
}

/// Replaces each `block`×`block` cell of the region with its average colour.
pub fn pixelate_region(pm: &mut Pixmap, lo: Pt, hi: Pt, block: u32) {
    let Some((x0, y0, x1, y1)) = clamp_region(pm, lo, hi) else {
        return;
    };
    let b = block.max(2) as usize;
    let stride = pm.width() as usize * 4;
    let data = pm.data_mut();
    let mut by = y0;
    while by < y1 {
        let ye = (by + b).min(y1);
        let mut bx = x0;
        while bx < x1 {
            let xe = (bx + b).min(x1);
            let mut sum = [0u32; 4];
            for y in by..ye {
                for x in bx..xe {
                    let i = y * stride + x * 4;
                    for c in 0..4 {
                        sum[c] += data[i + c] as u32;
                    }
                }
            }
            let n = ((ye - by) * (xe - bx)) as u32;
            for y in by..ye {
                for x in bx..xe {
                    let i = y * stride + x * 4;
                    for c in 0..4 {
                        data[i + c] = (sum[c] / n) as u8;
                    }
                }
            }
            bx = xe;
        }
        by = ye;
    }
}

/// Draws `src` onto `dst` at integer position (used by watermarks and plugins).
pub fn overlay(dst: &mut RgbaImage, src: &RgbaImage, x: i64, y: i64, opacity: f32) {
    let mut pm = to_pixmap(dst);
    let s = to_pixmap(src);
    let paint = PixmapPaint {
        opacity: opacity.clamp(0.0, 1.0),
        ..PixmapPaint::default()
    };
    pm.draw_pixmap(x as i32, y as i32, s.as_ref(), &paint, Transform::identity(), None);
    *dst = from_pixmap(&pm);
}

#[cfg(test)]
mod tests {
    use super::super::model::{DEFAULT_COLOR, Document};
    use super::*;
    use image::Rgba;

    fn white(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba([255, 255, 255, 255]))
    }

    fn obj(shape: Shape, color: Color, width: f32) -> Object {
        Object {
            id: 1,
            shape,
            color,
            width,
            visible: true,
        }
    }

    #[test]
    fn no_objects_is_identity() {
        let img = white(20, 10);
        assert_eq!(render(&img, &[]), img);
    }

    #[test]
    fn filled_rect_colours_exactly_its_area() {
        let r = obj(
            Shape::Rect {
                a: Pt::new(10.0, 10.0),
                b: Pt::new(30.0, 20.0),
                fill: true,
            },
            [0, 0, 255, 255],
            2.0,
        );
        let out = render(&white(50, 40), &[r]);
        assert_eq!(out.get_pixel(15, 15).0, [0, 0, 255, 255]);
        assert_eq!(out.get_pixel(5, 5).0, [255, 255, 255, 255]);
        assert_eq!(out.get_pixel(35, 15).0, [255, 255, 255, 255]);
    }

    #[test]
    fn hollow_ellipse_keeps_the_centre() {
        let e = obj(
            Shape::Ellipse {
                a: Pt::new(5.0, 5.0),
                b: Pt::new(45.0, 35.0),
                fill: false,
            },
            [255, 0, 0, 255],
            3.0,
        );
        let out = render(&white(50, 40), &[e]);
        assert_eq!(out.get_pixel(25, 20).0, [255, 255, 255, 255], "centre untouched");
        assert_ne!(
            out.get_pixel(25, 5).0,
            [255, 255, 255, 255],
            "top of the outline is drawn"
        );
    }

    #[test]
    fn arrow_draws_head_at_the_tip() {
        let a = obj(
            Shape::Arrow {
                a: Pt::new(5.0, 20.0),
                b: Pt::new(55.0, 20.0),
            },
            [0, 0, 0, 255],
            4.0,
        );
        let out = render(&white(60, 40), &[a]);
        assert_eq!(out.get_pixel(30, 20).0, [0, 0, 0, 255], "shaft");
        // the head is wider than the shaft near the tip
        assert_ne!(out.get_pixel(40, 24).0, [255, 255, 255, 255]);
    }

    #[test]
    fn marker_is_translucent() {
        let m = obj(
            Shape::Marker {
                pts: vec![Pt::new(5.0, 10.0), Pt::new(45.0, 10.0)],
            },
            [255, 255, 0, 255],
            4.0,
        );
        let out = render(&white(50, 20), &[m]);
        let p = out.get_pixel(25, 10).0;
        assert_eq!(p[0], 255);
        assert!(p[2] > 40 && p[2] < 255, "blue channel is partially covered: {p:?}");
    }

    #[test]
    fn pixelate_averages_each_cell() {
        let mut img = white(8, 8);
        for y in 0..8 {
            for x in 0..4 {
                img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        let p = obj(
            Shape::Pixelate {
                a: Pt::new(0.0, 0.0),
                b: Pt::new(8.0, 8.0),
                block: 8,
            },
            DEFAULT_COLOR,
            1.0,
        );
        let out = render(&img, &[p]);
        let v = out.get_pixel(0, 0).0[0];
        assert!((126..=128).contains(&v), "average of black and white halves: {v}");
        assert_eq!(out.get_pixel(7, 7).0[0], v);
    }

    #[test]
    fn blur_smooths_a_hard_edge_only_inside_the_region() {
        let mut img = white(40, 10);
        for y in 0..10 {
            for x in 0..20 {
                img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        let b = obj(
            Shape::Blur {
                a: Pt::new(10.0, 0.0),
                b: Pt::new(30.0, 10.0),
                radius: 4,
            },
            DEFAULT_COLOR,
            1.0,
        );
        let out = render(&img, &[b]);
        let edge = out.get_pixel(20, 5).0[0];
        assert!(edge > 20 && edge < 235, "edge is smoothed: {edge}");
        assert_eq!(out.get_pixel(2, 5).0[0], 0, "outside the region stays sharp");
        assert_eq!(out.get_pixel(38, 5).0[0], 255);
    }

    #[test]
    fn blur_actually_hides_text_like_detail() {
        // checkerboard (maximum detail) becomes nearly uniform
        let mut img = white(32, 32);
        for y in 0..32 {
            for x in 0..32 {
                let v = if (x + y) % 2 == 0 { 0 } else { 255 };
                img.put_pixel(x, y, Rgba([v, v, v, 255]));
            }
        }
        let b = obj(
            Shape::Blur {
                a: Pt::new(0.0, 0.0),
                b: Pt::new(32.0, 32.0),
                radius: 3,
            },
            DEFAULT_COLOR,
            1.0,
        );
        let out = render(&img, &[b]);
        let vals: Vec<u8> = (8..24).map(|x| out.get_pixel(x, 16).0[0]).collect();
        let spread = vals.iter().max().unwrap() - vals.iter().min().unwrap();
        assert!(spread < 60, "detail is gone: {vals:?}");
    }

    #[test]
    fn step_badge_is_drawn_with_a_number() {
        let s = obj(
            Shape::Step {
                at: Pt::new(30.0, 30.0),
                n: 7,
            },
            [200, 30, 30, 255],
            4.0,
        );
        let out = render(&white(60, 60), &[s]);
        assert_eq!(
            out.get_pixel(30 - 15, 30).0[..3],
            [200, 30, 30],
            "badge colour near its edge"
        );
        assert_eq!(out.get_pixel(2, 2).0, [255, 255, 255, 255]);
        if has_font() {
            // the badge is red (green channel 30); the white label lights some pixels up
            let white_inside = (22..38).any(|x| (22..38).any(|y| out.get_pixel(x, y).0[1] > 120));
            assert!(white_inside, "number glyph pixels");
        }
    }

    #[test]
    fn text_renders_pixels_and_measures() {
        if !has_font() {
            return;
        }
        let t = obj(
            Shape::Text {
                at: Pt::new(4.0, 4.0),
                text: "Hello".into(),
                size: 24.0,
            },
            [0, 0, 0, 255],
            1.0,
        );
        let out = render(&white(120, 50), &[t]);
        assert!(out.pixels().any(|p| p.0[0] < 100), "dark glyph pixels exist");
        let (w, h) = measure_text("Hello", 24.0);
        assert!(w > 30.0 && w < 110.0 && h > 20.0, "{w}x{h}");
        let (w2, _) = measure_text("Hello\nHello world", 24.0);
        assert!(w2 > w, "width is the longest line");
    }

    #[test]
    fn objects_remain_vector_until_render() {
        // moving an object after the fact changes the render without any accumulated raster damage
        let mut d = Document::new(white(40, 40));
        let id = d.add(
            Shape::Rect {
                a: Pt::new(0.0, 0.0),
                b: Pt::new(10.0, 10.0),
                fill: true,
            },
            [0, 0, 0, 255],
            2.0,
        );
        assert_eq!(d.render().get_pixel(5, 5).0[0], 0);
        d.get_mut(id).unwrap().shape.translate(20.0, 20.0);
        let out = d.render();
        assert_eq!(out.get_pixel(5, 5).0[0], 255, "old position is clean again");
        assert_eq!(out.get_pixel(25, 25).0[0], 0);
    }

    #[test]
    fn overlay_blends_with_opacity() {
        let mut dst = white(10, 10);
        let src = RgbaImage::from_pixel(4, 4, Rgba([0, 0, 0, 255]));
        overlay(&mut dst, &src, 2, 2, 0.5);
        let v = dst.get_pixel(3, 3).0[0];
        assert!((120..=135).contains(&v), "{v}");
        assert_eq!(dst.get_pixel(8, 8).0[0], 255);
    }
    fn filled(a: (f32, f32), b: (f32, f32), color: Color) -> Object {
        obj(
            Shape::Rect {
                a: Pt::new(a.0, a.1),
                b: Pt::new(b.0, b.1),
                fill: true,
            },
            color,
            2.0,
        )
    }

    fn stroke_of(pts: &[(f32, f32)], width: f32) -> Object {
        obj(
            Shape::Erase {
                pts: pts.iter().map(|p| Pt::new(p.0, p.1)).collect(),
            },
            [0, 0, 0, 255],
            width,
        )
    }

    #[test]
    fn the_eraser_wipes_drawn_marks_but_never_the_picture() {
        let base = RgbaImage::from_pixel(60, 40, Rgba([200, 220, 240, 255]));
        let red = [255, 0, 0, 255];
        let out = render(
            &base,
            &[
                filled((5.0, 5.0), (55.0, 35.0), red),
                stroke_of(&[(10.0, 20.0), (50.0, 20.0)], 10.0),
            ],
        );
        assert_eq!(
            out.get_pixel(30, 20).0,
            [200, 220, 240, 255],
            "the stroke shows the picture, not black or white"
        );
        assert_eq!(out.get_pixel(30, 8).0, red, "outside the stroke the mark stays");
        assert_eq!(out.get_pixel(30, 32).0, red);
        assert_eq!(out.get_pixel(1, 1).0, [200, 220, 240, 255], "the picture is untouched");
    }

    #[test]
    fn the_eraser_only_reaches_what_was_drawn_before_it() {
        let base = white(60, 40);
        let out = render(
            &base,
            &[
                stroke_of(&[(10.0, 20.0), (50.0, 20.0)], 12.0),
                filled((5.0, 5.0), (55.0, 35.0), [0, 0, 255, 255]),
            ],
        );
        assert_eq!(
            out.get_pixel(30, 20).0,
            [0, 0, 255, 255],
            "a later mark covers the old stroke"
        );
        let hidden = Object {
            visible: false,
            ..stroke_of(&[(10.0, 20.0), (50.0, 20.0)], 12.0)
        };
        let out = render(&base, &[filled((5.0, 5.0), (55.0, 35.0), [0, 0, 255, 255]), hidden]);
        assert_eq!(
            out.get_pixel(30, 20).0,
            [0, 0, 255, 255],
            "a hidden stroke erases nothing"
        );
    }

    #[test]
    fn the_eraser_leaves_blur_and_pixelate_alone() {
        let base = RgbaImage::from_fn(60, 40, |x, y| {
            if (x / 2 + y / 2) % 2 == 0 {
                Rgba([0, 0, 0, 255])
            } else {
                Rgba([255, 255, 255, 255])
            }
        });
        let pix = obj(
            Shape::Pixelate {
                a: Pt::new(10.0, 10.0),
                b: Pt::new(50.0, 30.0),
                block: 20,
            },
            [0, 0, 0, 255],
            1.0,
        );
        let with = render(&base, &[pix.clone(), stroke_of(&[(10.0, 20.0), (50.0, 20.0)], 14.0)]);
        let without = render(&base, &[pix]);
        assert_eq!(with, without, "pixelate hides what is under it for good");
    }

    #[test]
    fn a_region_render_matches_the_full_render() {
        let base = RgbaImage::from_fn(200, 120, |x, y| {
            Rgba([(x * 3) as u8, (y * 5) as u8, ((x + y) * 2) as u8, 255])
        });
        let objects = vec![
            filled((20.0, 20.0), (120.0, 80.0), [255, 0, 0, 255]),
            obj(
                Shape::Blur {
                    a: Pt::new(90.0, 40.0),
                    b: Pt::new(160.0, 100.0),
                    radius: 6,
                },
                [0, 0, 0, 255],
                1.0,
            ),
            obj(
                Shape::Pixelate {
                    a: Pt::new(10.0, 70.0),
                    b: Pt::new(60.0, 110.0),
                    block: 7,
                },
                [0, 0, 0, 255],
                1.0,
            ),
            obj(
                Shape::Marker {
                    pts: vec![Pt::new(30.0, 30.0), Pt::new(100.0, 50.0), Pt::new(140.0, 30.0)],
                },
                [255, 220, 0, 255],
                4.0,
            ),
            obj(
                Shape::Arrow {
                    a: Pt::new(15.0, 100.0),
                    b: Pt::new(95.0, 60.5),
                },
                [0, 255, 0, 255],
                5.0,
            ),
            stroke_of(&[(40.0, 60.0), (80.0, 70.0), (110.0, 45.0)], 14.0),
            obj(
                Shape::Spotlight {
                    a: Pt::new(5.0, 5.0),
                    b: Pt::new(190.0, 110.0),
                },
                [0, 0, 0, 255],
                1.0,
            ),
        ];
        let full = render(&base, &objects);
        for area in [
            Rect::new(25, 25, 40, 30),
            Rect::new(100, 50, 30, 30),
            Rect::new(0, 0, 30, 30),
            Rect::new(150, 80, 80, 80),
        ] {
            let (got, img) = render_region(&base, &objects, area).expect("inside the picture");
            assert_eq!(img.dimensions(), (got.w as u32, got.h as u32));
            assert!(got.intersect(&area).is_some_and(|i| i.w > 0));
            // the same shapes, drawn from slightly different float coordinates: a handful of anti-aliased edge pixels
            // may flip a sub-pixel sample, everything else is identical
            let mut off = 0;
            for y in 0..got.h {
                for x in 0..got.w {
                    let (a, b) = (
                        img.get_pixel(x as u32, y as u32).0,
                        full.get_pixel((got.x + x) as u32, (got.y + y) as u32).0,
                    );
                    let worst = a.iter().zip(b).map(|(p, q)| p.abs_diff(q)).max().unwrap();
                    assert!(
                        worst <= 64,
                        "pixel ({}, {}) of {area:?} -> {got:?}: {a:?} vs {b:?}",
                        got.x + x,
                        got.y + y
                    );
                    off += usize::from(worst > 2);
                }
            }
            assert!(off <= 4, "{off} edge pixels differ in {area:?}");
        }
        assert!(render_region(&base, &objects, Rect::new(500, 500, 10, 10)).is_none());
    }
}
