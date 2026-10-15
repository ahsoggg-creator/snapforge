//! Acceptance tests of the sandbox: what a plugin technically cannot do without a permission, and that runaway
//! scripts are stopped.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use image::{Rgba, RgbaImage};

use super::manifest::{Manifest, Permissions};
use super::runtime::{Env, Level, Limits, Plugin, PluginError, RunOutcome};
use super::store::TEMPLATES;
use crate::upload::{HttpRequest, HttpResponse};

#[derive(Default)]
struct MockEnv {
    http: Mutex<Vec<HttpRequest>>,
    clipboard: Mutex<String>,
    notes: Mutex<Vec<String>>,
}

impl Env for MockEnv {
    fn http(&self, req: &HttpRequest) -> Result<HttpResponse, String> {
        self.http.lock().unwrap().push(req.clone());
        Ok(HttpResponse {
            status: 200,
            body: b"https://files.example/abc".to_vec(),
        })
    }
    fn clipboard_text(&self) -> Result<String, String> {
        Ok(self.clipboard.lock().unwrap().clone())
    }
    fn set_clipboard_text(&self, t: &str) -> Result<(), String> {
        *self.clipboard.lock().unwrap() = t.to_owned();
        Ok(())
    }
    fn set_clipboard_image(&self, _: &RgbaImage) -> Result<(), String> {
        Ok(())
    }
    fn ocr(&self, _: &RgbaImage) -> Result<String, String> {
        Ok("recognised".into())
    }
    fn notify(&self, t: &str) {
        self.notes.lock().unwrap().push(t.to_owned());
    }
    fn show_text(&self, _: &str, t: &str) {
        self.notes.lock().unwrap().push(t.to_owned());
    }
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sf-sbx-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn white(w: u32, h: u32) -> RgbaImage {
    RgbaImage::from_pixel(w, h, Rgba([255, 255, 255, 255]))
}

fn load_with(script: &str, perms: Permissions, limits: Limits, env: Arc<MockEnv>, name: &str) -> (Plugin, PathBuf) {
    let dir = tmp(name);
    std::fs::write(dir.join("plugin.toml"), "name = \"Test\"\nversion = \"1.0\"\n").unwrap();
    std::fs::write(dir.join("main.rhai"), script).unwrap();
    (Plugin::load(&dir, perms, env, limits).expect("load"), dir)
}

fn quick() -> Limits {
    Limits {
        max_ops: 2_000_000,
        max_time: Duration::from_secs(5),
        ..Limits::default()
    }
}

fn run(script: &str, perms: Permissions, name: &str) -> (RunOutcome, Arc<MockEnv>) {
    let env = Arc::new(MockEnv::default());
    let (p, dir) = load_with(script, perms, quick(), env.clone(), name);
    let out = p.call_image("hook", Some(&white(40, 30)), vec![]);
    let _ = std::fs::remove_dir_all(dir);
    (out, env)
}

fn is_permission(e: &Option<PluginError>) -> bool {
    matches!(e, Some(PluginError::Permission(_)))
}

// ------------------------------------------------------------------------------------------------- network

#[test]
fn without_the_net_permission_nothing_is_ever_sent() {
    let (out, env) = run(
        r#"fn hook(img) { http_get("https://example.com/x"); img }"#,
        Permissions::default(),
        "net0",
    );
    assert!(is_permission(&out.error), "{:?}", out.error);
    assert!(env.http.lock().unwrap().is_empty(), "the transport was never called");
    let (out, env) = run(
        r#"fn hook(img) { http_post("https://example.com/x", "data", #{}); img }"#,
        Permissions::default(),
        "net1",
    );
    assert!(is_permission(&out.error) && env.http.lock().unwrap().is_empty());
    let (out, env) = run(
        r#"fn hook(img) { http_upload("https://example.com/x", "f", "a.png", img, #{}); img }"#,
        Permissions::default(),
        "net2",
    );
    assert!(is_permission(&out.error) && env.http.lock().unwrap().is_empty());
}

#[test]
fn net_is_limited_to_the_declared_hosts() {
    let perms = Permissions {
        net: vec!["example.com".into()],
        ..Default::default()
    };
    let (out, env) = run(
        r#"fn hook(img) { let r = http_get("https://api.example.com/v1?x=1"); log(r.body); img }"#,
        perms.clone(),
        "net3",
    );
    assert!(out.error.is_none(), "{:?}", out.error);
    assert_eq!(env.http.lock().unwrap().len(), 1);
    assert!(out.console.iter().any(|l| l.text.contains("files.example")));
    for url in [
        "https://evil.com/",
        "https://example.com.evil.net/",
        "https://evilexample.com/",
        "https://example.com@evil.com/",
        "https://evil.com/?u=example.com",
        "ftp://example.com/",
        "file:///C:/Windows/win.ini",
        "//example.com/",
    ] {
        let (out, env) = run(
            &format!(r#"fn hook(img) {{ http_get("{url}"); img }}"#),
            perms.clone(),
            "net4",
        );
        assert!(out.error.is_some(), "{url} must be refused");
        assert!(
            env.http.lock().unwrap().is_empty(),
            "{url} must not reach the transport"
        );
    }
}

// ------------------------------------------------------------------------------------------------- files

#[test]
fn without_fs_write_nothing_is_written() {
    let dir = tmp("fsw0-target");
    let target = dir.join("out.txt");
    let script = format!(
        r#"fn hook(img) {{ write_text("{}", "pwned"); img }}"#,
        target.display().to_string().replace('\\', "\\\\")
    );
    let (out, _) = run(&script, Permissions::default(), "fsw0");
    assert!(is_permission(&out.error), "{:?}", out.error);
    assert!(!target.exists());
    let png = dir.join("o.png");
    let script = format!(
        r#"fn hook(img) {{ save_image(img, "{}"); img }}"#,
        png.display().to_string().replace('\\', "\\\\")
    );
    let (out, _) = run(&script, Permissions::default(), "fsw1");
    assert!(is_permission(&out.error) && !png.exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn fs_write_works_only_inside_the_declared_folder() {
    let root = tmp("fsw2");
    let (allowed, other) = (root.join("allowed"), root.join("other"));
    std::fs::create_dir_all(&allowed).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    let perms = Permissions {
        fs_write: vec![allowed.clone()],
        ..Default::default()
    };
    let esc = |p: &Path| p.display().to_string().replace('\\', "\\\\");
    // inside: text and an image
    let script = format!(
        r#"fn hook(img) {{ write_text("{a}\\note.txt", "hi"); save_image(img, "{a}\\sub\\shot.png"); img }}"#,
        a = esc(&allowed)
    );
    let (out, _) = run(&script, perms.clone(), "fsw3");
    assert!(out.error.is_none(), "{:?}", out.error);
    assert_eq!(std::fs::read_to_string(allowed.join("note.txt")).unwrap(), "hi");
    assert!(allowed.join("sub").join("shot.png").exists());
    // outside, sibling-prefix and traversal
    for target in [
        other.join("x.txt"),
        root.join("allowed-evil.txt"),
        allowed.join("..").join("other").join("t.txt"),
    ] {
        let script = format!(r#"fn hook(img) {{ write_text("{}", "pwned"); img }}"#, esc(&target));
        let (out, _) = run(&script, perms.clone(), "fsw4");
        assert!(is_permission(&out.error), "{}: {:?}", target.display(), out.error);
    }
    assert!(!other.join("x.txt").exists() && !other.join("t.txt").exists() && !root.join("allowed-evil.txt").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn reading_files_needs_fs_read_and_stays_inside_the_folder() {
    let root = tmp("fsr");
    let (inside, outside) = (root.join("in"), root.join("secret"));
    std::fs::create_dir_all(&inside).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(inside.join("a.txt"), "public").unwrap();
    std::fs::write(outside.join("b.txt"), "private").unwrap();
    let esc = |p: &Path| p.display().to_string().replace('\\', "\\\\");
    let read = |p: &Path| format!(r#"fn hook(img) {{ log(read_text("{}")); img }}"#, esc(p));
    let perms = Permissions {
        fs_read: vec![inside.clone()],
        ..Default::default()
    };
    let (out, _) = run(&read(&inside.join("a.txt")), perms.clone(), "fsr0");
    assert!(out.error.is_none() && out.console.iter().any(|l| l.text == "public"));
    let (out, _) = run(&read(&outside.join("b.txt")), perms.clone(), "fsr1");
    assert!(is_permission(&out.error));
    assert!(!out.console.iter().any(|l| l.text.contains("private")));
    let (out, _) = run(&read(&inside.join("a.txt")), Permissions::default(), "fsr2");
    assert!(is_permission(&out.error), "no permission, no reading");
    std::fs::remove_dir_all(root).unwrap();
}

// ------------------------------------------------------------------------------------------------- limits

#[test]
fn an_endless_loop_is_stopped_by_the_operation_limit() {
    for (i, body) in [
        "loop {}",
        "while true {}",
        "let n = 0; loop { n += 1; }",
        "for i in 0..1000000000 { let x = i * 2; }",
    ]
    .iter()
    .enumerate()
    {
        let started = Instant::now();
        let (out, _) = run(
            &format!("fn hook(img) {{ {body}; img }}"),
            Permissions::default(),
            &format!("loop{i}"),
        );
        assert!(
            matches!(out.error, Some(PluginError::Limit(_))),
            "{body}: {:?}",
            out.error
        );
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{body} stopped promptly: {:?}",
            started.elapsed()
        );
    }
}

#[test]
fn the_time_limit_stops_slow_scripts_even_with_a_generous_operation_budget() {
    let env = Arc::new(MockEnv::default());
    let limits = Limits {
        max_ops: u64::MAX / 2,
        max_time: Duration::from_millis(300),
        ..Limits::default()
    };
    let (p, dir) = load_with(
        "fn hook(img) { let n = 0; loop { n += 1; } }",
        Permissions::default(),
        limits,
        env,
        "time",
    );
    let started = Instant::now();
    let out = p.call_image("hook", Some(&white(8, 8)), vec![]);
    assert!(
        matches!(out.error, Some(PluginError::Limit(ref m)) if m.contains("time")),
        "{:?}",
        out.error
    );
    assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn runaway_recursion_is_stopped() {
    let (out, _) = run(
        "fn boom(n) { boom(n + 1) }\nfn hook(img) { boom(0); img }",
        Permissions::default(),
        "rec",
    );
    assert!(matches!(out.error, Some(PluginError::Limit(_))), "{:?}", out.error);
}

#[test]
fn memory_hungry_scripts_are_stopped() {
    for (i, body) in [
        // string doubling
        r#"let s = "x"; loop { s += s; }"#,
        // an ever-growing array
        "let a = []; loop { a.push(1); }",
        // a huge map
        "let m = #{}; let i = 0; loop { m[i.to_string()] = i; i += 1; }",
        // too many big images
        "let k = []; loop { k.push(new_image(16000, 16000, [0, 0, 0])); }",
        // a single impossible image
        "let k = new_image(100000, 100000, [0, 0, 0]);",
    ]
    .iter()
    .enumerate()
    {
        let (out, _) = run(
            &format!("fn hook(img) {{ {body}; img }}"),
            Permissions::default(),
            &format!("mem{i}"),
        );
        assert!(out.error.is_some(), "{body} must fail");
        assert!(
            matches!(out.error, Some(PluginError::Limit(_)) | Some(PluginError::Script(_))),
            "{body}: {:?}",
            out.error
        );
    }
}

// ------------------------------------------------------------------------------------------------- escape hatches

#[test]
fn eval_import_and_unknown_host_functions_do_not_exist() {
    // These strings are hostile plugin scripts that the sandbox must REJECT; the host never evaluates anything itself.
    for body in [
        r#"eval("1 + 1")"#,
        r#"import "evil" as m;"#,
        r#"get_secret("token")"#,
        r#"read_secret("upload.token")"#,
        r#"env("PATH")"#,
        r#"exec("cmd.exe")"#,
        r#"system("calc")"#,
    ] {
        let env = Arc::new(MockEnv::default());
        let dir = tmp("esc");
        std::fs::write(dir.join("plugin.toml"), "name = \"Test\"\nversion = \"1.0\"\n").unwrap();
        std::fs::write(dir.join("main.rhai"), format!("fn hook(img) {{ {body}; img }}")).unwrap();
        // either the load fails or the call fails; it must never succeed
        let failed = match Plugin::load(&dir, Permissions::default(), env, quick()) {
            Err(_) => true,
            Ok(p) => p.call_image("hook", Some(&white(4, 4)), vec![]).error.is_some(),
        };
        assert!(failed, "{body} must not work");
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn plugins_cannot_touch_the_upload_tokens() {
    // The Credential Manager entries of the uploader are not reachable: a plugin has no API for them, and the
    // paths it may read are limited by its permissions (the credential store is not a file).
    let store = crate::secrets::MemoryStore::default();
    use crate::secrets::SecretStore;
    store.set("d1.token", "UPLOAD-TOKEN-VALUE-123").unwrap();
    let (out, env) = run(
        r#"fn hook(img) { log(clipboard_get()); log(read_text("C:\\Users")); log(json_get("x", "y")); img }"#,
        Permissions {
            clipboard: true,
            ..Default::default()
        },
        "tok",
    );
    assert!(out.console.iter().all(|l| !l.text.contains("UPLOAD-TOKEN-VALUE-123")));
    assert!(env.http.lock().unwrap().is_empty());
    crate::diag::clear_secrets();
}

// ------------------------------------------------------------------------------------------------- API

#[test]
fn clipboard_and_ui_need_their_permissions() {
    let (out, _) = run(
        r#"fn hook(img) { clipboard_set("x"); img }"#,
        Permissions::default(),
        "cb0",
    );
    assert!(is_permission(&out.error));
    let (out, env) = run(
        r#"fn hook(img) { clipboard_set("copied"); notify("done"); img }"#,
        Permissions {
            clipboard: true,
            ui: true,
            ..Default::default()
        },
        "cb1",
    );
    assert!(out.error.is_none(), "{:?}", out.error);
    assert_eq!(*env.clipboard.lock().unwrap(), "copied");
    assert_eq!(*env.notes.lock().unwrap(), vec!["done"]);
    let (out, env) = run(
        r#"fn hook(img) { notify("sneaky"); img }"#,
        Permissions {
            clipboard: true,
            ..Default::default()
        },
        "cb2",
    );
    assert!(
        is_permission(&out.error) && env.notes.lock().unwrap().is_empty(),
        "clipboard does not imply ui"
    );
}

#[test]
fn image_operations_change_pixels() {
    let script = r#"
        fn hook(img) {
            img.fill_rect(0, 0, 10, 10, [255, 0, 0]);
            img.pixelate(20, 0, 10, 10, 5);
            let c = img.crop(0, 0, 20, 20);
            let big = c.resize(40, 40);
            img.overlay(big, 20, 10, 0.5);
            let dim = measure_text("Hi", 20);
            log("dim " + dim[0] + "x" + dim[1]);
            log("avg " + img.average_color(0, 0, 10, 10)[0]);
            log("d " + distance(0.0, 0.0, 3.0, 4.0));
            img
        }"#;
    let (out, _) = run(script, Permissions::default(), "img");
    assert!(out.error.is_none(), "{:?}", out.error);
    let img = out.image.expect("the script returned an image");
    assert_eq!(img.get_pixel(2, 2).0, [255, 0, 0, 255]);
    assert_eq!(img.dimensions(), (40, 30));
    assert!(
        out.console.iter().any(|l| l.text == "avg 255")
            && out.console.iter().any(|l| l.text == "d 5.0" || l.text == "d 5")
    );
}

#[test]
fn editing_in_place_without_returning_still_counts() {
    let (out, _) = run(
        "fn hook(img) { img.set_pixel(0, 0, [1, 2, 3]); }",
        Permissions::default(),
        "inplace",
    );
    assert_eq!(out.image.unwrap().get_pixel(0, 0).0, [1, 2, 3, 255]);
    let (out, _) = run(
        "fn hook(img) { log(\"nothing changed\"); }",
        Permissions::default(),
        "noop",
    );
    assert!(out.image.is_none(), "an untouched picture is not reported as a result");
}

#[test]
fn registrations_and_console_are_collected() {
    let env = Arc::new(MockEnv::default());
    let script = r#"
        register_action("a1", "Action one");
        register_menu("m1", "Menu one");
        register_tool("t1", "Tool one");
        register_action("a1", "duplicate ignored");
        print("loaded");
        fn action_a1(img) { print("ran"); debug("dbg"); log("lg"); img }
    "#;
    let (p, dir) = load_with(script, Permissions::default(), quick(), env, "regs");
    assert_eq!(p.regs.actions, vec![("a1".to_owned(), "Action one".to_owned())]);
    assert_eq!(p.regs.menus.len(), 1);
    assert_eq!(p.regs.tools, vec![("t1".to_owned(), "Tool one".to_owned())]);
    assert!(p.has_fn("action_a1") && !p.has_fn("action_missing"));
    let out = p.call_image("action_a1", Some(&white(4, 4)), vec![]);
    let texts: Vec<_> = out.console.iter().map(|l| (l.level, l.text.as_str())).collect();
    assert!(texts.contains(&(Level::Print, "ran")) && texts.contains(&(Level::Log, "lg")));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn syntax_errors_are_reported_at_load_not_run() {
    let dir = tmp("syntax");
    std::fs::write(dir.join("plugin.toml"), "name = \"T\"\nversion = \"1.0\"\n").unwrap();
    std::fs::write(dir.join("main.rhai"), "fn broken( { ").unwrap();
    let r = Plugin::load(&dir, Permissions::default(), Arc::new(MockEnv::default()), quick());
    assert!(matches!(r, Err(PluginError::Load(_))));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_script_error_does_not_take_the_host_down() {
    let (out, _) = run("fn hook(img) { let x = 1 / 0; img }", Permissions::default(), "div");
    assert!(matches!(out.error, Some(PluginError::Script(_))));
    let (out, _) = run(
        "fn hook(img) { throw \"custom failure\"; }",
        Permissions::default(),
        "throw",
    );
    assert!(matches!(&out.error, Some(PluginError::Script(m)) if m.contains("custom failure")));
    assert!(out.console.iter().any(|l| l.level == Level::Error));
}

// ------------------------------------------------------------------------------------------------- shipped plugins

fn load_template(i: usize, env: Arc<MockEnv>) -> Plugin {
    let t = &TEMPLATES[i];
    let dir = tmp(&format!("tpl{i}"));
    std::fs::write(dir.join("plugin.toml"), t.manifest).unwrap();
    std::fs::write(dir.join("main.rhai"), t.script).unwrap();
    let perms = Manifest::parse(t.manifest).unwrap().permissions.resolve().unwrap();
    Plugin::load(&dir, perms, env, quick()).unwrap_or_else(|e| panic!("{}: {e}", t.id))
}

#[test]
fn the_hello_plugin_logs_captures_and_has_a_menu_item() {
    let env = Arc::new(MockEnv::default());
    let p = load_template(0, env.clone());
    assert_eq!(p.regs.menus[0].0, "hello");
    let out = p.call_image("on_capture", Some(&white(64, 48)), vec![]);
    assert!(
        out.error.is_none() && out.console.iter().any(|l| l.text.contains("64x48")),
        "{:?}",
        out.console
    );
    let out = p.call_plain("menu_hello", vec![]);
    assert!(out.error.is_none());
    assert_eq!(env.notes.lock().unwrap().len(), 1);
}

#[test]
fn the_watermark_plugin_marks_the_picture_and_needs_no_permissions() {
    let p = load_template(1, Arc::new(MockEnv::default()));
    assert!(p.perms.is_empty());
    let src = white(800, 400);
    let out = p.call_image(
        "on_before_save",
        Some(&src),
        vec![rhai::Dynamic::from(rhai::Map::new())],
    );
    assert!(out.error.is_none(), "{:?}", out.error);
    let marked = out.image.expect("a marked picture");
    if crate::editor::render::has_font() {
        assert_ne!(marked, src);
        assert_eq!(marked.get_pixel(3, 3), src.get_pixel(3, 3), "only the corner changes");
    }
    assert!(p.has_fn("action_mark"));
}

#[test]
fn the_custom_upload_plugin_posts_to_its_declared_host_only() {
    let env = Arc::new(MockEnv::default());
    let p = load_template(2, env.clone());
    let out = p.call_image("action_upload", Some(&white(32, 32)), vec![]);
    assert!(out.error.is_none(), "{:?}", out.error);
    let reqs = env.http.lock().unwrap();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].url, "https://upload.example.com/api/upload");
    assert!(
        reqs[0]
            .headers
            .iter()
            .any(|(k, v)| k == "Content-Type" && v.starts_with("multipart/form-data"))
    );
    assert!(String::from_utf8_lossy(&reqs[0].body).contains("filename=\"snapforge.png\""));
    assert_eq!(*env.clipboard.lock().unwrap(), "https://files.example/abc");
    // the same plugin with its permissions stripped cannot send anything
    drop(reqs);
    let env2 = Arc::new(MockEnv::default());
    let dir = tmp("tpl-stripped");
    std::fs::write(dir.join("plugin.toml"), TEMPLATES[2].manifest).unwrap();
    std::fs::write(dir.join("main.rhai"), TEMPLATES[2].script).unwrap();
    let p2 = Plugin::load(&dir, Permissions::default(), env2.clone(), quick()).unwrap();
    let out = p2.call_image("action_upload", Some(&white(32, 32)), vec![]);
    assert!(is_permission(&out.error));
    assert!(env2.http.lock().unwrap().is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}
