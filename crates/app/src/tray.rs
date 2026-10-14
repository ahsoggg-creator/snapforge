//! System tray icon with a small menu. Events arrive through a global channel and are polled by the UI loop.

use std::sync::mpsc::{Receiver, channel};
use std::sync::{Mutex, OnceLock};

use eframe::egui;
use snapforge_core::hotkey::HotAction;
use snapforge_core::settings::Lang;
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::i18n;

pub enum TrayEvent {
    Show,
    Capture(HotAction),
    Quit,
}

pub struct Tray {
    _icon: TrayIcon,
}

static EVENTS: OnceLock<Mutex<Receiver<TrayEvent>>> = OnceLock::new();

impl Tray {
    pub fn new(ctx: &egui::Context, lang: Lang) -> Option<Tray> {
        let (tx, rx) = channel();
        let _ = EVENTS.set(Mutex::new(rx));
        let t = |k| i18n::tr(lang, k);
        let item = |label: &str| MenuItem::new(label, true, None);
        let (show, region, window, screen, quit) = (
            item(t("tray.show")),
            item(t("tray.region")),
            item(t("tray.window")),
            item(t("tray.screen")),
            item(t("tray.quit")),
        );
        let menu = Menu::new();
        menu.append_items(&[
            &show,
            &PredefinedMenuItem::separator(),
            &region,
            &window,
            &screen,
            &PredefinedMenuItem::separator(),
            &quit,
        ])
        .ok()?;
        let ids: [(MenuId, TrayEvent); 5] = [
            (show.id().clone(), TrayEvent::Show),
            (region.id().clone(), TrayEvent::Capture(HotAction::CaptureRegion)),
            (window.id().clone(), TrayEvent::Capture(HotAction::CaptureWindow)),
            (screen.id().clone(), TrayEvent::Capture(HotAction::CaptureScreen)),
            (quit.id().clone(), TrayEvent::Quit),
        ];
        let (tx_menu, ctx_menu) = (tx.clone(), ctx.clone());
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            if let Some((_, ev)) = ids.iter().find(|(id, _)| *id == e.id) {
                let _ = tx_menu.send(match ev {
                    TrayEvent::Show => TrayEvent::Show,
                    TrayEvent::Capture(a) => TrayEvent::Capture(*a),
                    TrayEvent::Quit => TrayEvent::Quit,
                });
                ctx_menu.request_repaint();
            }
        }));
        let ctx_click = ctx.clone();
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = e
            {
                let _ = tx.send(TrayEvent::Show);
                ctx_click.request_repaint();
            }
        }));
        let bytes = include_bytes!("../assets/icon64.rgba");
        let icon = Icon::from_rgba(bytes.to_vec(), 64, 64).ok()?;
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("SnapForge")
            .with_icon(icon)
            .build()
            .ok()?;
        Some(Tray { _icon: tray })
    }
}

pub fn poll() -> Vec<TrayEvent> {
    EVENTS
        .get()
        .and_then(|m| m.lock().ok())
        .map(|rx| rx.try_iter().collect())
        .unwrap_or_default()
}
