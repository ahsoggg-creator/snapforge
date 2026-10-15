//! Runs the enabled, approved plugins at the right moments (capture, before/after save, after-capture actions,
//! menu items, editor tools) and keeps one misbehaving plugin from affecting the others.

use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::Sender;

use image::RgbaImage;
use rhai::{Dynamic, Map};

use super::runtime::{ConsoleLine, Env, Level, Limits, Plugin};
use super::store::Store;
use crate::upload::{HttpRequest, HttpResponse};

/// What a plugin asked the app to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostEvent {
    Notify(String),
    ShowText { title: String, text: String },
}

/// The real environment: network without redirects, the system clipboard, Windows OCR.
pub struct SystemEnv {
    pub events: Sender<HostEvent>,
}

impl Env for SystemEnv {
    fn http(&self, r: &HttpRequest) -> Result<HttpResponse, String> {
        let mut b = ureq::http::Request::builder().method(r.method.as_str()).uri(&r.url);
        b = b.header("User-Agent", crate::net::user_agent());
        for (k, v) in &r.headers {
            b = b.header(k.as_str(), v.as_str());
        }
        let req = b.body(r.body.clone()).map_err(|e| e.to_string())?;
        // no redirects: a redirect could leave the hosts the plugin was allowed to talk to
        let mut resp = crate::net::agent_no_redirects(20).run(req).map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let body = resp.body_mut().read_to_vec().map_err(|e| e.to_string())?;
        Ok(HttpResponse { status, body })
    }

    fn clipboard_text(&self) -> Result<String, String> {
        crate::output::read_text_from_clipboard().map_err(|e| e.to_string())
    }

    fn set_clipboard_text(&self, text: &str) -> Result<(), String> {
        crate::output::copy_text_to_clipboard(text).map_err(|e| e.to_string())
    }

    fn set_clipboard_image(&self, img: &RgbaImage) -> Result<(), String> {
        crate::output::copy_to_clipboard(img).map_err(|e| e.to_string())
    }

    fn ocr(&self, img: &RgbaImage) -> Result<String, String> {
        crate::ocr::recognize(img, None)
            .map(|d| d.text)
            .map_err(|e| e.to_string())
    }

    fn notify(&self, text: &str) {
        let _ = self.events.send(HostEvent::Notify(text.to_owned()));
    }

    fn show_text(&self, title: &str, text: &str) {
        let _ = self.events.send(HostEvent::ShowText {
            title: title.to_owned(),
            text: text.to_owned(),
        });
    }
}

#[derive(Clone, Debug)]
pub struct PluginReport {
    pub plugin: String,
    pub hook: String,
    pub console: Vec<ConsoleLine>,
    pub error: Option<String>,
    pub changed_image: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub plugin_id: String,
    pub plugin_name: String,
    pub id: String,
    pub title: String,
}

pub struct PluginHost {
    plugins: Vec<Plugin>,
    /// `(plugin name, reason)` for plugins that are enabled but failed to load.
    pub load_errors: Vec<(String, String)>,
}

impl PluginHost {
    /// Loads every plugin that is enabled AND whose current permissions the user approved.
    pub fn load(store: &Store, env: Arc<dyn Env>, limits: &Limits) -> PluginHost {
        let (mut plugins, mut load_errors) = (Vec::new(), Vec::new());
        for p in store.list().into_iter().filter(|p| p.runnable()) {
            match Plugin::load(&p.dir, p.perms.clone(), env.clone(), limits.clone()) {
                Ok(pl) => plugins.push(pl),
                Err(e) => load_errors.push((p.manifest.name.clone(), e.to_string())),
            }
        }
        PluginHost { plugins, load_errors }
    }

    pub fn empty() -> PluginHost {
        PluginHost {
            plugins: Vec::new(),
            load_errors: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    fn report(p: &Plugin, hook: &str, out: &super::runtime::RunOutcome) -> PluginReport {
        PluginReport {
            plugin: p.manifest.name.clone(),
            hook: hook.to_owned(),
            console: out.console.clone(),
            error: out.error.as_ref().map(|e| e.to_string()),
            changed_image: out.image.is_some(),
        }
    }

    /// Passes `img` through the `hook` of every plugin that has it; a failing plugin leaves the picture as it was.
    fn chain(
        &self,
        hook: &str,
        mut img: RgbaImage,
        extra: impl Fn() -> Vec<Dynamic>,
    ) -> (RgbaImage, Vec<PluginReport>) {
        let mut reports = Vec::new();
        for p in self.plugins.iter().filter(|p| p.has_fn(hook)) {
            let out = p.call_image(hook, Some(&img), extra());
            reports.push(Self::report(p, hook, &out));
            if out.error.is_none()
                && let Some(new) = out.image
            {
                img = new;
            }
        }
        (img, reports)
    }

    pub fn on_capture(&self, img: RgbaImage) -> (RgbaImage, Vec<PluginReport>) {
        self.chain("on_capture", img, Vec::new)
    }

    /// `app` is the application name in the shot; `format` is "png" or "jpg".
    pub fn on_before_save(&self, img: RgbaImage, app: &str, format: &str) -> (RgbaImage, Vec<PluginReport>) {
        let (w, h) = (img.width() as i64, img.height() as i64);
        let (app, format) = (app.to_owned(), format.to_owned());
        self.chain("on_before_save", img, move || {
            let mut m = Map::new();
            m.insert("app".into(), Dynamic::from(app.clone()));
            m.insert("format".into(), Dynamic::from(format.clone()));
            m.insert("width".into(), Dynamic::from(w));
            m.insert("height".into(), Dynamic::from(h));
            vec![Dynamic::from(m)]
        })
    }

    pub fn on_after_save(&self, path: &Path) -> Vec<PluginReport> {
        let mut reports = Vec::new();
        for p in self.plugins.iter().filter(|p| p.has_fn("on_after_save")) {
            let mut info = Map::new();
            info.insert("path".into(), Dynamic::from(path.display().to_string()));
            let out = p.call_plain(
                "on_after_save",
                vec![Dynamic::from(path.display().to_string()), Dynamic::from(info)],
            );
            reports.push(Self::report(p, "on_after_save", &out));
        }
        reports
    }

    fn entries(&self, pick: impl Fn(&Plugin) -> &Vec<(String, String)>) -> Vec<Entry> {
        self.plugins
            .iter()
            .flat_map(|p| {
                pick(p).iter().map(|(id, title)| Entry {
                    plugin_id: p.id.clone(),
                    plugin_name: p.manifest.name.clone(),
                    id: id.clone(),
                    title: title.clone(),
                })
            })
            .collect()
    }

    pub fn actions(&self) -> Vec<Entry> {
        self.entries(|p| &p.regs.actions)
    }

    pub fn menus(&self) -> Vec<Entry> {
        self.entries(|p| &p.regs.menus)
    }

    pub fn tools(&self) -> Vec<Entry> {
        self.entries(|p| &p.regs.tools)
    }

    fn find(&self, plugin_id: &str) -> Option<&Plugin> {
        self.plugins.iter().find(|p| p.id == plugin_id)
    }

    /// Runs `action_<id>(img)`; returns the new picture when the action produced one.
    pub fn run_action(&self, plugin_id: &str, id: &str, img: &RgbaImage) -> (Option<RgbaImage>, PluginReport) {
        self.run_image_fn(plugin_id, &format!("action_{id}"), img, Vec::new())
    }

    /// Runs `tool_<id>(img, x, y)`.
    pub fn run_tool(
        &self,
        plugin_id: &str,
        id: &str,
        img: &RgbaImage,
        x: i64,
        y: i64,
    ) -> (Option<RgbaImage>, PluginReport) {
        self.run_image_fn(
            plugin_id,
            &format!("tool_{id}"),
            img,
            vec![Dynamic::from(x), Dynamic::from(y)],
        )
    }

    fn run_image_fn(
        &self,
        plugin_id: &str,
        func: &str,
        img: &RgbaImage,
        extra: Vec<Dynamic>,
    ) -> (Option<RgbaImage>, PluginReport) {
        match self.find(plugin_id) {
            Some(p) if p.has_fn(func) => {
                let out = p.call_image(func, Some(img), extra);
                let report = Self::report(p, func, &out);
                (if out.error.is_none() { out.image } else { None }, report)
            }
            _ => (
                None,
                PluginReport {
                    plugin: plugin_id.to_owned(),
                    hook: func.to_owned(),
                    console: Vec::new(),
                    error: Some(format!("the plugin has no function {func}")),
                    changed_image: false,
                },
            ),
        }
    }

    pub fn run_menu(&self, plugin_id: &str, id: &str) -> PluginReport {
        let func = format!("menu_{id}");
        match self.find(plugin_id) {
            Some(p) if p.has_fn(&func) => Self::report(p, &func, &p.call_plain(&func, Vec::new())),
            _ => PluginReport {
                plugin: plugin_id.to_owned(),
                hook: func.clone(),
                console: Vec::new(),
                error: Some(format!("the plugin has no function {func}")),
                changed_image: false,
            },
        }
    }
}

/// Console text of a report, one line per entry (for the plugin console in the UI).
pub fn console_text(r: &PluginReport) -> String {
    let mut s = String::new();
    for l in &r.console {
        let tag = match l.level {
            Level::Print => "",
            Level::Log => "[log] ",
            Level::Error => "[error] ",
        };
        s.push_str(&format!("{tag}{}\n", l.text));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::store::{Store, TEMPLATES};
    use image::Rgba;

    struct NullEnv;

    impl Env for NullEnv {
        fn http(&self, _: &HttpRequest) -> Result<HttpResponse, String> {
            Err("offline".into())
        }
        fn clipboard_text(&self) -> Result<String, String> {
            Ok(String::new())
        }
        fn set_clipboard_text(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn set_clipboard_image(&self, _: &RgbaImage) -> Result<(), String> {
            Ok(())
        }
        fn ocr(&self, _: &RgbaImage) -> Result<String, String> {
            Ok(String::new())
        }
        fn notify(&self, _: &str) {}
        fn show_text(&self, _: &str, _: &str) {}
    }

    fn store(name: &str) -> Store {
        let d = std::env::temp_dir().join(format!("sf-host-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        Store::new(d)
    }

    fn write_plugin(s: &Store, name: &str, script: &str, approved: bool) -> String {
        let t = crate::plugin::store::Template {
            id: "x",
            title: "x",
            manifest: Box::leak(format!("name = \"{name}\"\nversion = \"1.0\"\n").into_boxed_str()),
            script: Box::leak(script.to_owned().into_boxed_str()),
        };
        s.create(&t, approved).unwrap()
    }

    #[test]
    fn only_enabled_and_approved_plugins_are_loaded() {
        let s = store("loading");
        write_plugin(&s, "On", "fn on_capture(img) { img.set_pixel(0, 0, [1, 1, 1]); }", true);
        write_plugin(
            &s,
            "Unapproved",
            "fn on_capture(img) { img.set_pixel(1, 0, [2, 2, 2]); }",
            false,
        );
        let off = write_plugin(
            &s,
            "Off",
            "fn on_capture(img) { img.set_pixel(2, 0, [3, 3, 3]); }",
            true,
        );
        s.set_enabled(&off, false).unwrap();
        let host = PluginHost::load(&s, Arc::new(NullEnv), &Limits::default());
        assert_eq!(host.len(), 1);
        let (img, reports) = host.on_capture(RgbaImage::from_pixel(4, 4, Rgba([9, 9, 9, 255])));
        assert_eq!(reports.len(), 1);
        assert_eq!(img.get_pixel(0, 0).0, [1, 1, 1, 255]);
        assert_eq!(
            img.get_pixel(1, 0).0,
            [9, 9, 9, 255],
            "the unapproved plugin did nothing"
        );
        assert_eq!(img.get_pixel(2, 0).0, [9, 9, 9, 255], "the disabled plugin did nothing");
        std::fs::remove_dir_all(s.root()).unwrap();
    }

    #[test]
    fn a_failing_plugin_does_not_stop_the_chain() {
        let s = store("chain");
        write_plugin(
            &s,
            "A first",
            "fn on_capture(img) { img.set_pixel(0, 0, [10, 0, 0]); }",
            true,
        );
        write_plugin(
            &s,
            "B broken",
            "fn on_capture(img) { img.set_pixel(1, 0, [20, 0, 0]); throw \"oops\"; }",
            true,
        );
        write_plugin(&s, "C loop", "fn on_capture(img) { loop {} }", true);
        write_plugin(
            &s,
            "D last",
            "fn on_capture(img) { img.set_pixel(3, 0, [40, 0, 0]); }",
            true,
        );
        let host = PluginHost::load(
            &s,
            Arc::new(NullEnv),
            &Limits {
                max_ops: 200_000,
                ..Limits::default()
            },
        );
        let (img, reports) = host.on_capture(RgbaImage::from_pixel(4, 1, Rgba([0, 0, 0, 255])));
        assert_eq!(reports.len(), 4);
        assert_eq!(reports.iter().filter(|r| r.error.is_some()).count(), 2);
        assert_eq!(img.get_pixel(0, 0).0[0], 10);
        assert_eq!(
            img.get_pixel(1, 0).0[0],
            0,
            "the failed plugin's partial edit is discarded"
        );
        assert_eq!(img.get_pixel(3, 0).0[0], 40, "later plugins still ran");
        std::fs::remove_dir_all(s.root()).unwrap();
    }

    #[test]
    fn actions_menus_and_tools_are_listed_and_runnable() {
        let s = store("entries");
        let id = s.create(&TEMPLATES[1], true).unwrap();
        s.create(&TEMPLATES[0], true).unwrap();
        let host = PluginHost::load(&s, Arc::new(NullEnv), &Limits::default());
        assert_eq!(host.actions().len(), 1);
        assert_eq!(host.actions()[0].id, "mark");
        assert_eq!(host.menus()[0].plugin_name, "Hello");
        let src = RgbaImage::from_pixel(400, 200, Rgba([255, 255, 255, 255]));
        let (out, rep) = host.run_action(&id, "mark", &src);
        assert!(rep.error.is_none(), "{:?}", rep.error);
        if crate::editor::render::has_font() {
            assert!(out.is_some());
        }
        let (none, rep) = host.run_action(&id, "nope", &src);
        assert!(none.is_none() && rep.error.is_some());
        assert!(host.run_menu("hello", "hello").error.is_none());
        std::fs::remove_dir_all(s.root()).unwrap();
    }

    #[test]
    fn before_save_and_after_save_hooks_get_their_arguments() {
        let s = store("save");
        write_plugin(
            &s,
            "Saver",
            r#"
            fn on_before_save(img, info) { log("saving " + info.app + " " + info.format + " " + info.width); img.set_pixel(0, 0, [7, 7, 7]); }
            fn on_after_save(path, info) { log("saved " + path + "|" + info.path); }
            "#,
            true,
        );
        let host = PluginHost::load(&s, Arc::new(NullEnv), &Limits::default());
        let (img, reports) = host.on_before_save(RgbaImage::from_pixel(10, 5, Rgba([0, 0, 0, 255])), "chrome", "png");
        assert_eq!(img.get_pixel(0, 0).0[0], 7);
        assert!(
            reports[0].console.iter().any(|l| l.text == "saving chrome png 10"),
            "{:?}",
            reports[0].console
        );
        let reports = host.on_after_save(Path::new("C:\\shots\\a.png"));
        assert!(
            reports[0]
                .console
                .iter()
                .any(|l| l.text.contains("C:\\shots\\a.png|C:\\shots\\a.png"))
        );
        std::fs::remove_dir_all(s.root()).unwrap();
    }
}
