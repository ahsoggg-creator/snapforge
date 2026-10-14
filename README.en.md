# SnapForge

[English](README.en.md) · [Русский](README.md)

A safer ShareX alternative for Windows: screen capture, an editor with vector annotations, a screenshot history and
global hotkeys. Everything stays on your computer; the only thing the app contacts is GitHub, to check for updates.

![Editor: blur, pixelate, arrow, numbered markers](docs/screenshots/03-editor.png)

*All screenshots were made on invented data (`SnapForge.exe --demo`) — no real windows, names or people are shown.*

## How it differs from ShareX

- **Safety.** No native DLL plugins and no arbitrary code. (Version 0.3.0 will add plugins as Rhai scripts in a
  sandbox with explicit permissions; 0.1.0 has no plugins.)
- **Local by default.** No uploads, no telemetry, no cloud. The only network request is the update check, once a day
  (it can be turned off).
- **Objects stay vector.** Arrows, frames, text, markers and blur can be moved, changed and deleted until you save
  the screenshot.
- **No administrator rights.** Installation and normal use are per-user.

## Features (0.1.0)

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
- **UI language:** Russian by default, English in the settings.
- **`snapforge-cli`:** `capture --region X,Y,W,H | --screen | --monitor N | --window`, `monitors`,
  `history list|clear`.

| Home | History |
| --- | --- |
| ![Home](docs/screenshots/01-home.png) | ![History](docs/screenshots/02-history.png) |

![Settings](docs/screenshots/04-settings.png)

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
- The only network request is the new-version check (`api.github.com`); it can be disabled. There is no telemetry.
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
scripts/      build and release, self-signed certificate, icon
docs/         screenshots, release notes
```

## License

Your choice of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
