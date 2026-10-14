//! Screen geometry: rectangles, monitors and the conversions between logical (DPI-scaled) and physical
//! (pixel) coordinates of the virtual desktop. Pure logic, no WinAPI.

use serde::{Deserialize, Serialize};

/// Axis-aligned integer rectangle. `w` and `h` are never negative after `normalized()`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    /// Rectangle spanned by two corner points in any order.
    pub fn from_points(ax: i32, ay: i32, bx: i32, by: i32) -> Rect {
        Rect::new(ax.min(bx), ay.min(by), (ax - bx).abs(), (ay - by).abs())
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    pub fn area(&self) -> i64 {
        if self.is_empty() {
            0
        } else {
            self.w as i64 * self.h as i64
        }
    }

    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && py >= self.y && px < self.right() && py < self.bottom()
    }

    pub fn intersect(&self, o: &Rect) -> Option<Rect> {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        (r > x && b > y).then(|| Rect::new(x, y, r - x, b - y))
    }

    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect::new(x, y, self.right().max(o.right()) - x, self.bottom().max(o.bottom()) - y)
    }

    pub fn translate(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }
}

/// One display. `rect` is in physical pixels of the virtual desktop (origin = top-left of the primary monitor,
/// monitors left of / above it have negative coordinates). `dpi` is the effective DPI (96 = 100 %).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Monitor {
    pub index: usize,
    pub name: String,
    pub rect: Rect,
    pub dpi: u32,
    pub primary: bool,
    /// Native monitor handle (HMONITOR) for the capture backends; 0 when unknown.
    #[serde(skip)]
    pub handle: isize,
}

impl Monitor {
    pub fn scale(&self) -> f64 {
        self.dpi.max(1) as f64 / 96.0
    }

    /// Size of the monitor in logical (DPI-independent) units.
    pub fn logical_size(&self) -> (f64, f64) {
        (self.rect.w as f64 / self.scale(), self.rect.h as f64 / self.scale())
    }

    /// Monitor-local logical point -> physical point of the virtual desktop.
    pub fn logical_to_virtual(&self, lx: f64, ly: f64) -> (i32, i32) {
        let s = self.scale();
        (
            self.rect.x + (lx * s).round() as i32,
            self.rect.y + (ly * s).round() as i32,
        )
    }

    /// Physical point of the virtual desktop -> monitor-local logical point.
    pub fn virtual_to_logical(&self, px: i32, py: i32) -> (f64, f64) {
        let s = self.scale();
        ((px - self.rect.x) as f64 / s, (py - self.rect.y) as f64 / s)
    }
}

/// Bounding box of all monitors (the virtual desktop).
pub fn virtual_bounds(monitors: &[Monitor]) -> Rect {
    monitors.iter().fold(Rect::default(), |acc, m| acc.union(&m.rect))
}

pub fn monitor_at(monitors: &[Monitor], x: i32, y: i32) -> Option<&Monitor> {
    monitors.iter().find(|m| m.rect.contains(x, y))
}

/// The monitor that covers most of `r`; ties go to the first one.
pub fn monitor_for_rect<'a>(monitors: &'a [Monitor], r: &Rect) -> Option<&'a Monitor> {
    let mut best: Option<(&Monitor, i64)> = None;
    for m in monitors {
        if let Some(i) = m.rect.intersect(r) {
            let a = i.area();
            if best.is_none_or(|b| a > b.1) {
                best = Some((m, a));
            }
        }
    }
    best.map(|(m, _)| m)
}

/// Pieces of `r` per monitor it overlaps (physical, virtual-desktop coordinates).
pub fn split_by_monitors<'a>(monitors: &'a [Monitor], r: &Rect) -> Vec<(&'a Monitor, Rect)> {
    monitors
        .iter()
        .filter_map(|m| m.rect.intersect(r).map(|i| (m, i)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mon(index: usize, x: i32, y: i32, w: i32, h: i32, dpi: u32, primary: bool) -> Monitor {
        Monitor {
            index,
            name: format!("M{index}"),
            rect: Rect::new(x, y, w, h),
            dpi,
            primary,
            handle: 0,
        }
    }

    /// Primary 4K at 150 %, a 1080p at 100 % to its left (negative X) and a portrait 1440p at 125 % above-right.
    fn setup() -> Vec<Monitor> {
        vec![
            mon(0, 0, 0, 3840, 2160, 144, true),
            mon(1, -1920, 400, 1920, 1080, 96, false),
            mon(2, 3840, -600, 1440, 2560, 120, false),
        ]
    }

    #[test]
    fn rect_from_points_any_order() {
        assert_eq!(Rect::from_points(10, 20, 5, 30), Rect::new(5, 20, 5, 10));
        assert_eq!(Rect::from_points(5, 30, 10, 20), Rect::new(5, 20, 5, 10));
    }

    #[test]
    fn intersect_union_basic() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 5, 10, 10);
        assert_eq!(a.intersect(&b), Some(Rect::new(5, 5, 5, 5)));
        assert_eq!(a.union(&b), Rect::new(0, 0, 15, 15));
        assert_eq!(a.intersect(&Rect::new(10, 0, 5, 5)), None);
        assert_eq!(Rect::default().union(&b), b);
    }

    #[test]
    fn virtual_bounds_cover_negative_origin() {
        assert_eq!(virtual_bounds(&setup()), Rect::new(-1920, -600, 7200, 2760));
    }

    #[test]
    fn monitor_lookup() {
        let m = setup();
        assert_eq!(monitor_at(&m, -1, 500).unwrap().index, 1);
        assert_eq!(monitor_at(&m, 0, 0).unwrap().index, 0);
        assert_eq!(monitor_at(&m, 3840, -600).unwrap().index, 2);
        assert!(monitor_at(&m, -1920, 0).is_none(), "gap above the left monitor");
    }

    #[test]
    fn dominant_monitor_by_overlap() {
        let m = setup();
        // 100 px on the left monitor, 300 px on the primary one
        let r = Rect::new(-100, 600, 400, 200);
        assert_eq!(monitor_for_rect(&m, &r).unwrap().index, 0);
        assert!(monitor_for_rect(&m, &Rect::new(10_000, 0, 5, 5)).is_none());
    }

    #[test]
    fn region_spanning_two_monitors_is_split() {
        let m = setup();
        let r = Rect::new(-100, 600, 400, 200);
        let parts = split_by_monitors(&m, &r);
        assert_eq!(parts.len(), 2);
        let total: i64 = parts.iter().map(|(_, p)| p.area()).sum();
        assert_eq!(total, r.area());
        assert_eq!(parts[0].1, Rect::new(0, 600, 300, 200));
        assert_eq!(parts[1].1, Rect::new(-100, 600, 100, 200));
    }

    #[test]
    fn logical_physical_roundtrip_per_dpi() {
        for mo in setup() {
            for (lx, ly) in [(0.0, 0.0), (100.0, 50.0), (333.0, 777.0)] {
                let (px, py) = mo.logical_to_virtual(lx, ly);
                let (bx, by) = mo.virtual_to_logical(px, py);
                assert!((bx - lx).abs() <= 0.5 / mo.scale() + 1e-9, "x on {}", mo.name);
                assert!((by - ly).abs() <= 0.5 / mo.scale() + 1e-9, "y on {}", mo.name);
            }
        }
    }

    #[test]
    fn logical_size_uses_scale() {
        let m = setup();
        assert_eq!(m[0].logical_size(), (2560.0, 1440.0));
        assert_eq!(m[1].logical_size(), (1920.0, 1080.0));
        assert_eq!(m[2].logical_size(), (1152.0, 2048.0));
    }

    #[test]
    fn logical_to_virtual_applies_monitor_origin() {
        let m = setup();
        // 150 %: logical (100, 100) is physical (150, 150) from the monitor origin
        assert_eq!(m[0].logical_to_virtual(100.0, 100.0), (150, 150));
        // left monitor has a negative origin
        assert_eq!(m[1].logical_to_virtual(10.0, 10.0), (-1910, 410));
        // 125 % on the right monitor with a negative Y origin
        assert_eq!(m[2].logical_to_virtual(80.0, 80.0), (3840 + 100, -600 + 100));
    }
}
