//! Windows integration: install locations, shortcuts, the "Apps & features" entry, autostart, elevation.
//! Everything is per-user (HKCU, %LOCALAPPDATA%): no administrator rights are needed.

use std::path::{Path, PathBuf};

use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, IPersistFile,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW,
    RegDeleteTreeW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::Win32::UI::Shell::{IShellLinkW, SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify, ShellExecuteW, ShellLink};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{Interface, PCWSTR, w};

pub const APP_NAME: &str = "SnapForge";
pub const REPO_URL: &str = "https://github.com/ahsoggg-creator/snapforge";
const UNINSTALL_PARENT: &str = "Software\Microsoft\Windows\CurrentVersion\Uninstall";
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

/// Sub-key name of the "Apps & features" entry (a test overrides it so a real installation is never touched).
fn uninstall_name() -> String {
    std::env::var("SNAPFORGE_UNINSTALL_NAME").unwrap_or_else(|_| APP_NAME.to_owned())
}

fn uninstall_key() -> String {
    format!("{UNINSTALL_PARENT}\{}", uninstall_name())
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(PathBuf::from)
}

/// `%LOCALAPPDATA%\Programs\SnapForge` (override with `SNAPFORGE_INSTALL_DIR` for tests).
pub fn install_dir() -> Option<PathBuf> {
    env_dir("SNAPFORGE_INSTALL_DIR").or_else(|| env_dir("LOCALAPPDATA").map(|d| d.join("Programs").join(APP_NAME)))
}

pub fn installed_exe() -> Option<PathBuf> {
    install_dir().map(|d| d.join("SnapForge.exe"))
}

fn start_menu_dir() -> Option<PathBuf> {
    env_dir("SNAPFORGE_START_MENU")
        .or_else(|| env_dir("APPDATA").map(|d| d.join("Microsoft\\Windows\\Start Menu\\Programs")))
}

fn desktop_dir() -> Option<PathBuf> {
    env_dir("USERPROFILE").map(|d| d.join("Desktop"))
}

/// True when this process is the installed copy.
pub fn is_installed_copy() -> bool {
    match (std::env::current_exe().ok(), installed_exe()) {
        (Some(a), Some(b)) => same_file(&a, &b),
        _ => false,
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// The exe is the installer when its file name contains "setup" or `--install` was passed.
pub fn is_setup_launch() -> bool {
    if std::env::args().any(|a| a == "--install") {
        return true;
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()))
        .is_some_and(|n| n.contains("setup"))
}

// ---------------------------------------------------------------- registry

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: the handle was opened by us and is closed exactly once.
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn create_key(path: &str) -> Result<Key, String> {
    let mut h = HKEY::default();
    let p = wide(path);
    // SAFETY: valid NUL-terminated path and out pointer.
    let r = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(p.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut h,
            None,
        )
    };
    if r.is_err() { Err(format!("registry: cannot create HKCU\\{path} ({r:?})")) } else { Ok(Key(h)) }
}

fn set_str(k: &Key, name: &str, value: &str) -> Result<(), String> {
    let n = wide(name);
    let v: Vec<u8> = wide(value).iter().flat_map(|c| c.to_le_bytes()).collect();
    // SAFETY: buffers outlive the call.
    let r = unsafe { RegSetValueExW(k.0, PCWSTR(n.as_ptr()), None, REG_SZ, Some(&v)) };
    if r.is_err() { Err(format!("registry: cannot set {name} ({r:?})")) } else { Ok(()) }
}

fn set_dword(k: &Key, name: &str, value: u32) -> Result<(), String> {
    let n = wide(name);
    // SAFETY: buffers outlive the call.
    let r = unsafe { RegSetValueExW(k.0, PCWSTR(n.as_ptr()), None, REG_DWORD, Some(&value.to_le_bytes())) };
    if r.is_err() { Err(format!("registry: cannot set {name} ({r:?})")) } else { Ok(()) }
}

fn open_key(path: &str, write: bool) -> Option<Key> {
    let mut h = HKEY::default();
    let p = wide(path);
    // SAFETY: valid NUL-terminated path and out pointer.
    let r = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(p.as_ptr()),
            None,
            if write { KEY_WRITE } else { KEY_READ },
            &mut h,
        )
    };
    r.is_ok().then_some(Key(h))
}

fn get_str(path: &str, name: &str) -> Option<String> {
    let k = open_key(path, false)?;
    let n = wide(name);
    let mut len = 0u32;
    // SAFETY: first call queries the size, second fills a buffer of that size.
    unsafe {
        if RegQueryValueExW(k.0, PCWSTR(n.as_ptr()), None, None, None, Some(&mut len)).is_err() || len == 0 {
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        RegQueryValueExW(k.0, PCWSTR(n.as_ptr()), None, None, Some(buf.as_mut_ptr()), Some(&mut len)).ok().ok()?;
        let u: Vec<u16> = buf.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        Some(String::from_utf16_lossy(&u).trim_end_matches('\0').to_owned())
    }
}

// ---------------------------------------------------------------- autostart

fn run_value_name() -> String {
    // tests and demo runs use their own value so they never touch the real autostart entry
    std::env::var("SNAPFORGE_RUN_VALUE").unwrap_or_else(|_| APP_NAME.to_owned())
}

pub fn autostart_enabled() -> bool {
    get_str(RUN_KEY, &run_value_name()).is_some()
}

pub fn set_autostart(on: bool) -> Result<(), String> {
    let name = run_value_name();
    if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        set_str(&create_key(RUN_KEY)?, &name, &format!("\"{}\" --minimized", exe.display()))
    } else if let Some(k) = open_key(RUN_KEY, true) {
        let n = wide(&name);
        // SAFETY: valid value name.
        unsafe {
            let _ = RegDeleteValueW(k.0, PCWSTR(n.as_ptr()));
        }
        Ok(())
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------- shortcuts

fn create_shortcut(lnk: &Path, target: &Path, args: &str, desc: &str) -> Result<(), String> {
    if let Some(d) = lnk.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    // SAFETY: standard COM usage on this thread; all strings are NUL-terminated and outlive the calls.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.message())?;
        let t = wide(&target.to_string_lossy());
        link.SetPath(PCWSTR(t.as_ptr())).map_err(|e| e.message())?;
        let a = wide(args);
        link.SetArguments(PCWSTR(a.as_ptr())).map_err(|e| e.message())?;
        let d = wide(desc);
        link.SetDescription(PCWSTR(d.as_ptr())).map_err(|e| e.message())?;
        if let Some(dir) = target.parent() {
            let wd = wide(&dir.to_string_lossy());
            let _ = link.SetWorkingDirectory(PCWSTR(wd.as_ptr()));
        }
        link.SetIconLocation(PCWSTR(t.as_ptr()), 0).map_err(|e| e.message())?;
        let file: IPersistFile = link.cast().map_err(|e| e.message())?;
        let l = wide(&lnk.to_string_lossy());
        file.Save(PCWSTR(l.as_ptr()), true).map_err(|e| e.message())?;
    }
    Ok(())
}

// ---------------------------------------------------------------- install / uninstall

#[derive(Clone, Copy, Debug)]
pub struct InstallOptions {
    pub desktop_shortcut: bool,
}

/// Copies the running exe to the install folder and registers it. Returns the installed exe path.
pub fn install(opts: InstallOptions) -> Result<PathBuf, String> {
    let dir = install_dir().ok_or("cannot find %LOCALAPPDATA%")?;
    let src = std::env::current_exe().map_err(|e| e.to_string())?;
    let dst = dir.join("SnapForge.exe");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    if !same_file(&src, &dst) {
        // a running copy cannot be overwritten, but it can be renamed out of the way
        if dst.exists() {
            let old = dir.join("SnapForge.exe.old");
            let _ = std::fs::remove_file(&old);
            std::fs::rename(&dst, &old).map_err(|e| format!("the installed copy is in use: {e}"))?;
        }
        std::fs::copy(&src, &dst).map_err(|e| format!("copy failed: {e}"))?;
    }

    let k = create_key(&uninstall_key())?;
    let exe = dst.to_string_lossy().into_owned();
    set_str(&k, "DisplayName", APP_NAME)?;
    set_str(&k, "DisplayVersion", env!("CARGO_PKG_VERSION"))?;
    set_str(&k, "Publisher", "ahsoggg")?;
    set_str(&k, "InstallLocation", &dir.to_string_lossy())?;
    set_str(&k, "DisplayIcon", &exe)?;
    set_str(&k, "UninstallString", &format!("\"{exe}\" --uninstall"))?;
    set_str(&k, "URLInfoAbout", REPO_URL)?;
    set_dword(&k, "NoModify", 1)?;
    set_dword(&k, "NoRepair", 1)?;
    let size_kb = std::fs::metadata(&dst).map(|m| (m.len() / 1024) as u32).unwrap_or(0);
    set_dword(&k, "EstimatedSize", size_kb)?;

    let menu = start_menu_dir().ok_or("cannot find the Start menu folder")?;
    create_shortcut(&menu.join("SnapForge.lnk"), &dst, "", "SnapForge")?;
    if opts.desktop_shortcut {
        if let Some(desk) = desktop_dir() {
            create_shortcut(&desk.join("SnapForge.lnk"), &dst, "", "SnapForge")?;
        }
    }
    // SAFETY: no parameters; refreshes shell icons.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    Ok(dst)
}

/// Removes shortcuts, registry entries and (after this process exits) the install folder.
/// With `remove_data` also the settings/history folder.
pub fn uninstall(remove_data: bool) -> Result<(), String> {
    if let Some(menu) = start_menu_dir() {
        let _ = std::fs::remove_file(menu.join("SnapForge.lnk"));
    }
    if let Some(desk) = desktop_dir() {
        let _ = std::fs::remove_file(desk.join("SnapForge.lnk"));
    }
    let _ = set_autostart(false);
    if let Some(parent) = open_key("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall", true) {
        // SAFETY: deletes only our own subkey.
        unsafe {
            let _ = RegDeleteTreeW(parent.0, w!("SnapForge"));
        }
    }
    let mut targets: Vec<PathBuf> = install_dir().into_iter().collect();
    if remove_data {
        targets.push(snapforge_core::paths::data_dir());
    }
    // the running exe lives in the install folder: let a detached shell delete it after we exit
    let cmds: Vec<String> = targets.iter().map(|d| format!("rmdir /s /q \"{}\"", d.display())).collect();
    if !cmds.is_empty() {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("cmd")
            .raw_arg(format!("/C ping -n 3 127.0.0.1 >nul & {}", cmds.join(" & ")))
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Runs `exe` through the UAC prompt (`runas`). The current process is not elevated and stays that way.
pub fn run_elevated(exe: &Path, args: &str) -> Result<(), String> {
    let (e, a) = (wide(&exe.to_string_lossy()), wide(args));
    // SAFETY: NUL-terminated strings outlive the call; no window or directory.
    let h = unsafe { ShellExecuteW(None, w!("runas"), PCWSTR(e.as_ptr()), PCWSTR(a.as_ptr()), PCWSTR::null(), SW_SHOWNORMAL) };
    if h.0 as isize > 32 { Ok(()) } else { Err("the UAC prompt was declined".into()) }
}

pub fn open_path(path: &Path) {
    let _ = std::process::Command::new("explorer").arg(path).spawn();
}

pub fn reveal(path: &Path) {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("explorer")
        .raw_arg(format!("/select,\"{}\"", path.display()))
        .spawn();
}

pub fn open_url(url: &str) {
    let _ = std::process::Command::new("explorer").arg(url).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_uninstall_roundtrip_in_sandbox_dirs() {
        let root = std::env::temp_dir().join(format!("sf-inst-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // SAFETY: tests in this module run one at a time on a dedicated set of variables.
        unsafe {
            std::env::set_var("SNAPFORGE_INSTALL_DIR", root.join("prog"));
            std::env::set_var("SNAPFORGE_START_MENU", root.join("menu"));
            std::env::set_var("SNAPFORGE_RUN_VALUE", "SnapForgeTest");
            std::env::set_var("SNAPFORGE_UNINSTALL_NAME", "SnapForgeTest");
        }
        let exe = install(InstallOptions { desktop_shortcut: false }).expect("install");
        assert!(exe.exists());
        assert!(root.join("menu").join("SnapForge.lnk").exists(), "Start menu shortcut");
        assert_eq!(
            get_str(&uninstall_key(), "DisplayVersion").as_deref(),
            Some(env!("CARGO_PKG_VERSION")),
            "Apps & features entry"
        );
        assert!(get_str(&uninstall_key(), "UninstallString").unwrap().ends_with("--uninstall"));
        // reinstall over an existing copy works
        install(InstallOptions { desktop_shortcut: false }).expect("reinstall");

        set_autostart(true).unwrap();
        assert!(autostart_enabled());
        set_autostart(false).unwrap();
        assert!(!autostart_enabled());

        // the registry/shortcut part of the real uninstall (the folder delete is scheduled after exit)
        uninstall(false).expect("uninstall");
        assert!(get_str(&uninstall_key(), "DisplayName").is_none());
        assert!(!root.join("menu").join("SnapForge.lnk").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
