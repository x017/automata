//! AUTOMATA input system.
//!
//! [`InputState`] collects raw window events into a per-frame input snapshot
//! that game code queries during its update:
//!
//! 1. The window runtime feeds it events between frames
//!    ([`InputState::handle_window_event`]).
//! 2. Game logic queries it through [`AppContext`](https://docs.rs) during
//!    the update (`pressed`, `just_pressed`, `cursor_position`, ...).
//! 3. The main loop calls [`InputState::end_frame`] after each update,
//!    clearing per-frame state (`just_pressed`, scroll deltas).
//!
//! Keyboard, mouse, and gamepad *mapping* (binding actions to inputs) is a
//! Phase 6 concern built on top of this crate.
//!
//! The keyboard/mouse key types are currently re-exports of `winit`'s
//! well-established W3C-based enums. They will migrate to engine-native types
//! when the Python binding layer needs them decoupled from the windowing
//! backend; call sites should always refer to them through this crate.

mod state;

pub use state::InputState;

pub use winit::event::{ElementState, MouseButton};
pub use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
