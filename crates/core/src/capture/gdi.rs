//! GDI fallback: `BitBlt` from the screen DC. Works everywhere, but cannot see some protected/accelerated content.

use image::RgbaImage;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap, CreateCompatibleDC,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CURSOR_SHOWING, CURSORINFO, DI_NORMAL, DrawIconEx, GetCursorInfo, GetIconInfo, ICONINFO,
};

use super::CaptureError;
use crate::geometry::Rect;

fn err(what: &str) -> CaptureError {
    CaptureError::Backend(format!("GDI: {what} failed"))
}

/// Copies `rect` (virtual-desktop pixels) from the screen.
pub fn grab(rect: Rect, cursor: bool) -> Result<RgbaImage, CaptureError> {
    if rect.is_empty() {
        return Err(CaptureError::OutsideScreen);
    }
    // SAFETY: every handle created here is released before returning; buffers are sized from the bitmap header.
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return Err(err("GetDC"));
        }
        let mem = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, rect.w, rect.h);
        let old = SelectObject(mem, bmp.into());
        let blit = BitBlt(
            mem,
            0,
            0,
            rect.w,
            rect.h,
            Some(screen),
            rect.x,
            rect.y,
            SRCCOPY | CAPTUREBLT,
        );

        if cursor {
            draw_cursor(mem, rect);
        }

        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: rect.w,
                biHeight: -rect.h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buf = vec![0u8; rect.w as usize * rect.h as usize * 4];
        let lines = GetDIBits(
            mem,
            bmp,
            0,
            rect.h as u32,
            Some(buf.as_mut_ptr().cast()),
            &mut bi,
            DIB_RGB_COLORS,
        );

        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);

        if blit.is_err() {
            return Err(err("BitBlt"));
        }
        if lines == 0 {
            return Err(err("GetDIBits"));
        }
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }
        RgbaImage::from_raw(rect.w as u32, rect.h as u32, buf).ok_or_else(|| err("buffer size"))
    }
}

/// Draws the current mouse cursor into `dc`, which holds the pixels of `rect`.
///
/// # Safety
/// `dc` must be a valid memory DC.
unsafe fn draw_cursor(dc: windows::Win32::Graphics::Gdi::HDC, rect: Rect) {
    // SAFETY: caller guarantees `dc`; the icon bitmaps from GetIconInfo are released here.
    unsafe {
        let mut ci = CURSORINFO {
            cbSize: std::mem::size_of::<CURSORINFO>() as u32,
            ..Default::default()
        };
        if GetCursorInfo(&mut ci).is_err() || ci.flags.0 & CURSOR_SHOWING.0 == 0 {
            return;
        }
        let mut ii = ICONINFO::default();
        if GetIconInfo(ci.hCursor.into(), &mut ii).is_err() {
            return;
        }
        let x = ci.ptScreenPos.x - ii.xHotspot as i32 - rect.x;
        let y = ci.ptScreenPos.y - ii.yHotspot as i32 - rect.y;
        let _ = DrawIconEx(dc, x, y, ci.hCursor.into(), 0, 0, 0, None, DI_NORMAL);
        if !ii.hbmMask.is_invalid() {
            let _ = DeleteObject(ii.hbmMask.into());
        }
        if !ii.hbmColor.is_invalid() {
            let _ = DeleteObject(ii.hbmColor.into());
        }
    }
}
