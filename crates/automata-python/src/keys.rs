//! String names for keyboard keys and mouse buttons.
//!
//! Python games refer to keys by lowercase `snake_case` names (`"a"`,
//! `"arrow_left"`, `"f1"`, ...). The names live with the keys in
//! `automata_input` ([`KeyCode::name`]), so every platform driver and both
//! script runners share one table.

use automata_input::{KeyCode, MouseButton};

/// All known key name/code pairs.
pub fn all_keys() -> impl Iterator<Item = (&'static str, KeyCode)> {
    KeyCode::ALL.iter().map(|&k| (k.name(), k))
}

/// All known mouse button name/button pairs.
pub fn all_buttons() -> impl Iterator<Item = (&'static str, MouseButton)> {
    MouseButton::NAMED
        .iter()
        .filter_map(|&b| b.name().map(|n| (n, b)))
}

/// Resolves a key name to its physical key code.
pub fn key_from_name(name: &str) -> Option<KeyCode> {
    KeyCode::from_name(name)
}

/// Resolves a mouse button name.
pub fn button_from_name(name: &str) -> Option<MouseButton> {
    MouseButton::from_name(name)
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
    fn the_python_lists_name_every_key() {
        // enums.py (at.Key) and script_api.py (the names both runners know)
        // must list exactly the engine's keys.
        let enums = include_str!("../python/automata/enums.py");
        let api = include_str!("../python/automata/script_api.py");
        let start = api
            .find("_KEYBOARD = (")
            .expect("_KEYBOARD in script_api.py");
        let keyboard = &api[start..start + api[start..].find("\n)\n").unwrap()];
        for (name, _) in all_keys() {
            assert!(
                enums.contains(&format!("= \"{name}\"")),
                "at.Key lacks {name}"
            );
            let stem = name.trim_end_matches(|c: char| c.is_ascii_digit());
            let listed = keyboard.contains(&format!("\"{name}\""))
                || (name.len() == 1 && keyboard.contains(name))
                || keyboard.contains(&format!("f\"{stem}{{i}}\""));
            assert!(listed, "script_api._KEYBOARD lacks {name}");
        }
    }

    #[test]
    fn unknown_names_return_none() {
        assert_eq!(key_from_name("not_a_key"), None);
        assert_eq!(button_from_name("not_a_button"), None);
    }
}
