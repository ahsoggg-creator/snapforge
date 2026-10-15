//! The "after capture" panel: pick what happens to the shot, review what OCR / face detection would change,
//! then run the chain. Heavy work (analysis, saving, uploading) runs on a background thread.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, channel};

use eframe::egui::{self, Color32, RichText, Sense, Stroke, StrokeKind, TextureHandle, Ui, vec2};
use snapforge_core::RgbaImage;
use snapforge_core::post::{self, Analyzer, Approved, Feature, Findings, Preset, Sink, StepReport};
use snapforge_core::secrets::SecretStore;
use snapforge_core::settings::Settings;
use snapforge_core::upload::{self, FilePart, HttpRequest, HttpResponse, Transport, UreqTransport};
use snapforge_core::{diag, face, naming, output};

use crate::{App, View, theme, unix_now};

pub enum Stage {
    Choose,
    Analyzing,
    Review,
    Working,
    Done(Vec<StepReport>),
}

pub struct WorkResult {
    pub output: RgbaImage,
    pub reports: Vec<StepReport>,
    pub open_editor: bool,
    pub saved: Option<PathBuf>,
}

pub struct Pending {
    pub image: RgbaImage,
    /// The screen without annotations plus the layers drawn in the overlay (opened as editable layers).
    pub editable: Option<(RgbaImage, Vec<snapforge_core::editor::Object>)>,
    pub app: String,
    pub preset: Preset,
    pub stage: Stage,
    pub findings: Option<Findings>,
    pub faces_ok: Vec<bool>,
    pub sens_ok: Vec<bool>,
    pub analysis_rx: Option<Receiver<Findings>>,
    pub work_rx: Option<Receiver<WorkResult>>,
    pub preview: Option<TextureHandle>,
    pub model_rx: Option<Receiver<Result<(), String>>>,
    pub model_progress: Arc<(AtomicU64, AtomicU64)>,
    pub show_text: bool,
}

/// Real OCR / faces, or (in the demo) OCR only: the face model is never downloaded by the demo.
struct DemoAwareAnalyzer {
    demo: bool,
}

impl Analyzer for DemoAwareAnalyzer {
    fn ocr(&self, img: &RgbaImage, language: Option<&str>) -> Result<snapforge_core::sensitive::OcrData, String> {
        post::SystemAnalyzer.ocr(img, language)
    }

    fn faces(&self, img: &RgbaImage, threshold: f32) -> Result<Vec<snapforge_core::geometry::Rect>, String> {
        if self.demo && face::model_state() != face::ModelState::Ready {
            return Err("the face detection model is not installed".into());
        }
        post::SystemAnalyzer.faces(img, threshold)
    }
}

/// Output side of the chain on the worker thread. The editor is opened by the UI afterwards.
struct WorkerSink {
    settings: Settings,
    app: String,
    demo: bool,
    store: Arc<dyn SecretStore>,
    host: Arc<snapforge_core::plugin::host::PluginHost>,
    reports: std::sync::mpsc::Sender<snapforge_core::plugin::host::PluginReport>,
    saved: Option<PathBuf>,
    editor: bool,
}

struct NoNetwork;

impl Transport for NoNetwork {
    fn send(&self, _: &HttpRequest) -> Result<HttpResponse, String> {
        Err("the network is not used in the demo".into())
    }
}

impl Sink for WorkerSink {
    fn save(&mut self, img: &RgbaImage) -> Result<String, String> {
        if self.demo {
            return Ok("(demo: not written)".into());
        }
        let s = &self.settings;
        let stem = naming::expand(&s.name_template, &self.app, chrono::Local::now().naive_local());
        // plugins may adjust the picture right before it is written, and are told where it went
        let (marked, reports) = self.host.on_before_save(img.clone(), &self.app, s.format.ext());
        for r in reports {
            let _ = self.reports.send(r);
        }
        let path =
            output::save_to_dir(&marked, &s.save_dir, &stem, s.format, s.jpeg_quality).map_err(|e| e.to_string())?;
        for r in self.host.on_after_save(&path) {
            let _ = self.reports.send(r);
        }
        let note = path.display().to_string();
        self.saved = Some(path);
        Ok(note)
    }

    fn clipboard_image(&mut self, img: &RgbaImage) -> Result<String, String> {
        if self.demo {
            return Ok("(demo: clipboard untouched)".into());
        }
        output::copy_to_clipboard(img)
            .map(|()| String::new())
            .map_err(|e| e.to_string())
    }

    fn clipboard_text(&mut self, text: &str) -> Result<String, String> {
        if self.demo {
            return Ok("(demo: clipboard untouched)".into());
        }
        output::copy_text_to_clipboard(text)
            .map(|()| format!("{} characters", text.chars().count()))
            .map_err(|e| e.to_string())
    }

    fn upload(&mut self, img: &RgbaImage) -> Result<String, String> {
        let s = &self.settings;
        let stem = naming::expand(&s.name_template, &self.app, chrono::Local::now().naive_local());
        let name = format!("{stem}.{}", s.format.ext());
        let bytes = output::encode(img, s.format, s.jpeg_quality).map_err(|e| e.to_string())?;
        let file = FilePart {
            mime: upload::mime_for(&name).into(),
            name,
            bytes,
        };
        let outcome = if self.demo {
            upload::upload(&s.uploads, None, &file, self.store.as_ref(), &NoNetwork)
        } else {
            upload::upload(&s.uploads, None, &file, self.store.as_ref(), &UreqTransport)
        }
        .map_err(|e| e.to_string())?;
        match outcome.link {
            Some(link) => {
                let copied = !self.demo && output::copy_text_to_clipboard(&link).is_ok();
                Ok(format!("{link}{}", if copied { " (link copied)" } else { "" }))
            }
            None => Ok(outcome.note),
        }
    }

    fn open_editor(&mut self, _: &RgbaImage) -> Result<String, String> {
        self.editor = true;
        Ok(String::new())
    }
}

impl App {
    /// Entry point after a capture: the panel (or the active preset straight away).
    pub fn begin_post(&mut self, ctx: &egui::Context, shot: crate::capture_flow::Shot) {
        let mut preset = self.settings.post.active_preset().clone();
        let shield = shot.privacy;
        if shield {
            // the shield in the overlay: hide sensitive text and faces for this shot. A missing face model is
            // reported on the review screen instead of silently skipping the faces
            preset.hide_sensitive = true;
            preset.blur_faces = true;
        }
        let editable = shot.base.map(|b| (b, shot.objects)).filter(|(_, o)| !o.is_empty());
        self.pending = Some(Pending {
            image: shot.image,
            app: shot.app,
            editable,
            preset,
            stage: Stage::Choose,
            findings: None,
            faces_ok: Vec::new(),
            sens_ok: Vec::new(),
            analysis_rx: None,
            work_rx: None,
            preview: None,
            model_rx: None,
            model_progress: Arc::default(),
            show_text: false,
        });
        self.view = View::PostCapture;
        if self.settings.post.show_panel && !shield {
            self.show_window(ctx);
        } else {
            // no panel: run the preset right away. The window comes up only when something needs the user (the
            // review of found faces, an error, the editor) or when it was open before the capture
            self.post_run(ctx);
            // the window stays away unless the user asked for it back (or there is no tray icon to find it by)
            if self.restore_window && (self.settings.post.restore_window || self.tray.is_none()) {
                self.show_window(ctx);
            }
        }
    }

    /// "Run": analysis first when the preset needs it, then the review, then the chain.
    pub fn post_run(&mut self, ctx: &egui::Context) {
        let demo = self.flags.demo;
        let opts = self.settings.post.options.clone();
        let Some(p) = &mut self.pending else { return };
        if p.preset.needs_analysis() && p.findings.is_none() {
            let (tx, rx) = channel();
            let (img, preset, ctx2) = (p.image.clone(), p.preset.clone(), ctx.clone());
            std::thread::spawn(move || {
                let f = post::analyze(&img, &preset, &opts, &DemoAwareAnalyzer { demo });
                let _ = tx.send(f);
                ctx2.request_repaint();
            });
            p.analysis_rx = Some(rx);
            p.stage = Stage::Analyzing;
        } else if p.findings.is_some()
            && matches!(p.stage, Stage::Choose | Stage::Analyzing)
            && (p.preset.blur_faces || p.preset.hide_sensitive)
        {
            p.stage = Stage::Review;
        } else {
            self.post_finish(ctx);
        }
    }

    pub fn post_finish(&mut self, ctx: &egui::Context) {
        let (host, reports_tx) = (self.plugins.host.clone(), self.plugins.reports_tx.clone());
        let (settings, demo, opts, store) = (
            self.settings.clone(),
            self.flags.demo,
            self.settings.post.options.clone(),
            self.secrets.clone(),
        );
        let Some(p) = &mut self.pending else { return };
        let approved = match &p.findings {
            Some(f) => Approved {
                faces: f
                    .faces
                    .iter()
                    .zip(&p.faces_ok)
                    .filter(|(_, ok)| **ok)
                    .map(|(r, _)| *r)
                    .collect(),
                sensitive: f
                    .sensitive
                    .iter()
                    .zip(&p.sens_ok)
                    .filter(|(_, ok)| **ok)
                    .map(|(m, _)| m.rect)
                    .collect(),
            },
            None => Approved::default(),
        };
        let ocr_text = p.findings.as_ref().and_then(|f| f.ocr_text.clone());
        let (img, preset, app, ctx2) = (p.image.clone(), p.preset.clone(), p.app.clone(), ctx.clone());
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut out = post::transform(&img, &preset, &opts, &approved);
            for key in &preset.plugin_actions {
                if let Some((plugin, action)) = key.split_once('/') {
                    let (new, report) = host.run_action(plugin, action, &out);
                    let _ = reports_tx.send(report);
                    if let Some(n) = new {
                        out = n;
                    }
                }
            }
            let mut sink = WorkerSink {
                host: host.clone(),
                reports: reports_tx.clone(),
                settings,
                app,
                demo,
                store,
                saved: None,
                editor: false,
            };
            let reports = post::deliver(&out, &preset, ocr_text.as_deref(), &mut sink);
            let _ = tx.send(WorkResult {
                output: out,
                reports,
                open_editor: sink.editor,
                saved: sink.saved,
            });
            ctx2.request_repaint();
        });
        p.work_rx = Some(rx);
        p.stage = Stage::Working;
    }

    /// Polls the background jobs of the pending shot.
    pub fn post_poll(&mut self, ctx: &egui::Context) {
        let review_wanted = self.settings.post.review_findings;
        let Some(p) = &mut self.pending else { return };
        if let Some(rx) = &p.analysis_rx
            && let Ok(f) = rx.try_recv()
        {
            p.faces_ok = vec![true; f.faces.len()];
            p.sens_ok = vec![true; f.sensitive.len()];
            // The review screen needs the user, so it brings the window up: only for what a person has to decide.
            // Nothing found means nothing to review, and with the review switched off everything found is hidden
            // at once. A feature that could not run (no model, no OCR language) is always shown, never skipped.
            let hides = p.preset.blur_faces || p.preset.hide_sensitive;
            let nothing_found = f.faces.is_empty() && f.sensitive.is_empty();
            let review = hides && (!f.errors.is_empty() || (review_wanted && !nothing_found));
            p.findings = Some(f);
            p.analysis_rx = None;
            if review {
                p.stage = Stage::Review;
                self.show_window(ctx);
            } else {
                self.post_finish(ctx);
            }
            return;
        }
        if let Some(rx) = &p.model_rx
            && let Ok(r) = rx.try_recv()
        {
            p.model_rx = None;
            match r {
                Ok(()) => {
                    // re-run the analysis now that the model exists
                    p.findings = None;
                    p.stage = Stage::Choose;
                    self.post_run(ctx);
                }
                Err(e) => self.notify(e, true),
            }
            return;
        }
        if let Some(rx) = &p.work_rx
            && let Ok(res) = rx.try_recv()
        {
            self.pending_done(ctx, res);
        }
    }

    fn pending_done(&mut self, ctx: &egui::Context, res: WorkResult) {
        let Some(p) = self.pending.take() else { return };
        let mut history_id = None;
        if self.settings.history_enabled {
            match self.history.add(&res.output, &p.app, unix_now()) {
                Ok(e) => history_id = Some(e.id),
                Err(e) => diag::log(diag::Level::Warn, &format!("history add failed: {e}")),
            }
        }
        let failed = res.reports.iter().filter(|r| r.result.is_err()).count();
        let summary: Vec<String> = res
            .reports
            .iter()
            .map(|r| match &r.result {
                Ok(note) if note.is_empty() => format!("{} ✔", self.t(step_key(r.step))),
                Ok(note) => format!("{} ✔ {note}", self.t(step_key(r.step))),
                Err(e) => format!("{} ✖ {e}", self.t(step_key(r.step))),
            })
            .collect();
        let quiet = !self.settings.post.show_panel && !res.open_editor && failed == 0;
        if quiet {
            // nothing to show: the shot was saved/copied as the preset says
            if self.view == View::PostCapture {
                self.view = View::Home;
            }
            return;
        }
        self.show_window(ctx);
        if res.open_editor {
            self.open_editor(ctx, res.output, p.app, history_id);
            if let Some(e) = &mut self.editor {
                e.saved_path = res.saved;
            }
            if !summary.is_empty() {
                self.notify(summary.join("   "), failed > 0);
            }
        } else {
            self.view = View::PostCapture;
            self.pending = Some(Pending {
                stage: Stage::Done(res.reports),
                editable: None,
                image: res.output,
                work_rx: None,
                ..p
            });
        }
    }

    pub fn post_view(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        if self.pending.is_none() {
            self.view = View::Home;
            return;
        }
        let lang = self.settings.language;
        let t = move |k: &'static str| crate::i18n::tr(lang, k);
        let mut act = None;
        // preview texture (built once per shot)
        if let Some(p) = &mut self.pending
            && p.preview.is_none()
        {
            let ci = egui::ColorImage::from_rgba_unmultiplied(
                [p.image.width() as usize, p.image.height() as usize],
                p.image.as_raw(),
            );
            p.preview = Some(ctx.load_texture("post-preview", ci, egui::TextureOptions::LINEAR));
        }
        let plugin_actions = self.plugins.host.actions();
        let model_ready = face::model_state() == face::ModelState::Ready;
        let (presets, active): (Vec<String>, usize) = (
            self.settings.post.presets.iter().map(|p| p.name.clone()).collect(),
            self.settings.post.active,
        );
        let Some(p) = &mut self.pending else { return };
        ui.add_space(4.0);
        ui.label(RichText::new(t("post.title")).size(20.0).strong());
        ui.add_space(4.0);
        ui.columns(2, |cols| {
            // left: the shot (with the findings drawn on it during the review)
            let ui = &mut cols[0];
            if let Some(tex) = &p.preview {
                let avail = ui.available_width().min(560.0);
                let k = (avail / p.image.width() as f32).min(1.0);
                let size = vec2(p.image.width() as f32 * k, p.image.height() as f32 * k);
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                ui.painter().image(
                    tex.id(),
                    rect,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                if let (Some(f), true) = (&p.findings, matches!(p.stage, Stage::Review)) {
                    let to = |r: &snapforge_core::geometry::Rect| {
                        egui::Rect::from_min_size(
                            rect.min + vec2(r.x as f32 * k, r.y as f32 * k),
                            vec2(r.w as f32 * k, r.h as f32 * k),
                        )
                    };
                    for (r, ok) in f.faces.iter().zip(&p.faces_ok) {
                        let c = if *ok { theme::GOOD } else { Color32::from_gray(120) };
                        ui.painter()
                            .rect_stroke(to(r), 2.0, Stroke::new(2.0, c), StrokeKind::Outside);
                    }
                    for (m, ok) in f.sensitive.iter().zip(&p.sens_ok) {
                        let c = if *ok { theme::WARN } else { Color32::from_gray(120) };
                        ui.painter()
                            .rect_stroke(to(&m.rect), 2.0, Stroke::new(2.0, c), StrokeKind::Outside);
                    }
                }
            }
            ui.label(
                RichText::new(format!("{}×{}", p.image.width(), p.image.height()))
                    .weak()
                    .small(),
            );

            // right: stage-specific controls
            let ui = &mut cols[1];
            match &mut p.stage {
                Stage::Choose => {
                    ui.horizontal(|ui| {
                        ui.label(t("post.preset"));
                        let mut idx = active;
                        egui::ComboBox::from_id_salt("preset_pick")
                            .selected_text(presets.get(idx).cloned().unwrap_or_default())
                            .show_ui(ui, |ui| {
                                for (i, n) in presets.iter().enumerate() {
                                    ui.selectable_value(&mut idx, i, n);
                                }
                            });
                        if idx != active {
                            act = Some(PostAct::Preset(idx));
                        }
                    });
                    let pr = &mut p.preset;
                    ui.label(RichText::new(t("post.transform")).strong());
                    ui.checkbox(&mut pr.trim_margins, t("post.trim"));
                    ui.checkbox(&mut pr.hide_sensitive, t("post.hide_sensitive"));
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut pr.blur_faces, t("post.blur_faces"));
                        if pr.blur_faces && !model_ready {
                            ui.colored_label(theme::WARN, t("post.model_missing"));
                        }
                    });
                    ui.checkbox(&mut pr.beautify, t("post.beautify"));
                    ui.checkbox(&mut pr.watermark, t("post.watermark"));
                    ui.checkbox(&mut pr.downscale, t("post.downscale"));
                    for e in &plugin_actions {
                        let key = format!("{}/{}", e.plugin_id, e.id);
                        let mut on = pr.plugin_actions.contains(&key);
                        if ui
                            .checkbox(&mut on, format!("{} — {}", e.plugin_name, e.title))
                            .changed()
                        {
                            pr.plugin_actions.retain(|k| k != &key);
                            if on {
                                pr.plugin_actions.push(key);
                            }
                        }
                    }
                    ui.add_space(4.0);
                    ui.label(RichText::new(t("post.outputs")).strong());
                    ui.checkbox(&mut pr.save, t("post.save"));
                    ui.checkbox(&mut pr.clipboard, t("post.clipboard"));
                    ui.checkbox(&mut pr.ocr_to_clipboard, t("post.ocr"));
                    ui.checkbox(&mut pr.open_editor, t("post.editor"));
                    let up_ready = self.settings.uploads.enabled && self.settings.uploads.active.is_some();
                    ui.add_enabled_ui(up_ready, |ui| {
                        ui.checkbox(&mut pr.upload, t("post.upload"));
                    });
                    if !up_ready {
                        pr.upload = false;
                        ui.label(RichText::new(t("post.upload_off")).weak().small());
                    }
                    if pr.clipboard && pr.ocr_to_clipboard {
                        ui.label(RichText::new(t("post.clip_conflict")).weak().small());
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if theme::primary_button(ui, t("post.run")).clicked() {
                            act = Some(PostAct::Run);
                        }
                        if ui.button(t("post.skip")).clicked() {
                            act = Some(PostAct::Skip);
                        }
                        if ui.button(t("post.discard")).clicked() {
                            act = Some(PostAct::Discard);
                        }
                    });
                    let mut show = self.settings.post.show_panel;
                    if ui.checkbox(&mut show, t("post.always_show")).changed() {
                        act = Some(PostAct::ShowPanel(show));
                    }
                }
                Stage::Analyzing => {
                    ui.add(egui::Spinner::new().size(28.0).color(theme::ACCENT));
                    ui.label(t("post.analyzing"));
                    ctx.request_repaint();
                }
                Stage::Review => {
                    ui.label(RichText::new(t("post.review")).strong());
                    ui.label(RichText::new(t("post.review_hint")).weak().small());
                    if let Some(f) = &p.findings {
                        for (feature, e) in &f.errors {
                            let faces = *feature == Feature::Faces;
                            // the core's own message is English; the missing-model case is shown translated
                            let shown = if faces && !model_ready {
                                t("set.model_missing").to_owned()
                            } else {
                                e.clone()
                            };
                            ui.colored_label(
                                theme::WARN,
                                format!("{}: {shown}", if faces { t("post.faces") } else { "OCR" }),
                            );
                            if *feature == Feature::Faces
                                && !model_ready
                                && p.model_rx.is_none()
                                && ui
                                    .button(format!("{} ({})", t("post.download_model"), face::MODEL_SIZE_HINT))
                                    .clicked()
                            {
                                act = Some(PostAct::DownloadModel);
                            }
                        }
                        if p.model_rx.is_some() {
                            let (done, total) = (
                                p.model_progress.0.load(Ordering::Relaxed),
                                p.model_progress.1.load(Ordering::Relaxed),
                            );
                            ui.add(
                                egui::ProgressBar::new(done as f32 / total.max(1) as f32).text(t("post.downloading")),
                            );
                            ctx.request_repaint();
                        }
                        if f.errors.iter().all(|(ft, _)| *ft != Feature::Faces) && p.preset.blur_faces {
                            ui.label(format!("{}: {}", t("post.faces"), f.faces.len()));
                            for (i, r) in f.faces.iter().enumerate() {
                                ui.checkbox(
                                    &mut p.faces_ok[i],
                                    format!("{} {} ({}×{})", t("post.face_n"), i + 1, r.w, r.h),
                                );
                            }
                        }
                        if p.preset.hide_sensitive && f.errors.iter().all(|(ft, _)| *ft != Feature::Ocr) {
                            ui.label(format!("{}: {}", t("post.sensitive"), f.sensitive.len()));
                            for (i, m) in f.sensitive.iter().enumerate() {
                                let kind = match m.kind {
                                    snapforge_core::sensitive::Kind::Email => t("post.kind_email"),
                                    snapforge_core::sensitive::Kind::Phone => t("post.kind_phone"),
                                    snapforge_core::sensitive::Kind::Secret => t("post.kind_secret"),
                                };
                                ui.checkbox(&mut p.sens_ok[i], format!("{kind}: {}", mask(&m.text)));
                            }
                        }
                        if let Some(text) = &f.ocr_text {
                            ui.checkbox(&mut p.show_text, t("post.show_text"));
                            if p.show_text {
                                let mut t_ = text.clone();
                                ui.add(
                                    egui::TextEdit::multiline(&mut t_)
                                        .desired_rows(4)
                                        .desired_width(f32::INFINITY),
                                );
                            }
                        }
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if theme::primary_button(ui, t("post.apply")).clicked() {
                            act = Some(PostAct::Finish);
                        }
                        if ui.button(t("post.back")).clicked() {
                            act = Some(PostAct::Back);
                        }
                    });
                }
                Stage::Working => {
                    ui.add(egui::Spinner::new().size(28.0).color(theme::ACCENT));
                    ui.label(t("post.working"));
                    ctx.request_repaint();
                }
                Stage::Done(reports) => {
                    ui.label(
                        RichText::new(format!("✔ {}", t("post.done")))
                            .size(18.0)
                            .color(theme::GOOD)
                            .strong(),
                    );
                    for r in reports.iter() {
                        match &r.result {
                            Ok(n) => ui.label(format!("✔ {} {n}", t(step_key(r.step)))),
                            Err(e) => ui.colored_label(theme::BAD, format!("✖ {}: {e}", t(step_key(r.step)))),
                        };
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if theme::primary_button(ui, t("post.open_editor")).clicked() {
                            act = Some(PostAct::OpenEditor);
                        }
                        if ui.button(t("ed.close")).clicked() {
                            act = Some(PostAct::Discard);
                        }
                    });
                }
            }
        });
        if let Some(a) = act {
            self.post_act(&ctx, a);
        }
    }

    fn post_act(&mut self, ctx: &egui::Context, a: PostAct) {
        match a {
            PostAct::Preset(i) => {
                self.settings.post.active = i;
                if let Some(p) = &mut self.pending {
                    p.preset = self.settings.post.presets[i].clone();
                    p.findings = None;
                }
                self.save_settings();
            }
            PostAct::ShowPanel(v) => {
                self.settings.post.show_panel = v;
                self.save_settings();
            }
            PostAct::Run => {
                // remember the ticked boxes on the active preset
                if let Some(p) = &self.pending {
                    let i = self.settings.post.active;
                    let name = self.settings.post.presets[i].name.clone();
                    self.settings.post.presets[i] = Preset {
                        name,
                        ..p.preset.clone()
                    };
                }
                self.save_settings();
                self.post_run(ctx);
            }
            PostAct::Finish => self.post_finish(ctx),
            PostAct::Back => {
                if let Some(p) = &mut self.pending {
                    p.stage = Stage::Choose;
                }
            }
            PostAct::Skip | PostAct::OpenEditor => {
                if let Some(p) = self.pending.take() {
                    let id = if self.settings.history_enabled {
                        self.history.add(&p.image, &p.app, unix_now()).ok().map(|e| e.id)
                    } else {
                        None
                    };
                    match p.editable {
                        Some((base, objects)) => self.open_editor_layers(ctx, base, objects, p.app, id),
                        None => self.open_editor(ctx, p.image, p.app, id),
                    }
                }
            }
            PostAct::Discard => {
                self.pending = None;
                self.view = View::Home;
            }
            PostAct::DownloadModel => {
                let Some(p) = &mut self.pending else { return };
                let (tx, rx) = channel();
                let prog = p.model_progress.clone();
                let ctx2 = ctx.clone();
                std::thread::spawn(move || {
                    let r = face::download_model(&|d, t| {
                        prog.0.store(d, Ordering::Relaxed);
                        prog.1.store(t, Ordering::Relaxed);
                        ctx2.request_repaint();
                    });
                    let _ = tx.send(r);
                    ctx2.request_repaint();
                });
                p.model_rx = Some(rx);
            }
        }
    }
}

enum PostAct {
    Preset(usize),
    ShowPanel(bool),
    Run,
    Finish,
    Back,
    Skip,
    OpenEditor,
    Discard,
    DownloadModel,
}

pub fn step_key(step: &str) -> &'static str {
    match step {
        "save" => "post.save",
        "clipboard" => "post.clipboard",
        "upload" => "post.upload",
        "ocr" => "post.ocr",
        _ => "post.editor",
    }
}

/// Shows the start and end of a found value so the user can recognise it without the full text on screen.
fn mask(s: &str) -> String {
    let n = s.chars().count();
    if n <= 6 {
        return "•".repeat(n);
    }
    let head: String = s.chars().take(2).collect();
    let tail: String = s.chars().skip(n - 2).collect();
    format!("{head}{}{tail}", "•".repeat(n.min(12) - 4))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real save path: a plugin's `on_before_save` changes what is written and `on_after_save` learns the path.
    #[test]
    fn saving_runs_the_plugin_hooks() {
        use snapforge_core::plugin::host::{DryEnv, PluginHost};
        use snapforge_core::plugin::runtime::Limits;
        use snapforge_core::plugin::store::{Store, TEMPLATES};
        let root = std::env::temp_dir().join(format!("sf-sink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = Store::new(root.join("plugins"));
        store.create(&TEMPLATES[1], true).unwrap(); // Watermark: on_before_save draws a mark
        let (etx, _erx) = std::sync::mpsc::channel();
        let host = Arc::new(PluginHost::load(
            &store,
            Arc::new(DryEnv { events: etx }),
            &Limits::default(),
        ));
        assert_eq!(host.len(), 1);
        let (rtx, rrx) = std::sync::mpsc::channel();
        let settings = Settings {
            save_dir: root.join("shots"),
            ..Settings::default()
        };
        let mut sink = WorkerSink {
            settings,
            app: "app".into(),
            demo: false,
            store: Arc::new(snapforge_core::secrets::MemoryStore::default()),
            host,
            reports: rtx,
            saved: None,
            editor: false,
        };
        let img = RgbaImage::from_pixel(800, 400, snapforge_core::image::Rgba([255, 255, 255, 255]));
        sink.save(&img).expect("save");
        let path = sink.saved.clone().expect("path");
        let written = snapforge_core::image::open(&path).unwrap().to_rgba8();
        if snapforge_core::editor::render::has_font() {
            assert_ne!(written, img, "the plugin's watermark is in the saved file");
        }
        assert_eq!(written.get_pixel(2, 2), img.get_pixel(2, 2));
        assert_eq!(
            rrx.try_iter().filter(|r| r.hook == "on_before_save").count(),
            1,
            "the hook reported"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn masking_hides_the_middle_of_secrets() {
        assert_eq!(mask("abc"), "•••");
        let m = mask("jane.doe@example.com");
        assert!(
            m.starts_with("ja") && m.ends_with("om") && !m.contains("example"),
            "{m}"
        );
    }

    #[test]
    fn step_names_map_to_translation_keys() {
        for s in ["save", "clipboard", "upload", "ocr", "editor"] {
            assert!(step_key(s).starts_with("post."));
        }
    }
}
