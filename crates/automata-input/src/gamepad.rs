//! Gamepad support: connection tracking, button/axis state, and events.
//!
//! On desktop this wraps [gilrs](https://docs.rs/gilrs); in the browser
//! (emscripten) the page reads the Gamepad API and feeds snapshots in (see
//! `gamepad_web.rs`). Either way the main loop calls [`Gamepads::update`]
//! once per frame; game code queries buttons/axes or matches on
//! [`GamepadEvent`]. Headless environments simply see no connected
//! gamepads.

#[cfg(not(target_os = "emscripten"))]
pub use crate::gamepad_gilrs::Gamepads;
#[cfg(target_os = "emscripten")]
pub use crate::gamepad_web::{GamepadAxis, GamepadButton, GamepadId, Gamepads, PadSnapshot};
#[cfg(not(target_os = "emscripten"))]
pub use gilrs::{Axis as GamepadAxis, Button as GamepadButton, GamepadId};

/// A gamepad event for the engine's event stream.
#[derive(Debug, Clone, PartialEq)]
pub enum GamepadEvent {
    /// A gamepad was connected.
    Connected {
        /// Which gamepad.
        id: GamepadId,
        /// Human-readable name (e.g. "Xbox Wireless Controller").
        name: String,
    },
    /// A gamepad was disconnected.
    Disconnected {
        /// Which gamepad.
        id: GamepadId,
    },
    /// A button changed state.
    Button {
        /// Which gamepad.
        id: GamepadId,
        /// Which button.
        button: GamepadButton,
        /// Whether the button is now pressed.
        pressed: bool,
    },
    /// An axis moved (analog stick or trigger).
    Axis {
        /// Which gamepad.
        id: GamepadId,
        /// Which axis.
        axis: GamepadAxis,
        /// Current value (-1.0..=1.0 for sticks, 0.0..=1.0 for triggers).
        value: f32,
    },
}
