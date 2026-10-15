//! User settings (JSON in the data folder). Secrets are never stored here — see `secrets` (Credential Manager).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::hotkey::Profile;
use crate::naming;
use crate::paths;
use crate::post::PostSettings;
use crate::upload::UploadSettings;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    #[default]
    Ru,
    En,
}

/// Tools of the capture overlay (the bar at the top of the screen while you pick an area).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayTool {
    Region,
    Window,
    Screen,
    /// Select an area and copy the text recognised in it (SnapForge's own tool).
    Ocr,
    Rect,
    Ellipse,
    Line,
    Arrow,
    Pencil,
    Marker,
    Text,
    Step,
    Blur,
    Pixelate,
    /// Pick a drawn object to move it, resize it with the handles, recolour it or delete it.
    Select,
    /// Dims everything outside the dragged rectangle.
    Spotlight,
    /// Wipes the drawn objects the cursor passes over.
    Eraser,
    /// Takes the drawing colour from a pixel of the screen.
    Picker,
}

impl OverlayTool {
    pub const ALL: [OverlayTool; 18] = [
        OverlayTool::Region,
        OverlayTool::Window,
        OverlayTool::Screen,
        OverlayTool::Ocr,
        OverlayTool::Select,
        OverlayTool::Rect,
        OverlayTool::Ellipse,
        OverlayTool::Line,
        OverlayTool::Arrow,
        OverlayTool::Pencil,
        OverlayTool::Marker,
        OverlayTool::Text,
        OverlayTool::Step,
        OverlayTool::Blur,
        OverlayTool::Pixelate,
        OverlayTool::Spotlight,
        OverlayTool::Eraser,
        OverlayTool::Picker,
    ];

    pub fn default_key(self) -> &'static str {
        match self {
            OverlayTool::Region => "S",
            OverlayTool::Window => "W",
            OverlayTool::Screen => "F",
            OverlayTool::Ocr => "O",
            OverlayTool::Rect => "R",
            OverlayTool::Ellipse => "E",
            OverlayTool::Line => "L",
            OverlayTool::Arrow => "A",
            OverlayTool::Pencil => "P",
            OverlayTool::Marker => "M",
            OverlayTool::Text => "T",
            OverlayTool::Step => "N",
            OverlayTool::Blur => "B",
            OverlayTool::Pixelate => "X",
            OverlayTool::Select => "V",
            OverlayTool::Spotlight => "H",
            OverlayTool::Eraser => "D",
            OverlayTool::Picker => "I",
        }
    }

    /// Tools that choose *what to capture* (the others draw on the screen first).
    pub fn is_capture(self) -> bool {
        matches!(
            self,
            OverlayTool::Region | OverlayTool::Window | OverlayTool::Screen | OverlayTool::Ocr
        )
    }
}

/// What a single click (no drag) does with the area or window tool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayClick {
    /// Captures the whole monitor under the cursor.
    #[default]
    Monitor,
    /// Captures all monitors together.
    AllScreens,
    /// Captures the window under the cursor (the monitor when there is none).
    Window,
    /// A click does nothing: only a drag selects.
    Nothing,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolbarPos {
    #[default]
    Top,
    Bottom,
}

/// What plays when a screenshot is taken.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureSound {
    #[default]
    Off,
    /// A short camera-shutter click made by `shutter::wav` (no file needed).
    Click,
    /// The WAV file chosen in `Settings::capture_sound_file`.
    Custom,
}

pub const DEFAULT_OVERLAY_ACCENT: [u8; 4] = [96, 165, 250, 255];

/// The colours offered in the overlay's toolbar.
pub fn default_overlay_palette() -> Vec<[u8; 4]> {
    vec![
        [230, 40, 40, 255],
        [250, 190, 40, 255],
        [60, 190, 90, 255],
        [60, 140, 250, 255],
        [255, 255, 255, 255],
        [20, 20, 20, 255],
    ]
}

pub fn default_overlay_keys() -> Vec<(OverlayTool, String)> {
    OverlayTool::ALL
        .iter()
        .map(|t| (*t, t.default_key().to_owned()))
        .collect()
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
    pub post: PostSettings,
    pub uploads: UploadSettings,
    /// Single-key shortcuts of the capture overlay's tools (empty = none).
    pub overlay_keys: Vec<(OverlayTool, String)>,
    /// The tool the overlay starts with.
    pub overlay_start: OverlayTool,
    pub overlay_color: [u8; 4],
    pub overlay_width: f32,
    /// Start every capture with "hide sensitive text" switched on in the overlay.
    pub overlay_privacy: bool,
    /// Full-screen and monitor captures also open the overlay with its toolbar (draw, blur, hide faces first).
    pub overlay_for_screens: bool,
    /// Tools left out of the overlay's toolbar (their keys still work).
    pub overlay_hidden_tools: Vec<OverlayTool>,
    /// Colours of the toolbar (1 to 10).
    pub overlay_palette: Vec<[u8; 4]>,
    /// New rectangles and ellipses are filled.
    pub overlay_fill: bool,
    pub overlay_click: OverlayClick,
    pub overlay_toolbar: ToolbarPos,
    /// How much the not-selected part of the screen is darkened (0 to 90 %).
    pub overlay_dim: u8,
    /// The magnifier with the pixel colour next to the cursor.
    pub overlay_magnifier: bool,
    /// Thin lines across the whole screen through the cursor while you pick what to capture.
    pub overlay_crosshair: bool,
    /// Rule-of-thirds grid inside the selected area.
    pub overlay_thirds: bool,
    /// Colour of the selection frame and the active buttons.
    pub overlay_accent: [u8; 4],
    pub capture_sound: CaptureSound,
    pub capture_sound_file: Option<PathBuf>,
    /// Screen recording: frames per second (15, 24, 30 or 60) and whether the mouse cursor is drawn into the video.
    pub video_fps: u32,
    pub video_cursor: bool,
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
            post: PostSettings::default(),
            uploads: UploadSettings::default(),
            overlay_keys: default_overlay_keys(),
            overlay_start: OverlayTool::Region,
            overlay_color: [230, 40, 40, 255],
            overlay_width: 4.0,
            overlay_privacy: false,
            overlay_for_screens: true,
            overlay_hidden_tools: Vec::new(),
            overlay_palette: default_overlay_palette(),
            overlay_fill: false,
            overlay_click: OverlayClick::Monitor,
            overlay_toolbar: ToolbarPos::Top,
            overlay_dim: 60,
            overlay_magnifier: true,
            overlay_crosshair: false,
            overlay_thirds: false,
            overlay_accent: DEFAULT_OVERLAY_ACCENT,
            capture_sound: CaptureSound::Off,
            capture_sound_file: None,
            video_fps: 30,
            video_cursor: true,
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
        for p in &mut self.profiles {
            p.add_missing_actions();
        }
        self.active_profile = self.active_profile.min(self.profiles.len() - 1);
        self.video_fps = self.video_fps.clamp(5, 60);
        self.jpeg_quality = self.jpeg_quality.clamp(1, 100);
        self.delay_ms = self.delay_ms.min(60_000);
        self.post.sanitize();
        self.overlay_width = self.overlay_width.clamp(1.0, 24.0);
        self.overlay_dim = self.overlay_dim.min(90);
        self.overlay_accent[3] = 255;
        if self.capture_sound == CaptureSound::Custom && self.capture_sound_file.is_none() {
            self.capture_sound = CaptureSound::Click;
        }
        self.overlay_palette.truncate(10);
        if self.overlay_palette.is_empty() {
            self.overlay_palette = default_overlay_palette();
        }
        for c in &mut self.overlay_palette {
            c[3] = 255;
        }
        self.overlay_hidden_tools.dedup();
        // at least one capture tool must stay on the bar, or the overlay cannot be used with the mouse
        if OverlayTool::ALL
            .iter()
            .filter(|t| t.is_capture())
            .all(|t| self.overlay_hidden_tools.contains(t))
        {
            self.overlay_hidden_tools.retain(|t| !t.is_capture());
        }
        // every tool has an entry; unknown key names fall back to the default and duplicates are dropped
        let mut keys: Vec<(OverlayTool, String)> = Vec::new();
        for t in OverlayTool::ALL {
            let wanted = self
                .overlay_keys
                .iter()
                .find(|(x, _)| *x == t)
                .map(|(_, k)| k.trim().to_owned());
            let key = match wanted {
                Some(k) if k.is_empty() => String::new(),
                Some(k) if crate::hotkey::parse_single_key(&k).is_some() => k,
                _ => t.default_key().to_owned(),
            };
            let taken = !key.is_empty()
                && keys
                    .iter()
                    .any(|(_, k)| crate::hotkey::parse_single_key(k) == crate::hotkey::parse_single_key(&key));
            keys.push((t, if taken { String::new() } else { key }));
        }
        self.overlay_keys = keys;
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

    #[test]
    fn the_new_look_and_sound_options_have_safe_defaults() {
        let s: Settings = serde_json::from_str(r#"{"overlay_dim":40}"#).unwrap();
        assert_eq!(s.overlay_dim, 40);
        assert!(!s.overlay_crosshair && !s.overlay_thirds);
        assert_eq!(s.overlay_accent, DEFAULT_OVERLAY_ACCENT);
        assert_eq!(s.capture_sound, CaptureSound::Off);
        let mut s = Settings {
            capture_sound: CaptureSound::Custom,
            overlay_accent: [1, 2, 3, 0],
            ..Settings::default()
        };
        s.sanitize();
        assert_eq!(s.capture_sound, CaptureSound::Click, "a custom sound needs a file");
        assert_eq!(s.overlay_accent, [1, 2, 3, 255]);
    }

    #[test]
    fn a_profile_from_an_older_version_gets_the_new_actions() {
        let mut s = Settings::default();
        s.profiles[0]
            .bindings
            .retain(|b| b.action != crate::hotkey::HotAction::RecordVideo);
        s.video_fps = 500;
        s.sanitize();
        let actions: Vec<_> = s.profiles[0].bindings.iter().map(|b| b.action).collect();
        for a in crate::hotkey::HotAction::ALL {
            assert_eq!(actions.iter().filter(|x| **x == a).count(), 1, "{a:?}");
        }
        assert_eq!(s.video_fps, 60);
    }

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
    fn uploads_are_off_in_a_fresh_config() {
        let s = Settings::default();
        assert!(!s.uploads.enabled && s.uploads.destinations.is_empty());
        assert!(s.post.presets.iter().all(|p| !p.upload));
    }

    #[test]
    fn overlay_keys_are_completed_validated_and_deduplicated() {
        let mut s = Settings::default();
        assert_eq!(s.overlay_keys.len(), OverlayTool::ALL.len());
        s.overlay_keys = vec![
            (OverlayTool::Rect, "q".into()),
            (OverlayTool::Arrow, "Q".into()),        // same key as Rect: dropped
            (OverlayTool::Text, "NoSuchKey".into()), // unknown: default
            (OverlayTool::Blur, String::new()),      // deliberately unbound
        ];
        s.sanitize();
        let key = |t| s.overlay_keys.iter().find(|(x, _)| *x == t).unwrap().1.clone();
        assert_eq!(key(OverlayTool::Rect), "q");
        assert_eq!(key(OverlayTool::Arrow), "");
        assert_eq!(key(OverlayTool::Text), "T");
        assert_eq!(key(OverlayTool::Blur), "");
        assert_eq!(key(OverlayTool::Region), "S", "missing tools get their default");
        assert_eq!(s.overlay_keys.len(), OverlayTool::ALL.len());
    }

    #[test]
    fn overlay_options_are_kept_in_range() {
        let mut s = Settings::default();
        assert_eq!(s.overlay_click, OverlayClick::Monitor);
        s.overlay_dim = 200;
        s.overlay_palette = Vec::new();
        s.overlay_hidden_tools = OverlayTool::ALL.iter().copied().filter(|t| t.is_capture()).collect();
        s.sanitize();
        assert_eq!(s.overlay_dim, 90);
        assert_eq!(s.overlay_palette, default_overlay_palette());
        assert!(
            s.overlay_hidden_tools.is_empty(),
            "the capture tools cannot all be hidden"
        );
        s.overlay_palette = vec![[1, 2, 3, 0]; 30];
        s.sanitize();
        assert_eq!(s.overlay_palette.len(), 10);
        assert_eq!(s.overlay_palette[0][3], 255);
    }

    #[test]
    fn old_settings_files_without_overlay_fields_still_load() {
        let s: Settings = serde_json::from_str(r#"{"language":"en"}"#).unwrap();
        assert_eq!(s.overlay_start, OverlayTool::Region);
        assert_eq!(s.overlay_keys.len(), OverlayTool::ALL.len());
    }

    #[test]
    fn no_secret_like_keys_in_settings_json() {
        let j = serde_json::to_string(&Settings::default()).unwrap().to_lowercase();
        for bad in ["token", "password", "secret", "apikey", "api_key"] {
            assert!(!j.contains(bad), "settings must not contain '{bad}'");
        }
    }
}
