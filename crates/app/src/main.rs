//! SnapForge — screen capture, editor, post-capture actions and sandboxed plugins.
//!
//! One executable: the app, the installer (`SnapForge-Setup.exe` / `--install`), the uninstaller (`--uninstall`),
//! the headless updater (`--update`), the demo (`--demo`) and the screenshot generator (`--screenshots <dir>`).
#![windows_subsystem = "windows"]
#![allow(clippy::chunks_exact_to_as_chunks)]

mod capture_flow;
mod demo;
mod demo_tour;
mod diag;
mod dragout;
mod editor_ui;
mod i18n;
mod installer;
mod post_ui;
mod selector;
mod settings_extra;
mod theme;
mod tray;
mod update;
mod views;
mod winint;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, TextureHandle, ViewportCommand};
use snapforge_core::capture::sys;
use snapforge_core::geometry::Monitor;
use snapforge_core::history::History;
use snapforge_core::hotkey::{HotAction, HotkeyManager, Registered};
use snapforge_core::secrets::{CredentialManager, MemoryStore, SecretStore};
use snapforge_core::settings::{Lang, Settings};
use snapforge_core::{diag as core_diag, paths};

pub use capture_flow::{CapMsg, Shot};
pub use editor_ui::EditorState;

pub enum Mode {
    App,
    Install(installer::Install),
    Uninstall(installer::Uninstall),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Home,
    History,
    Editor,
    PostCapture,
    Settings,
}

pub struct Flags {
    pub demo: bool,
    pub screenshots: Option<PathBuf>,
    pub minimized: bool,
}

pub struct Toast {
    pub text: String,
    pub error: bool,
    pub since: Instant,
}

pub struct App {
    pub mode: Mode,
    pub view: View,
    pub flags: Flags,
    pub settings: Settings,
    pub history: History,
    pub thumbs: HashMap<String, TextureHandle>,
    pub editor: Option<EditorState>,
    pub pending: Option<post_ui::Pending>,
    pub ocr_state: Option<editor_ui::OcrState>,
    pub model_dl: Option<settings_extra::ModelDownload>,
    pub confirm_uploads: bool,
    /// Credential Manager normally; an in-memory store in the demo so nothing real is touched.
    pub secrets: Arc<dyn SecretStore>,
    pub secret_state: HashMap<String, bool>,
    pub secret_input: HashMap<String, String>,
    pub toast_tx: Sender<(String, bool)>,
    pub toast_rx: Receiver<(String, bool)>,
    pub toast: Option<Toast>,
    pub icon: Option<TextureHandle>,
    pub cap_tx: Sender<CapMsg>,
    pub cap_rx: Receiver<CapMsg>,
    pub capturing: bool,
    pub window_hidden: bool,
    pub hotkeys: Option<HotkeyManager>,
    pub hotkey_rx: Receiver<HotAction>,
    pub hotkey_report: Vec<Registered>,
    pub tray: Option<tray::Tray>,
    pub update: update::Shared,
    pub monitors: Vec<Monitor>,
    pub monitor_choice: usize,
    pub search: String,
    pub confirm_clear: bool,
    pub confirm_close_editor: bool,
    pub hotkey_text: Vec<String>,
    pub recording: Option<usize>,
    pub tour: Option<demo_tour::Tour>,
    pub demo_counter: usize,
    pub quit: bool,
    /// One-shot scroll position for the settings page (used by the screenshot tour).
    pub settings_scroll: Option<f32>,
    /// Frames left before the window hides itself to the tray (start with `--minimized`).
    pub hide_after_frames: Option<u8>,
    pub desktop_shortcut: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext, mode: Mode, flags: Flags) -> App {
        theme::install(&cc.egui_ctx);
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        let demo = flags.demo;
        let settings = if demo {
            // invented folder: the demo must not show the real user name or paths
            Settings {
                save_dir: PathBuf::from(r"C:\Users\demo\Pictures\SnapForge"),
                ..Settings::default()
            }
        } else {
            Settings::load()
        };
        let history = History::open(&paths::history_dir());
        let (cap_tx, cap_rx) = channel();
        let (hk_tx, hotkey_rx) = channel();
        let (toast_tx, toast_rx) = channel();
        let is_app = matches!(mode, Mode::App);
        let hotkeys = (is_app && !demo).then(|| {
            let ctx = cc.egui_ctx.clone();
            HotkeyManager::spawn(move |a| {
                let _ = hk_tx.send(a);
                ctx.request_repaint();
            })
        });
        let icon = icon_image().map(|img| cc.egui_ctx.load_texture("app-icon", img, egui::TextureOptions::LINEAR));
        let mut app = App {
            mode,
            view: View::Home,
            tour: flags.screenshots.clone().map(demo_tour::Tour::new),
            settings,
            history,
            thumbs: HashMap::new(),
            editor: None,
            pending: None,
            ocr_state: None,
            model_dl: None,
            confirm_uploads: false,
            secrets: if demo {
                Arc::new(MemoryStore::default())
            } else {
                Arc::new(CredentialManager)
            },
            secret_state: HashMap::new(),
            secret_input: HashMap::new(),
            toast_tx,
            toast_rx,
            toast: None,
            icon,
            cap_tx,
            cap_rx,
            capturing: false,
            window_hidden: false,
            hotkeys,
            hotkey_rx,
            hotkey_report: Vec::new(),
            tray: None,
            update: Arc::new(Mutex::new(update::State::Idle)),
            monitors: sys::monitors(),
            monitor_choice: 0,
            search: String::new(),
            confirm_clear: false,
            confirm_close_editor: false,
            hotkey_text: Vec::new(),
            recording: None,
            demo_counter: 0,
            quit: false,
            settings_scroll: None,
            hide_after_frames: None,
            desktop_shortcut: true,
            flags,
        };
        app.localize_preset_names();
        if is_app && !demo {
            app.apply_hotkeys();
            app.tray = tray::Tray::new(&cc.egui_ctx, app.settings.language);
            app.purge_history();
            app.maybe_check_update(&cc.egui_ctx);
            // a window that is hidden before its first frame never gets any: hide it a few frames in
            if app.flags.minimized && app.settings.minimize_to_tray && app.tray.is_some() {
                app.hide_after_frames = Some(4);
            }
        }
        if demo {
            for i in 0..9 {
                let img = demo::demo_image(i, 960, 600);
                let _ = app
                    .history
                    .add(&img, demo::app_name(i), 1_790_000_000 + i as u64 * 86_400 * 2);
            }
        }
        core_diag::log(
            core_diag::Level::Info,
            &format!("started, mode={}", if demo { "demo" } else { "normal" }),
        );
        app
    }

    /// The built-in presets are named in the interface language; presets renamed by the user are left alone.
    pub fn localize_preset_names(&mut self) {
        const NAMES: [(&str, &str); 3] = [
            ("Editor", "Редактор"),
            ("Save + clipboard", "Сохранить и в буфер"),
            ("Private", "Приватный"),
        ];
        let ru = self.settings.language == Lang::Ru;
        for p in &mut self.settings.post.presets {
            for (en, rus) in NAMES {
                if ru && p.name == en {
                    p.name = rus.to_owned();
                } else if !ru && p.name == rus {
                    p.name = en.to_owned();
                }
            }
        }
    }

    pub fn t(&self, key: &'static str) -> &'static str {
        i18n::tr(self.settings.language, key)
    }

    pub fn tf(&self, key: &'static str, args: &[(&str, String)]) -> String {
        let mut s = self.t(key).to_owned();
        for (k, v) in args {
            s = s.replace(&format!("{{{k}}}"), v);
        }
        s
    }

    pub fn notify(&mut self, text: impl Into<String>, error: bool) {
        self.toast = Some(Toast {
            text: text.into(),
            error,
            since: Instant::now(),
        });
    }

    /// Saves settings (never in demo mode: nothing real is written there).
    pub fn save_settings(&mut self) {
        if self.flags.demo {
            return;
        }
        if let Err(e) = self.settings.save() {
            let msg = format!("{}: {e}", self.t("err.settings_save"));
            self.notify(msg, true);
        }
    }

    pub fn apply_hotkeys(&mut self) {
        let Some(m) = &self.hotkeys else { return };
        let bindings = if self.settings.hotkeys_enabled {
            self.settings.active_profile().bindings.clone()
        } else {
            Vec::new()
        };
        self.hotkey_report = m.set_bindings(bindings);
        self.hotkey_text = self
            .settings
            .active_profile()
            .bindings
            .iter()
            .map(|b| b.combo.to_string())
            .collect();
        for r in self.hotkey_report.iter().filter(|r| r.error.is_some()) {
            core_diag::log(
                core_diag::Level::Warn,
                &format!("hotkey {} is taken by another program", r.combo),
            );
        }
    }

    pub fn purge_history(&mut self) {
        let now = unix_now();
        if let Err(e) = self.history.purge_older_than(self.settings.history_retention_days, now) {
            core_diag::log(core_diag::Level::Warn, &format!("history purge failed: {e}"));
        }
    }

    pub fn maybe_check_update(&mut self, ctx: &egui::Context) {
        if !self.settings.check_updates || self.flags.demo {
            return;
        }
        let now = unix_now();
        if now.saturating_sub(self.settings.last_update_check) < update::CHECK_INTERVAL_SECS {
            return;
        }
        self.settings.last_update_check = now;
        self.save_settings();
        let ctx = ctx.clone();
        update::check(self.update.clone(), false, move || ctx.request_repaint());
    }

    pub fn thumb(&mut self, ctx: &egui::Context, id: &str) -> Option<TextureHandle> {
        if let Some(t) = self.thumbs.get(id) {
            return Some(t.clone());
        }
        let img = snapforge_core::image::open(self.history.thumb_path(id))
            .ok()?
            .to_rgba8();
        let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
        let tex = ctx.load_texture(format!("thumb-{id}"), ci, egui::TextureOptions::LINEAR);
        self.thumbs.insert(id.to_owned(), tex.clone());
        Some(tex)
    }

    fn handle_events(&mut self, ctx: &egui::Context) {
        if let Some(n) = self.hide_after_frames {
            if n == 0 {
                ctx.send_viewport_cmd(ViewportCommand::Visible(false));
                self.window_hidden = true;
                self.hide_after_frames = None;
            } else {
                self.hide_after_frames = Some(n - 1);
                ctx.request_repaint();
            }
        }
        while let Ok(a) = self.hotkey_rx.try_recv() {
            self.start_capture(ctx, a);
        }
        while let Ok(m) = self.cap_rx.try_recv() {
            self.on_capture_msg(ctx, m);
        }
        self.post_poll(ctx);
        while let Ok((text, error)) = self.toast_rx.try_recv() {
            self.notify(text, error);
        }
        for ev in tray::poll() {
            match ev {
                tray::TrayEvent::Show => self.show_window(ctx),
                tray::TrayEvent::Capture(a) => self.start_capture(ctx, a),
                tray::TrayEvent::Quit => self.quit = true,
            }
        }
        if ctx.input(|i| i.viewport().close_requested())
            && !self.quit
            && self.settings.minimize_to_tray
            && self.tray.is_some()
            && !self.flags.demo
        {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            self.window_hidden = true;
        }
        if self.quit {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        if let Some(t) = &self.toast {
            if t.since.elapsed() > Duration::from_secs(5) {
                self.toast = None;
            } else {
                ctx.request_repaint_after(Duration::from_millis(500));
            }
        }
    }

    pub fn show_window(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
        self.window_hidden = false;
    }
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn icon_image() -> Option<egui::ColorImage> {
    let bytes = include_bytes!("../assets/icon64.rgba");
    (bytes.len() == 64 * 64 * 4).then(|| egui::ColorImage::from_rgba_unmultiplied([64, 64], bytes))
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        match &self.mode {
            Mode::Install(_) | Mode::Uninstall(_) => self.installer_ui(ui),
            Mode::App => {
                self.handle_events(&ctx);
                self.app_ui(ui);
                self.tour_step(&ctx);
            }
        }
    }
}

fn parse_flags() -> (Flags, bool, bool, bool) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    let screenshots = args
        .iter()
        .position(|a| a == "--screenshots")
        .map(|i| PathBuf::from(args.get(i + 1).map(String::as_str).unwrap_or("docs/screenshots")));
    let flags = Flags {
        demo: has("--demo") || screenshots.is_some(),
        screenshots,
        minimized: has("--minimized"),
    };
    (flags, has("--update"), has("--uninstall"), has("--install"))
}

/// Returns false if another copy of the app is already running.
fn single_instance() -> bool {
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::core::w;
    // SAFETY: the handle is intentionally kept for the lifetime of the process.
    unsafe {
        match CreateMutexW(None, false, w!("Local\\SnapForge.SingleInstance")) {
            Ok(_) => GetLastError() != ERROR_ALREADY_EXISTS,
            Err(_) => true,
        }
    }
}

fn main() -> eframe::Result {
    diag::install_panic_hook();
    let (flags, headless_update, uninstall, install_flag) = parse_flags();
    if headless_update {
        attach_console();
        std::process::exit(update::run_headless());
    }
    if flags.demo {
        // nothing real is read or written in demo mode: a throw-away data folder
        let dir = std::env::temp_dir().join(format!("snapforge-demo-{}", std::process::id()));
        // SAFETY: set once at startup, before any other thread exists.
        unsafe { std::env::set_var("SNAPFORGE_DATA", &dir) };
    }
    if flags.screenshots.is_some() {
        attach_console();
    }
    core_diag::init(paths::logs_dir());
    update::cleanup_old();
    let mode = if uninstall {
        Mode::Uninstall(installer::Uninstall::default())
    } else if install_flag || winint::is_setup_launch() {
        Mode::Install(installer::Install::default())
    } else {
        Mode::App
    };
    if matches!(mode, Mode::App) && !flags.demo && !single_instance() {
        return Ok(());
    }
    let (size, min) = match mode {
        Mode::App => ([1180.0, 760.0], [860.0, 560.0]),
        Mode::Install(_) => ([560.0, 420.0], [560.0, 420.0]),
        Mode::Uninstall(_) => ([520.0, 300.0], [520.0, 300.0]),
    };
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size(size)
        .with_min_inner_size(min)
        .with_title("SnapForge")
        .with_app_id("SnapForge");
    if let Some(img) = icon_image() {
        viewport = viewport.with_icon(egui::IconData {
            rgba: img.as_raw().to_vec(),
            width: 64,
            height: 64,
        });
    }
    if !matches!(mode, Mode::App) {
        viewport = viewport.with_resizable(false).with_maximize_button(false);
    }
    let opts = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "SnapForge",
        opts,
        Box::new(move |cc| Ok(Box::new(App::new(cc, mode, flags)))),
    )
}

/// A GUI-subsystem exe has no console; attach to the parent's so `--update` / `--screenshots` can print.
fn attach_console() {
    use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    // SAFETY: harmless if the parent has no console.
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

pub fn lang_name(l: Lang) -> &'static str {
    match l {
        Lang::Ru => "Русский",
        Lang::En => "English",
    }
}
