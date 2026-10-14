//! Installer / uninstaller screens. The same executable becomes the installer when it is started as
//! `SnapForge-Setup.exe` (or with `--install`) and the uninstaller with `--uninstall`.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

use eframe::egui::{self, Align, Layout, RichText, Ui, vec2};

use crate::{App, Mode, theme, winint};

#[derive(Default)]
pub struct Install {
    job: Option<Receiver<Result<PathBuf, String>>>,
    result: Option<Result<PathBuf, String>>,
}

#[derive(Default)]
pub struct Uninstall {
    remove_data: bool,
    job: Option<Receiver<Result<(), String>>>,
    result: Option<Result<(), String>>,
}

impl App {
    pub fn installer_ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let icon = self.icon.clone();
        let lang = self.settings.language;
        let t = move |k: &'static str| crate::i18n::tr(lang, k);
        let desktop = &mut self.desktop_shortcut;
        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(18.0);
            ui.vertical_centered(|ui| {
                if let Some(icon) = &icon {
                    ui.add(egui::Image::new(icon).fit_to_exact_size(vec2(64.0, 64.0)));
                }
                ui.add_space(6.0);
                ui.label(RichText::new("SnapForge").size(24.0).strong());
                ui.label(RichText::new(format!("{} {}", t("inst.version"), env!("CARGO_PKG_VERSION"))).weak());
            });
            ui.add_space(14.0);
            match &mut self.mode {
                Mode::Install(s) => install_body(ui, s, desktop, &t),
                Mode::Uninstall(s) => uninstall_body(ui, s, &t),
                Mode::App => {}
            }
        });
        let _ = ctx;
    }
}

fn close(ui: &Ui) {
    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
}

fn install_body(ui: &mut Ui, s: &mut Install, desktop: &mut bool, t: &dyn Fn(&'static str) -> &'static str) {
    if let Some(rx) = &s.job {
        if let Ok(r) = rx.try_recv() {
            s.result = Some(r);
            s.job = None;
        } else {
            ui.vertical_centered(|ui| {
                ui.add_space(20.0);
                ui.add(egui::Spinner::new().size(32.0).color(theme::ACCENT));
                ui.label(t("inst.installing"));
            });
            ui.ctx().request_repaint();
            return;
        }
    }
    match &s.result {
        Some(Ok(exe)) => {
            let exe = exe.clone();
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(format!("✔ {}", t("inst.done")))
                        .size(18.0)
                        .color(theme::GOOD)
                        .strong(),
                );
                ui.add_space(4.0);
                ui.label(t("inst.done_text"));
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    ui.add_space((ui.available_width() - 260.0).max(0.0) / 2.0);
                    if theme::primary_button(ui, t("inst.launch")).clicked() {
                        let _ = std::process::Command::new(&exe).spawn();
                        close(ui);
                    }
                    if ui.button(t("inst.close")).clicked() {
                        close(ui);
                    }
                });
            });
        }
        Some(Err(e)) => {
            ui.label(RichText::new(t("inst.failed")).strong().color(theme::BAD));
            ui.label(e.as_str());
            if ui.button(t("inst.retry")).clicked() {
                s.result = None;
            }
        }
        None => {
            let dir = winint::install_dir()
                .map(|d| d.display().to_string())
                .unwrap_or_default();
            let upgrade = winint::installed_exe().is_some_and(|e| e.exists());
            theme::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(t("inst.folder")).weak());
                ui.label(dir);
                ui.add_space(6.0);
                ui.checkbox(desktop, t("inst.desktop"));
                ui.label(RichText::new(t("inst.note")).weak().small());
            });
            ui.add_space(12.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button(t("common.cancel")).clicked() {
                    close(ui);
                }
                if theme::primary_button(ui, if upgrade { t("inst.update") } else { t("inst.install") }).clicked() {
                    let (tx, rx) = channel();
                    let opts = winint::InstallOptions {
                        desktop_shortcut: *desktop,
                    };
                    let ctx = ui.ctx().clone();
                    std::thread::spawn(move || {
                        let _ = tx.send(winint::install(opts));
                        ctx.request_repaint();
                    });
                    s.job = Some(rx);
                }
            });
        }
    }
}

fn uninstall_body(ui: &mut Ui, s: &mut Uninstall, t: &dyn Fn(&'static str) -> &'static str) {
    if let Some(rx) = &s.job {
        if let Ok(r) = rx.try_recv() {
            s.result = Some(r);
            s.job = None;
        } else {
            ui.vertical_centered(|ui| ui.add(egui::Spinner::new().size(32.0).color(theme::ACCENT)));
            ui.ctx().request_repaint();
            return;
        }
    }
    match &s.result {
        Some(Ok(())) => {
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(format!("✔ {}", t("uninst.done")))
                        .size(18.0)
                        .color(theme::GOOD)
                        .strong(),
                );
                ui.add_space(12.0);
                if ui.button(t("inst.close")).clicked() {
                    close(ui);
                }
            });
        }
        Some(Err(e)) => {
            ui.label(RichText::new(t("uninst.failed")).strong().color(theme::BAD));
            ui.label(e.as_str());
        }
        None => {
            theme::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(t("uninst.text"));
                ui.checkbox(&mut s.remove_data, t("uninst.data"));
            });
            ui.add_space(10.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button(t("common.cancel")).clicked() {
                    close(ui);
                }
                if theme::primary_button(ui, t("uninst.go")).clicked() {
                    let (tx, rx) = channel();
                    let remove = s.remove_data;
                    let ctx = ui.ctx().clone();
                    std::thread::spawn(move || {
                        let _ = tx.send(winint::uninstall(remove));
                        ctx.request_repaint();
                    });
                    s.job = Some(rx);
                }
            });
        }
    }
}
