//! Action-based input mapping for Python games.
//!
//! Games name their actions ("jump", "fire") and axes (`move_x`) and bind
//! them to physical controls — keyboard keys, mouse buttons, gamepad
//! buttons and axes — so game code is device-independent and controls are
//! remappable at runtime. Evaluation happens against the frame's input
//! snapshot ([`crate::context::PyInput`]), keeping the engine's start-of-
//! frame semantics.
//!
//! Semantics mirror `automata_input::ActionMap`: axis-as-button bindings
//! trigger past a 0.5 threshold, and `just_pressed` on gamepad bindings
//! falls back to `pressed` (device state has no edge for pads).
//!
//! ```python
//! import automata as at
//!
//! actions = at.ActionMap()
//! actions.bind("jump", at.Binding.key("space"))
//! actions.bind("jump", at.Binding.gamepad_button("south"))
//! actions.bind_axis("move_x", at.AxisBinding.key_pair("a", "d"))
//!
//! # inside the update callback:
//! if actions.just_pressed("jump", ctx.input): ...
//! dx = actions.axis("move_x", ctx.input)
//! ```

use std::collections::HashMap;

use pyo3::exceptions::PyKeyError;
use pyo3::prelude::*;

use crate::context::PyInput;
use crate::{gamepads, keys};

/// Axis value past which an axis-as-button binding counts as active.
const AXIS_THRESHOLD: f32 = 0.5;

/// What kind of physical control a [`PyBinding`] refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindingKind {
    Key,
    Mouse,
    GamepadButton,
    GamepadAxisPositive,
    GamepadAxisNegative,
}

impl BindingKind {
    /// The kind's Python-facing name.
    fn name(self) -> &'static str {
        match self {
            Self::Key => "key",
            Self::Mouse => "mouse",
            Self::GamepadButton => "gamepad_button",
            Self::GamepadAxisPositive => "gamepad_axis_positive",
            Self::GamepadAxisNegative => "gamepad_axis_negative",
        }
    }
}

/// A physical input that can trigger an action.
///
/// Construct with the class methods: `Binding.key("space")`,
/// `Binding.mouse("left")`, `Binding.gamepad_button("south")`,
/// `Binding.gamepad_axis_positive("left_stick_x")`, ...
#[pyclass(name = "Binding", module = "automata._core", frozen)]
#[derive(Debug, Clone)]
pub struct PyBinding {
    kind: BindingKind,
    /// The control's name (key, mouse button, or gamepad control).
    #[pyo3(get)]
    pub name: String,
}

#[pymethods]
impl PyBinding {
    /// The binding kind: "key", "mouse", `gamepad_button`,
    /// `gamepad_axis_positive`, or `gamepad_axis_negative`.
    #[getter]
    fn kind(&self) -> &'static str {
        self.kind.name()
    }

    /// A keyboard key binding (`Binding.key("space")`).
    #[classmethod]
    fn key(_cls: &Bound<'_, pyo3::types::PyType>, name: &str) -> PyResult<Self> {
        if keys::key_from_name(name).is_none() {
            return Err(PyKeyError::new_err(format!("unknown key {name:?}")));
        }
        Ok(Self {
            kind: BindingKind::Key,
            name: name.to_owned(),
        })
    }

    /// A mouse button binding (`Binding.mouse("left")`).
    #[classmethod]
    fn mouse(_cls: &Bound<'_, pyo3::types::PyType>, name: &str) -> PyResult<Self> {
        if keys::button_from_name(name).is_none() {
            return Err(PyKeyError::new_err(format!(
                "unknown mouse button {name:?}"
            )));
        }
        Ok(Self {
            kind: BindingKind::Mouse,
            name: name.to_owned(),
        })
    }

    /// A gamepad button binding, any connected pad (`Binding.gamepad_button("south")`).
    #[classmethod]
    fn gamepad_button(_cls: &Bound<'_, pyo3::types::PyType>, name: &str) -> PyResult<Self> {
        if gamepads::button_from_name(name).is_none() {
            return Err(PyKeyError::new_err(format!(
                "unknown gamepad button {name:?}"
            )));
        }
        Ok(Self {
            kind: BindingKind::GamepadButton,
            name: name.to_owned(),
        })
    }

    /// A gamepad axis past +0.5 counts as pressed (e.g. analog triggers).
    #[classmethod]
    fn gamepad_axis_positive(_cls: &Bound<'_, pyo3::types::PyType>, name: &str) -> PyResult<Self> {
        Self::axis_binding(name, BindingKind::GamepadAxisPositive)
    }

    /// A gamepad axis past -0.5 counts as pressed.
    #[classmethod]
    fn gamepad_axis_negative(_cls: &Bound<'_, pyo3::types::PyType>, name: &str) -> PyResult<Self> {
        Self::axis_binding(name, BindingKind::GamepadAxisNegative)
    }

    fn __repr__(&self) -> String {
        format!("Binding.{}({:?})", self.kind.name(), self.name)
    }
}

impl PyBinding {
    /// Shared constructor for the two axis-as-button kinds.
    fn axis_binding(name: &str, kind: BindingKind) -> PyResult<Self> {
        if gamepads::axis_from_name(name).is_none() {
            return Err(PyKeyError::new_err(format!(
                "unknown gamepad axis {name:?}"
            )));
        }
        Ok(Self {
            kind,
            name: name.to_owned(),
        })
    }

    /// Whether the binding is active in the given input snapshot.
    fn active(&self, input: &PyInput) -> bool {
        match self.kind {
            BindingKind::Key => input.has_key_pressed(&self.name),
            BindingKind::Mouse => input.has_button_pressed(&self.name),
            BindingKind::GamepadButton => input.gamepads.has_button_pressed(&self.name),
            BindingKind::GamepadAxisPositive => {
                input.gamepads.axis_any_value(&self.name) >= AXIS_THRESHOLD
            }
            BindingKind::GamepadAxisNegative => {
                input.gamepads.axis_any_value(&self.name) <= -AXIS_THRESHOLD
            }
        }
    }

    /// Whether the binding became active this frame.
    ///
    /// Gamepad bindings have no edge in the snapshot, so they fall back to
    /// [`active`](Self::active) — matching `automata_input::ActionMap`.
    fn just_active(&self, input: &PyInput) -> bool {
        match self.kind {
            BindingKind::Key => input.has_key_just_pressed(&self.name),
            BindingKind::Mouse => input.has_button_just_pressed(&self.name),
            _ => self.active(input),
        }
    }
}

/// What kind of analog source a [`PyAxisBinding`] refers to.
#[derive(Debug, Clone)]
enum AxisKind {
    GamepadAxis {
        name: String,
        scale: f32,
    },
    ButtonPair {
        negative: PyBinding,
        positive: PyBinding,
    },
}

/// An analog axis source for [`PyActionMap::axis`].
///
/// Construct with the class methods: `AxisBinding.gamepad_axis("left_stick_x")`,
/// `AxisBinding.key_pair("a", "d")`, `AxisBinding.button_pair(neg, pos)`.
#[pyclass(name = "AxisBinding", module = "automata._core", frozen)]
#[derive(Debug, Clone)]
pub struct PyAxisBinding {
    kind: AxisKind,
}

#[pymethods]
impl PyAxisBinding {
    /// The binding kind: `gamepad_axis` or `button_pair`.
    #[getter]
    fn kind(&self) -> &'static str {
        match &self.kind {
            AxisKind::GamepadAxis { .. } => "gamepad_axis",
            AxisKind::ButtonPair { .. } => "button_pair",
        }
    }

    /// The gamepad axis name (`gamepad_axis` kind only).
    #[getter]
    fn name(&self) -> Option<&str> {
        match &self.kind {
            AxisKind::GamepadAxis { name, .. } => Some(name),
            AxisKind::ButtonPair { .. } => None,
        }
    }

    /// The axis scale (`gamepad_axis` kind only).
    #[getter]
    fn scale(&self) -> Option<f32> {
        match &self.kind {
            AxisKind::GamepadAxis { scale, .. } => Some(*scale),
            AxisKind::ButtonPair { .. } => None,
        }
    }

    /// The negative-direction binding (`button_pair` kind only).
    #[getter]
    fn negative(&self) -> Option<PyBinding> {
        match &self.kind {
            AxisKind::ButtonPair { negative, .. } => Some(negative.clone()),
            AxisKind::GamepadAxis { .. } => None,
        }
    }

    /// The positive-direction binding (`button_pair` kind only).
    #[getter]
    fn positive(&self) -> Option<PyBinding> {
        match &self.kind {
            AxisKind::ButtonPair { positive, .. } => Some(positive.clone()),
            AxisKind::GamepadAxis { .. } => None,
        }
    }

    /// A gamepad axis with a scale multiplier (use -1.0 to invert).
    #[classmethod]
    #[pyo3(signature = (name, scale=1.0))]
    fn gamepad_axis(
        _cls: &Bound<'_, pyo3::types::PyType>,
        name: &str,
        scale: f32,
    ) -> PyResult<Self> {
        if gamepads::axis_from_name(name).is_none() {
            return Err(PyKeyError::new_err(format!(
                "unknown gamepad axis {name:?}"
            )));
        }
        Ok(Self {
            kind: AxisKind::GamepadAxis {
                name: name.to_owned(),
                scale,
            },
        })
    }

    /// Two action-like bindings forming an axis: positive minus negative.
    #[classmethod]
    #[allow(clippy::needless_pass_by_value)] // pyo3 extraction needs PyRef by value
    fn button_pair(
        _cls: &Bound<'_, pyo3::types::PyType>,
        negative: PyRef<'_, PyBinding>,
        positive: PyRef<'_, PyBinding>,
    ) -> Self {
        Self {
            kind: AxisKind::ButtonPair {
                negative: negative.clone(),
                positive: positive.clone(),
            },
        }
    }

    /// WASD-style: two keys forming an axis (`AxisBinding.key_pair("a", "d")`).
    #[classmethod]
    fn key_pair(
        cls: &Bound<'_, pyo3::types::PyType>,
        negative: &str,
        positive: &str,
    ) -> PyResult<Self> {
        Ok(Self {
            kind: AxisKind::ButtonPair {
                negative: PyBinding::key(cls, negative)?,
                positive: PyBinding::key(cls, positive)?,
            },
        })
    }

    fn __repr__(&self) -> String {
        match &self.kind {
            AxisKind::GamepadAxis { name, scale } => {
                format!("AxisBinding.gamepad_axis({name:?}, {scale})")
            }
            AxisKind::ButtonPair { negative, positive } => format!(
                "AxisBinding.button_pair({negative:?}, {positive:?})",
                negative = negative.__repr__(),
                positive = positive.__repr__(),
            ),
        }
    }
}

impl PyAxisBinding {
    /// The axis value in the given input snapshot, -1.0..=1.0 (scaled).
    fn value(&self, input: &PyInput) -> f32 {
        match &self.kind {
            AxisKind::GamepadAxis { name, scale } => input.gamepads.axis_any_value(name) * scale,
            AxisKind::ButtonPair { negative, positive } => {
                let neg = i8::from(negative.active(input));
                let pos = i8::from(positive.active(input));
                f32::from(pos - neg)
            }
        }
    }
}

/// Named actions and axes bound to physical controls.
///
/// The map is game-owned: create it before `game.run()` (or store it in
/// `ctx.resources`), then query it inside the update callback against
/// `ctx.input`. Bindings are changeable at any time, so control-remapping
/// UIs can live entirely in game code.
#[pyclass(name = "ActionMap", module = "automata._core")]
#[derive(Debug, Default)]
pub struct PyActionMap {
    actions: HashMap<String, Vec<PyBinding>>,
    axes: HashMap<String, Vec<PyAxisBinding>>,
}

#[pymethods]
impl PyActionMap {
    /// An empty action map.
    #[new]
    fn new() -> Self {
        Self::default()
    }

    /// Binds `action` to a physical input (stacking with existing bindings).
    #[allow(clippy::needless_pass_by_value)] // pyo3 extraction needs owned PyRef
    fn bind(&mut self, action: &str, binding: PyRef<'_, PyBinding>) {
        self.bind_inner(action, binding.clone());
    }

    /// Binds a named analog axis (stacking with existing bindings).
    ///
    /// Multiple bindings for one axis — a WASD pair, an arrow-key pair,
    /// and a gamepad stick — are summed and clamped to -1.0..=1.0, so
    /// every bound device can drive it.
    #[allow(clippy::needless_pass_by_value)] // pyo3 extraction needs owned PyRef
    fn bind_axis(&mut self, name: &str, axis: PyRef<'_, PyAxisBinding>) {
        self.bind_axis_inner(name, axis.clone());
    }

    /// Removes all bindings for the axis `name`.
    fn unbind_axis(&mut self, name: &str) {
        self.axes.remove(name);
    }

    /// Replaces an action's bindings (for control-remapping UIs).
    #[allow(clippy::needless_pass_by_value)] // pyo3 extraction needs owned values
    fn rebind(&mut self, action: &str, bindings: Vec<PyRef<'_, PyBinding>>) {
        self.rebind_inner(action, bindings.iter().map(|b| (*b).clone()).collect());
    }

    /// Removes all bindings for `action`.
    fn unbind(&mut self, action: &str) {
        self.actions.remove(action);
    }

    /// The bindings for an action.
    fn bindings(&self, action: &str) -> Vec<PyBinding> {
        self.actions.get(action).cloned().unwrap_or_default()
    }

    /// All action names currently bound.
    fn actions(&self) -> Vec<String> {
        self.actions.keys().cloned().collect()
    }

    /// All axis names currently bound.
    fn axis_names(&self) -> Vec<String> {
        self.axes.keys().cloned().collect()
    }

    /// Whether any binding for `action` is active in `input`.
    fn pressed(&self, action: &str, input: &PyInput) -> bool {
        self.actions
            .get(action)
            .is_some_and(|bindings| bindings.iter().any(|b| b.active(input)))
    }

    /// Whether any binding for `action` became active this frame.
    ///
    /// Gamepad bindings fall back to `pressed` (the snapshot has no pad
    /// edges), matching `automata_input::ActionMap`.
    fn just_pressed(&self, action: &str, input: &PyInput) -> bool {
        self.actions
            .get(action)
            .is_some_and(|bindings| bindings.iter().any(|b| b.just_active(input)))
    }

    /// Current value of a named axis, -1.0..=1.0 (0.0 when unbound).
    ///
    /// All bindings stacked under `name` contribute; the sum is clamped.
    fn axis(&self, name: &str, input: &PyInput) -> f32 {
        let value: f32 = self
            .axes
            .get(name)
            .into_iter()
            .flatten()
            .map(|axis| axis.value(input))
            .sum();
        value.clamp(-1.0, 1.0)
    }

    /// Convenience: a 2D move vector from two named axes.
    fn axis_pair(&self, x: &str, y: &str, input: &PyInput) -> (f32, f32) {
        (self.axis(x, input), self.axis(y, input))
    }

    fn __len__(&self) -> usize {
        self.actions.len()
    }

    fn __contains__(&self, action: &str) -> bool {
        self.actions.contains_key(action)
    }
}

impl PyActionMap {
    /// `bind` on plain Rust data; the pymethod borrows and clones its
    /// argument, unit tests call this directly.
    pub(crate) fn bind_inner(&mut self, action: &str, binding: PyBinding) {
        self.actions
            .entry(action.to_owned())
            .or_default()
            .push(binding);
    }

    /// `bind_axis` on plain Rust data.
    pub(crate) fn bind_axis_inner(&mut self, name: &str, axis: PyAxisBinding) {
        self.axes.entry(name.to_owned()).or_default().push(axis);
    }

    /// `rebind` on plain Rust data.
    pub(crate) fn rebind_inner(&mut self, action: &str, bindings: Vec<PyBinding>) {
        self.actions.insert(action.to_owned(), bindings);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gamepads::{PyGamepad, PyGamepads};

    /// Builds a key binding without needing the Python runtime.
    fn key(name: &str) -> PyBinding {
        PyBinding {
            kind: BindingKind::Key,
            name: name.to_owned(),
        }
    }

    /// An input snapshot with the given keys held / newly pressed.
    fn input_with(
        pressed: &[&'static str],
        just_pressed: &[&'static str],
        pads: Vec<PyGamepad>,
    ) -> PyInput {
        let mut input = PyInput::default();
        for name in pressed {
            input.press_key_for_test(name);
        }
        for name in just_pressed {
            input.press_key_just_for_test(name);
        }
        input.gamepads = PyGamepads {
            pads,
            events: Vec::new(),
        };
        input
    }

    #[test]
    fn key_action_resolves_against_snapshot() {
        let mut map = PyActionMap::new();
        map.bind_inner("jump", key("space"));

        let idle = PyInput::default();
        assert!(!map.pressed("jump", &idle));
        assert!(!map.pressed("fire", &idle));

        let held = input_with(&["space"], &[], Vec::new());
        assert!(map.pressed("jump", &held));
        assert!(!map.just_pressed("jump", &held));

        let tapped = input_with(&["space"], &["space"], Vec::new());
        assert!(map.just_pressed("jump", &tapped));
    }

    #[test]
    fn key_pair_axis_resolves() {
        let mut map = PyActionMap::new();
        map.bind_axis_inner(
            "move_x",
            PyAxisBinding {
                kind: AxisKind::ButtonPair {
                    negative: key("a"),
                    positive: key("d"),
                },
            },
        );

        let right = input_with(&["d"], &[], Vec::new());
        assert_eq!(map.axis("move_x", &right), 1.0);

        let both = input_with(&["a", "d"], &[], Vec::new());
        assert_eq!(map.axis("move_x", &both), 0.0);

        let idle = PyInput::default();
        assert_eq!(map.axis("move_x", &idle), 0.0);
        assert_eq!(map.axis("unbound", &idle), 0.0);
    }

    #[test]
    fn axis_bindings_stack_and_clamp() {
        let mut map = PyActionMap::new();
        // A WASD pair and an arrow-key pair under one name both drive it.
        map.bind_axis_inner(
            "move_x",
            PyAxisBinding {
                kind: AxisKind::ButtonPair {
                    negative: key("a"),
                    positive: key("d"),
                },
            },
        );
        map.bind_axis_inner(
            "move_x",
            PyAxisBinding {
                kind: AxisKind::ButtonPair {
                    negative: key("arrow_left"),
                    positive: key("arrow_right"),
                },
            },
        );
        assert_eq!(map.axis_names(), vec!["move_x".to_owned()]);

        // Two positives sum to 2 and clamp to 1.
        let both = input_with(&["d", "arrow_right"], &[], Vec::new());
        assert_eq!(map.axis("move_x", &both), 1.0);

        let arrows = input_with(&["arrow_right"], &[], Vec::new());
        assert_eq!(map.axis("move_x", &arrows), 1.0);

        let opposing = input_with(&["a", "arrow_right"], &[], Vec::new());
        assert_eq!(map.axis("move_x", &opposing), 0.0);

        map.unbind_axis("move_x");
        assert_eq!(map.axis("move_x", &arrows), 0.0);
        assert_eq!(map.axis_names(), Vec::<String>::new());
    }

    #[test]
    fn gamepad_button_binding_uses_any_pad() {
        let mut pad = PyGamepad {
            id: 0,
            name: "Test Pad".to_owned(),
            ..PyGamepad::default()
        };
        pad.buttons.insert("south");

        let mut map = PyActionMap::new();
        map.bind_inner(
            "jump",
            PyBinding {
                kind: BindingKind::GamepadButton,
                name: "south".to_owned(),
            },
        );

        let input = input_with(&[], &[], vec![pad]);
        assert!(map.pressed("jump", &input));
        // Gamepad bindings fall back to pressed for just_pressed.
        assert!(map.just_pressed("jump", &input));

        let idle = PyInput::default();
        assert!(!map.pressed("jump", &idle));
    }

    #[test]
    fn gamepad_axis_bindings_use_threshold_and_scale() {
        let mut pad = PyGamepad {
            id: 0,
            name: "Test Pad".to_owned(),
            ..PyGamepad::default()
        };
        pad.axes.insert("left_stick_x", 0.8);
        pad.axes.insert("right_z", 0.9);

        let mut map = PyActionMap::new();
        map.bind_inner(
            "right",
            PyBinding {
                kind: BindingKind::GamepadAxisPositive,
                name: "left_stick_x".to_owned(),
            },
        );
        map.bind_inner(
            "left",
            PyBinding {
                kind: BindingKind::GamepadAxisNegative,
                name: "left_stick_x".to_owned(),
            },
        );
        map.bind_axis_inner(
            "look",
            PyAxisBinding {
                kind: AxisKind::GamepadAxis {
                    name: "left_stick_x".to_owned(),
                    scale: 0.5,
                },
            },
        );

        let input = input_with(&[], &[], vec![pad]);
        assert!(map.pressed("right", &input));
        assert!(!map.pressed("left", &input));
        assert!((map.axis("look", &input) - 0.4).abs() < f32::EPSILON);
    }

    #[test]
    fn rebind_and_unbind_update_bookkeeping() {
        let mut map = PyActionMap::new();
        map.bind_inner("jump", key("space"));
        assert_eq!(map.actions(), vec!["jump".to_owned()]);
        assert_eq!(map.bindings("jump").len(), 1);

        map.rebind_inner("jump", vec![key("enter"), key("w")]);
        assert_eq!(map.bindings("jump").len(), 2);
        assert!(map.pressed("jump", &input_with(&["w"], &[], Vec::new())));
        assert!(!map.pressed("jump", &input_with(&["space"], &[], Vec::new())));

        map.unbind("jump");
        assert_eq!(map.actions(), Vec::<String>::new());
        assert!(!map.pressed("jump", &input_with(&["w"], &[], Vec::new())));
    }
}
