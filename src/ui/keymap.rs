//! Keyboard bindings.
//!
//! Shortcuts are centralized here (not scattered through handlers) so they
//! can later be user-configurable without touching application logic.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone)]
pub struct Keymap {
    pub quit: KeyCode,
    pub back: KeyCode,
}

impl Default for Keymap {
    fn default() -> Self {
        Keymap {
            quit: KeyCode::Char('q'),
            back: KeyCode::Esc,
        }
    }
}

impl Keymap {
    pub fn matches(&self, key: &KeyEvent, target: KeyCode) -> bool {
        key.code == target
    }

    /// Ctrl+C always quits, everywhere.
    pub fn is_force_quit(key: &KeyEvent) -> bool {
        key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
    }
}
