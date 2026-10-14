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

pub const STEPS: [&str; 4] = ["home", "history", "editor", "settings"];

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
            "editor" => {
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
                } else if !t.asked && t.since.elapsed() > SETTLE {
                    ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
                    t.asked = true;
                }
            }
        }
        self.tour = Some(t);
    }
}
