//! Crash reports and the diagnostic text. Nothing here ever includes screenshots, OCR text or tokens: the log tail is
//! already redacted, and the whole report passes through `redact` once more.

use std::backtrace::Backtrace;
use std::panic::PanicHookInfo;

use snapforge_core::{diag, paths};

pub fn os_version() -> String {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("cmd")
        .args(["/C", "ver"])
        .creation_flags(0x0800_0000)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Windows".to_owned())
}

/// Version, system and monitor layout plus the end of the log. Safe to paste into a bug report.
pub fn diagnostic_text() -> String {
    let monitors: Vec<String> = snapforge_core::capture::sys::monitors()
        .iter()
        .map(|m| {
            format!(
                "  {}: {}x{} at ({},{}) {}%",
                m.index,
                m.rect.w,
                m.rect.h,
                m.rect.x,
                m.rect.y,
                m.dpi * 100 / 96
            )
        })
        .collect();
    let text = format!(
        "SnapForge {}\n{}\nMonitors:\n{}\n\nLog (last 40 lines):\n{}\n",
        env!("CARGO_PKG_VERSION"),
        os_version(),
        monitors.join("\n"),
        diag::tail(40)
    );
    diag::redact(&text)
}

fn report(info: &PanicHookInfo<'_>) -> String {
    let place = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_default();
    let msg = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(no message)".to_owned());
    let text = format!(
        "SnapForge {} crashed\nat: {place}\nmessage: {msg}\n\nbacktrace:\n{}\n\nlog tail:\n{}\n",
        env!("CARGO_PKG_VERSION"),
        Backtrace::force_capture(),
        diag::tail(40)
    );
    diag::redact(&text)
}

/// Writes `logs\crashes\crash-<time>.txt` on every panic (and still runs the default hook).
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let dir = paths::crashes_dir();
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join(format!("crash-{}.txt", chrono::Local::now().format("%Y%m%d-%H%M%S")));
        let _ = std::fs::write(&file, report(info));
        diag::log(diag::Level::Error, &format!("panic, report: {}", file.display()));
        default(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_report_has_no_registered_secret() {
        let dir = std::env::temp_dir().join(format!("sf-crash-{}", std::process::id()));
        // SAFETY: only this test uses this variable.
        unsafe { std::env::set_var("SNAPFORGE_DATA", &dir) };
        diag::init(paths::logs_dir());
        diag::register_secret("SUPERSECRETTOKEN123");
        diag::log(diag::Level::Info, "uploading with SUPERSECRETTOKEN123");
        let result = std::panic::catch_unwind(|| {
            install_panic_hook();
            panic!("boom SUPERSECRETTOKEN123");
        });
        assert!(result.is_err());
        let _ = std::panic::take_hook();
        let crashes: Vec<_> = std::fs::read_dir(paths::crashes_dir()).unwrap().flatten().collect();
        assert_eq!(crashes.len(), 1);
        let text = std::fs::read_to_string(crashes[0].path()).unwrap();
        assert!(text.contains("boom") && text.contains("SnapForge"));
        assert!(
            !text.contains("SUPERSECRETTOKEN123"),
            "secret leaked into the crash report"
        );
        assert!(!diagnostic_text().contains("SUPERSECRETTOKEN123"));
        diag::clear_secrets();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
