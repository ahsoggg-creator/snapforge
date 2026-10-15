# SnapForge plugins

A plugin is a small [Rhai](https://rhai.rs) script that reacts to captures, adds actions to the after-capture panel,
menu items and editor tools. Plugins run in a **sandbox** and can do only what the user approved.

- **No native code.** Only Rhai scripts. No DLLs, no executables, no `eval`, no `import` of files.
- **Explicit permissions.** A plugin asks for `fs.read`, `fs.write`, `net`, `clipboard` and `ui` in `plugin.toml`.
  The user sees the list when installing, and a plugin whose permissions change stops until they are approved again.
- **Hard limits** on operations, time, recursion, string/array/map sizes and the memory used for pictures.
- **No access to upload tokens.** Tokens live in the Windows Credential Manager and are not part of the plugin API.
- **Built-in editor.** *Plugins* in the app: a code editor with highlighting, templates, a test run on a generated
  picture with a console, enable/disable, export/import of `.sfplugin` files.

## A plugin on disk

A plugin is a folder (installed under `%APPDATA%\SnapForge\plugins\<id>\`) or a `.sfplugin` file (a zip of that folder):

```
plugin.toml     name, version, author, description, permissions
main.rhai       the script (the name can be changed with `main = "…"`)
```

```toml
name = "Watermark"
version = "1.0.0"
author = "you"
description = "Adds a text mark before saving"
main = "main.rhai"

[permissions]
clipboard = true                              # clipboard_get / clipboard_set / clipboard_set_image
ui = true                                     # notify / show_text
"fs.read"  = ["%USERPROFILE%\\Pictures"]      # load_image / read_text / list_dir / file_exists, only here
"fs.write" = ["%USERPROFILE%\\Pictures\\out"] # save_image / write_text, only here
net = ["api.example.com"]                     # http_get / http_post / http_upload, only these hosts
```

Rules for permissions:

- `fs.*` entries are **absolute folders** (environment variables like `%USERPROFILE%` are expanded). A drive root, a
  relative path or a path with `..` is rejected. Access is checked after resolving links and junctions, so a link
  inside an allowed folder cannot lead out of it. `fs.write` folders are also readable.
- `net` entries are host names (`api.example.com`; a scheme or path is ignored). A plugin may reach that host and its
  sub-domains; `*`, bare TLDs and empty entries are rejected. Redirects are **not followed**, because a redirect could
  leave the allowed hosts: the plugin gets the 3xx answer and may decide itself.
- Nothing is granted by default. `ui` and `clipboard` do not imply each other.

## How a script runs

Top-level statements run once when the plugin loads. This is where you register things:

```rhai
register_action("mark", "Add a watermark (plugin)");  // checkbox in the after-capture panel
register_menu("hello", "Say hello");                  // button on the Plugins screen
register_tool("pick", "Pick colour");                 // tool in the editor's tool list
```

Functions with special names are called by the app:

| Function | When | Arguments | Result |
| --- | --- | --- | --- |
| `on_capture(img)` | right after a capture, before the after-capture panel | picture | optional new picture |
| `on_before_save(img, info)` | before the picture is written to the screenshots folder | picture, `#{app, format, width, height}` | optional new picture |
| `on_after_save(path, info)` | after the file was written | path, `#{path}` | – |
| `action_<id>(img)` | the action registered with `register_action("<id>", …)` is ticked | picture | optional new picture |
| `menu_<id>()` | the menu item registered with `register_menu("<id>", …)` is clicked | – | – |
| `tool_<id>(img, x, y)` | click on the picture with the plugin tool | base picture, pixel position | optional new picture (replaces the base picture, undoable) |

A function may return a picture (`img`), or edit the picture it received in place and return nothing: both change the
result. If a function fails, **that plugin's change is discarded** and the other plugins still run; the error shows
up in the plugin console and as a notification.

## The API

Numbers may be integers or floats where a number is expected. A colour is `[r, g, b]` or `[r, g, b, a]` (0–255).

**Pictures** (`img` is an `Image`; methods change it in place)

| Call | What it does |
| --- | --- |
| `new_image(w, h, color)` | a new picture (1–16384 per side) |
| `img.width()`, `img.height()` | size |
| `img.get_pixel(x, y)` / `img.set_pixel(x, y, color)` | one pixel (`[r, g, b, a]`) |
| `img.fill_rect(x, y, w, h, color)` | filled rectangle |
| `img.draw_rect(x, y, w, h, color, line_width)` | rectangle outline |
| `img.draw_line(x1, y1, x2, y2, color, line_width)` | line |
| `img.draw_text(x, y, text, size, color)` | text (system font) |
| `img.blur(x, y, w, h, radius)` | blur an area |
| `img.pixelate(x, y, w, h, block)` | pixelate an area (good for hiding text) |
| `img.crop(x, y, w, h)` | a new picture with that area |
| `img.resize(w, h)` | a new picture of that size |
| `img.overlay(other, x, y, opacity)` | draw `other` on top (opacity 0–1) |
| `img.clone()` | a copy |
| `img.average_color(x, y, w, h)` | mean colour of an area |
| `img.ocr()` | recognised text (offline, Windows OCR) |
| `measure_text(text, size)` | `[width, height]` of the text |
| `distance(x1, y1, x2, y2)` | distance between two points |

**Files** (`fs.read` / `fs.write`) — `load_image(path)`, `save_image(img, path)` (`.png` or `.jpg`),
`read_text(path)`, `write_text(path, text)`, `list_dir(path)`, `file_exists(path)`. Files are limited to 64 MB.

**Network** (`net`) — each returns `#{status, body}`:

| Call | |
| --- | --- |
| `http_get(url)` / `http_get(url, headers)` | GET |
| `http_post(url, body, headers)` | POST a text body |
| `http_upload(url, field, filename, img, headers)` | POST the picture as `multipart/form-data` (PNG) |
| `json_get(body, "data.items[0].link")` | read a value from a JSON string (`()` if absent) |

`headers` is a map such as `#{ "Authorization": "Bearer …" }`. Only `http://` and `https://` URLs without
`user:password@` are accepted, and the host must be on the plugin's `net` list.

**App** — `clipboard_get()`, `clipboard_set(text)`, `clipboard_set_image(img)` (`clipboard`); `notify(text)`,
`show_text(title, text)` (`ui`); `log(text)` and `print(text)` (always allowed, shown in the console).

## Limits

Defaults for one function call (the app stops the script and reports it):

| Limit | Value |
| --- | --- |
| Operations | 20 000 000 (an endless loop ends with "too many operations") |
| Time | 10 s |
| Call depth / expression depth | 48 / 64 |
| String / array / map size | 1 MB / 20 000 / 2 000 items |
| Pixels allocated by the script (new images, crops, loads) | 512 MB in total per call |
| Script size | 512 KB |

The memory limit is about the objects the API creates; Rhai has no byte-exact heap limit, so the string, array and map
caps and the picture budget are what bounds a script. This is a sandbox for scripts you installed on purpose, not a
substitute for reading what you install.

## Examples

The repository ships three ready plugins in [`plugins/`](../plugins): **Hello** (menu item and a log), **Watermark**
(`on_before_save` + an action, no permissions) and **Custom Upload** (an action that uploads to your own server and
copies the link). They are also the templates of *New plugin*.

Log the text of every capture (offline OCR) and hide a fixed strip, e.g. a toolbar that shows your account name:

```rhai
fn on_capture(img) {
    let text = img.ocr();
    log("recognised " + text.len() + " characters");
    img.pixelate(0, 0, 220, 28, 8);
}
```

Save a copy next to the screenshots (needs `"fs.write"` for that folder):

```rhai
fn on_after_save(path, info) {
    let img = load_image(path);              // needs fs.read (a fs.write folder counts)
    let small = img.resize(img.width() / 2, img.height() / 2);
    save_image(small, "C:\\Users\\me\\Pictures\\SnapForge\\small.png");
}
```

Read the JSON answer of your own API (`net = ["api.example.com"]`):

```rhai
fn action_share(img) {
    let r = http_upload("https://api.example.com/v1/upload", "file", "shot.png", img, #{ "X-Key": "…" });
    let link = json_get(r.body, "data.url");
    if link != () { clipboard_set(link); notify("Link copied"); }
    img
}
```

## Installing, editing, sharing

- **In the app:** *Plugins → Create from the template* makes an editable copy; *Import .sfplugin…* shows what the
  plugin asks for and lets you install it enabled, or disabled. *Save* in the editor revokes the approval only when
  the permissions in `plugin.toml` changed. *Run on a test picture* uses a generated picture; with *No network or
  clipboard* ticked (the default) requests are not sent and the clipboard is not touched.
- **Export:** *Export…* writes a `.sfplugin` (a zip of `plugin.toml`, the script and small data files — never your
  local on/off and approval state). Archives are unpacked strictly: no absolute paths or `..`, only known file types
  (`toml`, `rhai`, `md`, `txt`, `png`, `jpg`, `json`), at most 64 files and 8 MB.
- **Command line:** `snapforge-cli plugin list`, `snapforge-cli plugin run <id|folder> --image in.png --hook
  action_mark --out out.png [--dry-run] [--grant]`. A plugin *folder* runs with no permissions unless you pass
  `--grant`, which allows exactly what its `plugin.toml` declares.

## Security notes (honest version)

- The sandbox is Rhai's: scripts have no ambient file, network, process or environment access; every capability is a
  function of this API with a permission check. `eval` is disabled and there is no module loading.
- A plugin that is approved for `net` and `fs.read` can send the files it may read to the hosts it may reach. Approve
  only what you understand; the permission list is shown for exactly this reason.
- Upload tokens and keys are never reachable: they are in the Credential Manager and there is no API for it. A
  token that a plugin author writes into their own script belongs to that plugin only.
- Timeouts cover script execution; a blocked network call is bounded by a 20 s HTTP timeout.
