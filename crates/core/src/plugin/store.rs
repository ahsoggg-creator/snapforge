//! Installed plugins on disk: one folder per plugin under the plugins directory, plus a small `state.json`
//! (enabled flag and the fingerprint of the permissions the user approved).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::manifest::{Manifest, Permissions};
use super::package;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct State {
    enabled: bool,
    /// Fingerprint of the permissions that were shown to and approved by the user.
    approved: String,
}

#[derive(Clone, Debug)]
pub struct Installed {
    pub id: String,
    pub dir: PathBuf,
    pub manifest: Manifest,
    pub perms: Permissions,
    pub enabled: bool,
    /// The user approved exactly the permissions in the current `plugin.toml`.
    pub approved: bool,
}

impl Installed {
    pub fn runnable(&self) -> bool {
        self.enabled && self.approved
    }
}

/// Built-in starting points for "New plugin".
pub struct Template {
    pub id: &'static str,
    pub title: &'static str,
    pub manifest: &'static str,
    pub script: &'static str,
}

pub const TEMPLATES: [Template; 3] = [
    Template {
        id: "hello",
        title: "Hello",
        manifest: include_str!("../../../../plugins/hello/plugin.toml"),
        script: include_str!("../../../../plugins/hello/main.rhai"),
    },
    Template {
        id: "watermark",
        title: "Watermark",
        manifest: include_str!("../../../../plugins/watermark/plugin.toml"),
        script: include_str!("../../../../plugins/watermark/main.rhai"),
    },
    Template {
        id: "custom-upload",
        title: "Upload to my server",
        manifest: include_str!("../../../../plugins/custom-upload/plugin.toml"),
        script: include_str!("../../../../plugins/custom-upload/main.rhai"),
    },
];

pub struct Store {
    root: PathBuf,
}

fn read_state(dir: &Path) -> State {
    std::fs::read_to_string(dir.join("state.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn write_state(dir: &Path, s: &State) -> Result<(), String> {
    std::fs::write(
        dir.join("state.json"),
        serde_json::to_vec_pretty(s).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

impl Store {
    pub fn new(root: PathBuf) -> Store {
        Store { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn dir_of(&self, id: &str) -> Result<PathBuf, String> {
        if id.is_empty() || !id.chars().all(|c| c.is_alphanumeric() || c == '-') {
            return Err("bad plugin id".into());
        }
        Ok(self.root.join(id))
    }

    pub fn list(&self) -> Vec<Installed> {
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(&self.root) else {
            return out;
        };
        for e in rd.flatten() {
            let dir = e.path();
            if !dir.is_dir() {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(dir.join("plugin.toml")) else {
                continue;
            };
            let Ok(manifest) = Manifest::parse(&text) else { continue };
            let Ok(perms) = manifest.permissions.resolve() else {
                continue;
            };
            let st = read_state(&dir);
            let approved = !st.approved.is_empty() && st.approved == perms.fingerprint();
            out.push(Installed {
                id: dir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                dir,
                manifest,
                perms,
                enabled: st.enabled,
                approved,
            });
        }
        out.sort_by_key(|a| a.manifest.name.to_lowercase());
        out
    }

    pub fn get(&self, id: &str) -> Option<Installed> {
        self.list().into_iter().find(|p| p.id == id)
    }

    fn unique_id(&self, base: &str) -> String {
        if !self.root.join(base).exists() {
            return base.to_owned();
        }
        (2..)
            .map(|n| format!("{base}-{n}"))
            .find(|c| !self.root.join(c).exists())
            .unwrap_or_else(|| base.to_owned())
    }

    /// Installs an archive. `approved` is the answer to the permission dialog the UI showed for
    /// `package::read_manifest(archive)`; when false the plugin is installed disabled and unapproved.
    pub fn install(&self, archive: &Path, approved: bool) -> Result<String, String> {
        let manifest = package::read_manifest(archive)?;
        std::fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
        let id = self.unique_id(&manifest.id());
        let dir = self.root.join(&id);
        package::unpack(archive, &dir)?;
        let perms = manifest.permissions.resolve().map_err(|e| e.to_string())?;
        write_state(
            &dir,
            &State {
                enabled: approved,
                approved: if approved { perms.fingerprint() } else { String::new() },
            },
        )?;
        Ok(id)
    }

    /// Creates a plugin from a template (the user's own code, disabled until the permissions are approved).
    pub fn create(&self, t: &Template, approved: bool) -> Result<String, String> {
        std::fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
        let manifest = Manifest::parse(t.manifest).map_err(|e| e.to_string())?;
        let id = self.unique_id(&manifest.id());
        let dir = self.root.join(&id);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("plugin.toml"), t.manifest).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(&manifest.main), t.script).map_err(|e| e.to_string())?;
        let perms = manifest.permissions.resolve().map_err(|e| e.to_string())?;
        write_state(
            &dir,
            &State {
                enabled: approved,
                approved: if approved { perms.fingerprint() } else { String::new() },
            },
        )?;
        Ok(id)
    }

    pub fn set_enabled(&self, id: &str, on: bool) -> Result<(), String> {
        let dir = self.dir_of(id)?;
        let mut st = read_state(&dir);
        st.enabled = on;
        write_state(&dir, &st)
    }

    /// Records that the user approved the permissions currently in `plugin.toml` (and enables the plugin).
    pub fn approve(&self, id: &str) -> Result<(), String> {
        let p = self.get(id).ok_or("no such plugin")?;
        write_state(
            &p.dir,
            &State {
                enabled: true,
                approved: p.perms.fingerprint(),
            },
        )
    }

    pub fn remove(&self, id: &str) -> Result<(), String> {
        std::fs::remove_dir_all(self.dir_of(id)?).map_err(|e| e.to_string())
    }

    pub fn export(&self, id: &str, out: &Path) -> Result<(), String> {
        package::pack(&self.dir_of(id)?, out)
    }

    pub fn read_files(&self, id: &str) -> Result<(String, String), String> {
        let p = self.get(id).ok_or("no such plugin")?;
        let manifest = std::fs::read_to_string(p.dir.join("plugin.toml")).map_err(|e| e.to_string())?;
        let script = std::fs::read_to_string(p.dir.join(&p.manifest.main)).map_err(|e| e.to_string())?;
        Ok((manifest, script))
    }

    /// Saves edited sources. A change of the permissions revokes the approval (the plugin must be approved again).
    /// The script is saved even when `plugin.toml` is invalid; the manifest error is returned so the editor can show it.
    pub fn save_files(&self, id: &str, manifest_text: &str, script: &str) -> Result<(), String> {
        let dir = self.dir_of(id)?;
        let current = self.get(id).ok_or("no such plugin")?;
        if script.len() > 512 << 10 {
            return Err("the script is larger than 512 KB".into());
        }
        std::fs::write(dir.join(&current.manifest.main), script).map_err(|e| e.to_string())?;
        let m = Manifest::parse(manifest_text).map_err(|e| e.to_string())?;
        if m.main != current.manifest.main {
            return Err("the main script name cannot be changed here".into());
        }
        std::fs::write(dir.join("plugin.toml"), manifest_text).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sf-store-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn templates_are_valid_plugins() {
        for t in &TEMPLATES {
            let m = Manifest::parse(t.manifest).unwrap_or_else(|e| panic!("{}: {e}", t.id));
            m.permissions.resolve().unwrap();
            assert!(!t.script.is_empty());
        }
    }

    #[test]
    fn create_list_enable_remove() {
        let s = Store::new(tmp("crud"));
        let id = s.create(&TEMPLATES[0], false).unwrap();
        let p = s.get(&id).unwrap();
        assert!(
            !p.enabled && !p.approved && !p.runnable(),
            "a new plugin does nothing until approved"
        );
        s.approve(&id).unwrap();
        assert!(s.get(&id).unwrap().runnable());
        s.set_enabled(&id, false).unwrap();
        assert!(!s.get(&id).unwrap().runnable());
        assert_eq!(
            s.create(&TEMPLATES[0], false).unwrap(),
            format!("{id}-2"),
            "ids stay unique"
        );
        s.remove(&id).unwrap();
        assert!(s.get(&id).is_none());
        std::fs::remove_dir_all(s.root()).unwrap();
    }

    #[test]
    fn editing_the_permissions_revokes_the_approval() {
        let s = Store::new(tmp("revoke"));
        let id = s.create(&TEMPLATES[0], true).unwrap();
        assert!(s.get(&id).unwrap().runnable());
        let (manifest, script) = s.read_files(&id).unwrap();
        // editing only the script keeps the approval
        s.save_files(&id, &manifest, &format!("{script}\n// edit")).unwrap();
        assert!(s.get(&id).unwrap().runnable());
        // asking for the network does not
        let widened = format!("{manifest}\nnet = [\"evil.example.com\"]\n");
        s.save_files(&id, &widened, &script).unwrap();
        let p = s.get(&id).unwrap();
        assert!(p.enabled && !p.approved && !p.runnable());
        s.approve(&id).unwrap();
        assert!(s.get(&id).unwrap().runnable());
        std::fs::remove_dir_all(s.root()).unwrap();
    }

    #[test]
    fn invalid_manifest_edits_are_reported_and_not_written() {
        let s = Store::new(tmp("invalid"));
        let id = s.create(&TEMPLATES[0], true).unwrap();
        let (manifest, script) = s.read_files(&id).unwrap();
        assert!(s.save_files(&id, "name = ", &script).is_err());
        assert!(
            s.save_files(&id, &manifest.replace("main.rhai", "other.rhai"), &script)
                .is_err()
        );
        assert_eq!(
            s.read_files(&id).unwrap().0,
            manifest,
            "the manifest on disk is unchanged"
        );
        std::fs::remove_dir_all(s.root()).unwrap();
    }

    #[test]
    fn export_then_install_elsewhere() {
        let a = Store::new(tmp("exp-a"));
        let b = Store::new(tmp("exp-b"));
        let id = a.create(&TEMPLATES[2], true).unwrap();
        let archive = a.root().join("out.sfplugin");
        a.export(&id, &archive).unwrap();
        // an imported plugin starts disabled until the user approves its permissions
        let id2 = b.install(&archive, false).unwrap();
        let p = b.get(&id2).unwrap();
        assert_eq!(p.manifest.name, "Custom Upload");
        assert!(!p.runnable());
        assert_eq!(p.perms.net, vec!["upload.example.com"]);
        let id3 = b.install(&archive, true).unwrap();
        assert!(b.get(&id3).unwrap().runnable());
        std::fs::remove_dir_all(a.root()).unwrap();
        std::fs::remove_dir_all(b.root()).unwrap();
    }

    #[test]
    fn bad_ids_cannot_escape_the_plugins_folder() {
        let s = Store::new(tmp("ids"));
        for bad in ["", "..", "a/b", "a\\b", "C:"] {
            assert!(s.remove(bad).is_err(), "{bad}");
            assert!(s.set_enabled(bad, true).is_err());
        }
    }
}
