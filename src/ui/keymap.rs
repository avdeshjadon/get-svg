//! Keyboard bindings.
//!
//! Shortcuts are centralized here (not scattered through handlers) so they
//! can later be user-configurable without touching application logic.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone)]
pub struct Keymap {
    pub quit: KeyCode,
    pub back: KeyCode,
    pub confirm: KeyCode,
    pub cancel: KeyCode,
    pub help: KeyCode,
    pub search: KeyCode,
    pub refresh: KeyCode,
    pub download: KeyCode,
    pub download_all: KeyCode,
    pub zip: KeyCode,
    pub toggle_select: KeyCode,
    pub select_all: KeyCode,
    pub select_none: KeyCode,
    pub copy: KeyCode,
    pub open: KeyCode,
    pub attribution: KeyCode,
}

impl Default for Keymap {
    fn default() -> Self {
        Keymap {
            quit: KeyCode::Char('q'),
            back: KeyCode::Esc,
            confirm: KeyCode::Char('y'),
            cancel: KeyCode::Char('n'),
            help: KeyCode::Char('?'),
            search: KeyCode::Char('/'),
            refresh: KeyCode::Char('r'),
            download: KeyCode::Char('d'),
            download_all: KeyCode::Char('A'),
            zip: KeyCode::Char('z'),
            toggle_select: KeyCode::Char(' '),
            select_all: KeyCode::Char('a'),
            select_none: KeyCode::Char('n'),
            copy: KeyCode::Char('c'),
            open: KeyCode::Char('o'),
            attribution: KeyCode::Char('a'),
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
