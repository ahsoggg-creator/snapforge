//! `plugin.toml` and the permissions a plugin asks for.
//!
//! ```toml
//! name = "Watermark"
//! version = "1.0.0"
//! author = "me"
//! description = "Adds a text mark to every capture"
//! main = "main.rhai"
//!
//! [permissions]
//! clipboard = true
//! ui = true
//! "fs.read" = ["%USERPROFILE%\\Pictures"]
//! "fs.write" = ["%USERPROFILE%\\Pictures\\marked"]
//! net = ["api.example.com"]
//! ```

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PermissionsDecl {
    pub clipboard: bool,
    pub ui: bool,
    #[serde(rename = "fs.read")]
    pub fs_read: Vec<String>,
    #[serde(rename = "fs.write")]
    pub fs_write: Vec<String>,
    pub net: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_main")]
    pub main: String,
    #[serde(default)]
    pub permissions: PermissionsDecl,
}

fn default_main() -> String {
    "main.rhai".to_owned()
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ManifestError {
    #[error("plugin.toml: {0}")]
    Parse(String),
    #[error("the name must be 1–64 characters")]
    BadName,
    #[error("the version must look like 1.2.3")]
    BadVersion,
    #[error("`main` must be a plain file name ending in .rhai")]
    BadMain,
    #[error("permission '{0}': {1}")]
    BadPermission(&'static str, String),
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest, ManifestError> {
        let m: Manifest = toml::from_str(text).map_err(|e| ManifestError::Parse(e.message().to_owned()))?;
        m.validate()?;
        Ok(m)
    }

    pub fn validate(&self) -> Result<(), ManifestError> {
        let n = self.name.trim();
        if n.is_empty() || n.chars().count() > 64 || n.chars().any(char::is_control) {
            return Err(ManifestError::BadName);
        }
        let parts: Vec<&str> = self.version.split('.').collect();
        if parts.len() < 2
            || parts.len() > 3
            || !parts
                .iter()
                .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        {
            return Err(ManifestError::BadVersion);
        }
        let main = Path::new(&self.main);
        if self.main.is_empty()
            || main.components().count() != 1
            || !self.main.to_ascii_lowercase().ends_with(".rhai")
            || self.main.contains(['/', '\\', ':'])
        {
            return Err(ManifestError::BadMain);
        }
        self.permissions.resolve().map(|_| ())
    }

    /// Folder-name-safe identifier derived from the name.
    pub fn id(&self) -> String {
        let s: String = self
            .name
            .trim()
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect();
        let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
        if s.is_empty() { "plugin".to_owned() } else { s }
    }
}

/// Permissions with environment variables expanded and values validated.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Permissions {
    pub clipboard: bool,
    pub ui: bool,
    pub fs_read: Vec<PathBuf>,
    pub fs_write: Vec<PathBuf>,
    /// Lower-case host names. A plugin may reach a host if it equals an entry or is a sub-domain of it.
    pub net: Vec<String>,
}

fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('%') {
            Some(j) if j > 0 => {
                match std::env::var(&after[..j]) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => {
                        out.push('%');
                        out.push_str(&after[..=j]);
                    }
                }
                rest = &after[j + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Lexical normalisation without touching the disk. `None` for relative paths and for any `..` component.
fn normalize_abs(p: &Path) -> Option<PathBuf> {
    if !p.is_absolute() {
        return None;
    }
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => return None,
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    Some(out)
}

fn is_root(p: &Path) -> bool {
    p.parent().is_none()
}

/// Resolves `p` through symlinks/junctions of its deepest existing ancestor, so a link inside an allowed folder
/// cannot lead outside of it.
pub fn resolve_real(p: &Path) -> Option<PathBuf> {
    let abs = normalize_abs(p)?;
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = abs.as_path();
    loop {
        if let Ok(real) = cur.canonicalize() {
            let mut out = real;
            for t in tail.iter().rev() {
                out.push(t);
            }
            return Some(out);
        }
        tail.push(cur.file_name()?.to_owned());
        cur = cur.parent()?;
    }
}

fn norm_case(p: &Path) -> String {
    let s = p.to_string_lossy().replace('/', "\\");
    s.trim_start_matches(r"\\?\").to_lowercase()
}

fn inside(dir: &Path, target: &Path) -> bool {
    let (d, t) = (norm_case(dir), norm_case(target));
    t == d || t.starts_with(&format!("{}\\", d.trim_end_matches('\\')))
}

impl PermissionsDecl {
    pub fn resolve(&self) -> Result<Permissions, ManifestError> {
        let dirs = |key: &'static str, list: &[String]| -> Result<Vec<PathBuf>, ManifestError> {
            list.iter()
                .map(|s| {
                    let p = normalize_abs(Path::new(&expand_env(s.trim()))).ok_or_else(|| {
                        ManifestError::BadPermission(key, format!("'{s}' must be an absolute folder path without '..'"))
                    })?;
                    if is_root(&p) {
                        return Err(ManifestError::BadPermission(
                            key,
                            format!("'{s}' is a drive root; name a folder"),
                        ));
                    }
                    Ok(p)
                })
                .collect()
        };
        let mut net = Vec::new();
        for h in &self.net {
            let host = h.trim().to_lowercase();
            let host = host
                .strip_prefix("https://")
                .or_else(|| host.strip_prefix("http://"))
                .unwrap_or(&host);
            let host = host
                .split('/')
                .next()
                .unwrap_or("")
                .split(':')
                .next()
                .unwrap_or("")
                .trim_start_matches("*.");
            if host.is_empty()
                || host == "*"
                || !host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
                || !host.contains('.') && host != "localhost"
            {
                return Err(ManifestError::BadPermission(
                    "net",
                    format!("'{h}' is not a specific host name"),
                ));
            }
            net.push(host.to_owned());
        }
        Ok(Permissions {
            clipboard: self.clipboard,
            ui: self.ui,
            fs_read: dirs("fs.read", &self.fs_read)?,
            fs_write: dirs("fs.write", &self.fs_write)?,
            net,
        })
    }
}

impl Permissions {
    pub fn is_empty(&self) -> bool {
        !self.clipboard && !self.ui && self.fs_read.is_empty() && self.fs_write.is_empty() && self.net.is_empty()
    }

    fn allowed_in(dirs: &[PathBuf], path: &Path) -> Option<PathBuf> {
        let real = resolve_real(path)?;
        dirs.iter()
            .filter_map(|d| resolve_real(d))
            .any(|d| inside(&d, &real))
            .then_some(real)
    }

    /// The real path if the plugin may read `path` (inside a `fs.read` or `fs.write` folder).
    pub fn check_read(&self, path: &Path) -> Option<PathBuf> {
        Self::allowed_in(&self.fs_read, path).or_else(|| Self::allowed_in(&self.fs_write, path))
    }

    /// The real path if the plugin may write `path` (inside a `fs.write` folder).
    pub fn check_write(&self, path: &Path) -> Option<PathBuf> {
        Self::allowed_in(&self.fs_write, path)
    }

    pub fn allows_host(&self, host: &str) -> bool {
        let h = host.trim().to_lowercase();
        self.net.iter().any(|d| h == *d || h.ends_with(&format!(".{d}")))
    }

    /// Stable fingerprint: a plugin must be re-approved when this changes.
    pub fn fingerprint(&self) -> String {
        let mut s = format!("c{}u{}", self.clipboard as u8, self.ui as u8);
        for (tag, list) in [
            ("r", self.fs_read.iter().map(|p| norm_case(p)).collect::<Vec<_>>()),
            ("w", self.fs_write.iter().map(|p| norm_case(p)).collect()),
            ("n", self.net.clone()),
        ] {
            let mut l = list;
            l.sort();
            s.push_str(&format!("|{tag}:{}", l.join(",")));
        }
        Sha256::digest(s.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
name = "Water Mark"
version = "1.2.0"
author = "me"
description = "marks"

[permissions]
clipboard = true
"fs.read" = ["C:\\Data\\in"]
"fs.write" = ["C:\\Data\\out"]
net = ["API.Example.com", "https://cdn.example.org/path"]
"#;

    #[test]
    fn a_full_manifest_parses() {
        let m = Manifest::parse(GOOD).unwrap();
        assert_eq!(m.main, "main.rhai");
        assert_eq!(m.id(), "water-mark");
        let p = m.permissions.resolve().unwrap();
        assert!(p.clipboard && !p.ui);
        assert_eq!(p.net, vec!["api.example.com", "cdn.example.org"]);
        assert_eq!(p.fs_write, vec![PathBuf::from(r"C:\Data\out")]);
    }

    #[test]
    fn a_minimal_manifest_has_no_permissions() {
        let m = Manifest::parse("name = \"Hello\"\nversion = \"0.1\"\n").unwrap();
        assert!(m.permissions.resolve().unwrap().is_empty());
    }

    #[test]
    fn invalid_manifests_are_rejected() {
        assert_eq!(
            Manifest::parse("name = \"\"\nversion = \"1.0\"").unwrap_err(),
            ManifestError::BadName
        );
        assert_eq!(
            Manifest::parse("name = \"x\"\nversion = \"one\"").unwrap_err(),
            ManifestError::BadVersion
        );
        assert_eq!(
            Manifest::parse("name = \"x\"\nversion = \"1.0\"\nmain = \"..\\\\evil.rhai\"").unwrap_err(),
            ManifestError::BadMain
        );
        assert_eq!(
            Manifest::parse("name = \"x\"\nversion = \"1.0\"\nmain = \"run.exe\"").unwrap_err(),
            ManifestError::BadMain
        );
        assert!(matches!(
            Manifest::parse("name = ").unwrap_err(),
            ManifestError::Parse(_)
        ));
    }

    #[test]
    fn too_broad_permissions_are_rejected() {
        let m = |extra: &str| format!("name = \"x\"\nversion = \"1.0\"\n[permissions]\n{extra}");
        for bad in [
            "net = [\"*\"]",
            "net = [\"com\"]",
            "net = [\"\"]",
            "\"fs.write\" = [\"C:\\\\\"]",
            "\"fs.read\" = [\"relative\\\\dir\"]",
            "\"fs.read\" = [\"C:\\\\a\\\\..\\\\b\"]",
        ] {
            assert!(
                matches!(Manifest::parse(&m(bad)), Err(ManifestError::BadPermission(..))),
                "{bad}"
            );
        }
    }

    #[test]
    fn environment_variables_are_expanded() {
        assert_eq!(expand_env("%SF_TEST_NOPE%\\x"), "%SF_TEST_NOPE%\\x");
        // SAFETY: a variable no other test reads.
        unsafe { std::env::set_var("SF_PLUGIN_TEST_DIR", "C:\\Base") };
        assert_eq!(expand_env("%SF_PLUGIN_TEST_DIR%\\out"), "C:\\Base\\out");
        assert_eq!(expand_env("100%"), "100%");
    }

    fn perms(read: &Path, write: &Path) -> Permissions {
        Permissions {
            fs_read: vec![read.to_path_buf()],
            fs_write: vec![write.to_path_buf()],
            ..Default::default()
        }
    }

    #[test]
    fn file_access_is_limited_to_the_declared_folders() {
        let root = std::env::temp_dir().join(format!("sf-perm-{}", std::process::id()));
        let (rd, wr, other) = (root.join("in"), root.join("out"), root.join("other"));
        for d in [&rd, &wr, &other] {
            std::fs::create_dir_all(d).unwrap();
        }
        let p = perms(&rd, &wr);
        assert!(p.check_read(&rd.join("a.png")).is_some());
        assert!(
            p.check_read(&wr.join("a.png")).is_some(),
            "write folders are readable too"
        );
        assert!(
            p.check_write(&wr.join("new").join("deep.png")).is_some(),
            "not-yet-existing paths inside are fine"
        );
        assert!(
            p.check_write(&rd.join("a.png")).is_none(),
            "read-only folder cannot be written"
        );
        assert!(p.check_read(&other.join("secret.txt")).is_none());
        assert!(p.check_write(&root.join("outside.txt")).is_none());
        // a sibling whose name merely starts with the allowed one
        std::fs::create_dir_all(root.join("out-evil")).unwrap();
        assert!(p.check_write(&root.join("out-evil").join("x")).is_none());
        // traversal is refused outright
        assert!(p.check_write(&wr.join("..").join("other").join("x")).is_none());
        assert!(p.check_read(Path::new("relative.txt")).is_none());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn paths_are_compared_case_insensitively() {
        let root = std::env::temp_dir().join(format!("sf-perm2-{}", std::process::id()));
        std::fs::create_dir_all(root.join("Out")).unwrap();
        let p = perms(&root.join("In"), &root.join("Out"));
        let upper = PathBuf::from(root.to_string_lossy().to_uppercase())
            .join("OUT")
            .join("f.png");
        assert!(p.check_write(&upper).is_some());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_junction_cannot_lead_out_of_the_allowed_folder() {
        let root = std::env::temp_dir().join(format!("sf-perm3-{}", std::process::id()));
        let (allowed, outside) = (root.join("allowed"), root.join("outside"));
        std::fs::create_dir_all(&allowed).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let link = allowed.join("link");
        // a directory junction needs no special rights on Windows
        let made = std::process::Command::new("cmd")
            .args([
                "/C",
                "mklink",
                "/J",
                &link.to_string_lossy(),
                &outside.to_string_lossy(),
            ])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if made {
            let p = perms(&allowed, &allowed);
            assert!(
                p.check_write(&link.join("x.txt")).is_none(),
                "junction target is outside"
            );
            assert!(p.check_write(&allowed.join("x.txt")).is_some());
        }
        let _ = std::process::Command::new("cmd")
            .args(["/C", "rmdir", &link.to_string_lossy()])
            .output();
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn hosts_match_exactly_or_as_subdomains() {
        let p = Permissions {
            net: vec!["example.com".into()],
            ..Default::default()
        };
        assert!(p.allows_host("example.com") && p.allows_host("API.Example.com"));
        assert!(!p.allows_host("evilexample.com") && !p.allows_host("example.com.evil.net") && !p.allows_host("com"));
    }

    #[test]
    fn the_fingerprint_changes_with_any_permission() {
        let a = Permissions::default();
        let b = Permissions {
            net: vec!["x.com".into()],
            ..Default::default()
        };
        let c = Permissions {
            clipboard: true,
            ..Default::default()
        };
        assert_ne!(a.fingerprint(), b.fingerprint());
        assert_ne!(a.fingerprint(), c.fingerprint());
        assert_eq!(a.fingerprint(), Permissions::default().fingerprint());
    }
}
