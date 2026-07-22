//! Global hotkey detection for TalaX dictation.
//!
//! Uses `rdev` to listen for keyboard events system-wide, matching against
//! a configurable key combination. Supports push-to-talk (held) and toggle
//! (tap on/off) modes.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors that can occur during hotkey setup or listening.
#[derive(Debug, thiserror::Error)]
pub enum HotkeyError {
    #[error("listen failed: {0}")]
    ListenFailed(String),
    #[error("invalid config: {0}")]
    InvalidConfig(String),
}

// ---------------------------------------------------------------------------
// Key
// ---------------------------------------------------------------------------

/// A platform-independent, serializable representation of a keyboard key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Key {
    Ctrl,
    Shift,
    Alt,
    Meta,
    Space,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    Letter(char),
    Number(u8),
}

impl Key {
    /// Convert from an `rdev::Key` into our serializable `Key`.
    ///
    /// Returns `None` for keys we don't model.
    pub fn from_rdev(k: &rdev::Key) -> Option<Self> {
        use rdev::Key as R;
        match k {
            R::ControlLeft | R::ControlRight => Some(Key::Ctrl),
            R::ShiftLeft | R::ShiftRight => Some(Key::Shift),
            R::Alt | R::AltGr => Some(Key::Alt),
            R::MetaLeft | R::MetaRight => Some(Key::Meta),
            R::Space => Some(Key::Space),
            R::F1 => Some(Key::F1),
            R::F2 => Some(Key::F2),
            R::F3 => Some(Key::F3),
            R::F4 => Some(Key::F4),
            R::F5 => Some(Key::F5),
            R::F6 => Some(Key::F6),
            R::F7 => Some(Key::F7),
            R::F8 => Some(Key::F8),
            R::F9 => Some(Key::F9),
            R::F10 => Some(Key::F10),
            R::F11 => Some(Key::F11),
            R::F12 => Some(Key::F12),
            R::KeyA => Some(Key::Letter('a')),
            R::KeyB => Some(Key::Letter('b')),
            R::KeyC => Some(Key::Letter('c')),
            R::KeyD => Some(Key::Letter('d')),
            R::KeyE => Some(Key::Letter('e')),
            R::KeyF => Some(Key::Letter('f')),
            R::KeyG => Some(Key::Letter('g')),
            R::KeyH => Some(Key::Letter('h')),
            R::KeyI => Some(Key::Letter('i')),
            R::KeyJ => Some(Key::Letter('j')),
            R::KeyK => Some(Key::Letter('k')),
            R::KeyL => Some(Key::Letter('l')),
            R::KeyM => Some(Key::Letter('m')),
            R::KeyN => Some(Key::Letter('n')),
            R::KeyO => Some(Key::Letter('o')),
            R::KeyP => Some(Key::Letter('p')),
            R::KeyQ => Some(Key::Letter('q')),
            R::KeyR => Some(Key::Letter('r')),
            R::KeyS => Some(Key::Letter('s')),
            R::KeyT => Some(Key::Letter('t')),
            R::KeyU => Some(Key::Letter('u')),
            R::KeyV => Some(Key::Letter('v')),
            R::KeyW => Some(Key::Letter('w')),
            R::KeyX => Some(Key::Letter('x')),
            R::KeyY => Some(Key::Letter('y')),
            R::KeyZ => Some(Key::Letter('z')),
            R::Num0 => Some(Key::Number(0)),
            R::Num1 => Some(Key::Number(1)),
            R::Num2 => Some(Key::Number(2)),
            R::Num3 => Some(Key::Number(3)),
            R::Num4 => Some(Key::Number(4)),
            R::Num5 => Some(Key::Number(5)),
            R::Num6 => Some(Key::Number(6)),
            R::Num7 => Some(Key::Number(7)),
            R::Num8 => Some(Key::Number(8)),
            R::Num9 => Some(Key::Number(9)),
            _ => None,
        }
    }

    /// Convert our `Key` to the canonical `rdev::Key` variant.
    ///
    /// For modifiers that have left/right variants, we pick the left one.
    pub fn to_rdev(&self) -> rdev::Key {
        use rdev::Key as R;
        match self {
            Key::Ctrl => R::ControlLeft,
            Key::Shift => R::ShiftLeft,
            Key::Alt => R::Alt,
            Key::Meta => R::MetaLeft,
            Key::Space => R::Space,
            Key::F1 => R::F1,
            Key::F2 => R::F2,
            Key::F3 => R::F3,
            Key::F4 => R::F4,
            Key::F5 => R::F5,
            Key::F6 => R::F6,
            Key::F7 => R::F7,
            Key::F8 => R::F8,
            Key::F9 => R::F9,
            Key::F10 => R::F10,
            Key::F11 => R::F11,
            Key::F12 => R::F12,
            Key::Letter(c) => match c.to_ascii_lowercase() {
                'a' => R::KeyA,
                'b' => R::KeyB,
                'c' => R::KeyC,
                'd' => R::KeyD,
                'e' => R::KeyE,
                'f' => R::KeyF,
                'g' => R::KeyG,
                'h' => R::KeyH,
                'i' => R::KeyI,
                'j' => R::KeyJ,
                'k' => R::KeyK,
                'l' => R::KeyL,
                'm' => R::KeyM,
                'n' => R::KeyN,
                'o' => R::KeyO,
                'p' => R::KeyP,
                'q' => R::KeyQ,
                'r' => R::KeyR,
                's' => R::KeyS,
                't' => R::KeyT,
                'u' => R::KeyU,
                'v' => R::KeyV,
                'w' => R::KeyW,
                'x' => R::KeyX,
                'y' => R::KeyY,
                'z' => R::KeyZ,
                _ => R::Unknown(0),
            },
            Key::Number(n) => match n {
                0 => R::Num0,
                1 => R::Num1,
                2 => R::Num2,
                3 => R::Num3,
                4 => R::Num4,
                5 => R::Num5,
                6 => R::Num6,
                7 => R::Num7,
                8 => R::Num8,
                9 => R::Num9,
                _ => R::Unknown(0),
            },
        }
    }

    /// Parse a single key token from a hotkey string (e.g. "Ctrl", "Space", "A", "F5", "7").
    fn from_token(s: &str) -> Result<Self, HotkeyError> {
        let lower = s.trim().to_lowercase();
        match lower.as_str() {
            "ctrl" | "control" => Ok(Key::Ctrl),
            "shift" => Ok(Key::Shift),
            "alt" => Ok(Key::Alt),
            "meta" | "super" | "win" | "cmd" | "command" => Ok(Key::Meta),
            "space" => Ok(Key::Space),
            "f1" => Ok(Key::F1),
            "f2" => Ok(Key::F2),
            "f3" => Ok(Key::F3),
            "f4" => Ok(Key::F4),
            "f5" => Ok(Key::F5),
            "f6" => Ok(Key::F6),
            "f7" => Ok(Key::F7),
            "f8" => Ok(Key::F8),
            "f9" => Ok(Key::F9),
            "f10" => Ok(Key::F10),
            "f11" => Ok(Key::F11),
            "f12" => Ok(Key::F12),
            other => {
                // Single letter
                if other.len() == 1 {
                    let ch = other.chars().next().unwrap();
                    if ch.is_ascii_alphabetic() {
                        return Ok(Key::Letter(ch));
                    }
                    if ch.is_ascii_digit() {
                        return Ok(Key::Number(ch.to_digit(10).unwrap() as u8));
                    }
                }
                Err(HotkeyError::InvalidConfig(format!(
                    "unrecognized key: '{s}'"
                )))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// HotkeyMode / HotkeyConfig / HotkeyEvent
// ---------------------------------------------------------------------------

/// How the hotkey behaves when activated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HotkeyMode {
    /// Hold to record, release to stop.
    PushToTalk,
    /// Tap once to start, tap again to stop.
    Toggle,
}

/// Configuration for a global hotkey combination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeyConfig {
    /// The keys that must be held together (order-independent).
    pub keys: Vec<Key>,
    /// Activation behaviour.
    pub mode: HotkeyMode,
}

/// Events emitted by the hotkey listener.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    /// The hotkey combination was fully pressed (all keys held).
    Pressed,
    /// The hotkey combination was released (at least one key lifted).
    Released,
}

// ---------------------------------------------------------------------------
// HotkeyHandle
// ---------------------------------------------------------------------------

/// Handle returned from [`HotkeyListener::start`]. Dropping or stopping it
/// suppresses callbacks; the OS listener remains detached until `rdev` exits.
pub struct HotkeyHandle {
    stop_flag: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl HotkeyHandle {
    fn signal_and_detach(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        drop(self.thread.take());
    }

    /// Suppress future callbacks without waiting on `rdev`'s blocking loop.
    pub fn stop(mut self) {
        self.signal_and_detach();
    }
}

impl Drop for HotkeyHandle {
    fn drop(&mut self) {
        self.signal_and_detach();
    }
}

// ---------------------------------------------------------------------------
// HotkeyListener
// ---------------------------------------------------------------------------

/// Listens for a global key combination and fires callbacks.
pub struct HotkeyListener {
    config: HotkeyConfig,
}

impl HotkeyListener {
    pub fn new(config: HotkeyConfig) -> Self {
        Self { config }
    }

    /// Validate the configuration without starting the listener.
    pub fn validate(&self) -> Result<(), HotkeyError> {
        if self.config.keys.is_empty() {
            return Err(HotkeyError::InvalidConfig(
                "hotkey must contain at least one key".into(),
            ));
        }
        Ok(())
    }

    /// Start the listener on a background thread.
    ///
    /// The `callback` is invoked with [`HotkeyEvent::Pressed`] when the full
    /// combination is detected, and [`HotkeyEvent::Released`] when any key in
    /// the combination is released.
    ///
    /// In [`HotkeyMode::Toggle`] mode, each activation alternates between
    /// `Pressed` and `Released`.
    pub fn start<F>(self, callback: F) -> Result<HotkeyHandle, HotkeyError>
    where
        F: Fn(HotkeyEvent) + Send + 'static,
    {
        self.validate()?;

        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop_flag);

        let required: HashSet<Key> = self.config.keys.iter().cloned().collect();
        let mode = self.config.mode;

        let handle = thread::spawn(move || {
            let mut held: HashSet<Key> = HashSet::new();
            let mut was_active = false;
            let mut toggled_on = false;

            let cb = move |event: rdev::Event| {
                if stop_clone.load(Ordering::SeqCst) {
                    return;
                }

                match event.event_type {
                    rdev::EventType::KeyPress(rdev_key) => {
                        if let Some(key) = Key::from_rdev(&rdev_key) {
                            held.insert(key);
                        }

                        let all_held = required.iter().all(|k| held.contains(k));

                        if all_held && !was_active {
                            was_active = true;
                            match mode {
                                HotkeyMode::PushToTalk => {
                                    callback(HotkeyEvent::Pressed);
                                }
                                HotkeyMode::Toggle => {
                                    if toggled_on {
                                        toggled_on = false;
                                        callback(HotkeyEvent::Released);
                                    } else {
                                        toggled_on = true;
                                        callback(HotkeyEvent::Pressed);
                                    }
                                }
                            }
                        }
                    }
                    rdev::EventType::KeyRelease(rdev_key) => {
                        if let Some(key) = Key::from_rdev(&rdev_key) {
                            held.remove(&key);
                        }

                        let all_held = required.iter().all(|k| held.contains(k));

                        if !all_held && was_active {
                            was_active = false;
                            if mode == HotkeyMode::PushToTalk {
                                callback(HotkeyEvent::Released);
                            }
                        }
                    }
                    _ => {}
                }
            };

            // rdev::listen has no shutdown API and blocks until an OS error or
            // process exit. The stop flag makes a detached listener dormant.
            if let Err(e) = rdev::listen(cb) {
                tracing::error!("rdev listen error: {:?}", e);
            }
        });

        Ok(HotkeyHandle {
            stop_flag,
            thread: Some(handle),
        })
    }
}

// ---------------------------------------------------------------------------
// parse_hotkey
// ---------------------------------------------------------------------------

/// Parse a human-readable hotkey string into a [`HotkeyConfig`].
///
/// Format: `Key1+Key2+...` (case-insensitive, order-independent).
/// The mode defaults to [`HotkeyMode::PushToTalk`].
///
/// # Examples
/// ```
/// # use talax_engine::hotkey::parse_hotkey;
/// let cfg = parse_hotkey("Ctrl+Shift+Space").unwrap();
/// assert_eq!(cfg.keys.len(), 3);
/// ```
pub fn parse_hotkey(s: &str) -> Result<HotkeyConfig, HotkeyError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(HotkeyError::InvalidConfig(
            "hotkey string cannot be empty".into(),
        ));
    }

    let keys: Vec<Key> = s
        .split('+')
        .map(Key::from_token)
        .collect::<Result<Vec<_>, _>>()?;

    if keys.is_empty() {
        return Err(HotkeyError::InvalidConfig(
            "hotkey must contain at least one key".into(),
        ));
    }

    // Check for duplicates.
    let unique: HashSet<&Key> = keys.iter().collect();
    if unique.len() != keys.len() {
        return Err(HotkeyError::InvalidConfig(
            "hotkey contains duplicate keys".into(),
        ));
    }

    Ok(HotkeyConfig {
        keys,
        mode: HotkeyMode::PushToTalk,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- parse_hotkey valid inputs -----------------------------------------

    #[test]
    fn parse_ctrl_shift_space() {
        let cfg = parse_hotkey("Ctrl+Shift+Space").unwrap();
        assert_eq!(cfg.keys, vec![Key::Ctrl, Key::Shift, Key::Space]);
        assert_eq!(cfg.mode, HotkeyMode::PushToTalk);
    }

    #[test]
    fn parse_single_f5() {
        let cfg = parse_hotkey("F5").unwrap();
        assert_eq!(cfg.keys, vec![Key::F5]);
    }

    #[test]
    fn parse_alt_z() {
        let cfg = parse_hotkey("Alt+Z").unwrap();
        assert_eq!(cfg.keys, vec![Key::Alt, Key::Letter('z')]);
    }

    #[test]
    fn parse_case_insensitive() {
        let cfg = parse_hotkey("ctrl+SHIFT+space").unwrap();
        assert_eq!(cfg.keys, vec![Key::Ctrl, Key::Shift, Key::Space]);
    }

    #[test]
    fn parse_meta_aliases() {
        for alias in &["Meta+A", "Super+A", "Win+A", "Cmd+A", "Command+A"] {
            let cfg = parse_hotkey(alias).unwrap();
            assert_eq!(cfg.keys[0], Key::Meta, "alias '{alias}' should map to Meta");
        }
    }

    #[test]
    fn parse_control_alias() {
        let cfg = parse_hotkey("Control+X").unwrap();
        assert_eq!(cfg.keys, vec![Key::Ctrl, Key::Letter('x')]);
    }

    #[test]
    fn parse_number_key() {
        let cfg = parse_hotkey("Ctrl+5").unwrap();
        assert_eq!(cfg.keys, vec![Key::Ctrl, Key::Number(5)]);
    }

    #[test]
    fn parse_all_f_keys() {
        for n in 1..=12 {
            let s = format!("F{n}");
            let cfg = parse_hotkey(&s).unwrap();
            assert_eq!(cfg.keys.len(), 1);
        }
    }

    #[test]
    fn parse_with_whitespace() {
        let cfg = parse_hotkey("  Ctrl + Shift + A  ").unwrap();
        assert_eq!(cfg.keys, vec![Key::Ctrl, Key::Shift, Key::Letter('a')]);
    }

    // -- parse_hotkey invalid inputs --------------------------------------

    #[test]
    fn parse_empty_string() {
        assert!(parse_hotkey("").is_err());
    }

    #[test]
    fn parse_whitespace_only() {
        assert!(parse_hotkey("   ").is_err());
    }

    #[test]
    fn parse_unrecognized_key() {
        let err = parse_hotkey("Ctrl+Banana").unwrap_err();
        assert!(err.to_string().contains("unrecognized key"));
    }

    #[test]
    fn parse_duplicate_keys() {
        let err = parse_hotkey("Ctrl+Ctrl+A").unwrap_err();
        assert!(err.to_string().contains("duplicate"));
    }

    // -- Key roundtrip rdev conversions -----------------------------------

    #[test]
    fn roundtrip_modifiers() {
        for key in &[Key::Ctrl, Key::Shift, Key::Alt, Key::Meta] {
            let rdev_key = key.to_rdev();
            let back = Key::from_rdev(&rdev_key).expect("should roundtrip");
            assert_eq!(&back, key);
        }
    }

    #[test]
    fn roundtrip_function_keys() {
        let fkeys = [
            Key::F1,
            Key::F2,
            Key::F3,
            Key::F4,
            Key::F5,
            Key::F6,
            Key::F7,
            Key::F8,
            Key::F9,
            Key::F10,
            Key::F11,
            Key::F12,
        ];
        for key in &fkeys {
            let rdev_key = key.to_rdev();
            let back = Key::from_rdev(&rdev_key).expect("should roundtrip");
            assert_eq!(&back, key);
        }
    }

    #[test]
    fn roundtrip_letters() {
        for ch in 'a'..='z' {
            let key = Key::Letter(ch);
            let rdev_key = key.to_rdev();
            let back = Key::from_rdev(&rdev_key).expect("should roundtrip");
            assert_eq!(back, Key::Letter(ch));
        }
    }

    #[test]
    fn roundtrip_numbers() {
        for n in 0..=9u8 {
            let key = Key::Number(n);
            let rdev_key = key.to_rdev();
            let back = Key::from_rdev(&rdev_key).expect("should roundtrip");
            assert_eq!(back, Key::Number(n));
        }
    }

    #[test]
    fn roundtrip_space() {
        let rdev_key = Key::Space.to_rdev();
        let back = Key::from_rdev(&rdev_key).unwrap();
        assert_eq!(back, Key::Space);
    }

    #[test]
    fn uppercase_letter_normalises() {
        // Key::Letter stores lowercase; to_rdev then from_rdev should preserve.
        let key = Key::Letter('a');
        let rdev_key = key.to_rdev();
        let back = Key::from_rdev(&rdev_key).unwrap();
        assert_eq!(back, Key::Letter('a'));
    }

    #[test]
    fn from_rdev_unknown_returns_none() {
        // An unmapped rdev key should return None.
        assert!(Key::from_rdev(&rdev::Key::Unknown(0xFFFF)).is_none());
    }

    // -- Serialization roundtrip ------------------------------------------

    #[test]
    fn config_serde_roundtrip() {
        let cfg = HotkeyConfig {
            keys: vec![Key::Ctrl, Key::Shift, Key::Space],
            mode: HotkeyMode::Toggle,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: HotkeyConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, back);
    }

    // -- Listener validation (no display needed) --------------------------

    #[test]
    fn listener_rejects_empty_keys() {
        let listener = HotkeyListener::new(HotkeyConfig {
            keys: vec![],
            mode: HotkeyMode::PushToTalk,
        });
        assert!(listener.validate().is_err());
    }

    #[test]
    fn listener_accepts_valid_config() {
        let listener = HotkeyListener::new(HotkeyConfig {
            keys: vec![Key::F5],
            mode: HotkeyMode::PushToTalk,
        });
        assert!(listener.validate().is_ok());
    }

    #[test]
    fn stop_does_not_wait_for_a_blocked_listener() {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let observed_flag = Arc::clone(&stop_flag);
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let worker = thread::spawn(move || {
            let _ = release_rx.recv();
        });
        let handle = HotkeyHandle {
            stop_flag,
            thread: Some(worker),
        };

        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let stopper = thread::spawn(move || {
            handle.stop();
            let _ = done_tx.send(());
        });

        let stopped = done_rx.recv_timeout(std::time::Duration::from_secs(1));
        let _ = release_tx.send(());
        let _ = stopper.join();

        assert!(stopped.is_ok(), "stop waited for the listener thread");
        assert!(observed_flag.load(Ordering::SeqCst));
    }
}
