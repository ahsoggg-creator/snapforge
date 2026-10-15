//! The post-capture chain. A preset is a set of checkboxes; running it is two phases:
//!
//! 1. `analyze` (OCR, face detection) finds *what would be changed* so the user can review it and untick items;
//! 2. `transform` applies the approved changes and `deliver` runs the outputs (save, clipboard, upload, editor).
//!
//! Every action can be switched off, and nothing is uploaded unless the preset asks for it AND uploads are enabled.

use std::path::PathBuf;

use image::RgbaImage;
use serde::{Deserialize, Serialize};

use crate::editor::{Document, Pt, Shape};
use crate::geometry::Rect;
use crate::imageops::{self, Corner};
use crate::sensitive::{self, OcrData};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preset {
    pub name: String,
    pub save: bool,
    pub clipboard: bool,
    pub open_editor: bool,
    pub upload: bool,
    pub blur_faces: bool,
    pub ocr_to_clipboard: bool,
    pub hide_sensitive: bool,
    pub trim_margins: bool,
    pub watermark: bool,
    pub downscale: bool,
    /// Plugin actions to run on the picture, as `plugin-id/action-id`.
    pub plugin_actions: Vec<String>,
}

impl Default for Preset {
    fn default() -> Preset {
        Preset {
            name: "Preset".into(),
            save: false,
            clipboard: false,
            open_editor: true,
            upload: false,
            blur_faces: false,
            ocr_to_clipboard: false,
            hide_sensitive: false,
            trim_margins: false,
            watermark: false,
            downscale: false,
            plugin_actions: Vec::new(),
        }
    }
}

impl Preset {
    pub fn needs_analysis(&self) -> bool {
        self.blur_faces || self.hide_sensitive || self.ocr_to_clipboard
    }

    pub fn any_action(&self) -> bool {
        self.save
            || self.clipboard
            || self.open_editor
            || self.upload
            || self.blur_faces
            || self.ocr_to_clipboard
            || self.hide_sensitive
            || self.trim_margins
            || self.watermark
            || self.downscale
            || !self.plugin_actions.is_empty()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HideStyle {
    #[default]
    Pixelate,
    Blur,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PostOptions {
    pub watermark_text: String,
    /// Optional logo (PNG) used instead of text.
    pub watermark_image: Option<PathBuf>,
    pub watermark_corner: Corner,
    pub watermark_opacity: f32,
    /// Scale to this percentage of the size (1–100).
    pub downscale_percent: u32,
    pub trim_tolerance: u8,
    pub hide_style: HideStyle,
    /// OCR language tag; `None` = the user's Windows languages.
    pub ocr_language: Option<String>,
    pub face_threshold: f32,
}

impl Default for PostOptions {
    fn default() -> PostOptions {
        PostOptions {
            watermark_text: "SnapForge".into(),
            watermark_image: None,
            watermark_corner: Corner::BottomRight,
            watermark_opacity: 0.6,
            downscale_percent: 50,
            trim_tolerance: 8,
            hide_style: HideStyle::Pixelate,
            ocr_language: None,
            face_threshold: 0.7,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PostSettings {
    /// Show the "after capture" panel for every shot. Off = run the active preset immediately.
    pub show_panel: bool,
    pub presets: Vec<Preset>,
    pub active: usize,
    pub options: PostOptions,
}

impl Default for PostSettings {
    fn default() -> PostSettings {
        let p = |name: &str, f: &dyn Fn(&mut Preset)| {
            let mut x = Preset {
                name: name.into(),
                open_editor: false,
                ..Preset::default()
            };
            f(&mut x);
            x
        };
        PostSettings {
            show_panel: true,
            presets: vec![
                p("Editor", &|x| x.open_editor = true),
                p("Save + clipboard", &|x| {
                    x.save = true;
                    x.clipboard = true;
                }),
                p("Private", &|x| {
                    x.blur_faces = true;
                    x.hide_sensitive = true;
                    x.trim_margins = true;
                    x.save = true;
                    x.open_editor = true;
                }),
            ],
            active: 0,
            options: PostOptions::default(),
        }
    }
}

impl PostSettings {
    pub fn sanitize(&mut self) {
        if self.presets.is_empty() {
            self.presets = PostSettings::default().presets;
        }
        self.active = self.active.min(self.presets.len() - 1);
        self.options.downscale_percent = self.options.downscale_percent.clamp(1, 100);
        self.options.watermark_opacity = self.options.watermark_opacity.clamp(0.05, 1.0);
        self.options.face_threshold = self.options.face_threshold.clamp(0.3, 0.99);
    }

    pub fn active_preset(&self) -> &Preset {
        &self.presets[self.active.min(self.presets.len() - 1)]
    }
}

// ------------------------------------------------------------------------------------------------- analysis

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feature {
    Faces,
    Ocr,
}

#[derive(Clone, Debug, Default)]
pub struct Findings {
    pub faces: Vec<Rect>,
    pub sensitive: Vec<sensitive::Match>,
    pub ocr_text: Option<String>,
    /// Features that could not run, with an honest reason (no model, no OCR language, …).
    pub errors: Vec<(Feature, String)>,
}

/// Source of OCR and face detection; the real one uses Windows OCR and the ONNX model, tests use fakes.
pub trait Analyzer {
    fn ocr(&self, img: &RgbaImage, language: Option<&str>) -> Result<OcrData, String>;
    fn faces(&self, img: &RgbaImage, threshold: f32) -> Result<Vec<Rect>, String>;
}

#[cfg(windows)]
pub struct SystemAnalyzer;

#[cfg(windows)]
impl Analyzer for SystemAnalyzer {
    fn ocr(&self, img: &RgbaImage, language: Option<&str>) -> Result<OcrData, String> {
        crate::ocr::recognize(img, language).map_err(|e| e.to_string())
    }

    fn faces(&self, img: &RgbaImage, threshold: f32) -> Result<Vec<Rect>, String> {
        let d = crate::face::FaceDetector::load().map_err(|e| e.to_string())?;
        d.detect(img, threshold).map_err(|e| e.to_string())
    }
}

/// Runs only what the preset needs. A missing model or OCR language is reported in `errors`, never faked.
pub fn analyze(img: &RgbaImage, preset: &Preset, opts: &PostOptions, a: &dyn Analyzer) -> Findings {
    let mut f = Findings::default();
    if preset.blur_faces {
        match a.faces(img, opts.face_threshold) {
            Ok(v) => f.faces = v,
            Err(e) => f.errors.push((Feature::Faces, e)),
        }
    }
    if preset.hide_sensitive || preset.ocr_to_clipboard {
        match a.ocr(img, opts.ocr_language.as_deref()) {
            Ok(data) => {
                if preset.hide_sensitive {
                    f.sensitive = sensitive::find(&data.lines, 3);
                }
                f.ocr_text = Some(data.text);
            }
            Err(e) => f.errors.push((Feature::Ocr, e)),
        }
    }
    f
}

/// What the user approved on the review screen.
#[derive(Clone, Debug, Default)]
pub struct Approved {
    pub faces: Vec<Rect>,
    pub sensitive: Vec<Rect>,
}

impl Approved {
    pub fn all(f: &Findings) -> Approved {
        Approved {
            faces: f.faces.clone(),
            sensitive: f.sensitive.iter().map(|m| m.rect).collect(),
        }
    }
}

fn corners(r: &Rect) -> (Pt, Pt) {
    (
        Pt::new(r.x as f32, r.y as f32),
        Pt::new(r.right() as f32, r.bottom() as f32),
    )
}

/// Applies the approved redactions and the picture-wide steps, in a fixed order:
/// faces → sensitive text → trim margins → watermark → downscale.
pub fn transform(img: &RgbaImage, preset: &Preset, opts: &PostOptions, approved: &Approved) -> RgbaImage {
    let mut d = Document::new(img.clone());
    let ink = [0, 0, 0, 255];
    if preset.blur_faces {
        for r in &approved.faces {
            let (a, b) = corners(r);
            let radius = (r.w.min(r.h) / 4).max(6) as u32;
            // blur plus pixelate: a blurred face can still be recognisable, a pixelated blur much less so
            d.add(Shape::Blur { a, b, radius }, ink, 1.0);
            d.add(
                Shape::Pixelate {
                    a,
                    b,
                    block: (r.w.min(r.h) / 5).max(6) as u32,
                },
                ink,
                1.0,
            );
        }
    }
    if preset.hide_sensitive {
        for r in &approved.sensitive {
            let (a, b) = corners(r);
            match opts.hide_style {
                HideStyle::Pixelate => d.add(
                    Shape::Pixelate {
                        a,
                        b,
                        block: (r.h / 2).max(5) as u32,
                    },
                    ink,
                    1.0,
                ),
                HideStyle::Blur => d.add(
                    Shape::Blur {
                        a,
                        b,
                        radius: (r.h / 2).max(5) as u32,
                    },
                    ink,
                    1.0,
                ),
            };
        }
    }
    let mut out = d.render();
    if preset.trim_margins {
        out = imageops::trim_margins(&out, opts.trim_tolerance, 6);
    }
    if preset.watermark {
        let logo = opts
            .watermark_image
            .as_ref()
            .and_then(|p| image::open(p).ok())
            .map(|i| i.to_rgba8());
        out = match logo {
            Some(l) => imageops::watermark_image(&out, &l, opts.watermark_corner, opts.watermark_opacity, 15),
            None => imageops::watermark_text(
                &out,
                &opts.watermark_text,
                opts.watermark_corner,
                opts.watermark_opacity,
                None,
            ),
        };
    }
    if preset.downscale {
        out = imageops::scale_percent(&out, opts.downscale_percent);
    }
    out
}

// ------------------------------------------------------------------------------------------------- outputs

/// Where results go. The app implements this with real files, the clipboard, the uploader and the editor.
pub trait Sink {
    fn save(&mut self, img: &RgbaImage) -> Result<String, String>;
    fn clipboard_image(&mut self, img: &RgbaImage) -> Result<String, String>;
    fn clipboard_text(&mut self, text: &str) -> Result<String, String>;
    /// Uploads and (on success) copies the link; returns a note for the report.
    fn upload(&mut self, img: &RgbaImage) -> Result<String, String>;
    fn open_editor(&mut self, img: &RgbaImage) -> Result<String, String>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct StepReport {
    pub step: &'static str,
    pub result: Result<String, String>,
}

/// Runs the output actions in a fixed order (save, clipboard image, upload, OCR text, editor). One failing step does
/// not stop the others. If both "clipboard" and "OCR to clipboard" are on, the text is the one that stays.
pub fn deliver(img: &RgbaImage, preset: &Preset, ocr_text: Option<&str>, sink: &mut dyn Sink) -> Vec<StepReport> {
    let mut r = Vec::new();
    let mut step = |name: &'static str, res: Result<String, String>| {
        r.push(StepReport {
            step: name,
            result: res,
        })
    };
    if preset.save {
        step("save", sink.save(img));
    }
    if preset.clipboard && !(preset.ocr_to_clipboard && ocr_text.is_some()) {
        step("clipboard", sink.clipboard_image(img));
    }
    if preset.upload {
        step("upload", sink.upload(img));
    }
    if preset.ocr_to_clipboard {
        match ocr_text {
            Some(t) if !t.trim().is_empty() => step("ocr", sink.clipboard_text(t)),
            Some(_) => step("ocr", Err("no text was recognised".into())),
            None => step("ocr", Err("text recognition did not run".into())),
        }
    }
    if preset.open_editor {
        step("editor", sink.open_editor(img));
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensitive::{OcrLine, OcrWord};
    use image::Rgba;

    struct FakeAnalyzer {
        ocr: Result<OcrData, String>,
        faces: Result<Vec<Rect>, String>,
    }

    impl Analyzer for FakeAnalyzer {
        fn ocr(&self, _: &RgbaImage, _: Option<&str>) -> Result<OcrData, String> {
            self.ocr.clone()
        }

        fn faces(&self, _: &RgbaImage, _: f32) -> Result<Vec<Rect>, String> {
            self.faces.clone()
        }
    }

    fn word(t: &str, x: i32) -> OcrWord {
        OcrWord {
            text: t.into(),
            rect: Rect::new(x, 20, t.len() as i32 * 10, 24),
        }
    }

    fn ocr_with_email() -> OcrData {
        OcrData {
            text: "mail jane@example.com now".into(),
            lines: vec![OcrLine {
                words: vec![word("mail", 10), word("jane@example.com", 60), word("now", 230)],
            }],
        }
    }

    fn checker(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, y| {
            if (x / 3 + y / 3) % 2 == 0 {
                Rgba([0, 0, 0, 255])
            } else {
                Rgba([255, 255, 255, 255])
            }
        })
    }

    fn ok_fake() -> FakeAnalyzer {
        FakeAnalyzer {
            ocr: Ok(ocr_with_email()),
            faces: Ok(vec![Rect::new(100, 100, 60, 60)]),
        }
    }

    #[test]
    fn analysis_runs_only_what_the_preset_needs() {
        let p = Preset {
            open_editor: true,
            ..Preset::default()
        };
        struct Boom;
        impl Analyzer for Boom {
            fn ocr(&self, _: &RgbaImage, _: Option<&str>) -> Result<OcrData, String> {
                panic!("OCR must not run")
            }
            fn faces(&self, _: &RgbaImage, _: f32) -> Result<Vec<Rect>, String> {
                panic!("faces must not run")
            }
        }
        let f = analyze(&checker(10, 10), &p, &PostOptions::default(), &Boom);
        assert!(f.faces.is_empty() && f.sensitive.is_empty() && f.errors.is_empty());
    }

    #[test]
    fn findings_list_faces_and_sensitive_text() {
        let p = Preset {
            blur_faces: true,
            hide_sensitive: true,
            ..Preset::default()
        };
        let f = analyze(&checker(300, 200), &p, &PostOptions::default(), &ok_fake());
        assert_eq!(f.faces.len(), 1);
        assert_eq!(f.sensitive.len(), 1);
        assert_eq!(f.sensitive[0].text, "jane@example.com");
        assert_eq!(f.ocr_text.as_deref(), Some("mail jane@example.com now"));
    }

    #[test]
    fn a_missing_model_is_reported_not_faked() {
        let p = Preset {
            blur_faces: true,
            hide_sensitive: true,
            ..Preset::default()
        };
        let a = FakeAnalyzer {
            ocr: Ok(ocr_with_email()),
            faces: Err("the face detection model is not installed".into()),
        };
        let f = analyze(&checker(300, 200), &p, &PostOptions::default(), &a);
        assert!(f.faces.is_empty());
        assert_eq!(f.errors.len(), 1);
        assert_eq!(f.errors[0].0, Feature::Faces);
        assert!(f.errors[0].1.contains("not installed"));
        assert_eq!(f.sensitive.len(), 1, "the other feature still works");
    }

    #[test]
    fn only_approved_regions_are_changed() {
        let img = checker(300, 200);
        let p = Preset {
            blur_faces: true,
            hide_sensitive: true,
            ..Preset::default()
        };
        let f = analyze(&img, &p, &PostOptions::default(), &ok_fake());
        // the user unticks the face and keeps the e-mail
        let approved = Approved {
            faces: vec![],
            sensitive: f.sensitive.iter().map(|m| m.rect).collect(),
        };
        let out = transform(&img, &p, &PostOptions::default(), &approved);
        let face_area = |i: &RgbaImage| {
            (100..160)
                .flat_map(|y| (100..160).map(move |x| (x, y)))
                .map(|(x, y)| i.get_pixel(x, y).0[0] as u32)
                .collect::<Vec<_>>()
        };
        assert_eq!(face_area(&out), face_area(&img), "unticked face untouched");
        let r = f.sensitive[0].rect;
        let changed = (r.y..r.bottom())
            .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| out.get_pixel(x as u32, y as u32) != img.get_pixel(x as u32, y as u32))
            .count();
        assert!(
            changed as i64 > r.area() / 3,
            "the e-mail area was redacted ({changed} px)"
        );
        // everything approved: the face is redacted too
        let all = transform(&img, &p, &PostOptions::default(), &Approved::all(&f));
        assert_ne!(face_area(&all), face_area(&img));
    }

    #[test]
    fn redaction_destroys_the_detail_underneath() {
        let img = checker(120, 80);
        let p = Preset {
            hide_sensitive: true,
            ..Preset::default()
        };
        let a = Approved {
            faces: vec![],
            sensitive: vec![Rect::new(10, 10, 100, 40)],
        };
        let out = transform(&img, &p, &PostOptions::default(), &a);
        let row: Vec<u8> = (20..100).map(|x| out.get_pixel(x, 30).0[0]).collect();
        let spread = row.iter().max().unwrap() - row.iter().min().unwrap();
        assert!(spread < 140, "the high-contrast pattern is flattened: spread {spread}");
    }

    #[test]
    fn picture_wide_steps_apply_in_order() {
        let mut img = RgbaImage::from_pixel(400, 300, Rgba([255, 255, 255, 255]));
        for y in 100..200 {
            for x in 100..300 {
                img.put_pixel(x, y, Rgba([200, 0, 0, 255]));
            }
        }
        let p = Preset {
            trim_margins: true,
            downscale: true,
            open_editor: false,
            ..Preset::default()
        };
        let o = PostOptions {
            downscale_percent: 50,
            ..Default::default()
        };
        let out = transform(&img, &p, &o, &Approved::default());
        // content 200x100 + 6 px padding each side = 212x112, then halved
        assert_eq!(out.dimensions(), (106, 56));
        let p2 = Preset {
            watermark: true,
            ..Preset::default()
        };
        let w = transform(&img, &p2, &PostOptions::default(), &Approved::default());
        assert_eq!(w.dimensions(), img.dimensions());
        if crate::editor::render::has_font() {
            assert_ne!(w, img, "watermark drawn");
        }
    }

    #[derive(Default)]
    struct Rec {
        calls: Vec<&'static str>,
        fail: Option<&'static str>,
    }

    impl Rec {
        fn hit(&mut self, n: &'static str) -> Result<String, String> {
            self.calls.push(n);
            if self.fail == Some(n) {
                Err(format!("{n} failed"))
            } else {
                Ok(n.to_owned())
            }
        }
    }

    impl Sink for Rec {
        fn save(&mut self, _: &RgbaImage) -> Result<String, String> {
            self.hit("save")
        }
        fn clipboard_image(&mut self, _: &RgbaImage) -> Result<String, String> {
            self.hit("clipboard")
        }
        fn clipboard_text(&mut self, _: &str) -> Result<String, String> {
            self.hit("ocr")
        }
        fn upload(&mut self, _: &RgbaImage) -> Result<String, String> {
            self.hit("upload")
        }
        fn open_editor(&mut self, _: &RgbaImage) -> Result<String, String> {
            self.hit("editor")
        }
    }

    #[test]
    fn every_action_can_be_off_and_nothing_runs_by_default() {
        let mut s = Rec::default();
        let none = Preset {
            open_editor: false,
            ..Preset::default()
        };
        assert!(!none.any_action());
        assert!(deliver(&checker(4, 4), &none, None, &mut s).is_empty());
        assert!(s.calls.is_empty(), "no output without a ticked action");
        // the built-in presets never upload
        assert!(PostSettings::default().presets.iter().all(|p| !p.upload));
    }

    #[test]
    fn outputs_run_in_a_fixed_order_and_survive_a_failing_step() {
        let all = Preset {
            save: true,
            clipboard: true,
            upload: true,
            open_editor: true,
            ..Preset::default()
        };
        let mut s = Rec {
            fail: Some("save"),
            ..Default::default()
        };
        let rep = deliver(&checker(4, 4), &all, None, &mut s);
        assert_eq!(s.calls, vec!["save", "clipboard", "upload", "editor"]);
        assert!(
            rep[0].result.is_err() && rep[1].result.is_ok(),
            "a failed save does not stop the rest"
        );
    }

    #[test]
    fn upload_is_only_called_when_the_preset_asks_for_it() {
        let p = Preset {
            save: true,
            open_editor: false,
            ..Preset::default()
        };
        let mut s = Rec::default();
        deliver(&checker(4, 4), &p, None, &mut s);
        assert_eq!(s.calls, vec!["save"]);
    }

    #[test]
    fn ocr_text_wins_over_the_image_in_the_clipboard() {
        let p = Preset {
            clipboard: true,
            ocr_to_clipboard: true,
            open_editor: false,
            ..Preset::default()
        };
        let mut s = Rec::default();
        deliver(&checker(4, 4), &p, Some("hello"), &mut s);
        assert_eq!(s.calls, vec!["ocr"]);
        let mut s = Rec::default();
        let rep = deliver(&checker(4, 4), &p, None, &mut s);
        assert_eq!(
            s.calls,
            vec!["clipboard"],
            "OCR failed to run: the image still goes to the clipboard"
        );
        assert!(rep.iter().any(|r| r.step == "ocr" && r.result.is_err()));
    }

    #[test]
    fn settings_sanitize_and_roundtrip() {
        let mut p = PostSettings {
            presets: vec![],
            active: 7,
            ..Default::default()
        };
        p.options.downscale_percent = 0;
        p.sanitize();
        assert_eq!(p.presets.len(), 3);
        assert_eq!(p.active, 2);
        assert_eq!(p.options.downscale_percent, 1);
        let back: PostSettings = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
    }

    #[cfg(windows)]
    #[test]
    fn real_ocr_finds_an_email_and_the_redaction_hides_it() {
        use crate::editor::{Document, Pt, Shape};
        if crate::ocr::available_languages().is_empty() || !crate::editor::render::has_font() {
            return;
        }
        let mut d = Document::new(RgbaImage::from_pixel(900, 160, Rgba([255, 255, 255, 255])));
        d.add(
            Shape::Text {
                at: Pt::new(20.0, 50.0),
                text: "Contact jane.doe@example.com today".into(),
                size: 44.0,
            },
            [0, 0, 0, 255],
            1.0,
        );
        let img = d.render();
        let p = Preset {
            hide_sensitive: true,
            ..Preset::default()
        };
        let f = analyze(&img, &p, &PostOptions::default(), &SystemAnalyzer);
        assert!(f.errors.is_empty(), "{:?}", f.errors);
        assert_eq!(f.sensitive.len(), 1, "one e-mail found in: {:?}", f.ocr_text);
        let out = transform(&img, &p, &PostOptions::default(), &Approved::all(&f));
        // re-running OCR on the redacted picture must no longer see the address
        let again = crate::ocr::recognize(&out, None).unwrap();
        assert!(!again.text.contains("example.com"), "still readable: {}", again.text);
    }
}
