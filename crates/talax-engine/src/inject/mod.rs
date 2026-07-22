use std::thread;
use std::time::Duration;

use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use serde::{Deserialize, Serialize};

/// How long to wait after issuing the paste keystroke before restoring the
/// previous clipboard contents, giving the target application time to read it.
const CLIPBOARD_SETTLE: Duration = Duration::from_millis(120);

/// Upper bound on how long to poll for our own text to appear on the clipboard
/// after `set_text`, and the per-iteration poll interval.
const CLIPBOARD_CONFIRM_TIMEOUT: Duration = Duration::from_millis(500);
const CLIPBOARD_POLL_INTERVAL: Duration = Duration::from_millis(10);

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
    /// Defaults to the safe `ClipboardOnly` mode. Auto-pasting (`Clipboard`)
    /// or simulated keystrokes (`TypeOut`) inject into whatever window
    /// currently holds focus, which is a footgun if focus has moved; callers
    /// must opt in to those modes explicitly.
    fn default() -> Self {
        Self::ClipboardOnly
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

    /// Whether to restore the original clipboard after a successful paste.
    /// Once copied, failed pastes leave the transcript available for a manual paste.
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

        // Confirm our text actually landed on the clipboard before pasting.
        // `set_text` can race the clipboard owner change on some platforms;
        // poll briefly so the paste does not fire against stale contents.
        self.wait_for_clipboard(text)?;

        // Simulate paste keystroke.
        self.simulate_paste()?;

        // Give the target app time to read the clipboard in response to the
        // paste before we overwrite it again. Restoring too early loses the
        // race and pastes the *original* clipboard instead of our text.
        thread::sleep(CLIPBOARD_SETTLE);

        // Restore original clipboard content (best-effort; never propagate).
        if let Some(original) = saved {
            let _ = self.set_clipboard(&original);
        }

        Ok(())
    }

    /// Poll until the clipboard confirms the expected text before pasting.
    fn wait_for_clipboard(&self, expected: &str) -> Result<(), InjectionError> {
        wait_for_expected_value(
            expected,
            CLIPBOARD_CONFIRM_TIMEOUT,
            CLIPBOARD_POLL_INTERVAL,
            || self.get_clipboard(),
        )
    }

    /// Simulate Ctrl+V (or Cmd+V on macOS).
    fn simulate_paste(&self) -> Result<(), InjectionError> {
        let mut enigo = Enigo::new(&Settings::default())
            .map_err(|e| InjectionError::KeystrokeSimulation(e.to_string()))?;

        let modifier = if cfg!(target_os = "macos") {
            Key::Meta
        } else {
            Key::Control
        };

        perform_paste(modifier, |key, direction| {
            enigo
                .key(key, direction)
                .map_err(|e| InjectionError::KeystrokeSimulation(e.to_string()))
        })
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

fn wait_for_expected_value<F>(
    expected: &str,
    timeout: Duration,
    poll_interval: Duration,
    mut read: F,
) -> Result<(), InjectionError>
where
    F: FnMut() -> Result<String, InjectionError>,
{
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if matches!(read(), Ok(ref current) if current == expected) {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(InjectionError::Timeout(format!(
                "clipboard did not confirm injected text within {} ms",
                timeout.as_millis()
            )));
        }
        thread::sleep(poll_interval);
    }
}

fn perform_paste<F>(modifier: Key, mut emit: F) -> Result<(), InjectionError>
where
    F: FnMut(Key, Direction) -> Result<(), InjectionError>,
{
    emit(modifier, Direction::Press)?;
    let click_result = emit(Key::Unicode('v'), Direction::Click);
    let release_result = emit(modifier, Direction::Release);
    click_result?;
    release_result
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
        // Default is the safe clipboard-only mode: text is placed on the
        // clipboard but never auto-pasted into the focused window.
        assert_eq!(cfg.mode, InjectionMode::ClipboardOnly);
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
        assert_eq!(cfg.mode, InjectionMode::ClipboardOnly);
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
        assert_eq!(injector.config.mode, InjectionMode::ClipboardOnly);
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

    #[test]
    fn clipboard_confirmation_retries_until_expected_text_appears() {
        let mut reads = 0;
        let result = wait_for_expected_value(
            "transcript",
            Duration::from_millis(100),
            Duration::ZERO,
            || {
                reads += 1;
                Ok(if reads == 1 {
                    "stale".to_string()
                } else {
                    "transcript".to_string()
                })
            },
        );

        assert!(result.is_ok());
        assert_eq!(reads, 2);
    }

    #[test]
    fn clipboard_confirmation_reports_timeout() {
        let result = wait_for_expected_value("transcript", Duration::ZERO, Duration::ZERO, || {
            Ok("stale".to_string())
        });

        assert!(matches!(result, Err(InjectionError::Timeout(_))));
    }

    #[test]
    fn paste_releases_modifier_when_click_fails() {
        let mut directions = Vec::new();
        let result = perform_paste(Key::Control, |_, direction| {
            directions.push(direction);
            if direction == Direction::Click {
                Err(InjectionError::KeystrokeSimulation(
                    "click failed".to_string(),
                ))
            } else {
                Ok(())
            }
        });

        assert!(matches!(
            result,
            Err(InjectionError::KeystrokeSimulation(_))
        ));
        assert_eq!(
            directions,
            vec![Direction::Press, Direction::Click, Direction::Release]
        );
    }
}
