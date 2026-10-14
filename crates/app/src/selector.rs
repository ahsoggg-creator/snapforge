//! Full-screen overlay for picking a region or a window.
//!
//! A plain Win32 window over the whole virtual desktop showing a *frozen* screenshot, dimmed, with the selection
//! cut out in full brightness. Because the process is per-monitor DPI aware, window and cursor coordinates are
//! physical pixels, so the selection is pixel-exact on every monitor. A magnifier next to the cursor shows the
//! pixels under it; the arrow keys move the cursor by one pixel (Shift: ten).

use std::ffi::c_void;

use snapforge_core::RgbaImage;
use snapforge_core::capture::sys::WindowInfo;
use snapforge_core::geometry::Rect;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_RETURN, VK_RIGHT, VK_SHIFT,
    VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Region,
    Window,
}

pub struct Request {
    /// Pixels of the whole virtual desktop.
    pub frozen: RgbaImage,
    /// Virtual-desktop position of `frozen`'s top-left pixel.
    pub origin: (i32, i32),
    pub mode: Mode,
    pub windows: Vec<WindowInfo>,
    /// One-line instruction shown at the top.
    pub hint: String,
}

const MAG_SRC: i32 = 17; // source pixels per side (odd: one centre pixel)
const MAG_ZOOM: i32 = 9;
const ACCENT: COLORREF = COLORREF(0x00FA_A560); // BGR of (96,165,250)

struct State {
    w: i32,
    h: i32,
    origin: (i32, i32),
    mode: Mode,
    windows: Vec<WindowInfo>,
    hint: Vec<u16>,
    bright_dc: HDC,
    dim_dc: HDC,
    back_dc: HDC,
    bright_bmp: HBITMAP,
    dim_bmp: HBITMAP,
    back_bmp: HBITMAP,
    bright_bits: *mut u8,
    font: HFONT,
    /// Mouse drag in client coordinates.
    drag: Option<(i32, i32)>,
    /// Keyboard-driven anchor (Enter once to start, Enter again to confirm).
    anchor: Option<(i32, i32)>,
    cur: (i32, i32),
    result: Option<Rect>,
}

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Shows the overlay and blocks until the user picks something or cancels. Coordinates returned are virtual-desktop
/// pixels.
pub fn run(req: Request) -> Option<Rect> {
    let (w, h) = (req.frozen.width() as i32, req.frozen.height() as i32);
    if w == 0 || h == 0 {
        return None;
    }
    // SAFETY: a self-contained Win32 window; every GDI object created in `State::new` is released in `State::free`.
    unsafe {
        let mut st = Box::new(State::new(&req));
        let inst = GetModuleHandleW(None).ok()?;
        let class = w!("SnapForgeSelector");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_OWNDC,
            lpfnWndProc: Some(wnd_proc),
            hInstance: inst.into(),
            hCursor: LoadCursorW(None, IDC_CROSS).unwrap_or_default(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassExW(&wc); // fails harmlessly when already registered
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class,
            w!("SnapForge selector"),
            WS_POPUP,
            req.origin.0,
            req.origin.1,
            w,
            h,
            None,
            None,
            Some(inst.into()),
            Some(&mut *st as *mut State as *const c_void),
        )
        .ok()?;
        // creating across monitors with different DPI can make Windows rescale the rectangle: set it explicitly
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            req.origin.0,
            req.origin.1,
            w,
            h,
            SWP_SHOWWINDOW,
        );
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
        if let Some((x, y)) = snapforge_core::capture::sys::cursor_pos() {
            st.cur = (x - st.origin.0, y - st.origin.1);
        }
        let _ = InvalidateRect(Some(hwnd), None, false);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let result = st.result;
        st.free();
        result.map(|r| r.translate(req.origin.0, req.origin.1))
    }
}

impl State {
    /// # Safety
    /// Creates GDI objects that must be released with `free`.
    unsafe fn new(req: &Request) -> State {
        // SAFETY: GDI object creation; sizes come from the image, the DIB pixel buffers are written within bounds.
        unsafe {
            let (w, h) = (req.frozen.width() as i32, req.frozen.height() as i32);
            let screen = GetDC(None);
            let make = |bits: &mut *mut c_void| {
                let bi = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: w,
                        biHeight: -h,
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB.0,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                CreateDIBSection(Some(screen), &bi, DIB_RGB_COLORS, bits, None, 0).unwrap_or_default()
            };
            let (mut b1, mut b2): (*mut c_void, *mut c_void) = (std::ptr::null_mut(), std::ptr::null_mut());
            let bright_bmp = make(&mut b1);
            let dim_bmp = make(&mut b2);
            let back_bmp = CreateCompatibleBitmap(screen, w, h);
            let n = (w as usize) * (h as usize) * 4;
            if !b1.is_null() && !b2.is_null() {
                let bright = std::slice::from_raw_parts_mut(b1 as *mut u8, n);
                let dim = std::slice::from_raw_parts_mut(b2 as *mut u8, n);
                for ((src, b), d) in req
                    .frozen
                    .as_raw()
                    .chunks_exact(4)
                    .zip(bright.chunks_exact_mut(4))
                    .zip(dim.chunks_exact_mut(4))
                {
                    b.copy_from_slice(&[src[2], src[1], src[0], 255]);
                    d.copy_from_slice(&[
                        (src[2] as u32 * 40 / 100) as u8,
                        (src[1] as u32 * 40 / 100) as u8,
                        (src[0] as u32 * 40 / 100) as u8,
                        255,
                    ]);
                }
            }
            let mk_dc = |bmp: HBITMAP| {
                let dc = CreateCompatibleDC(Some(screen));
                SelectObject(dc, bmp.into());
                dc
            };
            let st = State {
                w,
                h,
                origin: req.origin,
                mode: req.mode,
                windows: req.windows.clone(),
                hint: utf16(&req.hint),
                bright_dc: mk_dc(bright_bmp),
                dim_dc: mk_dc(dim_bmp),
                back_dc: mk_dc(back_bmp),
                bright_bmp,
                dim_bmp,
                back_bmp,
                bright_bits: b1 as *mut u8,
                font: CreateFontW(
                    -16,
                    0,
                    0,
                    0,
                    FW_SEMIBOLD.0 as i32,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET,
                    OUT_DEFAULT_PRECIS,
                    CLIP_DEFAULT_PRECIS,
                    CLEARTYPE_QUALITY,
                    0,
                    w!("Segoe UI"),
                ),
                drag: None,
                anchor: None,
                cur: (0, 0),
                result: None,
            };
            ReleaseDC(None, screen);
            st
        }
    }

    /// # Safety
    /// Must be called once, after the window is gone.
    unsafe fn free(&mut self) {
        // SAFETY: the objects were created in `new` and are not used afterwards.
        unsafe {
            for dc in [self.bright_dc, self.dim_dc, self.back_dc] {
                let _ = DeleteDC(dc);
            }
            for b in [self.bright_bmp, self.dim_bmp, self.back_bmp] {
                let _ = DeleteObject(b.into());
            }
            let _ = DeleteObject(self.font.into());
        }
    }

    fn pixel(&self, x: i32, y: i32) -> (u8, u8, u8) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h || self.bright_bits.is_null() {
            return (0, 0, 0);
        }
        // SAFETY: bounds checked above; the DIB holds w*h BGRA pixels.
        unsafe {
            let p = self.bright_bits.add((y as usize * self.w as usize + x as usize) * 4);
            (*p.add(2), *p.add(1), *p)
        }
    }

    fn hovered_window(&self) -> Option<Rect> {
        let (vx, vy) = (self.cur.0 + self.origin.0, self.cur.1 + self.origin.1);
        snapforge_core::capture::sys::window_at(&self.windows, vx, vy)
            .map(|w| w.rect.translate(-self.origin.0, -self.origin.1))
    }

    fn selection(&self) -> Option<Rect> {
        let start = self.drag.or(self.anchor);
        match (self.mode, start) {
            (Mode::Region, Some(s)) => {
                // the selection includes the pixel under the cursor, like a rubber band
                let (x0, y0) = (s.0.min(self.cur.0), s.1.min(self.cur.1));
                let (x1, y1) = (s.0.max(self.cur.0), s.1.max(self.cur.1));
                Some(Rect::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1))
            }
            (Mode::Window, _) => self.hovered_window(),
            _ => None,
        }
        .and_then(|r| r.intersect(&Rect::new(0, 0, self.w, self.h)))
    }

    /// # Safety
    /// `hdc` must be the window's paint DC.
    unsafe fn paint(&self, hdc: HDC) {
        // SAFETY: all DCs are owned by `self` and valid for its lifetime.
        unsafe {
            let back = self.back_dc;
            let _ = BitBlt(back, 0, 0, self.w, self.h, Some(self.dim_dc), 0, 0, SRCCOPY);
            let sel = self.selection();
            if let Some(r) = sel {
                let _ = BitBlt(back, r.x, r.y, r.w, r.h, Some(self.bright_dc), r.x, r.y, SRCCOPY);
                let pen = CreatePen(PS_SOLID, 2, ACCENT);
                let old_pen = SelectObject(back, pen.into());
                let old_brush = SelectObject(back, GetStockObject(NULL_BRUSH));
                let _ = Rectangle(back, r.x - 1, r.y - 1, r.right() + 1, r.bottom() + 1);
                SelectObject(back, old_brush);
                SelectObject(back, old_pen);
                let _ = DeleteObject(pen.into());
                self.label(back, &format!("{} × {}", r.w, r.h), r.x, (r.y - 30).max(2));
            }
            SetBkMode(back, TRANSPARENT);
            let old_font = SelectObject(back, self.font.into());
            SetTextColor(back, rgb(255, 255, 255));
            let _ = TextOutW(back, 14, 12, &self.hint);
            SelectObject(back, old_font);
            self.magnifier(back);
            let _ = BitBlt(hdc, 0, 0, self.w, self.h, Some(back), 0, 0, SRCCOPY);
        }
    }

    /// # Safety
    /// `dc` must be a valid DC.
    unsafe fn label(&self, dc: HDC, text: &str, x: i32, y: i32) {
        // SAFETY: valid DC; the brush created here is deleted before returning.
        unsafe {
            let t = utf16(text);
            let old = SelectObject(dc, self.font.into());
            let mut size = windows::Win32::Foundation::SIZE::default();
            let _ = GetTextExtentPoint32W(dc, &t, &mut size);
            let r = RECT {
                left: x,
                top: y,
                right: x + size.cx + 14,
                bottom: y + size.cy + 6,
            };
            let br = CreateSolidBrush(rgb(20, 22, 30));
            FillRect(dc, &r, br);
            let _ = DeleteObject(br.into());
            SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, rgb(255, 255, 255));
            let _ = TextOutW(dc, x + 7, y + 3, &t);
            SelectObject(dc, old);
        }
    }

    /// # Safety
    /// `dc` must be a valid DC.
    unsafe fn magnifier(&self, dc: HDC) {
        // SAFETY: valid DCs; sources are clamped by GDI, pens/brushes are released.
        unsafe {
            let side = MAG_SRC * MAG_ZOOM;
            let (cx, cy) = self.cur;
            let mut mx = cx + 26;
            let mut my = cy + 26;
            if mx + side + 4 > self.w {
                mx = cx - 26 - side;
            }
            if my + side + 70 > self.h {
                my = cy - 26 - side - 70;
            }
            let (mx, my) = (mx.max(4), my.max(4));
            SetStretchBltMode(dc, COLORONCOLOR);
            let _ = StretchBlt(
                dc,
                mx,
                my,
                side,
                side,
                Some(self.bright_dc),
                cx - MAG_SRC / 2,
                cy - MAG_SRC / 2,
                MAG_SRC,
                MAG_SRC,
                SRCCOPY,
            );
            let grid = CreatePen(PS_SOLID, 1, rgb(70, 70, 80));
            let old = SelectObject(dc, grid.into());
            for i in 0..=MAG_SRC {
                let o = i * MAG_ZOOM;
                let _ = MoveToEx(dc, mx + o, my, None);
                let _ = LineTo(dc, mx + o, my + side);
                let _ = MoveToEx(dc, mx, my + o, None);
                let _ = LineTo(dc, mx + side, my + o);
            }
            SelectObject(dc, old);
            let _ = DeleteObject(grid.into());
            // centre pixel and frame
            let pen = CreatePen(PS_SOLID, 2, ACCENT);
            let old_pen = SelectObject(dc, pen.into());
            let old_brush = SelectObject(dc, GetStockObject(NULL_BRUSH));
            let c = (MAG_SRC / 2) * MAG_ZOOM;
            let _ = Rectangle(dc, mx + c, my + c, mx + c + MAG_ZOOM + 1, my + c + MAG_ZOOM + 1);
            let _ = Rectangle(dc, mx - 1, my - 1, mx + side + 1, my + side + 1);
            SelectObject(dc, old_brush);
            SelectObject(dc, old_pen);
            let _ = DeleteObject(pen.into());
            let (r, g, b) = self.pixel(cx, cy);
            let (vx, vy) = (cx + self.origin.0, cy + self.origin.1);
            self.label(dc, &format!("{vx}, {vy}   #{r:02X}{g:02X}{b:02X}"), mx, my + side + 8);
            let sw = RECT {
                left: mx + side - 22,
                top: my + side + 10,
                right: mx + side,
                bottom: my + side + 28,
            };
            let br = CreateSolidBrush(rgb(r, g, b));
            FillRect(dc, &sw, br);
            let _ = DeleteObject(br.into());
        }
    }
}

fn client_cursor(hwnd: HWND) -> (i32, i32) {
    let mut p = POINT::default();
    // SAFETY: valid out parameter; `hwnd` is our window.
    unsafe {
        let _ = GetCursorPos(&mut p);
        let _ = ScreenToClient(hwnd, &mut p);
    }
    (p.x, p.y)
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // SAFETY: GWLP_USERDATA holds the `State` pointer installed at WM_NCCREATE; it outlives the window because
    // `run` frees it only after the message loop ends.
    unsafe {
        if msg == WM_NCCREATE {
            let cs = &*(lp.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let st = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        if st.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let st = &mut *st;
        let redraw = |hwnd: HWND| {
            let _ = InvalidateRect(Some(hwnd), None, false);
        };
        match msg {
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                st.paint(hdc);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_SETCURSOR => {
                SetCursor(LoadCursorW(None, IDC_CROSS).ok());
                LRESULT(1)
            }
            WM_MOUSEMOVE => {
                st.cur = client_cursor(hwnd);
                redraw(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                st.cur = client_cursor(hwnd);
                if st.mode == Mode::Region {
                    st.drag = Some(st.cur);
                    st.anchor = None;
                    SetCapture(hwnd);
                }
                redraw(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                st.cur = client_cursor(hwnd);
                let _ = ReleaseCapture();
                match st.mode {
                    Mode::Region => {
                        if let (Some(_), Some(r)) = (st.drag, st.selection()) {
                            // a click without dragging selects nothing yet
                            if r.w > 2 && r.h > 2 {
                                st.result = Some(r);
                                let _ = DestroyWindow(hwnd);
                            }
                        }
                        st.drag = None;
                    }
                    Mode::Window => {
                        st.result = st
                            .hovered_window()
                            .and_then(|r| r.intersect(&Rect::new(0, 0, st.w, st.h)));
                        if st.result.is_some() {
                            let _ = DestroyWindow(hwnd);
                        }
                    }
                }
                redraw(hwnd);
                LRESULT(0)
            }
            WM_RBUTTONDOWN => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_KEYDOWN => {
                let key = wp.0 as u16;
                let step = if GetKeyState(VK_SHIFT.0 as i32) < 0 { 10 } else { 1 };
                let (mut dx, mut dy) = (0, 0);
                if key == VK_ESCAPE.0 {
                    let _ = DestroyWindow(hwnd);
                } else if key == VK_LEFT.0 {
                    dx = -step;
                } else if key == VK_RIGHT.0 {
                    dx = step;
                } else if key == VK_UP.0 {
                    dy = -step;
                } else if key == VK_DOWN.0 {
                    dy = step;
                } else if key == VK_RETURN.0 {
                    match (st.mode, st.anchor) {
                        (Mode::Region, None) => st.anchor = Some(st.cur),
                        (Mode::Region, Some(_)) => {
                            st.result = st.selection();
                            let _ = DestroyWindow(hwnd);
                        }
                        (Mode::Window, _) => {
                            st.result = st.hovered_window();
                            let _ = DestroyWindow(hwnd);
                        }
                    }
                }
                if dx != 0 || dy != 0 {
                    let (nx, ny) = (
                        (st.cur.0 + dx).clamp(0, st.w - 1) + st.origin.0,
                        (st.cur.1 + dy).clamp(0, st.h - 1) + st.origin.1,
                    );
                    let _ = SetCursorPos(nx, ny);
                    st.cur = (nx - st.origin.0, ny - st.origin.1);
                }
                redraw(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use snapforge_core::capture::sys;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_MOUSE, MOUSE_EVENT_FLAGS, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT, SendInput,
    };

    /// One real cursor: the interactive tests must not overlap.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn click(flags: MOUSE_EVENT_FLAGS) {
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dwFlags: flags,
                    ..Default::default()
                },
            },
        };
        // SAFETY: one well-formed INPUT.
        unsafe {
            SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
        }
    }

    fn overlay_request(origin: (i32, i32)) -> Request {
        Request {
            frozen: RgbaImage::from_pixel(600, 400, image_pixel()),
            origin,
            mode: Mode::Region,
            windows: Vec::new(),
            hint: "test".into(),
        }
    }

    fn image_pixel() -> snapforge_core::image::Rgba<u8> {
        snapforge_core::image::Rgba([30, 120, 200, 255])
    }

    /// Drives the real overlay with synthesized mouse input. Needs an interactive desktop, so it is skipped in CI:
    /// `cargo test -p snapforge-app -- --ignored selector`.
    #[test]
    #[ignore = "needs an interactive desktop"]
    fn selector_returns_the_dragged_rectangle_in_virtual_pixels() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        sys::enable_dpi_awareness();
        let origin = (100, 100);
        let t = std::thread::spawn(move || run(overlay_request(origin)));
        std::thread::sleep(std::time::Duration::from_millis(900));
        // drag from virtual (150,160) to (349,299): the selection includes both end pixels
        // SAFETY: moving the cursor.
        unsafe {
            let _ = SetCursorPos(150, 160);
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
        click(MOUSEEVENTF_LEFTDOWN);
        std::thread::sleep(std::time::Duration::from_millis(150));
        // SAFETY: as above.
        unsafe {
            let _ = SetCursorPos(250, 200);
            std::thread::sleep(std::time::Duration::from_millis(100));
            let _ = SetCursorPos(349, 299);
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
        click(MOUSEEVENTF_LEFTUP);
        let r = t.join().unwrap();
        assert_eq!(r, Some(Rect::new(150, 160, 200, 140)));
    }

    #[test]
    #[ignore = "needs an interactive desktop"]
    fn right_click_cancels() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        sys::enable_dpi_awareness();
        let t = std::thread::spawn(move || run(overlay_request((100, 100))));
        std::thread::sleep(std::time::Duration::from_millis(900));
        // SAFETY: moving the cursor into the overlay.
        unsafe {
            let _ = SetCursorPos(200, 200);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        click(windows::Win32::UI::Input::KeyboardAndMouse::MOUSEEVENTF_RIGHTDOWN);
        click(windows::Win32::UI::Input::KeyboardAndMouse::MOUSEEVENTF_RIGHTUP);
        assert_eq!(t.join().unwrap(), None);
    }
}
