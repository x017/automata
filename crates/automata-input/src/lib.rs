//! AUTOMATA input system.
//!
//! Two layers:
//!
//! - **Device state.** [`InputState`] collects keyboard/mouse events into a
//!   per-frame snapshot; [`Gamepads`] does the same for gamepads. The main
//!   loop feeds them events between frames and clears them after each
//!   update.
//! - **Action mapping.** [`ActionMap`] binds named actions (`jump`) and
//!   axes (`move_x`) to physical controls so game code is device-independent
//!   and controls are remappable at runtime.
//!
//! Gamepad and action events flow onto the engine's event stream as
//! [`GamepadEvent`]s; keyboard/mouse stay poll-based (`pressed`/`just_pressed`).
//!
//! The keyboard, mouse and touch types ([`KeyCode`], [`InputEvent`] …) are
//! the engine's own, so the same game code runs on every platform driver:
//! winit on desktop, SDL for the coming Android build. Drivers translate their events into
//! [`InputEvent`]s for [`InputState::handle_event`].

mod gamepad;
#[cfg(not(target_os = "emscripten"))]
mod gamepad_gilrs;
#[cfg(target_os = "emscripten")]
mod gamepad_web;
mod keys;
mod mapping;
mod state;

#[cfg(target_os = "emscripten")]
pub use gamepad::PadSnapshot;
pub use gamepad::{GamepadAxis, GamepadButton, GamepadEvent, GamepadId, Gamepads};
pub use mapping::{ActionMap, AxisBinding, Binding};
pub use state::{InputState, Touch};

pub use keys::{ElementState, InputEvent, KeyCode, ModifiersState, MouseButton, TouchPhase};

/// Everything you need for input: `use automata_input::prelude::*`.
pub mod prelude {
    pub use crate::gamepad::{GamepadAxis, GamepadButton, GamepadEvent, GamepadId, Gamepads};
    pub use crate::mapping::{ActionMap, AxisBinding, Binding};
    pub use crate::state::InputState;
    pub use crate::{ElementState, InputEvent, KeyCode, ModifiersState, MouseButton, TouchPhase};
}
