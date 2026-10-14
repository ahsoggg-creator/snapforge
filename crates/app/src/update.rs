//! Self-update from GitHub releases: the only network request SnapForge makes by default.
//!
//! The latest release must carry `SnapForge.exe` and `SHA256SUMS.txt`. The new exe is downloaded next to the
//! running one, its SHA-256 is verified against the sums file, then the files are swapped: Windows lets a running
//! executable be renamed (not overwritten), so the old one becomes `SnapForge.exe.old` and is deleted on the next
//! start. The new version runs after a restart.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};

const API: &str = "https://api.github.com/repos/ahsoggg-creator/snapforge/releases/latest";
const EXE_ASSET: &str = "SnapForge.exe";
const SUM_ASSET: &str = "SHA256SUMS.txt";
/// Check at most once a day.
pub const CHECK_INTERVAL_SECS: u64 = 24 * 3600;

#[derive(Clone, PartialEq, Debug)]
pub enum State {
    Idle,
    Checking,
    UpToDate,
    Available {
        version: String,
        url: String,
    },
    Downloading {
        version: String,
        done: u64,
        total: u64,
    },
    /// Installed; takes effect after a restart.
    Ready {
        version: String,
    },
    Failed(String),
}

pub type Shared = Arc<Mutex<State>>;

pub struct Release {
    pub version: String,
    pub page: String,
    exe_url: String,
    exe_size: u64,
    sum_url: String,
}

fn agent() -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::NativeTls)
        .build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_global(Some(std::time::Duration::from_secs(120)))
        .build()
        .into()
}

fn user_agent() -> String {
    format!("SnapForge/{}", env!("CARGO_PKG_VERSION"))
}

/// `"v1.2.3"` / `"1.2.3"` -> (1, 2, 3).
pub fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let core = s.split(['-', '+']).next()?;
    let mut it = core.split('.').map(|p| p.parse::<u64>().ok());
    Some((
        it.next()??,
        it.next().flatten().unwrap_or(0),
        it.next().flatten().unwrap_or(0),
    ))
}

pub fn is_newer(remote: &str, local: &str) -> bool {
    matches!((parse_version(remote), parse_version(local)), (Some(r), Some(l)) if r > l)
}

/// Parses `sha256sum` output: `<64 hex>  <name>` (a `*` before the name marks binary mode).
pub fn parse_sums(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let (hash, name) = l.trim().split_once(char::is_whitespace)?;
            let name = name.trim().trim_start_matches('*');
            (hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()) && !name.is_empty())
                .then(|| (name.to_owned(), hash.to_ascii_lowercase()))
        })
        .collect()
}

#[cfg(test)]
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

pub fn latest() -> Result<Release, String> {
    let mut resp = agent()
        .get(API)
        .header("User-Agent", &user_agent())
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(404) => "no releases on GitHub yet".to_owned(),
            e => format!("GitHub is unreachable: {e}"),
        })?;
    let text = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let tag = v["tag_name"].as_str().ok_or("no tag_name in the GitHub response")?;
    let asset = |name: &str| {
        v["assets"].as_array().and_then(|a| {
            a.iter().find(|x| x["name"].as_str() == Some(name)).map(|x| {
                (
                    x["browser_download_url"].as_str().unwrap_or("").to_owned(),
                    x["size"].as_u64().unwrap_or(0),
                )
            })
        })
    };
    let (exe_url, exe_size) = asset(EXE_ASSET).ok_or("the release has no SnapForge.exe")?;
    let (sum_url, _) = asset(SUM_ASSET).ok_or("the release has no SHA256SUMS.txt")?;
    Ok(Release {
        version: tag.trim_start_matches('v').to_owned(),
        page: v["html_url"].as_str().unwrap_or(crate::winint::REPO_URL).to_owned(),
        exe_url,
        exe_size,
        sum_url,
    })
}

fn target_exe() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|e| e.to_string())
}

/// Deletes leftovers of a previous update.
pub fn cleanup_old() {
    if let Ok(exe) = target_exe() {
        let _ = std::fs::remove_file(exe.with_extension("exe.old"));
        let _ = std::fs::remove_file(exe.with_extension("exe.download"));
    }
}

fn dir_writable(dir: &Path) -> bool {
    let probe = dir.join(".snapforge-write-test");
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// Replaces `target` with `new`: the running file is renamed to `*.old`, then `new` takes its name.
pub fn swap_in(new: &Path, target: &Path) -> Result<(), String> {
    let old = target.with_extension("exe.old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(target, &old).map_err(|e| format!("cannot move the current file: {e}"))?;
    if let Err(e) = std::fs::rename(new, target) {
        let _ = std::fs::rename(&old, target); // roll back
        return Err(format!("cannot place the new file: {e}"));
    }
    Ok(())
}

fn download(rel: &Release, state: &Shared, repaint: &dyn Fn()) -> Result<(), String> {
    let exe = target_exe()?;
    let tmp = exe.with_extension("exe.download");
    let sums_text = agent()
        .get(&rel.sum_url)
        .header("User-Agent", &user_agent())
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    let expected = parse_sums(&sums_text)
        .remove(EXE_ASSET)
        .ok_or("SHA256SUMS.txt has no entry for SnapForge.exe")?;

    let mut resp = agent()
        .get(&rel.exe_url)
        .header("User-Agent", &user_agent())
        .call()
        .map_err(|e| e.to_string())?;
    let mut reader = resp.body_mut().as_reader();
    let mut file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        *state.lock().unwrap_or_else(|e| e.into_inner()) = State::Downloading {
            version: rel.version.clone(),
            done,
            total: rel.exe_size.max(done),
        };
        repaint();
    }
    drop(file);
    let actual: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if actual != expected {
        let _ = std::fs::remove_file(&tmp);
        return Err("the SHA-256 of the download does not match SHA256SUMS.txt; the file was discarded".into());
    }
    swap_in(&tmp, &exe)
}

/// Checks for a newer release. With `auto_install` it also downloads, verifies and swaps the exe.
pub fn check(state: Shared, auto_install: bool, repaint: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        let set = |s: State| {
            *state.lock().unwrap_or_else(|e| e.into_inner()) = s;
            repaint();
        };
        set(State::Checking);
        match latest() {
            Err(e) => set(State::Failed(e)),
            Ok(rel) if !is_newer(&rel.version, env!("CARGO_PKG_VERSION")) => set(State::UpToDate),
            Ok(rel) => {
                let writable = target_exe()
                    .ok()
                    .and_then(|e| e.parent().map(dir_writable))
                    .unwrap_or(false);
                if !auto_install || !writable {
                    set(State::Available {
                        version: rel.version.clone(),
                        url: rel.page.clone(),
                    });
                    return;
                }
                match download(&rel, &state, &|| {}) {
                    Ok(()) => set(State::Ready { version: rel.version }),
                    Err(e) => set(State::Failed(e)),
                }
            }
        }
    });
}

/// Installs an update that `check` found (button "Update").
pub fn install(state: Shared, repaint: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        let set = |s: State| {
            *state.lock().unwrap_or_else(|e| e.into_inner()) = s;
            repaint();
        };
        match latest().and_then(|rel| download(&rel, &state, &repaint).map(|()| rel.version)) {
            Ok(v) => set(State::Ready { version: v }),
            Err(e) => set(State::Failed(e)),
        }
    });
}

/// `SnapForge.exe --update`: no window. Exit code 0 = up to date or updated, 1 = failed.
pub fn run_headless() -> i32 {
    println!("SnapForge {} — checking for updates…", env!("CARGO_PKG_VERSION"));
    let rel = match latest() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if !is_newer(&rel.version, env!("CARGO_PKG_VERSION")) {
        println!("already up to date ({})", rel.version);
        return 0;
    }
    println!("downloading {}…", rel.version);
    let state: Shared = Arc::new(Mutex::new(State::Idle));
    match download(&rel, &state, &|| {}) {
        Ok(()) => {
            println!("updated to {}; it takes effect on the next start", rel.version);
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert_eq!(parse_version("v1.2.3-beta+5"), Some((1, 2, 3)));
        assert_eq!(parse_version("junk"), None);
    }

    #[test]
    fn sums_file_is_parsed() {
        let h = "a".repeat(64);
        let text = format!(
            "{h}  SnapForge.exe\n{}  *snapforge-cli.exe\nnot a line\n",
            "B".repeat(64)
        );
        let m = parse_sums(&text);
        assert_eq!(m.get("SnapForge.exe"), Some(&h));
        assert_eq!(
            m.get("snapforge-cli.exe"),
            Some(&"b".repeat(64)),
            "binary marker stripped, hash lower-cased"
        );
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn hash_of_a_file_matches_known_value() {
        let p = std::env::temp_dir().join(format!("sf-upd-{}.bin", std::process::id()));
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(
            sha256_file(&p).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        std::fs::remove_file(&p).unwrap();
    }

    #[test]
    fn swap_replaces_and_keeps_old() {
        let d = std::env::temp_dir().join(format!("sf-swap-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let (target, new) = (d.join("SnapForge.exe"), d.join("SnapForge.exe.download"));
        std::fs::write(&target, b"old").unwrap();
        std::fs::write(&new, b"new").unwrap();
        swap_in(&new, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert_eq!(std::fs::read(target.with_extension("exe.old")).unwrap(), b"old");
        assert!(!new.exists());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn failed_swap_rolls_back() {
        let d = std::env::temp_dir().join(format!("sf-swap2-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let target = d.join("SnapForge.exe");
        std::fs::write(&target, b"old").unwrap();
        assert!(swap_in(&d.join("missing.download"), &target).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"old", "original restored");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
