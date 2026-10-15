//! `SnapForge.exe --screenshots <dir>`: walks through the screens on invented data and saves a PNG of each.
//! That is how the README screenshots are made — no real file name, window or person appears in them.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, Event, ViewportCommand};
use snapforge_core::editor::{Pt, Shape};
use snapforge_core::image::RgbaImage;

use crate::editor_ui::{EditorState, Tool};
use crate::{App, View, demo};

/// Time to let the UI settle before a screenshot.
const SETTLE: Duration = Duration::from_millis(900);
/// The review step waits for OCR (slower in a debug build).
const SETTLE_REVIEW: Duration = Duration::from_millis(3500);

pub const STEPS: [&str; 9] = [
    "home",
    "history",
    "post",
    "review",
    "editor",
    "chart",
    "settings",
    "settings-post",
    "settings-uploads",
];

pub struct Tour {
    dir: PathBuf,
    step: usize,
    prepared: bool,
    asked: bool,
    since: Instant,
}

impl Tour {
    pub fn new(dir: PathBuf) -> Tour {
        Tour {
            dir,
            step: 0,
            prepared: false,
            asked: false,
            since: Instant::now(),
        }
    }
}

fn save_png(path: &std::path::Path, img: &egui::ColorImage) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let raw: Vec<u8> = img.pixels.iter().flat_map(|p| p.to_array()).collect();
    RgbaImage::from_raw(img.width() as u32, img.height() as u32, raw)
        .ok_or("bad screenshot size")?
        .save(path)
        .map_err(|e| e.to_string())
}

impl App {
    fn tour_prepare(&mut self, ctx: &egui::Context, step: &str) {
        match step {
            "home" => self.view = View::Home,
            "history" => self.view = View::History,
            "post" => {
                self.settings.post.active = 1;
                self.begin_post(ctx, demo::demo_image(2, 1100, 640), "demo-browser".into());
            }
            "review" => {
                // the "Private" preset: real OCR finds the invented address and phone number; the face model is
                // not installed in the demo's throw-away data folder, which the panel says honestly
                self.pending = None;
                self.settings.post.active = 2;
                self.begin_post(ctx, demo::demo_image(2, 1100, 640), "demo-browser".into());
                self.post_run(ctx);
            }
            "chart" => {
                self.pending = None;
                let mut es = EditorState::new(ctx, demo::demo_image(1, 1100, 640), "demo-notes".into(), None);
                let mut spec = snapforge_core::editor::ChartSpec::sample(snapforge_core::editor::ChartKind::Bar);
                spec.title = "Revenue, k€".into();
                spec.series[0].name = "2025".into();
                spec.series.push(snapforge_core::editor::Series {
                    name: "2026".into(),
                    color: snapforge_core::editor::chart::PALETTE[3],
                    values: vec![15.0, 22.0, 13.0, 21.0],
                });
                spec.show_legend = true;
                let d = &mut es.ed.doc;
                d.add(
                    Shape::Chart {
                        a: Pt::new(140.0, 150.0),
                        b: Pt::new(560.0, 520.0),
                        spec,
                    },
                    [0, 0, 0, 0],
                    1.0,
                );
                let pie = snapforge_core::editor::ChartSpec {
                    title: "Share".into(),
                    labels: ["A", "B", "C"].iter().map(|s| (*s).to_owned()).collect(),
                    series: vec![snapforge_core::editor::Series {
                        name: "share".into(),
                        color: [0, 0, 0, 255],
                        values: vec![50.0, 30.0, 20.0],
                    }],
                    ..snapforge_core::editor::ChartSpec::sample(snapforge_core::editor::ChartKind::Pie)
                };
                d.add(
                    Shape::Chart {
                        a: Pt::new(600.0, 150.0),
                        b: Pt::new(980.0, 520.0),
                        spec: pie,
                    },
                    [0, 0, 0, 0],
                    1.0,
                );
                es.tool = Tool::Select;
                self.editor = Some(es);
                self.view = View::Editor;
            }
            "settings-post" => {
                self.view = View::Settings;
                self.settings_scroll = Some(1050.0);
            }
            "settings-uploads" => {
                self.settings.uploads = snapforge_core::upload::UploadSettings {
                    enabled: true,
                    destinations: vec![snapforge_core::upload::Destination {
                        id: "demo1".into(),
                        name: "Team S3".into(),
                        enabled: false,
                        kind: snapforge_core::upload::DestKind::S3(snapforge_core::upload::S3Dest {
                            endpoint: "https://s3.example.com".into(),
                            bucket: "screenshots".into(),
                            prefix: "team/".into(),
                            ..Default::default()
                        }),
                    }],
                    active: Some("demo1".into()),
                };
                self.view = View::Settings;
                self.settings_scroll = Some(1750.0);
            }
            "editor" => {
                self.pending = None;
                let mut es = EditorState::new(ctx, demo::demo_image(0, 1100, 640), "demo-office".into(), None);
                let red = [230, 40, 40, 255];
                let d = &mut es.ed.doc;
                d.add(
                    Shape::Blur {
                        a: Pt::new(676.0, 114.0),
                        b: Pt::new(962.0, 146.0),
                        radius: 9,
                    },
                    red,
                    1.0,
                );
                d.add(
                    Shape::Pixelate {
                        a: Pt::new(676.0, 148.0),
                        b: Pt::new(962.0, 178.0),
                        block: 9,
                    },
                    red,
                    1.0,
                );
                d.add(
                    Shape::Rect {
                        a: Pt::new(86.0, 62.0),
                        b: Pt::new(1014.0, 100.0),
                        fill: false,
                    },
                    red,
                    4.0,
                );
                d.add(
                    Shape::Arrow {
                        a: Pt::new(430.0, 330.0),
                        b: Pt::new(250.0, 470.0),
                    },
                    [250, 190, 40, 255],
                    5.0,
                );
                d.add(
                    Shape::Step {
                        at: Pt::new(120.0, 140.0),
                        n: 1,
                    },
                    red,
                    4.0,
                );
                d.add(
                    Shape::Step {
                        at: Pt::new(120.0, 200.0),
                        n: 2,
                    },
                    red,
                    4.0,
                );
                d.add(
                    Shape::Text {
                        at: Pt::new(440.0, 300.0),
                        text: "Check this chart".into(),
                        size: 30.0,
                    },
                    red,
                    1.0,
                );
                es.tool = Tool::Select;
                es.selected = Some(4);
                self.editor = Some(es);
                self.view = View::Editor;
            }
            "settings" => self.view = View::Settings,
            _ => {}
        }
    }

    /// Drives the screenshot tour (no-op unless started with `--screenshots <dir>`).
    pub fn tour_step(&mut self, ctx: &egui::Context) {
        let Some(mut t) = self.tour.take() else { return };
        ctx.request_repaint();
        match STEPS.get(t.step) {
            None => ctx.send_viewport_cmd(ViewportCommand::Close),
            Some(&step) => {
                if !t.prepared {
                    self.tour_prepare(ctx, step);
                    t.prepared = true;
                    t.since = Instant::now();
                }
                let shot = ctx.input(|i| {
                    i.events.iter().find_map(|e| match e {
                        Event::Screenshot { image, .. } => Some(image.clone()),
                        _ => None,
                    })
                });
                if let Some(img) = shot {
                    let file = t.dir.join(format!("{:02}-{step}.png", t.step + 1));
                    match save_png(&file, &img) {
                        Ok(()) => println!("saved {}", file.display()),
                        Err(e) => eprintln!("cannot save {}: {e}", file.display()),
                    }
                    t.step += 1;
                    t.prepared = false;
                    t.asked = false;
                } else if !t.asked && t.since.elapsed() > if step == "review" { SETTLE_REVIEW } else { SETTLE } {
                    ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
                    t.asked = true;
                }
            }
        }
        self.tour = Some(t);
    }
}
