//! System tray icon with a small menu. Events arrive through a global channel and are polled by the UI loop.
//!
//! The channel and the event handlers live as long as the process, so the icon can be rebuilt (language change)
//! without losing the connection to the UI.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Mutex, OnceLock};

use eframe::egui;
use snapforge_core::hotkey::HotAction;
use snapforge_core::settings::Lang;
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::i18n;

#[derive(Clone, Copy)]
pub enum TrayEvent {
    Show,
    Settings,
    Capture(HotAction),
    /// Start or stop a screen recording.
    Record,
    Quit,
}

pub struct Tray {
    icon: TrayIcon,
    record: MenuItem,
    lang: Lang,
}

struct Channel {
    tx: Sender<TrayEvent>,
    rx: Mutex<Receiver<TrayEvent>>,
}

static CHANNEL: OnceLock<Channel> = OnceLock::new();
/// Menu item ids of the icon that exists now.
static IDS: Mutex<Vec<(MenuId, TrayEvent)>> = Mutex::new(Vec::new());

fn channel_ref() -> &'static Channel {
    CHANNEL.get_or_init(|| {
        let (tx, rx) = channel();
        Channel { tx, rx: Mutex::new(rx) }
    })
}

impl Tray {
    pub fn new(ctx: &egui::Context, lang: Lang, recording: bool) -> Option<Tray> {
        let ch = channel_ref();
        let t = |k| i18n::tr(lang, k);
        let item = |label: &str| MenuItem::new(label, true, None);
        let show = item(t("tray.show"));
        let settings = item(t("tray.settings"));
        let region = item(t("tray.region"));
        let window = item(t("tray.window"));
        let screen = item(t("tray.screen"));
        let monitor = item(t("tray.monitor"));
        let record = item(t(if recording { "tray.record_stop" } else { "tray.record" }));
        let quit = item(t("tray.quit"));
        let menu = Menu::new();
        menu.append_items(&[
            &show,
            &settings,
            &PredefinedMenuItem::separator(),
            &region,
            &window,
            &screen,
            &monitor,
            &PredefinedMenuItem::separator(),
            &record,
            &PredefinedMenuItem::separator(),
            &quit,
        ])
        .ok()?;
        *IDS.lock().unwrap_or_else(|e| e.into_inner()) = vec![
            (show.id().clone(), TrayEvent::Show),
            (settings.id().clone(), TrayEvent::Settings),
            (region.id().clone(), TrayEvent::Capture(HotAction::CaptureRegion)),
            (window.id().clone(), TrayEvent::Capture(HotAction::CaptureWindow)),
            (screen.id().clone(), TrayEvent::Capture(HotAction::CaptureScreen)),
            (monitor.id().clone(), TrayEvent::Capture(HotAction::CaptureMonitor)),
            (record.id().clone(), TrayEvent::Record),
            (quit.id().clone(), TrayEvent::Quit),
        ];
        let (tx_menu, ctx_menu) = (ch.tx.clone(), ctx.clone());
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let ev = IDS
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .iter()
                .find(|(id, _)| *id == e.id)
                .map(|(_, ev)| *ev);
            if let Some(ev) = ev {
                let _ = tx_menu.send(ev);
                ctx_menu.request_repaint();
            }
        }));
        let (tx_click, ctx_click) = (ch.tx.clone(), ctx.clone());
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            // a left click opens the window, a right click the menu
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = e
            {
                let _ = tx_click.send(TrayEvent::Show);
                ctx_click.request_repaint();
            }
        }));
        let bytes = include_bytes!("../assets/icon64.rgba");
        let icon = Icon::from_rgba(bytes.to_vec(), 64, 64).ok()?;
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip("SnapForge")
            .with_icon(icon)
            .build()
            .ok()?;
        Some(Tray {
            icon: tray,
            record,
            lang,
        })
    }

    /// Switches the menu item and the tooltip between "record" and "stop" (with the elapsed time).
    pub fn set_recording(&self, recording: Option<std::time::Duration>) {
        let t = |k| i18n::tr(self.lang, k);
        match recording {
            Some(d) => {
                self.record.set_text(t("tray.record_stop"));
                let s = d.as_secs();
                let _ = self.icon.set_tooltip(Some(format!(
                    "SnapForge — {} {:02}:{:02}",
                    t("tray.recording"),
                    s / 60,
                    s % 60
                )));
            }
            None => {
                self.record.set_text(t("tray.record"));
                let _ = self.icon.set_tooltip(Some("SnapForge"));
            }
        }
    }
}

pub fn poll() -> Vec<TrayEvent> {
    channel_ref()
        .rx
        .lock()
        .map(|rx| rx.try_iter().collect())
        .unwrap_or_default()
}
