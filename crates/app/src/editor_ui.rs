//! The image editor screen: tools, canvas (zoom/pan, drawing, selection), properties and layers.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{
    self, Color32, Key, PointerButton, Pos2, Rect as ERect, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions,
    Ui, vec2,
};
use snapforge_core::RgbaImage;
use snapforge_core::editor::{ChartKind, ChartSpec, DEFAULT_COLOR, DEFAULT_WIDTH, Editor, Pt, Shape};
use snapforge_core::geometry::Rect;
use snapforge_core::{diag, naming, output};

use crate::{App, View, theme};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Select,
    Crop,
    Arrow,
    Line,
    Rect,
    Ellipse,
    Text,
    Pencil,
    Marker,
    Blur,
    Pixelate,
    Step,
    Chart,
    Eraser,
    /// A tool registered by a plugin (see `EditorState::plugin_tool`); not part of `ALL`.
    Plugin,
}

impl Tool {
    pub const ALL: [Tool; 14] = [
        Tool::Select,
        Tool::Crop,
        Tool::Arrow,
        Tool::Line,
        Tool::Rect,
        Tool::Ellipse,
        Tool::Text,
        Tool::Pencil,
        Tool::Marker,
        Tool::Blur,
        Tool::Pixelate,
        Tool::Step,
        Tool::Chart,
        Tool::Eraser,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Tool::Select => "tool.select",
            Tool::Crop => "tool.crop",
            Tool::Arrow => "tool.arrow",
            Tool::Line => "tool.line",
            Tool::Rect => "tool.rect",
            Tool::Ellipse => "tool.ellipse",
            Tool::Text => "tool.text",
            Tool::Pencil => "tool.pencil",
            Tool::Marker => "tool.marker",
            Tool::Blur => "tool.blur",
            Tool::Pixelate => "tool.pixelate",
            Tool::Step => "tool.step",
            Tool::Chart => "tool.chart",
            Tool::Eraser => "tool.eraser",
            Tool::Plugin => "tool.select",
        }
    }
}

enum Gesture {
    None,
    Draw(u64),
    Move { id: u64, last: Pt },
    Handle { id: u64, idx: usize },
    Crop(Pt),
}

pub struct EditorState {
    pub ed: Editor,
    pub tool: Tool,
    pub color: [u8; 4],
    pub width: f32,
    pub fill: bool,
    pub text_size: f32,
    pub selected: Option<u64>,
    pub app: String,
    pub history_id: Option<String>,
    pub saved_path: Option<PathBuf>,
    texture: Option<TextureHandle>,
    dirty: bool,
    /// 0 = fit to the window.
    zoom: f32,
    pan: egui::Vec2,
    gesture: Gesture,
    crop: Option<(Pt, Pt)>,
    style_checkpointed: bool,
    text_checkpointed: bool,
    focus_text: bool,
    pub chart_dialog: Option<ChartDialog>,
    /// `(plugin id, tool id)` of the active plugin tool.
    pub plugin_tool: Option<(String, String)>,
    /// A click waiting to be handed to a plugin tool: `(plugin id, tool id, x, y)` in image pixels.
    pub tool_click: Option<(String, String, i64, i64)>,
}

/// Data editor for a chart object (opened after drawing one, or from the properties panel).
pub struct ChartDialog {
    pub id: u64,
    pub spec: ChartSpec,
    pub error: Option<String>,
}

pub fn pos_to_pt(p: Pos2, origin: Pos2, scale: f32) -> Pt {
    Pt::new((p.x - origin.x) / scale, (p.y - origin.y) / scale)
}

fn col(c: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

impl EditorState {
    pub fn new(_ctx: &egui::Context, image: RgbaImage, app: String, history_id: Option<String>) -> EditorState {
        EditorState {
            ed: Editor::new(image),
            tool: Tool::Arrow,
            color: DEFAULT_COLOR,
            width: DEFAULT_WIDTH,
            fill: false,
            text_size: 28.0,
            selected: None,
            app,
            history_id,
            saved_path: None,
            texture: None,
            dirty: true,
            zoom: 0.0,
            pan: egui::Vec2::ZERO,
            gesture: Gesture::None,
            crop: None,
            style_checkpointed: false,
            text_checkpointed: false,
            focus_text: false,
            chart_dialog: None,
            plugin_tool: None,
            tool_click: None,
        }
    }

    /// Adds layers (drawn elsewhere, e.g. in the capture overlay) on top of the picture.
    pub fn add_objects(&mut self, objects: Vec<snapforge_core::editor::Object>) {
        for o in objects {
            self.ed.doc.add(o.shape, o.color, o.width);
        }
        self.dirty = true;
    }

    pub fn modified(&self) -> bool {
        self.ed.can_undo()
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn refresh_texture(&mut self, ctx: &egui::Context) {
        if !self.dirty && self.texture.is_some() {
            return;
        }
        let img = self.ed.doc.render();
        let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
        let opts = TextureOptions {
            magnification: egui::TextureFilter::Nearest,
            minification: egui::TextureFilter::Linear,
            ..Default::default()
        };
        match &mut self.texture {
            Some(t) if t.size() == [ci.width(), ci.height()] => t.set(ci, opts),
            slot => *slot = Some(ctx.load_texture("editor-image", ci, opts)),
        }
        self.dirty = false;
    }

    /// Replaces the base picture (objects stay where they are); undoable.
    pub fn replace_base(&mut self, img: RgbaImage) {
        self.ed.checkpoint();
        self.ed.doc.base = Arc::new(img);
        self.dirty = true;
    }

    fn do_undo(&mut self) {
        if self.ed.undo() {
            self.selected = self.selected.filter(|id| self.ed.doc.get(*id).is_some());
            self.dirty = true;
        }
    }

    fn do_redo(&mut self) {
        if self.ed.redo() {
            self.selected = self.selected.filter(|id| self.ed.doc.get(*id).is_some());
            self.dirty = true;
        }
    }

    fn delete_selected(&mut self) {
        if let Some(id) = self.selected.take() {
            self.ed.checkpoint();
            self.ed.doc.remove(id);
            self.dirty = true;
        }
    }

    fn apply_crop(&mut self) {
        if let Some((a, b)) = self.crop.take() {
            let r = Rect::from_points(
                a.x.round() as i32,
                a.y.round() as i32,
                b.x.round() as i32,
                b.y.round() as i32,
            );
            if r.w >= 2 && r.h >= 2 {
                self.ed.checkpoint();
                if !self.ed.doc.crop(r) {
                    self.ed.cancel_checkpoint();
                }
                self.selected = None;
                self.zoom = 0.0;
                self.pan = egui::Vec2::ZERO;
                self.dirty = true;
            }
        }
    }

    fn rotate(&mut self, cw: bool) {
        self.ed.checkpoint();
        if cw {
            self.ed.doc.rotate_cw()
        } else {
            self.ed.doc.rotate_ccw()
        }
        self.crop = None;
        self.zoom = 0.0;
        self.pan = egui::Vec2::ZERO;
        self.dirty = true;
    }

    fn shape_for(&self, tool: Tool, p: Pt) -> Option<Shape> {
        Some(match tool {
            Tool::Arrow => Shape::Arrow { a: p, b: p },
            Tool::Line => Shape::Line { a: p, b: p },
            Tool::Rect => Shape::Rect {
                a: p,
                b: p,
                fill: self.fill,
            },
            Tool::Ellipse => Shape::Ellipse {
                a: p,
                b: p,
                fill: self.fill,
            },
            Tool::Pencil => Shape::Pencil { pts: vec![p] },
            Tool::Marker => Shape::Marker { pts: vec![p] },
            Tool::Blur => Shape::Blur { a: p, b: p, radius: 10 },
            Tool::Pixelate => Shape::Pixelate { a: p, b: p, block: 10 },
            Tool::Chart => Shape::Chart {
                a: p,
                b: p,
                spec: ChartSpec::sample(ChartKind::Bar),
            },
            _ => return None,
        })
    }

    fn update_draw(&mut self, id: u64, p: Pt) {
        if let Some(o) = self.ed.doc.get_mut(id) {
            match &mut o.shape {
                Shape::Arrow { b, .. }
                | Shape::Line { b, .. }
                | Shape::Rect { b, .. }
                | Shape::Ellipse { b, .. }
                | Shape::Blur { b, .. }
                | Shape::Chart { b, .. }
                | Shape::Pixelate { b, .. } => *b = p,
                Shape::Pencil { pts } | Shape::Marker { pts }
                    if pts.last().is_none_or(|l| (l.x - p.x).hypot(l.y - p.y) > 0.8) =>
                {
                    pts.push(p);
                }
                _ => {}
            }
            self.dirty = true;
        }
    }

    /// True if a just-drawn shape is too small to be intentional.
    fn is_tiny(&self, id: u64) -> bool {
        match self.ed.doc.get(id).map(|o| &o.shape) {
            Some(Shape::Arrow { a, b }) | Some(Shape::Line { a, b }) => (a.x - b.x).hypot(a.y - b.y) < 3.0,
            Some(Shape::Rect { a, b, .. })
            | Some(Shape::Ellipse { a, b, .. })
            | Some(Shape::Blur { a, b, .. })
            | Some(Shape::Pixelate { a, b, .. }) => (a.x - b.x).abs() < 3.0 || (a.y - b.y).abs() < 3.0,
            Some(Shape::Chart { a, b, .. }) => (a.x - b.x).abs() < 60.0 || (a.y - b.y).abs() < 60.0,
            _ => false,
        }
    }

    /// The canvas: draws the image and handles every pointer interaction.
    pub fn canvas(&mut self, ui: &mut Ui) {
        self.refresh_texture(ui.ctx());
        let avail = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(avail, Sense::click_and_drag());
        let painter = ui.painter_at(avail);
        painter.rect_filled(avail, 0.0, ui.visuals().extreme_bg_color);
        let Some(tex) = self.texture.clone() else { return };
        let (iw, ih) = self.ed.doc.size();
        let (iwf, ihf) = (iw as f32, ih as f32);
        let fit = ((avail.width() - 24.0) / iwf)
            .min((avail.height() - 24.0) / ihf)
            .clamp(0.02, 1.0);
        let scale = if self.zoom > 0.0 { self.zoom } else { fit };
        let size = vec2(iwf * scale, ihf * scale);
        let img_rect = ERect::from_center_size(avail.center() + self.pan, size);

        // zoom around the cursor (Ctrl + wheel), pan with the middle button
        if resp.hovered() {
            let (scroll, ctrl, cursor) =
                ui.input(|i| (i.smooth_scroll_delta.y, i.modifiers.ctrl, i.pointer.hover_pos()));
            if let (true, true, Some(c)) = (ctrl, scroll != 0.0, cursor) {
                let p = pos_to_pt(c, img_rect.min, scale);
                let ns = (scale * 1.12_f32.powf(scroll.signum())).clamp(0.05, 16.0);
                let min = Pos2::new(c.x - p.x * ns, c.y - p.y * ns);
                let new_center = min + vec2(iwf * ns, ihf * ns) / 2.0;
                self.zoom = ns;
                self.pan = new_center - avail.center();
            }
        }
        if resp.dragged_by(PointerButton::Middle) {
            self.pan += resp.drag_delta();
        }

        painter.image(
            tex.id(),
            img_rect,
            ERect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
        painter.rect_stroke(
            img_rect,
            0.0,
            Stroke::new(1.0, Color32::from_gray(70)),
            StrokeKind::Outside,
        );

        let pos = resp
            .interact_pointer_pos()
            .or_else(|| ui.input(|i| i.pointer.hover_pos()));
        let at = pos.map(|p| pos_to_pt(p, img_rect.min, scale));
        let tool = self.tool;
        let handle_reach = 9.0 / scale;

        if resp.drag_started_by(PointerButton::Primary)
            && let Some(p) = at
        {
            self.gesture = Gesture::None;
            match tool {
                Tool::Select => {
                    let sel_handle = self.selected.and_then(|id| {
                        let o = self.ed.doc.get(id)?;
                        o.shape
                            .handles()
                            .iter()
                            .position(|h| (h.x - p.x).hypot(h.y - p.y) <= handle_reach)
                            .map(|idx| (id, idx))
                    });
                    if let Some((id, idx)) = sel_handle {
                        self.ed.checkpoint();
                        self.gesture = Gesture::Handle { id, idx };
                    } else if let Some(id) = self.ed.doc.hit_test(p, handle_reach) {
                        self.selected = Some(id);
                        self.ed.checkpoint();
                        self.gesture = Gesture::Move { id, last: p };
                    } else {
                        self.selected = None;
                    }
                }
                Tool::Crop => {
                    self.crop = Some((p, p));
                    self.gesture = Gesture::Crop(p);
                }
                Tool::Text | Tool::Step | Tool::Eraser | Tool::Plugin => {}
                _ => {
                    if let Some(shape) = self.shape_for(tool, p) {
                        self.ed.checkpoint();
                        let id = self.ed.doc.add(shape, self.color, self.width);
                        self.selected = Some(id);
                        self.gesture = Gesture::Draw(id);
                        self.dirty = true;
                    }
                }
            }
        }
        if resp.dragged_by(PointerButton::Primary)
            && let Some(p) = at
        {
            match self.gesture {
                Gesture::Draw(id) => self.update_draw(id, p),
                Gesture::Move { id, last } => {
                    if let Some(o) = self.ed.doc.get_mut(id) {
                        o.shape.translate(p.x - last.x, p.y - last.y);
                    }
                    self.gesture = Gesture::Move { id, last: p };
                    self.dirty = true;
                }
                Gesture::Handle { id, idx } => {
                    if let Some(o) = self.ed.doc.get_mut(id) {
                        o.shape.move_handle(idx, p);
                    }
                    self.dirty = true;
                }
                Gesture::Crop(start) => {
                    let clamp = |v: f32, max: f32| v.clamp(0.0, max);
                    self.crop = Some((start, Pt::new(clamp(p.x, iwf), clamp(p.y, ihf))));
                }
                Gesture::None => {}
            }
        }
        if resp.drag_stopped_by(PointerButton::Primary) {
            if let Gesture::Draw(id) = self.gesture
                && self.is_tiny(id)
            {
                self.ed.doc.remove(id);
                self.ed.cancel_checkpoint();
                self.selected = None;
                self.dirty = true;
            }
            // a freshly drawn chart asks for its data right away
            if let Gesture::Draw(id) = self.gesture
                && let Some(Shape::Chart { spec, .. }) = self.ed.doc.get(id).map(|o| o.shape.clone())
            {
                self.chart_dialog = Some(ChartDialog { id, spec, error: None });
            }
            self.gesture = Gesture::None;
        }

        if resp.clicked_by(PointerButton::Primary)
            && let Some(p) = at
        {
            match tool {
                Tool::Select => self.selected = self.ed.doc.hit_test(p, handle_reach),
                Tool::Eraser => {
                    if let Some(id) = self.ed.doc.hit_test(p, handle_reach) {
                        self.ed.checkpoint();
                        self.ed.doc.remove(id);
                        self.selected = None;
                        self.dirty = true;
                    }
                }
                Tool::Text => {
                    self.ed.checkpoint();
                    let id = self.ed.doc.add(
                        Shape::Text {
                            at: p,
                            text: "Text".into(),
                            size: self.text_size,
                        },
                        self.color,
                        self.width,
                    );
                    self.selected = Some(id);
                    self.focus_text = true;
                    self.dirty = true;
                }
                Tool::Plugin => {
                    if let Some((pid, id)) = self.plugin_tool.clone() {
                        self.tool_click = Some((pid, id, p.x as i64, p.y as i64));
                    }
                }
                Tool::Step => {
                    self.ed.checkpoint();
                    let n = self.ed.doc.next_step_number();
                    let id = self.ed.doc.add(Shape::Step { at: p, n }, self.color, self.width);
                    self.selected = Some(id);
                    self.dirty = true;
                }
                _ => {}
            }
        }

        // keyboard
        let (del, esc, enter, undo, redo) = ui.input(|i| {
            (
                i.key_pressed(Key::Delete),
                i.key_pressed(Key::Escape),
                i.key_pressed(Key::Enter),
                i.modifiers.command && i.key_pressed(Key::Z) && !i.modifiers.shift,
                i.modifiers.command && (i.key_pressed(Key::Y) || (i.key_pressed(Key::Z) && i.modifiers.shift)),
            )
        });
        let typing = ui.ctx().egui_wants_keyboard_input();
        if !typing {
            if del {
                self.delete_selected();
            }
            if undo {
                self.do_undo();
            }
            if redo {
                self.do_redo();
            }
        }
        if esc {
            self.crop = None;
            self.selected = None;
        }
        if enter && !typing && self.crop.is_some() {
            self.apply_crop();
        }

        // overlays
        let to_screen = |p: Pt| Pos2::new(img_rect.min.x + p.x * scale, img_rect.min.y + p.y * scale);
        if let Some((a, b)) = self.crop {
            let r = ERect::from_two_pos(to_screen(a), to_screen(b));
            let dim = Color32::from_black_alpha(140);
            painter.rect_filled(
                ERect::from_min_max(img_rect.min, Pos2::new(img_rect.max.x, r.min.y)),
                0.0,
                dim,
            );
            painter.rect_filled(
                ERect::from_min_max(Pos2::new(img_rect.min.x, r.max.y), img_rect.max),
                0.0,
                dim,
            );
            painter.rect_filled(
                ERect::from_min_max(Pos2::new(img_rect.min.x, r.min.y), Pos2::new(r.min.x, r.max.y)),
                0.0,
                dim,
            );
            painter.rect_filled(
                ERect::from_min_max(Pos2::new(r.max.x, r.min.y), Pos2::new(img_rect.max.x, r.max.y)),
                0.0,
                dim,
            );
            painter.rect_stroke(r, 0.0, Stroke::new(1.5, theme::ACCENT), StrokeKind::Middle);
        }
        if let Some(o) = self.selected.and_then(|id| self.ed.doc.get(id)) {
            let (lo, hi) = o.shape.bounds(o.width);
            let r = ERect::from_two_pos(to_screen(lo), to_screen(hi)).expand(2.0);
            painter.rect_stroke(r, 2.0, Stroke::new(1.0, theme::ACCENT), StrokeKind::Outside);
            for h in o.shape.handles() {
                let hr = ERect::from_center_size(to_screen(h), vec2(9.0, 9.0));
                painter.rect_filled(hr, 2.0, Color32::WHITE);
                painter.rect_stroke(hr, 2.0, Stroke::new(1.5, theme::ACCENT), StrokeKind::Outside);
            }
        }
        if tool != Tool::Select && resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
    }

    /// Style controls and the selected object's own settings.
    pub fn properties(&mut self, ui: &mut Ui, t: &dyn Fn(&'static str) -> &'static str) {
        ui.label(egui::RichText::new(t("ed.style")).strong());
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label(t("ed.color"));
            changed |= ui.color_edit_button_srgba_unmultiplied(&mut self.color).changed();
        });
        ui.horizontal_wrapped(|ui| {
            for c in [
                [230, 40, 40, 255],
                [250, 190, 40, 255],
                [60, 190, 90, 255],
                [60, 140, 250, 255],
                [255, 255, 255, 255],
                [20, 20, 20, 255],
            ] {
                let (r, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
                ui.painter().rect_filled(r, 4.0, col(c));
                ui.painter()
                    .rect_stroke(r, 4.0, Stroke::new(1.0, Color32::from_gray(90)), StrokeKind::Inside);
                if resp.clicked() {
                    self.color = c;
                    changed = true;
                }
            }
        });
        changed |= ui
            .add(egui::Slider::new(&mut self.width, 1.0..=24.0).text(t("ed.width")))
            .changed();
        changed |= ui.checkbox(&mut self.fill, t("ed.fill")).changed();
        changed |= ui
            .add(egui::Slider::new(&mut self.text_size, 10.0..=120.0).text(t("ed.text_size")))
            .changed();

        if changed && let Some(id) = self.selected {
            if !self.style_checkpointed {
                self.ed.checkpoint();
                self.style_checkpointed = true;
            }
            let (color, width, fill, size) = (self.color, self.width, self.fill, self.text_size);
            if let Some(o) = self.ed.doc.get_mut(id) {
                o.color = color;
                o.width = width;
                match &mut o.shape {
                    Shape::Rect { fill: f, .. } | Shape::Ellipse { fill: f, .. } => *f = fill,
                    Shape::Text { size: s, .. } => *s = size,
                    _ => {}
                }
            }
            self.dirty = true;
        }
        if !ui.input(|i| i.pointer.any_down()) {
            self.style_checkpointed = false;
        }

        if let Some(id) = self.selected {
            if let Some(Shape::Chart { spec, .. }) = self.ed.doc.get(id).map(|o| o.shape.clone())
                && ui.button(t("ed.chart_edit")).clicked()
            {
                self.chart_dialog = Some(ChartDialog { id, spec, error: None });
            }
            if let Some(Shape::Text { text, .. }) = self.ed.doc.get(id).map(|o| o.shape.clone()) {
                ui.separator();
                let mut edited = text.clone();
                let r = ui.add(
                    egui::TextEdit::multiline(&mut edited)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                );
                if self.focus_text {
                    r.request_focus();
                    self.focus_text = false;
                }
                if r.changed() {
                    if !self.text_checkpointed {
                        self.ed.checkpoint();
                        self.text_checkpointed = true;
                    }
                    if let Some(o) = self.ed.doc.get_mut(id)
                        && let Shape::Text { text, .. } = &mut o.shape
                    {
                        *text = edited;
                    }
                    self.dirty = true;
                }
                if r.lost_focus() {
                    self.text_checkpointed = false;
                }
            }
            if ui.button(t("ed.delete")).clicked() {
                self.delete_selected();
            }
        }
        ui.separator();
        ui.label(egui::RichText::new(t("ed.layers")).strong());
        let ids: Vec<(u64, &'static str, bool)> = self
            .ed
            .doc
            .objects
            .iter()
            .rev()
            .map(|o| (o.id, o.shape.kind_name(), o.visible))
            .collect();
        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
            for (id, kind, visible) in ids {
                ui.horizontal(|ui| {
                    let mut v = visible;
                    if ui.checkbox(&mut v, "").on_hover_text(t("ed.visible")).changed() {
                        self.ed.checkpoint();
                        if let Some(o) = self.ed.doc.get_mut(id) {
                            o.visible = v;
                        }
                        self.dirty = true;
                    }
                    let label = format!(
                        "{} #{id}",
                        t(match kind {
                            "line" => "tool.line",
                            "arrow" => "tool.arrow",
                            "rect" => "tool.rect",
                            "ellipse" => "tool.ellipse",
                            "text" => "tool.text",
                            "pencil" => "tool.pencil",
                            "marker" => "tool.marker",
                            "blur" => "tool.blur",
                            "pixelate" => "tool.pixelate",
                            "chart" => "tool.chart",
                            _ => "tool.step",
                        })
                    );
                    if ui.selectable_label(self.selected == Some(id), label).clicked() {
                        self.selected = Some(id);
                    }
                    if ui.small_button("^").clicked() {
                        self.ed.checkpoint();
                        self.ed.doc.move_layer(id, true);
                        self.dirty = true;
                    }
                    if ui.small_button("v").clicked() {
                        self.ed.checkpoint();
                        self.ed.doc.move_layer(id, false);
                        self.dirty = true;
                    }
                });
            }
        });
    }
}

impl App {
    pub fn editor_view(&mut self, ui: &mut Ui) {
        let Some(mut es) = self.editor.take() else {
            self.view = View::Home;
            return;
        };
        let lang = self.settings.language;
        let t = move |k: &'static str| crate::i18n::tr(lang, k);
        let mut action: Option<EditorAction> = None;

        egui::Panel::top("editor_bar").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button(t("ed.close")).clicked() {
                    action = Some(EditorAction::Close);
                }
                ui.separator();
                if ui
                    .add_enabled(es.ed.can_undo(), egui::Button::new(t("ed.undo")))
                    .clicked()
                {
                    es.do_undo();
                }
                if ui
                    .add_enabled(es.ed.can_redo(), egui::Button::new(t("ed.redo")))
                    .clicked()
                {
                    es.do_redo();
                }
                ui.separator();
                if ui.button(t("ed.rot_left")).clicked() {
                    es.rotate(false);
                }
                if ui.button(t("ed.rot_right")).clicked() {
                    es.rotate(true);
                }
                ui.separator();
                if ui.button(t("ed.fit")).clicked() {
                    es.zoom = 0.0;
                    es.pan = egui::Vec2::ZERO;
                }
                if ui.button("100%").clicked() {
                    es.zoom = 1.0;
                    es.pan = egui::Vec2::ZERO;
                }
                ui.separator();
                if ui.button(t("ed.ocr")).clicked() {
                    action = Some(EditorAction::Ocr);
                }
                ui.separator();
                if theme::primary_button(ui, t("ed.save")).clicked() {
                    action = Some(EditorAction::Save);
                }
                if ui.button(t("ed.save_as")).clicked() {
                    action = Some(EditorAction::SaveAs);
                }
                if ui.button(t("ed.copy")).clicked() {
                    action = Some(EditorAction::Copy);
                }
                let drag = ui
                    .add(egui::Button::new(t("ed.drag")).sense(Sense::click_and_drag()))
                    .on_hover_text(t("ed.drag_hint"));
                if drag.drag_started() {
                    action = Some(EditorAction::DragOut);
                }
                if let Some(p) = &es.saved_path
                    && ui.link(egui::RichText::new(p.display().to_string()).small()).clicked()
                {
                    crate::winint::reveal(p);
                }
            });
        });
        egui::Panel::left("editor_tools").exact_size(140.0).show(ui, |ui| {
            for tool in Tool::ALL {
                let sel = es.tool == tool;
                if ui
                    .add_sized([124.0, 26.0], egui::Button::selectable(sel, t(tool.key())))
                    .clicked()
                {
                    es.tool = tool;
                    es.crop = None;
                    if tool != Tool::Select {
                        es.selected = None;
                    }
                }
            }
            let plugin_tools = self.plugins.host.tools();
            if !plugin_tools.is_empty() {
                ui.separator();
                ui.label(egui::RichText::new(t("ed.plugin_tools")).weak().small());
                for pt in &plugin_tools {
                    let sel = es.tool == Tool::Plugin
                        && es.plugin_tool.as_ref() == Some(&(pt.plugin_id.clone(), pt.id.clone()));
                    if ui
                        .add_sized([124.0, 26.0], egui::Button::selectable(sel, &pt.title))
                        .on_hover_text(&pt.plugin_name)
                        .clicked()
                    {
                        es.tool = Tool::Plugin;
                        es.plugin_tool = Some((pt.plugin_id.clone(), pt.id.clone()));
                        es.crop = None;
                        es.selected = None;
                    }
                }
            }
            if es.tool == Tool::Crop && es.crop.is_some() {
                ui.separator();
                if theme::primary_button(ui, t("ed.apply_crop")).clicked() {
                    es.apply_crop();
                }
            }
        });
        egui::Panel::right("editor_props").exact_size(280.0).show(ui, |ui| {
            ui.spacing_mut().slider_width = 110.0;
            es.properties(ui, &t);
        });
        egui::CentralPanel::default().frame(egui::Frame::new()).show(ui, |ui| {
            es.canvas(ui);
        });
        // a click with a plugin tool: the plugin works on the base picture and its result replaces it
        if let Some((pid, id, x, y)) = es.tool_click.take() {
            let (host, base, ctx2) = (self.plugins.host.clone(), (*es.ed.doc.base).clone(), ui.ctx().clone());
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(host.run_tool(&pid, &id, &base, x, y));
                ctx2.request_repaint();
            });
            self.tool_rx = Some(rx);
        }
        if let Some(rx) = &self.tool_rx
            && let Ok((img, report)) = rx.try_recv()
        {
            self.tool_rx = None;
            if let Some(img) = img {
                es.replace_base(img);
            }
            let _ = self.plugins.reports_tx.send(report);
        }

        let ctx = ui.ctx().clone();
        chart_dialog_ui(&ctx, &mut es, &t);
        self.editor = Some(es);
        self.ocr_window(&ctx);
        if let Some(a) = action {
            self.run_editor_action(ui.ctx(), a);
        }
        if self.confirm_close_editor {
            self.close_confirm(ui.ctx());
        }
    }

    fn close_confirm(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut keep = false;
        egui::Window::new(self.t("ed.unsaved_title"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(self.t("ed.unsaved_text"));
                ui.horizontal(|ui| {
                    if ui.button(self.t("ed.discard")).clicked() {
                        close = true;
                    }
                    if theme::primary_button(ui, self.t("ed.keep_editing")).clicked() {
                        keep = true;
                    }
                });
            });
        if close {
            self.editor = None;
            self.view = View::Home;
            self.confirm_close_editor = false;
        }
        if keep {
            self.confirm_close_editor = false;
        }
    }

    fn run_editor_action(&mut self, ctx: &egui::Context, a: EditorAction) {
        match a {
            EditorAction::Close => {
                let unsaved = self
                    .editor
                    .as_ref()
                    .is_some_and(|e| e.modified() && e.saved_path.is_none());
                if unsaved {
                    self.confirm_close_editor = true;
                } else {
                    self.editor = None;
                    self.view = View::Home;
                }
            }
            EditorAction::Save => self.editor_save(),
            EditorAction::SaveAs => self.editor_save_as(),
            EditorAction::Copy => self.editor_copy(),
            EditorAction::DragOut => self.editor_drag_out(ctx),
            EditorAction::Ocr => self.editor_ocr(ctx),
        }
    }

    fn flattened(&self) -> Option<(RgbaImage, String)> {
        self.editor.as_ref().map(|e| (e.ed.doc.render(), e.app.clone()))
    }

    /// After saving or copying an edited picture, the history keeps the edited version.
    fn sync_history(&mut self, img: &RgbaImage) {
        let Some(id) = self
            .editor
            .as_ref()
            .and_then(|e| e.modified().then(|| e.history_id.clone()).flatten())
        else {
            return;
        };
        if self.history.replace_image(&id, img).is_ok() {
            self.thumbs.remove(&id);
        }
    }

    pub fn editor_save(&mut self) {
        let Some((img, app)) = self.flattened() else { return };
        let stem = naming::expand(&self.settings.name_template, &app, chrono::Local::now().naive_local());
        let s = &self.settings;
        match output::save_to_dir(&img, &s.save_dir, &stem, s.format, s.jpeg_quality) {
            Ok(path) => {
                diag::log(diag::Level::Info, &format!("saved {}x{}", img.width(), img.height()));
                self.sync_history(&img);
                let msg = self.tf("ed.saved", &[("path", path.display().to_string())]);
                if let Some(e) = &mut self.editor {
                    e.saved_path = Some(path);
                }
                self.notify(msg, false);
            }
            Err(e) => {
                let msg = format!("{}: {e}", self.t("err.save"));
                self.notify(msg, true);
            }
        }
    }

    pub fn editor_save_as(&mut self) {
        let Some((img, app)) = self.flattened() else { return };
        let stem = naming::expand(&self.settings.name_template, &app, chrono::Local::now().naive_local());
        let mut dlg = rfd::FileDialog::new()
            .add_filter("PNG", &["png"])
            .add_filter("JPEG", &["jpg", "jpeg"])
            .set_file_name(format!("{stem}.{}", self.settings.format.ext()));
        if self.settings.save_dir.is_dir() {
            dlg = dlg.set_directory(&self.settings.save_dir);
        }
        if let Some(path) = dlg.save_file() {
            match output::save_as(&img, &path, self.settings.jpeg_quality) {
                Ok(()) => {
                    self.sync_history(&img);
                    let msg = self.tf("ed.saved", &[("path", path.display().to_string())]);
                    if let Some(e) = &mut self.editor {
                        e.saved_path = Some(path);
                    }
                    self.notify(msg, false);
                }
                Err(e) => {
                    let msg = format!("{}: {e}", self.t("err.save"));
                    self.notify(msg, true);
                }
            }
        }
    }

    pub fn editor_copy(&mut self) {
        let Some((img, _)) = self.flattened() else { return };
        match output::copy_to_clipboard(&img) {
            Ok(()) => {
                self.sync_history(&img);
                let msg = self.t("ed.copied").to_owned();
                self.notify(msg, false);
            }
            Err(e) => {
                let msg = format!("{}: {e}", self.t("err.clipboard"));
                self.notify(msg, true);
            }
        }
    }

    /// Writes the picture to a temp file and starts an OLE drag of that file (drop it into Explorer, a chat, …).
    fn editor_drag_out(&mut self, _ctx: &egui::Context) {
        let Some((img, app)) = self.flattened() else { return };
        let stem = naming::expand(&self.settings.name_template, &app, chrono::Local::now().naive_local());
        let dir = std::env::temp_dir().join("SnapForge-drag");
        match output::save_to_dir(&img, &dir, &stem, self.settings.format, self.settings.jpeg_quality) {
            Ok(path) => {
                if let Err(e) = crate::dragout::drag_file(&path) {
                    self.notify(e, true);
                }
            }
            Err(e) => {
                let msg = format!("{}: {e}", self.t("err.save"));
                self.notify(msg, true);
            }
        }
    }
}

pub enum EditorAction {
    Close,
    Save,
    SaveAs,
    Copy,
    DragOut,
    Ocr,
}

/// OCR window state: running in the background, or finished.
pub enum OcrState {
    Running(std::sync::mpsc::Receiver<Result<String, String>>),
    Done(Result<String, String>),
}

fn chart_dialog_ui(ctx: &egui::Context, es: &mut EditorState, t: &dyn Fn(&'static str) -> &'static str) {
    let Some(mut dlg) = es.chart_dialog.take() else { return };
    let (mut apply, mut cancel) = (false, false);
    egui::Window::new(t("chart.title"))
        .collapsible(false)
        .resizable(true)
        .default_width(560.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            let spec = &mut dlg.spec;
            ui.horizontal(|ui| {
                ui.label(t("chart.name"));
                ui.add(egui::TextEdit::singleline(&mut spec.title).desired_width(200.0));
                ui.label(t("chart.kind"));
                for (k, key) in [
                    (ChartKind::Bar, "chart.bar"),
                    (ChartKind::Line, "chart.line"),
                    (ChartKind::Pie, "chart.pie"),
                ] {
                    ui.selectable_value(&mut spec.kind, k, t(key));
                }
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut spec.show_legend, t("chart.legend"));
                ui.checkbox(&mut spec.show_values, t("chart.values"));
                if ui.button(t("chart.paste")).clicked() {
                    let parsed = snapforge_core::output::read_text_from_clipboard()
                        .map_err(|e| e.to_string())
                        .and_then(|text| {
                            snapforge_core::editor::chart::spec_from_table(
                                &snapforge_core::editor::chart::parse_table(&text),
                                spec.kind,
                            )
                        });
                    match parsed {
                        Ok(new) => {
                            spec.labels = new.labels;
                            spec.series = new.series;
                            spec.show_legend = new.show_legend;
                            dlg.error = None;
                        }
                        Err(e) => dlg.error = Some(format!("{}: {e}", t("chart.paste_err"))),
                    }
                }
            });
            if let Some(e) = &dlg.error {
                ui.colored_label(theme::BAD, e);
            }
            let spec = &mut dlg.spec;
            egui::ScrollArea::both().max_height(280.0).show(ui, |ui| {
                egui::Grid::new("chart_grid").spacing([6.0, 4.0]).show(ui, |ui| {
                    ui.label(egui::RichText::new(t("chart.label")).weak());
                    for s in &mut spec.series {
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut s.name).desired_width(80.0));
                            ui.color_edit_button_srgba_unmultiplied(&mut s.color);
                        });
                    }
                    ui.end_row();
                    for i in 0..spec.labels.len() {
                        ui.add(egui::TextEdit::singleline(&mut spec.labels[i]).desired_width(90.0));
                        for s in &mut spec.series {
                            if let Some(v) = s.values.get_mut(i) {
                                ui.add(egui::DragValue::new(v).speed(0.5));
                            }
                        }
                        ui.end_row();
                    }
                });
            });
            ui.horizontal(|ui| {
                if ui.button(t("chart.add_row")).clicked() {
                    spec.labels.push(format!("#{}", spec.labels.len() + 1));
                }
                if spec.labels.len() > 1 && ui.button(t("chart.del_row")).clicked() {
                    spec.labels.pop();
                }
                if ui.button(t("chart.add_col")).clicked() {
                    let n = spec.series.len();
                    let palette = &snapforge_core::editor::chart::PALETTE;
                    spec.series.push(snapforge_core::editor::Series {
                        name: format!("{} {}", t("chart.series"), n + 1),
                        color: palette[n % palette.len()],
                        values: Vec::new(),
                    });
                }
                if spec.series.len() > 1 && ui.button(t("chart.del_col")).clicked() {
                    spec.series.pop();
                }
            });
            spec.normalize();
            ui.separator();
            ui.horizontal(|ui| {
                if theme::primary_button(ui, t("chart.apply")).clicked() {
                    apply = true;
                }
                if ui.button(t("common.cancel")).clicked() {
                    cancel = true;
                }
            });
        });
    if apply {
        es.ed.checkpoint();
        if let Some(o) = es.ed.doc.get_mut(dlg.id)
            && let Shape::Chart { spec, .. } = &mut o.shape
        {
            *spec = dlg.spec.clone();
        }
        es.selected = Some(dlg.id);
        es.dirty = true;
    } else if !cancel {
        es.chart_dialog = Some(dlg);
    }
}

impl App {
    pub fn editor_ocr(&mut self, ctx: &egui::Context) {
        let Some((img, _)) = self.flattened() else { return };
        let lang = self.settings.post.options.ocr_language.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx2 = ctx.clone();
        std::thread::spawn(move || {
            let r = snapforge_core::ocr::recognize(&img, lang.as_deref())
                .map(|d| d.text)
                .map_err(|e| e.to_string());
            let _ = tx.send(r);
            ctx2.request_repaint();
        });
        self.ocr_state = Some(OcrState::Running(rx));
    }

    pub fn ocr_window(&mut self, ctx: &egui::Context) {
        let lang = self.settings.language;
        if let Some(OcrState::Running(rx)) = &self.ocr_state
            && let Ok(r) = rx.try_recv()
        {
            self.ocr_state = Some(OcrState::Done(r));
        }
        let mut close = false;
        let mut copy = None;
        if let Some(state) = &self.ocr_state {
            egui::Window::new(crate::i18n::tr(lang, "ocr.title"))
                .collapsible(false)
                .default_width(480.0)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| match state {
                    OcrState::Running(_) => {
                        ui.add(egui::Spinner::new().size(24.0).color(theme::ACCENT));
                        ui.label(crate::i18n::tr(lang, "ocr.running"));
                        ctx.request_repaint();
                    }
                    OcrState::Done(Ok(text)) => {
                        if text.trim().is_empty() {
                            ui.label(crate::i18n::tr(lang, "ocr.empty"));
                        } else {
                            let mut shown = text.clone();
                            ui.add(
                                egui::TextEdit::multiline(&mut shown)
                                    .desired_rows(8)
                                    .desired_width(f32::INFINITY),
                            );
                            if ui.button(crate::i18n::tr(lang, "ocr.copy")).clicked() {
                                copy = Some(text.clone());
                            }
                        }
                        if ui.button(crate::i18n::tr(lang, "inst.close")).clicked() {
                            close = true;
                        }
                    }
                    OcrState::Done(Err(e)) => {
                        ui.colored_label(theme::BAD, e);
                        if ui.button(crate::i18n::tr(lang, "inst.close")).clicked() {
                            close = true;
                        }
                    }
                });
        }
        if let Some(text) = copy {
            match output::copy_text_to_clipboard(&text) {
                Ok(()) => self.notify(crate::i18n::tr(lang, "ocr.copied"), false),
                Err(e) => self.notify(e.to_string(), true),
            }
        }
        if close {
            self.ocr_state = None;
        }
    }
}
