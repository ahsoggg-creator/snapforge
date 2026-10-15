//! The plugins screen: installed plugins, a code editor with highlighting, templates, a test run on a generated
//! picture with a console, permission approval, import and export of `.sfplugin` archives.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use eframe::egui::{self, RichText, TextureHandle, Ui, vec2};
use snapforge_core::RgbaImage;
use snapforge_core::plugin::host::{DryEnv, HostEvent, PluginHost, PluginReport, SystemEnv, console_text};
use snapforge_core::plugin::manifest::{Manifest, Permissions};
use snapforge_core::plugin::package;
use snapforge_core::plugin::runtime::{Env, Limits, Plugin};
use snapforge_core::plugin::store::{Installed, Store, TEMPLATES};
use snapforge_core::settings::Lang;

use crate::i18n::tr;
use crate::{App, demo, highlight, theme};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Script,
    Manifest,
}

pub struct RunResult {
    pub console: String,
    pub image: Option<RgbaImage>,
    pub error: Option<String>,
    pub elapsed_ms: u128,
}

pub struct InstallDialog {
    pub archive: PathBuf,
    pub manifest: Manifest,
    pub perms: Permissions,
}

pub struct PluginsState {
    pub store: Store,
    pub host: Arc<PluginHost>,
    pub selected: Option<String>,
    pub tab: Tab,
    pub manifest_text: String,
    pub script_text: String,
    pub loaded_id: Option<String>,
    pub dirty: bool,
    pub error: Option<String>,
    pub console: String,
    pub run_fn: String,
    pub dry_run: bool,
    pub template: usize,
    pub install: Option<InstallDialog>,
    pub events_tx: Sender<HostEvent>,
    pub events_rx: Receiver<HostEvent>,
    pub reports_tx: Sender<PluginReport>,
    pub reports_rx: Receiver<PluginReport>,
    pub run_rx: Option<Receiver<RunResult>>,
    pub result_tex: Option<TextureHandle>,
    pub result_note: String,
    pub text_window: Option<(String, String)>,
    pub new_menu: bool,
}

impl PluginsState {
    pub fn new(root: PathBuf) -> PluginsState {
        let (events_tx, events_rx) = channel();
        let (reports_tx, reports_rx) = channel();
        let mut s = PluginsState {
            store: Store::new(root),
            host: Arc::new(PluginHost::empty()),
            selected: None,
            tab: Tab::Script,
            manifest_text: String::new(),
            script_text: String::new(),
            loaded_id: None,
            dirty: false,
            error: None,
            console: String::new(),
            run_fn: String::new(),
            dry_run: true,
            template: 0,
            install: None,
            events_tx,
            events_rx,
            reports_tx,
            reports_rx,
            run_rx: None,
            result_tex: None,
            result_note: String::new(),
            text_window: None,
            new_menu: false,
        };
        s.reload(false);
        s
    }

    /// Rebuilds the set of running plugins from disk. In the demo nothing real is reachable (dry environment).
    pub fn reload(&mut self, demo: bool) {
        let env: Arc<dyn Env> = if demo {
            Arc::new(DryEnv {
                events: self.events_tx.clone(),
            })
        } else {
            Arc::new(SystemEnv {
                events: self.events_tx.clone(),
            })
        };
        self.host = Arc::new(PluginHost::load(&self.store, env, &Limits::default()));
    }
}

/// Human-readable permission lines.
pub fn perm_lines(lang: Lang, p: &Permissions) -> Vec<String> {
    let list = |v: &[PathBuf]| v.iter().map(|x| x.display().to_string()).collect::<Vec<_>>().join(", ");
    let mut out = Vec::new();
    if p.is_empty() {
        out.push(tr(lang, "perm.none").to_owned());
    }
    if p.clipboard {
        out.push(tr(lang, "perm.clipboard").to_owned());
    }
    if p.ui {
        out.push(tr(lang, "perm.ui").to_owned());
    }
    if !p.fs_read.is_empty() {
        out.push(format!("{} {}", tr(lang, "perm.fs_read"), list(&p.fs_read)));
    }
    if !p.fs_write.is_empty() {
        out.push(format!("{} {}", tr(lang, "perm.fs_write"), list(&p.fs_write)));
    }
    if !p.net.is_empty() {
        out.push(format!("{} {}", tr(lang, "perm.net"), p.net.join(", ")));
    }
    out
}

impl App {
    /// Applies what plugins asked the app to show and what their hooks reported.
    pub fn plugins_poll(&mut self, ctx: &egui::Context) {
        while let Ok(ev) = self.plugins.events_rx.try_recv() {
            match ev {
                HostEvent::Notify(t) => self.notify(t, false),
                HostEvent::ShowText { title, text } => self.plugins.text_window = Some((title, text)),
            }
        }
        let mut errors = Vec::new();
        while let Ok(r) = self.plugins.reports_rx.try_recv() {
            let text = console_text(&r);
            if !text.is_empty() || r.error.is_some() {
                self.plugins
                    .console
                    .push_str(&format!("— {} · {}\n{}", r.plugin, r.hook, text));
            }
            if let Some(e) = &r.error {
                errors.push(format!("{}: {e}", r.plugin));
            }
        }
        if let Some(e) = errors.into_iter().next() {
            self.notify(format!("{}: {e}", self.t("plug.hook_failed")), true);
        }
        if let Some(rx) = &self.plugins.run_rx
            && let Ok(res) = rx.try_recv()
        {
            self.plugins.run_rx = None;
            self.plugins.console.push_str(&res.console);
            self.plugins.result_note = match &res.error {
                Some(e) => format!("✖ {e}"),
                None => format!("✔ {} ms", res.elapsed_ms),
            };
            self.plugins.result_tex = res.image.map(|img| {
                let ci = egui::ColorImage::from_rgba_unmultiplied(
                    [img.width() as usize, img.height() as usize],
                    img.as_raw(),
                );
                ctx.load_texture("plugin-result", ci, egui::TextureOptions::LINEAR)
            });
        }
    }

    pub fn plugin_select(&mut self, id: Option<String>) {
        let p = &mut self.plugins;
        p.selected = id.clone();
        p.dirty = false;
        p.error = None;
        p.result_tex = None;
        p.result_note.clear();
        p.run_fn.clear();
        match id.and_then(|i| p.store.read_files(&i).ok().map(|f| (i, f))) {
            Some((i, (m, s))) => {
                p.loaded_id = Some(i);
                p.manifest_text = m;
                p.script_text = s;
            }
            None => {
                p.loaded_id = None;
                p.manifest_text.clear();
                p.script_text.clear();
            }
        }
    }

    pub fn plugins_view(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let lang = self.settings.language;
        let list: Vec<Installed> = self.plugins.store.list();
        if self.plugins.selected.is_some() && self.plugins.loaded_id != self.plugins.selected {
            let id = self.plugins.selected.clone();
            self.plugin_select(id);
        }
        let mut act: Option<PlugAct> = None;

        egui::Panel::left("plugin_list").exact_size(250.0).show(ui, |ui| {
            ui.label(RichText::new(tr(lang, "plug.installed")).strong());
            if list.is_empty() {
                ui.label(RichText::new(tr(lang, "plug.none")).weak());
            }
            for p in &list {
                ui.horizontal(|ui| {
                    let mut on = p.enabled && p.approved;
                    if ui
                        .checkbox(&mut on, "")
                        .on_hover_text(tr(lang, "plug.enable"))
                        .changed()
                    {
                        act = Some(PlugAct::Toggle(p.id.clone(), on));
                    }
                    let label = format!("{} {}", p.manifest.name, p.manifest.version);
                    if ui
                        .selectable_label(self.plugins.selected.as_deref() == Some(p.id.as_str()), label)
                        .clicked()
                    {
                        act = Some(PlugAct::Select(p.id.clone()));
                    }
                    if !p.approved {
                        ui.colored_label(theme::WARN, "!")
                            .on_hover_text(tr(lang, "plug.needs_approval"));
                    }
                });
            }
            ui.separator();
            ui.label(RichText::new(tr(lang, "plug.new")).strong());
            egui::ComboBox::from_id_salt("plug_tpl")
                .selected_text(TEMPLATES[self.plugins.template].title)
                .show_ui(ui, |ui| {
                    for (i, t) in TEMPLATES.iter().enumerate() {
                        ui.selectable_value(&mut self.plugins.template, i, t.title);
                    }
                });
            if ui.button(tr(lang, "plug.create")).clicked() {
                act = Some(PlugAct::Create);
            }
            if ui.button(tr(lang, "plug.import")).clicked() {
                act = Some(PlugAct::Import);
            }
            ui.separator();
            ui.label(RichText::new(tr(lang, "plug.menu")).strong());
            let menus = self.plugins.host.menus();
            if menus.is_empty() {
                ui.label(RichText::new(tr(lang, "plug.no_menu")).weak());
            }
            for m in menus {
                if ui.button(format!("{} — {}", m.plugin_name, m.title)).clicked() {
                    act = Some(PlugAct::Menu(m.plugin_id, m.id));
                }
            }
            for (name, err) in &self.plugins.host.load_errors {
                ui.colored_label(theme::BAD, format!("{name}: {err}"));
            }
        });

        egui::CentralPanel::default().show(ui, |ui| {
            let Some(sel) = list.iter().find(|p| Some(&p.id) == self.plugins.selected.as_ref()) else {
                ui.add_space(30.0);
                ui.label(RichText::new(tr(lang, "plug.pick")).weak());
                ui.label(RichText::new(tr(lang, "plug.sandbox_note")).weak().small());
                return;
            };
            ui.horizontal(|ui| {
                ui.label(RichText::new(&sel.manifest.name).size(18.0).strong());
                ui.label(RichText::new(format!("{} · {}", sel.manifest.version, sel.manifest.author)).weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(tr(lang, "plug.delete")).clicked() {
                        act = Some(PlugAct::Delete(sel.id.clone()));
                    }
                    if ui.button(tr(lang, "plug.export")).clicked() {
                        act = Some(PlugAct::Export(sel.id.clone()));
                    }
                });
            });
            if !sel.manifest.description.is_empty() {
                ui.label(RichText::new(&sel.manifest.description).weak());
            }
            // permissions: shown for every plugin, and the approval gate when they changed
            theme::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(tr(lang, "plug.requests")).strong());
                for l in perm_lines(lang, &sel.perms) {
                    ui.label(format!("• {l}"));
                }
                if !sel.approved {
                    ui.colored_label(theme::WARN, tr(lang, "plug.needs_approval"));
                    if theme::primary_button(ui, tr(lang, "plug.approve")).clicked() {
                        act = Some(PlugAct::Approve(sel.id.clone()));
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.plugins.tab, Tab::Script, "main.rhai");
                ui.selectable_value(&mut self.plugins.tab, Tab::Manifest, "plugin.toml");
                if theme::primary_button(ui, tr(lang, "plug.save")).clicked() {
                    act = Some(PlugAct::Save);
                }
                if self.plugins.dirty {
                    ui.colored_label(theme::WARN, tr(lang, "plug.unsaved"));
                }
            });
            if let Some(e) = &self.plugins.error {
                ui.colored_label(theme::BAD, e);
            }
            let toml = self.plugins.tab == Tab::Manifest;
            let font = egui::FontId::monospace(13.0);
            let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, wrap: f32| {
                ui.fonts_mut(|f| f.layout_job(highlight::layout(text.as_str(), toml, font.clone(), wrap)))
            };
            let height = (ui.available_height() - 210.0).max(160.0);
            egui::ScrollArea::vertical().max_height(height).show(ui, |ui| {
                let text = if toml {
                    &mut self.plugins.manifest_text
                } else {
                    &mut self.plugins.script_text
                };
                let r = ui.add(
                    egui::TextEdit::multiline(text)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .desired_rows(14)
                        .layouter(&mut layouter),
                );
                if r.changed() {
                    self.plugins.dirty = true;
                }
            });
            ui.separator();
            let funcs = self.plugin_functions(&sel.id);
            ui.horizontal(|ui| {
                ui.label(tr(lang, "plug.run_fn"));
                if self.plugins.run_fn.is_empty() || !funcs.contains(&self.plugins.run_fn) {
                    self.plugins.run_fn = funcs.first().cloned().unwrap_or_default();
                }
                egui::ComboBox::from_id_salt("plug_fn")
                    .selected_text(self.plugins.run_fn.clone())
                    .show_ui(ui, |ui| {
                        for f in &funcs {
                            ui.selectable_value(&mut self.plugins.run_fn, f.clone(), f);
                        }
                    });
                ui.checkbox(&mut self.plugins.dry_run, tr(lang, "plug.dry"))
                    .on_hover_text(tr(lang, "plug.dry_hint"));
                let busy = self.plugins.run_rx.is_some();
                if ui
                    .add_enabled(!busy && !funcs.is_empty(), egui::Button::new(tr(lang, "plug.run")))
                    .clicked()
                {
                    act = Some(PlugAct::Run(sel.id.clone()));
                }
                if busy {
                    ui.spinner();
                }
                ui.label(&self.plugins.result_note);
            });
            let console_w = ui.available_width() - if self.plugins.result_tex.is_some() { 200.0 } else { 0.0 };
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(console_w);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(tr(lang, "plug.console")).strong());
                        if ui.small_button(tr(lang, "plug.clear")).clicked() {
                            self.plugins.console.clear();
                        }
                    });
                    egui::ScrollArea::vertical()
                        .id_salt("plug_console")
                        .max_height(150.0)
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            let mut c = self.plugins.console.clone();
                            ui.add(
                                egui::TextEdit::multiline(&mut c)
                                    .font(egui::FontId::monospace(12.0))
                                    .desired_rows(7)
                                    .desired_width(console_w - 12.0)
                                    .interactive(true),
                            );
                        });
                });
                if let Some(t) = &self.plugins.result_tex {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(tr(lang, "plug.result")).strong());
                        ui.add(egui::Image::new(t).fit_to_exact_size(vec2(180.0, 120.0)));
                    });
                }
            });
        });

        self.plugin_install_dialog(&ctx, &mut act);
        if let Some((title, text)) = self.plugins.text_window.clone() {
            let mut open = true;
            egui::Window::new(title)
                .open(&mut open)
                .default_width(420.0)
                .show(&ctx, |ui| {
                    let mut t = text;
                    ui.add(
                        egui::TextEdit::multiline(&mut t)
                            .desired_rows(8)
                            .desired_width(f32::INFINITY),
                    );
                });
            if !open {
                self.plugins.text_window = None;
            }
        }
        if let Some(a) = act {
            self.plugin_act(&ctx, a);
        }
    }

    /// Hooks, actions, menus and tools the selected plugin defines (what can be test-run).
    fn plugin_functions(&self, id: &str) -> Vec<String> {
        let Some(p) = self.plugins.store.get(id) else {
            return Vec::new();
        };
        let Ok(plugin) = Plugin::load(
            &p.dir,
            p.perms.clone(),
            Arc::new(DryEnv {
                events: self.plugins.events_tx.clone(),
            }),
            Limits::default(),
        ) else {
            return Vec::new();
        };
        plugin
            .functions()
            .into_iter()
            .filter(|f| {
                matches!(f.as_str(), "on_capture" | "on_before_save" | "on_after_save")
                    || f.starts_with("action_")
                    || f.starts_with("menu_")
                    || f.starts_with("tool_")
            })
            .collect()
    }

    fn plugin_install_dialog(&mut self, ctx: &egui::Context, act: &mut Option<PlugAct>) {
        let lang = self.settings.language;
        let Some(d) = &self.plugins.install else { return };
        let (name, version, author) = (
            d.manifest.name.clone(),
            d.manifest.version.clone(),
            d.manifest.author.clone(),
        );
        let lines = perm_lines(lang, &d.perms);
        let mut choice = None;
        egui::Window::new(tr(lang, "plug.install_title"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.label(RichText::new(format!("{name} {version}")).strong().size(16.0));
                if !author.is_empty() {
                    ui.label(RichText::new(author).weak());
                }
                ui.add_space(4.0);
                ui.label(tr(lang, "plug.requests"));
                for l in lines {
                    ui.label(format!("• {l}"));
                }
                ui.add_space(4.0);
                ui.label(RichText::new(tr(lang, "plug.install_warn")).weak().small());
                ui.horizontal(|ui| {
                    if theme::primary_button(ui, tr(lang, "plug.install_allow")).clicked() {
                        choice = Some(true);
                    }
                    if ui.button(tr(lang, "plug.install_off")).clicked() {
                        choice = Some(false);
                    }
                    if ui.button(tr(lang, "common.cancel")).clicked() {
                        *act = Some(PlugAct::CancelInstall);
                    }
                });
            });
        if let Some(approved) = choice {
            *act = Some(PlugAct::Install(approved));
        }
    }

    fn plugin_act(&mut self, ctx: &egui::Context, a: PlugAct) {
        let demo = self.flags.demo;
        match a {
            PlugAct::Select(id) => self.plugin_select(Some(id)),
            PlugAct::Toggle(id, on) => {
                let r = if on {
                    // enabling an unapproved plugin means approving what it asks for: only via the explicit button
                    match self.plugins.store.get(&id) {
                        Some(p) if p.approved => self.plugins.store.set_enabled(&id, true),
                        _ => Err(self.t("plug.needs_approval").to_owned()),
                    }
                } else {
                    self.plugins.store.set_enabled(&id, false)
                };
                match r {
                    Ok(()) => self.plugins.reload(demo),
                    Err(e) => self.notify(e, true),
                }
            }
            PlugAct::Approve(id) => match self.plugins.store.approve(&id) {
                Ok(()) => self.plugins.reload(demo),
                Err(e) => self.notify(e, true),
            },
            PlugAct::Create => {
                let t = &TEMPLATES[self.plugins.template];
                match self.plugins.store.create(t, false) {
                    Ok(id) => {
                        self.plugin_select(Some(id));
                        self.plugins.reload(demo);
                    }
                    Err(e) => self.notify(e, true),
                }
            }
            PlugAct::Delete(id) => {
                let _ = self.plugins.store.remove(&id);
                self.plugin_select(None);
                self.plugins.reload(demo);
            }
            PlugAct::Export(id) => {
                let dlg = rfd::FileDialog::new()
                    .add_filter("SnapForge plugin", &[package::EXT])
                    .set_file_name(format!("{id}.{}", package::EXT));
                if let Some(path) = dlg.save_file() {
                    match self.plugins.store.export(&id, &path) {
                        Ok(()) => self.notify(format!("{}: {}", self.t("plug.exported"), path.display()), false),
                        Err(e) => self.notify(e, true),
                    }
                }
            }
            PlugAct::Import => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("SnapForge plugin", &[package::EXT])
                    .pick_file()
                {
                    match package::read_manifest(&path)
                        .and_then(|m| m.permissions.resolve().map(|p| (m, p)).map_err(|e| e.to_string()))
                    {
                        Ok((manifest, perms)) => {
                            self.plugins.install = Some(InstallDialog {
                                archive: path,
                                manifest,
                                perms,
                            })
                        }
                        Err(e) => self.notify(e, true),
                    }
                }
            }
            PlugAct::CancelInstall => self.plugins.install = None,
            PlugAct::Install(approved) => {
                if let Some(d) = self.plugins.install.take() {
                    match self.plugins.store.install(&d.archive, approved) {
                        Ok(id) => {
                            self.plugin_select(Some(id));
                            self.plugins.reload(demo);
                        }
                        Err(e) => self.notify(e, true),
                    }
                }
            }
            PlugAct::Save => {
                let Some(id) = self.plugins.selected.clone() else {
                    return;
                };
                match self
                    .plugins
                    .store
                    .save_files(&id, &self.plugins.manifest_text, &self.plugins.script_text)
                {
                    Ok(()) => {
                        self.plugins.dirty = false;
                        self.plugins.error = None;
                        self.plugins.reload(demo);
                    }
                    Err(e) => self.plugins.error = Some(e),
                }
            }
            PlugAct::Menu(plugin, id) => {
                let host = self.plugins.host.clone();
                let tx = self.plugins.reports_tx.clone();
                let ctx2 = ctx.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(host.run_menu(&plugin, &id));
                    ctx2.request_repaint();
                });
            }
            PlugAct::Run(id) => self.plugin_test_run(ctx, &id),
        }
    }

    /// Test run: loads the saved plugin and calls the chosen function on a generated picture.
    pub fn plugin_test_run(&mut self, ctx: &egui::Context, id: &str) {
        let lang = self.settings.language;
        if self.plugins.dirty {
            self.notify(tr(lang, "plug.save_first"), true);
            return;
        }
        let Some(p) = self.plugins.store.get(id) else { return };
        if !p.approved {
            self.notify(tr(lang, "plug.needs_approval"), true);
            return;
        }
        let env: Arc<dyn Env> = if self.plugins.dry_run || self.flags.demo {
            Arc::new(DryEnv {
                events: self.plugins.events_tx.clone(),
            })
        } else {
            Arc::new(SystemEnv {
                events: self.plugins.events_tx.clone(),
            })
        };
        let func = self.plugins.run_fn.clone();
        let (tx, rx) = channel();
        let ctx2 = ctx.clone();
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let res = match Plugin::load(&p.dir, p.perms.clone(), env, Limits::default()) {
                Err(e) => RunResult {
                    console: format!("[error] {e}\n"),
                    image: None,
                    error: Some(e.to_string()),
                    elapsed_ms: started.elapsed().as_millis(),
                },
                Ok(plugin) => {
                    let test = demo::demo_image(0, 640, 400);
                    let out = plugin.call_named(&func, &test, (320, 200));
                    let mut console = format!("— {} · {func}\n", plugin.manifest.name);
                    for l in &out.console {
                        let tag = match l.level {
                            snapforge_core::plugin::Level::Print => "",
                            snapforge_core::plugin::Level::Log => "[log] ",
                            snapforge_core::plugin::Level::Error => "[error] ",
                        };
                        console.push_str(&format!("{tag}{}\n", l.text));
                    }
                    RunResult {
                        console,
                        image: out.image,
                        error: out.error.map(|e| e.to_string()),
                        elapsed_ms: out.elapsed.as_millis(),
                    }
                }
            };
            let _ = tx.send(res);
            ctx2.request_repaint();
        });
        self.plugins.run_rx = Some(rx);
        self.plugins.result_note.clear();
    }
}

enum PlugAct {
    Select(String),
    Toggle(String, bool),
    Approve(String),
    Create,
    Delete(String),
    Export(String),
    Import,
    CancelInstall,
    Install(bool),
    Save,
    Menu(String, String),
    Run(String),
}
