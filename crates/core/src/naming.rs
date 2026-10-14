//! File-name templates: `{date}_{time}_{app}` and safe, unique file names.

use std::path::{Path, PathBuf};

use chrono::NaiveDateTime;

pub const DEFAULT_TEMPLATE: &str = "{date}_{time}_{app}";
const MAX_STEM: usize = 120;
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2",
    "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Expands `{date}` (`2026-10-15`), `{time}` (`13-45-09`) and `{app}`. Unknown placeholders are kept as text.
pub fn expand(template: &str, app: &str, now: NaiveDateTime) -> String {
    let app = if app.trim().is_empty() { "screen" } else { app };
    let s = template
        .replace("{date}", &now.format("%Y-%m-%d").to_string())
        .replace("{time}", &now.format("%H-%M-%S").to_string())
        .replace("{app}", app);
    sanitize(&s)
}

/// Makes `name` a valid Windows file stem: no forbidden characters, no trailing dots/spaces, no device names.
pub fn sanitize(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    s = s.trim_matches(|c: char| c == '.' || c == ' ').to_owned();
    if s.chars().count() > MAX_STEM {
        s = s.chars().take(MAX_STEM).collect();
        s = s.trim_end_matches(|c: char| c == '.' || c == ' ').to_owned();
    }
    let head = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    if RESERVED.contains(&head.as_str()) {
        s.insert(0, '_');
    }
    if s.is_empty() { "capture".to_owned() } else { s }
}

/// `dir/stem.ext`, or `dir/stem_1.ext`, `stem_2.ext`… if that already exists.
pub fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.{ext}"));
    if !first.exists() {
        return first;
    }
    (1..)
        .map(|n| dir.join(format!("{stem}_{n}.{ext}")))
        .find(|p| !p.exists())
        .expect("unbounded range")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn t() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 5)
            .unwrap()
            .and_hms_opt(9, 4, 7)
            .unwrap()
    }

    #[test]
    fn default_template() {
        assert_eq!(expand(DEFAULT_TEMPLATE, "chrome", t()), "2026-10-05_09-04-07_chrome");
    }

    #[test]
    fn empty_app_falls_back() {
        assert_eq!(expand("{app}", "  ", t()), "screen");
    }

    #[test]
    fn forbidden_chars_are_replaced() {
        assert_eq!(expand("{app}", "a<b>:c|d?", t()), "a_b__c_d_");
        assert_eq!(sanitize("..x. "), "x");
    }

    #[test]
    fn reserved_names_get_prefix() {
        assert_eq!(sanitize("con"), "_con");
        assert_eq!(sanitize("NUL.txt"), "_NUL.txt");
        assert_eq!(sanitize("console"), "console");
    }

    #[test]
    fn long_names_are_cut() {
        assert_eq!(sanitize(&"x".repeat(500)).chars().count(), MAX_STEM);
    }

    #[test]
    fn unique_path_adds_counter() {
        let dir = std::env::temp_dir().join(format!("sf-naming-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p1 = unique_path(&dir, "shot", "png");
        assert_eq!(p1.file_name().unwrap(), "shot.png");
        std::fs::write(&p1, b"").unwrap();
        let p2 = unique_path(&dir, "shot", "png");
        assert_eq!(p2.file_name().unwrap(), "shot_1.png");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
