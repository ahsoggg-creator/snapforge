//! The plugin sandbox: Rhai scripts with hard limits and an API where every capability checks the permissions the
//! user approved. A script can only reach the outside world through the functions registered here — Rhai itself has
//! no file, network or process access, `eval` and `import` are switched off, and there is no way to ask for upload
//! tokens (they are not part of the API at all).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use image::RgbaImage;
use rhai::{AST, Array, Dynamic, Engine, EvalAltResult, ImmutableString, Map, Position, Scope};

use super::manifest::{Manifest, Permissions};
use crate::editor::render::{blur_region, measure_text, overlay, pixelate_region};
use crate::editor::{Document, Pt, Shape};
use crate::geometry::Rect;
use crate::upload::{self, FilePart, HttpRequest, HttpResponse};

/// Hard limits for one run (one hook call).
#[derive(Clone, Debug)]
pub struct Limits {
    /// Rhai "operations" (roughly one per expression/statement step).
    pub max_ops: u64,
    pub max_time: Duration,
    pub max_call_depth: usize,
    pub max_expr_depth: usize,
    pub max_string: usize,
    pub max_array: usize,
    pub max_map: usize,
    /// Total bytes of pixels the script may allocate (new images, crops, clones, loaded files) in one run.
    pub max_image_bytes: u64,
    pub max_file_bytes: u64,
    pub max_http_bytes: usize,
    pub max_source_bytes: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_ops: 20_000_000,
            max_time: Duration::from_secs(10),
            max_call_depth: 48,
            max_expr_depth: 64,
            max_string: 1 << 20,
            max_array: 20_000,
            max_map: 2_000,
            max_image_bytes: 512 << 20,
            max_file_bytes: 64 << 20,
            max_http_bytes: 16 << 20,
            max_source_bytes: 512 << 10,
        }
    }
}

/// What the host provides to scripts. Everything here is only reachable after a permission check.
pub trait Env: Send + Sync {
    /// Must NOT follow redirects (a redirect could leave the allowed domains).
    fn http(&self, req: &HttpRequest) -> Result<HttpResponse, String>;
    fn clipboard_text(&self) -> Result<String, String>;
    fn set_clipboard_text(&self, text: &str) -> Result<(), String>;
    fn set_clipboard_image(&self, img: &RgbaImage) -> Result<(), String>;
    fn ocr(&self, img: &RgbaImage) -> Result<String, String>;
    fn notify(&self, text: &str);
    fn show_text(&self, title: &str, text: &str);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Print,
    Log,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleLine {
    pub level: Level,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PluginError {
    #[error("cannot load the plugin: {0}")]
    Load(String),
    #[error("limit exceeded: {0}")]
    Limit(String),
    #[error("{0}")]
    Permission(String),
    #[error("script error: {0}")]
    Script(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Registrations {
    /// `(id, title)`: shown as checkboxes in the after-capture panel; run as `action_<id>(img)`.
    pub actions: Vec<(String, String)>,
    /// Menu items; run as `menu_<id>()`.
    pub menus: Vec<(String, String)>,
    /// Editor tools; a click runs `tool_<id>(img, x, y)`.
    pub tools: Vec<(String, String)>,
}

pub struct RunOutcome {
    pub image: Option<RgbaImage>,
    pub console: Vec<ConsoleLine>,
    pub error: Option<PluginError>,
    pub elapsed: Duration,
}

struct Ctx {
    perms: Permissions,
    env: Arc<dyn Env>,
    limits: Limits,
    image_bytes: AtomicU64,
    console: Mutex<Vec<ConsoleLine>>,
    regs: Mutex<Registrations>,
}

#[derive(Clone)]
pub struct Img(Arc<Mutex<RgbaImage>>);

impl Img {
    fn new(i: RgbaImage) -> Img {
        Img(Arc::new(Mutex::new(i)))
    }

    fn get(&self) -> RgbaImage {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

type R<T> = Result<T, Box<EvalAltResult>>;

fn fail<T>(msg: impl Into<String>) -> R<T> {
    Err(EvalAltResult::ErrorRuntime(Dynamic::from(msg.into()), Position::NONE).into())
}

fn denied<T>(what: &str, detail: &str) -> R<T> {
    fail(format!("permission denied: {what} — {detail}"))
}

impl Ctx {
    fn charge_image(&self, w: u32, h: u32) -> R<()> {
        if w == 0 || h == 0 || w > 16384 || h > 16384 {
            return fail(format!("an image of {w}x{h} pixels is not allowed (1..16384 per side)"));
        }
        let bytes = w as u64 * h as u64 * 4;
        let used = self.image_bytes.fetch_add(bytes, Ordering::Relaxed) + bytes;
        if used > self.limits.max_image_bytes {
            return Err(EvalAltResult::ErrorDataTooLarge("image memory".into(), Position::NONE).into());
        }
        Ok(())
    }

    fn line(&self, level: Level, text: String) {
        let mut c = self.console.lock().unwrap_or_else(|e| e.into_inner());
        if c.len() < 2000 {
            c.push(ConsoleLine {
                level,
                text: text.chars().take(4000).collect(),
            });
        }
    }
}

fn int(d: &Dynamic) -> R<i64> {
    d.as_int()
        .or_else(|_| d.as_float().map(|f| f as i64))
        .or_else(|e| fail(format!("expected a number, got {e}")))
}

fn num(d: &Dynamic) -> R<f64> {
    d.as_float()
        .or_else(|_| d.as_int().map(|i| i as f64))
        .or_else(|e| fail(format!("expected a number, got {e}")))
}

fn color(a: &Array) -> R<[u8; 4]> {
    if a.len() != 3 && a.len() != 4 {
        return fail("a colour is an array [r, g, b] or [r, g, b, a]");
    }
    let mut c = [0, 0, 0, 255u8];
    for (i, v) in a.iter().enumerate() {
        let n = int(v)?;
        if !(0..=255).contains(&n) {
            return fail("colour components are 0..255");
        }
        c[i] = n as u8;
    }
    Ok(c)
}

fn rect_in(img: &RgbaImage, x: i64, y: i64, w: i64, h: i64) -> R<Rect> {
    if w <= 0 || h <= 0 {
        return fail("the width and height of an area must be positive");
    }
    let r = Rect::new(
        x.clamp(-1_000_000, 1_000_000) as i32,
        y.clamp(-1_000_000, 1_000_000) as i32,
        w.min(1_000_000) as i32,
        h.min(1_000_000) as i32,
    );
    match r.intersect(&Rect::new(0, 0, img.width() as i32, img.height() as i32)) {
        Some(r) => Ok(r),
        None => fail("the area is outside of the image"),
    }
}

fn host_of(url: &str) -> R<String> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"));
    let Some(rest) = rest else {
        return fail("only http:// and https:// URLs are supported");
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return fail("URLs with a user name or password are not allowed");
    }
    let host = authority.rsplit_once(':').map_or(authority, |(h, p)| {
        if p.chars().all(|c| c.is_ascii_digit()) {
            h
        } else {
            authority
        }
    });
    if host.is_empty() {
        return fail("the URL has no host");
    }
    Ok(host.to_lowercase())
}

fn map_to_headers(m: &Map) -> Vec<(String, String)> {
    m.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn response_map(r: HttpResponse, max: usize) -> Map {
    let mut m = Map::new();
    m.insert("status".into(), Dynamic::from(r.status as i64));
    let body: String = String::from_utf8_lossy(&r.body[..r.body.len().min(max)]).into_owned();
    m.insert("body".into(), Dynamic::from(body));
    m
}

fn finish_draw(img: &Img, shape: Shape, c: [u8; 4], width: f64) {
    let mut g = img.0.lock().unwrap_or_else(|e| e.into_inner());
    let mut d = Document::new(g.clone());
    d.add(shape, c, width as f32);
    *g = d.render();
}

fn engine_for(ctx: &Arc<Ctx>, deadline: Instant) -> Engine {
    let l = &ctx.limits;
    let mut e = Engine::new();
    e.set_max_operations(l.max_ops);
    e.set_max_call_levels(l.max_call_depth);
    e.set_max_expr_depths(l.max_expr_depth, l.max_expr_depth);
    e.set_max_string_size(l.max_string);
    e.set_max_array_size(l.max_array);
    e.set_max_map_size(l.max_map);
    e.disable_symbol("eval");
    e.set_module_resolver(rhai::module_resolvers::DummyModuleResolver::new());
    let tick = std::sync::atomic::AtomicU32::new(0);
    e.on_progress(move |_| {
        let n = tick.fetch_add(1, Ordering::Relaxed);
        if n.is_multiple_of(256) && Instant::now() > deadline {
            Some(Dynamic::from("time"))
        } else {
            None
        }
    });
    let c = ctx.clone();
    e.on_print(move |s| c.line(Level::Print, s.to_owned()));
    let c = ctx.clone();
    e.on_debug(move |s, _, _| c.line(Level::Log, s.to_owned()));

    register_logging(&mut e, ctx);
    register_images(&mut e, ctx);
    register_fs(&mut e, ctx);
    register_net(&mut e, ctx);
    register_host(&mut e, ctx);
    e
}

fn register_logging(e: &mut Engine, ctx: &Arc<Ctx>) {
    let c = ctx.clone();
    e.register_fn("log", move |s: &str| c.line(Level::Log, s.to_owned()));
    let regs = |kind: u8, c: Arc<Ctx>| {
        move |id: &str, title: &str| -> R<()> {
            if id.is_empty() || !id.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
                return fail("an id is letters, digits and underscores");
            }
            let mut r = c.regs.lock().unwrap_or_else(|e| e.into_inner());
            let list = match kind {
                0 => &mut r.actions,
                1 => &mut r.menus,
                _ => &mut r.tools,
            };
            if !list.iter().any(|(i, _)| i == id) && list.len() < 32 {
                list.push((id.to_owned(), title.chars().take(80).collect()));
            }
            Ok(())
        }
    };
    e.register_fn("register_action", regs(0, ctx.clone()));
    e.register_fn("register_menu", regs(1, ctx.clone()));
    e.register_fn("register_tool", regs(2, ctx.clone()));
    e.register_fn("json_get", |body: &str, path: &str| -> Dynamic {
        upload::json_path(body, path).map_or(Dynamic::UNIT, Dynamic::from)
    });
    e.register_fn(
        "distance",
        |x1: Dynamic, y1: Dynamic, x2: Dynamic, y2: Dynamic| -> R<f64> {
            let (x1, y1, x2, y2) = (num(&x1)?, num(&y1)?, num(&x2)?, num(&y2)?);
            Ok(((x2 - x1).powi(2) + (y2 - y1).powi(2)).sqrt())
        },
    );
}

fn register_images(e: &mut Engine, ctx: &Arc<Ctx>) {
    e.register_type_with_name::<Img>("Image");
    let c = ctx.clone();
    e.register_fn("new_image", move |w: i64, h: i64, col: Array| -> R<Img> {
        if !(1..=16384).contains(&w) || !(1..=16384).contains(&h) {
            return fail("an image side must be 1..16384 pixels");
        }
        c.charge_image(w as u32, h as u32)?;
        let col = color(&col)?;
        Ok(Img::new(RgbaImage::from_pixel(w as u32, h as u32, image::Rgba(col))))
    });
    e.register_fn("width", |i: &mut Img| i.0.lock().map(|g| g.width() as i64).unwrap_or(0));
    e.register_fn("height", |i: &mut Img| {
        i.0.lock().map(|g| g.height() as i64).unwrap_or(0)
    });
    e.register_fn("get_pixel", |i: &mut Img, x: i64, y: i64| -> R<Array> {
        let g = i.0.lock().unwrap_or_else(|e| e.into_inner());
        if x < 0 || y < 0 || x >= g.width() as i64 || y >= g.height() as i64 {
            return fail("the pixel is outside of the image");
        }
        Ok(g.get_pixel(x as u32, y as u32)
            .0
            .iter()
            .map(|v| Dynamic::from(*v as i64))
            .collect())
    });
    e.register_fn("set_pixel", |i: &mut Img, x: i64, y: i64, col: Array| -> R<()> {
        let col = color(&col)?;
        let mut g = i.0.lock().unwrap_or_else(|e| e.into_inner());
        if x < 0 || y < 0 || x >= g.width() as i64 || y >= g.height() as i64 {
            return fail("the pixel is outside of the image");
        }
        g.put_pixel(x as u32, y as u32, image::Rgba(col));
        Ok(())
    });
    e.register_fn(
        "fill_rect",
        |i: &mut Img, x: i64, y: i64, w: i64, h: i64, col: Array| -> R<()> {
            let col = color(&col)?;
            let r = rect_in(&i.get(), x, y, w, h)?;
            finish_draw(
                i,
                Shape::Rect {
                    a: Pt::new(r.x as f32, r.y as f32),
                    b: Pt::new(r.right() as f32, r.bottom() as f32),
                    fill: true,
                },
                col,
                1.0,
            );
            Ok(())
        },
    );
    e.register_fn(
        "draw_rect",
        |i: &mut Img, x: i64, y: i64, w: i64, h: i64, col: Array, width: Dynamic| -> R<()> {
            let (col, width) = (color(&col)?, num(&width)?);
            if w <= 0 || h <= 0 || !(0.5..=200.0).contains(&width) {
                return fail("bad rectangle or line width");
            }
            finish_draw(
                i,
                Shape::Rect {
                    a: Pt::new(x as f32, y as f32),
                    b: Pt::new((x + w) as f32, (y + h) as f32),
                    fill: false,
                },
                col,
                width,
            );
            Ok(())
        },
    );
    e.register_fn(
        "draw_line",
        |i: &mut Img, x1: i64, y1: i64, x2: i64, y2: i64, col: Array, width: Dynamic| -> R<()> {
            let (col, width) = (color(&col)?, num(&width)?);
            if !(0.5..=200.0).contains(&width) {
                return fail("bad line width");
            }
            finish_draw(
                i,
                Shape::Line {
                    a: Pt::new(x1 as f32, y1 as f32),
                    b: Pt::new(x2 as f32, y2 as f32),
                },
                col,
                width,
            );
            Ok(())
        },
    );
    e.register_fn(
        "draw_text",
        |i: &mut Img, x: i64, y: i64, text: &str, size: Dynamic, col: Array| -> R<()> {
            let (col, size) = (color(&col)?, num(&size)?);
            if !(4.0..=500.0).contains(&size) || text.chars().count() > 2000 {
                return fail("bad text size or length");
            }
            finish_draw(
                i,
                Shape::Text {
                    at: Pt::new(x as f32, y as f32),
                    text: text.to_owned(),
                    size: size as f32,
                },
                col,
                1.0,
            );
            Ok(())
        },
    );
    e.register_fn("measure_text", |text: &str, size: Dynamic| -> R<Array> {
        let size = num(&size)?;
        if !(4.0..=500.0).contains(&size) {
            return fail("bad text size");
        }
        let (w, h) = measure_text(text, size as f32);
        Ok(vec![Dynamic::from(w.ceil() as i64), Dynamic::from(h.ceil() as i64)])
    });
    e.register_fn(
        "blur",
        |i: &mut Img, x: i64, y: i64, w: i64, h: i64, radius: i64| -> R<()> {
            let r = rect_in(&i.get(), x, y, w, h)?;
            let mut g = i.0.lock().unwrap_or_else(|e| e.into_inner());
            let mut pm = tiny_skia::Pixmap::new(g.width(), g.height()).ok_or("bad size")?;
            pm.data_mut().copy_from_slice(g.as_raw());
            blur_region(
                &mut pm,
                Pt::new(r.x as f32, r.y as f32),
                Pt::new(r.right() as f32, r.bottom() as f32),
                radius.clamp(1, 200) as u32,
            );
            g.as_mut().copy_from_slice(pm.data());
            Ok(())
        },
    );
    e.register_fn(
        "pixelate",
        |i: &mut Img, x: i64, y: i64, w: i64, h: i64, block: i64| -> R<()> {
            let r = rect_in(&i.get(), x, y, w, h)?;
            let mut g = i.0.lock().unwrap_or_else(|e| e.into_inner());
            let mut pm = tiny_skia::Pixmap::new(g.width(), g.height()).ok_or("bad size")?;
            pm.data_mut().copy_from_slice(g.as_raw());
            pixelate_region(
                &mut pm,
                Pt::new(r.x as f32, r.y as f32),
                Pt::new(r.right() as f32, r.bottom() as f32),
                block.clamp(2, 500) as u32,
            );
            g.as_mut().copy_from_slice(pm.data());
            Ok(())
        },
    );
    let c = ctx.clone();
    e.register_fn("crop", move |i: &mut Img, x: i64, y: i64, w: i64, h: i64| -> R<Img> {
        let src = i.get();
        let r = rect_in(&src, x, y, w, h)?;
        c.charge_image(r.w as u32, r.h as u32)?;
        Ok(Img::new(
            image::imageops::crop_imm(&src, r.x as u32, r.y as u32, r.w as u32, r.h as u32).to_image(),
        ))
    });
    let c = ctx.clone();
    e.register_fn("resize", move |i: &mut Img, w: i64, h: i64| -> R<Img> {
        if !(1..=16384).contains(&w) || !(1..=16384).contains(&h) {
            return fail("an image side must be 1..16384 pixels");
        }
        c.charge_image(w as u32, h as u32)?;
        Ok(Img::new(image::imageops::resize(
            &i.get(),
            w as u32,
            h as u32,
            image::imageops::FilterType::Lanczos3,
        )))
    });
    let c = ctx.clone();
    e.register_fn("clone", move |i: &mut Img| -> R<Img> {
        let src = i.get();
        c.charge_image(src.width(), src.height())?;
        Ok(Img::new(src))
    });
    e.register_fn(
        "overlay",
        |i: &mut Img, other: Img, x: i64, y: i64, opacity: Dynamic| -> R<()> {
            let top = other.get();
            let mut g = i.0.lock().unwrap_or_else(|e| e.into_inner());
            overlay(&mut g, &top, x, y, num(&opacity)?.clamp(0.0, 1.0) as f32);
            Ok(())
        },
    );
    e.register_fn(
        "average_color",
        |i: &mut Img, x: i64, y: i64, w: i64, h: i64| -> R<Array> {
            let g = i.get();
            let r = rect_in(&g, x, y, w, h)?;
            let (mut sum, mut n) = ([0u64; 4], 0u64);
            for yy in r.y..r.bottom() {
                for xx in r.x..r.right() {
                    for (s, v) in sum.iter_mut().zip(g.get_pixel(xx as u32, yy as u32).0) {
                        *s += v as u64;
                    }
                    n += 1;
                }
            }
            Ok(sum.iter().map(|s| Dynamic::from((*s / n.max(1)) as i64)).collect())
        },
    );
    let c = ctx.clone();
    e.register_fn("ocr", move |i: &mut Img| -> R<ImmutableString> {
        c.env
            .ocr(&i.get())
            .map(Into::into)
            .or_else(|er| fail(format!("OCR failed: {er}")))
    });
}

fn read_limited(path: &Path, max: u64) -> R<Vec<u8>> {
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len > max {
        return fail(format!("the file is larger than the {} MB limit", max >> 20));
    }
    std::fs::read(path).or_else(|e| fail(e.to_string()))
}

fn register_fs(e: &mut Engine, ctx: &Arc<Ctx>) {
    let read_path = |c: &Arc<Ctx>, p: &str| -> R<PathBuf> {
        c.perms.check_read(Path::new(p)).map_or_else(
            || denied("fs.read", &format!("'{p}' is not inside a folder the plugin may read")),
            Ok,
        )
    };
    let write_path = |c: &Arc<Ctx>, p: &str| -> R<PathBuf> {
        c.perms.check_write(Path::new(p)).map_or_else(
            || {
                denied(
                    "fs.write",
                    &format!("'{p}' is not inside a folder the plugin may write"),
                )
            },
            Ok,
        )
    };
    let c = ctx.clone();
    e.register_fn("load_image", move |p: &str| -> R<Img> {
        let path = read_path(&c, p)?;
        let bytes = read_limited(&path, c.limits.max_file_bytes)?;
        let img = image::load_from_memory(&bytes)
            .or_else(|er| fail(format!("not a readable image: {er}")))?
            .to_rgba8();
        c.charge_image(img.width(), img.height())?;
        Ok(Img::new(img))
    });
    let c = ctx.clone();
    e.register_fn("save_image", move |i: &mut Img, p: &str| -> R<()> {
        let path = write_path(&c, p)?;
        let img = i.get();
        let bytes = crate::output::encode(&img, crate::output::format_for_path(&path), 90)
            .or_else(|er| fail(er.to_string()))?;
        if bytes.len() as u64 > c.limits.max_file_bytes {
            return fail("the encoded image is larger than the file size limit");
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).or_else(|er| fail(er.to_string()))?;
        }
        std::fs::write(&path, bytes).or_else(|er| fail(er.to_string()))
    });
    let c = ctx.clone();
    e.register_fn("read_text", move |p: &str| -> R<ImmutableString> {
        let path = read_path(&c, p)?;
        let bytes = read_limited(&path, c.limits.max_file_bytes.min(c.limits.max_string as u64))?;
        Ok(String::from_utf8_lossy(&bytes).into_owned().into())
    });
    let c = ctx.clone();
    e.register_fn("write_text", move |p: &str, text: &str| -> R<()> {
        let path = write_path(&c, p)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).or_else(|er| fail(er.to_string()))?;
        }
        std::fs::write(&path, text).or_else(|er| fail(er.to_string()))
    });
    let c = ctx.clone();
    e.register_fn("file_exists", move |p: &str| -> R<bool> {
        Ok(read_path(&c, p)?.exists())
    });
    let c = ctx.clone();
    e.register_fn("list_dir", move |p: &str| -> R<Array> {
        let path = read_path(&c, p)?;
        let rd = std::fs::read_dir(&path).or_else(|er| fail(er.to_string()))?;
        Ok(rd
            .flatten()
            .take(c.limits.max_array)
            .map(|e| Dynamic::from(e.file_name().to_string_lossy().into_owned()))
            .collect())
    });
}

fn register_net(e: &mut Engine, ctx: &Arc<Ctx>) {
    fn send(c: &Arc<Ctx>, method: &str, url: &str, headers: Vec<(String, String)>, body: Vec<u8>) -> R<Map> {
        let host = host_of(url)?;
        if !c.perms.allows_host(&host) {
            return denied("net", &format!("'{host}' is not in the plugin's list of allowed hosts"));
        }
        let req = HttpRequest {
            method: method.into(),
            url: url.to_owned(),
            headers,
            body,
        };
        let resp = c
            .env
            .http(&req)
            .or_else(|er| fail(format!("request failed: {}", crate::diag::redact(&er))))?;
        Ok(response_map(resp, c.limits.max_http_bytes))
    }
    let c = ctx.clone();
    e.register_fn("http_get", move |url: &str| -> R<Map> {
        send(&c, "GET", url, Vec::new(), Vec::new())
    });
    let c = ctx.clone();
    e.register_fn("http_get", move |url: &str, headers: Map| -> R<Map> {
        send(&c, "GET", url, map_to_headers(&headers), Vec::new())
    });
    let c = ctx.clone();
    e.register_fn("http_post", move |url: &str, body: &str, headers: Map| -> R<Map> {
        send(&c, "POST", url, map_to_headers(&headers), body.as_bytes().to_vec())
    });
    let c = ctx.clone();
    e.register_fn(
        "http_upload",
        move |url: &str, field: &str, filename: &str, i: Img, headers: Map| -> R<Map> {
            let png = crate::history::encode_png_fast(&i.get()).or_else(|er| fail(er.to_string()))?;
            if png.len() > c.limits.max_http_bytes {
                return fail("the image is larger than the upload size limit");
            }
            let boundary = format!(
                "----SnapForgePlugin{:x}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            );
            let part = FilePart {
                name: filename.to_owned(),
                mime: "image/png".into(),
                bytes: png,
            };
            let body = upload::multipart(&boundary, &[], field, &part);
            let mut h = map_to_headers(&headers);
            h.push((
                "Content-Type".into(),
                format!("multipart/form-data; boundary={boundary}"),
            ));
            send(&c, "POST", url, h, body)
        },
    );
}

fn register_host(e: &mut Engine, ctx: &Arc<Ctx>) {
    let c = ctx.clone();
    e.register_fn("clipboard_get", move || -> R<ImmutableString> {
        if !c.perms.clipboard {
            return denied("clipboard", "the plugin did not ask for clipboard access");
        }
        c.env.clipboard_text().map(Into::into).or_else(fail)
    });
    let c = ctx.clone();
    e.register_fn("clipboard_set", move |t: &str| -> R<()> {
        if !c.perms.clipboard {
            return denied("clipboard", "the plugin did not ask for clipboard access");
        }
        c.env.set_clipboard_text(t).or_else(fail)
    });
    let c = ctx.clone();
    e.register_fn("clipboard_set_image", move |i: &mut Img| -> R<()> {
        if !c.perms.clipboard {
            return denied("clipboard", "the plugin did not ask for clipboard access");
        }
        c.env.set_clipboard_image(&i.get()).or_else(fail)
    });
    let c = ctx.clone();
    e.register_fn("notify", move |t: &str| -> R<()> {
        if !c.perms.ui {
            return denied("ui", "the plugin did not ask for the ui permission");
        }
        c.env.notify(&t.chars().take(300).collect::<String>());
        Ok(())
    });
    let c = ctx.clone();
    e.register_fn("show_text", move |title: &str, t: &str| -> R<()> {
        if !c.perms.ui {
            return denied("ui", "the plugin did not ask for the ui permission");
        }
        c.env.show_text(
            &title.chars().take(80).collect::<String>(),
            &t.chars().take(20_000).collect::<String>(),
        );
        Ok(())
    });
}

fn classify(e: &EvalAltResult) -> PluginError {
    match e {
        EvalAltResult::ErrorTooManyOperations(..) => {
            PluginError::Limit("too many operations (possible endless loop)".into())
        }
        EvalAltResult::ErrorTerminated(..) => PluginError::Limit("time limit exceeded".into()),
        EvalAltResult::ErrorDataTooLarge(what, _) => PluginError::Limit(format!("{what} is too large")),
        EvalAltResult::ErrorStackOverflow(..) => PluginError::Limit("recursion is too deep".into()),
        EvalAltResult::ErrorRuntime(d, _) => {
            let s = d.to_string();
            if s.starts_with("permission denied") {
                PluginError::Permission(s)
            } else {
                PluginError::Script(s)
            }
        }
        EvalAltResult::ErrorInFunctionCall(_, _, inner, _) => classify(inner),
        other => PluginError::Script(other.to_string()),
    }
}

pub struct Plugin {
    pub id: String,
    pub manifest: Manifest,
    pub dir: PathBuf,
    pub perms: Permissions,
    pub regs: Registrations,
    ast: AST,
    env: Arc<dyn Env>,
    limits: Limits,
}

impl Plugin {
    /// Reads `plugin.toml` and the main script from `dir`, compiles it and runs its top level (where scripts call
    /// `register_action(…)` etc.). `perms` is what the user approved.
    pub fn load(dir: &Path, perms: Permissions, env: Arc<dyn Env>, limits: Limits) -> Result<Plugin, PluginError> {
        let manifest_text = std::fs::read_to_string(dir.join("plugin.toml"))
            .map_err(|e| PluginError::Load(format!("plugin.toml: {e}")))?;
        let manifest = Manifest::parse(&manifest_text).map_err(|e| PluginError::Load(e.to_string()))?;
        let src_path = dir.join(&manifest.main);
        let len = std::fs::metadata(&src_path)
            .map_err(|e| PluginError::Load(format!("{}: {e}", manifest.main)))?
            .len();
        if len as usize > limits.max_source_bytes {
            return Err(PluginError::Load(format!(
                "{} is larger than {} KB",
                manifest.main,
                limits.max_source_bytes >> 10
            )));
        }
        let source =
            std::fs::read_to_string(&src_path).map_err(|e| PluginError::Load(format!("{}: {e}", manifest.main)))?;
        let id = manifest.id();
        let ctx = Arc::new(Ctx {
            perms: perms.clone(),
            env: env.clone(),
            limits: limits.clone(),
            image_bytes: AtomicU64::new(0),
            console: Mutex::default(),
            regs: Mutex::default(),
        });
        let engine = engine_for(&ctx, Instant::now() + limits.max_time);
        let ast = engine.compile(&source).map_err(|e| PluginError::Load(e.to_string()))?;
        engine
            .run_ast_with_scope(&mut Scope::new(), &ast)
            .map_err(|e| classify(&e))?;
        let regs = ctx.regs.lock().unwrap_or_else(|e| e.into_inner()).clone();
        Ok(Plugin {
            id,
            manifest,
            dir: dir.to_path_buf(),
            perms,
            regs,
            ast,
            env,
            limits,
        })
    }

    /// Names of all functions the script defines (for the "run on a test picture" list).
    pub fn functions(&self) -> Vec<String> {
        let mut v: Vec<String> = self.ast.iter_functions().map(|f| f.name.to_string()).collect();
        v.sort();
        v.dedup();
        v
    }

    pub fn has_fn(&self, name: &str) -> bool {
        self.ast.iter_functions().any(|f| f.name == name)
    }

    /// Calls a function of the script with `image` (and extra arguments) and returns the image it returns, if any.
    pub fn call_image(&self, name: &str, image: Option<&RgbaImage>, extra: Vec<Dynamic>) -> RunOutcome {
        let started = Instant::now();
        let ctx = Arc::new(Ctx {
            perms: self.perms.clone(),
            env: self.env.clone(),
            limits: self.limits.clone(),
            image_bytes: AtomicU64::new(0),
            console: Mutex::default(),
            regs: Mutex::default(),
        });
        let engine = engine_for(&ctx, started + self.limits.max_time);
        let mut args: Vec<Dynamic> = Vec::new();
        let mut handle = None;
        if let Some(i) = image {
            if let Err(e) = ctx.charge_image(i.width(), i.height()) {
                return RunOutcome {
                    image: None,
                    console: Vec::new(),
                    error: Some(classify(&e)),
                    elapsed: started.elapsed(),
                };
            }
            let h = Img::new(i.clone());
            handle = Some(h.clone());
            args.push(Dynamic::from(h));
        }
        args.extend(extra);
        let res = engine.call_fn::<Dynamic>(&mut Scope::new(), &self.ast, name, args);
        let mut out = RunOutcome {
            image: None,
            console: Vec::new(),
            error: None,
            elapsed: started.elapsed(),
        };
        match res {
            Ok(v) => {
                if v.is::<Img>() {
                    out.image = v.try_cast::<Img>().map(|h| h.get());
                } else if v.is_unit() {
                    // a script that edited its argument in place and returned nothing still changed the picture
                    out.image = handle.map(|h| h.get()).filter(|g| image.is_some_and(|orig| orig != g));
                } else {
                    ctx.line(Level::Log, format!("returned {v}"));
                }
            }
            Err(e) => out.error = Some(classify(&e)),
        }
        out.console = std::mem::take(&mut *ctx.console.lock().unwrap_or_else(|e| e.into_inner()));
        if let Some(e) = &out.error {
            out.console.push(ConsoleLine {
                level: Level::Error,
                text: e.to_string(),
            });
        }
        out.elapsed = started.elapsed();
        out
    }

    /// Calls `name` with the arguments its kind expects: hooks and actions get the picture, `on_before_save` also
    /// an info map, `tool_*` also the click position, `menu_*` and `on_after_save` take no picture.
    /// Used by the test run in the editor and by `snapforge-cli plugin run`.
    pub fn call_named(&self, name: &str, img: &RgbaImage, at: (i64, i64)) -> RunOutcome {
        let mut info = Map::new();
        info.insert("app".into(), Dynamic::from("test".to_owned()));
        info.insert("format".into(), Dynamic::from("png".to_owned()));
        info.insert("width".into(), Dynamic::from(img.width() as i64));
        info.insert("height".into(), Dynamic::from(img.height() as i64));
        if name.starts_with("menu_") {
            self.call_plain(name, Vec::new())
        } else if name == "on_after_save" {
            let path = "C:\\test\\shot.png".to_owned();
            info.insert("path".into(), Dynamic::from(path.clone()));
            self.call_plain(name, vec![Dynamic::from(path), Dynamic::from(info)])
        } else if name == "on_before_save" {
            self.call_image(name, Some(img), vec![Dynamic::from(info)])
        } else if name.starts_with("tool_") {
            self.call_image(name, Some(img), vec![Dynamic::from(at.0), Dynamic::from(at.1)])
        } else {
            self.call_image(name, Some(img), Vec::new())
        }
    }

    /// Calls a function that takes only plain arguments (menu items, `on_after_save`).
    pub fn call_plain(&self, name: &str, args: Vec<Dynamic>) -> RunOutcome {
        self.call_image(name, None, args)
    }
}
