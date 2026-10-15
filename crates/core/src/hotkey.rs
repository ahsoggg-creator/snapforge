//! Global hotkeys: parsing/formatting of combinations, profiles, conflict detection and (on Windows) the
//! `RegisterHotKey` message-loop thread.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const MOD_ALT: u8 = 1;
pub const MOD_CTRL: u8 = 2;
pub const MOD_SHIFT: u8 = 4;
pub const MOD_WIN: u8 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Combo {
    pub mods: u8,
    /// Windows virtual-key code.
    pub vk: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ComboError {
    #[error("empty combination")]
    Empty,
    #[error("unknown key '{0}'")]
    UnknownKey(String),
    #[error("a key is required")]
    NoKey,
    #[error("more than one key in the combination")]
    TwoKeys,
    #[error("letters, digits and symbols need at least one modifier (Ctrl, Alt, Shift, Win)")]
    NoModifier,
}

const NAMED: &[(&str, u32)] = &[
    ("printscreen", 0x2C),
    ("prtsc", 0x2C),
    ("space", 0x20),
    ("enter", 0x0D),
    ("tab", 0x09),
    ("esc", 0x1B),
    ("escape", 0x1B),
    ("insert", 0x2D),
    ("delete", 0x2E),
    ("home", 0x24),
    ("end", 0x23),
    ("pageup", 0x21),
    ("pagedown", 0x22),
    ("up", 0x26),
    ("down", 0x28),
    ("left", 0x25),
    ("right", 0x27),
    ("pause", 0x13),
    ("backspace", 0x08),
];

/// A single key (no modifiers) by name: `A`, `7`, `F5`, `PrintScreen`, `Space`…
pub fn parse_single_key(name: &str) -> Option<u32> {
    key_from_name(name.trim())
}

fn key_from_name(name: &str) -> Option<u32> {
    let n = name.to_ascii_lowercase();
    if n.len() == 1 {
        let c = n.as_bytes()[0];
        if c.is_ascii_lowercase() {
            return Some(c.to_ascii_uppercase() as u32);
        }
        if c.is_ascii_digit() {
            return Some(c as u32);
        }
    }
    if let Some(rest) = n.strip_prefix('f')
        && let Ok(k) = rest.parse::<u32>()
        && (1..=24).contains(&k)
    {
        return Some(0x70 + k - 1);
    }
    NAMED.iter().find(|(k, _)| *k == n).map(|(_, v)| *v)
}

pub fn key_label(vk: u32) -> String {
    key_name(vk)
}

fn key_name(vk: u32) -> String {
    match vk {
        0x41..=0x5A | 0x30..=0x39 => char::from_u32(vk).unwrap_or('?').to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x70 + 1),
        0x2C => "PrintScreen".to_owned(),
        _ => NAMED
            .iter()
            .find(|(_, v)| *v == vk)
            .map(|(k, _)| {
                let mut c = k.chars();
                c.next()
                    .map(|f| f.to_ascii_uppercase().to_string() + c.as_str())
                    .unwrap_or_default()
            })
            .unwrap_or_else(|| format!("0x{vk:02X}")),
    }
}

impl Combo {
    /// Function keys and PrintScreen may be used alone; anything else needs a modifier.
    pub fn validate(&self) -> Result<(), ComboError> {
        let standalone = matches!(self.vk, 0x70..=0x87 | 0x2C);
        if self.mods == 0 && !standalone {
            return Err(ComboError::NoModifier);
        }
        Ok(())
    }
}

impl FromStr for Combo {
    type Err = ComboError;

    fn from_str(s: &str) -> Result<Combo, ComboError> {
        if s.trim().is_empty() {
            return Err(ComboError::Empty);
        }
        let mut mods = 0u8;
        let mut vk = None;
        for part in s.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => mods |= MOD_CTRL,
                "alt" => mods |= MOD_ALT,
                "shift" => mods |= MOD_SHIFT,
                "win" | "super" | "meta" => mods |= MOD_WIN,
                _ => {
                    let k = key_from_name(part).ok_or_else(|| ComboError::UnknownKey(part.to_owned()))?;
                    if vk.replace(k).is_some() {
                        return Err(ComboError::TwoKeys);
                    }
                }
            }
        }
        let combo = Combo {
            mods,
            vk: vk.ok_or(ComboError::NoKey)?,
        };
        combo.validate()?;
        Ok(combo)
    }
}

impl fmt::Display for Combo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (bit, name) in [
            (MOD_CTRL, "Ctrl"),
            (MOD_ALT, "Alt"),
            (MOD_SHIFT, "Shift"),
            (MOD_WIN, "Win"),
        ] {
            if self.mods & bit != 0 {
                write!(f, "{name}+")?;
            }
        }
        f.write_str(&key_name(self.vk))
    }
}

impl Serialize for Combo {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Combo {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Combo, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// What a hotkey triggers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotAction {
    CaptureRegion,
    CaptureWindow,
    CaptureScreen,
    /// The monitor under the cursor.
    CaptureMonitor,
}

impl HotAction {
    pub const ALL: [HotAction; 4] = [
        HotAction::CaptureRegion,
        HotAction::CaptureWindow,
        HotAction::CaptureScreen,
        HotAction::CaptureMonitor,
    ];
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub action: HotAction,
    pub combo: Combo,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub bindings: Vec<Binding>,
}

impl Profile {
    pub fn default_profile() -> Profile {
        let b = |action, s: &str| Binding {
            action,
            combo: s.parse().expect("built-in combo"),
        };
        Profile {
            name: "Default".to_owned(),
            bindings: vec![
                b(HotAction::CaptureRegion, "Ctrl+Shift+S"),
                b(HotAction::CaptureWindow, "Ctrl+Shift+W"),
                b(HotAction::CaptureScreen, "Ctrl+Shift+F"),
                b(HotAction::CaptureMonitor, "Ctrl+Shift+M"),
            ],
        }
    }

    /// Pairs of binding indices that use the same combination.
    pub fn internal_conflicts(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for i in 0..self.bindings.len() {
            for j in i + 1..self.bindings.len() {
                if self.bindings[i].combo == self.bindings[j].combo {
                    out.push((i, j));
                }
            }
        }
        out
    }
}

#[cfg(windows)]
pub use win::{HotkeyManager, Registered, poll_pressed_combo, probe_system};

#[cfg(windows)]
mod win {
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::sync::{Arc, Mutex};

    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        HOT_KEY_MODIFIERS, MOD_ALT as W_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT as W_SHIFT, MOD_WIN as W_WIN,
        RegisterHotKey, UnregisterHotKey,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WM_APP, WM_HOTKEY,
    };

    use super::*;

    const WM_RECONFIGURE: u32 = WM_APP + 1;
    const WM_STOP: u32 = WM_APP + 2;

    fn win_mods(mods: u8) -> HOT_KEY_MODIFIERS {
        let mut m = MOD_NOREPEAT;
        if mods & MOD_ALT != 0 {
            m |= W_ALT;
        }
        if mods & MOD_CTRL != 0 {
            m |= MOD_CONTROL;
        }
        if mods & MOD_SHIFT != 0 {
            m |= W_SHIFT;
        }
        if mods & MOD_WIN != 0 {
            m |= W_WIN;
        }
        m
    }

    /// The key combination held down right now, if any key besides the modifiers is down. Unlike window key events
    /// this also sees `PrintScreen`, which is how the settings screen can record any combination.
    pub fn poll_pressed_combo() -> Option<Combo> {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        let down = |vk: u32| -> bool {
            // SAFETY: a plain read of the keyboard state.
            unsafe { GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0 }
        };
        let mut mods = 0u8;
        if down(0x11) {
            mods |= MOD_CTRL;
        }
        if down(0x12) {
            mods |= MOD_ALT;
        }
        if down(0x10) {
            mods |= MOD_SHIFT;
        }
        if down(0x5B) || down(0x5C) {
            mods |= MOD_WIN;
        }
        // 0x08..=0xFE minus the modifiers (Shift/Ctrl/Alt, both Win keys, left/right variants) and the mouse buttons
        (0x08u32..=0xFE)
            .filter(|vk| !matches!(vk, 0x10..=0x12 | 0x5B | 0x5C | 0xA0..=0xA5))
            .find(|vk| down(*vk))
            .map(|vk| Combo { mods, vk })
    }

    /// Asks Windows whether `combo` is free by registering and immediately releasing it.
    /// `Err` means another program (or the system) already owns it.
    pub fn probe_system(combo: &Combo) -> Result<(), String> {
        const PROBE_ID: i32 = 0x5F00;
        // SAFETY: registering for the current thread (no window) and releasing straight away.
        unsafe {
            RegisterHotKey(None, PROBE_ID, win_mods(combo.mods), combo.vk).map_err(|e| e.message())?;
            let _ = UnregisterHotKey(None, PROBE_ID);
        }
        Ok(())
    }

    /// Result of registering one binding.
    #[derive(Clone, Debug)]
    pub struct Registered {
        pub action: HotAction,
        pub combo: Combo,
        pub error: Option<String>,
    }

    /// Owns a thread with a message loop that receives `WM_HOTKEY`.
    pub struct HotkeyManager {
        thread_id: u32,
        pending: Arc<Mutex<Vec<Binding>>>,
        results: Receiver<Vec<Registered>>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl HotkeyManager {
        /// `on_hotkey` runs on the hotkey thread; keep it short (send into a channel, request a repaint).
        pub fn spawn(on_hotkey: impl Fn(HotAction) + Send + 'static) -> HotkeyManager {
            let pending: Arc<Mutex<Vec<Binding>>> = Arc::default();
            let (res_tx, results) = channel();
            let (id_tx, id_rx) = channel();
            let p = pending.clone();
            let handle = std::thread::spawn(move || run(p, res_tx, id_tx, on_hotkey));
            let thread_id = id_rx.recv().unwrap_or(0);
            HotkeyManager {
                thread_id,
                pending,
                results,
                handle: Some(handle),
            }
        }

        /// Replaces all registered hotkeys; returns what Windows accepted.
        pub fn set_bindings(&self, bindings: Vec<Binding>) -> Vec<Registered> {
            *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = bindings;
            // SAFETY: plain thread message to our own message-loop thread.
            let posted = unsafe { PostThreadMessageW(self.thread_id, WM_RECONFIGURE, WPARAM(0), LPARAM(0)) };
            if posted.is_err() {
                return Vec::new();
            }
            self.results
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap_or_default()
        }
    }

    impl Drop for HotkeyManager {
        fn drop(&mut self) {
            // SAFETY: as above.
            unsafe {
                let _ = PostThreadMessageW(self.thread_id, WM_STOP, WPARAM(0), LPARAM(0));
            }
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
        }
    }

    fn run(
        pending: Arc<Mutex<Vec<Binding>>>,
        res_tx: Sender<Vec<Registered>>,
        id_tx: Sender<u32>,
        on_hotkey: impl Fn(HotAction),
    ) {
        // SAFETY: standard message loop on a thread we own; the queue is created by PeekMessage before the id
        // is published, so posted messages cannot be lost.
        unsafe {
            let mut msg = MSG::default();
            let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
            let _ = id_tx.send(GetCurrentThreadId());
            let mut active: Vec<(i32, HotAction)> = Vec::new();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                match msg.message {
                    WM_HOTKEY => {
                        let id = msg.wParam.0 as i32;
                        if let Some((_, a)) = active.iter().find(|(i, _)| *i == id) {
                            on_hotkey(*a);
                        }
                    }
                    WM_RECONFIGURE => {
                        for (id, _) in active.drain(..) {
                            let _ = UnregisterHotKey(None, id);
                        }
                        let wanted = pending.lock().unwrap_or_else(|e| e.into_inner()).clone();
                        let mut report = Vec::new();
                        for (n, b) in wanted.iter().enumerate() {
                            let id = 0x6000 + n as i32;
                            let r = RegisterHotKey(None, id, win_mods(b.combo.mods), b.combo.vk);
                            let error = match r {
                                Ok(()) => {
                                    active.push((id, b.action));
                                    None
                                }
                                Err(e) => Some(e.message()),
                            };
                            report.push(Registered {
                                action: b.action,
                                combo: b.combo,
                                error,
                            });
                        }
                        let _ = res_tx.send(report);
                    }
                    WM_STOP => break,
                    _ => {}
                }
            }
            for (id, _) in active {
                let _ = UnregisterHotKey(None, id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_format_roundtrip() {
        let c: Combo = "ctrl + shift+s".parse().unwrap();
        assert_eq!(
            c,
            Combo {
                mods: MOD_CTRL | MOD_SHIFT,
                vk: 0x53
            }
        );
        assert_eq!(c.to_string(), "Ctrl+Shift+S");
        assert_eq!("Win+Alt+F12".parse::<Combo>().unwrap().to_string(), "Alt+Win+F12");
        assert_eq!("PrintScreen".parse::<Combo>().unwrap().vk, 0x2C);
        assert_eq!("alt+PageDown".parse::<Combo>().unwrap().to_string(), "Alt+Pagedown");
    }

    #[test]
    fn parse_errors() {
        assert_eq!("".parse::<Combo>(), Err(ComboError::Empty));
        assert_eq!("Ctrl+Shift".parse::<Combo>(), Err(ComboError::NoKey));
        assert_eq!("Ctrl+A+B".parse::<Combo>(), Err(ComboError::TwoKeys));
        assert_eq!("A".parse::<Combo>(), Err(ComboError::NoModifier));
        assert!(matches!("Ctrl+Banana".parse::<Combo>(), Err(ComboError::UnknownKey(_))));
        assert!("F5".parse::<Combo>().is_ok(), "function keys may be used alone");
    }

    #[test]
    fn serde_uses_text() {
        let b = Binding {
            action: HotAction::CaptureRegion,
            combo: "Ctrl+Alt+R".parse().unwrap(),
        };
        let j = serde_json::to_string(&b).unwrap();
        assert_eq!(j, r#"{"action":"capture_region","combo":"Ctrl+Alt+R"}"#);
        assert_eq!(serde_json::from_str::<Binding>(&j).unwrap(), b);
        assert!(serde_json::from_str::<Binding>(r#"{"action":"capture_region","combo":"nope"}"#).is_err());
    }

    #[test]
    fn default_profile_has_no_conflicts() {
        assert!(Profile::default_profile().internal_conflicts().is_empty());
    }

    #[test]
    fn duplicates_are_detected() {
        let mut p = Profile::default_profile();
        p.bindings[2].combo = p.bindings[0].combo;
        assert_eq!(p.internal_conflicts(), vec![(0, 2)]);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "needs a real interactive desktop"]
    fn manager_registers_and_reports_conflicts() {
        let m = HotkeyManager::spawn(|_| {});
        // Ctrl+Alt+Shift+F23 is a combination nobody uses
        let combo: Combo = "Ctrl+Alt+Shift+F23".parse().unwrap();
        let b = vec![
            Binding {
                action: HotAction::CaptureRegion,
                combo,
            },
            Binding {
                action: HotAction::CaptureWindow,
                combo,
            },
        ];
        let r = m.set_bindings(b);
        assert_eq!(r.len(), 2);
        assert!(r[0].error.is_none(), "first registration succeeds: {:?}", r[0].error);
        assert!(r[1].error.is_some(), "the same combination cannot be registered twice");
        // after reconfiguring, the combination is free again
        let r = m.set_bindings(vec![Binding {
            action: HotAction::CaptureScreen,
            combo,
        }]);
        assert!(r[0].error.is_none());
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "needs a real interactive desktop"]
    fn probe_detects_taken_combination() {
        let combo: Combo = "Ctrl+Alt+Shift+F22".parse().unwrap();
        let m = HotkeyManager::spawn(|_| {});
        assert!(probe_system(&combo).is_ok());
        assert!(
            m.set_bindings(vec![Binding {
                action: HotAction::CaptureRegion,
                combo
            }])[0]
                .error
                .is_none()
        );
        assert!(probe_system(&combo).is_err(), "held by the manager's thread");
    }
}
