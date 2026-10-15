//! Editor document: a raster base plus a list of vector objects that stay editable until export.

use std::sync::Arc;

use image::RgbaImage;
use serde::{Deserialize, Serialize};

use super::render;
use crate::geometry::Rect;

pub type Color = [u8; 4];

pub const DEFAULT_COLOR: Color = [230, 40, 40, 255];
pub const DEFAULT_WIDTH: f32 = 4.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Pt {
    pub x: f32,
    pub y: f32,
}

impl Pt {
    pub const fn new(x: f32, y: f32) -> Pt {
        Pt { x, y }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Shape {
    Line {
        a: Pt,
        b: Pt,
    },
    Arrow {
        a: Pt,
        b: Pt,
    },
    Rect {
        a: Pt,
        b: Pt,
        fill: bool,
    },
    Ellipse {
        a: Pt,
        b: Pt,
        fill: bool,
    },
    Text {
        at: Pt,
        text: String,
        size: f32,
    },
    Pencil {
        pts: Vec<Pt>,
    },
    Marker {
        pts: Vec<Pt>,
    },
    Blur {
        a: Pt,
        b: Pt,
        radius: u32,
    },
    Pixelate {
        a: Pt,
        b: Pt,
        block: u32,
    },
    Step {
        at: Pt,
        n: u32,
    },
    /// Dims everything outside the rectangle, so the eye goes to what is inside.
    Spotlight {
        a: Pt,
        b: Pt,
    },
    /// A chart drawn from a data table; `color` of the object is the text colour (alpha 0 = default).
    Chart {
        a: Pt,
        b: Pt,
        spec: super::chart::ChartSpec,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Object {
    pub id: u64,
    pub shape: Shape,
    pub color: Color,
    /// Stroke width in image pixels (for `Step`: derives the badge radius).
    pub width: f32,
    /// Hidden objects stay in the document but are not drawn or exported.
    #[serde(default = "yes")]
    pub visible: bool,
}

fn yes() -> bool {
    true
}

fn dist_to_segment(p: Pt, a: Pt, b: Pt) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0)
    };
    ((p.x - (a.x + t * dx)).powi(2) + (p.y - (a.y + t * dy)).powi(2)).sqrt()
}

pub fn norm_rect(a: Pt, b: Pt) -> (Pt, Pt) {
    (Pt::new(a.x.min(b.x), a.y.min(b.y)), Pt::new(a.x.max(b.x), a.y.max(b.y)))
}

pub fn step_radius(width: f32) -> f32 {
    10.0 + width * 2.0
}

impl Shape {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Shape::Line { .. } => "line",
            Shape::Arrow { .. } => "arrow",
            Shape::Rect { .. } => "rect",
            Shape::Ellipse { .. } => "ellipse",
            Shape::Text { .. } => "text",
            Shape::Pencil { .. } => "pencil",
            Shape::Marker { .. } => "marker",
            Shape::Blur { .. } => "blur",
            Shape::Pixelate { .. } => "pixelate",
            Shape::Step { .. } => "step",
            Shape::Chart { .. } => "chart",
            Shape::Spotlight { .. } => "spotlight",
        }
    }

    /// Scales every length of the shape by `f` (the picture was resized): widths are handled by the caller.
    fn scale_params(&mut self, f: f32) {
        match self {
            Shape::Text { size, .. } => *size *= f,
            Shape::Blur { radius, .. } => *radius = ((*radius as f32 * f).round() as u32).max(1),
            Shape::Pixelate { block, .. } => *block = ((*block as f32 * f).round() as u32).max(2),
            _ => {}
        }
    }

    fn points_mut(&mut self) -> Vec<&mut Pt> {
        match self {
            Shape::Line { a, b }
            | Shape::Arrow { a, b }
            | Shape::Rect { a, b, .. }
            | Shape::Ellipse { a, b, .. }
            | Shape::Blur { a, b, .. }
            | Shape::Chart { a, b, .. }
            | Shape::Spotlight { a, b }
            | Shape::Pixelate { a, b, .. } => vec![a, b],
            Shape::Text { at, .. } | Shape::Step { at, .. } => vec![at],
            Shape::Pencil { pts } | Shape::Marker { pts } => pts.iter_mut().collect(),
        }
    }

    pub fn translate(&mut self, dx: f32, dy: f32) {
        for p in self.points_mut() {
            p.x += dx;
            p.y += dy;
        }
    }

    fn map_points(&mut self, f: impl Fn(Pt) -> Pt) {
        for p in self.points_mut() {
            *p = f(*p);
        }
    }

    /// Axis-aligned bounds `(min, max)` in image pixels, including the stroke.
    pub fn bounds(&self, width: f32) -> (Pt, Pt) {
        let pad = width / 2.0;
        let (lo, hi) = match self {
            Shape::Line { a, b } | Shape::Arrow { a, b } => norm_rect(*a, *b),
            Shape::Rect { a, b, .. }
            | Shape::Ellipse { a, b, .. }
            | Shape::Blur { a, b, .. }
            | Shape::Chart { a, b, .. }
            | Shape::Spotlight { a, b }
            | Shape::Pixelate { a, b, .. } => norm_rect(*a, *b),
            Shape::Text { at, text, size } => {
                let (w, h) = render::measure_text(text, *size);
                (*at, Pt::new(at.x + w, at.y + h))
            }
            Shape::Step { at, .. } => {
                let r = step_radius(width);
                return (Pt::new(at.x - r, at.y - r), Pt::new(at.x + r, at.y + r));
            }
            Shape::Pencil { pts } | Shape::Marker { pts } => {
                let mut lo = Pt::new(f32::MAX, f32::MAX);
                let mut hi = Pt::new(f32::MIN, f32::MIN);
                for p in pts {
                    lo = Pt::new(lo.x.min(p.x), lo.y.min(p.y));
                    hi = Pt::new(hi.x.max(p.x), hi.y.max(p.y));
                }
                if pts.is_empty() {
                    (Pt::default(), Pt::default())
                } else {
                    (lo, hi)
                }
            }
        };
        let pad = if matches!(self, Shape::Marker { .. }) {
            width * 2.5
        } else {
            pad
        };
        (Pt::new(lo.x - pad, lo.y - pad), Pt::new(hi.x + pad, hi.y + pad))
    }

    /// Does the point `p` select this shape? `tol` is extra reach in image pixels.
    pub fn hit(&self, p: Pt, width: f32, tol: f32) -> bool {
        let reach = width / 2.0 + tol;
        match self {
            Shape::Line { a, b } | Shape::Arrow { a, b } => dist_to_segment(p, *a, *b) <= reach.max(tol + 2.0),
            Shape::Rect { a, b, fill } => {
                let (lo, hi) = norm_rect(*a, *b);
                let inside = p.x >= lo.x - reach && p.x <= hi.x + reach && p.y >= lo.y - reach && p.y <= hi.y + reach;
                if *fill {
                    return inside;
                }
                let inner = p.x > lo.x + reach && p.x < hi.x - reach && p.y > lo.y + reach && p.y < hi.y - reach;
                inside && !inner
            }
            Shape::Ellipse { a, b, fill } => {
                let (lo, hi) = norm_rect(*a, *b);
                let (cx, cy) = ((lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0);
                let (rx, ry) = (((hi.x - lo.x) / 2.0).max(1.0), ((hi.y - lo.y) / 2.0).max(1.0));
                let d = (((p.x - cx) / rx).powi(2) + ((p.y - cy) / ry).powi(2)).sqrt();
                if *fill {
                    d <= 1.0 + reach / rx.min(ry)
                } else {
                    (d - 1.0).abs() <= reach / rx.min(ry)
                }
            }
            Shape::Pencil { pts } | Shape::Marker { pts } => {
                let reach = if matches!(self, Shape::Marker { .. }) {
                    width * 2.5 + tol
                } else {
                    reach
                };
                match pts.len() {
                    0 => false,
                    1 => ((p.x - pts[0].x).powi(2) + (p.y - pts[0].y).powi(2)).sqrt() <= reach,
                    _ => pts.windows(2).any(|w| dist_to_segment(p, w[0], w[1]) <= reach),
                }
            }
            Shape::Blur { .. }
            | Shape::Pixelate { .. }
            | Shape::Spotlight { .. }
            | Shape::Chart { .. }
            | Shape::Text { .. }
            | Shape::Step { .. } => {
                let (lo, hi) = self.bounds(0.0);
                p.x >= lo.x && p.x <= hi.x && p.y >= lo.y && p.y <= hi.y || {
                    matches!(self, Shape::Step { .. }) && {
                        let (lo, hi) = self.bounds(width);
                        p.x >= lo.x && p.x <= hi.x && p.y >= lo.y && p.y <= hi.y
                    }
                }
            }
        }
    }

    /// Drag handles for resizing: line ends, rectangle corners.
    pub fn handles(&self) -> Vec<Pt> {
        match self {
            Shape::Line { a, b } | Shape::Arrow { a, b } => vec![*a, *b],
            Shape::Rect { a, b, .. }
            | Shape::Ellipse { a, b, .. }
            | Shape::Blur { a, b, .. }
            | Shape::Chart { a, b, .. }
            | Shape::Spotlight { a, b }
            | Shape::Pixelate { a, b, .. } => {
                let (lo, hi) = norm_rect(*a, *b);
                vec![lo, Pt::new(hi.x, lo.y), hi, Pt::new(lo.x, hi.y)]
            }
            _ => Vec::new(),
        }
    }

    /// Handles for reshaping the shape with the mouse: the two ends of a line or arrow, for everything else the eight
    /// points of its box (corners TL, TR, BR, BL, then the middle of the top, right, bottom and left side). A
    /// numbered marker has none (its size follows the stroke width).
    pub fn edit_handles(&self) -> Vec<Pt> {
        match self {
            Shape::Line { a, b } | Shape::Arrow { a, b } => vec![*a, *b],
            Shape::Step { .. } => Vec::new(),
            _ => {
                let (lo, hi) = self.bounds(0.0);
                let (mx, my) = ((lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0);
                vec![
                    lo,
                    Pt::new(hi.x, lo.y),
                    hi,
                    Pt::new(lo.x, hi.y),
                    Pt::new(mx, lo.y),
                    Pt::new(hi.x, my),
                    Pt::new(mx, hi.y),
                    Pt::new(lo.x, my),
                ]
            }
        }
    }

    /// The shape with its handle `i` (an index into `edit_handles`) dragged to `to`. Works from the shape as it was
    /// when the drag began, so dragging past the opposite side mirrors the shape instead of tangling the handles.
    pub fn resized(&self, i: usize, to: Pt) -> Shape {
        let mut out = self.clone();
        match &mut out {
            Shape::Line { a, b } | Shape::Arrow { a, b } => {
                if i == 0 {
                    *a = to;
                } else {
                    *b = to;
                }
                return out;
            }
            Shape::Step { .. } => return out,
            _ => {}
        }
        let (lo, hi) = self.bounds(0.0);
        let (mut nlo, mut nhi) = (lo, hi);
        match i {
            0 => nlo = to,
            1 => {
                nhi.x = to.x;
                nlo.y = to.y;
            }
            2 => nhi = to,
            3 => {
                nlo.x = to.x;
                nhi.y = to.y;
            }
            4 => nlo.y = to.y,
            5 => nhi.x = to.x,
            6 => nhi.y = to.y,
            _ => nlo.x = to.x,
        }
        let (w, h) = (hi.x - lo.x, hi.y - lo.y);
        let sx = if w > 1e-3 { (nhi.x - nlo.x) / w } else { 1.0 };
        let sy = if h > 1e-3 { (nhi.y - nlo.y) / h } else { 1.0 };
        if let Shape::Text { at, size, .. } = &mut out {
            // the text keeps its proportions: the size follows the side that was dragged, the opposite one stays put
            let s = match i {
                4 | 6 => sy.abs(),
                5 | 7 => sx.abs(),
                _ => sx.abs().max(sy.abs()),
            }
            .clamp(0.1, 20.0);
            let (nw, nh) = (w * s, h * s);
            let left = matches!(i, 0 | 3 | 7);
            let top = matches!(i, 0 | 1 | 4);
            *at = Pt::new(if left { hi.x - nw } else { lo.x }, if top { hi.y - nh } else { lo.y });
            *size = (*size * s).clamp(6.0, 400.0);
            return out;
        }
        out.map_points(|p| {
            Pt::new(
                if w > 1e-3 {
                    nlo.x + (p.x - lo.x) * sx
                } else {
                    p.x + nlo.x - lo.x
                },
                if h > 1e-3 {
                    nlo.y + (p.y - lo.y) * sy
                } else {
                    p.y + nlo.y - lo.y
                },
            )
        });
        out
    }

    /// Moves handle `i` (index into `handles()`) to `to`.
    pub fn move_handle(&mut self, i: usize, to: Pt) {
        match self {
            Shape::Line { a, b } | Shape::Arrow { a, b } => {
                if i == 0 {
                    *a = to
                } else {
                    *b = to
                }
            }
            Shape::Rect { a, b, .. }
            | Shape::Ellipse { a, b, .. }
            | Shape::Blur { a, b, .. }
            | Shape::Chart { a, b, .. }
            | Shape::Spotlight { a, b }
            | Shape::Pixelate { a, b, .. } => {
                let (lo, hi) = norm_rect(*a, *b);
                // the opposite corner stays fixed
                let anchor = match i {
                    0 => hi,
                    1 => Pt::new(lo.x, hi.y),
                    2 => lo,
                    _ => Pt::new(hi.x, lo.y),
                };
                *a = anchor;
                *b = to;
            }
            _ => {}
        }
    }
}

/// Everything the undo stack needs to restore a state. Cheap to clone: the base image is shared.
#[derive(Clone)]
pub struct Document {
    pub base: Arc<RgbaImage>,
    pub objects: Vec<Object>,
    next_id: u64,
}

impl Document {
    pub fn new(base: RgbaImage) -> Document {
        Document {
            base: Arc::new(base),
            objects: Vec::new(),
            next_id: 1,
        }
    }

    pub fn size(&self) -> (u32, u32) {
        self.base.dimensions()
    }

    pub fn add(&mut self, shape: Shape, color: Color, width: f32) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.objects.push(Object {
            id,
            shape,
            color,
            width,
            visible: true,
        });
        id
    }

    pub fn get(&self, id: u64) -> Option<&Object> {
        self.objects.iter().find(|o| o.id == id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut Object> {
        self.objects.iter_mut().find(|o| o.id == id)
    }

    pub fn remove(&mut self, id: u64) -> bool {
        let n = self.objects.len();
        self.objects.retain(|o| o.id != id);
        self.objects.len() != n
    }

    /// Topmost object under `p`.
    pub fn hit_test(&self, p: Pt, tol: f32) -> Option<u64> {
        self.objects
            .iter()
            .rev()
            .find(|o| o.visible && o.shape.hit(p, o.width, tol))
            .map(|o| o.id)
    }

    /// Number for the next numbered marker.
    pub fn next_step_number(&self) -> u32 {
        self.objects
            .iter()
            .filter_map(|o| match o.shape {
                Shape::Step { n, .. } => Some(n),
                _ => None,
            })
            .max()
            .map_or(1, |n| n + 1)
    }

    /// Rotates the image and all objects 90° clockwise. Text stays upright (only its anchor moves).
    pub fn rotate_cw(&mut self) {
        let h = self.base.height() as f32;
        self.base = Arc::new(image::imageops::rotate90(&*self.base));
        for o in &mut self.objects {
            o.shape.map_points(|p| Pt::new(h - p.y, p.x));
        }
    }

    pub fn rotate_ccw(&mut self) {
        let w = self.base.width() as f32;
        self.base = Arc::new(image::imageops::rotate270(&*self.base));
        for o in &mut self.objects {
            o.shape.map_points(|p| Pt::new(p.y, w - p.x));
        }
    }

    /// Mirrors the picture and all objects left-to-right.
    pub fn flip_horizontal(&mut self) {
        let w = self.base.width() as f32;
        self.base = Arc::new(image::imageops::flip_horizontal(&*self.base));
        for o in &mut self.objects {
            o.shape.map_points(|p| Pt::new(w - p.x, p.y));
        }
    }

    /// Mirrors the picture and all objects top-to-bottom.
    pub fn flip_vertical(&mut self) {
        let h = self.base.height() as f32;
        self.base = Arc::new(image::imageops::flip_vertical(&*self.base));
        for o in &mut self.objects {
            o.shape.map_points(|p| Pt::new(p.x, h - p.y));
        }
    }

    /// Resizes the picture to `percent` % (1..=400); objects scale with it. Returns false when nothing changes.
    pub fn resize_percent(&mut self, percent: u32) -> bool {
        let percent = percent.clamp(1, 400);
        let (w, h) = self.base.dimensions();
        let (nw, nh) = ((w * percent / 100).max(1), (h * percent / 100).max(1));
        if (nw, nh) == (w, h) || nw.max(nh) > 16_000 {
            return false;
        }
        let (fx, fy) = (nw as f32 / w as f32, nh as f32 / h as f32);
        self.base = Arc::new(image::imageops::resize(
            &*self.base,
            nw,
            nh,
            image::imageops::FilterType::Lanczos3,
        ));
        let f = (fx + fy) / 2.0;
        for o in &mut self.objects {
            o.shape.map_points(|p| Pt::new(p.x * fx, p.y * fy));
            o.shape.scale_params(f);
            o.width *= f;
        }
        true
    }

    /// Puts the picture on a coloured background with rounded corners and a shadow; objects keep their place on it.
    pub fn beautify(&mut self, o: &render::Beautify) {
        let pad = o.padding.min(400) as f32;
        self.base = Arc::new(render::beautify(&self.base, o));
        for ob in &mut self.objects {
            ob.shape.translate(pad, pad);
        }
    }

    /// Crops to `r` (clamped to the image); objects keep their place relative to the pixels. Returns false if
    /// the rectangle is empty or covers the whole image.
    pub fn crop(&mut self, r: Rect) -> bool {
        let full = Rect::new(0, 0, self.base.width() as i32, self.base.height() as i32);
        let Some(r) = r.intersect(&full) else { return false };
        if r == full {
            return false;
        }
        self.base =
            Arc::new(image::imageops::crop_imm(&*self.base, r.x as u32, r.y as u32, r.w as u32, r.h as u32).to_image());
        for o in &mut self.objects {
            o.shape.translate(-(r.x as f32), -(r.y as f32));
        }
        true
    }

    /// Moves object `id` one step up (towards the front) or down. Returns false at the end of the list.
    pub fn move_layer(&mut self, id: u64, up: bool) -> bool {
        let Some(i) = self.objects.iter().position(|o| o.id == id) else {
            return false;
        };
        let j = if up { i + 1 } else { i.wrapping_sub(1) };
        if j >= self.objects.len() {
            return false;
        }
        self.objects.swap(i, j);
        true
    }

    /// Flattens the document: base + all objects.
    pub fn render(&self) -> RgbaImage {
        render::render(&self.base, &self.objects)
    }
}

/// Document with undo/redo.
pub struct Editor {
    pub doc: Document,
    undo: Vec<Document>,
    redo: Vec<Document>,
}

const MAX_UNDO: usize = 100;

impl Editor {
    pub fn new(base: RgbaImage) -> Editor {
        Editor {
            doc: Document::new(base),
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// Call before every user-visible change (start of a drag, a delete, a rotation…).
    pub fn checkpoint(&mut self) {
        self.undo.push(self.doc.clone());
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Forgets the last checkpoint (the gesture it was taken for produced no change).
    pub fn cancel_checkpoint(&mut self) {
        self.undo.pop();
    }

    /// Number of steps `undo` can go back.
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(prev) => {
                self.redo.push(std::mem::replace(&mut self.doc, prev));
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(next) => {
                self.undo.push(std::mem::replace(&mut self.doc, next));
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn doc(w: u32, h: u32) -> Document {
        Document::new(RgbaImage::from_pixel(w, h, Rgba([255, 255, 255, 255])))
    }

    fn arrow_doc() -> Document {
        let mut d = Document::new(RgbaImage::from_fn(40, 20, |x, _| {
            if x < 20 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([0, 0, 255, 255])
            }
        }));
        d.add(
            Shape::Line {
                a: Pt::new(10.0, 5.0),
                b: Pt::new(30.0, 15.0),
            },
            DEFAULT_COLOR,
            4.0,
        );
        d
    }

    #[test]
    fn flipping_mirrors_the_picture_and_the_objects() {
        let mut d = arrow_doc();
        d.flip_horizontal();
        assert_eq!(
            d.base.get_pixel(0, 0).0,
            [0, 0, 255, 255],
            "blue half is now on the left"
        );
        let Shape::Line { a, b } = &d.objects[0].shape else {
            panic!()
        };
        assert_eq!((a.x, a.y, b.x, b.y), (30.0, 5.0, 10.0, 15.0));
        d.flip_vertical();
        let Shape::Line { a, b } = &d.objects[0].shape else {
            panic!()
        };
        assert_eq!((a.y, b.y), (15.0, 5.0));
    }

    #[test]
    fn resizing_scales_the_picture_and_the_objects() {
        let mut d = arrow_doc();
        d.add(
            Shape::Blur {
                a: Pt::new(0.0, 0.0),
                b: Pt::new(10.0, 10.0),
                radius: 10,
            },
            DEFAULT_COLOR,
            1.0,
        );
        assert!(d.resize_percent(50));
        assert_eq!(d.size(), (20, 10));
        let Shape::Line { b, .. } = &d.objects[0].shape else {
            panic!()
        };
        assert_eq!((b.x, b.y), (15.0, 7.5));
        assert_eq!(d.objects[0].width, 2.0);
        let Shape::Blur { radius, .. } = &d.objects[1].shape else {
            panic!()
        };
        assert_eq!(*radius, 5);
        assert!(!d.resize_percent(100), "100 % changes nothing");
    }

    #[test]
    fn beautify_adds_padding_rounds_corners_and_moves_the_objects() {
        let mut d = arrow_doc();
        let o = render::Beautify {
            padding: 20,
            radius: 8,
            shadow: true,
            ..render::Beautify::default()
        };
        d.beautify(&o);
        assert_eq!(d.size(), (80, 60));
        let Shape::Line { a, .. } = &d.objects[0].shape else {
            panic!()
        };
        assert_eq!((a.x, a.y), (30.0, 25.0));
        assert_eq!(
            d.base.get_pixel(30, 30).0,
            [255, 0, 0, 255],
            "the picture sits in the middle, untouched"
        );
        let corner = d.base.get_pixel(20, 20).0;
        assert_ne!(
            corner,
            [255, 0, 0, 255],
            "the corner of the picture is rounded off: {corner:?}"
        );
        assert_eq!(d.base.get_pixel(0, 0)[3], 255, "the background is opaque");
    }

    #[test]
    fn a_spotlight_dims_everything_but_its_rectangle() {
        let mut d = doc(40, 40);
        d.add(
            Shape::Spotlight {
                a: Pt::new(10.0, 10.0),
                b: Pt::new(30.0, 30.0),
            },
            DEFAULT_COLOR,
            1.0,
        );
        let out = d.render();
        assert_eq!(out.get_pixel(20, 20).0, [255, 255, 255, 255], "inside stays bright");
        assert!(
            out.get_pixel(2, 2)[0] < 150,
            "outside is darker: {:?}",
            out.get_pixel(2, 2)
        );
        assert!(d.objects[0].shape.hit(Pt::new(20.0, 20.0), 1.0, 2.0));
    }

    #[test]
    fn ids_are_unique_and_removal_works() {
        let mut d = doc(10, 10);
        let a = d.add(
            Shape::Line {
                a: Pt::new(0.0, 0.0),
                b: Pt::new(5.0, 5.0),
            },
            DEFAULT_COLOR,
            2.0,
        );
        let b = d.add(
            Shape::Line {
                a: Pt::new(0.0, 0.0),
                b: Pt::new(5.0, 5.0),
            },
            DEFAULT_COLOR,
            2.0,
        );
        assert_ne!(a, b);
        assert!(d.remove(a) && !d.remove(a));
        assert_eq!(d.objects.len(), 1);
    }

    #[test]
    fn hit_test_returns_topmost() {
        let mut d = doc(100, 100);
        let r = |a, b| Shape::Rect {
            a: Pt::new(a, a),
            b: Pt::new(b, b),
            fill: true,
        };
        let _low = d.add(r(0.0, 50.0), DEFAULT_COLOR, 2.0);
        let top = d.add(r(20.0, 70.0), DEFAULT_COLOR, 2.0);
        assert_eq!(d.hit_test(Pt::new(30.0, 30.0), 2.0), Some(top));
        assert!(d.hit_test(Pt::new(5.0, 5.0), 2.0).is_some());
        assert_eq!(d.hit_test(Pt::new(95.0, 95.0), 2.0), None);
    }

    #[test]
    fn hollow_rect_hits_only_the_border() {
        let s = Shape::Rect {
            a: Pt::new(10.0, 10.0),
            b: Pt::new(90.0, 90.0),
            fill: false,
        };
        assert!(s.hit(Pt::new(10.0, 50.0), 4.0, 2.0));
        assert!(!s.hit(Pt::new(50.0, 50.0), 4.0, 2.0));
    }

    #[test]
    fn line_hit_uses_distance() {
        let s = Shape::Line {
            a: Pt::new(0.0, 0.0),
            b: Pt::new(100.0, 0.0),
        };
        assert!(s.hit(Pt::new(50.0, 3.0), 4.0, 2.0));
        assert!(!s.hit(Pt::new(50.0, 20.0), 4.0, 2.0));
    }

    #[test]
    fn rotate_cw_moves_objects_with_pixels() {
        let mut d = doc(40, 20);
        // mark the top-left pixel and an object on it
        let mut img = (*d.base).clone();
        img.put_pixel(0, 0, Rgba([1, 2, 3, 255]));
        d.base = Arc::new(img);
        d.add(
            Shape::Step {
                at: Pt::new(0.0, 0.0),
                n: 1,
            },
            DEFAULT_COLOR,
            4.0,
        );
        d.rotate_cw();
        assert_eq!(d.size(), (20, 40));
        // top-left goes to top-right after a clockwise turn
        assert_eq!(d.base.get_pixel(19, 0).0, [1, 2, 3, 255]);
        match d.objects[0].shape {
            Shape::Step { at, .. } => assert_eq!((at.x, at.y), (20.0, 0.0)),
            _ => unreachable!(),
        }
    }

    #[test]
    fn rotate_ccw_inverts_cw() {
        let mut d = doc(40, 20);
        d.add(
            Shape::Step {
                at: Pt::new(7.0, 3.0),
                n: 1,
            },
            DEFAULT_COLOR,
            4.0,
        );
        d.rotate_cw();
        d.rotate_ccw();
        assert_eq!(d.size(), (40, 20));
        match d.objects[0].shape {
            Shape::Step { at, .. } => assert_eq!((at.x, at.y), (7.0, 3.0)),
            _ => unreachable!(),
        }
    }

    #[test]
    fn crop_shifts_objects_and_clamps() {
        let mut d = doc(100, 100);
        d.add(
            Shape::Step {
                at: Pt::new(60.0, 70.0),
                n: 1,
            },
            DEFAULT_COLOR,
            4.0,
        );
        assert!(d.crop(Rect::new(50, 50, 200, 200)));
        assert_eq!(d.size(), (50, 50));
        match d.objects[0].shape {
            Shape::Step { at, .. } => assert_eq!((at.x, at.y), (10.0, 20.0)),
            _ => unreachable!(),
        }
        assert!(!d.crop(Rect::new(500, 500, 10, 10)), "outside the image");
        assert!(!d.crop(Rect::new(0, 0, 50, 50)), "whole image is a no-op");
    }

    #[test]
    fn layers_reorder_and_hide() {
        let mut d = doc(20, 20);
        let a = d.add(
            Shape::Rect {
                a: Pt::new(0.0, 0.0),
                b: Pt::new(10.0, 10.0),
                fill: true,
            },
            [255, 0, 0, 255],
            1.0,
        );
        let b = d.add(
            Shape::Rect {
                a: Pt::new(0.0, 0.0),
                b: Pt::new(10.0, 10.0),
                fill: true,
            },
            [0, 0, 255, 255],
            1.0,
        );
        assert_eq!(d.render().get_pixel(5, 5).0, [0, 0, 255, 255], "later object is on top");
        assert!(d.move_layer(a, true));
        assert_eq!(d.render().get_pixel(5, 5).0, [255, 0, 0, 255], "moved to the front");
        assert!(!d.move_layer(a, true), "already on top");
        d.get_mut(a).unwrap().visible = false;
        assert_eq!(
            d.render().get_pixel(5, 5).0,
            [0, 0, 255, 255],
            "hidden objects are not drawn"
        );
        assert_eq!(d.hit_test(Pt::new(5.0, 5.0), 1.0), Some(b), "and cannot be picked");
    }

    #[test]
    fn step_numbers_increment() {
        let mut d = doc(10, 10);
        assert_eq!(d.next_step_number(), 1);
        d.add(
            Shape::Step {
                at: Pt::default(),
                n: 1,
            },
            DEFAULT_COLOR,
            4.0,
        );
        d.add(
            Shape::Step {
                at: Pt::default(),
                n: 5,
            },
            DEFAULT_COLOR,
            4.0,
        );
        assert_eq!(d.next_step_number(), 6);
    }

    #[test]
    fn undo_redo_restores_objects_and_base() {
        let mut e = Editor::new(RgbaImage::from_pixel(30, 20, Rgba([0, 0, 0, 255])));
        e.checkpoint();
        e.doc.add(
            Shape::Step {
                at: Pt::default(),
                n: 1,
            },
            DEFAULT_COLOR,
            4.0,
        );
        e.checkpoint();
        e.doc.rotate_cw();
        assert_eq!(e.doc.size(), (20, 30));
        assert!(e.undo());
        assert_eq!(e.doc.size(), (30, 20));
        assert_eq!(e.doc.objects.len(), 1);
        assert!(e.undo());
        assert!(e.doc.objects.is_empty());
        assert!(!e.undo());
        assert!(e.redo() && e.redo() && !e.redo());
        assert_eq!(e.doc.size(), (20, 30));
        // a new edit clears the redo stack
        assert!(e.undo());
        e.checkpoint();
        assert!(!e.can_redo());
    }

    #[test]
    fn handles_resize_keeping_opposite_corner() {
        let mut s = Shape::Rect {
            a: Pt::new(10.0, 10.0),
            b: Pt::new(50.0, 40.0),
            fill: false,
        };
        assert_eq!(s.handles().len(), 4);
        s.move_handle(2, Pt::new(80.0, 90.0)); // bottom-right
        let (lo, hi) = match &s {
            Shape::Rect { a, b, .. } => norm_rect(*a, *b),
            _ => unreachable!(),
        };
        assert_eq!((lo.x, lo.y, hi.x, hi.y), (10.0, 10.0, 80.0, 90.0));
    }

    #[test]
    fn every_shape_but_the_step_can_be_reshaped_from_its_handles() {
        let rect = Shape::Rect {
            a: Pt::new(10.0, 10.0),
            b: Pt::new(50.0, 30.0),
            fill: false,
        };
        assert_eq!(rect.edit_handles().len(), 8);
        // the right-hand middle handle stretches only the width
        let Shape::Rect { a, b, .. } = rect.resized(5, Pt::new(90.0, 99.0)) else {
            panic!()
        };
        assert_eq!((a.x, a.y, b.x, b.y), (10.0, 10.0, 90.0, 30.0));
        // dragging a corner past the opposite one mirrors the box
        let Shape::Rect { a, b, .. } = rect.resized(0, Pt::new(70.0, 50.0)) else {
            panic!()
        };
        let (lo, hi) = norm_rect(a, b);
        assert_eq!((lo.x, lo.y, hi.x, hi.y), (50.0, 30.0, 70.0, 50.0));

        let line = Shape::Arrow {
            a: Pt::new(0.0, 0.0),
            b: Pt::new(5.0, 5.0),
        };
        assert_eq!(line.edit_handles().len(), 2);
        let Shape::Arrow { a, b } = line.resized(1, Pt::new(9.0, 2.0)) else {
            panic!()
        };
        assert_eq!((a.x, b.x, b.y), (0.0, 9.0, 2.0));

        let pencil = Shape::Pencil {
            pts: vec![Pt::new(0.0, 0.0), Pt::new(10.0, 10.0), Pt::new(20.0, 0.0)],
        };
        let Shape::Pencil { pts } = pencil.resized(2, Pt::new(40.0, 20.0)) else {
            panic!()
        };
        assert_eq!((pts[1].x, pts[1].y, pts[2].x), (20.0, 20.0, 40.0));

        let step = Shape::Step {
            at: Pt::new(3.0, 3.0),
            n: 1,
        };
        assert!(step.edit_handles().is_empty());
        assert_eq!(step.resized(0, Pt::new(9.0, 9.0)), step);
    }

    #[test]
    fn dragging_a_text_corner_scales_it_around_the_opposite_corner() {
        let t = Shape::Text {
            at: Pt::new(100.0, 100.0),
            text: "Hello".into(),
            size: 20.0,
        };
        let (lo, hi) = t.bounds(0.0);
        // bottom-right corner moved to twice the distance from the top-left one
        let to = Pt::new(lo.x + (hi.x - lo.x) * 2.0, lo.y + (hi.y - lo.y) * 2.0);
        let Shape::Text { at, size, .. } = t.resized(2, to) else {
            panic!()
        };
        assert_eq!((at.x, at.y), (100.0, 100.0));
        assert!((size - 40.0).abs() < 0.01, "{size}");
        // from the top-left corner the bottom-right one stays where it was
        let Shape::Text { at, size, text } = t.resized(0, Pt::new(lo.x - (hi.x - lo.x), lo.y - (hi.y - lo.y))) else {
            panic!()
        };
        assert!((size - 40.0).abs() < 0.01);
        let grown = Shape::Text { at, size, text };
        let (_, hi2) = grown.bounds(0.0);
        assert!(
            (hi2.x - hi.x).abs() < 1.5 && (hi2.y - hi.y).abs() < 1.5,
            "{hi2:?} vs {hi:?}"
        );
    }

    #[test]
    fn serde_roundtrip_keeps_objects_vector() {
        let mut d = doc(10, 10);
        d.add(
            Shape::Text {
                at: Pt::new(1.0, 2.0),
                text: "hi".into(),
                size: 20.0,
            },
            DEFAULT_COLOR,
            2.0,
        );
        let json = serde_json::to_string(&d.objects).unwrap();
        let back: Vec<Object> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d.objects);
    }
}
