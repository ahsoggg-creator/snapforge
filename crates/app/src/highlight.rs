//! Syntax highlighting for the plugin editor (Rhai scripts and `plugin.toml`). Pure functions: text in, coloured
//! byte ranges out; they cover the whole text without gaps and never panic on unfinished input.

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{Color32, FontId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tok {
    Plain,
    Keyword,
    Builtin,
    String,
    Number,
    Comment,
    Section,
    Key,
}

const KEYWORDS: [&str; 33] = [
    "fn", "let", "const", "if", "else", "while", "loop", "for", "in", "return", "throw", "try", "catch", "true",
    "false", "import", "export", "as", "switch", "break", "continue", "do", "until", "private", "this", "global", "is",
    "not", "and", "or", "type_of", "print", "debug",
];

/// The host functions plugins can call (kept in step with `core::plugin::runtime`).
pub const BUILTINS: [&str; 41] = [
    "register_action",
    "register_menu",
    "register_tool",
    "new_image",
    "load_image",
    "save_image",
    "read_text",
    "write_text",
    "file_exists",
    "list_dir",
    "http_get",
    "http_post",
    "http_upload",
    "json_get",
    "clipboard_get",
    "clipboard_set",
    "clipboard_set_image",
    "notify",
    "show_text",
    "log",
    "measure_text",
    "distance",
    "width",
    "height",
    "get_pixel",
    "set_pixel",
    "fill_rect",
    "draw_rect",
    "draw_line",
    "draw_text",
    "blur",
    "pixelate",
    "crop",
    "resize",
    "clone",
    "overlay",
    "average_color",
    "ocr",
    "on_capture",
    "on_before_save",
    "on_after_save",
];

fn push(out: &mut Vec<(usize, usize, Tok)>, start: usize, end: usize, tok: Tok) {
    if end <= start {
        return;
    }
    match out.last_mut() {
        Some(last) if last.2 == tok && last.1 == start => last.1 = end,
        _ => out.push((start, end, tok)),
    }
}

pub fn rhai_tokens(src: &str) -> Vec<(usize, usize, Tok)> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let start = i;
        let c = b[i];
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            push(&mut out, start, i, Tok::Comment);
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                i += 1;
            }
            i = (i + 2).min(b.len());
            push(&mut out, start, i, Tok::Comment);
        } else if c == b'"' || c == b'`' || c == b'\'' {
            i += 1;
            while i < b.len() && b[i] != c {
                if b[i] == b'\\' && c != b'`' {
                    i += 1;
                }
                if c != b'`' && i < b.len() && b[i] == b'\n' {
                    break; // unterminated: stop at the end of the line
                }
                i += 1;
            }
            i = (i + 1).min(b.len());
            push(&mut out, start, i, Tok::String);
        } else if c.is_ascii_digit() {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'.') {
                i += 1;
            }
            push(&mut out, start, i, Tok::Number);
        } else if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let w = &src[start..i];
            let tok = if KEYWORDS.contains(&w) {
                Tok::Keyword
            } else if BUILTINS.contains(&w)
                || w.starts_with("action_")
                || w.starts_with("menu_")
                || w.starts_with("tool_")
            {
                Tok::Builtin
            } else {
                Tok::Plain
            };
            push(&mut out, start, i, tok);
        } else {
            // advance by one whole character
            i += src[i..].chars().next().map_or(1, char::len_utf8);
            push(&mut out, start, i, Tok::Plain);
        }
    }
    out
}

pub fn toml_tokens(src: &str) -> Vec<(usize, usize, Tok)> {
    let mut out = Vec::new();
    let mut pos = 0;
    for line in src.split_inclusive('\n') {
        let end = pos + line.len();
        let trimmed = line.trim_start();
        let lead = pos + (line.len() - trimmed.len());
        push(&mut out, pos, lead, Tok::Plain);
        if trimmed.starts_with('#') {
            push(&mut out, lead, end, Tok::Comment);
        } else if trimmed.starts_with('[') {
            push(&mut out, lead, end, Tok::Section);
        } else if let Some(eq) = trimmed.find('=') {
            push(&mut out, lead, lead + eq, Tok::Key);
            push(&mut out, lead + eq, lead + eq + 1, Tok::Plain);
            // the value: strings, a trailing comment, numbers and booleans
            let value = &trimmed[eq + 1..];
            let base = lead + eq + 1;
            let mut i = 0;
            let vb = value.as_bytes();
            while i < vb.len() {
                let s = i;
                match vb[i] {
                    b'"' | b'\'' => {
                        let q = vb[i];
                        i += 1;
                        while i < vb.len() && vb[i] != q && vb[i] != b'\n' {
                            if vb[i] == b'\\' {
                                i += 1;
                            }
                            i += 1;
                        }
                        i = (i + 1).min(vb.len());
                        push(&mut out, base + s, base + i, Tok::String);
                    }
                    b'#' => {
                        i = vb.len();
                        push(&mut out, base + s, base + i, Tok::Comment);
                    }
                    c if c.is_ascii_alphanumeric() => {
                        while i < vb.len() && (vb[i].is_ascii_alphanumeric() || vb[i] == b'.') {
                            i += 1;
                        }
                        push(&mut out, base + s, base + i, Tok::Number);
                    }
                    _ => {
                        i += value[i..].chars().next().map_or(1, char::len_utf8);
                        push(&mut out, base + s, base + i, Tok::Plain);
                    }
                }
            }
        } else {
            push(&mut out, lead, end, Tok::Plain);
        }
        pos = end;
    }
    out
}

fn color(t: Tok) -> Color32 {
    match t {
        Tok::Plain => Color32::from_rgb(214, 218, 228),
        Tok::Keyword => Color32::from_rgb(198, 120, 221),
        Tok::Builtin => Color32::from_rgb(97, 175, 239),
        Tok::String => Color32::from_rgb(152, 195, 121),
        Tok::Number => Color32::from_rgb(229, 192, 123),
        Tok::Comment => Color32::from_rgb(110, 118, 134),
        Tok::Section => Color32::from_rgb(224, 108, 117),
        Tok::Key => Color32::from_rgb(86, 182, 194),
    }
}

pub fn layout(src: &str, toml: bool, font: FontId, wrap: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let toks = if toml { toml_tokens(src) } else { rhai_tokens(src) };
    for (s, e, t) in toks {
        job.append(
            &src[s..e],
            0.0,
            TextFormat {
                font_id: font.clone(),
                color: color(t),
                ..Default::default()
            },
        );
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str, toml: bool) -> Vec<(String, Tok)> {
        let toks = if toml { toml_tokens(src) } else { rhai_tokens(src) };
        toks.into_iter().map(|(s, e, t)| (src[s..e].to_owned(), t)).collect()
    }

    fn assert_covers(src: &str, toml: bool) {
        let toks = if toml { toml_tokens(src) } else { rhai_tokens(src) };
        let mut at = 0;
        for (s, e, _) in &toks {
            assert_eq!(*s, at, "gap or overlap in {src:?}");
            at = *e;
        }
        assert_eq!(at, src.len(), "the tokens cover the whole text of {src:?}");
    }

    #[test]
    fn rhai_basics() {
        let k = kinds("fn on_capture(img) { let x = 42; // done\n notify(\"hi\") }", false);
        assert!(k.contains(&("fn".into(), Tok::Keyword)));
        assert!(k.contains(&("on_capture".into(), Tok::Builtin)));
        assert!(k.contains(&("let".into(), Tok::Keyword)));
        assert!(k.contains(&("42".into(), Tok::Number)));
        assert!(k.contains(&("// done".into(), Tok::Comment)));
        assert!(k.contains(&("notify".into(), Tok::Builtin)));
        assert!(k.contains(&("\"hi\"".into(), Tok::String)));
    }

    #[test]
    fn rhai_block_comments_strings_and_escapes() {
        let k = kinds("/* a\nb */ x \"q\\\"uote\" `multi\nline`", false);
        assert_eq!(k[0], ("/* a\nb */".into(), Tok::Comment));
        assert!(k.contains(&("\"q\\\"uote\"".into(), Tok::String)));
        assert!(k.contains(&("`multi\nline`".into(), Tok::String)));
    }

    #[test]
    fn unfinished_input_never_panics_and_is_fully_covered() {
        for src in [
            "\"open string",
            "/* open comment",
            "`open",
            "'",
            "\\",
            "ёжик = \"ключ\" // комментарий",
            "0x",
            "",
            "a\u{1F600}b",
            "fn",
            "\"a\\",
        ] {
            assert_covers(src, false);
            assert_covers(src, true);
        }
    }

    #[test]
    fn toml_sections_keys_values_and_comments() {
        let k = kinds(
            "# c\n[permissions]\nname = \"x\" # tail\nui = true\n\"fs.read\" = [\"C:\\\\a\"]\n",
            true,
        );
        assert!(k.contains(&("# c\n".into(), Tok::Comment)));
        assert!(k.contains(&("[permissions]\n".into(), Tok::Section)));
        assert!(k.contains(&("name ".into(), Tok::Key)));
        assert!(k.contains(&("\"x\"".into(), Tok::String)));
        assert!(k.contains(&("# tail\n".into(), Tok::Comment)));
        assert!(k.contains(&("true".into(), Tok::Number)));
        assert_covers("# c\n[permissions]\nname = \"x\" # tail\nui = true\n", true);
    }
}
