//! Main window: navigation, home, history and settings screens.

use eframe::egui::{self, Align, Color32, Layout, RichText, Ui, vec2};
use snapforge_core::hotkey::{Combo, HotAction, Profile};
use snapforge_core::settings::{ImageFormat, Lang};
use snapforge_core::{diag, history, naming};

use crate::i18n::tr;
use crate::{App, View, diag as app_diag, theme, update, winint};

fn action_key(a: HotAction) -> &'static str {
    match a {
        HotAction::CaptureRegion => "act.region",
        HotAction::CaptureWindow => "act.window",
        HotAction::CaptureScreen => "act.screen",
        HotAction::CaptureMonitor => "act.monitor",
    }
}

impl App {
    pub fn app_ui(&mut self, ui: &mut Ui) {
        let lang = self.settings.language;
        egui::Panel::top("nav").show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(icon) = &self.icon {
                    ui.add(egui::Image::new(icon).fit_to_exact_size(vec2(22.0, 22.0)));
                }
                ui.label(RichText::new("SnapForge").strong().size(16.0));
                ui.add_space(12.0);
                let mut tabs = vec![
                    (View::Home, "nav.home"),
                    (View::History, "nav.history"),
                    (View::Plugins, "nav.plugins"),
                    (View::Settings, "nav.settings"),
                ];
                if self.editor.is_some() {
                    tabs.insert(1, (View::Editor, "nav.editor"));
                }
                if self.pending.is_some() {
                    tabs.insert(1, (View::PostCapture, "nav.post"));
                }
                for (v, key) in tabs {
                    if ui
                        .add(egui::Button::selectable(self.view == v, tr(lang, key)))
                        .clicked()
                    {
                        self.view = v;
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| self.update_badge(ui));
            });
        });
        match self.view {
            View::Home => egui::CentralPanel::default().show(ui, |ui| self.home_view(ui)).inner,
            View::History => egui::CentralPanel::default().show(ui, |ui| self.history_view(ui)).inner,
            View::Settings => {
                egui::CentralPanel::default()
                    .show(ui, |ui| self.settings_view(ui))
                    .inner
            }
            View::Editor => self.editor_view(ui),
            View::Plugins => self.plugins_view(ui),
            View::PostCapture => egui::CentralPanel::default().show(ui, |ui| self.post_view(ui)).inner,
        }
        self.toast_ui(ui.ctx());
    }

    fn toast_ui(&mut self, ctx: &egui::Context) {
        let Some(t) = &self.toast else { return };
        let (text, error) = (t.text.clone(), t.error);
        egui::Area::new(egui::Id::new("toast"))
            .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -18.0])
            .show(ctx, |ui| {
                theme::card(ui)
                    .stroke(egui::Stroke::new(1.5, if error { theme::BAD } else { theme::GOOD }))
                    .show(ui, |ui| {
                        ui.label(RichText::new(text).color(if error { theme::BAD } else { Color32::WHITE }));
                    });
            });
    }

    fn update_badge(&mut self, ui: &mut Ui) {
        let lang = self.settings.language;
        let state = self.update.lock().unwrap_or_else(|e| e.into_inner()).clone();
        match state {
            update::State::Available { version, url } => {
                if theme::primary_button(ui, &self.tf("upd.available", &[("v", version)])).clicked() {
                    let ctx = ui.ctx().clone();
                    update::install(self.update.clone(), move || ctx.request_repaint());
                }
                if ui.small_button(tr(lang, "upd.notes")).clicked() {
                    winint::open_url(&url);
                }
            }
            update::State::Downloading { done, total, .. } => {
                ui.add(
                    egui::ProgressBar::new(done as f32 / total.max(1) as f32)
                        .desired_width(120.0)
                        .text(tr(lang, "upd.downloading")),
                );
            }
            update::State::Ready { version } => {
                if theme::primary_button(ui, &self.tf("upd.restart", &[("v", version)])).clicked() {
                    if let Ok(exe) = std::env::current_exe() {
                        let _ = std::process::Command::new(exe).spawn();
                    }
                    self.quit = true;
                }
            }
            _ => {}
        }
    }

    fn home_view(&mut self, ui: &mut Ui) {
        let lang = self.settings.language;
        let ctx = ui.ctx().clone();
        ui.add_space(6.0);
        ui.label(RichText::new(tr(lang, "home.title")).size(22.0).strong());
        ui.label(RichText::new(tr(lang, "home.subtitle")).weak());
        ui.add_space(10.0);

        let hint = |app: &App, a: HotAction| -> String {
            app.settings
                .active_profile()
                .bindings
                .iter()
                .find(|b| b.action == a)
                .map(|b| b.combo.to_string())
                .unwrap_or_default()
        };
        ui.horizontal_wrapped(|ui| {
            for (a, key) in [
                (HotAction::CaptureRegion, "act.region"),
                (HotAction::CaptureWindow, "act.window"),
                (HotAction::CaptureScreen, "act.screen"),
            ] {
                let h = hint(self, a);
                let label = if h.is_empty() {
                    tr(lang, key).to_owned()
                } else {
                    format!("{}\n{}", tr(lang, key), h)
                };
                let b = egui::Button::new(RichText::new(label).size(15.0)).min_size(vec2(190.0, 62.0));
                if ui.add_enabled(!self.capturing, b).clicked() {
                    self.start_capture(&ctx, a);
                }
            }
            ui.vertical(|ui| {
                let h = hint(self, HotAction::CaptureMonitor);
                let label = format!("{} {}\n{}", tr(lang, "act.monitor_n"), self.monitor_choice + 1, h);
                let b = egui::Button::new(RichText::new(label).size(15.0)).min_size(vec2(190.0, 62.0));
                if ui.add_enabled(!self.capturing, b).clicked() {
                    self.start_capture_on(&ctx, HotAction::CaptureMonitor, Some(self.monitor_choice));
                }
            });
        });
        ui.horizontal(|ui| {
            ui.label(tr(lang, "home.monitor"));
            let count = self.monitors.len().max(1);
            egui::ComboBox::from_id_salt("monitor_pick")
                .selected_text(format!("{} {}", tr(lang, "act.monitor_n"), self.monitor_choice + 1))
                .show_ui(ui, |ui| {
                    for i in 0..count {
                        let txt = match self.monitors.get(i) {
                            Some(m) => format!(
                                "{} {} — {}×{}, {}%",
                                tr(lang, "act.monitor_n"),
                                i + 1,
                                m.rect.w,
                                m.rect.h,
                                m.dpi * 100 / 96
                            ),
                            None => format!("{} {}", tr(lang, "act.monitor_n"), i + 1),
                        };
                        ui.selectable_value(&mut self.monitor_choice, i, txt);
                    }
                });
            if ui.small_button("⟳").on_hover_text(tr(lang, "home.rescan")).clicked() {
                self.monitors = snapforge_core::capture::sys::monitors();
                self.monitor_choice = self.monitor_choice.min(self.monitors.len().saturating_sub(1));
            }
        });
        ui.add_space(6.0);
        let mut changed = false;
        ui.horizontal(|ui| {
            let mut secs = self.settings.delay_ms as f32 / 1000.0;
            if ui
                .add(
                    egui::Slider::new(&mut secs, 0.0..=10.0)
                        .step_by(0.5)
                        .text(tr(lang, "home.delay")),
                )
                .changed()
            {
                self.settings.delay_ms = (secs * 1000.0) as u32;
                changed = true;
            }
            changed |= ui
                .checkbox(&mut self.settings.cursor_in_shot, tr(lang, "home.cursor"))
                .changed();
        });
        if changed {
            self.save_settings();
        }

        let conflicts: Vec<String> = self
            .hotkey_report
            .iter()
            .filter(|r| r.error.is_some())
            .map(|r| r.combo.to_string())
            .collect();
        if !conflicts.is_empty() {
            ui.add_space(6.0);
            ui.colored_label(
                theme::WARN,
                self.tf("home.hotkey_conflict", &[("keys", conflicts.join(", "))]),
            );
        }

        ui.add_space(14.0);
        ui.label(RichText::new(tr(lang, "home.recent")).strong());
        let recent: Vec<String> = self.history.entries().iter().take(6).map(|e| e.id.clone()).collect();
        if recent.is_empty() {
            ui.label(RichText::new(tr(lang, "home.no_history")).weak());
        } else {
            ui.horizontal_wrapped(|ui| {
                for id in recent {
                    if let Some(tex) = self.thumb(&ctx, &id) {
                        let r = ui.add(egui::Button::image(
                            egui::Image::new(&tex).fit_to_exact_size(vec2(200.0, 125.0)),
                        ));
                        if r.clicked() {
                            self.open_history_entry(&ctx, &id);
                        }
                    }
                }
            });
        }
        ui.add_space(14.0);
        ui.label(RichText::new(tr(lang, "home.privacy")).weak().small());
    }

    fn history_view(&mut self, ui: &mut Ui) {
        let lang = self.settings.language;
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr(lang, "hist.title")).size(20.0).strong());
            ui.add_space(12.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text(tr(lang, "hist.search"))
                    .desired_width(240.0),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button(tr(lang, "hist.clear")).clicked() {
                    self.confirm_clear = true;
                }
            });
        });
        let days = self.settings.history_retention_days;
        let retention = if days == 0 {
            tr(lang, "hist.keep_forever").to_owned()
        } else {
            self.tf("hist.keep_days", &[("n", days.to_string())])
        };
        ui.label(RichText::new(retention).weak().small());
        ui.add_space(6.0);
        let entries: Vec<history::Entry> = self.history.search(&self.search).into_iter().cloned().collect();
        if entries.is_empty() {
            ui.label(RichText::new(tr(lang, "hist.empty")).weak());
        }
        let mut open = None;
        let mut delete = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for e in &entries {
                    ui.allocate_ui(vec2(250.0, 200.0), |ui| {
                        theme::card(ui).show(ui, |ui| {
                            ui.set_min_size(vec2(224.0, 186.0));
                            if let Some(tex) = self.thumb(&ctx, &e.id)
                                && ui
                                    .add(egui::Button::image(
                                        egui::Image::new(&tex).fit_to_exact_size(vec2(224.0, 140.0)),
                                    ))
                                    .clicked()
                            {
                                open = Some(e.id.clone());
                            }
                            ui.label(
                                RichText::new(format!("{}  {}×{}", history::date_string(e.created), e.width, e.height))
                                    .small(),
                            );
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&e.app).weak().small());
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui.small_button(tr(lang, "hist.delete")).clicked() {
                                        delete = Some(e.id.clone());
                                    }
                                });
                            });
                        });
                    });
                }
            });
        });
        if let Some(id) = open {
            self.open_history_entry(&ctx, &id);
        }
        if let Some(id) = delete {
            let _ = self.history.delete(&id);
            self.thumbs.remove(&id);
        }
        if self.confirm_clear {
            let mut close = false;
            egui::Window::new(tr(lang, "hist.clear_title"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(&ctx, |ui| {
                    ui.label(self.tf("hist.clear_text", &[("n", self.history.entries().len().to_string())]));
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new(tr(lang, "hist.clear_yes")).color(Color32::WHITE))
                                    .fill(theme::BAD),
                            )
                            .clicked()
                        {
                            let n = self.history.clear().unwrap_or(0);
                            self.thumbs.clear();
                            self.notify(self.tf("hist.cleared", &[("n", n.to_string())]), false);
                            close = true;
                        }
                        if ui.button(tr(lang, "common.cancel")).clicked() {
                            close = true;
                        }
                    });
                });
            if close {
                self.confirm_clear = false;
            }
        }
    }

    fn settings_view(&mut self, ui: &mut Ui) {
        let lang = self.settings.language;
        let ctx = ui.ctx().clone();
        let mut changed = false;
        let mut area = egui::ScrollArea::vertical();
        if let Some(y) = self.settings_scroll.take() {
            area = area.vertical_scroll_offset(y);
        }
        area.show(ui, |ui| {
            ui.label(RichText::new(tr(lang, "set.title")).size(20.0).strong());
            ui.add_space(6.0);

            theme::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(tr(lang, "set.general")).strong());
                ui.horizontal(|ui| {
                    ui.label(tr(lang, "set.language"));
                    let before = self.settings.language;
                    egui::ComboBox::from_id_salt("lang")
                        .selected_text(crate::lang_name(self.settings.language))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.settings.language, Lang::Ru, "Русский");
                            ui.selectable_value(&mut self.settings.language, Lang::En, "English");
                        });
                    if before != self.settings.language {
                        changed = true;
                        self.localize_preset_names();
                        if !self.flags.demo {
                            self.tray = crate::tray::Tray::new(&ctx, self.settings.language).or(self.tray.take());
                        }
                    }
                });
                changed |= ui
                    .checkbox(&mut self.settings.minimize_to_tray, tr(lang, "set.tray"))
                    .changed();
                let mut auto = winint::autostart_enabled();
                if ui.checkbox(&mut auto, tr(lang, "set.autostart")).changed() && !self.flags.demo {
                    match winint::set_autostart(auto) {
                        Ok(()) => {
                            self.settings.autostart = auto;
                            changed = true;
                        }
                        Err(e) => self.notify(e, true),
                    }
                }
            });
            ui.add_space(8.0);

            theme::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(tr(lang, "set.saving")).strong());
                ui.horizontal(|ui| {
                    ui.label(tr(lang, "set.folder"));
                    ui.label(self.settings.save_dir.display().to_string());
                    if ui.button(tr(lang, "set.choose")).clicked()
                        && let Some(d) = rfd::FileDialog::new()
                            .set_directory(&self.settings.save_dir)
                            .pick_folder()
                    {
                        self.settings.save_dir = d;
                        changed = true;
                    }
                    if ui.button(tr(lang, "set.open_folder")).clicked() {
                        let _ = std::fs::create_dir_all(&self.settings.save_dir);
                        winint::open_path(&self.settings.save_dir);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(tr(lang, "set.template"));
                    changed |= ui
                        .add(egui::TextEdit::singleline(&mut self.settings.name_template).desired_width(260.0))
                        .changed();
                    let preview = naming::expand(
                        &self.settings.name_template,
                        "chrome",
                        chrono::Local::now().naive_local(),
                    );
                    ui.label(RichText::new(format!("= {preview}.{}", self.settings.format.ext())).weak());
                });
                ui.label(RichText::new(tr(lang, "set.template_help")).weak().small());
                ui.horizontal(|ui| {
                    ui.label(tr(lang, "set.format"));
                    changed |= ui
                        .selectable_value(&mut self.settings.format, ImageFormat::Png, "PNG")
                        .changed();
                    changed |= ui
                        .selectable_value(&mut self.settings.format, ImageFormat::Jpeg, "JPEG")
                        .changed();
                    if self.settings.format == ImageFormat::Jpeg {
                        changed |= ui
                            .add(
                                egui::Slider::new(&mut self.settings.jpeg_quality, 40..=100)
                                    .text(tr(lang, "set.quality")),
                            )
                            .changed();
                    }
                });
            });
            ui.add_space(8.0);

            theme::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(tr(lang, "set.history")).strong());
                changed |= ui
                    .checkbox(&mut self.settings.history_enabled, tr(lang, "set.history_on"))
                    .changed();
                ui.horizontal(|ui| {
                    ui.label(tr(lang, "set.retention"));
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut self.settings.history_retention_days)
                                .range(0..=3650)
                                .suffix(tr(lang, "set.days")),
                        )
                        .changed();
                    ui.label(RichText::new(tr(lang, "set.retention_hint")).weak().small());
                });
                if ui.button(tr(lang, "hist.clear")).clicked() {
                    self.confirm_clear = true;
                    self.view = View::History;
                }
            });
            ui.add_space(8.0);

            self.hotkeys_card(ui, &mut changed);
            ui.add_space(8.0);
            self.post_card(ui, &mut changed);
            ui.add_space(8.0);
            self.uploads_card(ui, &mut changed);
            ui.add_space(8.0);
            self.updates_card(ui, &mut changed);
            ui.add_space(8.0);
            self.system_card(ui);
        });
        self.uploads_confirm(&ctx, &mut changed);
        if changed {
            self.save_settings();
        }
    }

    fn hotkeys_card(&mut self, ui: &mut Ui, changed: &mut bool) {
        let lang = self.settings.language;
        let ctx = ui.ctx().clone();
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(tr(lang, "set.hotkeys")).strong());
            *changed |= ui
                .checkbox(&mut self.settings.hotkeys_enabled, tr(lang, "set.hotkeys_on"))
                .changed();
            ui.horizontal(|ui| {
                ui.label(tr(lang, "set.profile"));
                let names: Vec<String> = self.settings.profiles.iter().map(|p| p.name.clone()).collect();
                let mut idx = self.settings.active_profile;
                egui::ComboBox::from_id_salt("profile")
                    .selected_text(names.get(idx).cloned().unwrap_or_default())
                    .show_ui(ui, |ui| {
                        for (i, n) in names.iter().enumerate() {
                            ui.selectable_value(&mut idx, i, n);
                        }
                    });
                if idx != self.settings.active_profile {
                    self.settings.active_profile = idx;
                    self.hotkey_text.clear();
                    *changed = true;
                }
                if ui.button(tr(lang, "set.profile_new")).clicked() {
                    let mut p: Profile = self.settings.active_profile().clone();
                    p.name = format!("{} {}", tr(lang, "set.profile_word"), self.settings.profiles.len() + 1);
                    self.settings.profiles.push(p);
                    self.settings.active_profile = self.settings.profiles.len() - 1;
                    self.hotkey_text.clear();
                    *changed = true;
                }
                if self.settings.profiles.len() > 1 && ui.button(tr(lang, "set.profile_delete")).clicked() {
                    self.settings.profiles.remove(self.settings.active_profile);
                    self.settings.active_profile = 0;
                    self.hotkey_text.clear();
                    *changed = true;
                }
            });
            let ai = self.settings.active_profile;
            ui.horizontal(|ui| {
                ui.label(tr(lang, "set.profile_name"));
                *changed |= ui.text_edit_singleline(&mut self.settings.profiles[ai].name).changed();
            });
            if self.hotkey_text.len() != self.settings.profiles[ai].bindings.len() {
                self.hotkey_text = self.settings.profiles[ai]
                    .bindings
                    .iter()
                    .map(|b| b.combo.to_string())
                    .collect();
            }
            // recording: the next key combination pressed becomes the binding
            if let Some(i) = self.recording {
                let pressed = ctx.input(|inp| {
                    inp.events.iter().find_map(|e| match e {
                        egui::Event::Key {
                            key,
                            pressed: true,
                            modifiers,
                            ..
                        } => Some((*key, *modifiers)),
                        _ => None,
                    })
                });
                if let Some((key, m)) = pressed {
                    if key == egui::Key::Escape {
                        self.recording = None;
                    } else {
                        let mut s = String::new();
                        for (on, n) in [
                            (m.ctrl, "Ctrl+"),
                            (m.alt, "Alt+"),
                            (m.shift, "Shift+"),
                            (m.mac_cmd || m.command && !m.ctrl, "Win+"),
                        ] {
                            if on {
                                s.push_str(n);
                            }
                        }
                        s.push_str(key.name());
                        if let Ok(c) = s.parse::<Combo>() {
                            self.hotkey_text[i] = c.to_string();
                            self.recording = None;
                        }
                    }
                }
            }
            let mut conflict_msgs: Vec<String> = Vec::new();
            egui::Grid::new("hotkey_grid")
                .num_columns(4)
                .spacing([10.0, 6.0])
                .show(ui, |ui| {
                    for i in 0..self.settings.profiles[ai].bindings.len() {
                        let action = self.settings.profiles[ai].bindings[i].action;
                        ui.label(tr(lang, action_key(action)));
                        let parsed = self.hotkey_text[i].parse::<Combo>();
                        let mut text = self.hotkey_text[i].clone();
                        let color = if parsed.is_ok() {
                            ui.visuals().text_color()
                        } else {
                            theme::BAD
                        };
                        let r = ui.add_sized([170.0, 22.0], egui::TextEdit::singleline(&mut text).text_color(color));
                        if r.changed() {
                            self.hotkey_text[i] = text;
                        }
                        let rec = self.recording == Some(i);
                        if ui
                            .button(if rec {
                                tr(lang, "set.recording")
                            } else {
                                tr(lang, "set.record")
                            })
                            .clicked()
                        {
                            self.recording = if rec { None } else { Some(i) };
                        }
                        let status = self.hotkey_report.iter().find(|h| h.action == action);
                        match (&parsed, status) {
                            (Err(e), _) => {
                                ui.colored_label(theme::BAD, e.to_string());
                            }
                            (Ok(_), Some(s)) if s.error.is_some() => {
                                ui.colored_label(theme::WARN, tr(lang, "set.taken"));
                            }
                            (Ok(_), Some(_)) => {
                                ui.colored_label(theme::GOOD, "✔");
                            }
                            _ => {
                                ui.label("");
                            }
                        }
                        ui.end_row();
                    }
                });
            if ui.button(tr(lang, "set.apply_hotkeys")).clicked() {
                let mut p = self.settings.profiles[ai].clone();
                let mut ok = true;
                for (i, b) in p.bindings.iter_mut().enumerate() {
                    match self.hotkey_text[i].parse::<Combo>() {
                        Ok(c) => b.combo = c,
                        Err(_) => ok = false,
                    }
                }
                let internal = p.internal_conflicts();
                if !ok {
                    conflict_msgs.push(tr(lang, "set.err_invalid").to_owned());
                } else if !internal.is_empty() {
                    conflict_msgs.push(tr(lang, "set.err_duplicate").to_owned());
                } else {
                    self.settings.profiles[ai] = p;
                    self.apply_hotkeys();
                    *changed = true;
                    let taken = self.hotkey_report.iter().filter(|r| r.error.is_some()).count();
                    if taken > 0 {
                        conflict_msgs.push(self.tf("set.err_taken", &[("n", taken.to_string())]));
                    } else {
                        self.notify(tr(lang, "set.hotkeys_applied"), false);
                    }
                }
            }
            for m in conflict_msgs {
                ui.colored_label(theme::WARN, m);
            }
        });
    }

    fn updates_card(&mut self, ui: &mut Ui, changed: &mut bool) {
        let lang = self.settings.language;
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(tr(lang, "set.updates")).strong());
            *changed |= ui
                .checkbox(&mut self.settings.check_updates, tr(lang, "set.updates_on"))
                .changed();
            ui.label(RichText::new(tr(lang, "set.updates_note")).weak().small());
            let state = self.update.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let text = match &state {
                update::State::Idle => String::new(),
                update::State::Checking => tr(lang, "upd.checking").to_owned(),
                update::State::UpToDate => tr(lang, "upd.uptodate").to_owned(),
                update::State::Available { version, .. } => self.tf("upd.available", &[("v", version.clone())]),
                update::State::Downloading { .. } => tr(lang, "upd.downloading").to_owned(),
                update::State::Ready { version } => self.tf("upd.ready", &[("v", version.clone())]),
                update::State::Failed(e) => format!("{}: {e}", tr(lang, "upd.failed")),
            };
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!self.flags.demo, egui::Button::new(tr(lang, "upd.check_now")))
                    .clicked()
                {
                    let ctx = ui.ctx().clone();
                    update::check(self.update.clone(), false, move || ctx.request_repaint());
                }
                ui.label(text);
            });
        });
    }

    fn system_card(&mut self, ui: &mut Ui) {
        let lang = self.settings.language;
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(tr(lang, "set.system")).strong());
            if !winint::is_installed_copy() {
                ui.horizontal(|ui| {
                    ui.label(tr(lang, "set.portable"));
                    if ui
                        .add_enabled(!self.flags.demo, egui::Button::new(tr(lang, "set.install")))
                        .clicked()
                        && let Ok(exe) = std::env::current_exe()
                    {
                        let _ = std::process::Command::new(exe).arg("--install").spawn();
                        self.quit = true;
                    }
                });
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!self.flags.demo, egui::Button::new(tr(lang, "set.run_admin")))
                    .clicked()
                {
                    match std::env::current_exe()
                        .map_err(|e| e.to_string())
                        .and_then(|e| winint::run_elevated(&e, ""))
                    {
                        Ok(()) => self.quit = true,
                        Err(e) => self.notify(e, true),
                    }
                }
                ui.label(RichText::new(tr(lang, "set.run_admin_hint")).weak().small());
            });
            ui.separator();
            ui.label(RichText::new(tr(lang, "set.diagnostics")).strong());
            ui.label(RichText::new(tr(lang, "set.diag_note")).weak().small());
            ui.horizontal(|ui| {
                if ui.button(tr(lang, "set.open_logs")).clicked() {
                    let dir = snapforge_core::paths::logs_dir();
                    let _ = std::fs::create_dir_all(&dir);
                    winint::open_path(&dir);
                }
                if ui.button(tr(lang, "set.copy_diag")).clicked() {
                    let text = app_diag::diagnostic_text();
                    match snapforge_core::output::copy_text_to_clipboard(&text) {
                        Ok(()) => self.notify(tr(lang, "set.diag_copied"), false),
                        Err(e) => self.notify(e.to_string(), true),
                    }
                    diag::log(diag::Level::Info, "diagnostic text copied");
                }
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!("SnapForge {}", env!("CARGO_PKG_VERSION")))
                    .weak()
                    .small(),
            );
            if ui.link(winint::REPO_URL).clicked() {
                winint::open_url(winint::REPO_URL);
            }
        });
    }
}
