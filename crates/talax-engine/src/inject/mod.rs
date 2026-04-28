use std::thread;
use std::time::Duration;

use arboard::Clipboard;
use enigo::{Enigo, Keyboard, Settings};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors that can occur during text injection.
#[derive(Debug, thiserror::Error)]
pub enum InjectionError {
    #[error("clipboard access failed: {0}")]
    ClipboardAccess(String),

    #[error("keystroke simulation failed: {0}")]
    KeystrokeSimulation(String),

    #[error("operation timed out: {0}")]
    Timeout(String),
}

// ---------------------------------------------------------------------------
// InjectionMode
// ---------------------------------------------------------------------------

/// How injected text reaches the target application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InjectionMode {
    /// Copy text to clipboard, then simulate Ctrl+V (Cmd+V on macOS).
    Clipboard,
    /// Simulate individual keystrokes (slower but broader compatibility).
    TypeOut,
    /// Copy text to clipboard without pasting (safe fallback).
    ClipboardOnly,
}

impl Default for InjectionMode {
    fn default() -> Self {
        Self::Clipboard
    }
}

// ---------------------------------------------------------------------------
// InjectionConfig
// ---------------------------------------------------------------------------

/// Configuration for the text injector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InjectionConfig {
    /// Injection strategy.
    #[serde(default)]
    pub mode: InjectionMode,

    /// Delay in milliseconds between keystrokes in `TypeOut` mode.
    #[serde(default = "default_type_delay_ms")]
    pub type_delay_ms: u64,

    /// Whether to save and restore the original clipboard content after
    /// injecting via `Clipboard` mode.
    #[serde(default = "default_restore_clipboard")]
    pub restore_clipboard: bool,
}

fn default_type_delay_ms() -> u64 {
    12
}

fn default_restore_clipboard() -> bool {
    true
}

impl Default for InjectionConfig {
    fn default() -> Self {
        Self {
            mode: InjectionMode::default(),
            type_delay_ms: default_type_delay_ms(),
            restore_clipboard: default_restore_clipboard(),
        }
    }
}

// ---------------------------------------------------------------------------
// TextInjector
// ---------------------------------------------------------------------------

/// Injects text into the currently focused application.
pub struct TextInjector {
    config: InjectionConfig,
}

impl TextInjector {
    /// Create a new injector with the given configuration.
    pub fn new(config: InjectionConfig) -> Self {
        Self { config }
    }

    /// Inject `text` into the focused application using the configured mode.
    pub fn inject(&self, text: &str) -> Result<(), InjectionError> {
        match self.config.mode {
            InjectionMode::Clipboard => self.inject_clipboard(text),
            InjectionMode::TypeOut => self.inject_typeout(text),
            InjectionMode::ClipboardOnly => self.set_clipboard(text),
        }
    }

    /// Copy `text` to the system clipboard.
    pub fn set_clipboard(&self, text: &str) -> Result<(), InjectionError> {
        let mut cb =
            Clipboard::new().map_err(|e| InjectionError::ClipboardAccess(e.to_string()))?;
        cb.set_text(text)
            .map_err(|e| InjectionError::ClipboardAccess(e.to_string()))?;
        Ok(())
    }

    /// Read the current text from the system clipboard.
    pub fn get_clipboard(&self) -> Result<String, InjectionError> {
        let mut cb =
            Clipboard::new().map_err(|e| InjectionError::ClipboardAccess(e.to_string()))?;
        cb.get_text()
            .map_err(|e| InjectionError::ClipboardAccess(e.to_string()))
    }

    // -- private helpers ----------------------------------------------------

    /// Clipboard mode: save clipboard -> set text -> Ctrl/Cmd+V -> restore.
    fn inject_clipboard(&self, text: &str) -> Result<(), InjectionError> {
        let saved = if self.config.restore_clipboard {
            // Best-effort save; clipboard may be empty or contain non-text.
            self.get_clipboard().ok()
        } else {
            None
        };

        self.set_clipboard(text)?;

        // Simulate paste keystroke.
        self.simulate_paste()?;

        // Small delay so the target app processes the paste event.
        thread::sleep(Duration::from_millis(50));

        // Restore original clipboard content.
        if let Some(original) = saved {
            // Best-effort restore; don't propagate errors here.
            let _ = self.set_clipboard(&original);
        }

        Ok(())
    }

    /// Simulate Ctrl+V (or Cmd+V on macOS).
    fn simulate_paste(&self) -> Result<(), InjectionError> {
        let mut enigo = Enigo::new(&Settings::default())
            .map_err(|e| InjectionError::KeystrokeSimulation(e.to_string()))?;

        let modifier = if cfg!(target_os = "macos") {
            enigo::Key::Meta
        } else {
            enigo::Key::Control
        };

        enigo
            .key(modifier, enigo::Direction::Press)
            .map_err(|e| InjectionError::KeystrokeSimulation(e.to_string()))?;
        enigo
            .key(enigo::Key::Unicode('v'), enigo::Direction::Click)
            .map_err(|e| InjectionError::KeystrokeSimulation(e.to_string()))?;
        enigo
            .key(modifier, enigo::Direction::Release)
            .map_err(|e| InjectionError::KeystrokeSimulation(e.to_string()))?;

        Ok(())
    }

    /// TypeOut mode: type each character with a small inter-key delay.
    fn inject_typeout(&self, text: &str) -> Result<(), InjectionError> {
        let mut enigo = Enigo::new(&Settings::default())
            .map_err(|e| InjectionError::KeystrokeSimulation(e.to_string()))?;

        let delay = Duration::from_millis(self.config.type_delay_ms);

        for ch in text.chars() {
            enigo
                .text(&ch.to_string())
                .map_err(|e| InjectionError::KeystrokeSimulation(e.to_string()))?;

            if !delay.is_zero() {
                thread::sleep(delay);
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_values() {
        let cfg = InjectionConfig::default();
        assert_eq!(cfg.mode, InjectionMode::Clipboard);
        assert_eq!(cfg.type_delay_ms, 12);
        assert!(cfg.restore_clipboard);
    }

    #[test]
    fn injection_mode_serde_roundtrip() {
        for mode in [
            InjectionMode::Clipboard,
            InjectionMode::TypeOut,
            InjectionMode::ClipboardOnly,
        ] {
            let json = serde_json::to_string(&mode).unwrap();
            let back: InjectionMode = serde_json::from_str(&json).unwrap();
            assert_eq!(mode, back);
        }
    }

    #[test]
    fn injection_config_serde_roundtrip() {
        let cfg = InjectionConfig {
            mode: InjectionMode::TypeOut,
            type_delay_ms: 25,
            restore_clipboard: false,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: InjectionConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.mode, InjectionMode::TypeOut);
        assert_eq!(back.type_delay_ms, 25);
        assert!(!back.restore_clipboard);
    }

    #[test]
    fn config_deserialise_with_defaults() {
        // An empty JSON object should populate all defaults.
        let cfg: InjectionConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg.mode, InjectionMode::Clipboard);
        assert_eq!(cfg.type_delay_ms, 12);
        assert!(cfg.restore_clipboard);
    }

    #[test]
    fn injection_mode_json_values() {
        assert_eq!(
            serde_json::to_string(&InjectionMode::Clipboard).unwrap(),
            "\"clipboard\""
        );
        assert_eq!(
            serde_json::to_string(&InjectionMode::TypeOut).unwrap(),
            "\"type_out\""
        );
        assert_eq!(
            serde_json::to_string(&InjectionMode::ClipboardOnly).unwrap(),
            "\"clipboard_only\""
        );
    }

    #[test]
    fn injector_creation_succeeds() {
        // TextInjector::new is infallible (display-server dependence
        // is deferred to the actual inject/clipboard calls).
        let injector = TextInjector::new(InjectionConfig::default());
        assert_eq!(injector.config.mode, InjectionMode::Clipboard);
    }

    #[test]
    fn error_display() {
        let e = InjectionError::ClipboardAccess("no display".into());
        assert!(e.to_string().contains("clipboard access failed"));

        let e = InjectionError::KeystrokeSimulation("no display".into());
        assert!(e.to_string().contains("keystroke simulation failed"));

        let e = InjectionError::Timeout("5s elapsed".into());
        assert!(e.to_string().contains("timed out"));
    }
}
