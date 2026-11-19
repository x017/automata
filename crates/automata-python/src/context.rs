//! The per-frame context handed to the Python update callback.
//!
//! Everything here is an owned snapshot taken at the start of the frame —
//! no references into the engine cross the FFI boundary. Writes (scene
//! commands, outgoing events, the quit flag) are buffered through shared
//! cells and applied by the game loop right after the callback returns.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use automata_input::{Gamepads, InputState};
use pyo3::exceptions::PyKeyError;
use pyo3::prelude::*;

use crate::gamepads::{PyGamepadEvent, PyGamepads};
use crate::keys;
use crate::scene::SceneView;

/// Frame timing information.
#[pyclass(name = "FrameTime", module = "automata._core", frozen)]
#[derive(Debug, Clone, Copy)]
pub struct FrameTime {
    /// Seconds since the previous frame.
    #[pyo3(get)]
    pub delta: f64,
    /// Seconds since the game started.
    #[pyo3(get)]
    pub elapsed: f64,
    /// Number of frames since the game started (the current frame included).
    #[pyo3(get)]
    pub frame: u64,
}

/// Modifier key state at the start of the frame.
///
/// Four independent flags are the natural representation here.
#[allow(clippy::struct_excessive_bools)]
#[pyclass(name = "Modifiers", module = "automata._core", frozen)]
#[derive(Debug, Clone, Copy, Default)]
pub struct PyModifiers {
    /// Whether any shift key is held.
    #[pyo3(get)]
    pub shift: bool,
    /// Whether any control key is held.
    #[pyo3(get)]
    pub ctrl: bool,
    /// Whether any alt key is held.
    #[pyo3(get)]
    pub alt: bool,
    /// Whether any super (logo) key is held.
    #[pyo3(get)]
    pub super_key: bool,
}

/// Keyboard, mouse, and gamepad state at the start of the frame.
///
/// Keys are addressed by lowercase `snake_case` names (`"w"`, `"arrow_left"`,
/// `"f1"`, `"space"`, `"escape"`, ...); mouse buttons by `"left"`,
/// `"right"`, `"middle"`, `"back"`, `"forward"`. Gamepads live under
/// [`gamepads`](PyInput::gamepads).
#[pyclass(name = "Input", module = "automata._core", frozen)]
#[derive(Debug, Clone, Default)]
pub struct PyInput {
    pub(crate) pressed: HashSet<&'static str>,
    pub(crate) just_pressed: HashSet<&'static str>,
    pub(crate) just_released: HashSet<&'static str>,
    pub(crate) buttons_pressed: HashSet<&'static str>,
    pub(crate) buttons_just_pressed: HashSet<&'static str>,
    pub(crate) buttons_just_released: HashSet<&'static str>,
    /// Cursor position in logical pixels, or `None` when the cursor is
    /// outside the window (or in headless mode).
    #[pyo3(get)]
    pub cursor: Option<(f64, f64)>,
    /// Relative mouse movement accumulated this frame, in physical pixels.
    #[pyo3(get)]
    pub mouse_delta: (f64, f64),
    /// Scroll wheel delta accumulated during the frame.
    #[pyo3(get)]
    pub scroll: (f64, f64),
    /// Modifier key state.
    #[pyo3(get)]
    pub modifiers: PyModifiers,
    /// Gamepad state and events.
    #[pyo3(get)]
    pub gamepads: PyGamepads,
}

impl PyInput {
    /// Snapshots an engine input state and the gamepad state.
    pub fn snapshot(
        input: &InputState,
        gamepads: &Gamepads,
        gamepad_events: Vec<PyGamepadEvent>,
    ) -> Self {
        let modifiers = input.modifiers();
        let mut snapshot = Self {
            cursor: input.cursor_position(),
            mouse_delta: input.mouse_delta(),
            scroll: input.scroll_delta(),
            modifiers: PyModifiers {
                shift: modifiers.shift_key(),
                ctrl: modifiers.control_key(),
                alt: modifiers.alt_key(),
                super_key: modifiers.super_key(),
            },
            gamepads: PyGamepads::snapshot(gamepads, gamepad_events),
            ..Self::default()
        };
        for (name, code) in keys::all_keys() {
            if input.pressed(code) {
                snapshot.pressed.insert(name);
            }
            if input.just_pressed(code) {
                snapshot.just_pressed.insert(name);
            }
            if input.just_released(code) {
                snapshot.just_released.insert(name);
            }
        }
        for (name, button) in keys::all_buttons() {
            if input.button_pressed(button) {
                snapshot.buttons_pressed.insert(name);
            }
            if input.button_just_pressed(button) {
                snapshot.buttons_just_pressed.insert(name);
            }
            if input.button_just_released(button) {
                snapshot.buttons_just_released.insert(name);
            }
        }
        snapshot
    }
}

impl PyInput {
    /// Whether a key is held, by name. Internal: action-map evaluation.
    pub(crate) fn has_key_pressed(&self, name: &str) -> bool {
        self.pressed.contains(name)
    }

    /// Whether a key was pressed this frame, by name. Internal.
    pub(crate) fn has_key_just_pressed(&self, name: &str) -> bool {
        self.just_pressed.contains(name)
    }

    /// Whether a mouse button is held, by name. Internal.
    pub(crate) fn has_button_pressed(&self, name: &str) -> bool {
        self.buttons_pressed.contains(name)
    }

    /// Whether a mouse button was pressed this frame, by name. Internal.
    pub(crate) fn has_button_just_pressed(&self, name: &str) -> bool {
        self.buttons_just_pressed.contains(name)
    }

    /// Marks a key as held. Test-only: snapshot construction for unit tests.
    #[cfg(test)]
    pub(crate) fn press_key_for_test(&mut self, name: &'static str) {
        self.pressed.insert(name);
    }

    /// Marks a key as newly pressed this frame. Test-only.
    #[cfg(test)]
    pub(crate) fn press_key_just_for_test(&mut self, name: &'static str) {
        self.just_pressed.insert(name);
    }
}

/// Validates a key name, raising `KeyError` for unknown names.
fn check_key(name: &str) -> PyResult<()> {
    if keys::key_from_name(name).is_none() {
        return Err(PyKeyError::new_err(format!("unknown key {name:?}")));
    }
    Ok(())
}

/// Validates a mouse button name, raising `KeyError` for unknown names.
fn check_button(name: &str) -> PyResult<()> {
    if keys::button_from_name(name).is_none() {
        return Err(PyKeyError::new_err(format!(
            "unknown mouse button {name:?}"
        )));
    }
    Ok(())
}

#[pymethods]
impl PyInput {
    /// Whether the key is currently held.
    fn pressed(&self, key: &str) -> PyResult<bool> {
        check_key(key)?;
        Ok(self.pressed.contains(key))
    }

    /// Whether the key was pressed during this frame.
    fn just_pressed(&self, key: &str) -> PyResult<bool> {
        check_key(key)?;
        Ok(self.just_pressed.contains(key))
    }

    /// Whether the key was released during this frame.
    fn just_released(&self, key: &str) -> PyResult<bool> {
        check_key(key)?;
        Ok(self.just_released.contains(key))
    }

    /// Whether the mouse button is currently held.
    fn mouse_pressed(&self, button: &str) -> PyResult<bool> {
        check_button(button)?;
        Ok(self.buttons_pressed.contains(button))
    }

    /// Whether the mouse button was pressed during this frame.
    fn mouse_just_pressed(&self, button: &str) -> PyResult<bool> {
        check_button(button)?;
        Ok(self.buttons_just_pressed.contains(button))
    }

    /// Whether the mouse button was released during this frame.
    fn mouse_just_released(&self, button: &str) -> PyResult<bool> {
        check_button(button)?;
        Ok(self.buttons_just_released.contains(button))
    }
}

/// Shared store for Python-level resources (arbitrary Python objects,
/// keyed by name). Owned by the game; the per-frame view clones the handle.
pub type ResourceStore = Rc<RefCell<HashMap<String, Py<PyAny>>>>;

/// Named, game-owned storage for arbitrary Python objects.
///
/// Resources persist across frames and are destroyed with the game. Unlike
/// the scene view, reads and writes take effect immediately.
#[pyclass(name = "Resources", unsendable, module = "automata._core")]
#[derive(Debug, Clone)]
pub struct PyResources {
    store: ResourceStore,
}

impl PyResources {
    /// Wraps the game's resource store.
    pub fn new(store: ResourceStore) -> Self {
        Self { store }
    }
}

#[pymethods]
impl PyResources {
    /// Stores a value under `name`, replacing any previous value.
    fn set(&self, name: &str, value: Py<PyAny>) {
        self.store.borrow_mut().insert(name.to_owned(), value);
    }

    /// The value stored under `name`, or `None`.
    fn get(&self, py: Python<'_>, name: &str) -> Option<Py<PyAny>> {
        self.store
            .borrow()
            .get(name)
            .map(|value| value.clone_ref(py))
    }

    /// Removes and returns the value stored under `name`, or `None`.
    fn remove(&self, py: Python<'_>, name: &str) -> Option<Py<PyAny>> {
        let _ = py;
        self.store.borrow_mut().remove(name)
    }

    /// Removes all resources.
    fn clear(&self) {
        self.store.borrow_mut().clear();
    }

    /// The names of all stored resources.
    fn names(&self) -> Vec<String> {
        self.store.borrow().keys().cloned().collect()
    }

    fn __contains__(&self, name: &str) -> bool {
        self.store.borrow().contains_key(name)
    }

    fn __len__(&self) -> usize {
        self.store.borrow().len()
    }
}

/// Shared quit flag between the context and the game loop.
pub type QuitFlag = Rc<Cell<bool>>;

/// The per-frame update context: timing, input, scene, events, resources,
/// and quit control.
///
/// Valid only inside the update callback; reads observe start-of-frame
/// state and writes are applied right after the callback returns.
#[pyclass(name = "FrameContext", unsendable, module = "automata._core")]
#[derive(Debug, Clone)]
pub struct FrameContext {
    /// Frame timing.
    #[pyo3(get)]
    pub time: FrameTime,
    /// Keyboard and mouse state.
    #[pyo3(get)]
    pub input: PyInput,
    /// Scene read view and write buffer.
    #[pyo3(get)]
    pub scene: SceneView,
    /// Event read view and send buffer.
    #[pyo3(get)]
    pub events: crate::events::PyEvents,
    /// 3D physics queries (raycasts, impulses, grounded).
    #[pyo3(get)]
    pub physics: crate::physics::PyPhysics,
    /// 2D physics queries.
    #[pyo3(get)]
    pub physics2d: crate::physics::PyPhysics2D,
    /// Audio playback and control.
    #[pyo3(get)]
    pub audio: crate::audio::PyAudio,
    /// Named Python-object storage.
    #[pyo3(get)]
    pub resources: PyResources,
    quit: QuitFlag,
}

impl FrameContext {
    /// Assembles the frame context from its parts.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        time: FrameTime,
        input: PyInput,
        scene: SceneView,
        events: crate::events::PyEvents,
        resources: PyResources,
        quit: QuitFlag,
        physics: crate::physics::PyPhysics,
        physics2d: crate::physics::PyPhysics2D,
        audio: crate::audio::PyAudio,
    ) -> Self {
        Self {
            time,
            input,
            scene,
            events,
            physics,
            physics2d,
            audio,
            resources,
            quit,
        }
    }
}

#[pymethods]
impl FrameContext {
    /// Requests game shutdown; the loop stops after this frame.
    fn quit(&self) {
        self.quit.set(true);
    }
}
