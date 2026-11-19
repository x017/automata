//! AUTOMATA input system.
//!
//! Two layers:
//!
//! - **Device state.** [`InputState`] collects keyboard/mouse events into a
//!   per-frame snapshot; [`Gamepads`] does the same for gamepads. The main
//!   loop feeds them events between frames and clears them after each
//!   update.
//! - **Action mapping.** [`ActionMap`] binds named actions ("jump") and
//!   axes ("move_x") to physical controls so game code is device-independent
//!   and controls are remappable at runtime.
//!
//! Gamepad and action events flow onto the engine's event stream as
//! [`GamepadEvent`]s; keyboard/mouse stay poll-based (pressed/just_pressed).
//!
//! The keyboard/mouse key types are currently re-exports of `winit`'s
//! well-established W3C-based enums; call sites should always refer to them
//! through this crate.

mod gamepad;
mod mapping;
mod state;

pub use gamepad::{GamepadAxis, GamepadButton, GamepadEvent, GamepadId, Gamepads};
pub use mapping::{ActionMap, AxisBinding, Binding};
pub use state::InputState;

pub use winit::event::{ElementState, MouseButton};
pub use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};

/// Everything you need for input: `use automata_input::prelude::*`.
pub mod prelude {
    pub use crate::gamepad::{GamepadAxis, GamepadButton, GamepadEvent, GamepadId, Gamepads};
    pub use crate::mapping::{ActionMap, AxisBinding, Binding};
    pub use crate::state::InputState;
    pub use crate::{ElementState, KeyCode, ModifiersState, MouseButton, PhysicalKey};
}
