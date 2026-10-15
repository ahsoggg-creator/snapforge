//! Taking a screenshot: hide the window, capture (or let the user pick a region/window on a frozen frame), then
//! hand the result back to the UI thread.

use eframe::egui::{self, ViewportCommand};
use snapforge_core::RgbaImage;
use snapforge_core::capture::{self, CaptureOptions, sys};
use snapforge_core::diag;
use snapforge_core::geometry::virtual_bounds;
use snapforge_core::hotkey::HotAction;

use crate::selector::{self, Mode as SelMode};
use crate::{App, View, demo, editor_ui::EditorState};

pub struct Shot {
    pub image: RgbaImage,
    pub app: String,
}

pub enum CapMsg {
    Done(Shot),
    Cancelled,
    Failed(String),
}

/// Our own process name must not become the `{app}` part of a file name.
fn clean_app(app: String) -> String {
    if app.eq_ignore_ascii_case("snapforge") {
        String::new()
    } else {
        app
    }
}

fn grab(
    action: HotAction,
    monitor: Option<usize>,
    delay_ms: u32,
    cursor: bool,
    hint: String,
) -> Result<Option<Shot>, String> {
    let opts = CaptureOptions {
        cursor,
        force_gdi: false,
    };
    let fg = clean_app(sys::foreground_app());
    if delay_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms as u64));
    }
    let monitors = sys::monitors();
    match action {
        HotAction::CaptureScreen => {
            let c = capture::capture_virtual_desktop(opts).map_err(|e| e.to_string())?;
            Ok(Some(Shot {
                image: c.image,
                app: fg,
            }))
        }
        HotAction::CaptureMonitor => {
            let idx = monitor
                .or_else(|| capture::monitor_under_cursor(&monitors).map(|m| m.index))
                .ok_or("no monitor found")?;
            let c = capture::capture_monitor(idx, opts).map_err(|e| e.to_string())?;
            Ok(Some(Shot {
                image: c.image,
                app: fg,
            }))
        }
        HotAction::CaptureRegion | HotAction::CaptureWindow => {
            let bounds = virtual_bounds(&monitors);
            let frozen = capture::capture_rect_on(&monitors, bounds, opts).map_err(|e| e.to_string())?;
            let windows = sys::visible_windows(0);
            let mode = if action == HotAction::CaptureWindow {
                SelMode::Window
            } else {
                SelMode::Region
            };
            let req = selector::Request {
                frozen: frozen.image.clone(),
                origin: (bounds.x, bounds.y),
                mode,
                windows: windows.clone(),
                hint,
            };
            let Some(rect) = selector::run(req) else {
                return Ok(None);
            };
            let image = capture::crop_piece(&frozen.image, bounds, rect);
            let app = windows
                .iter()
                .find(|w| w.rect == rect)
                .map(|w| w.app.clone())
                .unwrap_or(fg);
            Ok(Some(Shot { image, app }))
        }
    }
}

impl App {
    pub fn start_capture(&mut self, ctx: &egui::Context, action: HotAction) {
        self.start_capture_on(ctx, action, None);
    }

    pub fn start_capture_on(&mut self, ctx: &egui::Context, action: HotAction, monitor: Option<usize>) {
        if self.capturing {
            return;
        }
        self.capturing = true;
        if self.flags.demo {
            // the demo never touches the screen: it produces an invented window
            let i = self.demo_counter;
            self.demo_counter += 1;
            let (image, reports) = self.plugins.host.on_capture(demo::demo_image(i, 1100, 640));
            for r in reports {
                let _ = self.plugins.reports_tx.send(r);
            }
            let shot = Shot {
                image,
                app: demo::app_name(i).to_owned(),
            };
            let _ = self.cap_tx.send(CapMsg::Done(shot));
            ctx.request_repaint();
            return;
        }
        let visible = !self.window_hidden;
        if visible {
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            self.window_hidden = true;
        }
        let (tx, ctx2) = (self.cap_tx.clone(), ctx.clone());
        let (delay, cursor) = (self.settings.delay_ms, self.settings.cursor_in_shot);
        let hint = self
            .t(if action == HotAction::CaptureWindow {
                "sel.hint_window"
            } else {
                "sel.hint_region"
            })
            .to_owned();
        let (host, reports_tx) = (self.plugins.host.clone(), self.plugins.reports_tx.clone());
        std::thread::spawn(move || {
            // give the window time to disappear from the screen
            std::thread::sleep(std::time::Duration::from_millis(if visible { 280 } else { 60 }));
            let msg = match grab(action, monitor, delay, cursor, hint) {
                Ok(Some(mut shot)) => {
                    let (image, reports) = host.on_capture(shot.image);
                    shot.image = image;
                    for r in reports {
                        let _ = reports_tx.send(r);
                    }
                    diag::log(
                        diag::Level::Info,
                        &format!("captured {}x{}", shot.image.width(), shot.image.height()),
                    );
                    CapMsg::Done(shot)
                }
                Ok(None) => CapMsg::Cancelled,
                Err(e) => {
                    diag::log(diag::Level::Error, &format!("capture failed: {e}"));
                    CapMsg::Failed(e)
                }
            };
            let _ = tx.send(msg);
            ctx2.request_repaint();
        });
    }

    pub fn on_capture_msg(&mut self, ctx: &egui::Context, msg: CapMsg) {
        self.capturing = false;
        match msg {
            CapMsg::Done(shot) => {
                // the history gets the picture only after the chain ran, so redactions are never bypassed
                self.begin_post(ctx, shot.image, shot.app);
            }
            CapMsg::Cancelled => {
                self.show_window(ctx);
            }
            CapMsg::Failed(e) => {
                self.show_window(ctx);
                let msg = format!("{}: {e}", self.t("err.capture"));
                self.notify(msg, true);
            }
        }
    }

    pub fn open_editor(&mut self, ctx: &egui::Context, image: RgbaImage, app: String, history_id: Option<String>) {
        self.editor = Some(EditorState::new(ctx, image, app, history_id));
        self.view = View::Editor;
    }

    pub fn open_history_entry(&mut self, ctx: &egui::Context, id: &str) {
        let app = self
            .history
            .entries()
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.app.clone())
            .unwrap_or_default();
        match self.history.load_image(id) {
            Ok(img) => self.open_editor(ctx, img, app, Some(id.to_owned())),
            Err(e) => {
                let msg = format!("{}: {e}", self.t("err.open"));
                self.notify(msg, true);
            }
        }
    }
}
