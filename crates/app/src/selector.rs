//! Full-screen capture overlay: pick a region, a window or a monitor — and annotate the screen first.
//!
//! A plain Win32 window over the whole virtual desktop showing a *frozen* screenshot, dimmed, with the selection cut
//! out in full brightness. A toolbar at the top (like ShareX's) offers the capture modes and drawing tools (rectangle,
//! ellipse, line, arrow, pencil, marker, text, numbered steps, blur, pixelate, spotlight); what you draw stays an
//! editable vector object (move it, stretch it by its handles, recolour it, erase it) that is handed to the editor
//! together with the chosen area. A single click without dragging captures the whole screen (configurable). Because the process is per-monitor DPI aware,
//! window and cursor coordinates are physical pixels, so the selection is pixel-exact on every monitor. A magnifier
//! next to the cursor shows the pixels under it; the arrow keys move the cursor by one pixel (Shift: ten).

use std::collections::HashMap;
use std::ffi::c_void;

use snapforge_core::RgbaImage;
use snapforge_core::capture::sys::WindowInfo;
use snapforge_core::editor::{Editor, Object, Pt, Shape};
use snapforge_core::geometry::Rect;
use snapforge_core::settings::{OverlayClick, OverlayTool as Tool};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_ESCAPE, VK_LEFT,
    VK_RETURN, VK_RIGHT, VK_SHIFT, VK_UP,
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
    /// What the "screen" tool captures instead of the monitor under the cursor (virtual-desktop pixels): the whole
    /// desktop or one chosen monitor.
    pub screen_target: Option<Rect>,
    /// Only pick an area (screen recording): the toolbar offers just the capture modes.
    pub capture_only: bool,
    pub opts: OverlayOpts,
}

/// The user's choices for the overlay (settings) and the texts that go with its extra buttons.
#[derive(Clone)]
pub struct OverlayOpts {
    /// Tools left off the toolbar (their keys still work).
    pub hidden: Vec<Tool>,
    pub palette: Vec<[u8; 4]>,
    pub fill: bool,
    pub click: OverlayClick,
    pub bottom: bool,
    /// Darkening of the part of the screen that is not selected, in percent.
    pub dim: u8,
    pub magnifier: bool,
    pub redo_label: String,
    pub fill_label: String,
    pub select_hint: String,
    pub eraser_hint: String,
    pub picker_hint: String,
}

impl Default for OverlayOpts {
    fn default() -> OverlayOpts {
        OverlayOpts {
            hidden: Vec::new(),
            palette: snapforge_core::settings::default_overlay_palette(),
            fill: false,
            click: OverlayClick::Monitor,
            bottom: false,
            dim: 60,
            magnifier: true,
            redo_label: "Redo".into(),
            fill_label: "Fill".into(),
            select_hint: String::new(),
            eraser_hint: String::new(),
            picker_hint: String::new(),
        }
    }
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
    pub fill: bool,
}

const MAG_SRC: i32 = 17; // source pixels per side (odd: one centre pixel)
const MAG_ZOOM: i32 = 9;
const ACCENT: COLORREF = COLORREF(0x00FA_A560); // BGR of (96,165,250)
const BTN: i32 = 34;
const GAP: i32 = 10;
const ROW_GAP: i32 = 4;
const BAR_PAD: i32 = 8;
/// Reach (in pixels) of a click on a drawn object or one of its handles.
const PICK_TOL: f32 = 6.0;
const HANDLE_R: f32 = 9.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Btn {
    Tool(Tool),
    Color(usize),
    /// Shows the current drawing colour (a picked one may not be in the palette).
    Swatch,
    WidthDown,
    WidthUp,
    Fill,
    Privacy,
    Undo,
    Redo,
    Cancel,
}

#[derive(Clone, Copy, Debug)]
pub struct Button {
    pub rect: Rect,
    pub kind: Btn,
}

/// Lays the toolbar out centred at the top of `monitor` (client coordinates). Returns the bar rectangle and buttons.
#[cfg(test)]
pub fn toolbar_layout(monitor: Rect) -> (Rect, Vec<Button>) {
    toolbar_layout_opts(monitor, false, &[], 6, false)
}

fn tool_group(t: Tool) -> u8 {
    match t {
        Tool::Region | Tool::Window | Tool::Screen | Tool::Ocr => 0,
        Tool::Select => 1,
        Tool::Spotlight | Tool::Eraser | Tool::Picker => 3,
        _ => 2,
    }
}

type Item = (Option<Btn>, i32); // (button, width); no button = a gap

fn row_width(row: &[Item]) -> i32 {
    row.iter().map(|(_, w)| *w).sum::<i32>() + row.iter().filter(|(b, _)| b.is_some()).count() as i32 * 2
}

/// Breaks a row that is wider than `limit` into several, between two buttons.
fn wrap_row(items: Vec<Item>, limit: i32) -> Vec<Vec<Item>> {
    let mut rows: Vec<Vec<Item>> = vec![Vec::new()];
    for it in items {
        let cur = rows.last_mut().expect("one row at least");
        let used = row_width(cur);
        let need = it.1 + if it.0.is_some() { 2 } else { 0 };
        if used + need > limit && cur.iter().any(|(b, _)| b.is_some()) {
            if it.0.is_none() {
                continue; // a gap never starts a row
            }
            rows.push(vec![it]);
        } else if !(it.0.is_none() && cur.is_empty()) {
            cur.push(it);
        }
    }
    for r in &mut rows {
        while r.last().is_some_and(|(b, _)| b.is_none()) {
            r.pop();
        }
    }
    rows.retain(|r| !r.is_empty());
    rows
}

/// `capture_only` leaves just the area/window/monitor modes and the cancel button (used when recording a video).
/// `hidden` tools are left off, `colors` is the number of palette buttons, `bottom` puts the bar at the bottom of
/// the monitor. Rows that would not fit the monitor wrap.
pub fn toolbar_layout_opts(
    monitor: Rect,
    capture_only: bool,
    hidden: &[Tool],
    colors: usize,
    bottom: bool,
) -> (Rect, Vec<Button>) {
    let limit = (monitor.w - 40).max(280);
    let mut tools: Vec<Item> = Vec::new();
    let mut prev: Option<Tool> = None;
    for t in Tool::ALL {
        let usable = !capture_only || matches!(t, Tool::Region | Tool::Window | Tool::Screen);
        if !usable || hidden.contains(&t) {
            continue;
        }
        if prev.is_some_and(|p| tool_group(p) != tool_group(t)) {
            tools.push((None, GAP));
        }
        tools.push((Some(Btn::Tool(t)), BTN));
        prev = Some(t);
    }
    let mut rows: Vec<Vec<Item>>;
    if capture_only {
        tools.push((None, GAP));
        tools.push((Some(Btn::Cancel), BTN));
        rows = wrap_row(tools, limit);
    } else {
        rows = wrap_row(tools, limit);
        let mut opt: Vec<Item> = vec![(Some(Btn::Swatch), 26), (None, 4)];
        for i in 0..colors {
            opt.push((Some(Btn::Color(i)), 24));
        }
        opt.push((None, GAP));
        opt.push((Some(Btn::WidthDown), 26));
        opt.push((None, 30)); // the number
        opt.push((Some(Btn::WidthUp), 26));
        opt.push((None, GAP));
        opt.push((Some(Btn::Fill), BTN));
        opt.push((Some(Btn::Privacy), BTN));
        opt.push((None, GAP));
        opt.push((Some(Btn::Undo), BTN));
        opt.push((Some(Btn::Redo), BTN));
        opt.push((Some(Btn::Cancel), BTN));
        rows.extend(wrap_row(opt, limit));
    }
    let widest = rows.iter().map(|r| row_width(r)).max().unwrap_or(0);
    let total = widest + BAR_PAD * 2;
    let n = rows.len() as i32;
    let height = n * BTN + (n - 1) * ROW_GAP + BAR_PAD * 2;
    let bar = Rect::new(
        (monitor.x + (monitor.w - total) / 2).max(monitor.x),
        if bottom {
            monitor.bottom() - height - 10
        } else {
            monitor.y + 10
        },
        total,
        height,
    );
    let mut out = Vec::new();
    for (ri, row) in rows.iter().enumerate() {
        let mut x = bar.x + (bar.w - row_width(row)) / 2;
        let y = bar.y + BAR_PAD + ri as i32 * (BTN + ROW_GAP);
        for (b, w) in row {
            if let Some(kind) = b {
                let size = match kind {
                    Btn::Color(_) => 20,
                    Btn::Swatch => 22,
                    Btn::WidthDown | Btn::WidthUp => 24,
                    _ => BTN,
                };
                out.push(Button {
                    rect: Rect::new(x + (w - size) / 2, y + (BTN - size) / 2, size, size),
                    kind: *kind,
                });
                x += w + 2;
            } else {
                x += w;
            }
        }
    }
    (bar, out)
}

/// Keeps a drag square (rectangles and ellipses) or on a multiple of 45 degrees (lines and arrows) while Shift is held.
fn constrain(tool: Tool, start: (i32, i32), cur: (i32, i32)) -> (i32, i32) {
    let (dx, dy) = (cur.0 - start.0, cur.1 - start.1);
    match tool {
        Tool::Rect | Tool::Ellipse | Tool::Blur | Tool::Pixelate | Tool::Spotlight => {
            let m = dx.abs().max(dy.abs());
            (start.0 + m * dx.signum(), start.1 + m * dy.signum())
        }
        Tool::Line | Tool::Arrow => {
            let (ax, ay) = (dx.abs() as f32, dy.abs() as f32);
            if ay < ax * 0.4142 {
                (cur.0, start.1)
            } else if ax < ay * 0.4142 {
                (start.0, cur.1)
            } else {
                let m = ((ax + ay) / 2.0).round() as i32;
                (start.0 + m * dx.signum(), start.1 + m * dy.signum())
            }
        }
        _ => cur,
    }
}

struct Drag {
    start: (i32, i32),
    pts: Vec<Pt>,
}

struct TextEntry {
    at: (i32, i32),
    buf: String,
}

/// A mouse edit of the selected object: moving it, or dragging one of its handles.
struct Edit {
    /// The shape as it was when the drag began (every step is computed from it).
    orig: Shape,
    from: Pt,
    handle: Option<usize>,
    changed: bool,
}

fn shift_down() -> bool {
    // SAFETY: reads the keyboard state only.
    unsafe { GetKeyState(VK_SHIFT.0 as i32) < 0 }
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
    /// The monitor (client coordinates) the toolbar currently sits on.
    bar_monitor: Rect,
    buttons: Vec<Button>,
    screen_target: Option<Rect>,
    capture_only: bool,
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
    palette: Vec<[u8; 4]>,
    hidden: Vec<Tool>,
    fill: bool,
    click: OverlayClick,
    bottom: bool,
    dim: u8,
    magnifier: bool,
    redo_label: String,
    fill_label: String,
    select_hint: String,
    eraser_hint: String,
    picker_hint: String,
    /// The object the handles belong to (hidden while a capture tool is active).
    selected: Option<u64>,
    edit: Option<Edit>,
    /// The eraser is held down.
    erasing: bool,
    erased_any: bool,
    /// Mouse position while the toolbar itself is being dragged; once moved by hand it stays where it was put.
    bar_drag: Option<(i32, i32)>,
    bar_pinned: bool,
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
            fill: st.fill,
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
            let o = &req.opts;
            let (bar, buttons) = toolbar_layout_opts(monitor, req.capture_only, &o.hidden, o.palette.len(), o.bottom);
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
                bar_monitor: monitor,
                buttons,
                screen_target: req.screen_target.map(|r| r.translate(-req.origin.0, -req.origin.1)),
                capture_only: req.capture_only,
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
                palette: if o.palette.is_empty() {
                    snapforge_core::settings::default_overlay_palette()
                } else {
                    o.palette.clone()
                },
                hidden: o.hidden.clone(),
                fill: o.fill,
                click: o.click,
                bottom: o.bottom,
                dim: o.dim.min(90),
                magnifier: o.magnifier,
                redo_label: o.redo_label.clone(),
                fill_label: o.fill_label.clone(),
                select_hint: o.select_hint.clone(),
                eraser_hint: o.eraser_hint.clone(),
                picker_hint: o.picker_hint.clone(),
                selected: None,
                edit: None,
                erasing: false,
                erased_any: false,
                bar_drag: None,
                bar_pinned: false,
            };
            ReleaseDC(None, screen);
            st.write_pixels(&req.frozen);
            st
        }
    }

    fn full(&self) -> Rect {
        Rect::new(0, 0, self.w, self.h)
    }

    /// Copies `img` into the bright DIB and a darkened copy into the dim DIB.
    fn write_pixels(&mut self, img: &RgbaImage) {
        let full = self.full();
        self.write_pixels_rect(img, full);
    }

    /// Like `write_pixels`, for the part of the picture inside `r` only (moving an object touches a small area).
    fn write_pixels_rect(&mut self, img: &RgbaImage, r: Rect) {
        if self.bright_bits.is_null() || self.dim_bits.is_null() {
            return;
        }
        let Some(r) = r.intersect(&self.full()) else { return };
        let n = (self.w as usize) * (self.h as usize) * 4;
        // SAFETY: both DIB sections were created with w*h BGRA pixels.
        let (bright, dim) = unsafe {
            (
                std::slice::from_raw_parts_mut(self.bright_bits, n),
                std::slice::from_raw_parts_mut(self.dim_bits, n),
            )
        };
        let keep = 100 - self.dim.min(90) as u32;
        let raw = img.as_raw();
        for y in r.y..r.bottom() {
            let off = (y as usize * self.w as usize + r.x as usize) * 4;
            let end = off + r.w as usize * 4;
            if end > raw.len() || end > n {
                return;
            }
            for ((src, b), d) in raw[off..end]
                .chunks_exact(4)
                .zip(bright[off..end].chunks_exact_mut(4))
                .zip(dim[off..end].chunks_exact_mut(4))
            {
                b.copy_from_slice(&[src[2], src[1], src[0], 255]);
                d.copy_from_slice(&[
                    (src[2] as u32 * keep / 100) as u8,
                    (src[1] as u32 * keep / 100) as u8,
                    (src[0] as u32 * keep / 100) as u8,
                    255,
                ]);
            }
        }
    }

    /// Re-renders the base picture with all annotations into the DIBs.
    fn refresh(&mut self) {
        let img = self.ed.doc.render();
        self.write_pixels(&img);
    }

    /// Re-renders, but copies only the area `r` into the DIBs.
    fn refresh_rect(&mut self, r: Rect) {
        let img = self.ed.doc.render();
        self.write_pixels_rect(&img, r);
    }

    /// The part of the picture a change to `o` can touch: its box with room for arrow heads and the like (a
    /// spotlight dims everything, so it is the whole picture).
    fn dirty_rect(&self, o: &Object) -> Rect {
        if matches!(o.shape, Shape::Spotlight { .. } | Shape::Chart { .. }) {
            return self.full();
        }
        let (lo, hi) = o.shape.bounds(o.width);
        let m = (o.width * 8.0).max(40.0);
        Rect::new(
            (lo.x - m) as i32,
            (lo.y - m) as i32,
            (hi.x - lo.x + 2.0 * m) as i32 + 1,
            (hi.y - lo.y + 2.0 * m) as i32 + 1,
        )
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
        if self.screen_target.is_some() {
            return self.screen_target;
        }
        self.monitors
            .iter()
            .copied()
            .find(|m| m.contains(self.cur.0, self.cur.1))
    }

    /// Moves the toolbar to the monitor under the cursor (only while nothing is being drawn or typed).
    fn follow_cursor(&mut self) {
        if self.bar_pinned
            || self.bar_drag.is_some()
            || self.draw.is_some()
            || self.region_drag.is_some()
            || self.text.is_some()
            || self.anchor.is_some()
            || self.edit.is_some()
            || self.erasing
        {
            return;
        }
        let Some(m) = self
            .monitors
            .iter()
            .copied()
            .find(|m| m.contains(self.cur.0, self.cur.1))
        else {
            return;
        };
        if m != self.bar_monitor {
            let (bar, buttons) =
                toolbar_layout_opts(m, self.capture_only, &self.hidden, self.palette.len(), self.bottom);
            self.bar_monitor = m;
            self.bar = bar;
            self.buttons = buttons;
        }
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
            (Tool::Window, _) if !self.over_bar(self.cur) => self.hovered_window().or_else(|| self.fallback_monitor()),
            (Tool::Screen, _) if !self.over_bar(self.cur) => self.hovered_monitor(),
            _ => None,
        }
        .and_then(|r| r.intersect(&Rect::new(0, 0, self.w, self.h)))
    }

    fn text_size(&self) -> f32 {
        14.0 + self.width * 3.0
    }

    fn pt(&self) -> Pt {
        Pt::new(self.cur.0 as f32, self.cur.1 as f32)
    }

    /// The monitor a click captures when no window is under the cursor (nothing when clicks are switched off).
    fn fallback_monitor(&self) -> Option<Rect> {
        if self.click == OverlayClick::Nothing {
            return None;
        }
        self.hovered_monitor()
    }

    /// What a single click without dragging captures, per the settings.
    fn click_capture(&self) -> Option<Rect> {
        match self.click {
            OverlayClick::Nothing => None,
            OverlayClick::AllScreens => Some(self.full()),
            OverlayClick::Monitor => self.hovered_monitor(),
            OverlayClick::Window => self.hovered_window().or_else(|| self.hovered_monitor()),
        }
        .and_then(|r| r.intersect(&self.full()))
    }

    // ---------------------------------------------------------------------------------- drawn objects

    /// The selected object, while a drawing tool is active (a capture tool hides the selection).
    fn live_selection(&self) -> Option<&Object> {
        if self.tool.is_capture() {
            return None;
        }
        self.selected.and_then(|id| self.ed.doc.get(id))
    }

    fn object_at(&self, p: (i32, i32)) -> Option<u64> {
        self.ed.doc.hit_test(Pt::new(p.0 as f32, p.1 as f32), PICK_TOL)
    }

    /// The handle of the selected object under `p` (the nearest one when several are in reach).
    fn handle_at(&self, p: (i32, i32)) -> Option<usize> {
        let o = self.live_selection()?;
        let q = Pt::new(p.0 as f32, p.1 as f32);
        o.shape
            .edit_handles()
            .iter()
            .enumerate()
            .map(|(i, h)| (i, (h.x - q.x).hypot(h.y - q.y)))
            .filter(|(_, d)| *d <= HANDLE_R)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// Starts moving (`handle` = None) or stretching the selected object.
    fn start_edit(&mut self, handle: Option<usize>) -> bool {
        let Some(o) = self.live_selection() else { return false };
        let orig = o.shape.clone();
        self.edit = Some(Edit {
            orig,
            from: self.pt(),
            handle,
            changed: false,
        });
        self.ed.checkpoint();
        true
    }

    fn apply_edit(&mut self) {
        let (Some(edit), Some(id)) = (&self.edit, self.selected) else {
            return;
        };
        let cur = self.pt();
        let new = match edit.handle {
            Some(i) => edit.orig.resized(i, cur),
            None => {
                let mut sh = edit.orig.clone();
                sh.translate(cur.x - edit.from.x, cur.y - edit.from.y);
                sh
            }
        };
        let Some(o) = self.ed.doc.get(id) else { return };
        if o.shape == new {
            return;
        }
        let before = self.dirty_rect(o);
        if let Some(o) = self.ed.doc.get_mut(id) {
            o.shape = new;
        }
        if let Some(e) = &mut self.edit {
            e.changed = true;
        }
        if let Some(o) = self.ed.doc.get(id) {
            let r = before.union(&self.dirty_rect(o));
            self.refresh_rect(r);
        }
    }

    /// The mouse was released: keep the change, or drop the undo step of a click that moved nothing.
    fn finish_edit(&mut self) {
        if let Some(e) = self.edit.take()
            && !e.changed
        {
            self.ed.cancel_checkpoint();
        }
    }

    /// Right click / Esc during an edit: puts the object back as it was.
    fn cancel_edit(&mut self) {
        let Some(e) = self.edit.take() else { return };
        self.ed.cancel_checkpoint();
        if let Some(id) = self.selected
            && let Some(o) = self.ed.doc.get_mut(id)
        {
            o.shape = e.orig;
        }
        self.refresh();
    }

    /// Selects the object under the cursor (or clears the selection); returns whether there is one.
    fn select_at_cursor(&mut self) -> bool {
        self.selected = self.object_at(self.cur);
        self.selected.is_some()
    }

    /// Removes the topmost object within reach of the cursor.
    fn erase_at_cursor(&mut self) {
        let tol = (self.width * 1.5).max(8.0);
        let Some(id) = self.ed.doc.hit_test(self.pt(), tol) else {
            return;
        };
        let Some(o) = self.ed.doc.get(id) else { return };
        let r = self.dirty_rect(o);
        self.ed.doc.remove(id);
        if self.selected == Some(id) {
            self.selected = None;
        }
        self.erased_any = true;
        self.refresh_rect(r);
    }

    fn finish_erase(&mut self) {
        if self.erasing && !self.erased_any {
            self.ed.cancel_checkpoint();
        }
        self.erasing = false;
        self.erased_any = false;
    }

    fn delete_selected(&mut self) -> bool {
        let Some(o) = self.live_selection() else { return false };
        let (id, r) = (o.id, self.dirty_rect(o));
        self.ed.checkpoint();
        self.ed.doc.remove(id);
        self.selected = None;
        self.edit = None;
        self.refresh_rect(r);
        true
    }

    /// Applies `f` to the selected object as one undo step.
    fn edit_selected(&mut self, f: impl FnOnce(&mut Object)) {
        let Some(o) = self.live_selection() else { return };
        let (id, before) = (o.id, self.dirty_rect(o));
        self.ed.checkpoint();
        if let Some(o) = self.ed.doc.get_mut(id) {
            f(o);
        }
        if let Some(o) = self.ed.doc.get(id) {
            let r = before.union(&self.dirty_rect(o));
            self.refresh_rect(r);
        }
    }

    fn set_color(&mut self, c: [u8; 4]) {
        self.color = c;
        self.edit_selected(|o| o.color = c);
    }

    fn set_width(&mut self, w: f32) {
        let w = w.clamp(1.0, 24.0);
        if w == self.width {
            return;
        }
        self.width = w;
        self.edit_selected(|o| {
            o.width = w;
            if let Shape::Text { size, .. } = &mut o.shape {
                *size = 14.0 + w * 3.0;
            }
        });
    }

    fn toggle_fill(&mut self) {
        self.fill = !self.fill;
        let fill = self.fill;
        if self
            .live_selection()
            .is_some_and(|o| matches!(o.shape, Shape::Rect { .. } | Shape::Ellipse { .. }))
        {
            self.edit_selected(|o| {
                if let Shape::Rect { fill: f, .. } | Shape::Ellipse { fill: f, .. } = &mut o.shape {
                    *f = fill;
                }
            });
        }
    }

    /// The picker tool: the drawing colour becomes the colour of the pixel under the cursor.
    fn pick_color(&mut self) {
        let (r, g, b) = self.pixel(self.cur.0, self.cur.1);
        self.set_color([r, g, b, 255]);
    }

    /// Moves the whole toolbar by hand (drag on its background); it stays there afterwards.
    fn drag_bar(&mut self, to: (i32, i32)) {
        let Some(from) = self.bar_drag else { return };
        let (mut dx, mut dy) = (to.0 - from.0, to.1 - from.1);
        // keep the bar on the desktop
        dx = dx.clamp(-self.bar.x, self.w - self.bar.right());
        dy = dy.clamp(-self.bar.y, self.h - self.bar.bottom());
        if dx != 0 || dy != 0 {
            self.bar = self.bar.translate(dx, dy);
            for b in &mut self.buttons {
                b.rect = b.rect.translate(dx, dy);
            }
            self.bar_pinned = true;
        }
        self.bar_drag = Some((from.0 + dx, from.1 + dy));
    }

    fn add_object(&mut self, shape: Shape) {
        self.ed.checkpoint();
        let id = self.ed.doc.add(shape, self.color, self.width);
        // a new object is selected at once: its handles can be dragged right away
        self.selected = Some(id);
        if let Some(o) = self.ed.doc.get(id) {
            let r = self.dirty_rect(o);
            self.refresh_rect(r);
        }
    }

    /// Adds the shape of the finished mouse drag (if it is big enough to be intentional).
    fn finish_draw(&mut self) {
        let Some(d) = self.draw.take() else { return };
        let end = if shift_down() {
            constrain(self.tool, d.start, self.cur)
        } else {
            self.cur
        };
        let (a, b) = (
            Pt::new(d.start.0 as f32, d.start.1 as f32),
            Pt::new(end.0 as f32, end.1 as f32),
        );
        let long = (a.x - b.x).hypot(a.y - b.y) >= 3.0;
        let boxy = (a.x - b.x).abs() >= 3.0 && (a.y - b.y).abs() >= 3.0;
        let fill = self.fill;
        let shape = match self.tool {
            Tool::Rect if boxy => Some(Shape::Rect { a, b, fill }),
            Tool::Ellipse if boxy => Some(Shape::Ellipse { a, b, fill }),
            Tool::Line if long => Some(Shape::Line { a, b }),
            Tool::Arrow if long => Some(Shape::Arrow { a, b }),
            Tool::Blur if boxy => Some(Shape::Blur { a, b, radius: 10 }),
            Tool::Pixelate if boxy => Some(Shape::Pixelate { a, b, block: 10 }),
            Tool::Spotlight if boxy => Some(Shape::Spotlight { a, b }),
            Tool::Pencil if d.pts.len() > 1 => Some(Shape::Pencil { pts: d.pts }),
            Tool::Marker if d.pts.len() > 1 => Some(Shape::Marker { pts: d.pts }),
            _ => None,
        };
        if let Some(sh) = shape {
            self.add_object(sh);
        }
    }

    fn commit_text(&mut self) {
        let Some(t) = self.text.take() else { return };
        if !t.buf.trim().is_empty() {
            let size = self.text_size();
            self.add_object(Shape::Text {
                at: Pt::new(t.at.0 as f32, t.at.1 as f32),
                text: t.buf,
                size,
            });
        }
    }

    fn add_step(&mut self, at: (i32, i32)) {
        let n = self.ed.doc.next_step_number();
        self.add_object(Shape::Step {
            at: Pt::new(at.0 as f32, at.1 as f32),
            n,
        });
    }

    fn undo(&mut self) {
        if self.ed.undo() {
            self.after_history();
        }
    }

    fn redo(&mut self) {
        if self.ed.redo() {
            self.after_history();
        }
    }

    fn after_history(&mut self) {
        self.edit = None;
        if self.selected.is_some_and(|id| self.ed.doc.get(id).is_none()) {
            self.selected = None;
        }
        self.refresh();
    }

    fn press_button(&mut self, b: Button) -> Action {
        self.commit_text();
        match b.kind {
            Btn::Tool(t) => {
                self.tool = t;
                self.anchor = None;
            }
            Btn::Color(i) => {
                if let Some(c) = self.palette.get(i).copied() {
                    self.set_color(c);
                }
            }
            Btn::Swatch => {}
            Btn::WidthDown => self.set_width(self.width - 1.0),
            Btn::WidthUp => self.set_width(self.width + 1.0),
            Btn::Fill => self.toggle_fill(),
            Btn::Privacy => self.privacy = !self.privacy,
            Btn::Undo => self.undo(),
            Btn::Redo => self.redo(),
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
                let ly = if self.bottom {
                    (r.y - 30).max(4)
                } else {
                    (r.y - 30).max(self.bar.bottom() + 4)
                };
                self.label(back, &format!("{} × {}", r.w, r.h), r.x, ly);
            }
            self.paint_drawing(back);
            self.paint_toolbar(back);
            if self.magnifier && (self.tool.is_capture() || self.tool == Tool::Picker) && !self.over_bar(self.cur) {
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
                let s = d.start;
                let c = if shift_down() {
                    constrain(self.tool, s, self.cur)
                } else {
                    self.cur
                };
                let w = (self.width.round() as i32).max(1);
                let (style, width, col) = match self.tool {
                    Tool::Blur | Tool::Pixelate | Tool::Spotlight => (PS_DOT, 1, ACCENT),
                    Tool::Marker => (PS_SOLID, w * 5, rgba(self.color)),
                    _ => (PS_SOLID, w, rgba(self.color)),
                };
                let pen = CreatePen(style, width, col);
                let old_pen = SelectObject(dc, pen.into());
                let fill_brush = (self.fill && matches!(self.tool, Tool::Rect | Tool::Ellipse))
                    .then(|| CreateSolidBrush(rgba(self.color)));
                let old_brush = SelectObject(dc, fill_brush.map_or(GetStockObject(NULL_BRUSH), |b| b.into()));
                match self.tool {
                    Tool::Rect | Tool::Blur | Tool::Pixelate | Tool::Spotlight => {
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
                if let Some(b) = fill_brush {
                    let _ = DeleteObject(b.into());
                }
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
            self.paint_edit_marks(dc);
        }
    }

    /// The outline and handles of the selected object, the outline of the one the select tool or the eraser is
    /// about to take, and the eraser's circle.
    ///
    /// # Safety
    /// `dc` must be a valid DC.
    unsafe fn paint_edit_marks(&self, dc: HDC) {
        // SAFETY: valid DC; every pen/brush created here is deleted before returning.
        unsafe {
            let outline = |o: &Object, style: PEN_STYLE, col: COLORREF| {
                let (lo, hi) = o.shape.bounds(o.width);
                let pen = CreatePen(style, 1, col);
                let (op, ob) = (
                    SelectObject(dc, pen.into()),
                    SelectObject(dc, GetStockObject(NULL_BRUSH)),
                );
                let _ = Rectangle(dc, lo.x as i32 - 3, lo.y as i32 - 3, hi.x as i32 + 4, hi.y as i32 + 4);
                SelectObject(dc, ob);
                SelectObject(dc, op);
                let _ = DeleteObject(pen.into());
            };
            let idle = self.edit.is_none() && !self.over_bar(self.cur) && self.draw.is_none();
            if idle && matches!(self.tool, Tool::Select | Tool::Eraser) {
                let hit = if self.tool == Tool::Eraser {
                    self.ed.doc.hit_test(self.pt(), (self.width * 1.5).max(8.0))
                } else {
                    self.object_at(self.cur)
                };
                if let Some(o) = hit.and_then(|id| self.ed.doc.get(id)) {
                    let col = if self.tool == Tool::Eraser {
                        rgb(235, 90, 90)
                    } else {
                        ACCENT
                    };
                    outline(o, PS_DOT, col);
                }
            }
            if let Some(o) = self.live_selection() {
                outline(o, PS_DASH, ACCENT);
                let pen = CreatePen(PS_SOLID, 2, ACCENT);
                let br = CreateSolidBrush(rgb(255, 255, 255));
                let (op, ob) = (SelectObject(dc, pen.into()), SelectObject(dc, br.into()));
                for h in o.shape.edit_handles() {
                    let (x, y) = (h.x as i32, h.y as i32);
                    let _ = Rectangle(dc, x - 4, y - 4, x + 5, y + 5);
                }
                SelectObject(dc, ob);
                SelectObject(dc, op);
                let _ = DeleteObject(pen.into());
                let _ = DeleteObject(br.into());
            }
            if self.tool == Tool::Eraser && !self.over_bar(self.cur) {
                let r = (self.width * 1.5).max(8.0) as i32;
                let pen = CreatePen(PS_SOLID, 2, rgb(255, 255, 255));
                let (op, ob) = (
                    SelectObject(dc, pen.into()),
                    SelectObject(dc, GetStockObject(NULL_BRUSH)),
                );
                let _ = Ellipse(dc, self.cur.0 - r, self.cur.1 - r, self.cur.0 + r, self.cur.1 + r);
                SelectObject(dc, ob);
                SelectObject(dc, op);
                let _ = DeleteObject(pen.into());
            }
        }
    }

    /// The line under the toolbar: what the current tool does.
    fn tool_hint(&self) -> String {
        let pick = |own: &String| {
            if own.is_empty() {
                String::from_utf16_lossy(&self.hint)
            } else {
                own.clone()
            }
        };
        match self.tool {
            Tool::Select => pick(&self.select_hint),
            Tool::Eraser => pick(&self.eraser_hint),
            Tool::Picker => pick(&self.picker_hint),
            _ => String::from_utf16_lossy(&self.hint),
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
                    Btn::Color(i) => self.palette.get(i) == Some(&self.color),
                    Btn::Privacy => self.privacy,
                    Btn::Fill => self.fill,
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
                let _ = TextOutW(dc, cx - sz.cx / 2, down.rect.y + (down.rect.h - sz.cy) / 2, &t);
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
                Some(Btn::Fill) => self.fill_label.clone(),
                Some(Btn::Redo) => format!("{}  [Ctrl+Y]", self.redo_label),
                Some(Btn::Swatch) => format!("#{:02X}{:02X}{:02X}", self.color[0], self.color[1], self.color[2]),
                Some(Btn::Color(i)) => self
                    .palette
                    .get(i)
                    .map(|c| format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]))
                    .unwrap_or_default(),
                Some(Btn::Undo) => format!("{}  [Ctrl+Z]", self.undo_label),
                Some(Btn::Cancel) => format!("{}  [Esc]", self.cancel_label),
                Some(Btn::WidthDown) => "[".to_owned(),
                Some(Btn::WidthUp) => "]".to_owned(),
                _ => self.tool_hint(),
            };
            if !text.is_empty() {
                let old = SelectObject(dc, self.font.into());
                let t = utf16(&text);
                let mut sz = windows::Win32::Foundation::SIZE::default();
                let _ = GetTextExtentPoint32W(dc, &t, &mut sz);
                SelectObject(dc, old);
                let y = if self.bottom {
                    bar.y - 8 - (sz.cy + 6)
                } else {
                    bar.bottom() + 8
                };
                self.label(dc, &text, (bar.x + (bar.w - sz.cx - 14) / 2).max(4), y);
            }
        }
    }

    /// # Safety
    /// `dc` must be a valid DC.
    unsafe fn paint_button(&self, dc: HDC, b: &Button, active: bool, hot: bool) {
        // SAFETY: valid DC; GDI objects are released.
        unsafe {
            let r = b.rect;
            if matches!(b.kind, Btn::Color(_) | Btn::Swatch) {
                let (fill, ring, ring_w) = match b.kind {
                    Btn::Color(i) => (
                        self.palette.get(i).copied().unwrap_or([0, 0, 0, 255]),
                        if active { ACCENT } else { rgb(110, 114, 130) },
                        if active { 3 } else { 1 },
                    ),
                    _ => (self.color, rgb(235, 238, 248), 2),
                };
                let br = CreateSolidBrush(rgba(fill));
                let pen = CreatePen(PS_SOLID, ring_w, ring);
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
                Btn::Tool(Tool::Select) => {
                    // the pointer arrow
                    let pts = [
                        (x + 2, y),
                        (x + 2, y + h - 2),
                        (x + 6, y + h - 6),
                        (x + 10, y + h),
                        (x + 13, y + h - 2),
                        (x + 9, y + h - 8),
                        (x + 14, y + h - 8),
                        (x + 2, y),
                    ];
                    for p in pts.windows(2) {
                        line(p[0].0, p[0].1, p[1].0, p[1].1);
                    }
                }
                Btn::Tool(Tool::Spotlight) => {
                    let _ = Rectangle(dc, x - 1, y, x + w + 1, y + h);
                    let _ = Rectangle(dc, x + 4, y + 4, x + w - 4, y + h - 4);
                }
                Btn::Tool(Tool::Eraser) => {
                    // a tilted block of rubber
                    let pts = [
                        (x + 1, y + h - 5),
                        (x + 9, y + 1),
                        (x + w, y + 8),
                        (x + w - 8, y + h - 1),
                        (x + 1, y + h - 5),
                    ];
                    for p in pts.windows(2) {
                        line(p[0].0, p[0].1, p[1].0, p[1].1);
                    }
                    line(x + 7, y + h - 2, x + w - 4, y + 3);
                }
                Btn::Tool(Tool::Picker) => {
                    // a pipette
                    line(x + 1, y + h - 1, x + w - 5, y + 5);
                    let _ = Ellipse(dc, x + w - 8, y, x + w, y + 8);
                    line(x + 1, y + h - 1, x + 3, y + h - 4);
                }
                Btn::Fill => {
                    if self.fill {
                        let br = CreateSolidBrush(rgba(self.color));
                        let ob = SelectObject(dc, br.into());
                        let _ = Rectangle(dc, x + 1, y + 2, x + w - 1, y + h - 2);
                        SelectObject(dc, ob);
                        let _ = DeleteObject(br.into());
                    } else {
                        let _ = Rectangle(dc, x + 1, y + 2, x + w - 1, y + h - 2);
                        line(x + 5, y + h - 6, x + w - 5, y + 6);
                    }
                }
                Btn::WidthDown | Btn::WidthUp => {
                    line(x + 1, y + h / 2, x + w - 1, y + h / 2);
                    if b.kind == Btn::WidthUp {
                        line(x + w / 2, y + 1, x + w / 2, y + h - 1);
                    }
                }
                Btn::Undo | Btn::Redo | Btn::Cancel => {
                    let old = SelectObject(dc, self.symbols.into());
                    SetBkMode(dc, TRANSPARENT);
                    SetTextColor(dc, if b.kind == Btn::Cancel { rgb(235, 110, 110) } else { ink });
                    let glyph = match b.kind {
                        Btn::Undo => "↶",
                        Btn::Redo => "↷",
                        _ => "✕",
                    };
                    let _ = TextOutW(dc, r.x + 8, r.y + 4, &utf16(glyph));
                    SelectObject(dc, old);
                }
                Btn::Color(_) | Btn::Swatch => {}
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
                let p = client_cursor(hwnd);
                let moving = st.bar_drag.is_some()
                    || st.edit.is_some()
                    || st.handle_at(p).is_some()
                    || (st.tool == Tool::Select && st.object_at(p).is_some());
                let id = if moving {
                    IDC_SIZEALL
                } else if st.over_bar(p) {
                    IDC_ARROW
                } else {
                    IDC_CROSS
                };
                SetCursor(LoadCursorW(None, id).ok());
                LRESULT(1)
            }
            WM_MOUSEMOVE => {
                st.cur = client_cursor(hwnd);
                if st.bar_drag.is_some() {
                    st.drag_bar(st.cur);
                } else if st.edit.is_some() {
                    st.apply_edit();
                } else if st.erasing {
                    st.erase_at_cursor();
                } else if let Some(d) = &mut st.draw
                    && matches!(st.tool, Tool::Pencil | Tool::Marker)
                {
                    d.pts.push(Pt::new(st.cur.0 as f32, st.cur.1 as f32));
                }
                st.follow_cursor();
                redraw(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                st.cur = client_cursor(hwnd);
                if st.over_bar(st.cur) {
                    if let Some(b) = st.button_at(st.cur) {
                        if let Action::Cancel = st.press_button(b) {
                            let _ = DestroyWindow(hwnd);
                        }
                    } else {
                        // the background of the bar: drag it out of the way
                        st.bar_drag = Some(st.cur);
                        SetCapture(hwnd);
                    }
                    redraw(hwnd);
                    return LRESULT(0);
                }
                st.commit_text();
                // the handles of the selected object win over drawing a new shape
                if let Some(i) = st.handle_at(st.cur)
                    && st.start_edit(Some(i))
                {
                    SetCapture(hwnd);
                    redraw(hwnd);
                    return LRESULT(0);
                }
                match st.tool {
                    Tool::Region | Tool::Ocr => {
                        st.region_drag = Some(st.cur);
                        st.anchor = None;
                        SetCapture(hwnd);
                    }
                    Tool::Window | Tool::Screen => {}
                    Tool::Select => {
                        if st.select_at_cursor() && st.start_edit(None) {
                            SetCapture(hwnd);
                        }
                    }
                    Tool::Eraser => {
                        st.ed.checkpoint();
                        st.erasing = true;
                        st.erased_any = false;
                        st.erase_at_cursor();
                        SetCapture(hwnd);
                    }
                    Tool::Picker => st.pick_color(),
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
                if st.bar_drag.take().is_some() {
                    // the bar stays where it was dropped
                } else if st.edit.is_some() {
                    st.finish_edit();
                } else if st.erasing {
                    st.finish_erase();
                } else if st.draw.is_some() {
                    st.finish_draw();
                } else if st.over_bar(st.cur) && st.region_drag.is_none() {
                    // the click belonged to the toolbar
                } else {
                    match st.tool {
                        Tool::Region | Tool::Ocr => {
                            if st.region_drag.is_some() {
                                let picked = st.selection().filter(|r| r.w > 2 && r.h > 2 && (r.w > 4 || r.h > 4));
                                // a click without dragging takes the whole screen (or what the settings say)
                                st.result = picked.or_else(|| st.click_capture());
                                if st.result.is_some() {
                                    let _ = DestroyWindow(hwnd);
                                }
                            }
                            st.region_drag = None;
                        }
                        Tool::Window => {
                            st.result = st
                                .hovered_window()
                                .or_else(|| st.fallback_monitor())
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
                // first cancel what is in progress, then deselect, then the whole overlay
                if st.edit.is_some() {
                    st.cancel_edit();
                    let _ = ReleaseCapture();
                    redraw(hwnd);
                } else if st.erasing || st.bar_drag.is_some() {
                    st.finish_erase();
                    st.bar_drag = None;
                    let _ = ReleaseCapture();
                    redraw(hwnd);
                } else if st.draw.take().is_some() || st.text.take().is_some() || st.region_drag.take().is_some() {
                    let _ = ReleaseCapture();
                    redraw(hwnd);
                } else if st.live_selection().is_some() {
                    st.selected = None;
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
                    if st.edit.is_some() {
                        st.cancel_edit();
                        let _ = ReleaseCapture();
                    } else if st.live_selection().is_some() {
                        st.selected = None;
                    } else {
                        let _ = DestroyWindow(hwnd);
                    }
                } else if ctrl && key == b'Z' as u16 {
                    if GetKeyState(VK_SHIFT.0 as i32) < 0 {
                        st.redo();
                    } else {
                        st.undo();
                    }
                } else if ctrl && key == b'Y' as u16 {
                    st.redo();
                } else if (key == VK_DELETE.0 || key == VK_BACK.0) && st.delete_selected() {
                    // the selected object is gone
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
                            st.result = st.hovered_window().or_else(|| st.fallback_monitor());
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
                    st.set_width(st.width - 1.0);
                } else if key == 0xDD {
                    // ]
                    st.set_width(st.width + 1.0);
                } else if !ctrl
                    && key != VK_BACK.0
                    && let Some((t, _)) = st.keys.iter().find(|(t, vk)| {
                        *vk == key as u32
                            && (!st.capture_only || matches!(t, Tool::Region | Tool::Window | Tool::Screen))
                    })
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
            screen_target: None,
            capture_only: false,
            opts: OverlayOpts::default(),
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

    #[test]
    fn hidden_tools_leave_the_bar_and_a_narrow_monitor_gets_more_rows() {
        let (_, buttons) = toolbar_layout_opts(
            Rect::new(0, 0, 1920, 1080),
            false,
            &[Tool::Blur, Tool::Eraser],
            6,
            false,
        );
        assert!(!buttons.iter().any(|b| b.kind == Btn::Tool(Tool::Blur)));
        assert!(buttons.iter().any(|b| b.kind == Btn::Tool(Tool::Rect)));
        let narrow = Rect::new(0, 0, 640, 480);
        let (bar, buttons) = toolbar_layout_opts(narrow, false, &[], 10, false);
        assert!(bar.x >= 0 && bar.right() <= 640, "{bar:?}");
        assert!(bar.h > BTN + 2 * BAR_PAD, "more than one row");
        for b in &buttons {
            assert_eq!(bar.intersect(&b.rect), Some(b.rect), "{:?}", b.kind);
        }
        let (bottom, _) = toolbar_layout_opts(Rect::new(0, 0, 1920, 1080), false, &[], 6, true);
        assert!(bottom.bottom() <= 1080 && bottom.y > 900, "{bottom:?}");
        // a recording picker offers the capture modes and cancel only
        let (_, rec) = toolbar_layout_opts(Rect::new(0, 0, 1920, 1080), true, &[], 6, false);
        assert!(rec.iter().all(|b| matches!(
            b.kind,
            Btn::Tool(Tool::Region | Tool::Window | Tool::Screen) | Btn::Cancel
        )));
    }

    #[test]
    fn shift_keeps_squares_and_45_degree_lines() {
        assert_eq!(constrain(Tool::Rect, (10, 10), (50, 30)), (50, 50));
        assert_eq!(constrain(Tool::Ellipse, (10, 10), (0, 30)), (-10, 30));
        assert_eq!(constrain(Tool::Arrow, (0, 0), (100, 10)), (100, 0));
        assert_eq!(constrain(Tool::Line, (0, 0), (10, 100)), (0, 100));
        assert_eq!(constrain(Tool::Line, (0, 0), (60, 80)), (70, 70));
        assert_eq!(constrain(Tool::Pencil, (0, 0), (7, 9)), (7, 9));
    }

    /// The overlay state without a window: no input is injected, nothing touches the desktop.
    fn with_state(req: Request, f: impl FnOnce(&mut State)) {
        // SAFETY: the state is freed right after use; nothing else holds its GDI objects.
        unsafe {
            let mut st = State::new(&req);
            f(&mut st);
            st.free();
        }
    }

    fn draw_rect(st: &mut State, from: (i32, i32), to: (i32, i32)) {
        st.tool = Tool::Rect;
        st.cur = to;
        st.draw = Some(Drag {
            start: from,
            pts: Vec::new(),
        });
        st.finish_draw();
    }

    #[test]
    fn a_drawn_rectangle_can_be_stretched_moved_recoloured_and_deleted() {
        with_state(base_request((0, 0), Tool::Rect), |st| {
            draw_rect(st, (100, 100), (200, 160));
            assert_eq!(st.ed.doc.objects.len(), 1);
            let id = st.selected.expect("a new shape is selected");
            // dragging the bottom-right handle stretches it
            let i = st.handle_at((200, 160)).expect("a handle under the corner");
            assert!(st.start_edit(Some(i)));
            st.cur = (300, 220);
            st.apply_edit();
            st.finish_edit();
            let Shape::Rect { a, b, .. } = st.ed.doc.get(id).unwrap().shape.clone() else {
                panic!()
            };
            assert_eq!((a.x, a.y, b.x, b.y), (100.0, 100.0, 300.0, 220.0));
            // moving: select with the select tool, drag the body
            st.tool = Tool::Select;
            st.cur = (100, 150);
            assert!(st.select_at_cursor() && st.start_edit(None));
            st.cur = (130, 170);
            st.apply_edit();
            st.finish_edit();
            let Shape::Rect { a, .. } = st.ed.doc.get(id).unwrap().shape.clone() else {
                panic!()
            };
            assert_eq!((a.x, a.y), (130.0, 120.0));
            // a click that moves nothing leaves no undo step behind
            st.cur = (130, 150);
            assert!(st.select_at_cursor() && st.start_edit(None));
            let depth = st.ed.undo_depth();
            st.finish_edit();
            assert_eq!(st.ed.undo_depth(), depth - 1);
            // colour and width follow the selection
            st.set_color([1, 2, 3, 255]);
            st.set_width(9.0);
            let o = st.ed.doc.get(id).unwrap();
            assert_eq!((o.color, o.width), ([1, 2, 3, 255], 9.0));
            // delete, then undo brings it back
            assert!(st.delete_selected());
            assert!(st.ed.doc.objects.is_empty());
            st.undo();
            assert_eq!(st.ed.doc.objects.len(), 1);
            st.redo();
            assert!(st.ed.doc.objects.is_empty() && st.selected.is_none());
        });
    }

    #[test]
    fn a_capture_tool_hides_the_selection_and_cancelling_an_edit_restores_the_shape() {
        with_state(base_request((0, 0), Tool::Rect), |st| {
            draw_rect(st, (50, 50), (150, 120));
            let before = st.ed.doc.objects[0].shape.clone();
            st.tool = Tool::Region;
            assert!(st.live_selection().is_none() && st.handle_at((150, 120)).is_none());
            st.tool = Tool::Rect;
            let i = st.handle_at((150, 120)).unwrap();
            st.start_edit(Some(i));
            st.cur = (400, 300);
            st.apply_edit();
            assert_ne!(st.ed.doc.objects[0].shape, before);
            st.cancel_edit();
            assert_eq!(st.ed.doc.objects[0].shape, before);
            assert_eq!(st.ed.undo_depth(), 1, "only the drawing itself is on the stack");
        });
    }

    #[test]
    fn the_eraser_wipes_what_it_touches_in_one_undo_step() {
        with_state(base_request((0, 0), Tool::Rect), |st| {
            draw_rect(st, (50, 50), (150, 120));
            draw_rect(st, (250, 50), (350, 120));
            st.tool = Tool::Eraser;
            st.ed.checkpoint();
            st.erasing = true;
            for p in [(50, 80), (250, 80)] {
                st.cur = p;
                st.erase_at_cursor();
            }
            st.finish_erase();
            assert!(st.ed.doc.objects.is_empty());
            st.undo();
            assert_eq!(st.ed.doc.objects.len(), 2, "one undo restores both");
            // an eraser stroke that hits nothing adds no undo step
            let depth = st.ed.undo_depth();
            st.ed.checkpoint();
            st.erasing = true;
            st.cur = (500, 350);
            st.erase_at_cursor();
            st.finish_erase();
            assert_eq!(st.ed.undo_depth(), depth);
        });
    }

    #[test]
    fn a_single_click_captures_the_screen_as_configured() {
        let monitors = vec![Rect::new(0, 0, 300, 400), Rect::new(300, 0, 300, 400)];
        let run = |click, at: (i32, i32)| {
            let req = Request {
                opts: OverlayOpts {
                    click,
                    ..OverlayOpts::default()
                },
                monitors: monitors.clone(),
                ..base_request((0, 0), Tool::Region)
            };
            let mut out = None;
            with_state(req, |st| {
                st.cur = at;
                out = st.click_capture();
            });
            out
        };
        assert_eq!(
            run(OverlayClick::Monitor, (400, 300)),
            Some(Rect::new(300, 0, 300, 400))
        );
        assert_eq!(run(OverlayClick::AllScreens, (10, 10)), Some(Rect::new(0, 0, 600, 400)));
        assert_eq!(run(OverlayClick::Nothing, (10, 10)), None);
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
