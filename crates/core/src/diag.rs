//! Diagnostics: a rotating log file and redaction of anything secret-looking.
//!
//! Policy: the log only ever receives metadata (what happened, sizes, durations, error text) — never screenshot
//! content, OCR text, URLs with credentials or tokens. `redact` is a safety net on top of that, applied to every
//! line written and to crash reports.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

pub const MAX_LOG_BYTES: u64 = 512 * 1024;
pub const LOG_NAME: &str = "snapforge.log";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

struct Sink {
    dir: PathBuf,
    file: Option<File>,
    size: u64,
}

static SINK: Mutex<Option<Sink>> = Mutex::new(None);
static SECRETS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Exact secret values (upload tokens, keys) that must never appear in any log or report.
pub fn register_secret(secret: &str) {
    let s = secret.trim();
    if s.len() >= 4 {
        let mut g = SECRETS.lock().unwrap_or_else(|e| e.into_inner());
        if !g.iter().any(|x| x == s) {
            g.push(s.to_owned());
        }
    }
}

pub fn clear_secrets() {
    SECRETS.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

static PATTERNS: LazyLock<Vec<(regex::Regex, &'static str)>> = LazyLock::new(|| {
    let p = |re: &str, rep: &'static str| (regex::Regex::new(re).expect("static regex"), rep);
    vec![
        p(r"(?i)bot\d{5,}:[A-Za-z0-9_-]{20,}", "bot<redacted>"),
        p(
            r"(?i)(authorization|x-api-key|x-auth-token|cookie)\s*[:=]\s*[^\r\n]+",
            "$1: <redacted>",
        ),
        p(r"(?i)bearer\s+[A-Za-z0-9._~+/=-]{8,}", "Bearer <redacted>"),
        p(
            r"(?i)([?&;\s]|\b)(token|access_token|api[_-]?key|secret|secret[_-]?key|password|passwd|signature|x-amz-signature|x-amz-credential|key)=([^&\s]+)",
            "$1$2=<redacted>",
        ),
        p(r"\b(AKIA|ASIA)[0-9A-Z]{16}\b", "<aws-key-id>"),
        p(
            r"https://discord(?:app)?\.com/api/webhooks/\d+/[A-Za-z0-9_-]+",
            "<discord-webhook>",
        ),
    ]
});

/// Removes registered secrets and common credential shapes from `s`.
pub fn redact(s: &str) -> String {
    let mut out = s.to_owned();
    for secret in SECRETS.lock().unwrap_or_else(|e| e.into_inner()).iter() {
        out = out.replace(secret.as_str(), "<redacted>");
    }
    for (re, rep) in PATTERNS.iter() {
        out = re.replace_all(&out, *rep).into_owned();
    }
    out
}

/// Starts logging into `dir` (created if missing). Safe to call more than once.
pub fn init(dir: PathBuf) {
    let _ = fs::create_dir_all(&dir);
    let path = dir.join(LOG_NAME);
    let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let file = OpenOptions::new().create(true).append(true).open(&path).ok();
    *SINK.lock().unwrap_or_else(|e| e.into_inner()) = Some(Sink { dir, file, size });
}

pub fn log(level: Level, msg: &str) {
    let line = format!(
        "{} {:<5} {}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        match level {
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        },
        redact(msg).replace('\n', " | ")
    );
    let mut g = SINK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(sink) = g.as_mut() else { return };
    if sink.size + line.len() as u64 > MAX_LOG_BYTES {
        sink.file = None;
        let cur = sink.dir.join(LOG_NAME);
        let old = sink.dir.join(format!("{LOG_NAME}.1"));
        let _ = fs::remove_file(&old);
        let _ = fs::rename(&cur, &old);
        sink.file = OpenOptions::new().create(true).append(true).open(&cur).ok();
        sink.size = 0;
    }
    if let Some(f) = sink.file.as_mut()
        && f.write_all(line.as_bytes()).is_ok()
    {
        sink.size += line.len() as u64;
    }
}

/// Last `n` lines of the current log (already redacted when written).
pub fn tail(n: usize) -> String {
    let dir = SINK
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|s| s.dir.clone());
    let Some(dir) = dir else { return String::new() };
    let text = fs::read_to_string(dir.join(LOG_NAME)).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

#[macro_export]
macro_rules! sf_info {
    ($($a:tt)*) => { $crate::diag::log($crate::diag::Level::Info, &format!($($a)*)) };
}
#[macro_export]
macro_rules! sf_warn {
    ($($a:tt)*) => { $crate::diag::log($crate::diag::Level::Warn, &format!($($a)*)) };
}
#[macro_export]
macro_rules! sf_error {
    ($($a:tt)*) => { $crate::diag::log($crate::diag::Level::Error, &format!($($a)*)) };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The secret list and the sink are process-wide; tests that touch them run one at a time.
    static SERIAL: Mutex<()> = Mutex::new(());

    #[test]
    fn registered_secret_is_removed() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        register_secret("s3cr3t-value-123");
        let out = redact("uploading with key s3cr3t-value-123 now");
        assert!(!out.contains("s3cr3t-value-123"));
        clear_secrets();
    }

    #[test]
    fn telegram_bot_url_is_masked() {
        let out = redact("POST https://api.telegram.org/bot123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw/sendPhoto");
        assert!(!out.contains("AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw"), "{out}");
    }

    #[test]
    fn auth_headers_and_query_tokens_are_masked() {
        assert!(!redact("Authorization: Bearer abcdef1234567890").contains("abcdef1234567890"));
        assert!(!redact("GET /x?token=SECRETVALUE&a=1").contains("SECRETVALUE"));
        assert!(redact("GET /x?token=SECRETVALUE&a=1").contains("a=1"));
        assert!(!redact("X-Amz-Signature=deadbeefcafe0123").contains("deadbeefcafe0123"));
        assert!(!redact("id AKIAABCDEFGHIJKLMNOP end").contains("AKIAABCDEFGHIJKLMNOP"));
    }

    #[test]
    fn discord_webhook_is_masked() {
        let out = redact("https://discord.com/api/webhooks/123456789012345678/abcDEF_ghi-JKLmno");
        assert!(!out.contains("abcDEF_ghi"), "{out}");
    }

    #[test]
    fn plain_text_is_untouched() {
        assert_eq!(redact("captured 1920x1080 in 35 ms"), "captured 1920x1080 in 35 ms");
    }

    #[test]
    fn log_rotates_and_redacts() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("sf-diag-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        init(dir.clone());
        register_secret("TOPSECRETTOKEN");
        log(Level::Info, "token TOPSECRETTOKEN used");
        let t = tail(5);
        assert!(!t.contains("TOPSECRETTOKEN") && t.contains("<redacted>"), "{t}");
        let big = "x".repeat(1000);
        for _ in 0..700 {
            log(Level::Info, &big);
        }
        assert!(dir.join(format!("{LOG_NAME}.1")).exists(), "rotated file");
        assert!(fs::metadata(dir.join(LOG_NAME)).unwrap().len() <= MAX_LOG_BYTES);
        clear_secrets();
        *SINK.lock().unwrap() = None;
        let _ = fs::remove_dir_all(&dir);
    }
}
