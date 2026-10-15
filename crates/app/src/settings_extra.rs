//! Settings cards added in 0.2.0: the after-capture chain (presets, watermark, OCR, face model) and uploads.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, channel};

use eframe::egui::{self, RichText, Ui};
use snapforge_core::face::{self, ModelState};
use snapforge_core::imageops::Corner;
use snapforge_core::post::{HideStyle, Preset};
use snapforge_core::upload::{self, DestKind, Destination, FilePart, HttpDest, S3Dest, UreqTransport};
use snapforge_core::{ocr, output};

use crate::i18n::tr;
use crate::{App, theme, unix_now};

pub struct ModelDownload {
    pub rx: Receiver<Result<(), String>>,
    pub progress: Arc<(AtomicU64, AtomicU64)>,
}

fn corner_key(c: Corner) -> &'static str {
    match c {
        Corner::TopLeft => "corner.top_left",
        Corner::TopRight => "corner.top_right",
        Corner::BottomRight => "corner.bottom_right",
        Corner::BottomLeft => "corner.bottom_left",
        Corner::Center => "corner.center",
    }
}

fn secret_label_key(name: &str) -> &'static str {
    match name {
        "webhook" => "up.secret.webhook",
        "access_key" => "up.secret.access_key",
        "secret_key" => "up.secret.secret_key",
        _ => "up.secret.token",
    }
}

impl App {
    pub fn post_card(&mut self, ui: &mut Ui, changed: &mut bool) {
        let lang = self.settings.language;
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(tr(lang, "set.post")).strong());
            *changed |= ui
                .checkbox(&mut self.settings.post.show_panel, tr(lang, "set.post_show"))
                .changed();

            ui.horizontal(|ui| {
                ui.label(tr(lang, "post.preset"));
                let names: Vec<String> = self.settings.post.presets.iter().map(|p| p.name.clone()).collect();
                let mut idx = self.settings.post.active;
                egui::ComboBox::from_id_salt("post_preset")
                    .selected_text(names.get(idx).cloned().unwrap_or_default())
                    .show_ui(ui, |ui| {
                        for (i, n) in names.iter().enumerate() {
                            ui.selectable_value(&mut idx, i, n);
                        }
                    });
                if idx != self.settings.post.active {
                    self.settings.post.active = idx;
                    *changed = true;
                }
                if ui.button(tr(lang, "set.preset_new")).clicked() {
                    let mut p: Preset = self.settings.post.active_preset().clone();
                    p.name = format!(
                        "{} {}",
                        tr(lang, "set.profile_word"),
                        self.settings.post.presets.len() + 1
                    );
                    self.settings.post.presets.push(p);
                    self.settings.post.active = self.settings.post.presets.len() - 1;
                    *changed = true;
                }
                if self.settings.post.presets.len() > 1 && ui.button(tr(lang, "set.preset_delete")).clicked() {
                    self.settings.post.presets.remove(self.settings.post.active);
                    self.settings.post.active = 0;
                    *changed = true;
                }
            });
            let uploads_ready = self.settings.uploads.enabled;
            let ai = self.settings.post.active.min(self.settings.post.presets.len() - 1);
            ui.horizontal(|ui| {
                ui.label(tr(lang, "set.preset_name"));
                *changed |= ui
                    .text_edit_singleline(&mut self.settings.post.presets[ai].name)
                    .changed();
            });
            let p = &mut self.settings.post.presets[ai];
            ui.horizontal_wrapped(|ui| {
                *changed |= ui.checkbox(&mut p.trim_margins, tr(lang, "post.trim")).changed();
                *changed |= ui
                    .checkbox(&mut p.hide_sensitive, tr(lang, "post.hide_sensitive"))
                    .changed();
                *changed |= ui.checkbox(&mut p.blur_faces, tr(lang, "post.blur_faces")).changed();
                *changed |= ui.checkbox(&mut p.watermark, tr(lang, "post.watermark")).changed();
                *changed |= ui.checkbox(&mut p.downscale, tr(lang, "post.downscale")).changed();
            });
            ui.horizontal_wrapped(|ui| {
                *changed |= ui.checkbox(&mut p.save, tr(lang, "post.save")).changed();
                *changed |= ui.checkbox(&mut p.clipboard, tr(lang, "post.clipboard")).changed();
                *changed |= ui.checkbox(&mut p.ocr_to_clipboard, tr(lang, "post.ocr")).changed();
                *changed |= ui.checkbox(&mut p.open_editor, tr(lang, "post.editor")).changed();
                ui.add_enabled_ui(uploads_ready, |ui| {
                    *changed |= ui.checkbox(&mut p.upload, tr(lang, "post.upload")).changed();
                });
            });
            if !uploads_ready {
                p.upload = false;
            }
            ui.separator();

            let o = &mut self.settings.post.options;
            ui.horizontal(|ui| {
                ui.label(tr(lang, "set.wm_text"));
                *changed |= ui
                    .add(egui::TextEdit::singleline(&mut o.watermark_text).desired_width(180.0))
                    .changed();
                ui.label(tr(lang, "set.wm_image"));
                ui.label(
                    o.watermark_image
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                );
                if ui.button(tr(lang, "set.choose")).clicked()
                    && let Some(f) = rfd::FileDialog::new().add_filter("PNG", &["png"]).pick_file()
                {
                    o.watermark_image = Some(f);
                    *changed = true;
                }
                if o.watermark_image.is_some() && ui.small_button("✖").clicked() {
                    o.watermark_image = None;
                    *changed = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label(tr(lang, "set.wm_corner"));
                egui::ComboBox::from_id_salt("wm_corner")
                    .selected_text(tr(lang, corner_key(o.watermark_corner)))
                    .show_ui(ui, |ui| {
                        for c in Corner::ALL {
                            *changed |= ui
                                .selectable_value(&mut o.watermark_corner, c, tr(lang, corner_key(c)))
                                .changed();
                        }
                    });
                *changed |= ui
                    .add(egui::Slider::new(&mut o.watermark_opacity, 0.1..=1.0).text(tr(lang, "set.wm_opacity")))
                    .changed();
            });
            ui.horizontal(|ui| {
                *changed |= ui
                    .add(egui::Slider::new(&mut o.downscale_percent, 10..=100).text(tr(lang, "set.downscale_pct")))
                    .changed();
                ui.label(tr(lang, "set.hide_style"));
                *changed |= ui
                    .selectable_value(&mut o.hide_style, HideStyle::Pixelate, tr(lang, "set.style_pixelate"))
                    .changed();
                *changed |= ui
                    .selectable_value(&mut o.hide_style, HideStyle::Blur, tr(lang, "set.style_blur"))
                    .changed();
            });
            ui.horizontal(|ui| {
                ui.label(tr(lang, "set.ocr_lang"));
                let langs = ocr::available_languages();
                if langs.is_empty() {
                    ui.colored_label(theme::WARN, tr(lang, "set.ocr_none"));
                } else {
                    let shown = o
                        .ocr_language
                        .clone()
                        .unwrap_or_else(|| tr(lang, "set.ocr_auto").to_owned());
                    egui::ComboBox::from_id_salt("ocr_lang")
                        .selected_text(shown)
                        .show_ui(ui, |ui| {
                            *changed |= ui
                                .selectable_value(&mut o.ocr_language, None, tr(lang, "set.ocr_auto"))
                                .changed();
                            for l in langs {
                                *changed |= ui.selectable_value(&mut o.ocr_language, Some(l.clone()), l).changed();
                            }
                        });
                }
                *changed |= ui
                    .add(egui::Slider::new(&mut o.face_threshold, 0.4..=0.95).text(tr(lang, "set.face_threshold")))
                    .changed();
            });
            ui.separator();
            self.face_model_row(ui);
        });
    }

    fn face_model_row(&mut self, ui: &mut Ui) {
        let lang = self.settings.language;
        if let Some(d) = &self.model_dl
            && let Ok(r) = d.rx.try_recv()
        {
            self.model_dl = None;
            match r {
                Ok(()) => self.notify(tr(lang, "set.model_ready"), false),
                Err(e) => self.notify(e, true),
            }
        }
        ui.label(RichText::new(tr(lang, "set.face_model")).strong());
        let state = face::model_state();
        ui.horizontal(|ui| {
            match state {
                ModelState::Ready => ui.colored_label(theme::GOOD, tr(lang, "set.model_ready")),
                ModelState::Missing => ui.colored_label(theme::WARN, tr(lang, "set.model_missing")),
                ModelState::Corrupt => ui.colored_label(theme::BAD, tr(lang, "set.model_corrupt")),
            };
            if let Some(d) = &self.model_dl {
                let (done, total) = (
                    d.progress.0.load(Ordering::Relaxed),
                    d.progress.1.load(Ordering::Relaxed),
                );
                ui.add(egui::ProgressBar::new(done as f32 / total.max(1) as f32).desired_width(140.0));
                ui.ctx().request_repaint();
            } else if state != ModelState::Ready {
                if ui
                    .add_enabled(
                        !self.flags.demo,
                        egui::Button::new(format!(
                            "{} ({})",
                            tr(lang, "set.model_download"),
                            face::MODEL_SIZE_HINT
                        )),
                    )
                    .clicked()
                {
                    let (tx, rx) = channel();
                    let progress: Arc<(AtomicU64, AtomicU64)> = Arc::default();
                    let (p2, ctx) = (progress.clone(), ui.ctx().clone());
                    std::thread::spawn(move || {
                        let r = face::download_model(&|d, t| {
                            p2.0.store(d, Ordering::Relaxed);
                            p2.1.store(t, Ordering::Relaxed);
                            ctx.request_repaint();
                        });
                        let _ = tx.send(r);
                        ctx.request_repaint();
                    });
                    self.model_dl = Some(ModelDownload { rx, progress });
                }
            } else if ui.button(tr(lang, "set.model_delete")).clicked() {
                let _ = std::fs::remove_file(face::model_path());
            }
        });
        ui.label(RichText::new(tr(lang, "set.model_note")).weak().small());
    }

    pub fn uploads_card(&mut self, ui: &mut Ui, changed: &mut bool) {
        let lang = self.settings.language;
        let ctx = ui.ctx().clone();
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(tr(lang, "up.title")).strong());
            let mut on = self.settings.uploads.enabled;
            if ui.checkbox(&mut on, tr(lang, "up.master")).changed() {
                if on {
                    self.confirm_uploads = true; // switching ON needs an explicit confirmation
                } else {
                    self.settings.uploads.enabled = false;
                    *changed = true;
                }
            }
            ui.label(RichText::new(tr(lang, "up.master_note")).weak().small());
            if !self.settings.uploads.enabled {
                return;
            }
            if self.settings.uploads.destinations.is_empty() {
                ui.label(RichText::new(tr(lang, "up.none")).weak());
            }
            let mut remove = None;
            for i in 0..self.settings.uploads.destinations.len() {
                let (id, title) = {
                    let d = &self.settings.uploads.destinations[i];
                    (d.id.clone(), format!("{} — {}", d.name, d.kind.label()))
                };
                egui::CollapsingHeader::new(title)
                    .id_salt(&id)
                    .default_open(self.flags.demo)
                    .show(ui, |ui| {
                        let active = self.settings.uploads.active.as_deref() == Some(id.as_str());
                        ui.horizontal(|ui| {
                            let d = &mut self.settings.uploads.destinations[i];
                            *changed |= ui.checkbox(&mut d.enabled, tr(lang, "up.enabled")).changed();
                            if ui.selectable_label(active, tr(lang, "up.active")).clicked() {
                                self.settings.uploads.active = Some(id.clone());
                                *changed = true;
                            }
                            ui.label(tr(lang, "up.name"));
                            *changed |= ui.text_edit_singleline(&mut d.name).changed();
                        });
                        let d = &mut self.settings.uploads.destinations[i];
                        match &mut d.kind {
                            DestKind::Http(h) => *changed |= http_fields(ui, h, lang),
                            DestKind::Telegram { chat_id } => {
                                ui.horizontal(|ui| {
                                    ui.label(tr(lang, "up.chat_id"));
                                    *changed |= ui.text_edit_singleline(chat_id).changed();
                                });
                            }
                            DestKind::S3(s) => *changed |= s3_fields(ui, s, lang),
                            DestKind::Discord => {}
                        }
                        let names = d.kind.secret_names();
                        self.secret_rows(ui, &id, names);
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(!self.flags.demo, egui::Button::new(tr(lang, "up.test")))
                                .clicked()
                            {
                                self.test_upload(&ctx, &id);
                            }
                            if ui.button(tr(lang, "up.delete")).clicked() {
                                remove = Some(i);
                            }
                        });
                    });
            }
            if let Some(i) = remove {
                let d = self.settings.uploads.destinations.remove(i);
                for n in d.kind.secret_names() {
                    self.secrets.delete(&format!("{}.{n}", d.id));
                }
                if self.settings.uploads.active.as_deref() == Some(d.id.as_str()) {
                    self.settings.uploads.active = None;
                }
                *changed = true;
            }
            ui.horizontal(|ui| {
                ui.label(tr(lang, "up.add"));
                for (label, make) in [
                    ("HTTP", DestKind::Http(HttpDest::default())),
                    ("Discord", DestKind::Discord),
                    ("Telegram", DestKind::Telegram { chat_id: String::new() }),
                    ("S3", DestKind::S3(S3Dest::default())),
                ] {
                    if ui.button(label).clicked() {
                        let id = format!("d{}", unix_now() * 10 + self.settings.uploads.destinations.len() as u64);
                        self.settings.uploads.destinations.push(Destination {
                            id: id.clone(),
                            name: label.to_owned(),
                            enabled: false,
                            kind: make,
                        });
                        if self.settings.uploads.active.is_none() {
                            self.settings.uploads.active = Some(id);
                        }
                        *changed = true;
                    }
                }
            });
        });
    }

    /// One row per secret: state, a masked input, save/forget. Values go straight to the Credential Manager.
    fn secret_rows(&mut self, ui: &mut Ui, id: &str, names: &[&str]) {
        let lang = self.settings.language;
        for name in names {
            let key = format!("{id}.{name}");
            let is_set = *self
                .secret_state
                .entry(key.clone())
                .or_insert_with(|| self.secrets.get(&key).is_some());
            ui.horizontal(|ui| {
                ui.label(format!("{}:", tr(lang, secret_label_key(name))));
                if is_set {
                    ui.colored_label(theme::GOOD, tr(lang, "up.secret_set"));
                } else {
                    ui.colored_label(theme::WARN, tr(lang, "up.secret_unset"));
                }
                let input = self.secret_input.entry(key.clone()).or_default();
                ui.add(
                    egui::TextEdit::singleline(input)
                        .password(true)
                        .hint_text(tr(lang, "up.secret_hint"))
                        .desired_width(200.0),
                );
                if ui.button(tr(lang, "up.secret_save")).clicked() && !input.trim().is_empty() {
                    let value = std::mem::take(input);
                    match self.secrets.set(&key, value.trim()) {
                        Ok(()) => {
                            self.secret_state.insert(key.clone(), true);
                        }
                        Err(e) => self.notify(e, true),
                    }
                }
                if is_set && ui.button(tr(lang, "up.secret_clear")).clicked() {
                    self.secrets.delete(&key);
                    self.secret_state.insert(key.clone(), false);
                }
            });
        }
    }

    fn test_upload(&mut self, ctx: &egui::Context, id: &str) {
        let lang = self.settings.language;
        let mut s = self.settings.uploads.clone();
        s.enabled = true; // an explicit click on "Test" for a destination the user already configured
        let (store, ctx2, id) = (self.secrets.clone(), ctx.clone(), id.to_owned());
        let img =
            snapforge_core::image::RgbaImage::from_pixel(64, 64, snapforge_core::image::Rgba([96, 165, 250, 255]));
        let tx = self.toast_tx.clone();
        std::thread::spawn(move || {
            let bytes = output::encode(&img, snapforge_core::settings::ImageFormat::Png, 90).unwrap_or_default();
            let file = FilePart {
                name: "snapforge-test.png".into(),
                mime: "image/png".into(),
                bytes,
            };
            let msg = match upload::upload(&s, Some(&id), &file, store.as_ref(), &UreqTransport) {
                Ok(o) => (
                    format!("{} {}", tr(lang, "up.test_ok"), o.link.unwrap_or_default()),
                    false,
                ),
                Err(e) => (e.to_string(), true),
            };
            let _ = tx.send(msg);
            ctx2.request_repaint();
        });
    }

    pub fn uploads_confirm(&mut self, ctx: &egui::Context, changed: &mut bool) {
        if !self.confirm_uploads {
            return;
        }
        let lang = self.settings.language;
        let (mut yes, mut no) = (false, false);
        egui::Window::new(tr(lang, "up.confirm_title"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.label(tr(lang, "up.confirm_text"));
                ui.horizontal(|ui| {
                    if theme::primary_button(ui, tr(lang, "up.confirm_yes")).clicked() {
                        yes = true;
                    }
                    if ui.button(tr(lang, "common.cancel")).clicked() {
                        no = true;
                    }
                });
            });
        if yes {
            self.settings.uploads.enabled = true;
            *changed = true;
        }
        if yes || no {
            self.confirm_uploads = false;
        }
    }
}

fn http_fields(ui: &mut Ui, h: &mut HttpDest, lang: snapforge_core::settings::Lang) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(tr(lang, "up.url"));
        changed |= ui
            .add(egui::TextEdit::singleline(&mut h.url).desired_width(360.0))
            .changed();
        ui.label(tr(lang, "up.method"));
        egui::ComboBox::from_id_salt("http_method")
            .selected_text(h.method.clone())
            .show_ui(ui, |ui| {
                for m in ["POST", "PUT"] {
                    changed |= ui.selectable_value(&mut h.method, m.to_owned(), m).changed();
                }
            });
    });
    ui.horizontal(|ui| {
        ui.label(tr(lang, "up.file_field"));
        changed |= ui
            .add(egui::TextEdit::singleline(&mut h.file_field).desired_width(120.0))
            .changed();
    });
    ui.horizontal(|ui| {
        ui.label(tr(lang, "up.json_path"));
        changed |= ui
            .add(egui::TextEdit::singleline(&mut h.json_path).desired_width(160.0))
            .changed();
    });
    ui.label(tr(lang, "up.headers"));
    let mut drop = None;
    for (i, (k, v)) in h.headers.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            changed |= ui.add(egui::TextEdit::singleline(k).desired_width(140.0)).changed();
            changed |= ui.add(egui::TextEdit::singleline(v).desired_width(260.0)).changed();
            if ui.small_button("✖").clicked() {
                drop = Some(i);
            }
        });
    }
    if let Some(i) = drop {
        h.headers.remove(i);
        changed = true;
    }
    if ui.small_button(tr(lang, "up.add_row")).clicked() {
        h.headers.push((String::new(), String::new()));
        changed = true;
    }
    changed
}

fn s3_fields(ui: &mut Ui, s: &mut S3Dest, lang: snapforge_core::settings::Lang) -> bool {
    let mut changed = false;
    let mut row = |ui: &mut Ui, label: &'static str, v: &mut String| {
        ui.horizontal(|ui| {
            ui.label(tr(lang, label));
            changed |= ui.add(egui::TextEdit::singleline(v).desired_width(320.0)).changed();
        });
    };
    row(ui, "up.endpoint", &mut s.endpoint);
    row(ui, "up.bucket", &mut s.bucket);
    row(ui, "up.region", &mut s.region);
    row(ui, "up.prefix", &mut s.prefix);
    row(ui, "up.public_base", &mut s.public_base);
    changed |= ui.checkbox(&mut s.path_style, tr(lang, "up.path_style")).changed();
    changed
}
