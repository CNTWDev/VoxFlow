use std::time::Duration;
use std::panic::{catch_unwind, AssertUnwindSafe};
#[cfg(target_os = "windows")]
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use crate::error::InjectError;

pub struct TextInjector {
    pub paste_delay_ms: u64,
    pub post_paste_delay_ms: u64,
    pub restore_text_clipboard: bool,
}

impl TextInjector {
    pub fn new(paste_delay_ms: u64, post_paste_delay_ms: u64, restore_text_clipboard: bool) -> Self {
        Self { paste_delay_ms, post_paste_delay_ms, restore_text_clipboard }
    }

    // Must be called from a spawn_blocking context — arboard requires same-thread open/close on Windows.
    pub fn inject_sync(&self, text: String) -> Result<(), InjectError> {
        use arboard::Clipboard;

        let mut clipboard = Clipboard::new()
            .map_err(|e| InjectError::Clipboard(e.to_string()))?;

        // Save current text clipboard content
        let saved = if self.restore_text_clipboard {
            clipboard.get_text().ok()
        } else {
            None
        };

        // Write transcription to clipboard
        clipboard.set_text(&text)
            .map_err(|e| InjectError::Clipboard(e.to_string()))?;

        // Give clipboard time to propagate
        std::thread::sleep(Duration::from_millis(self.paste_delay_ms));

        // Simulate paste. Keep this behind catch_unwind so a backend panic does not
        // take the whole app down; native aborts/segfaults still need backend fixes.
        catch_unwind(AssertUnwindSafe(Self::send_paste))
            .map_err(|_| InjectError::Keys("paste simulation panicked".into()))??;

        // Wait for target app to read
        std::thread::sleep(Duration::from_millis(self.post_paste_delay_ms));

        // Restore previous clipboard text (best-effort)
        if let Some(prev) = saved {
            let _ = clipboard.set_text(prev);
        }

        Ok(())
    }

    fn send_paste() -> Result<(), InjectError> {
        #[cfg(target_os = "macos")]
        {
            let status = std::process::Command::new("osascript")
                .args([
                    "-e",
                    r#"tell application "System Events" to keystroke "v" using command down"#,
                ])
                .status()
                .map_err(|e| InjectError::Keys(e.to_string()))?;
            if !status.success() {
                return Err(InjectError::Keys(format!(
                    "paste command failed with status {status}"
                )));
            }
        }
        #[cfg(target_os = "windows")]
        {
            let mut enigo = Enigo::new(&Settings::default())
                .map_err(|e| InjectError::Keys(e.to_string()))?;
            enigo.key(Key::Control, Direction::Press)
                .map_err(|e| InjectError::Keys(e.to_string()))?;
            let paste_result = enigo.key(Key::V, Direction::Click)
                .map_err(|e| InjectError::Keys(e.to_string()));
            let release_result = enigo.key(Key::Control, Direction::Release)
                .map_err(|e| InjectError::Keys(e.to_string()));
            release_result?;
            paste_result?;
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            return Err(InjectError::Keys("unsupported platform".into()));
        }

        Ok(())
    }
}
