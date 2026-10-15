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
        "Выделите область · один щелчок — весь экран · стрелки двигают курсор на 1 пиксель (Shift — на 10) · Enter — начать/подтвердить · Esc или правая кнопка — отмена",
        "Drag to select · one click takes the whole screen · arrow keys move the cursor by 1 px (Shift: 10) · Enter starts/confirms · Esc or right click cancels",
    ),
    (
        "sel.hint_window",
        "Наведите на окно и щёлкните (мимо окон — весь экран) · Esc или правая кнопка — отмена",
        "Hover a window and click (click beside the windows for the whole screen) · Esc or right click cancels",
    ),
    (
        "sel.hint_screen",
        "Нарисуйте или замажьте что нужно, затем щёлкните (или Enter) — снимок экрана · Esc или правая кнопка — отмена",
        "Draw or blur what you need, then click (or Enter) to capture the screen · Esc or right click cancels",
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
    ("upd.retry", "Повторить загрузку ({v})", "Retry download ({v})"),
    ("upd.install", "Установить", "Install"),
    ("upd.check_now", "Проверить сейчас", "Check now"),
    // tray
    ("tray.show", "Открыть SnapForge", "Open SnapForge"),
    ("tray.region", "Снимок области", "Capture region"),
    ("tray.window", "Снимок окна", "Capture window"),
    ("tray.screen", "Снимок всего экрана", "Capture full screen"),
    ("home.after", "После снимка:", "After a shot:"),
    (
        "home.show_panel",
        "каждый раз спрашивать (панель)",
        "ask every time (panel)",
    ),
    ("tool.spotlight", "Затемнение", "Spotlight"),
    ("tool.picker", "Пипетка", "Eyedropper"),
    ("ed.flip_h", "↔ Отразить", "↔ Flip"),
    ("ed.flip_v", "↕ Отразить", "↕ Flip"),
    ("ed.resize", "Размер…", "Resize…"),
    ("ed.resize_title", "Размер картинки", "Resize the picture"),
    ("ed.resize_pct", "Масштаб, %", "Scale, %"),
    ("ed.frame", "Рамка…", "Frame…"),
    ("ed.frame_title", "Рамка и фон", "Frame and background"),
    ("ed.frame_pad", "Отступ", "Padding"),
    ("ed.frame_radius", "Скругление", "Corner radius"),
    ("ed.frame_shadow", "Тень", "Shadow"),
    ("ed.frame_top", "Фон сверху", "Background top"),
    ("ed.frame_bottom", "снизу", "bottom"),
    ("ed.frame_apply", "Применить", "Apply"),
    ("ed.auto_faces", "Скрыть лица", "Hide faces"),
    ("ed.auto_text", "Скрыть текст", "Hide text"),
    (
        "ed.auto_hint",
        "Найти автоматически и закрыть блюром (останется отдельным слоем: можно убрать или сдвинуть)",
        "Find automatically and cover with blur (stays a separate layer: you can move or delete it)",
    ),
    (
        "ed.need_model",
        "Нужна модель поиска лиц: скачайте её в настройках (около 1,2 МБ)",
        "The face detection model is needed: download it in the settings (about 1.2 MB)",
    ),
    ("ed.faces_none", "Лица не найдены", "No faces found"),
    ("ed.faces_added", "Скрыто лиц: {n}", "Faces hidden: {n}"),
    (
        "ed.text_none",
        "Чувствительного текста не найдено",
        "No sensitive text found",
    ),
    ("ed.text_added", "Скрыто фрагментов: {n}", "Pieces hidden: {n}"),
    ("tray.settings", "Настройки", "Settings"),
    (
        "tray.monitor",
        "Снимок монитора под курсором",
        "Capture monitor under cursor",
    ),
    ("tray.record", "Записать видео", "Record video"),
    ("tray.record_stop", "Остановить запись", "Stop recording"),
    ("tray.recording", "запись", "recording"),
    ("tray.quit", "Выход", "Quit"),
    ("act.record", "Запись видео", "Record video"),
    (
        "sel.hint_record",
        "Выберите область, окно или монитор для записи · Esc или правая кнопка — отмена",
        "Pick the area, window or monitor to record · Esc or right click cancels",
    ),
    ("rec.banner", "Идёт запись: {t}", "Recording: {t}"),
    ("rec.stop", "Остановить запись", "Stop recording"),
    ("rec.saved", "Видео сохранено", "Video saved"),
    ("rec.last", "Последнее видео:", "Last video:"),
    ("rec.open", "Открыть", "Open"),
    ("rec.folder", "В папке", "Show in folder"),
    ("rec.failed", "Не удалось записать видео", "Recording failed"),
    (
        "rec.started",
        "Запись идёт: остановить — {key} или значок в трее",
        "Recording: stop with {key} or the tray icon",
    ),
    (
        "rec.demo",
        "В демо-режиме запись видео отключена",
        "Video recording is disabled in demo mode",
    ),
    ("set.video", "Запись видео", "Video recording"),
    ("set.video_fps", "Кадров в секунду:", "Frames per second:"),
    (
        "set.video_cursor",
        "Показывать курсор в видео",
        "Show the cursor in the video",
    ),
    (
        "set.video_note",
        "Видео H.264 (MP4) без звука, сохраняется в папку для снимков. Максимум 4096×2304: для больших экранов выберите монитор или область.",
        "H.264 (MP4) video without sound, saved to the screenshots folder. At most 4096×2304: pick one monitor or a region on bigger screens.",
    ),
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
    // capture overlay
    ("otool.region", "Область", "Region"),
    ("otool.window", "Окно", "Window"),
    (
        "otool.screen",
        "Весь экран (монитор под курсором)",
        "Whole screen (the monitor under the cursor)",
    ),
    (
        "otool.ocr",
        "Текст из области (OCR) — сразу в буфер",
        "Text from an area (OCR) — straight to the clipboard",
    ),
    ("otool.rect", "Рамка", "Rectangle"),
    ("otool.ellipse", "Эллипс", "Ellipse"),
    ("otool.line", "Линия", "Line"),
    ("otool.arrow", "Стрелка", "Arrow"),
    ("otool.pencil", "Карандаш", "Pencil"),
    ("otool.marker", "Маркер", "Marker"),
    ("otool.text", "Текст", "Text"),
    ("otool.step", "Метка с номером", "Numbered marker"),
    ("otool.blur", "Размытие", "Blur"),
    ("otool.pixelate", "Пикселизация", "Pixelate"),
    (
        "otool.select",
        "Выбрать: двигать, менять размер, удалить нарисованное",
        "Select: move, resize or delete what you drew",
    ),
    (
        "otool.spotlight",
        "Затемнить всё вокруг выделенного",
        "Dim everything around the selected area",
    ),
    (
        "otool.eraser",
        "Ластик (стирает нарисованное)",
        "Eraser (wipes what you drew)",
    ),
    (
        "otool.picker",
        "Пипетка: взять цвет с экрана",
        "Picker: take a colour from the screen",
    ),
    ("otool.redo", "Вернуть", "Redo"),
    ("otool.fill", "Заливка рамок и эллипсов", "Fill rectangles and ellipses"),
    (
        "sel.hint_select",
        "Щёлкните по нарисованному · тяните — двигать · за квадратики — менять размер · Delete — удалить · Esc — снять выбор",
        "Click what you drew · drag to move · drag the squares to resize · Delete removes · Esc deselects",
    ),
    (
        "sel.hint_eraser",
        "Проведите по нарисованному, чтобы стереть · Ctrl+Z вернёт · правая кнопка — отмена",
        "Drag over what you drew to wipe it · Ctrl+Z brings it back · right click cancels",
    ),
    (
        "sel.hint_picker",
        "Щёлкните по любой точке экрана — этот цвет станет цветом рисования",
        "Click any point of the screen — its colour becomes the drawing colour",
    ),
    (
        "set.ov_tools",
        "Инструменты на панели (снятая галочка прячет кнопку, клавиша продолжает работать):",
        "Tools on the toolbar (unticking hides the button, its key keeps working):",
    ),
    ("set.ov_palette", "Цвета на панели:", "Toolbar colours:"),
    ("set.ov_palette_add", "+ цвет", "+ colour"),
    ("set.ov_palette_reset", "Цвета по умолчанию", "Default colours"),
    (
        "set.ov_click",
        "Один щелчок без выделения:",
        "A single click without dragging:",
    ),
    (
        "set.ov_click_monitor",
        "весь монитор под курсором",
        "the whole monitor under the cursor",
    ),
    ("set.ov_click_all", "все мониторы сразу", "all monitors together"),
    (
        "set.ov_click_window",
        "окно под курсором",
        "the window under the cursor",
    ),
    (
        "set.ov_click_nothing",
        "ничего (только выделение мышью)",
        "nothing (only dragging selects)",
    ),
    ("set.ov_toolbar", "Панель инструментов:", "Toolbar position:"),
    ("set.ov_top", "вверху", "top"),
    ("set.ov_bottom", "внизу", "bottom"),
    (
        "set.ov_dim",
        "Затемнение невыбранной части экрана",
        "Darkening of the part of the screen that is not selected",
    ),
    (
        "set.ov_magnifier",
        "Лупа с цветом пикселя у курсора",
        "Magnifier with the pixel colour next to the cursor",
    ),
    (
        "set.ov_fill",
        "Новые рамки и эллипсы — с заливкой",
        "New rectangles and ellipses are filled",
    ),
    (
        "set.post_review",
        "Показывать проверку найденных лиц и текста перед скрытием (иначе всё найденное закрывается сразу; сообщение об ошибке покажется в любом случае)",
        "Show the review of found faces and text before hiding them (otherwise everything found is hidden at once; an error is always shown)",
    ),
    (
        "set.post_restore",
        "Возвращать окно SnapForge после снимка, если оно было открыто (при выключенной панели «после снимка»)",
        "Bring the SnapForge window back after a shot if it was open (when the after-capture panel is off)",
    ),
    ("otool.undo", "Отменить", "Undo"),
    ("otool.cancel", "Отмена", "Cancel"),
    (
        "otool.privacy",
        "Скрыть чувствительное (почта, телефоны, ключи) — после снимка",
        "Hide sensitive text (e-mail, phones, keys) — after the capture",
    ),
    ("set.overlay", "Панель при съёмке", "Capture toolbar"),
    (
        "set.overlay_help",
        "Панель инструментов вверху экрана при выборе области: можно рисовать на снимке до выделения. Нарисованное попадает в редактор как отдельные объекты.",
        "The toolbar at the top of the screen while you pick an area: draw on the screenshot before selecting. What you draw reaches the editor as separate objects.",
    ),
    ("set.overlay_start", "Начинать с инструмента:", "Start with the tool:"),
    (
        "set.overlay_privacy",
        "Сразу включать «скрыть чувствительное»",
        "Switch on \"hide sensitive text\" from the start",
    ),
    (
        "set.overlay_screens",
        "Панель инструментов и для снимка всего экрана / монитора",
        "Show the toolbar for full-screen and monitor captures too",
    ),
    (
        "set.overlay_keys",
        "Клавиши инструментов (одна клавиша; пусто — без клавиши):",
        "Tool keys (a single key; empty = no key):",
    ),
    (
        "set.key_taken",
        "уже занята другим инструментом",
        "used by another tool",
    ),
    (
        "set.key_reserved",
        "зарезервирована (Esc, Enter, стрелки…)",
        "reserved (Esc, Enter, arrows…)",
    ),
    ("set.key_unknown", "неизвестная клавиша", "unknown key"),
    ("set.reset_hotkeys", "Вернуть по умолчанию", "Restore the defaults"),
    (
        "set.hotkeys_hint",
        "Нажмите «Записать» и сочетание — подойдёт любая клавиша, включая PrintScreen и Win. Сочетание применяется сразу.",
        "Press \"Record\" and then the combination — any key works, including PrintScreen and Win. It applies at once.",
    ),
    // plugins
    ("nav.plugins", "Плагины", "Plugins"),
    ("plug.installed", "Установленные плагины", "Installed plugins"),
    (
        "plug.none",
        "Плагинов пока нет. Создайте из шаблона или импортируйте .sfplugin.",
        "No plugins yet. Create one from a template or import a .sfplugin.",
    ),
    ("plug.enable", "Включить / выключить", "Enable / disable"),
    (
        "plug.needs_approval",
        "Нужно подтвердить разрешения",
        "The permissions must be approved",
    ),
    ("plug.new", "Новый плагин", "New plugin"),
    ("plug.create", "Создать из шаблона", "Create from the template"),
    ("plug.import", "Импорт .sfplugin…", "Import .sfplugin…"),
    ("plug.menu", "Пункты меню плагинов", "Plugin menu items"),
    ("plug.no_menu", "Нет пунктов меню.", "No menu items."),
    (
        "plug.pick",
        "Выберите плагин слева или создайте новый.",
        "Pick a plugin on the left or create a new one.",
    ),
    (
        "plug.sandbox_note",
        "Плагины — скрипты Rhai в песочнице: у них нет доступа к файлам, сети и буферу обмена, пока вы не одобрите явно запрошенные разрешения. Нативный код и произвольные библиотеки не поддерживаются.",
        "Plugins are Rhai scripts in a sandbox: they cannot touch files, the network or the clipboard until you approve the permissions they ask for. Native code and arbitrary libraries are not supported.",
    ),
    ("plug.delete", "Удалить", "Delete"),
    ("plug.export", "Экспорт…", "Export…"),
    ("plug.requests", "Плагин запрашивает:", "The plugin asks for:"),
    (
        "plug.approve",
        "Одобрить эти разрешения и включить",
        "Approve these permissions and enable",
    ),
    ("plug.save", "Сохранить", "Save"),
    ("plug.unsaved", "есть несохранённые правки", "unsaved changes"),
    ("plug.run_fn", "Запустить на тестовом снимке:", "Run on a test picture:"),
    ("plug.dry", "Без сети и буфера", "No network or clipboard"),
    (
        "plug.dry_hint",
        "Пробный запуск: запросы не отправляются, буфер обмена не меняется",
        "A dry run: no request is sent and the clipboard is not changed",
    ),
    ("plug.run", "Запустить", "Run"),
    ("plug.console", "Консоль", "Console"),
    ("plug.clear", "Очистить", "Clear"),
    ("plug.result", "Результат", "Result"),
    ("plug.install_title", "Установить плагин?", "Install the plugin?"),
    (
        "plug.install_warn",
        "Плагин сможет только то, что перечислено выше. Разрешения можно проверить в любой момент; если их изменить, плагин остановится до нового подтверждения.",
        "The plugin can do only what is listed above. If its permissions change later, it stops until you approve them again.",
    ),
    ("plug.install_allow", "Установить и разрешить", "Install and allow"),
    ("plug.install_off", "Установить выключенным", "Install disabled"),
    ("plug.hook_failed", "Ошибка плагина", "Plugin error"),
    ("plug.exported", "Экспортировано", "Exported"),
    ("plug.save_first", "Сначала сохраните плагин", "Save the plugin first"),
    (
        "perm.none",
        "Разрешений нет: плагин только обрабатывает переданное ему изображение.",
        "No permissions: the plugin only processes the picture it is given.",
    ),
    (
        "perm.clipboard",
        "Буфер обмена (читать и записывать)",
        "Clipboard (read and write)",
    ),
    (
        "perm.ui",
        "Показывать уведомления и окна с текстом",
        "Show notifications and text windows",
    ),
    ("perm.fs_read", "Читать файлы только в:", "Read files only in:"),
    ("perm.fs_write", "Записывать файлы только в:", "Write files only in:"),
    (
        "perm.net",
        "Обращаться по сети только к:",
        "Reach only these hosts on the network:",
    ),
    ("post.plugin_actions", "Плагины", "Plugins"),
    ("ed.plugin_tools", "Инструменты плагинов", "Plugin tools"),
    // post-capture panel
    ("nav.post", "После снимка", "After capture"),
    ("post.title", "После снимка", "After capture"),
    ("post.preset", "Набор:", "Preset:"),
    ("post.transform", "Обработка", "Processing"),
    ("post.trim", "Обрезать пустые поля", "Trim empty margins"),
    (
        "post.hide_sensitive",
        "Скрыть чувствительное (почта, телефоны, ключи)",
        "Hide sensitive text (e-mail, phones, keys)",
    ),
    ("post.blur_faces", "Размыть лица", "Blur faces"),
    ("post.model_missing", "модель не установлена", "model not installed"),
    ("post.watermark", "Добавить водяной знак", "Add a watermark"),
    ("post.downscale", "Уменьшить размер", "Reduce the size"),
    ("post.outputs", "Что сделать", "Then"),
    ("post.save", "Сохранить в папку", "Save to the folder"),
    ("post.clipboard", "Копировать в буфер", "Copy to the clipboard"),
    (
        "post.ocr",
        "Распознать текст и скопировать его",
        "Recognise text and copy it",
    ),
    ("post.editor", "Открыть в редакторе", "Open in the editor"),
    (
        "post.upload",
        "Загрузить (ссылка — в буфер)",
        "Upload (the link is copied)",
    ),
    (
        "post.upload_off",
        "Загрузка выключена: включите её и выберите назначение в настройках.",
        "Uploads are off: enable them and pick a destination in the settings.",
    ),
    (
        "post.clip_conflict",
        "Выбраны оба варианта: в буфере останется распознанный текст.",
        "Both are ticked: the recognised text stays in the clipboard.",
    ),
    ("post.run", "Выполнить", "Run"),
    ("post.skip", "Только в редактор", "Editor only"),
    ("post.discard", "Отбросить снимок", "Discard the shot"),
    (
        "post.always_show",
        "Показывать эту панель после каждого снимка",
        "Show this panel after every shot",
    ),
    ("post.analyzing", "Ищу лица и текст…", "Looking for faces and text…"),
    ("post.review", "Проверьте найденное", "Review what was found"),
    (
        "post.review_hint",
        "Снимите галочку, если область менять не нужно. Рамки на снимке слева.",
        "Untick anything that should stay as it is. The frames are drawn on the shot.",
    ),
    ("post.faces", "Лица", "Faces"),
    ("post.face_n", "Лицо", "Face"),
    ("post.sensitive", "Чувствительный текст", "Sensitive text"),
    ("post.kind_email", "почта", "e-mail"),
    ("post.kind_phone", "телефон", "phone"),
    ("post.kind_secret", "ключ/токен", "key/token"),
    (
        "post.show_text",
        "Показать распознанный текст",
        "Show the recognised text",
    ),
    ("post.download_model", "Скачать модель", "Download the model"),
    ("post.downloading", "Скачиваю модель…", "Downloading the model…"),
    ("post.apply", "Применить", "Apply"),
    ("post.back", "Назад", "Back"),
    ("post.working", "Выполняю…", "Working…"),
    ("post.done", "Готово", "Done"),
    ("post.open_editor", "Открыть в редакторе", "Open in the editor"),
    // editor extras
    ("tool.chart", "График", "Chart"),
    ("ed.ocr", "Распознать текст (OCR)", "Recognise text (OCR)"),
    ("ed.chart_edit", "Данные графика…", "Chart data…"),
    ("ocr.title", "Распознанный текст", "Recognised text"),
    ("ocr.running", "Распознаю…", "Recognising…"),
    ("ocr.copy", "Скопировать", "Copy"),
    ("ocr.empty", "Текст не найден.", "No text found."),
    ("ocr.copied", "Текст скопирован", "Text copied"),
    ("chart.title", "График", "Chart"),
    ("chart.name", "Заголовок:", "Title:"),
    ("chart.kind", "Тип:", "Type:"),
    ("chart.bar", "Столбчатая", "Bar"),
    ("chart.line", "Линейная", "Line"),
    ("chart.pie", "Круговая", "Pie"),
    (
        "chart.paste",
        "Вставить из буфера (CSV/TSV)",
        "Paste from the clipboard (CSV/TSV)",
    ),
    (
        "chart.paste_err",
        "Не удалось разобрать данные",
        "Could not read the data",
    ),
    ("chart.apply", "Применить", "Apply"),
    ("chart.add_row", "+ строка", "+ row"),
    ("chart.add_col", "+ ряд", "+ series"),
    ("chart.del_row", "− строка", "− row"),
    ("chart.del_col", "− ряд", "− series"),
    ("chart.legend", "Легенда", "Legend"),
    ("chart.values", "Значения", "Values"),
    ("chart.label", "Подпись", "Label"),
    ("chart.series", "Ряд", "Series"),
    // settings: post-capture
    ("set.post", "После снимка", "After capture"),
    (
        "set.post_show",
        "Показывать панель «после снимка» (иначе сразу выполняется выбранный набор)",
        "Show the after-capture panel (otherwise the active preset runs at once)",
    ),
    ("set.preset_new", "Новый набор", "New preset"),
    ("set.preset_delete", "Удалить набор", "Delete preset"),
    ("set.preset_name", "Название набора:", "Preset name:"),
    ("set.wm_text", "Водяной знак — текст:", "Watermark text:"),
    ("set.wm_image", "или картинка (PNG):", "or an image (PNG):"),
    ("set.wm_corner", "Угол:", "Corner:"),
    ("set.wm_opacity", "Прозрачность", "Opacity"),
    ("set.downscale_pct", "Уменьшить до, %", "Scale to, %"),
    ("set.hide_style", "Скрывать текст:", "Hide text with:"),
    ("set.style_pixelate", "пикселизацией", "pixelation"),
    ("set.style_blur", "размытием", "blur"),
    ("set.ocr_lang", "Язык OCR:", "OCR language:"),
    ("set.ocr_auto", "по языкам Windows", "from Windows languages"),
    (
        "set.ocr_none",
        "В Windows не установлен ни один язык с распознаванием текста.",
        "No language with text recognition is installed in Windows.",
    ),
    ("set.face_model", "Модель поиска лиц", "Face detection model"),
    (
        "set.model_ready",
        "установлена (SHA-256 проверена)",
        "installed (SHA-256 verified)",
    ),
    (
        "set.model_missing",
        "не установлена — поиск лиц не работает",
        "not installed — face detection does not work",
    ),
    (
        "set.model_corrupt",
        "повреждена — скачайте заново",
        "damaged — download it again",
    ),
    ("set.model_download", "Скачать модель", "Download the model"),
    ("set.model_delete", "Удалить модель", "Delete the model"),
    (
        "set.model_note",
        "Модель не входит в программу: она скачивается по запросу с GitHub, хеш SHA-256 проверяется. Распознавание лиц идёт на этом компьютере.",
        "The model is not part of the program: it is downloaded on request from GitHub and its SHA-256 is verified. Detection runs on this computer.",
    ),
    ("set.face_threshold", "Порог уверенности", "Confidence threshold"),
    ("corner.top_left", "слева сверху", "top left"),
    ("corner.top_right", "справа сверху", "top right"),
    ("corner.bottom_right", "справа снизу", "bottom right"),
    ("corner.bottom_left", "слева снизу", "bottom left"),
    ("corner.center", "по центру", "centre"),
    // settings: uploads
    ("up.title", "Загрузка", "Uploads"),
    ("up.master", "Разрешить загрузку снимков", "Allow uploading screenshots"),
    (
        "up.master_note",
        "Выключено по умолчанию. Пока вы это не включите и не настроите назначение, ничего никуда не отправляется. Токены и ключи хранятся только в Диспетчере учётных данных Windows, не в файлах настроек.",
        "Off by default. Until you switch it on and configure a destination nothing is sent anywhere. Tokens and keys are stored only in the Windows Credential Manager, never in the settings files.",
    ),
    ("up.none", "Назначений пока нет.", "No destinations yet."),
    ("up.add", "Добавить:", "Add:"),
    ("up.enabled", "включено", "enabled"),
    ("up.active", "по умолчанию", "default"),
    ("up.delete", "Удалить назначение", "Delete destination"),
    ("up.name", "Название:", "Name:"),
    ("up.secret_set", "задан", "set"),
    ("up.secret_unset", "не задан", "not set"),
    ("up.secret_save", "Сохранить в Windows", "Save to Windows"),
    ("up.secret_clear", "Забыть", "Forget"),
    ("up.secret_hint", "значение", "value"),
    ("up.url", "URL:", "URL:"),
    ("up.method", "Метод:", "Method:"),
    (
        "up.headers",
        "Заголовки (в значении можно писать {secret:token}):",
        "Headers (a value may contain {secret:token}):",
    ),
    (
        "up.file_field",
        "Поле файла (пусто — тело запроса):",
        "File field (empty = raw body):",
    ),
    (
        "up.json_path",
        "Путь к ссылке в JSON-ответе (например data.link):",
        "Link path in the JSON response (e.g. data.link):",
    ),
    ("up.chat_id", "Chat ID или @канал:", "Chat ID or @channel:"),
    ("up.endpoint", "Адрес сервера (https://…):", "Endpoint (https://…):"),
    ("up.bucket", "Бакет:", "Bucket:"),
    ("up.region", "Регион:", "Region:"),
    ("up.prefix", "Префикс ключа:", "Key prefix:"),
    (
        "up.public_base",
        "Публичный адрес для ссылки (необязательно):",
        "Public base URL for the link (optional):",
    ),
    (
        "up.path_style",
        "Адресация вида endpoint/bucket/key",
        "Path-style addressing (endpoint/bucket/key)",
    ),
    ("up.add_row", "+ заголовок", "+ header"),
    ("up.secret.token", "Токен", "Token"),
    ("up.secret.webhook", "URL вебхука", "Webhook URL"),
    ("up.secret.access_key", "Access key", "Access key"),
    ("up.secret.secret_key", "Secret key", "Secret key"),
    (
        "up.test",
        "Проверить (отправит маленькую тестовую картинку)",
        "Test (sends a tiny test picture)",
    ),
    ("up.test_ok", "Тест прошёл", "Test succeeded"),
    ("up.confirm_title", "Включить загрузку?", "Enable uploads?"),
    (
        "up.confirm_text",
        "Снимки смогут отправляться на назначения, которые вы включите ниже. Это ваше решение: картинки могут содержать личные данные.",
        "Screenshots may then be sent to the destinations you enable below. It is your decision: pictures may contain personal data.",
    ),
    ("up.confirm_yes", "Включить", "Enable"),
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
