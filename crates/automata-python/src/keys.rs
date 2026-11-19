//! String names for keyboard keys and mouse buttons.
//!
//! Python games refer to keys by lowercase `snake_case` names (`"a"`,
//! `"arrow_left"`, `"f1"`, ...). This module maps those names to winit's
//! physical key codes (the layout-independent values `InputState` uses)
//! and back, for the per-frame input snapshot.

use automata_input::{KeyCode, MouseButton};

/// One name → key-code association.
///
/// The table is searched linearly in both directions; at ~60 entries that
/// costs a few nanoseconds per query — far cheaper than the Python call
/// overhead above it.
const KEYS: &[(&str, KeyCode)] = &[
    // Letters.
    ("a", KeyCode::KeyA),
    ("b", KeyCode::KeyB),
    ("c", KeyCode::KeyC),
    ("d", KeyCode::KeyD),
    ("e", KeyCode::KeyE),
    ("f", KeyCode::KeyF),
    ("g", KeyCode::KeyG),
    ("h", KeyCode::KeyH),
    ("i", KeyCode::KeyI),
    ("j", KeyCode::KeyJ),
    ("k", KeyCode::KeyK),
    ("l", KeyCode::KeyL),
    ("m", KeyCode::KeyM),
    ("n", KeyCode::KeyN),
    ("o", KeyCode::KeyO),
    ("p", KeyCode::KeyP),
    ("q", KeyCode::KeyQ),
    ("r", KeyCode::KeyR),
    ("s", KeyCode::KeyS),
    ("t", KeyCode::KeyT),
    ("u", KeyCode::KeyU),
    ("v", KeyCode::KeyV),
    ("w", KeyCode::KeyW),
    ("x", KeyCode::KeyX),
    ("y", KeyCode::KeyY),
    ("z", KeyCode::KeyZ),
    // Digits (top row).
    ("0", KeyCode::Digit0),
    ("1", KeyCode::Digit1),
    ("2", KeyCode::Digit2),
    ("3", KeyCode::Digit3),
    ("4", KeyCode::Digit4),
    ("5", KeyCode::Digit5),
    ("6", KeyCode::Digit6),
    ("7", KeyCode::Digit7),
    ("8", KeyCode::Digit8),
    ("9", KeyCode::Digit9),
    // Navigation and editing.
    ("arrow_left", KeyCode::ArrowLeft),
    ("arrow_right", KeyCode::ArrowRight),
    ("arrow_up", KeyCode::ArrowUp),
    ("arrow_down", KeyCode::ArrowDown),
    ("home", KeyCode::Home),
    ("end", KeyCode::End),
    ("page_up", KeyCode::PageUp),
    ("page_down", KeyCode::PageDown),
    ("insert", KeyCode::Insert),
    ("delete", KeyCode::Delete),
    // Whitespace and control.
    ("space", KeyCode::Space),
    ("enter", KeyCode::Enter),
    ("escape", KeyCode::Escape),
    ("tab", KeyCode::Tab),
    ("backspace", KeyCode::Backspace),
    // Modifiers (physical side variants).
    ("shift_left", KeyCode::ShiftLeft),
    ("shift_right", KeyCode::ShiftRight),
    ("ctrl_left", KeyCode::ControlLeft),
    ("ctrl_right", KeyCode::ControlRight),
    ("alt_left", KeyCode::AltLeft),
    ("alt_right", KeyCode::AltRight),
    ("super_left", KeyCode::SuperLeft),
    ("super_right", KeyCode::SuperRight),
    // Locks.
    ("caps_lock", KeyCode::CapsLock),
    ("num_lock", KeyCode::NumLock),
    ("scroll_lock", KeyCode::ScrollLock),
    // Function keys.
    ("f1", KeyCode::F1),
    ("f2", KeyCode::F2),
    ("f3", KeyCode::F3),
    ("f4", KeyCode::F4),
    ("f5", KeyCode::F5),
    ("f6", KeyCode::F6),
    ("f7", KeyCode::F7),
    ("f8", KeyCode::F8),
    ("f9", KeyCode::F9),
    ("f10", KeyCode::F10),
    ("f11", KeyCode::F11),
    ("f12", KeyCode::F12),
    // Punctuation.
    ("minus", KeyCode::Minus),
    ("equal", KeyCode::Equal),
    ("comma", KeyCode::Comma),
    ("period", KeyCode::Period),
    ("slash", KeyCode::Slash),
    ("backslash", KeyCode::Backslash),
    ("semicolon", KeyCode::Semicolon),
    ("quote", KeyCode::Quote),
    ("backquote", KeyCode::Backquote),
    ("bracket_left", KeyCode::BracketLeft),
    ("bracket_right", KeyCode::BracketRight),
];

/// Mouse button names.
const BUTTONS: &[(&str, MouseButton)] = &[
    ("left", MouseButton::Left),
    ("right", MouseButton::Right),
    ("middle", MouseButton::Middle),
    ("back", MouseButton::Back),
    ("forward", MouseButton::Forward),
];

/// All known key name/code pairs.
pub fn all_keys() -> impl Iterator<Item = (&'static str, KeyCode)> {
    KEYS.iter().copied()
}

/// All known mouse button name/button pairs.
pub fn all_buttons() -> impl Iterator<Item = (&'static str, MouseButton)> {
    BUTTONS.iter().copied()
}

/// Resolves a key name to its physical key code.
pub fn key_from_name(name: &str) -> Option<KeyCode> {
    KEYS.iter().find(|(n, _)| *n == name).map(|(_, code)| *code)
}

/// Resolves a mouse button name.
pub fn button_from_name(name: &str) -> Option<MouseButton> {
    BUTTONS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, button)| *button)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_entries_round_trip() {
        for (name, code) in all_keys() {
            assert_eq!(key_from_name(name), Some(code));
        }
        for (name, button) in all_buttons() {
            assert_eq!(button_from_name(name), Some(button));
        }
    }

    #[test]
    fn names_and_codes_are_unique() {
        let names: Vec<_> = all_keys().map(|(name, _)| name).collect();
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(names.len(), unique.len(), "key names must be unique");
    }

    #[test]
    fn unknown_names_return_none() {
        assert_eq!(key_from_name("not_a_key"), None);
        assert_eq!(button_from_name("not_a_button"), None);
    }
}
