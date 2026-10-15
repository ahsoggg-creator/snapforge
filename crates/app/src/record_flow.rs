//! Screen recording from the UI side: pick the area, start, show the progress in the tray, stop and report.

use std::time::Duration;

use eframe::egui::{self, ViewportCommand};
use snapforge_core::geometry::Rect;
use snapforge_core::hotkey::HotAction;
use snapforge_core::record::{Finished, RecordOptions, Recorder};

use crate::capture_flow::{CapMsg, pick_area};
use crate::{App, winint};

impl App {
    /// The "record" hotkey, tray item and button: stops a running recording, otherwise asks for the area.
    pub fn toggle_recording(&mut self, ctx: &egui::Context) {
        if self.recorder.is_some() {
            self.stop_recording(ctx);
            return;
        }
        if self.capturing {
            return;
        }
        if self.flags.demo {
            let m = self.t("rec.demo").to_owned();
            self.notify(m, true);
            return;
        }
        self.capturing = true;
        let visible = !self.window_hidden;
        self.restore_window = visible;
        if visible {
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            self.window_hidden = true;
        }
        let (tx, ctx2) = (self.cap_tx.clone(), ctx.clone());
        let setup = self.overlay_setup(HotAction::RecordVideo);
        std::thread::spawn(move || {
            // give the window time to disappear from the screen
            std::thread::sleep(Duration::from_millis(if visible { 280 } else { 60 }));
            let msg = match pick_area(setup) {
                Ok(area) => CapMsg::RecordArea(area),
                Err(e) => CapMsg::Failed(e),
            };
            let _ = tx.send(msg);
            ctx2.request_repaint();
        });
    }

    pub fn start_recording(&mut self, ctx: &egui::Context, rect: Rect) {
        let name = chrono::Local::now()
            .format("SnapForge_%Y-%m-%d_%H-%M-%S.mp4")
            .to_string();
        let opts = RecordOptions {
            rect,
            fps: self.settings.video_fps,
            cursor: self.settings.video_cursor,
            path: self.settings.save_dir.join(name),
        };
        match Recorder::start(opts) {
            Ok(rec) => {
                snapforge_core::diag::log(snapforge_core::diag::Level::Info, "recording started");
                self.rec_secs = 0;
                if let Some(t) = &self.tray {
                    t.set_recording(Some(Duration::ZERO));
                } else {
                    // without a tray icon the window stays reachable from the taskbar
                    self.show_window(ctx);
                    ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
                }
                self.recorder = Some(rec);
                ctx.request_repaint_after(Duration::from_millis(500));
            }
            Err(e) => {
                self.show_window(ctx);
                let msg = format!("{}: {e}", self.t("rec.failed"));
                self.notify(msg, true);
            }
        }
    }

    /// Finishes the file on a worker thread (the encoder flushes its buffers); the result arrives as `VideoSaved`.
    pub fn stop_recording(&mut self, ctx: &egui::Context) {
        let Some(rec) = self.recorder.take() else { return };
        if let Some(t) = &self.tray {
            t.set_recording(None);
        }
        let (tx, ctx2) = (self.cap_tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(CapMsg::VideoSaved(rec.stop()));
            ctx2.request_repaint();
        });
    }

    pub fn video_saved(&mut self, ctx: &egui::Context, res: Result<Finished, String>) {
        self.show_window(ctx);
        match res {
            Ok(f) => {
                snapforge_core::diag::log(
                    snapforge_core::diag::Level::Info,
                    &format!("recorded {} frames, {:.1} s", f.frames, f.seconds),
                );
                let msg = format!("{}: {}", self.t("rec.saved"), f.path.display());
                self.last_video = Some(f.path);
                self.view = crate::View::Home;
                self.notify(msg, false);
            }
            Err(e) => {
                let msg = format!("{}: {e}", self.t("rec.failed"));
                self.notify(msg, true);
            }
        }
    }

    /// Keeps the tray tooltip's timer going and notices a recorder that ended by itself.
    pub fn recording_tick(&mut self, ctx: &egui::Context) {
        let Some(rec) = &self.recorder else { return };
        if rec.has_ended() {
            self.stop_recording(ctx);
            return;
        }
        let elapsed = rec.elapsed();
        if elapsed.as_secs() != self.rec_secs {
            self.rec_secs = elapsed.as_secs();
            if let Some(t) = &self.tray {
                t.set_recording(Some(elapsed));
            }
        }
        ctx.request_repaint_after(Duration::from_millis(500));
    }

    pub fn open_last_video(&self) {
        if let Some(p) = &self.last_video {
            winint::open_path(p);
        }
    }
}
