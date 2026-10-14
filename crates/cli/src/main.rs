//! `snapforge-cli` — the same engine as the app, from the console.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use snapforge_core::capture::{self, CaptureOptions, Target, sys};
use snapforge_core::geometry::Rect;
use snapforge_core::history::{self, History};
use snapforge_core::settings::Settings;
use snapforge_core::{naming, output, paths};

#[derive(Parser)]
#[command(name = "snapforge-cli", version, about = "SnapForge from the console: capture, history")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Take a screenshot.
    Capture(CaptureArgs),
    /// List monitors with their pixel rectangles and DPI.
    Monitors,
    /// Work with the local screenshot history.
    History {
        #[command(subcommand)]
        cmd: HistoryCmd,
    },
}

#[derive(Args)]
#[command(group(clap::ArgGroup::new("what").args(["region", "screen", "monitor", "window"])))]
struct CaptureArgs {
    /// Region in virtual-desktop pixels: X,Y,W,H (X/Y may be negative on multi-monitor setups).
    #[arg(long, value_name = "X,Y,W,H", allow_hyphen_values = true)]
    region: Option<String>,
    /// All monitors (the whole virtual desktop). This is the default.
    #[arg(long)]
    screen: bool,
    /// One monitor by index (see `monitors`).
    #[arg(long, value_name = "N")]
    monitor: Option<usize>,
    /// The foreground window.
    #[arg(long)]
    window: bool,
    /// Output file (.png or .jpg). Default: the screenshots folder from the settings.
    #[arg(long, short, value_name = "FILE")]
    out: Option<PathBuf>,
    /// Wait this many milliseconds before capturing.
    #[arg(long, default_value_t = 0)]
    delay: u32,
    /// Include the mouse cursor.
    #[arg(long)]
    cursor: bool,
    /// Use GDI instead of Windows.Graphics.Capture.
    #[arg(long)]
    gdi: bool,
    /// Also add the screenshot to the history.
    #[arg(long)]
    history: bool,
}

#[derive(Subcommand)]
enum HistoryCmd {
    /// List entries (newest first).
    List {
        /// Filter by application, date or size.
        query: Option<String>,
    },
    /// Delete every screenshot from the history.
    Clear,
}

fn parse_region(s: &str) -> Result<Rect, String> {
    let n: Vec<i32> = s
        .split(',')
        .map(|p| p.trim().parse::<i32>().map_err(|_| format!("'{p}' is not a number")))
        .collect::<Result<_, _>>()?;
    match n[..] {
        [x, y, w, h] if w > 0 && h > 0 => Ok(Rect::new(x, y, w, h)),
        [_, _, _, _] => Err("width and height must be positive".into()),
        _ => Err("expected X,Y,W,H".into()),
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.cmd {
        Cmd::Monitors => {
            for m in sys::monitors() {
                println!(
                    "{}: {}  {}x{} at ({},{})  {}% {}",
                    m.index,
                    m.name,
                    m.rect.w,
                    m.rect.h,
                    m.rect.x,
                    m.rect.y,
                    m.dpi * 100 / 96,
                    if m.primary { "primary" } else { "" }
                );
            }
            Ok(())
        }
        Cmd::Capture(a) => {
            let settings = Settings::load();
            let app = sys::foreground_app();
            let (target, app) = if let Some(r) = &a.region {
                (Target::Rect(parse_region(r)?), app)
            } else if let Some(i) = a.monitor {
                (Target::Monitor(i), app)
            } else if a.window {
                let hwnd_app = app.clone();
                let w = sys::visible_windows(0)
                    .into_iter()
                    .find(|w| w.app == hwnd_app)
                    .ok_or("no foreground window found")?;
                (Target::Rect(w.rect), w.app)
            } else {
                (Target::Screen, app)
            };
            let opts = CaptureOptions {
                cursor: a.cursor || settings.cursor_in_shot,
                force_gdi: a.gdi,
            };
            let shot = capture::take(target, a.delay, opts).map_err(|e| e.to_string())?;
            let (w, h) = shot.image.dimensions();
            let path = match &a.out {
                Some(p) => {
                    output::save_as(&shot.image, p, settings.jpeg_quality).map_err(|e| e.to_string())?;
                    p.clone()
                }
                None => {
                    let stem = naming::expand(&settings.name_template, &app, chrono::Local::now().naive_local());
                    output::save_to_dir(&shot.image, &settings.save_dir, &stem, settings.format, settings.jpeg_quality)
                        .map_err(|e| e.to_string())?
                }
            };
            if a.history {
                History::open(&paths::history_dir())
                    .add(&shot.image, &app, now_unix())
                    .map_err(|e| e.to_string())?;
            }
            println!("{} ({w}x{h}, {:?})", path.display(), shot.backend);
            Ok(())
        }
        Cmd::History { cmd } => {
            let mut h = History::open(&paths::history_dir());
            match cmd {
                HistoryCmd::List { query } => {
                    for e in h.search(query.as_deref().unwrap_or("")) {
                        println!("{}  {}x{}  {}  {}", history::date_string(e.created), e.width, e.height, e.app, e.id);
                    }
                }
                HistoryCmd::Clear => println!("deleted {}", h.clear().map_err(|e| e.to_string())?),
            }
            Ok(())
        }
    }
}

fn main() -> ExitCode {
    sys::enable_dpi_awareness();
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_parsing() {
        assert_eq!(parse_region("10,20,300,200"), Ok(Rect::new(10, 20, 300, 200)));
        assert_eq!(parse_region("-1920, 0, 100, 100"), Ok(Rect::new(-1920, 0, 100, 100)));
        assert!(parse_region("1,2,3").is_err());
        assert!(parse_region("1,2,0,5").is_err());
        assert!(parse_region("a,b,c,d").is_err());
    }
}
