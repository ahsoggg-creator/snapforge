//! Taking a screenshot: hide the window, capture (or let the user pick a region/window on a frozen frame, annotating
//! it first in the overlay), then hand the result back to the UI thread.

use std::collections::HashMap;

use eframe::egui::{self, ViewportCommand};
use snapforge_core::RgbaImage;
use snapforge_core::capture::{self, CaptureOptions, sys};
use snapforge_core::diag;
use snapforge_core::editor::{Document, Object};
use snapforge_core::geometry::{Rect, virtual_bounds};
use snapforge_core::hotkey::{self, HotAction};
use snapforge_core::record::Finished;
use snapforge_core::settings::OverlayTool;

use crate::selector::{self, Outcome};
use crate::{App, View, demo, editor_ui::EditorState};

pub struct Shot {
    /// The finished picture: `base` with `objects` drawn on it.
    pub image: RgbaImage,
    pub app: String,
    /// The cropped screen without annotations, and the annotations drawn in the overlay (editable later).
    pub base: Option<RgbaImage>,
    pub objects: Vec<Object>,
    /// "Hide sensitive text" was on in the overlay.
    pub privacy: bool,
}

impl Shot {
    pub fn plain(image: RgbaImage, app: String) -> Shot {
        Shot {
            image,
            app,
            base: None,
            objects: Vec::new(),
            privacy: false,
        }
    }
}

pub enum CapMsg {
    Done(Shot, Option<OverlayMemory>),
    /// Text recognised in the selected area (the "OCR" overlay tool): copied, no editor.
    Text(Result<String, String>),
    Cancelled,
    Failed(String),
    /// The area picked for a screen recording (`None`: cancelled).
    RecordArea(Option<Rect>),
    /// A recording was stopped and written (or failed).
    VideoSaved(Result<Finished, String>),
}

/// What the overlay remembers between captures.
#[derive(Clone, Copy)]
pub struct OverlayMemory {
    pub color: [u8; 4],
    pub width: f32,
    pub privacy: bool,
}

/// Texts and options of the overlay, prepared on the UI thread (translated).
#[derive(Clone)]
pub struct OverlaySetup {
    pub start_tool: OverlayTool,
    pub keys: Vec<(OverlayTool, u32)>,
    pub labels: HashMap<OverlayTool, String>,
    pub undo_label: String,
    pub cancel_label: String,
    pub privacy_label: String,
    pub hint: String,
    pub color: [u8; 4],
    pub width: f32,
    pub privacy: bool,
    pub ocr_language: Option<String>,
    /// Full-screen and monitor captures go through the overlay too.
    pub screens_overlay: bool,
    pub screen_hint: String,
    pub record_hint: String,
}

/// Our own process name must not become the `{app}` part of a file name.
fn clean_app(app: String) -> String {
    if app.eq_ignore_ascii_case("snapforge") {
        String::new()
    } else {
        app
    }
}

/// Moves the annotations from the coordinates of the whole frozen desktop into the crop and renders them.
fn finish_shot(base: RgbaImage, mut objects: Vec<Object>, dx: i32, dy: i32, app: String, privacy: bool) -> Shot {
    for o in &mut objects {
        o.shape.translate(-(dx as f32), -(dy as f32));
    }
    let mut doc = Document::new(base.clone());
    for o in &objects {
        doc.add(o.shape.clone(), o.color, o.width);
    }
    let image = if objects.is_empty() { base.clone() } else { doc.render() };
    Shot {
        image,
        app,
        base: Some(base),
        objects,
        privacy,
    }
}

type GrabResult = (Option<Shot>, Option<OverlayMemory>, Option<Result<String, String>>);

fn grab(
    action: HotAction,
    monitor: Option<usize>,
    delay_ms: u32,
    cursor: bool,
    setup: OverlaySetup,
) -> Result<GrabResult, String> {
    let opts = CaptureOptions {
        cursor,
        force_gdi: false,
    };
    let fg = clean_app(sys::foreground_app());
    if delay_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms as u64));
    }
    let monitors = sys::monitors();
    let screens = matches!(action, HotAction::CaptureScreen | HotAction::CaptureMonitor);
    if screens && !setup.screens_overlay {
        let c = if action == HotAction::CaptureScreen {
            capture::capture_virtual_desktop(opts)
        } else {
            let idx = monitor
                .or_else(|| capture::monitor_under_cursor(&monitors).map(|m| m.index))
                .ok_or("no monitor found")?;
            capture::capture_monitor(idx, opts)
        }
        .map_err(|e| e.to_string())?;
        return Ok((Some(Shot::plain(c.image, fg)), None, None));
    }
    let bounds = virtual_bounds(&monitors);
    let frozen = capture::capture_rect_on(&monitors, bounds, opts).map_err(|e| e.to_string())?;
    let windows = sys::visible_windows(0);
    let screen_target = match action {
        HotAction::CaptureScreen => Some(bounds),
        HotAction::CaptureMonitor => monitor.and_then(|i| monitors.iter().find(|m| m.index == i).map(|m| m.rect)),
        _ => None,
    };
    let req = selector::Request {
        frozen: frozen.image.clone(),
        origin: (bounds.x, bounds.y),
        windows: windows.clone(),
        monitors: monitors.iter().map(|m| m.rect).collect(),
        start_tool: match action {
            HotAction::CaptureWindow => OverlayTool::Window,
            HotAction::CaptureScreen | HotAction::CaptureMonitor => OverlayTool::Screen,
            HotAction::CaptureRegion | HotAction::RecordVideo => setup.start_tool,
        },
        keys: setup.keys.clone(),
        labels: setup.labels.clone(),
        undo_label: setup.undo_label.clone(),
        cancel_label: setup.cancel_label.clone(),
        privacy_label: setup.privacy_label.clone(),
        hint: if screens {
            setup.screen_hint.clone()
        } else {
            setup.hint.clone()
        },
        color: setup.color,
        width: setup.width,
        privacy: setup.privacy,
        screen_target,
        capture_only: false,
    };
    let Some(Outcome {
        rect,
        objects,
        color,
        width,
        tool,
        privacy,
    }) = selector::run(req)
    else {
        return Ok((None, None, None));
    };
    let memory = Some(OverlayMemory { color, width, privacy });
    let base = capture::crop_piece(&frozen.image, bounds, rect);
    let app = windows
        .iter()
        .find(|w| w.rect == rect)
        .map(|w| w.app.clone())
        .unwrap_or(fg);
    let shot = finish_shot(base, objects, rect.x - bounds.x, rect.y - bounds.y, app, privacy);
    if tool == OverlayTool::Ocr {
        let text = snapforge_core::ocr::recognize(&shot.image, setup.ocr_language.as_deref())
            .map(|d| d.text)
            .map_err(|e| e.to_string());
        return Ok((None, memory, Some(text)));
    }
    Ok((Some(shot), memory, None))
}

/// Shows the overlay with only the capture modes and returns the chosen area (screen recording).
pub fn pick_area(setup: OverlaySetup) -> Result<Option<Rect>, String> {
    let monitors = sys::monitors();
    let bounds = virtual_bounds(&monitors);
    let frozen = capture::capture_rect_on(&monitors, bounds, CaptureOptions::default()).map_err(|e| e.to_string())?;
    let req = selector::Request {
        frozen: frozen.image,
        origin: (bounds.x, bounds.y),
        windows: sys::visible_windows(0),
        monitors: monitors.iter().map(|m| m.rect).collect(),
        start_tool: OverlayTool::Region,
        keys: setup.keys,
        labels: setup.labels,
        undo_label: setup.undo_label,
        cancel_label: setup.cancel_label,
        privacy_label: setup.privacy_label,
        hint: setup.record_hint,
        color: setup.color,
        width: setup.width,
        privacy: false,
        screen_target: None,
        capture_only: true,
    };
    Ok(selector::run(req).map(|o| o.rect))
}

impl App {
    pub fn overlay_setup(&self, action: HotAction) -> OverlaySetup {
        let s = &self.settings;
        let tool_key = |t: OverlayTool| -> &'static str {
            match t {
                OverlayTool::Region => "otool.region",
                OverlayTool::Window => "otool.window",
                OverlayTool::Screen => "otool.screen",
                OverlayTool::Ocr => "otool.ocr",
                OverlayTool::Rect => "otool.rect",
                OverlayTool::Ellipse => "otool.ellipse",
                OverlayTool::Line => "otool.line",
                OverlayTool::Arrow => "otool.arrow",
                OverlayTool::Pencil => "otool.pencil",
                OverlayTool::Marker => "otool.marker",
                OverlayTool::Text => "otool.text",
                OverlayTool::Step => "otool.step",
                OverlayTool::Blur => "otool.blur",
                OverlayTool::Pixelate => "otool.pixelate",
            }
        };
        OverlaySetup {
            start_tool: s.overlay_start,
            keys: s
                .overlay_keys
                .iter()
                .filter_map(|(t, k)| hotkey::parse_single_key(k).map(|vk| (*t, vk)))
                .collect(),
            labels: OverlayTool::ALL
                .iter()
                .map(|t| (*t, self.t(tool_key(*t)).to_owned()))
                .collect(),
            undo_label: self.t("otool.undo").to_owned(),
            cancel_label: self.t("otool.cancel").to_owned(),
            privacy_label: self.t("otool.privacy").to_owned(),
            hint: self
                .t(if action == HotAction::CaptureWindow {
                    "sel.hint_window"
                } else {
                    "sel.hint_region"
                })
                .to_owned(),
            color: s.overlay_color,
            width: s.overlay_width,
            privacy: s.overlay_privacy,
            ocr_language: s.post.options.ocr_language.clone(),
            screens_overlay: s.overlay_for_screens,
            screen_hint: self.t("sel.hint_screen").to_owned(),
            record_hint: self.t("sel.hint_record").to_owned(),
        }
    }

    pub fn start_capture(&mut self, ctx: &egui::Context, action: HotAction) {
        self.start_capture_on(ctx, action, None);
    }

    pub fn start_capture_on(&mut self, ctx: &egui::Context, action: HotAction, monitor: Option<usize>) {
        if action == HotAction::RecordVideo {
            self.toggle_recording(ctx);
            return;
        }
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
            let _ = self
                .cap_tx
                .send(CapMsg::Done(Shot::plain(image, demo::app_name(i).to_owned()), None));
            ctx.request_repaint();
            return;
        }
        let visible = !self.window_hidden;
        self.restore_window = visible;
        if visible {
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            self.window_hidden = true;
        }
        let (tx, ctx2) = (self.cap_tx.clone(), ctx.clone());
        let (delay, cursor) = (self.settings.delay_ms, self.settings.cursor_in_shot);
        let setup = self.overlay_setup(action);
        let (host, reports_tx) = (self.plugins.host.clone(), self.plugins.reports_tx.clone());
        std::thread::spawn(move || {
            // give the window time to disappear from the screen
            std::thread::sleep(std::time::Duration::from_millis(if visible { 280 } else { 60 }));
            let msg = match grab(action, monitor, delay, cursor, setup) {
                Ok((Some(mut shot), memory, _)) => {
                    let (image, reports) = host.on_capture(shot.image);
                    // plugins work on the finished picture; the editable layers stay as the user drew them
                    shot.image = image;
                    for r in reports {
                        let _ = reports_tx.send(r);
                    }
                    diag::log(
                        diag::Level::Info,
                        &format!("captured {}x{}", shot.image.width(), shot.image.height()),
                    );
                    CapMsg::Done(shot, memory)
                }
                Ok((None, _, Some(text))) => CapMsg::Text(text),
                Ok((None, _, None)) => CapMsg::Cancelled,
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
            CapMsg::Done(shot, memory) => {
                self.remember_overlay(memory);
                // the history gets the picture only after the chain ran, so redactions are never bypassed
                self.begin_post(ctx, shot);
            }
            CapMsg::Text(res) => {
                if self.restore_window {
                    self.show_window(ctx);
                }
                match res {
                    Ok(text) if text.trim().is_empty() => {
                        let m = self.t("ocr.empty").to_owned();
                        self.notify(m, true);
                    }
                    Ok(text) => match snapforge_core::output::copy_text_to_clipboard(&text) {
                        Ok(()) => {
                            let preview: String = text.chars().take(80).collect();
                            let m = format!("{}: {preview}", self.t("ocr.copied"));
                            self.notify(m, false);
                        }
                        Err(e) => self.notify(e.to_string(), true),
                    },
                    Err(e) => self.notify(e, true),
                }
            }
            CapMsg::Cancelled => {
                if self.restore_window {
                    self.show_window(ctx);
                }
            }
            CapMsg::RecordArea(Some(rect)) => self.start_recording(ctx, rect),
            CapMsg::RecordArea(None) => {
                if self.restore_window {
                    self.show_window(ctx);
                }
            }
            CapMsg::VideoSaved(res) => self.video_saved(ctx, res),
            CapMsg::Failed(e) => {
                self.show_window(ctx);
                let msg = format!("{}: {e}", self.t("err.capture"));
                self.notify(msg, true);
            }
        }
    }

    fn remember_overlay(&mut self, memory: Option<OverlayMemory>) {
        if let Some(m) = memory {
            let s = &mut self.settings;
            if s.overlay_color != m.color || s.overlay_width != m.width || s.overlay_privacy != m.privacy {
                s.overlay_color = m.color;
                s.overlay_width = m.width;
                s.overlay_privacy = m.privacy;
                self.save_settings();
            }
        }
    }

    pub fn open_editor(&mut self, ctx: &egui::Context, image: RgbaImage, app: String, history_id: Option<String>) {
        self.editor = Some(EditorState::new(ctx, image, app, history_id));
        self.view = View::Editor;
    }

    /// Opens the editor on the cropped screen with the overlay annotations as editable layers.
    pub fn open_editor_layers(
        &mut self,
        ctx: &egui::Context,
        base: RgbaImage,
        objects: Vec<Object>,
        app: String,
        history_id: Option<String>,
    ) {
        let mut es = EditorState::new(ctx, base, app, history_id);
        es.add_objects(objects);
        self.editor = Some(es);
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
