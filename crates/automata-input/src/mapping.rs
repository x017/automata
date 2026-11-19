//! Action-based input mapping.
//!
//! Games name their actions ("jump", "fire", "move") and bind them to
//! physical controls: keyboard keys, mouse buttons, gamepad buttons, and
//! gamepad axes. [`ActionMap`] resolves those bindings against the current
//! [`InputState`]/[`Gamepads`] each frame, so game code asks
//! `map.pressed("jump")` or `map.axis("move_x")` and never touches raw
//! device ids.
//!
//! Bindings are runtime-configurable (`bind`, `unbind_action`) and
//! serialisable (every binding is plain data), so control remapping UIs can
//! live entirely in game code.
//!
//! ```rust
//! use automata_input::prelude::*;
//!
//! let mut map = ActionMap::new();
//! map.bind("jump", Binding::key(KeyCode::Space));
//! map.bind("jump", Binding::gamepad_button(GamepadButton::South));
//! map.bind_axis("move_x", AxisBinding::left_stick_x());
//! ```

use std::collections::HashMap;

use crate::gamepad::{GamepadAxis, GamepadButton, Gamepads};
use crate::state::InputState;
use crate::{KeyCode, MouseButton};

/// A physical input that can trigger an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Binding {
    /// A keyboard key.
    Key(KeyCode),
    /// A mouse button.
    MouseButton(MouseButton),
    /// A gamepad button (any connected gamepad).
    GamepadButton(GamepadButton),
    /// A gamepad axis past a threshold in the positive direction.
    GamepadAxisPositive(GamepadAxis),
    /// A gamepad axis past a threshold in the negative direction.
    GamepadAxisNegative(GamepadAxis),
}

impl Binding {
    /// A keyboard key binding.
    pub fn key(key: KeyCode) -> Self {
        Self::Key(key)
    }

    /// A mouse button binding.
    pub fn mouse(button: MouseButton) -> Self {
        Self::MouseButton(button)
    }

    /// A gamepad button binding.
    pub fn gamepad_button(button: GamepadButton) -> Self {
        Self::GamepadButton(button)
    }
}

/// An analog axis source for [`ActionMap::axis`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AxisBinding {
    /// A gamepad axis, scaled by `scale` (default 1.0).
    GamepadAxis {
        /// Which axis.
        axis: GamepadAxis,
        /// Multiplier (use -1.0 to invert).
        scale: f32,
    },
    /// A pair of action-like buttons: negative minus positive → -1..=1.
    ButtonPair {
        /// Binding that produces -1.
        negative: Binding,
        /// Binding that produces +1.
        positive: Binding,
    },
}

impl AxisBinding {
    /// The left stick X axis.
    pub fn left_stick_x() -> Self {
        Self::GamepadAxis {
            axis: GamepadAxis::LeftStickX,
            scale: 1.0,
        }
    }

    /// The left stick Y axis.
    pub fn left_stick_y() -> Self {
        Self::GamepadAxis {
            axis: GamepadAxis::LeftStickY,
            scale: 1.0,
        }
    }

    /// The right stick X axis.
    pub fn right_stick_x() -> Self {
        Self::GamepadAxis {
            axis: GamepadAxis::RightStickX,
            scale: 1.0,
        }
    }

    /// The right stick Y axis.
    pub fn right_stick_y() -> Self {
        Self::GamepadAxis {
            axis: GamepadAxis::RightStickY,
            scale: 1.0,
        }
    }

    /// WASD-style: two keys forming an axis.
    pub fn key_pair(negative: KeyCode, positive: KeyCode) -> Self {
        Self::ButtonPair {
            negative: Binding::Key(negative),
            positive: Binding::Key(positive),
        }
    }

    /// A gamepad axis with a custom scale.
    pub fn gamepad_axis(axis: GamepadAxis, scale: f32) -> Self {
        Self::GamepadAxis { axis, scale }
    }
}

/// Default threshold for axis-as-button bindings.
const AXIS_THRESHOLD: f32 = 0.5;

/// The action map: named actions and axes bound to physical controls.
#[derive(Debug, Default)]
pub struct ActionMap {
    actions: HashMap<String, Vec<Binding>>,
    axes: HashMap<String, AxisBinding>,
}

impl ActionMap {
    /// An empty action map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind `action` to a physical input (stacking with existing bindings).
    pub fn bind(&mut self, action: impl Into<String>, binding: Binding) -> &mut Self {
        self.actions.entry(action.into()).or_default().push(binding);
        self
    }

    /// Bind a named analog axis.
    pub fn bind_axis(&mut self, name: impl Into<String>, axis: AxisBinding) -> &mut Self {
        self.axes.insert(name.into(), axis);
        self
    }

    /// Remove all bindings for `action`.
    pub fn unbind_action(&mut self, action: &str) -> &mut Self {
        self.actions.remove(action);
        self
    }

    /// Replace an action's bindings (for control-remapping UIs).
    pub fn rebind(&mut self, action: impl Into<String>, bindings: Vec<Binding>) -> &mut Self {
        self.actions.insert(action.into(), bindings);
        self
    }

    /// The bindings for an action.
    pub fn bindings(&self, action: &str) -> &[Binding] {
        self.actions.get(action).map(Vec::as_slice).unwrap_or(&[])
    }

    /// All action names currently bound.
    pub fn action_names(&self) -> impl Iterator<Item = &str> {
        self.actions.keys().map(String::as_str)
    }

    /// Is any binding for `action` currently active?
    pub fn pressed(&self, action: &str, input: &InputState, gamepads: &Gamepads) -> bool {
        self.bindings(action)
            .iter()
            .any(|b| binding_active(b, input, gamepads))
    }

    /// Did any binding for `action` become active this frame?
    pub fn just_pressed(&self, action: &str, input: &InputState, gamepads: &Gamepads) -> bool {
        // Axis crossings and gamepad buttons don't have a just-pressed edge
        // in the device state; keyboard/mouse do. For gamepad buttons, fall
        // back to "pressed" (the engine delivers edges via GamepadEvent).
        self.bindings(action).iter().any(|b| match b {
            Binding::Key(key) => input.just_pressed(*key),
            Binding::MouseButton(button) => input.button_just_pressed(*button),
            Binding::GamepadButton(button) => gamepads.pressed_any(*button),
            Binding::GamepadAxisPositive(axis) => gamepads.axis_any(*axis) >= AXIS_THRESHOLD,
            Binding::GamepadAxisNegative(axis) => gamepads.axis_any(*axis) <= -AXIS_THRESHOLD,
        })
    }

    /// Current value of a named axis, -1.0..=1.0.
    pub fn axis(&self, name: &str, input: &InputState, gamepads: &Gamepads) -> f32 {
        match self.axes.get(name) {
            Some(AxisBinding::GamepadAxis { axis, scale }) => gamepads.axis_any(*axis) * scale,
            Some(AxisBinding::ButtonPair { negative, positive }) => {
                let neg = binding_active(negative, input, gamepads) as i8;
                let pos = binding_active(positive, input, gamepads) as i8;
                (pos - neg) as f32
            }
            None => 0.0,
        }
    }

    /// Convenience: a 2D move vector from two named axes.
    pub fn axis_pair(
        &self,
        x: &str,
        y: &str,
        input: &InputState,
        gamepads: &Gamepads,
    ) -> (f32, f32) {
        (self.axis(x, input, gamepads), self.axis(y, input, gamepads))
    }
}

fn binding_active(binding: &Binding, input: &InputState, gamepads: &Gamepads) -> bool {
    match binding {
        Binding::Key(key) => input.pressed(*key),
        Binding::MouseButton(button) => input.button_pressed(*button),
        Binding::GamepadButton(button) => gamepads.pressed_any(*button),
        Binding::GamepadAxisPositive(axis) => gamepads.axis_any(*axis) >= AXIS_THRESHOLD,
        Binding::GamepadAxisNegative(axis) => gamepads.axis_any(*axis) <= -AXIS_THRESHOLD,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_binding_resolves_against_input_state() {
        let mut input = InputState::new();
        input.handle_key(KeyCode::Space, crate::ElementState::Pressed);

        let mut map = ActionMap::new();
        map.bind("jump", Binding::key(KeyCode::Space));
        let gamepads = Gamepads::new();

        assert!(map.pressed("jump", &input, &gamepads));
        assert!(map.just_pressed("jump", &input, &gamepads));
        assert!(!map.pressed("fire", &input, &gamepads));
    }

    #[test]
    fn mouse_binding_resolves() {
        let mut input = InputState::new();
        input.handle_mouse_button(MouseButton::Left, crate::ElementState::Pressed);

        let mut map = ActionMap::new();
        map.bind("fire", Binding::mouse(MouseButton::Left));
        let gamepads = Gamepads::new();

        assert!(map.pressed("fire", &input, &gamepads));
    }

    #[test]
    fn key_pair_axis() {
        let mut input = InputState::new();
        input.handle_key(KeyCode::KeyD, crate::ElementState::Pressed);

        let mut map = ActionMap::new();
        map.bind_axis(
            "move_x",
            AxisBinding::key_pair(KeyCode::KeyA, KeyCode::KeyD),
        );
        let gamepads = Gamepads::new();

        assert_eq!(map.axis("move_x", &input, &gamepads), 1.0);

        input.handle_key(KeyCode::KeyA, crate::ElementState::Pressed);
        assert_eq!(map.axis("move_x", &input, &gamepads), 0.0);
    }

    #[test]
    fn rebind_replaces_bindings() {
        let mut map = ActionMap::new();
        map.bind("jump", Binding::key(KeyCode::Space));
        map.rebind("jump", vec![Binding::key(KeyCode::Enter)]);
        assert_eq!(map.bindings("jump"), &[Binding::Key(KeyCode::Enter)]);
    }
}
