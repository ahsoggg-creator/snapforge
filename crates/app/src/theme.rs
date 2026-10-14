//! Look of the app: dark by default, one accent colour.

use eframe::egui::{self, Color32, CornerRadius, Margin, Response, Stroke, Theme, Ui, vec2};

pub const ACCENT: Color32 = Color32::from_rgb(96, 165, 250);
pub const GOOD: Color32 = Color32::from_rgb(74, 190, 120);
pub const WARN: Color32 = Color32::from_rgb(240, 180, 70);
pub const BAD: Color32 = Color32::from_rgb(235, 90, 90);

pub fn install(ctx: &egui::Context) {
    for theme in [Theme::Dark, Theme::Light] {
        ctx.style_mut_of(theme, |s| {
            let dark = theme == Theme::Dark;
            s.spacing.item_spacing = vec2(8.0, 6.0);
            s.spacing.button_padding = vec2(10.0, 4.0);
            s.spacing.menu_margin = Margin::same(6);
            s.spacing.interact_size.y = 24.0;
            let v = &mut s.visuals;
            let r = CornerRadius::same(6);
            v.window_corner_radius = CornerRadius::same(10);
            v.menu_corner_radius = CornerRadius::same(8);
            for w in [
                &mut v.widgets.noninteractive,
                &mut v.widgets.inactive,
                &mut v.widgets.hovered,
                &mut v.widgets.active,
                &mut v.widgets.open,
            ] {
                w.corner_radius = r;
            }
            v.selection.bg_fill = if dark {
                Color32::from_rgb(38, 79, 150)
            } else {
                Color32::from_rgb(190, 214, 255)
            };
            v.selection.stroke = Stroke::new(1.0, ACCENT);
            v.hyperlink_color = ACCENT;
            if dark {
                v.panel_fill = Color32::from_rgb(28, 30, 36);
                v.window_fill = Color32::from_rgb(36, 38, 46);
                v.extreme_bg_color = Color32::from_rgb(18, 19, 24);
                v.faint_bg_color = Color32::from_rgb(36, 38, 46);
                v.widgets.inactive.weak_bg_fill = Color32::from_rgb(48, 51, 60);
                v.widgets.inactive.bg_fill = Color32::from_rgb(48, 51, 60);
                v.widgets.hovered.weak_bg_fill = Color32::from_rgb(60, 64, 76);
                v.widgets.hovered.bg_fill = Color32::from_rgb(60, 64, 76);
            }
        });
    }
}

pub fn card(ui: &Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(ui.visuals().window_fill)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(10))
}

pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    ui.add(
        egui::Button::new(egui::RichText::new(text).strong().color(Color32::from_rgb(12, 20, 36)))
            .fill(ACCENT)
            .min_size(vec2(0.0, 28.0)),
    )
}
