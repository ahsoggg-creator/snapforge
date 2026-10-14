//! User settings (JSON in the data folder). Secrets are never stored here — see `secrets` (Credential Manager).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::hotkey::Profile;
use crate::naming;
use crate::paths;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    #[default]
    Ru,
    En,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    #[default]
    Png,
    Jpeg,
}

impl ImageFormat {
    pub fn ext(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub language: Lang,
    pub save_dir: PathBuf,
    pub name_template: String,
    pub format: ImageFormat,
    pub jpeg_quality: u8,
    pub cursor_in_shot: bool,
    pub delay_ms: u32,
    pub history_enabled: bool,
    /// Screenshots older than this many days are deleted; 0 keeps them forever.
    pub history_retention_days: u32,
    pub check_updates: bool,
    pub last_update_check: u64,
    pub autostart: bool,
    pub minimize_to_tray: bool,
    pub hotkeys_enabled: bool,
    pub profiles: Vec<Profile>,
    pub active_profile: usize,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            language: Lang::Ru,
            save_dir: paths::default_pictures_dir(),
            name_template: naming::DEFAULT_TEMPLATE.to_owned(),
            format: ImageFormat::Png,
            jpeg_quality: 90,
            cursor_in_shot: false,
            delay_ms: 0,
            history_enabled: true,
            history_retention_days: 30,
            check_updates: true,
            last_update_check: 0,
            autostart: false,
            minimize_to_tray: true,
            hotkeys_enabled: true,
            profiles: vec![Profile::default_profile()],
            active_profile: 0,
        }
    }
}

impl Settings {
    pub fn load() -> Settings {
        Settings::load_from(&paths::settings_file())
    }

    pub fn load_from(path: &Path) -> Settings {
        let Ok(text) = fs::read_to_string(path) else {
            return Settings::default();
        };
        match serde_json::from_str::<Settings>(&text) {
            Ok(mut s) => {
                s.sanitize();
                s
            }
            Err(e) => {
                crate::sf_warn!("settings.json is unreadable ({e}); using defaults, old file kept as .bad");
                let _ = fs::rename(path, path.with_extension("json.bad"));
                Settings::default()
            }
        }
    }

    /// Keeps invariants that hand-edited files can break.
    pub fn sanitize(&mut self) {
        if self.profiles.is_empty() {
            self.profiles.push(Profile::default_profile());
        }
        self.active_profile = self.active_profile.min(self.profiles.len() - 1);
        self.jpeg_quality = self.jpeg_quality.clamp(1, 100);
        self.delay_ms = self.delay_ms.min(60_000);
        if self.name_template.trim().is_empty() {
            self.name_template = naming::DEFAULT_TEMPLATE.to_owned();
        }
    }

    pub fn active_profile(&self) -> &Profile {
        &self.profiles[self.active_profile.min(self.profiles.len() - 1)]
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&paths::settings_file())
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?)?;
        fs::rename(&tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sf-settings-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d.join("settings.json")
    }

    #[test]
    fn roundtrip() {
        let p = tmp("rt");
        let s = Settings {
            language: Lang::En,
            delay_ms: 1500,
            ..Settings::default()
        };
        s.save_to(&p).unwrap();
        assert_eq!(Settings::load_from(&p), s);
    }

    #[test]
    fn missing_fields_take_defaults() {
        let p = tmp("partial");
        fs::write(&p, r#"{"language":"en"}"#).unwrap();
        let s = Settings::load_from(&p);
        assert_eq!(s.language, Lang::En);
        assert!(s.history_enabled && s.check_updates);
    }

    #[test]
    fn broken_file_is_kept_aside() {
        let p = tmp("bad");
        fs::write(&p, "{ not json").unwrap();
        assert_eq!(Settings::load_from(&p), Settings::default());
        assert!(p.with_extension("json.bad").exists());
    }

    #[test]
    fn sanitize_fixes_hand_edits() {
        let mut s = Settings::default();
        s.profiles.clear();
        s.active_profile = 9;
        s.jpeg_quality = 0;
        s.name_template = " ".into();
        s.sanitize();
        assert_eq!(s.profiles.len(), 1);
        assert_eq!(s.active_profile, 0);
        assert_eq!(s.jpeg_quality, 1);
        assert_eq!(s.name_template, naming::DEFAULT_TEMPLATE);
    }

    #[test]
    fn no_secret_like_keys_in_settings_json() {
        let j = serde_json::to_string(&Settings::default()).unwrap().to_lowercase();
        for bad in ["token", "password", "secret", "apikey", "api_key"] {
            assert!(!j.contains(bad), "settings must not contain '{bad}'");
        }
    }
}
