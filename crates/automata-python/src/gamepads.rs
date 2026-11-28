//! Gamepad state snapshot and events for the Python frame context.
//!
//! Mirrors the keyboard/mouse snapshot in [`crate::context::PyInput`]:
//! everything is owned data captured at the start of the frame. Gamepads
//! are addressed by integer id; buttons and axes by lowercase `snake_case`
//! names (`"south"`, `"dpad_up"`, `"left_stick_x"`, ...). The gilrs
//! `Unknown` variants are intentionally unnameable.

use std::collections::{HashMap, HashSet};

use automata_input::{GamepadAxis, GamepadButton, GamepadEvent, GamepadId, Gamepads};
use pyo3::exceptions::PyKeyError;
use pyo3::prelude::*;

/// Gamepad button names.
const BUTTONS: &[(&str, GamepadButton)] = &[
    ("south", GamepadButton::South),
    ("east", GamepadButton::East),
    ("north", GamepadButton::North),
    ("west", GamepadButton::West),
    ("c", GamepadButton::C),
    ("z", GamepadButton::Z),
    ("left_trigger", GamepadButton::LeftTrigger),
    ("left_trigger2", GamepadButton::LeftTrigger2),
    ("right_trigger", GamepadButton::RightTrigger),
    ("right_trigger2", GamepadButton::RightTrigger2),
    ("select", GamepadButton::Select),
    ("start", GamepadButton::Start),
    ("mode", GamepadButton::Mode),
    ("left_thumb", GamepadButton::LeftThumb),
    ("right_thumb", GamepadButton::RightThumb),
    ("dpad_up", GamepadButton::DPadUp),
    ("dpad_down", GamepadButton::DPadDown),
    ("dpad_left", GamepadButton::DPadLeft),
    ("dpad_right", GamepadButton::DPadRight),
];

/// Gamepad axis names.
const AXES: &[(&str, GamepadAxis)] = &[
    ("left_stick_x", GamepadAxis::LeftStickX),
    ("left_stick_y", GamepadAxis::LeftStickY),
    ("left_z", GamepadAxis::LeftZ),
    ("right_z", GamepadAxis::RightZ),
    ("right_stick_x", GamepadAxis::RightStickX),
    ("right_stick_y", GamepadAxis::RightStickY),
    ("dpad_x", GamepadAxis::DPadX),
    ("dpad_y", GamepadAxis::DPadY),
];

/// All known gamepad button name/button pairs.
pub fn all_buttons() -> impl Iterator<Item = (&'static str, GamepadButton)> {
    BUTTONS.iter().copied()
}

/// All known gamepad axis name/axis pairs.
pub fn all_axes() -> impl Iterator<Item = (&'static str, GamepadAxis)> {
    AXES.iter().copied()
}

/// Resolves a gamepad button name.
pub fn button_from_name(name: &str) -> Option<GamepadButton> {
    BUTTONS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, button)| *button)
}

/// Resolves a gamepad axis name.
pub fn axis_from_name(name: &str) -> Option<GamepadAxis> {
    AXES.iter().find(|(n, _)| *n == name).map(|(_, axis)| *axis)
}

/// The name of a gamepad button, if it has one.
pub fn button_name(button: GamepadButton) -> Option<&'static str> {
    BUTTONS
        .iter()
        .find(|(_, b)| *b == button)
        .map(|(name, _)| *name)
}

/// The name of a gamepad axis, if it has one.
pub fn axis_name(axis: GamepadAxis) -> Option<&'static str> {
    AXES.iter().find(|(_, a)| *a == axis).map(|(name, _)| *name)
}

/// Validates a gamepad button name, raising `KeyError` for unknown names.
fn check_button(name: &str) -> PyResult<()> {
    if button_from_name(name).is_none() {
        return Err(PyKeyError::new_err(format!(
            "unknown gamepad button {name:?}"
        )));
    }
    Ok(())
}

/// Validates a gamepad axis name, raising `KeyError` for unknown names.
fn check_axis(name: &str) -> PyResult<()> {
    if axis_from_name(name).is_none() {
        return Err(PyKeyError::new_err(format!(
            "unknown gamepad axis {name:?}"
        )));
    }
    Ok(())
}

/// One connected gamepad's state at the start of the frame.
#[pyclass(name = "Gamepad", module = "automata._core", frozen)]
#[derive(Debug, Clone, Default)]
pub struct PyGamepad {
    /// The gamepad's id (stable while connected).
    #[pyo3(get)]
    pub id: usize,
    /// Human-readable name (e.g. "Xbox Wireless Controller").
    #[pyo3(get)]
    pub name: String,
    pub(crate) buttons: HashSet<&'static str>,
    pub(crate) axes: HashMap<&'static str, f32>,
}

#[pymethods]
impl PyGamepad {
    /// Whether the button is currently held.
    fn pressed(&self, button: &str) -> PyResult<bool> {
        check_button(button)?;
        Ok(self.buttons.contains(button))
    }

    /// Current value of the axis (0.0 when resting).
    fn axis(&self, axis: &str) -> PyResult<f32> {
        check_axis(axis)?;
        Ok(self.axes.get(axis).copied().unwrap_or(0.0))
    }

    /// Left stick position as `(x, y)`.
    fn left_stick(&self) -> (f32, f32) {
        (
            self.axes.get("left_stick_x").copied().unwrap_or(0.0),
            self.axes.get("left_stick_y").copied().unwrap_or(0.0),
        )
    }

    /// Right stick position as `(x, y)`.
    fn right_stick(&self) -> (f32, f32) {
        (
            self.axes.get("right_stick_x").copied().unwrap_or(0.0),
            self.axes.get("right_stick_y").copied().unwrap_or(0.0),
        )
    }

    fn __repr__(&self) -> String {
        format!("Gamepad(id={}, name={:?})", self.id, self.name)
    }
}

/// A gamepad event: connection, disconnection, or a button/axis change.
///
/// `kind` is `"connected"`, `"disconnected"`, `"button"`, or `"axis"`;
/// only the fields relevant to the kind are set.
#[pyclass(name = "GamepadEvent", module = "automata._core", frozen)]
#[derive(Debug, Clone, Default)]
pub struct PyGamepadEvent {
    /// The event kind: "connected", "disconnected", "button", or "axis".
    #[pyo3(get)]
    pub kind: &'static str,
    /// Which gamepad.
    #[pyo3(get)]
    pub id: usize,
    /// Device name ("connected" events only).
    #[pyo3(get)]
    pub name: Option<String>,
    /// Button name ("button" events only).
    #[pyo3(get)]
    pub button: Option<&'static str>,
    /// New button state ("button" events only).
    #[pyo3(get)]
    pub pressed: Option<bool>,
    /// Axis name ("axis" events only).
    #[pyo3(get)]
    pub axis: Option<&'static str>,
    /// New axis value ("axis" events only).
    #[pyo3(get)]
    pub value: Option<f32>,
}

impl PyGamepadEvent {
    /// Converts an engine gamepad event, dropping unnameable controls.
    fn from_engine(event: &GamepadEvent) -> Option<Self> {
        let converted = match event {
            GamepadEvent::Connected { id, name } => Self {
                kind: "connected",
                id: usize::from(*id),
                name: Some(name.clone()),
                ..Self::default()
            },
            GamepadEvent::Disconnected { id } => Self {
                kind: "disconnected",
                id: usize::from(*id),
                ..Self::default()
            },
            GamepadEvent::Button {
                id,
                button,
                pressed,
            } => Self::button_event(usize::from(*id), *button, *pressed)?,
            GamepadEvent::Axis { id, axis, value } => {
                Self::axis_event(usize::from(*id), *axis, *value)?
            }
        };
        Some(converted)
    }

    /// A "button" event, if the button has a name.
    fn button_event(id: usize, button: GamepadButton, pressed: bool) -> Option<Self> {
        Some(Self {
            kind: "button",
            id,
            button: Some(button_name(button)?),
            pressed: Some(pressed),
            ..Self::default()
        })
    }

    /// An "axis" event, if the axis has a name.
    fn axis_event(id: usize, axis: GamepadAxis, value: f32) -> Option<Self> {
        Some(Self {
            kind: "axis",
            id,
            axis: Some(axis_name(axis)?),
            value: Some(value),
            ..Self::default()
        })
    }

    /// Converts a batch of engine events.
    pub fn convert_all(events: impl IntoIterator<Item = GamepadEvent>) -> Vec<Self> {
        events
            .into_iter()
            .filter_map(|event| Self::from_engine(&event))
            .collect()
    }
}

#[pymethods]
impl PyGamepadEvent {
    fn __repr__(&self) -> String {
        format!("GamepadEvent(kind={:?}, id={})", self.kind, self.id)
    }
}

/// Gamepad state at the start of the frame.
///
/// Buttons and axes are addressed by lowercase `snake_case` names
/// (`"south"`, `"dpad_up"`, `"left_stick_x"`, ...). The `*_any` methods
/// query across all connected gamepads — the common single-player case.
#[pyclass(name = "Gamepads", module = "automata._core", frozen)]
#[derive(Debug, Clone, Default)]
pub struct PyGamepads {
    pub(crate) pads: Vec<PyGamepad>,
    /// Gamepad events readable this frame.
    #[pyo3(get)]
    pub events: Vec<PyGamepadEvent>,
}

impl PyGamepads {
    /// Snapshots the engine's gamepad state.
    pub fn snapshot(gamepads: &Gamepads, events: Vec<PyGamepadEvent>) -> Self {
        let mut pads: Vec<_> = gamepads
            .connected_ids()
            .map(|id| Self::snapshot_pad(gamepads, id))
            .collect();
        pads.sort_by_key(|pad| pad.id);
        Self { pads, events }
    }

    /// Snapshots one connected gamepad.
    fn snapshot_pad(gamepads: &Gamepads, id: GamepadId) -> PyGamepad {
        let mut pad = PyGamepad {
            id: usize::from(id),
            name: gamepads.name(id).unwrap_or_default(),
            ..PyGamepad::default()
        };
        for (name, button) in all_buttons() {
            if gamepads.pressed(id, button) {
                pad.buttons.insert(name);
            }
        }
        for (name, axis) in all_axes() {
            pad.axes.insert(name, gamepads.axis(id, axis));
        }
        pad
    }

    /// Finds a pad by id.
    fn pad(&self, id: usize) -> Option<&PyGamepad> {
        self.pads.iter().find(|pad| pad.id == id)
    }

    /// Whether a button is held on any pad, by name. Internal: action maps.
    pub(crate) fn has_button_pressed(&self, name: &str) -> bool {
        self.pads.iter().any(|pad| pad.buttons.contains(name))
    }

    /// The primary pad's axis value, by name (0.0 when absent). Internal.
    pub(crate) fn axis_any_value(&self, name: &str) -> f32 {
        self.pads
            .first()
            .and_then(|pad| pad.axes.get(name).copied())
            .unwrap_or(0.0)
    }
}

#[pymethods]
impl PyGamepads {
    /// The ids of all connected gamepads, sorted.
    fn ids(&self) -> Vec<usize> {
        self.pads.iter().map(|pad| pad.id).collect()
    }

    /// The gamepad with `id`, or `None` when not connected.
    fn get(&self, id: usize) -> Option<PyGamepad> {
        self.pad(id).cloned()
    }

    /// The first connected gamepad, if any (single-player convenience).
    fn primary(&self) -> Option<PyGamepad> {
        self.pads.first().cloned()
    }

    /// Whether any gamepad is connected.
    fn any_connected(&self) -> bool {
        !self.pads.is_empty()
    }

    /// Whether the button is held on gamepad `id`.
    fn pressed(&self, id: usize, button: &str) -> PyResult<bool> {
        check_button(button)?;
        Ok(self.pad(id).is_some_and(|pad| pad.buttons.contains(button)))
    }

    /// Whether the button is held on any connected gamepad.
    fn pressed_any(&self, button: &str) -> PyResult<bool> {
        check_button(button)?;
        Ok(self.pads.iter().any(|pad| pad.buttons.contains(button)))
    }

    /// Current value of the axis on gamepad `id` (0.0 when disconnected).
    fn axis(&self, id: usize, axis: &str) -> PyResult<f32> {
        check_axis(axis)?;
        Ok(self
            .pad(id)
            .and_then(|pad| pad.axes.get(axis).copied())
            .unwrap_or(0.0))
    }

    /// Current value of the axis on the primary gamepad (0.0 when none).
    fn axis_any(&self, axis: &str) -> PyResult<f32> {
        check_axis(axis)?;
        Ok(self
            .pads
            .first()
            .and_then(|pad| pad.axes.get(axis).copied())
            .unwrap_or(0.0))
    }

    fn __len__(&self) -> usize {
        self.pads.len()
    }

    /// Iterates over the connected gamepads.
    fn __iter__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let list = pyo3::types::PyList::new(py, self.pads.clone())?;
        Ok(list.as_any().try_iter()?.unbind().into_any())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_entries_round_trip() {
        for (name, button) in all_buttons() {
            assert_eq!(button_from_name(name), Some(button));
            assert_eq!(button_name(button), Some(name));
        }
        for (name, axis) in all_axes() {
            assert_eq!(axis_from_name(name), Some(axis));
            assert_eq!(axis_name(axis), Some(name));
        }
    }

    #[test]
    fn unknown_names_return_none() {
        assert_eq!(button_from_name("not_a_button"), None);
        assert_eq!(axis_from_name("not_an_axis"), None);
    }

    #[test]
    fn empty_snapshot_has_no_pads() {
        let gamepads = Gamepads::new();
        let snapshot = PyGamepads::snapshot(&gamepads, Vec::new());
        // Sandboxes have no hardware; a pad appearing here would come from
        // the host and is still handled gracefully.
        assert_eq!(snapshot.events.len(), 0);
        for pad in &snapshot.pads {
            assert!(snapshot.ids().contains(&pad.id));
        }
    }

    #[test]
    fn events_convert_and_drop_unknown_controls() {
        let button = PyGamepadEvent::button_event(0, GamepadButton::South, true).unwrap();
        assert_eq!(button.kind, "button");
        assert_eq!(button.button, Some("south"));
        assert_eq!(button.pressed, Some(true));
        assert!(PyGamepadEvent::button_event(0, GamepadButton::Unknown, true).is_none());

        let axis = PyGamepadEvent::axis_event(1, GamepadAxis::LeftStickX, 0.5).unwrap();
        assert_eq!(axis.kind, "axis");
        assert!((axis.value.unwrap() - 0.5).abs() < f32::EPSILON);
        assert!(PyGamepadEvent::axis_event(1, GamepadAxis::Unknown, 0.5).is_none());
    }
}
