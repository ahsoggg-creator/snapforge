//! Interface strings. Russian is the default; English is switched in the settings. Every string the UI shows goes
//! through `tr` — nothing is hard-coded in the views.

use std::collections::HashMap;
use std::sync::LazyLock;

use snapforge_core::settings::Lang;

/// `(key, русский, English)`. `{name}` placeholders are filled by `App::tf`.
const TABLE: &[(&str, &str, &str)] = &[
    // navigation
    ("nav.home", "Главная", "Home"),
    ("nav.history", "История", "History"),
    ("nav.editor", "Редактор", "Editor"),
    ("nav.settings", "Настройки", "Settings"),
    ("common.cancel", "Отмена", "Cancel"),
    // capture actions
    ("act.region", "Область", "Region"),
    ("act.window", "Окно", "Window"),
    ("act.screen", "Весь экран", "Full screen"),
    ("act.monitor", "Монитор под курсором", "Monitor under cursor"),
    ("act.monitor_n", "Монитор", "Monitor"),
    (
        "sel.hint_region",
        "Выделите область · стрелки двигают курсор на 1 пиксель (Shift — на 10) · Enter — начать/подтвердить · Esc или правая кнопка — отмена",
        "Drag to select · arrow keys move the cursor by 1 px (Shift: 10) · Enter starts/confirms · Esc or right click cancels",
    ),
    (
        "sel.hint_window",
        "Наведите на окно и щёлкните · Esc или правая кнопка — отмена",
        "Hover a window and click · Esc or right click cancels",
    ),
    // home
    ("home.title", "Снимок экрана без лишнего", "Screenshots, nothing extra"),
    (
        "home.subtitle",
        "Захват, редактор и история — всё хранится только на этом компьютере.",
        "Capture, editor and history — everything stays on this computer.",
    ),
    ("home.monitor", "Монитор:", "Monitor:"),
    ("home.rescan", "Обновить список мониторов", "Refresh the monitor list"),
    ("home.delay", "Задержка, с", "Delay, s"),
    ("home.cursor", "Курсор в кадре", "Include cursor"),
    (
        "home.hotkey_conflict",
        "Сочетания заняты другой программой: {keys}. Измените их в настройках.",
        "These hotkeys are taken by another program: {keys}. Change them in the settings.",
    ),
    ("home.recent", "Недавние снимки", "Recent screenshots"),
    (
        "home.no_history",
        "Пока пусто — сделайте первый снимок.",
        "Nothing yet — take your first screenshot.",
    ),
    (
        "home.privacy",
        "Ничего не отправляется в сеть, пока вы сами не настроите загрузку. Единственный запрос по умолчанию — проверка обновлений на GitHub.",
        "Nothing is sent anywhere until you set up an upload yourself. The only request by default is the update check on GitHub.",
    ),
    // history
    ("hist.title", "История", "History"),
    (
        "hist.search",
        "Поиск: программа, дата, размер",
        "Search: app, date, size",
    ),
    ("hist.clear", "Очистить историю…", "Clear history…"),
    ("hist.clear_title", "Очистить историю?", "Clear the history?"),
    (
        "hist.clear_text",
        "Будет удалено снимков: {n}. Это нельзя отменить.",
        "{n} screenshots will be deleted. This cannot be undone.",
    ),
    ("hist.clear_yes", "Удалить всё", "Delete all"),
    ("hist.cleared", "Удалено снимков: {n}", "Deleted screenshots: {n}"),
    ("hist.delete", "Удалить", "Delete"),
    ("hist.empty", "Ничего не найдено.", "Nothing found."),
    (
        "hist.keep_days",
        "Снимки старше {n} дн. удаляются автоматически.",
        "Screenshots older than {n} days are removed automatically.",
    ),
    (
        "hist.keep_forever",
        "Снимки хранятся, пока вы их не удалите.",
        "Screenshots are kept until you delete them.",
    ),
    // editor
    ("tool.select", "Выбор", "Select"),
    ("tool.crop", "Обрезка", "Crop"),
    ("tool.arrow", "Стрелка", "Arrow"),
    ("tool.line", "Линия", "Line"),
    ("tool.rect", "Прямоугольник", "Rectangle"),
    ("tool.ellipse", "Эллипс", "Ellipse"),
    ("tool.text", "Текст", "Text"),
    ("tool.pencil", "Карандаш", "Pencil"),
    ("tool.marker", "Маркер", "Marker"),
    ("tool.blur", "Размытие", "Blur"),
    ("tool.pixelate", "Пикселизация", "Pixelate"),
    ("tool.step", "Метка №", "Step №"),
    ("tool.eraser", "Ластик", "Eraser"),
    ("ed.close", "Закрыть", "Close"),
    ("ed.undo", "Отменить", "Undo"),
    ("ed.redo", "Повторить", "Redo"),
    ("ed.rot_left", "↺ Влево", "↺ Left"),
    ("ed.rot_right", "↻ Вправо", "↻ Right"),
    ("ed.fit", "По размеру окна", "Fit"),
    ("ed.save", "Сохранить", "Save"),
    ("ed.save_as", "Сохранить как…", "Save as…"),
    ("ed.copy", "В буфер", "Copy"),
    ("ed.drag", "Перетащить", "Drag out"),
    (
        "ed.drag_hint",
        "Зажмите и перетащите изображение в проводник, чат или другое окно",
        "Hold and drag the picture into Explorer, a chat or another window",
    ),
    ("ed.saved", "Сохранено: {path}", "Saved: {path}"),
    ("ed.copied", "Скопировано в буфер обмена", "Copied to the clipboard"),
    ("ed.apply_crop", "Применить обрезку", "Apply crop"),
    ("ed.style", "Оформление", "Style"),
    ("ed.color", "Цвет", "Colour"),
    ("ed.width", "Толщина", "Width"),
    ("ed.fill", "Заливка", "Fill"),
    ("ed.text_size", "Размер текста", "Text size"),
    ("ed.delete", "Удалить объект", "Delete object"),
    ("ed.layers", "Слои", "Layers"),
    ("ed.visible", "Показать / скрыть", "Show / hide"),
    ("ed.unsaved_title", "Закрыть без сохранения?", "Close without saving?"),
    (
        "ed.unsaved_text",
        "В снимок внесены правки, которые не сохранены.",
        "The screenshot has edits that were not saved.",
    ),
    ("ed.discard", "Закрыть без сохранения", "Discard and close"),
    ("ed.keep_editing", "Продолжить редактирование", "Keep editing"),
    // errors
    ("err.capture", "Не удалось сделать снимок", "Could not capture"),
    (
        "err.clipboard",
        "Не удалось скопировать в буфер",
        "Could not copy to the clipboard",
    ),
    ("err.open", "Не удалось открыть снимок", "Could not open the screenshot"),
    ("err.save", "Не удалось сохранить", "Could not save"),
    (
        "err.settings_save",
        "Не удалось сохранить настройки",
        "Could not save the settings",
    ),
    // settings
    ("set.title", "Настройки", "Settings"),
    ("set.general", "Общие", "General"),
    ("set.language", "Язык", "Language"),
    (
        "set.tray",
        "Сворачивать в трей при закрытии окна",
        "Minimise to the tray when the window is closed",
    ),
    ("set.autostart", "Запускать вместе с Windows", "Start with Windows"),
    ("set.saving", "Сохранение", "Saving"),
    ("set.folder", "Папка:", "Folder:"),
    ("set.choose", "Выбрать…", "Choose…"),
    ("set.open_folder", "Открыть", "Open"),
    ("set.template", "Шаблон имени:", "File name:"),
    (
        "set.template_help",
        "Подстановки: {date} — дата, {time} — время, {app} — программа в кадре.",
        "Placeholders: {date} date, {time} time, {app} application in the shot.",
    ),
    ("set.format", "Формат:", "Format:"),
    ("set.quality", "Качество JPEG", "JPEG quality"),
    ("set.history", "История", "History"),
    (
        "set.history_on",
        "Запоминать снимки в истории (только на этом компьютере)",
        "Remember screenshots in the history (this computer only)",
    ),
    ("set.retention", "Хранить:", "Keep for:"),
    ("set.days", " дн.", " days"),
    (
        "set.retention_hint",
        "0 — не удалять автоматически",
        "0 — never delete automatically",
    ),
    ("set.hotkeys", "Горячие клавиши", "Hotkeys"),
    (
        "set.hotkeys_on",
        "Включить глобальные горячие клавиши",
        "Enable global hotkeys",
    ),
    ("set.profile", "Профиль:", "Profile:"),
    ("set.profile_new", "Новый", "New"),
    ("set.profile_delete", "Удалить", "Delete"),
    ("set.profile_name", "Название:", "Name:"),
    ("set.profile_word", "Профиль", "Profile"),
    ("set.record", "Записать", "Record"),
    ("set.recording", "Нажмите сочетание…", "Press the keys…"),
    ("set.taken", "занято другой программой", "taken by another program"),
    ("set.apply_hotkeys", "Применить сочетания", "Apply hotkeys"),
    ("set.hotkeys_applied", "Сочетания применены", "Hotkeys applied"),
    (
        "set.err_invalid",
        "Есть некорректное сочетание — исправьте красные поля.",
        "Some combination is invalid — fix the red fields.",
    ),
    (
        "set.err_duplicate",
        "Два действия используют одно и то же сочетание.",
        "Two actions use the same combination.",
    ),
    (
        "set.err_taken",
        "Сочетаний, занятых другой программой: {n}. Выберите другие.",
        "Combinations taken by another program: {n}. Pick different ones.",
    ),
    ("set.updates", "Обновления", "Updates"),
    (
        "set.updates_on",
        "Проверять обновления раз в сутки",
        "Check for updates once a day",
    ),
    (
        "set.updates_note",
        "Это единственный сетевой запрос по умолчанию: он спрашивает api.github.com о последнем релизе. Телеметрии нет.",
        "This is the only network request by default: it asks api.github.com for the latest release. There is no telemetry.",
    ),
    ("set.system", "Система", "System"),
    ("set.portable", "Запущена переносная копия.", "This is a portable copy."),
    (
        "set.install",
        "Установить на этот компьютер",
        "Install on this computer",
    ),
    (
        "set.run_admin",
        "Перезапустить от администратора",
        "Restart as administrator",
    ),
    (
        "set.run_admin_hint",
        "Нужно только чтобы горячие клавиши работали поверх окон, запущенных от администратора. Обычная копия остаётся без повышенных прав.",
        "Only needed for hotkeys to work over windows that run as administrator. The normal copy stays unelevated.",
    ),
    ("set.diagnostics", "Диагностика", "Diagnostics"),
    (
        "set.diag_note",
        "Журнал и отчёты о сбоях лежат в папке данных; в них нет содержимого снимков и токенов.",
        "The log and crash reports live in the data folder; they contain no screenshot content and no tokens.",
    ),
    ("set.open_logs", "Открыть папку журналов", "Open the logs folder"),
    ("set.copy_diag", "Скопировать диагностику", "Copy diagnostics"),
    (
        "set.diag_copied",
        "Диагностический текст скопирован",
        "Diagnostic text copied",
    ),
    // updates
    ("upd.available", "Доступна версия {v}", "Version {v} is available"),
    ("upd.notes", "Что нового", "What's new"),
    ("upd.checking", "Проверяю…", "Checking…"),
    (
        "upd.uptodate",
        "Установлена последняя версия",
        "You have the latest version",
    ),
    ("upd.downloading", "Скачиваю…", "Downloading…"),
    (
        "upd.ready",
        "Версия {v} установлена, перезапустите программу",
        "Version {v} installed, restart the app",
    ),
    ("upd.restart", "Перезапустить ({v})", "Restart ({v})"),
    ("upd.failed", "Не удалось обновиться", "Update failed"),
    ("upd.check_now", "Проверить сейчас", "Check now"),
    // tray
    ("tray.show", "Открыть SnapForge", "Open SnapForge"),
    ("tray.region", "Снимок области", "Capture region"),
    ("tray.window", "Снимок окна", "Capture window"),
    ("tray.screen", "Снимок всего экрана", "Capture full screen"),
    ("tray.quit", "Выход", "Quit"),
    // installer
    ("inst.version", "версия", "version"),
    ("inst.installing", "Установка…", "Installing…"),
    ("inst.done", "Готово", "Done"),
    (
        "inst.done_text",
        "SnapForge установлен. Найти его можно через поиск в меню «Пуск».",
        "SnapForge is installed. Find it with the Start menu search.",
    ),
    ("inst.launch", "Запустить SnapForge", "Launch SnapForge"),
    ("inst.close", "Закрыть", "Close"),
    ("inst.failed", "Не удалось установить", "Installation failed"),
    ("inst.retry", "Попробовать ещё раз", "Try again"),
    ("inst.folder", "Папка установки", "Install folder"),
    ("inst.desktop", "Ярлык на рабочем столе", "Desktop shortcut"),
    (
        "inst.note",
        "Ярлык в меню «Пуск» создаётся всегда. Права администратора не нужны.",
        "A Start menu shortcut is always created. No administrator rights are needed.",
    ),
    ("inst.install", "Установить", "Install"),
    ("inst.update", "Обновить", "Update"),
    (
        "uninst.text",
        "SnapForge будет удалён с этого компьютера.",
        "SnapForge will be removed from this computer.",
    ),
    (
        "uninst.data",
        "Удалить также настройки и историю снимков",
        "Also delete the settings and the screenshot history",
    ),
    ("uninst.go", "Удалить", "Uninstall"),
    ("uninst.done", "SnapForge удалён", "SnapForge was removed"),
    ("uninst.failed", "Не удалось удалить", "Uninstall failed"),
];

static MAP: LazyLock<HashMap<&'static str, (&'static str, &'static str)>> =
    LazyLock::new(|| TABLE.iter().map(|(k, ru, en)| (*k, (*ru, *en))).collect());

/// The string for `key` in `lang`; an unknown key is returned as is, so a gap is visible rather than blank.
pub fn tr(lang: Lang, key: &'static str) -> &'static str {
    match MAP.get(key) {
        Some((ru, en)) => match lang {
            Lang::Ru => ru,
            Lang::En => en,
        },
        None => key,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique_and_never_empty() {
        assert_eq!(MAP.len(), TABLE.len(), "duplicate key");
        for (k, ru, en) in TABLE {
            assert!(!ru.is_empty() && !en.is_empty(), "{k} is empty");
        }
    }

    #[test]
    fn placeholders_match_between_languages() {
        let ph = |s: &str| {
            let mut v: Vec<String> = s
                .split('{')
                .skip(1)
                .filter_map(|p| p.split('}').next())
                .map(str::to_owned)
                .collect();
            v.sort();
            v
        };
        for (k, ru, en) in TABLE {
            if !k.starts_with("set.template_help") {
                assert_eq!(ph(ru), ph(en), "placeholders differ in {k}");
            }
        }
    }

    /// Every dotted key used in the UI sources must exist in the table (so nothing shows up as a raw key).
    #[test]
    fn all_keys_used_in_the_sources_exist() {
        const PREFIXES: [&str; 14] = [
            "act.", "common.", "ed.", "err.", "hist.", "home.", "inst.", "nav.", "sel.", "set.", "tool.", "tray.",
            "uninst.", "upd.",
        ];
        let sources = [
            include_str!("views.rs"),
            include_str!("editor_ui.rs"),
            include_str!("installer.rs"),
            include_str!("capture_flow.rs"),
            include_str!("tray.rs"),
            include_str!("main.rs"),
        ];
        let mut missing = Vec::new();
        for src in sources {
            for part in src.split('"').skip(1).step_by(2) {
                let is_key = PREFIXES.iter().any(|p| part.starts_with(p))
                    && part
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'_');
                if is_key && !MAP.contains_key(part) {
                    missing.push(part.to_owned());
                }
            }
        }
        assert!(missing.is_empty(), "missing translations: {missing:?}");
    }

    #[test]
    fn russian_and_english_differ() {
        assert_ne!(tr(Lang::Ru, "nav.home"), tr(Lang::En, "nav.home"));
        assert_eq!(tr(Lang::En, "no.such_key"), "no.such_key");
    }
}
