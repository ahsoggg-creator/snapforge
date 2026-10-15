# SnapForge

[English](README.en.md) · [Русский](README.md)

A safer ShareX alternative for Windows: screen capture, an editor with vector annotations, a screenshot history and
global hotkeys. Everything stays on your computer; the only thing the app contacts is GitHub, to check for updates.

![The built-in plugin editor: highlighted code, permissions, console](docs/screenshots/07-plugins.png)

*All screenshots were made on invented data (`SnapForge.exe --demo`) — no real windows, names or people are shown.*

## How it differs from ShareX

- **Safety.** No native DLL plugins and no arbitrary code: plugins are Rhai scripts in a sandbox that can do only
  what you explicitly allowed (files only in the folders you name, network only to the domains you name, the clipboard,
  the UI), with limits on time, operations and memory.
- **Local by default.** Uploads are off until you switch them on and configure them; no telemetry, no cloud. The only
  network request by default is the update check, once a day (it can be turned off).
- **Objects stay vector.** Arrows, frames, text, markers and blur can be moved, changed and deleted until you save
  the screenshot.
- **No administrator rights.** Installation and normal use are per-user.

## Features (0.3.0)

- **Capture:** region (frozen frame, magnifier next to the cursor, pixel-exact selection, arrow keys nudge the
  cursor by 1 px, Shift: 10), window, full screen, a single monitor. Multiple monitors with different scaling: all
  coordinates are physical pixels of the virtual desktop and a region spanning monitors is stitched from pieces.
  Optional delay and cursor in the shot.
- **How it captures:** Windows.Graphics.Capture, falling back to GDI if it is unavailable or fails.
- **Hotkeys:** global (`RegisterHotKey`), configurable in the app, with a "record" button; duplicates and combinations
  already used by other programs are reported right away; several profiles. Tray icon, optional autostart.
- **Editor:** crop, rotate, arrow, line, rectangle and ellipse (filled or not), text, pencil, marker, blur and
  pixelate for an area, numbered markers, eraser, undo/redo, zoom (Ctrl + wheel, pan with the middle button), a layers
  panel (order, show/hide, delete).
- **Output:** to a folder by a name template (`{date}_{time}_{app}`), to the clipboard, "Save as…" (PNG/JPEG), and
  dragging the image out of the window into Explorer, a chat, etc.
- **History:** thumbnail grid, search by application, date and size, delete, open in the editor. A retention period
  and a "clear history" button.
- **After capture (0.2.0).** A panel with checkboxes that you can adjust at the moment of capture; presets are
  saved. Available actions: save, copy to the clipboard, open in the editor, upload, blur faces, recognise text and
  copy it, hide sensitive text (e-mail addresses, phone numbers, keys and tokens found by OCR), trim empty margins, add a
  watermark (text or PNG), reduce the size. Every action can be turned off; by default only "open in the editor" is on
  and uploading is unavailable until it is enabled in the settings.
- **Review before applying.** Found faces and text fragments are framed on the shot, and any area can be unticked
  before it is covered.
- **Faces and OCR work offline only.** OCR uses `Windows.Media.Ocr` with the languages installed in Windows; face
  detection is a small ONNX model run on this computer (`tract`). The model is **not bundled**: you download it with
  a button (about 1.2 MB), its SHA-256 is verified, and without it the feature honestly says "model not installed".
- **Charts (0.2.0).** A "Chart" tool: bar, line and pie charts from a table typed in or pasted from the clipboard
  (CSV/TSV, decimal comma or dot). A chart is a normal editable object: move it, resize it, change colours, labels and
  data.
- **Uploads (0.2.0, off by default).** A custom HTTP uploader (URL, method, headers, file field, JSON path of the link in
  the response), a Discord webhook, a Telegram bot, S3-compatible storage (AWS SigV4 signing). Tokens and keys are
  stored **only in the Windows Credential Manager**, never in the settings files, and are stripped from the log and
  crash reports. The link is copied to the clipboard after an upload.
- **Plugins (0.3.0).** Rhai scripts: the hooks `on_capture`, `on_before_save`, `on_after_save`, actions in the
  after-capture panel, menu items and editor tools; picture operations (blur, pixelate, crop, text, overlay, pixels,
  measuring), OCR, the clipboard, notifications — all through permissions. A plugin is a folder or a `.sfplugin` archive
  with `plugin.toml` (name, version, author, description, requested permissions) and a script. The built-in editor has
  a plugin list, highlighted code, templates ("Hello", "Watermark", "Upload to my own server"), a test run on a sample
  picture with an error/output console, enable/disable and export/import of `.sfplugin`. Installing shows exactly what
  the plugin asks for; if its permissions change, it stops until you approve them again. A plugin cannot read upload
  tokens. Details and examples: [docs/PLUGINS.md](docs/PLUGINS.md); three ready plugins live in [`plugins/`](plugins).
- **UI language:** Russian by default, English in the settings.
- **`snapforge-cli`:** `capture --region X,Y,W,H | --screen | --monitor N | --window`, `monitors`, `ocr <file>`,
  `plugin list`, `plugin run <plugin> --image file --hook name`, `history list|clear`.

| A plugin with permissions and a test run | The after-capture review |
| --- | --- |
| ![Plugin](docs/screenshots/08-plugins-upload.png) | ![Review](docs/screenshots/04-review.png) |

| After capture | Charts in the editor |
| --- | --- |
| ![After capture](docs/screenshots/03-post.png) | ![Charts](docs/screenshots/06-chart.png) |

| Home | History |
| --- | --- |
| ![Home](docs/screenshots/01-home.png) | ![History](docs/screenshots/02-history.png) |

![Settings: uploads and after capture](docs/screenshots/11-settings-uploads.png)

## Installation

Download `SnapForge-Setup.exe` from [Releases](https://github.com/ahsoggg-creator/snapforge/releases/latest) and run
it. No administrator rights are needed: the app is installed to `%LOCALAPPDATA%\Programs\SnapForge`, adds a Start menu
shortcut (a desktop one if you like) and an entry in "Apps & features". Uninstall from there or with
`SnapForge.exe --uninstall`.

A portable copy is simply `SnapForge.exe`; it can be installed later from the settings.

`SnapForge.exe` switches:

| Switch | What it does |
| --- | --- |
| `--install` / `--uninstall` | the install / uninstall window (same as `SnapForge-Setup.exe`) |
| `--update` | check for and install an update without a window |
| `--minimized` | start in the tray (what autostart uses) |
| `--demo` | invented data: nothing real is read or written |
| `--screenshots <folder>` | regenerate the README screenshots |

### Code signing and SmartScreen

The release executables are **not signed**, so SmartScreen may warn about an "unknown publisher". That is expected
for a project without a paid certificate; you can verify a download against `SHA256SUMS.txt` from the release. The
`scripts/` folder has optional scripts for signing with a self-signed certificate (`make_cert.ps1`,
`trust_cert.ps1`, `sign.ps1`, `release.ps1`) — the private key stays on your computer and `*.pfx` files are
git-ignored. A self-signed certificate does not remove the SmartScreen warning until you trust it yourself.

## Privacy and diagnostics

- Screenshots are personal data: the history lives only in `%APPDATA%\SnapForge\history`. The retention period
  (30 days by default) and "clear history" are in the settings.
- The only network request by default is the new-version check (`api.github.com`); it can be disabled. Uploading is
  off until you enable it and enable a specific destination; the face model is downloaded only when you press the
  button. There is no telemetry.
- Log: `%APPDATA%\SnapForge\logs\snapforge.log` (rotates at 512 KB). Crash reports:
  `logs\crashes\crash-<time>.txt` (version, panic location, stack, log tail). Screenshot content never goes into the
  log or the reports; tokens and keys that might appear in text are stripped.
- Settings → "Diagnostics": open the logs folder or copy a diagnostic text.

## Building

```powershell
cargo build --release          # SnapForge.exe and snapforge-cli.exe in target\release
cargo test --workspace         # unit tests
cargo test --workspace -- --ignored --test-threads=1   # tests that need a real desktop
cargo clippy --workspace --all-targets -- -D warnings
```

Needs Rust stable (edition 2024) and Windows with MSVC. The full set of release files: `scripts\release.ps1`.

## Project layout

```
crates/core   all logic without UI: capture, monitor geometry, editor, history, settings, hotkeys
crates/app    egui GUI + installer, auto-update, tray, diagnostics, demo mode
crates/cli    snapforge-cli
plugins/      three ready plugins (Hello, Watermark, Custom Upload)
scripts/      build and release, self-signed certificate, icon
docs/         PLUGINS.md, screenshots, release notes
```

## License

Your choice of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
