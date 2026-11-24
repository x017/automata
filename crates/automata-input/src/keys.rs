//! The engine's own keyboard, mouse and touch types, independent of any
//! windowing library. Platform drivers (winit on desktop; SDL for Android)
//! translate their events into [`InputEvent`]s; everything above them
//! (game code, the Python API, action maps) only sees these types.
//!
//! Variant names follow the W3C `KeyboardEvent.code` values that winit and
//! browsers use, so `KeyCode::KeyW` is "the key in the W position" on any
//! layout.

use std::ops::{BitOr, BitOrAssign};

/// Declares [`KeyCode`] with its snake-case name for scripts.
macro_rules! key_codes {
    ($( $(#[$doc:meta])* $variant:ident => $name:literal, )*) => {
        /// A physical key, by its position on a US keyboard.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[non_exhaustive]
        pub enum KeyCode {
            $( $(#[$doc])* $variant, )*
        }

        impl KeyCode {
            /// Every key, in declaration order.
            pub const ALL: &'static [KeyCode] = &[$(KeyCode::$variant,)*];

            /// The key's name in scripts: `"a"`, `"arrow_left"`, `"f1"`,
            /// `"numpad_5"`.
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    $( KeyCode::$variant => $name, )*
                }
            }
        }
    };
}

key_codes! {
    /// A.
    KeyA => "a",
    /// B.
    KeyB => "b",
    /// C.
    KeyC => "c",
    /// D.
    KeyD => "d",
    /// E.
    KeyE => "e",
    /// F.
    KeyF => "f",
    /// G.
    KeyG => "g",
    /// H.
    KeyH => "h",
    /// I.
    KeyI => "i",
    /// J.
    KeyJ => "j",
    /// K.
    KeyK => "k",
    /// L.
    KeyL => "l",
    /// M.
    KeyM => "m",
    /// N.
    KeyN => "n",
    /// O.
    KeyO => "o",
    /// P.
    KeyP => "p",
    /// Q.
    KeyQ => "q",
    /// R.
    KeyR => "r",
    /// S.
    KeyS => "s",
    /// T.
    KeyT => "t",
    /// U.
    KeyU => "u",
    /// V.
    KeyV => "v",
    /// W.
    KeyW => "w",
    /// X.
    KeyX => "x",
    /// Y.
    KeyY => "y",
    /// Z.
    KeyZ => "z",
    /// 0 on the top row.
    Digit0 => "0",
    /// 1 on the top row.
    Digit1 => "1",
    /// 2 on the top row.
    Digit2 => "2",
    /// 3 on the top row.
    Digit3 => "3",
    /// 4 on the top row.
    Digit4 => "4",
    /// 5 on the top row.
    Digit5 => "5",
    /// 6 on the top row.
    Digit6 => "6",
    /// 7 on the top row.
    Digit7 => "7",
    /// 8 on the top row.
    Digit8 => "8",
    /// 9 on the top row.
    Digit9 => "9",
    /// ←.
    ArrowLeft => "arrow_left",
    /// →.
    ArrowRight => "arrow_right",
    /// ↑.
    ArrowUp => "arrow_up",
    /// ↓.
    ArrowDown => "arrow_down",
    /// Home.
    Home => "home",
    /// End.
    End => "end",
    /// Page Up.
    PageUp => "page_up",
    /// Page Down.
    PageDown => "page_down",
    /// Insert.
    Insert => "insert",
    /// Delete (forward delete).
    Delete => "delete",
    /// Space bar.
    Space => "space",
    /// Enter / Return.
    Enter => "enter",
    /// Escape.
    Escape => "escape",
    /// Tab.
    Tab => "tab",
    /// Backspace.
    Backspace => "backspace",
    /// Left Shift.
    ShiftLeft => "shift_left",
    /// Right Shift.
    ShiftRight => "shift_right",
    /// Left Control.
    ControlLeft => "ctrl_left",
    /// Right Control.
    ControlRight => "ctrl_right",
    /// Left Alt (Option).
    AltLeft => "alt_left",
    /// Right Alt (Option, `AltGr`).
    AltRight => "alt_right",
    /// Left Windows / Command key.
    SuperLeft => "super_left",
    /// Right Windows / Command key.
    SuperRight => "super_right",
    /// Caps Lock.
    CapsLock => "caps_lock",
    /// Num Lock.
    NumLock => "num_lock",
    /// Scroll Lock.
    ScrollLock => "scroll_lock",
    /// F1.
    F1 => "f1",
    /// F2.
    F2 => "f2",
    /// F3.
    F3 => "f3",
    /// F4.
    F4 => "f4",
    /// F5.
    F5 => "f5",
    /// F6.
    F6 => "f6",
    /// F7.
    F7 => "f7",
    /// F8.
    F8 => "f8",
    /// F9.
    F9 => "f9",
    /// F10.
    F10 => "f10",
    /// F11.
    F11 => "f11",
    /// F12.
    F12 => "f12",
    /// `-`.
    Minus => "minus",
    /// `=`.
    Equal => "equal",
    /// `,`.
    Comma => "comma",
    /// `.`.
    Period => "period",
    /// `/`.
    Slash => "slash",
    /// `\`.
    Backslash => "backslash",
    /// `;`.
    Semicolon => "semicolon",
    /// `'`.
    Quote => "quote",
    /// `` ` ``.
    Backquote => "backquote",
    /// `[`.
    BracketLeft => "bracket_left",
    /// `]`.
    BracketRight => "bracket_right",
    /// Numpad 0.
    Numpad0 => "numpad_0",
    /// Numpad 1.
    Numpad1 => "numpad_1",
    /// Numpad 2.
    Numpad2 => "numpad_2",
    /// Numpad 3.
    Numpad3 => "numpad_3",
    /// Numpad 4.
    Numpad4 => "numpad_4",
    /// Numpad 5.
    Numpad5 => "numpad_5",
    /// Numpad 6.
    Numpad6 => "numpad_6",
    /// Numpad 7.
    Numpad7 => "numpad_7",
    /// Numpad 8.
    Numpad8 => "numpad_8",
    /// Numpad 9.
    Numpad9 => "numpad_9",
    /// Numpad `+`.
    NumpadAdd => "numpad_add",
    /// Numpad `-`.
    NumpadSubtract => "numpad_subtract",
    /// Numpad `*`.
    NumpadMultiply => "numpad_multiply",
    /// Numpad `/`.
    NumpadDivide => "numpad_divide",
    /// Numpad `.`.
    NumpadDecimal => "numpad_decimal",
    /// Numpad Enter.
    NumpadEnter => "numpad_enter",
    /// Print Screen.
    PrintScreen => "print_screen",
    /// Pause / Break.
    Pause => "pause",
    /// The context-menu key.
    ContextMenu => "context_menu",
    /// Android's Back button (and a browser Back key).
    BrowserBack => "back_button",
}

impl KeyCode {
    /// The key a script name means (see [`name`](Self::name)).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| k.name() == name)
    }
}

/// A mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MouseButton {
    /// The primary button (and a finger on a touch screen).
    Left,
    /// The secondary button.
    Right,
    /// The wheel button.
    Middle,
    /// The "back" side button.
    Back,
    /// The "forward" side button.
    Forward,
    /// Any other button, by number.
    Other(u16),
}

impl MouseButton {
    /// The named buttons, for script names.
    pub const NAMED: &'static [MouseButton] = &[
        MouseButton::Left,
        MouseButton::Right,
        MouseButton::Middle,
        MouseButton::Back,
        MouseButton::Forward,
    ];

    /// The button's name in scripts (`"left"` …), or `None` for
    /// [`Other`](Self::Other).
    #[must_use]
    pub const fn name(self) -> Option<&'static str> {
        match self {
            Self::Left => Some("left"),
            Self::Right => Some("right"),
            Self::Middle => Some("middle"),
            Self::Back => Some("back"),
            Self::Forward => Some("forward"),
            Self::Other(_) => None,
        }
    }

    /// The button a script name means.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::NAMED.iter().copied().find(|b| b.name() == Some(name))
    }
}

/// Whether a key or button went down or up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ElementState {
    /// It went down.
    Pressed,
    /// It went up.
    Released,
}

impl ElementState {
    /// Whether this is [`Pressed`](Self::Pressed).
    #[must_use]
    pub const fn is_pressed(self) -> bool {
        matches!(self, Self::Pressed)
    }
}

/// The modifier keys held (either side).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ModifiersState(u8);

impl ModifiersState {
    /// Shift.
    pub const SHIFT: Self = Self(1);
    /// Control.
    pub const CONTROL: Self = Self(2);
    /// Alt / Option.
    pub const ALT: Self = Self(4);
    /// Windows / Command.
    pub const SUPER: Self = Self(8);

    /// No modifiers.
    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Whether every modifier in `other` is held.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether Shift is held.
    #[must_use]
    pub const fn shift_key(self) -> bool {
        self.contains(Self::SHIFT)
    }

    /// Whether Control is held.
    #[must_use]
    pub const fn control_key(self) -> bool {
        self.contains(Self::CONTROL)
    }

    /// Whether Alt is held.
    #[must_use]
    pub const fn alt_key(self) -> bool {
        self.contains(Self::ALT)
    }

    /// Whether Windows / Command is held.
    #[must_use]
    pub const fn super_key(self) -> bool {
        self.contains(Self::SUPER)
    }

    /// `self` with `other` held or not.
    #[must_use]
    pub const fn with(self, other: Self, held: bool) -> Self {
        if held {
            Self(self.0 | other.0)
        } else {
            Self(self.0 & !other.0)
        }
    }
}

impl BitOr for ModifiersState {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for ModifiersState {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// Where a touch is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TouchPhase {
    /// A finger landed.
    Started,
    /// It moved.
    Moved,
    /// It lifted.
    Ended,
    /// The system took it away (a gesture, the app losing focus).
    Cancelled,
}

/// One input event, as a platform driver reports it. Positions are in
/// physical pixels from the window's top-left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputEvent {
    /// A key went down or up. Key repeat sends `Pressed` again.
    Key {
        /// Which key.
        key: KeyCode,
        /// Down or up.
        state: ElementState,
    },
    /// A mouse button went down or up.
    MouseButton {
        /// Which button.
        button: MouseButton,
        /// Down or up.
        state: ElementState,
    },
    /// The cursor moved to `(x, y)`.
    CursorMoved {
        /// Pixels from the left.
        x: f64,
        /// Pixels from the top.
        y: f64,
    },
    /// The wheel or touchpad scrolled (lines or pixels, as the platform
    /// reports; +y is away from the user).
    Scroll {
        /// Horizontal amount.
        x: f64,
        /// Vertical amount.
        y: f64,
    },
    /// Raw mouse movement, which keeps coming while the cursor is locked.
    MouseMotion {
        /// Rightward movement.
        dx: f64,
        /// Downward movement.
        dy: f64,
    },
    /// A finger touched, moved or lifted.
    Touch {
        /// The finger (stable while it's down).
        id: u64,
        /// What happened.
        phase: TouchPhase,
        /// Pixels from the left.
        x: f64,
        /// Pixels from the top.
        y: f64,
    },
    /// The held modifier keys changed.
    Modifiers(ModifiersState),
    /// The window lost focus: every held key and button is let go.
    FocusLost,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names_are_unique_and_round_trip() {
        let mut names = std::collections::HashSet::new();
        for &k in KeyCode::ALL {
            assert!(names.insert(k.name()), "{} twice", k.name());
            assert_eq!(KeyCode::from_name(k.name()), Some(k));
        }
        assert_eq!(KeyCode::from_name("arrow_left"), Some(KeyCode::ArrowLeft));
        assert_eq!(
            KeyCode::from_name("Arrow_Left"),
            None,
            "names are lowercase"
        );
        assert_eq!(KeyCode::from_name("not_a_key"), None);
        assert!(KeyCode::ALL.len() > 100);
    }

    #[test]
    fn mouse_buttons_have_names() {
        for &b in MouseButton::NAMED {
            assert_eq!(MouseButton::from_name(b.name().unwrap()), Some(b));
        }
        assert_eq!(MouseButton::Other(7).name(), None);
        assert_eq!(MouseButton::from_name("wheel"), None);
    }

    #[test]
    fn modifiers_combine() {
        let m = ModifiersState::SHIFT | ModifiersState::CONTROL;
        assert!(m.shift_key() && m.control_key() && !m.alt_key() && !m.super_key());
        assert!(m.contains(ModifiersState::SHIFT));
        let m = m
            .with(ModifiersState::SHIFT, false)
            .with(ModifiersState::SUPER, true);
        assert!(!m.shift_key() && m.super_key());
        assert_eq!(ModifiersState::default(), ModifiersState::empty());
    }
}
