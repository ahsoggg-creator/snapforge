//! Screen capture. Everything is expressed in physical pixels of the virtual desktop; a region that spans
//! several monitors (with different DPI) is captured per monitor and stitched together.
//!
//! Backends: Windows.Graphics.Capture first, GDI as the fallback when WGC is unavailable or fails.

use image::{Rgba, RgbaImage, imageops};

use crate::geometry::{Monitor, Rect, split_by_monitors, virtual_bounds};

#[cfg(windows)]
pub mod gdi;
#[cfg(windows)]
pub mod sys;
#[cfg(windows)]
pub mod wgc;

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("the area is outside of every monitor")]
    OutsideScreen,
    #[error("{0}")]
    Backend(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Wgc,
    Gdi,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CaptureOptions {
    pub cursor: bool,
    /// Skip Windows.Graphics.Capture and use GDI only (used by tests and as a user override).
    pub force_gdi: bool,
}

pub struct Capture {
    pub image: RgbaImage,
    /// Backend that produced (most of) the pixels.
    pub backend: Backend,
}

/// Composes `parts` (virtual-desktop rect + pixels of that rect) into one image covering `target`.
/// Gaps between monitors stay black.
pub fn stitch(parts: &[(Rect, RgbaImage)], target: Rect) -> RgbaImage {
    let mut out = RgbaImage::from_pixel(target.w.max(1) as u32, target.h.max(1) as u32, Rgba([0, 0, 0, 255]));
    for (rect, img) in parts {
        imageops::replace(&mut out, img, (rect.x - target.x) as i64, (rect.y - target.y) as i64);
    }
    out
}

/// Crops `img` (which holds the pixels of `frame_rect`) to `piece`, both in virtual-desktop coordinates.
pub fn crop_piece(img: &RgbaImage, frame_rect: Rect, piece: Rect) -> RgbaImage {
    let x = (piece.x - frame_rect.x).max(0) as u32;
    let y = (piece.y - frame_rect.y).max(0) as u32;
    let w = (piece.w as u32).min(img.width().saturating_sub(x));
    let h = (piece.h as u32).min(img.height().saturating_sub(y));
    imageops::crop_imm(img, x, y, w, h).to_image()
}

#[cfg(windows)]
pub fn capture_rect(rect: Rect, opts: CaptureOptions) -> Result<Capture, CaptureError> {
    capture_rect_on(&sys::monitors(), rect, opts)
}

#[cfg(windows)]
pub fn capture_rect_on(monitors: &[Monitor], rect: Rect, opts: CaptureOptions) -> Result<Capture, CaptureError> {
    let rect = rect
        .intersect(&virtual_bounds(monitors))
        .ok_or(CaptureError::OutsideScreen)?;
    let pieces = split_by_monitors(monitors, &rect);
    if pieces.is_empty() {
        return Err(CaptureError::OutsideScreen);
    }
    let wgc = if opts.force_gdi { None } else { wgc::Wgc::new().ok() };
    let mut used = Backend::Gdi;
    let mut parts = Vec::new();
    for (mon, piece) in pieces {
        let via_wgc = wgc
            .as_ref()
            .and_then(|w| match w.grab_monitor(mon.handle, opts.cursor) {
                Ok(frame) => Some(crop_piece(&frame, mon.rect, piece)),
                Err(e) => {
                    crate::sf_warn!("WGC failed on {} ({e}); falling back to GDI", mon.name);
                    None
                }
            });
        let img = match via_wgc {
            Some(i) => {
                used = Backend::Wgc;
                i
            }
            None => gdi::grab(piece, opts.cursor)?,
        };
        parts.push((piece, img));
    }
    let image = if parts.len() == 1 && parts[0].0 == rect {
        parts.pop().map(|p| p.1).unwrap_or_default()
    } else {
        stitch(&parts, rect)
    };
    Ok(Capture { image, backend: used })
}

#[cfg(windows)]
pub fn capture_monitor(index: usize, opts: CaptureOptions) -> Result<Capture, CaptureError> {
    let monitors = sys::monitors();
    let m = monitors.get(index).ok_or(CaptureError::OutsideScreen)?;
    capture_rect_on(&monitors, m.rect, opts)
}

#[cfg(windows)]
pub fn capture_virtual_desktop(opts: CaptureOptions) -> Result<Capture, CaptureError> {
    let monitors = sys::monitors();
    capture_rect_on(&monitors, virtual_bounds(&monitors), opts)
}

/// What to capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// Any rectangle of the virtual desktop (a region or a window frame).
    Rect(Rect),
    /// One monitor by index (see `sys::monitors`).
    Monitor(usize),
    /// All monitors.
    Screen,
}

/// Waits `delay_ms`, then captures `target`.
#[cfg(windows)]
pub fn take(target: Target, delay_ms: u32, opts: CaptureOptions) -> Result<Capture, CaptureError> {
    if delay_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms as u64));
    }
    match target {
        Target::Rect(r) => capture_rect(r, opts),
        Target::Monitor(i) => capture_monitor(i, opts),
        Target::Screen => capture_virtual_desktop(opts),
    }
}

/// The monitor under the cursor (falls back to the primary one).
#[cfg(windows)]
pub fn monitor_under_cursor(monitors: &[Monitor]) -> Option<&Monitor> {
    let (x, y) = sys::cursor_pos()?;
    crate::geometry::monitor_at(monitors, x, y).or_else(|| monitors.iter().find(|m| m.primary))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, c: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba(c))
    }

    #[test]
    fn stitch_places_parts_by_virtual_coordinates() {
        // a region spanning a monitor at negative X and the primary one
        let left = (Rect::new(-2, 0, 2, 2), solid(2, 2, [255, 0, 0, 255]));
        let right = (Rect::new(0, 0, 3, 2), solid(3, 2, [0, 255, 0, 255]));
        let out = stitch(&[left, right], Rect::new(-2, 0, 5, 2));
        assert_eq!(out.dimensions(), (5, 2));
        assert_eq!(out.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(out.get_pixel(1, 1).0, [255, 0, 0, 255]);
        assert_eq!(out.get_pixel(2, 0).0, [0, 255, 0, 255]);
        assert_eq!(out.get_pixel(4, 1).0, [0, 255, 0, 255]);
    }

    #[test]
    fn stitch_leaves_gaps_black() {
        let a = (Rect::new(0, 0, 1, 1), solid(1, 1, [9, 9, 9, 255]));
        let b = (Rect::new(3, 0, 1, 1), solid(1, 1, [9, 9, 9, 255]));
        let out = stitch(&[a, b], Rect::new(0, 0, 4, 1));
        assert_eq!(out.get_pixel(1, 0).0, [0, 0, 0, 255]);
        assert_eq!(out.get_pixel(2, 0).0, [0, 0, 0, 255]);
    }

    #[test]
    fn crop_piece_is_relative_to_the_frame() {
        let mut frame = solid(10, 10, [0, 0, 0, 255]);
        frame.put_pixel(4, 5, Rgba([1, 2, 3, 255]));
        // frame holds the monitor at virtual (-10, 100)
        let piece = crop_piece(&frame, Rect::new(-10, 100, 10, 10), Rect::new(-6, 105, 3, 2));
        assert_eq!(piece.dimensions(), (3, 2));
        assert_eq!(piece.get_pixel(0, 0).0, [1, 2, 3, 255]);
    }

    #[test]
    fn crop_piece_never_exceeds_frame() {
        let frame = solid(4, 4, [0, 0, 0, 255]);
        let piece = crop_piece(&frame, Rect::new(0, 0, 4, 4), Rect::new(2, 2, 10, 10));
        assert_eq!(piece.dimensions(), (2, 2));
    }

    #[cfg(windows)]
    #[test]
    fn real_capture_of_primary_monitor_gdi_and_wgc() {
        sys::enable_dpi_awareness();
        let monitors = sys::monitors();
        let m = &monitors[0];
        let gdi = capture_rect(
            m.rect,
            CaptureOptions {
                cursor: false,
                force_gdi: true,
            },
        )
        .unwrap();
        assert_eq!(gdi.backend, Backend::Gdi);
        assert_eq!(gdi.image.dimensions(), (m.rect.w as u32, m.rect.h as u32));
        let auto = capture_rect(m.rect, CaptureOptions::default()).expect("capture must succeed through some backend");
        eprintln!(
            "backend used: {:?}, {:?} px, monitors: {}",
            auto.backend,
            auto.image.dimensions(),
            monitors.len()
        );
        assert_eq!(auto.image.dimensions(), gdi.image.dimensions());
        // a live desktop changes between grabs, so only check that real pixels came back
        assert!(
            auto.image.pixels().any(|p| p.0[..3] != [0, 0, 0]),
            "capture is not all black"
        );
        assert!(
            gdi.image.pixels().any(|p| p.0[..3] != [0, 0, 0]),
            "GDI capture is not all black"
        );
    }
}
