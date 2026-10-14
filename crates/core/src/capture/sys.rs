//! Win32 enumeration: monitors, top-level windows, the cursor and the foreground application.

use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GetCursorPos, GetForegroundWindow, GetWindowLongW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, WS_EX_TOOLWINDOW,
};
use windows::core::BOOL;

use crate::geometry::{Monitor, Rect};

/// Makes the process per-monitor DPI aware (v2) so every coordinate is a physical pixel of the virtual desktop.
/// Call once at startup, before any window exists. The app manifest does the same; this covers the CLI and tests.
pub fn enable_dpi_awareness() {
    // SAFETY: plain process-wide setting; failure (already set by the manifest) is fine.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

unsafe extern "system" fn monitor_cb(h: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    // SAFETY: `data` is the pointer to the Vec passed by `monitors()` below.
    let list = unsafe { &mut *(data.0 as *mut Vec<Monitor>) };
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: `info` has the cbSize the API requires.
    if unsafe { GetMonitorInfoW(h, &mut info.monitorInfo as *mut _ as *mut _) }.as_bool() {
        let r = info.monitorInfo.rcMonitor;
        let (mut dx, mut dy) = (96u32, 96u32);
        // SAFETY: out parameters are valid locals.
        let _ = unsafe { GetDpiForMonitor(h, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) };
        let len = info
            .szDevice
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(info.szDevice.len());
        list.push(Monitor {
            index: list.len(),
            name: String::from_utf16_lossy(&info.szDevice[..len]),
            rect: Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top),
            dpi: dx,
            primary: info.monitorInfo.dwFlags & 1 != 0,
            handle: h.0 as isize,
        });
    }
    BOOL(1)
}

/// All monitors, primary first, then left-to-right / top-to-bottom. `index` follows this order.
pub fn monitors() -> Vec<Monitor> {
    let mut list: Vec<Monitor> = Vec::new();
    // SAFETY: the callback only touches the Vec we pass in `LPARAM`.
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(monitor_cb), LPARAM(&mut list as *mut _ as isize));
    }
    list.sort_by_key(|m| (!m.primary, m.rect.x, m.rect.y));
    for (i, m) in list.iter_mut().enumerate() {
        m.index = i;
    }
    list
}

pub fn cursor_pos() -> Option<(i32, i32)> {
    let mut p = POINT::default();
    // SAFETY: `p` is a valid out parameter.
    unsafe { GetCursorPos(&mut p) }.ok().map(|_| (p.x, p.y))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowInfo {
    pub hwnd: isize,
    pub title: String,
    pub app: String,
    /// Visible frame in virtual-desktop pixels (without the invisible resize border).
    pub rect: Rect,
}

fn process_name(hwnd: HWND) -> String {
    let mut pid = 0u32;
    // SAFETY: valid out parameter; the process handle is closed below.
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };
        let mut buf = [0u16; 520];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            h,
            PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(h);
        if ok.is_err() {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        Path::new(&path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

fn window_info(hwnd: HWND) -> Option<WindowInfo> {
    // SAFETY: read-only queries on a window handle; every out parameter is a local of the right type.
    unsafe {
        let mut rc = RECT::default();
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rc as *mut _ as *mut _,
            std::mem::size_of::<RECT>() as u32,
        )
        .ok()?;
        let mut t = [0u16; 256];
        let n = GetWindowTextW(hwnd, &mut t);
        Some(WindowInfo {
            hwnd: hwnd.0 as isize,
            title: String::from_utf16_lossy(&t[..n.max(0) as usize]),
            app: process_name(hwnd),
            rect: Rect::new(rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top),
        })
    }
}

struct EnumState {
    exclude: isize,
    out: Vec<WindowInfo>,
}

unsafe extern "system" fn window_cb(hwnd: HWND, data: LPARAM) -> BOOL {
    // SAFETY: `data` points to the EnumState of `visible_windows`.
    let st = unsafe { &mut *(data.0 as *mut EnumState) };
    // SAFETY: read-only window queries.
    unsafe {
        if hwnd.0 as isize == st.exclude || !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return BOOL(1);
        }
        if GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
            return BOOL(1);
        }
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4);
        if cloaked != 0 {
            return BOOL(1);
        }
    }
    if let Some(w) = window_info(hwnd).filter(|w| !w.title.is_empty() && w.rect.w > 8 && w.rect.h > 8) {
        st.out.push(w);
    }
    BOOL(1)
}

/// Visible top-level windows in z-order (topmost first). `exclude` is a window to skip (our own overlay).
pub fn visible_windows(exclude: isize) -> Vec<WindowInfo> {
    let mut st = EnumState {
        exclude,
        out: Vec::new(),
    };
    // SAFETY: the callback only touches `st`, which outlives the call.
    unsafe {
        let _ = EnumWindows(Some(window_cb), LPARAM(&mut st as *mut _ as isize));
    }
    st.out
}

/// Topmost window containing the point (list must be in z-order, as returned by `visible_windows`).
pub fn window_at(windows: &[WindowInfo], x: i32, y: i32) -> Option<&WindowInfo> {
    windows.iter().find(|w| w.rect.contains(x, y))
}

/// Executable name (without `.exe`) of the foreground window, or an empty string.
pub fn foreground_app() -> String {
    // SAFETY: read-only query.
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        String::new()
    } else {
        process_name(hwnd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_at_prefers_topmost() {
        let w = |h, x, y| WindowInfo {
            hwnd: h,
            title: "t".into(),
            app: "a".into(),
            rect: Rect::new(x, y, 100, 100),
        };
        let list = vec![w(1, 50, 50), w(2, 0, 0)];
        assert_eq!(window_at(&list, 60, 60).unwrap().hwnd, 1);
        assert_eq!(window_at(&list, 10, 10).unwrap().hwnd, 2);
        assert!(window_at(&list, 500, 500).is_none());
    }

    #[test]
    #[ignore = "needs a real interactive desktop"]
    fn real_monitors_are_enumerated() {
        enable_dpi_awareness();
        let m = monitors();
        assert!(!m.is_empty(), "at least one monitor");
        assert!(m[0].primary);
        assert!(m.iter().all(|x| x.rect.w > 0 && x.rect.h > 0 && x.dpi >= 96));
    }
}
