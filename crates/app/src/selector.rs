//! Full-screen capture overlay: pick a region, a window or a monitor — and annotate the screen first.
//!
//! A plain Win32 window over the whole virtual desktop showing a *frozen* screenshot, dimmed, with the selection cut
//! out in full brightness. A toolbar at the top (like ShareX's) offers the capture modes and drawing tools (rectangle,
//! ellipse, line, arrow, pencil, marker, text, numbered steps, blur, pixelate); what you draw stays an editable vector
//! object that is handed to the editor together with the chosen area. Because the process is per-monitor DPI aware,
//! window and cursor coordinates are physical pixels, so the selection is pixel-exact on every monitor. A magnifier
//! next to the cursor shows the pixels under it; the arrow keys move the cursor by one pixel (Shift: ten).

use std::collections::HashMap;
use std::ffi::c_void;

use snapforge_core::RgbaImage;
use snapforge_core::capture::sys::WindowInfo;
use snapforge_core::editor::{Editor, Object, Pt, Shape};
use snapforge_core::geometry::Rect;
use snapforge_core::settings::OverlayTool as Tool;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_BACK, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_RETURN,
    VK_RIGHT, VK_SHIFT, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

/// Everything the overlay needs; texts are already translated.
pub struct Request {
    /// Pixels of the whole virtual desktop.
    pub frozen: RgbaImage,
    /// Virtual-desktop position of `frozen`'s top-left pixel.
    pub origin: (i32, i32),
    pub windows: Vec<WindowInfo>,
    /// Monitors in virtual-desktop pixels (the "whole screen" tool captures the one under the cursor).
    pub monitors: Vec<Rect>,
    pub start_tool: Tool,
    /// Single-key shortcuts: `(tool, virtual-key code)`.
    pub keys: Vec<(Tool, u32)>,
    /// Tooltips per tool.
    pub labels: HashMap<Tool, String>,
    pub undo_label: String,
    pub cancel_label: String,
    /// One-line instruction under the toolbar.
    pub hint: String,
    pub color: [u8; 4],
    pub width: f32,
    pub privacy: bool,
    pub privacy_label: String,
}

/// What the user chose.
pub struct Outcome {
    /// The captured area in virtual-desktop pixels.
    pub rect: Rect,
    /// Annotations drawn before choosing the area, in pixels of `Request::frozen`.
    pub objects: Vec<Object>,
    /// Last drawing colour, width and tool (remembered for next time).
    pub color: [u8; 4],
    pub width: f32,
    pub tool: Tool,
    /// "Hide sensitive text" was switched on in the toolbar.
    pub privacy: bool,
}

const MAG_SRC: i32 = 17; // source pixels per side (odd: one centre pixel)
const MAG_ZOOM: i32 = 9;
const ACCENT: COLORREF = COLORREF(0x00FA_A560); // BGR of (96,165,250)
const BTN: i32 = 34;
const PALETTE: [[u8; 4]; 6] = [
    [230, 40, 40, 255],
    [250, 190, 40, 255],
    [60, 190, 90, 255],
    [60, 140, 250, 255],
    [255, 255, 255, 255],
    [20, 20, 20, 255],
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Btn {
    Tool(Tool),
    Color(usize),
    WidthDown,
    WidthUp,
    Privacy,
    Undo,
    Cancel,
}

#[derive(Clone, Copy, Debug)]
pub struct Button {
    pub rect: Rect,
    pub kind: Btn,
}

/// Lays the toolbar out centred at the top of `monitor` (client coordinates). Returns the bar rectangle and buttons.
pub fn toolbar_layout(monitor: Rect) -> (Rect, Vec<Button>) {
    let mut items: Vec<(Btn, i32)> = Vec::new(); // (button, width); a width of 0 is a separator
    for t in Tool::ALL {
        if t == Tool::Pencil || t == Tool::Rect || t == Tool::Window {
            // groups: capture | shapes | freehand & text | effects
        }
        items.push((Btn::Tool(t), BTN));
    }
    let tool_count = items.len();
    let mut widths: Vec<(Option<Btn>, i32)> = Vec::new();
    for (i, (b, w)) in items.into_iter().enumerate() {
        if i == 4 {
            widths.push((None, 10)); // after the capture tools
        }
        widths.push((Some(b), w));
    }
    let _ = tool_count;
    widths.push((None, 10));
    for i in 0..PALETTE.len() {
        widths.push((Some(Btn::Color(i)), 24));
    }
    widths.push((None, 10));
    widths.push((Some(Btn::WidthDown), 26));
    widths.push((None, 30)); // the number
    widths.push((Some(Btn::WidthUp), 26));
    widths.push((None, 10));
    widths.push((Some(Btn::Privacy), BTN));
    widths.push((Some(Btn::Undo), BTN));
    widths.push((Some(Btn::Cancel), BTN));
    let pad = 8;
    let total: i32 = widths.iter().map(|(_, w)| *w).sum::<i32>()
        + pad * 2
        + widths.iter().filter(|(b, _)| b.is_some()).count() as i32 * 2;
    let bar = Rect::new(
        monitor.x + (monitor.w - total) / 2,
        monitor.y + 10,
        total,
        BTN + pad * 2,
    );
    let mut x = bar.x + pad;
    let mut out = Vec::new();
    for (b, w) in widths {
        if let Some(kind) = b {
            let size = if matches!(kind, Btn::Color(_)) {
                20
            } else if matches!(kind, Btn::WidthDown | Btn::WidthUp) {
                24
            } else {
                BTN
            };
            out.push(Button {
                rect: Rect::new(x + (w - size) / 2, bar.y + pad + (BTN - size) / 2, size, size),
                kind,
            });
            x += w + 2;
        } else {
            x += w;
        }
    }
    (bar, out)
}

struct Drag {
    start: (i32, i32),
    pts: Vec<Pt>,
}

struct TextEntry {
    at: (i32, i32),
    buf: String,
}

struct State {
    w: i32,
    h: i32,
    origin: (i32, i32),
    tool: Tool,
    windows: Vec<WindowInfo>,
    monitors: Vec<Rect>,
    keys: Vec<(Tool, u32)>,
    labels: HashMap<Tool, String>,
    undo_label: String,
    cancel_label: String,
    hint: Vec<u16>,
    bar: Rect,
    buttons: Vec<Button>,
    ed: Editor,
    color: [u8; 4],
    width: f32,
    privacy: bool,
    privacy_label: String,
    bright_dc: HDC,
    dim_dc: HDC,
    back_dc: HDC,
    bright_bmp: HBITMAP,
    dim_bmp: HBITMAP,
    back_bmp: HBITMAP,
    bright_bits: *mut u8,
    dim_bits: *mut u8,
    font: HFONT,
    symbols: HFONT,
    /// Mouse drag selecting the capture area (client coordinates).
    region_drag: Option<(i32, i32)>,
    /// Keyboard-driven anchor (Enter once to start, Enter again to confirm).
    anchor: Option<(i32, i32)>,
    /// A shape being drawn with a mouse drag.
    draw: Option<Drag>,
    text: Option<TextEntry>,
    cur: (i32, i32),
    result: Option<Rect>,
}

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

fn rgba(c: [u8; 4]) -> COLORREF {
    rgb(c[0], c[1], c[2])
}

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Shows the overlay and blocks until the user picks something or cancels.
pub fn run(req: Request) -> Option<Outcome> {
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
        let outcome = st.result.map(|r| Outcome {
            rect: r.translate(req.origin.0, req.origin.1),
            objects: st.ed.doc.objects.clone(),
            color: st.color,
            width: st.width,
            tool: st.tool,
            privacy: st.privacy,
        });
        st.free();
        outcome
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
            let mk_dc = |bmp: HBITMAP| {
                let dc = CreateCompatibleDC(Some(screen));
                SelectObject(dc, bmp.into());
                dc
            };
            let font = |height: i32, face: windows::core::PCWSTR| {
                CreateFontW(
                    height,
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
                    face,
                )
            };
            let cursor = snapforge_core::capture::sys::cursor_pos().map(|(x, y)| (x - req.origin.0, y - req.origin.1));
            let monitor = req
                .monitors
                .iter()
                .map(|m| m.translate(-req.origin.0, -req.origin.1))
                .find(|m| cursor.is_some_and(|(x, y)| m.contains(x, y)))
                .or_else(|| req.monitors.first().map(|m| m.translate(-req.origin.0, -req.origin.1)))
                .unwrap_or(Rect::new(0, 0, w, h));
            let (bar, buttons) = toolbar_layout(monitor);
            let mut st = State {
                w,
                h,
                origin: req.origin,
                tool: req.start_tool,
                windows: req.windows.clone(),
                monitors: req
                    .monitors
                    .iter()
                    .map(|m| m.translate(-req.origin.0, -req.origin.1))
                    .collect(),
                keys: req.keys.clone(),
                labels: req.labels.clone(),
                undo_label: req.undo_label.clone(),
                cancel_label: req.cancel_label.clone(),
                hint: utf16(&req.hint),
                bar,
                buttons,
                ed: Editor::new(req.frozen.clone()),
                color: req.color,
                width: req.width.clamp(1.0, 24.0),
                privacy: req.privacy,
                privacy_label: req.privacy_label.clone(),
                bright_dc: mk_dc(bright_bmp),
                dim_dc: mk_dc(dim_bmp),
                back_dc: mk_dc(back_bmp),
                bright_bmp,
                dim_bmp,
                back_bmp,
                bright_bits: b1 as *mut u8,
                dim_bits: b2 as *mut u8,
                font: font(-16, w!("Segoe UI")),
                symbols: font(-20, w!("Segoe UI Symbol")),
                region_drag: None,
                anchor: None,
                draw: None,
                text: None,
                cur: (0, 0),
                result: None,
            };
            ReleaseDC(None, screen);
            st.write_pixels(&req.frozen);
            st
        }
    }

    /// Copies `img` into the bright DIB and a darkened copy into the dim DIB.
    fn write_pixels(&mut self, img: &RgbaImage) {
        if self.bright_bits.is_null() || self.dim_bits.is_null() {
            return;
        }
        let n = (self.w as usize) * (self.h as usize) * 4;
        // SAFETY: both DIB sections were created with w*h BGRA pixels.
        let (bright, dim) = unsafe {
            (
                std::slice::from_raw_parts_mut(self.bright_bits, n),
                std::slice::from_raw_parts_mut(self.dim_bits, n),
            )
        };
        for ((src, b), d) in img
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

    /// Re-renders the base picture with all annotations into the DIBs.
    fn refresh(&mut self) {
        let img = self.ed.doc.render();
        self.write_pixels(&img);
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
            let _ = DeleteObject(self.symbols.into());
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

    fn hovered_monitor(&self) -> Option<Rect> {
        self.monitors
            .iter()
            .copied()
            .find(|m| m.contains(self.cur.0, self.cur.1))
    }

    fn over_bar(&self, p: (i32, i32)) -> bool {
        self.bar.contains(p.0, p.1)
    }

    fn button_at(&self, p: (i32, i32)) -> Option<Button> {
        self.buttons.iter().copied().find(|b| b.rect.contains(p.0, p.1))
    }

    fn selection(&self) -> Option<Rect> {
        let start = self.region_drag.or(self.anchor);
        match (self.tool, start) {
            (Tool::Region | Tool::Ocr, Some(s)) => {
                // the selection includes the pixel under the cursor, like a rubber band
                let (x0, y0) = (s.0.min(self.cur.0), s.1.min(self.cur.1));
                let (x1, y1) = (s.0.max(self.cur.0), s.1.max(self.cur.1));
                Some(Rect::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1))
            }
            (Tool::Window, _) if !self.over_bar(self.cur) => self.hovered_window(),
            (Tool::Screen, _) if !self.over_bar(self.cur) => self.hovered_monitor(),
            _ => None,
        }
        .and_then(|r| r.intersect(&Rect::new(0, 0, self.w, self.h)))
    }

    fn text_size(&self) -> f32 {
        14.0 + self.width * 3.0
    }

    /// Adds the shape of the finished mouse drag (if it is big enough to be intentional).
    fn finish_draw(&mut self) {
        let Some(d) = self.draw.take() else { return };
        let (a, b) = (
            Pt::new(d.start.0 as f32, d.start.1 as f32),
            Pt::new(self.cur.0 as f32, self.cur.1 as f32),
        );
        let long = (a.x - b.x).hypot(a.y - b.y) >= 3.0;
        let boxy = (a.x - b.x).abs() >= 3.0 && (a.y - b.y).abs() >= 3.0;
        let shape = match self.tool {
            Tool::Rect if boxy => Some(Shape::Rect { a, b, fill: false }),
            Tool::Ellipse if boxy => Some(Shape::Ellipse { a, b, fill: false }),
            Tool::Line if long => Some(Shape::Line { a, b }),
            Tool::Arrow if long => Some(Shape::Arrow { a, b }),
            Tool::Blur if boxy => Some(Shape::Blur { a, b, radius: 10 }),
            Tool::Pixelate if boxy => Some(Shape::Pixelate { a, b, block: 10 }),
            Tool::Pencil if d.pts.len() > 1 => Some(Shape::Pencil { pts: d.pts }),
            Tool::Marker if d.pts.len() > 1 => Some(Shape::Marker { pts: d.pts }),
            _ => None,
        };
        if let Some(s) = shape {
            self.ed.checkpoint();
            self.ed.doc.add(s, self.color, self.width);
            self.refresh();
        }
    }

    fn commit_text(&mut self) {
        let Some(t) = self.text.take() else { return };
        if !t.buf.trim().is_empty() {
            self.ed.checkpoint();
            let size = self.text_size();
            self.ed.doc.add(
                Shape::Text {
                    at: Pt::new(t.at.0 as f32, t.at.1 as f32),
                    text: t.buf,
                    size,
                },
                self.color,
                self.width,
            );
            self.refresh();
        }
    }

    fn add_step(&mut self, at: (i32, i32)) {
        self.ed.checkpoint();
        let n = self.ed.doc.next_step_number();
        self.ed.doc.add(
            Shape::Step {
                at: Pt::new(at.0 as f32, at.1 as f32),
                n,
            },
            self.color,
            self.width,
        );
        self.refresh();
    }

    fn undo(&mut self) {
        if self.ed.undo() {
            self.refresh();
        }
    }

    fn press_button(&mut self, b: Button) -> Action {
        self.commit_text();
        match b.kind {
            Btn::Tool(t) => {
                self.tool = t;
                self.anchor = None;
            }
            Btn::Color(i) => self.color = PALETTE[i],
            Btn::WidthDown => self.width = (self.width - 1.0).max(1.0),
            Btn::WidthUp => self.width = (self.width + 1.0).min(24.0),
            Btn::Privacy => self.privacy = !self.privacy,
            Btn::Undo => self.undo(),
            Btn::Cancel => return Action::Cancel,
        }
        Action::None
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
                self.label(
                    back,
                    &format!("{} × {}", r.w, r.h),
                    r.x,
                    (r.y - 30).max(self.bar.bottom() + 4),
                );
            }
            self.paint_drawing(back);
            self.paint_toolbar(back);
            if self.tool.is_capture() && !self.over_bar(self.cur) {
                self.magnifier(back);
            }
            let _ = BitBlt(hdc, 0, 0, self.w, self.h, Some(back), 0, 0, SRCCOPY);
        }
    }

    /// The shape being drawn and the text being typed, as cheap GDI previews.
    ///
    /// # Safety
    /// `dc` must be a valid DC.
    unsafe fn paint_drawing(&self, dc: HDC) {
        // SAFETY: valid DC; every pen/brush created here is deleted before returning.
        unsafe {
            if let Some(d) = &self.draw {
                let (s, c) = (d.start, self.cur);
                let w = (self.width.round() as i32).max(1);
                let (style, width, col) = match self.tool {
                    Tool::Blur | Tool::Pixelate => (PS_DOT, 1, ACCENT),
                    Tool::Marker => (PS_SOLID, w * 5, rgba(self.color)),
                    _ => (PS_SOLID, w, rgba(self.color)),
                };
                let pen = CreatePen(style, width, col);
                let old_pen = SelectObject(dc, pen.into());
                let old_brush = SelectObject(dc, GetStockObject(NULL_BRUSH));
                match self.tool {
                    Tool::Rect | Tool::Blur | Tool::Pixelate => {
                        let _ = Rectangle(dc, s.0.min(c.0), s.1.min(c.1), s.0.max(c.0), s.1.max(c.1));
                    }
                    Tool::Ellipse => {
                        let _ = Ellipse(dc, s.0.min(c.0), s.1.min(c.1), s.0.max(c.0), s.1.max(c.1));
                    }
                    Tool::Line => {
                        let _ = MoveToEx(dc, s.0, s.1, None);
                        let _ = LineTo(dc, c.0, c.1);
                    }
                    Tool::Arrow => {
                        let _ = MoveToEx(dc, s.0, s.1, None);
                        let _ = LineTo(dc, c.0, c.1);
                        let (dx, dy) = ((c.0 - s.0) as f32, (c.1 - s.1) as f32);
                        let len = dx.hypot(dy).max(1.0);
                        let (ux, uy) = (dx / len, dy / len);
                        let head = (self.width * 4.5).max(12.0).min(len * 0.9);
                        for side in [-1.0f32, 1.0] {
                            let (hx, hy) = (
                                c.0 as f32 - ux * head - uy * head * 0.42 * side,
                                c.1 as f32 - uy * head + ux * head * 0.42 * side,
                            );
                            let _ = MoveToEx(dc, c.0, c.1, None);
                            let _ = LineTo(dc, hx as i32, hy as i32);
                        }
                    }
                    Tool::Pencil | Tool::Marker => {
                        for (i, p) in d.pts.iter().enumerate() {
                            if i == 0 {
                                let _ = MoveToEx(dc, p.x as i32, p.y as i32, None);
                            } else {
                                let _ = LineTo(dc, p.x as i32, p.y as i32);
                            }
                        }
                    }
                    _ => {}
                }
                SelectObject(dc, old_brush);
                SelectObject(dc, old_pen);
                let _ = DeleteObject(pen.into());
            }
            if let Some(t) = &self.text {
                let size = self.text_size() as i32;
                let f = CreateFontW(
                    -size,
                    0,
                    0,
                    0,
                    FW_NORMAL.0 as i32,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET,
                    OUT_DEFAULT_PRECIS,
                    CLIP_DEFAULT_PRECIS,
                    CLEARTYPE_QUALITY,
                    0,
                    w!("Segoe UI"),
                );
                let old = SelectObject(dc, f.into());
                SetBkMode(dc, TRANSPARENT);
                SetTextColor(dc, rgba(self.color));
                let shown = utf16(&format!("{}|", t.buf));
                let _ = TextOutW(dc, t.at.0, t.at.1, &shown);
                SelectObject(dc, old);
                let _ = DeleteObject(f.into());
            }
        }
    }

    /// # Safety
    /// `dc` must be a valid DC.
    unsafe fn paint_toolbar(&self, dc: HDC) {
        // SAFETY: valid DC; GDI objects are released.
        unsafe {
            let bar = &self.bar;
            let br = CreateSolidBrush(rgb(24, 26, 34));
            let pen = CreatePen(PS_SOLID, 1, rgb(70, 74, 90));
            let (ob, op) = (SelectObject(dc, br.into()), SelectObject(dc, pen.into()));
            let _ = RoundRect(dc, bar.x, bar.y, bar.right(), bar.bottom(), 14, 14);
            SelectObject(dc, ob);
            SelectObject(dc, op);
            let _ = DeleteObject(br.into());
            let _ = DeleteObject(pen.into());

            let hover = self.button_at(self.cur);
            for b in &self.buttons {
                let active = match b.kind {
                    Btn::Tool(t) => t == self.tool,
                    Btn::Color(i) => PALETTE[i] == self.color,
                    Btn::Privacy => self.privacy,
                    _ => false,
                };
                let hot = hover.is_some_and(|h| h.rect == b.rect);
                self.paint_button(dc, b, active, hot);
            }
            // the width number between the − and + buttons
            if let (Some(down), Some(up)) = (
                self.buttons.iter().find(|b| b.kind == Btn::WidthDown),
                self.buttons.iter().find(|b| b.kind == Btn::WidthUp),
            ) {
                let old = SelectObject(dc, self.font.into());
                SetBkMode(dc, TRANSPARENT);
                SetTextColor(dc, rgb(230, 232, 240));
                let t = utf16(&format!("{}", self.width.round() as i32));
                let mut sz = windows::Win32::Foundation::SIZE::default();
                let _ = GetTextExtentPoint32W(dc, &t, &mut sz);
                let cx = (down.rect.right() + up.rect.x) / 2;
                let _ = TextOutW(dc, cx - sz.cx / 2, bar.y + (bar.h - sz.cy) / 2, &t);
                SelectObject(dc, old);
            }
            // hint or the tooltip of the hovered button, centred under the bar
            let text = match hover.map(|b| b.kind) {
                Some(Btn::Tool(t)) => {
                    let key = self
                        .keys
                        .iter()
                        .find(|(x, _)| *x == t)
                        .map(|(_, vk)| snapforge_core::hotkey::key_label(*vk));
                    let name = self.labels.get(&t).cloned().unwrap_or_default();
                    match key {
                        Some(k) => format!("{name}  [{k}]"),
                        None => name,
                    }
                }
                Some(Btn::Privacy) => self.privacy_label.clone(),
                Some(Btn::Undo) => format!("{}  [Ctrl+Z]", self.undo_label),
                Some(Btn::Cancel) => format!("{}  [Esc]", self.cancel_label),
                Some(Btn::WidthDown) => "[".to_owned(),
                Some(Btn::WidthUp) => "]".to_owned(),
                _ => String::from_utf16_lossy(&self.hint),
            };
            if !text.is_empty() {
                let old = SelectObject(dc, self.font.into());
                let t = utf16(&text);
                let mut sz = windows::Win32::Foundation::SIZE::default();
                let _ = GetTextExtentPoint32W(dc, &t, &mut sz);
                SelectObject(dc, old);
                self.label(dc, &text, bar.x + (bar.w - sz.cx - 14) / 2, bar.bottom() + 8);
            }
        }
    }

    /// # Safety
    /// `dc` must be a valid DC.
    unsafe fn paint_button(&self, dc: HDC, b: &Button, active: bool, hot: bool) {
        // SAFETY: valid DC; GDI objects are released.
        unsafe {
            let r = b.rect;
            if let Btn::Color(i) = b.kind {
                let br = CreateSolidBrush(rgba(PALETTE[i]));
                let pen = CreatePen(
                    PS_SOLID,
                    if active { 3 } else { 1 },
                    if active { ACCENT } else { rgb(110, 114, 130) },
                );
                let (ob, op) = (SelectObject(dc, br.into()), SelectObject(dc, pen.into()));
                let _ = Ellipse(dc, r.x, r.y, r.right(), r.bottom());
                SelectObject(dc, ob);
                SelectObject(dc, op);
                let _ = DeleteObject(br.into());
                let _ = DeleteObject(pen.into());
                return;
            }
            if active || hot {
                let br = CreateSolidBrush(if active { rgb(38, 79, 150) } else { rgb(50, 54, 68) });
                let pen = CreatePen(PS_SOLID, 1, if active { ACCENT } else { rgb(70, 74, 90) });
                let (ob, op) = (SelectObject(dc, br.into()), SelectObject(dc, pen.into()));
                let _ = RoundRect(dc, r.x, r.y, r.right(), r.bottom(), 8, 8);
                SelectObject(dc, ob);
                SelectObject(dc, op);
                let _ = DeleteObject(br.into());
                let _ = DeleteObject(pen.into());
            }
            let ink = rgb(225, 228, 238);
            let pen = CreatePen(PS_SOLID, 2, ink);
            let old_pen = SelectObject(dc, pen.into());
            let old_brush = SelectObject(dc, GetStockObject(NULL_BRUSH));
            let (x, y, w, h) = (r.x + 7, r.y + 7, r.w - 14, r.h - 14);
            let line = |x1: i32, y1: i32, x2: i32, y2: i32| {
                let _ = MoveToEx(dc, x1, y1, None);
                let _ = LineTo(dc, x2, y2);
            };
            match b.kind {
                Btn::Tool(Tool::Region) => {
                    let c = 6;
                    for (px, py, sx, sy) in [
                        (x, y, 1, 1),
                        (x + w, y, -1, 1),
                        (x, y + h, 1, -1),
                        (x + w, y + h, -1, -1),
                    ] {
                        line(px, py, px + sx * c, py);
                        line(px, py, px, py + sy * c);
                    }
                }
                Btn::Tool(Tool::Window) => {
                    let _ = Rectangle(dc, x, y, x + w, y + h);
                    line(x, y + 5, x + w, y + 5);
                }
                Btn::Tool(Tool::Screen) => {
                    let _ = Rectangle(dc, x, y + 1, x + w, y + h - 4);
                    line(x + w / 2 - 4, y + h - 1, x + w / 2 + 4, y + h - 1);
                }
                Btn::Tool(Tool::Ocr) => {
                    // a frame with text lines inside
                    let _ = Rectangle(dc, x, y, x + w, y + h);
                    for k in 1..4 {
                        line(x + 3, y + k * h / 4, x + w - 3 - (k % 2) * 4, y + k * h / 4);
                    }
                }
                Btn::Privacy => {
                    // a padlock
                    let _ = Rectangle(dc, x + 1, y + h / 2 - 1, x + w - 1, y + h);
                    let _ = Arc(
                        dc,
                        x + 3,
                        y,
                        x + w - 3,
                        y + h - 2,
                        x + w - 3,
                        y + h / 2 - 1,
                        x + 3,
                        y + h / 2 - 1,
                    );
                }
                Btn::Tool(Tool::Rect) => {
                    let _ = Rectangle(dc, x, y + 2, x + w, y + h - 2);
                }
                Btn::Tool(Tool::Ellipse) => {
                    let _ = Ellipse(dc, x, y + 2, x + w, y + h - 2);
                }
                Btn::Tool(Tool::Line) => line(x, y + h, x + w, y),
                Btn::Tool(Tool::Arrow) => {
                    line(x, y + h, x + w, y);
                    line(x + w, y, x + w - 8, y + 1);
                    line(x + w, y, x + w - 1, y + 8);
                }
                Btn::Tool(Tool::Pencil) => {
                    let pts = [
                        (x, y + h - 2),
                        (x + 5, y + 4),
                        (x + 9, y + h - 4),
                        (x + 14, y + 2),
                        (x + w, y + 8),
                    ];
                    for p in pts.windows(2) {
                        line(p[0].0, p[0].1, p[1].0, p[1].1);
                    }
                }
                Btn::Tool(Tool::Marker) => {
                    let m = CreatePen(PS_SOLID, 8, rgb(250, 200, 60));
                    let o = SelectObject(dc, m.into());
                    line(x + 2, y + h - 4, x + w - 2, y + 4);
                    SelectObject(dc, o);
                    let _ = DeleteObject(m.into());
                }
                Btn::Tool(Tool::Text) => {
                    let old = SelectObject(dc, self.font.into());
                    SetBkMode(dc, TRANSPARENT);
                    SetTextColor(dc, ink);
                    let _ = TextOutW(dc, r.x + 11, r.y + 6, &utf16("A"));
                    SelectObject(dc, old);
                }
                Btn::Tool(Tool::Step) => {
                    let _ = Ellipse(dc, x, y, x + w, y + h);
                    let old = SelectObject(dc, self.font.into());
                    SetBkMode(dc, TRANSPARENT);
                    SetTextColor(dc, ink);
                    let _ = TextOutW(dc, r.x + 13, r.y + 7, &utf16("1"));
                    SelectObject(dc, old);
                }
                Btn::Tool(Tool::Blur) => {
                    let _ = Rectangle(dc, x, y, x + w, y + h);
                    for k in 1..4 {
                        line(x + 3, y + k * h / 4, x + w - 3, y + k * h / 4);
                    }
                }
                Btn::Tool(Tool::Pixelate) => {
                    let _ = Rectangle(dc, x, y, x + w, y + h);
                    line(x + w / 2, y, x + w / 2, y + h);
                    line(x, y + h / 2, x + w, y + h / 2);
                }
                Btn::WidthDown | Btn::WidthUp => {
                    line(x + 1, y + h / 2, x + w - 1, y + h / 2);
                    if b.kind == Btn::WidthUp {
                        line(x + w / 2, y + 1, x + w / 2, y + h - 1);
                    }
                }
                Btn::Undo | Btn::Cancel => {
                    let old = SelectObject(dc, self.symbols.into());
                    SetBkMode(dc, TRANSPARENT);
                    SetTextColor(dc, if b.kind == Btn::Cancel { rgb(235, 110, 110) } else { ink });
                    let glyph = if b.kind == Btn::Undo { "↶" } else { "✕" };
                    let _ = TextOutW(dc, r.x + 8, r.y + 4, &utf16(glyph));
                    SelectObject(dc, old);
                }
                Btn::Color(_) => {}
            }
            SelectObject(dc, old_brush);
            SelectObject(dc, old_pen);
            let _ = DeleteObject(pen.into());
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

enum Action {
    None,
    Cancel,
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
                let over = st.over_bar(client_cursor(hwnd));
                SetCursor(LoadCursorW(None, if over { IDC_ARROW } else { IDC_CROSS }).ok());
                LRESULT(1)
            }
            WM_MOUSEMOVE => {
                st.cur = client_cursor(hwnd);
                if let Some(d) = &mut st.draw
                    && matches!(st.tool, Tool::Pencil | Tool::Marker)
                {
                    d.pts.push(Pt::new(st.cur.0 as f32, st.cur.1 as f32));
                }
                redraw(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                st.cur = client_cursor(hwnd);
                if st.over_bar(st.cur) {
                    if let Some(b) = st.button_at(st.cur)
                        && let Action::Cancel = st.press_button(b)
                    {
                        let _ = DestroyWindow(hwnd);
                    }
                    redraw(hwnd);
                    return LRESULT(0);
                }
                st.commit_text();
                match st.tool {
                    Tool::Region | Tool::Ocr => {
                        st.region_drag = Some(st.cur);
                        st.anchor = None;
                        SetCapture(hwnd);
                    }
                    Tool::Window | Tool::Screen => {}
                    Tool::Text => {
                        st.text = Some(TextEntry {
                            at: st.cur,
                            buf: String::new(),
                        });
                    }
                    Tool::Step => st.add_step(st.cur),
                    _ => {
                        st.draw = Some(Drag {
                            start: st.cur,
                            pts: vec![Pt::new(st.cur.0 as f32, st.cur.1 as f32)],
                        });
                        SetCapture(hwnd);
                    }
                }
                redraw(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                st.cur = client_cursor(hwnd);
                let _ = ReleaseCapture();
                if st.draw.is_some() {
                    st.finish_draw();
                } else if st.over_bar(st.cur) && st.region_drag.is_none() {
                    // the click belonged to the toolbar
                } else {
                    match st.tool {
                        Tool::Region | Tool::Ocr => {
                            if let (Some(_), Some(r)) = (st.region_drag, st.selection()) {
                                // a click without dragging selects nothing yet
                                if r.w > 2 && r.h > 2 {
                                    st.result = Some(r);
                                    let _ = DestroyWindow(hwnd);
                                }
                            }
                            st.region_drag = None;
                        }
                        Tool::Window => {
                            st.result = st
                                .hovered_window()
                                .and_then(|r| r.intersect(&Rect::new(0, 0, st.w, st.h)));
                            if st.result.is_some() {
                                let _ = DestroyWindow(hwnd);
                            }
                        }
                        Tool::Screen => {
                            st.result = st
                                .hovered_monitor()
                                .and_then(|r| r.intersect(&Rect::new(0, 0, st.w, st.h)));
                            if st.result.is_some() {
                                let _ = DestroyWindow(hwnd);
                            }
                        }
                        _ => {}
                    }
                }
                redraw(hwnd);
                LRESULT(0)
            }
            WM_RBUTTONDOWN => {
                // first cancel what is in progress, then the whole overlay
                if st.draw.take().is_some() || st.text.take().is_some() || st.region_drag.take().is_some() {
                    let _ = ReleaseCapture();
                    redraw(hwnd);
                } else {
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_CHAR => {
                if let Some(t) = &mut st.text {
                    match wp.0 as u32 {
                        8 => {
                            t.buf.pop();
                        }
                        13 => st.commit_text(),
                        c if c >= 32 && c != 127 => {
                            if let Some(ch) = char::from_u32(c) {
                                t.buf.push(ch);
                            }
                        }
                        _ => {}
                    }
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            WM_KEYDOWN => {
                let key = wp.0 as u16;
                if st.text.is_some() {
                    if key == VK_ESCAPE.0 {
                        st.text = None;
                    }
                    redraw(hwnd);
                    return LRESULT(0);
                }
                let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
                let step = if GetKeyState(VK_SHIFT.0 as i32) < 0 { 10 } else { 1 };
                let (mut dx, mut dy) = (0, 0);
                if key == VK_ESCAPE.0 {
                    let _ = DestroyWindow(hwnd);
                } else if ctrl && key == b'Z' as u16 {
                    st.undo();
                } else if key == VK_LEFT.0 {
                    dx = -step;
                } else if key == VK_RIGHT.0 {
                    dx = step;
                } else if key == VK_UP.0 {
                    dy = -step;
                } else if key == VK_DOWN.0 {
                    dy = step;
                } else if key == VK_RETURN.0 {
                    match (st.tool, st.anchor) {
                        (Tool::Region | Tool::Ocr, None) => st.anchor = Some(st.cur),
                        (Tool::Region | Tool::Ocr, Some(_)) => {
                            st.result = st.selection();
                            let _ = DestroyWindow(hwnd);
                        }
                        (Tool::Window, _) => {
                            st.result = st.hovered_window();
                            let _ = DestroyWindow(hwnd);
                        }
                        (Tool::Screen, _) => {
                            st.result = st.hovered_monitor();
                            let _ = DestroyWindow(hwnd);
                        }
                        _ => {}
                    }
                } else if key == 0xDB {
                    // [
                    st.width = (st.width - 1.0).max(1.0);
                } else if key == 0xDD {
                    // ]
                    st.width = (st.width + 1.0).min(24.0);
                } else if !ctrl
                    && key != VK_BACK.0
                    && let Some((t, _)) = st.keys.iter().find(|(_, vk)| *vk == key as u32)
                {
                    st.tool = *t;
                    st.anchor = None;
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

    fn pause(ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }

    fn move_to(x: i32, y: i32) {
        // SAFETY: moving the cursor.
        unsafe {
            let _ = SetCursorPos(x, y);
        }
    }

    fn drag(from: (i32, i32), to: (i32, i32)) {
        move_to(from.0, from.1);
        pause(150);
        click(MOUSEEVENTF_LEFTDOWN);
        pause(150);
        move_to((from.0 + to.0) / 2, (from.1 + to.1) / 2);
        pause(100);
        move_to(to.0, to.1);
        pause(150);
        click(MOUSEEVENTF_LEFTUP);
        pause(150);
    }

    fn base_request(origin: (i32, i32), start_tool: Tool) -> Request {
        Request {
            frozen: RgbaImage::from_pixel(600, 400, snapforge_core::image::Rgba([30, 120, 200, 255])),
            origin,
            windows: Vec::new(),
            monitors: vec![Rect::new(origin.0, origin.1, 600, 400)],
            start_tool,
            keys: Vec::new(),
            labels: HashMap::new(),
            undo_label: "Undo".into(),
            cancel_label: "Cancel".into(),
            hint: "test".into(),
            color: [230, 40, 40, 255],
            width: 4.0,
            privacy: false,
            privacy_label: "Hide sensitive text".into(),
        }
    }

    /// Centre (virtual pixels) of a toolbar button of an overlay created at `origin` over one 600x400 monitor.
    fn button_center(origin: (i32, i32), kind: Btn) -> (i32, i32) {
        let (_, buttons) = toolbar_layout(Rect::new(0, 0, 600, 400));
        let b = buttons.iter().find(|b| b.kind == kind).expect("button");
        (origin.0 + b.rect.x + b.rect.w / 2, origin.1 + b.rect.y + b.rect.h / 2)
    }

    #[test]
    fn the_toolbar_fits_and_has_every_tool_once() {
        let (bar, buttons) = toolbar_layout(Rect::new(0, 0, 1920, 1080));
        assert!(bar.x > 0 && bar.right() < 1920, "centred inside the monitor: {bar:?}");
        for t in Tool::ALL {
            assert_eq!(buttons.iter().filter(|b| b.kind == Btn::Tool(t)).count(), 1, "{t:?}");
        }
        for b in &buttons {
            assert!(bar.intersect(&b.rect) == Some(b.rect), "{b:?} lies inside the bar");
        }
        // no two buttons overlap
        for (i, a) in buttons.iter().enumerate() {
            for b in &buttons[i + 1..] {
                assert!(
                    a.rect.intersect(&b.rect).is_none(),
                    "{:?} overlaps {:?}",
                    a.kind,
                    b.kind
                );
            }
        }
        // the toolbar of a monitor at a negative position sits on that monitor
        let (bar2, _) = toolbar_layout(Rect::new(-1920, 0, 1920, 1080));
        assert!(bar2.x < 0 && bar2.right() <= 0);
    }

    /// Drives the real overlay with synthesized mouse input. Needs an interactive desktop, so it is skipped in CI:
    /// `cargo test -p snapforge-app -- --ignored selector`.
    #[test]
    #[ignore = "needs an interactive desktop"]
    fn selector_returns_the_dragged_rectangle_in_virtual_pixels() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        sys::enable_dpi_awareness();
        let origin = (100, 100);
        let t = std::thread::spawn(move || run(base_request(origin, Tool::Region)));
        pause(900);
        // drag from virtual (150,160) to (349,299): the selection includes both end pixels
        drag((150, 160), (349, 299));
        let r = t.join().unwrap().expect("a selection");
        assert_eq!(r.rect, Rect::new(150, 160, 200, 140));
        assert!(r.objects.is_empty());
    }

    #[test]
    #[ignore = "needs an interactive desktop"]
    fn window_mode_picks_the_hovered_window() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        sys::enable_dpi_awareness();
        let window = |hwnd, x, y, w, h| sys::WindowInfo {
            hwnd,
            title: "t".into(),
            app: "demo".into(),
            rect: Rect::new(x, y, w, h),
        };
        // z-order: the small window is above the big one
        let windows = vec![window(1, 300, 260, 100, 80), window(2, 200, 200, 300, 200)];
        let req = Request {
            windows,
            ..base_request((100, 100), Tool::Window)
        };
        let t = std::thread::spawn(move || run(req));
        pause(900);
        move_to(320, 280);
        pause(150);
        click(MOUSEEVENTF_LEFTDOWN);
        click(MOUSEEVENTF_LEFTUP);
        assert_eq!(t.join().unwrap().map(|o| o.rect), Some(Rect::new(300, 260, 100, 80)));
    }

    #[test]
    #[ignore = "needs an interactive desktop"]
    fn right_click_cancels() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        sys::enable_dpi_awareness();
        let t = std::thread::spawn(move || run(base_request((100, 100), Tool::Region)));
        pause(900);
        move_to(200, 300);
        pause(100);
        click(windows::Win32::UI::Input::KeyboardAndMouse::MOUSEEVENTF_RIGHTDOWN);
        click(windows::Win32::UI::Input::KeyboardAndMouse::MOUSEEVENTF_RIGHTUP);
        assert!(t.join().unwrap().is_none());
    }

    #[test]
    #[ignore = "needs an interactive desktop"]
    fn drawing_an_arrow_then_choosing_the_area_returns_both() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        sys::enable_dpi_awareness();
        let origin = (100, 100);
        let t = std::thread::spawn(move || run(base_request(origin, Tool::Arrow)));
        pause(900);
        // an arrow from client (200,200) to (300,260) (virtual +100)
        drag((300, 300), (400, 360));
        // switch to the area tool by clicking its toolbar button, then select
        let (cx, cy) = button_center(origin, Btn::Tool(Tool::Region));
        move_to(cx, cy);
        pause(150);
        click(MOUSEEVENTF_LEFTDOWN);
        click(MOUSEEVENTF_LEFTUP);
        pause(200);
        drag((250, 250), (449, 399));
        let out = t.join().unwrap().expect("an outcome");
        assert_eq!(out.rect, Rect::new(250, 250, 200, 150));
        assert_eq!(out.objects.len(), 1, "the arrow is handed over as an editable object");
        match &out.objects[0].shape {
            Shape::Arrow { a, b } => {
                assert_eq!((a.x, a.y, b.x, b.y), (200.0, 200.0, 300.0, 260.0));
            }
            other => panic!("expected an arrow, got {other:?}"),
        }
        assert_eq!(out.tool, Tool::Region);
    }

    #[test]
    #[ignore = "needs an interactive desktop"]
    fn a_numbered_step_and_undo() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        sys::enable_dpi_awareness();
        let origin = (100, 100);
        let t = std::thread::spawn(move || run(base_request(origin, Tool::Step)));
        pause(900);
        for p in [(250, 300), (320, 320)] {
            move_to(p.0, p.1);
            pause(120);
            click(MOUSEEVENTF_LEFTDOWN);
            click(MOUSEEVENTF_LEFTUP);
            pause(150);
        }
        // undo the second badge through the toolbar button
        let (ux, uy) = button_center(origin, Btn::Undo);
        move_to(ux, uy);
        pause(150);
        click(MOUSEEVENTF_LEFTDOWN);
        click(MOUSEEVENTF_LEFTUP);
        pause(200);
        let (cx, cy) = button_center(origin, Btn::Tool(Tool::Region));
        move_to(cx, cy);
        pause(150);
        click(MOUSEEVENTF_LEFTDOWN);
        click(MOUSEEVENTF_LEFTUP);
        pause(200);
        drag((200, 200), (399, 399));
        let out = t.join().unwrap().expect("an outcome");
        assert_eq!(out.objects.len(), 1);
        assert!(matches!(out.objects[0].shape, Shape::Step { n: 1, .. }));
    }

    #[test]
    #[ignore = "needs an interactive desktop"]
    fn the_cancel_button_closes_the_overlay() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        sys::enable_dpi_awareness();
        let origin = (100, 100);
        let t = std::thread::spawn(move || run(base_request(origin, Tool::Region)));
        pause(900);
        let (x, y) = button_center(origin, Btn::Cancel);
        move_to(x, y);
        pause(150);
        click(MOUSEEVENTF_LEFTDOWN);
        click(MOUSEEVENTF_LEFTUP);
        assert!(t.join().unwrap().is_none());
    }
}
