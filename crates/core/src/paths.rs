//! Locations of SnapForge's local data. `SNAPFORGE_DATA` overrides the root (tests, portable use).

use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("SNAPFORGE_DATA") {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("SnapForge")
}

pub fn settings_file() -> PathBuf {
    data_dir().join("settings.json")
}

pub fn logs_dir() -> PathBuf {
    data_dir().join("logs")
}

pub fn crashes_dir() -> PathBuf {
    logs_dir().join("crashes")
}

pub fn history_dir() -> PathBuf {
    data_dir().join("history")
}

pub fn models_dir() -> PathBuf {
    data_dir().join("models")
}

pub fn plugins_dir() -> PathBuf {
    data_dir().join("plugins")
}

pub fn default_pictures_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(|p| PathBuf::from(p).join("Pictures").join("SnapForge"))
        .unwrap_or_else(|| data_dir().join("shots"))
}
