//! AUTOMATA in the browser: the engine's Python module, `automata._core`,
//! built for Pyodide.
//!
//! A browser page can't sit in a game loop; it gets called back once per
//! animation frame. So importing `automata` here installs a *step driver*
//! ([`driver::step`]) instead of a window driver: `game.run()` starts the
//! game and returns, and the page (`web/player/player.js`) then calls
//! [`web_frame`] from each `requestAnimationFrame` until it returns
//! `False`. Pyodide's canvas (`pyodide.canvas.setCanvas3D`) is where
//! WebGL2 draws; scripts don't change.
//!
//! Between frames the page reports input with the `web_*` functions below
//! (keys by `KeyboardEvent.code`, positions in canvas pixels), polls the
//! gamepads into [`web_gamepads`], reads [`web_cursor_locked`] to lock or
//! free the pointer, and takes the frame's mixed sound from [`web_audio`]
//! for its `AudioWorklet`.

mod driver;

use automata_input::{ElementState, InputEvent, KeyCode, ModifiersState, MouseButton, TouchPhase};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

/// Runs one frame of the running game at the canvas's current drawing
/// size (`canvas.width`, `canvas.height`), with the input the page queued
/// since the last one. Returns whether the game goes on. Raises the
/// game's exception, or `automata.Error` when WebGL couldn't start.
#[pyfunction]
fn web_frame(py: Python<'_>, width: u32, height: u32) -> PyResult<bool> {
    driver::set_canvas_size(width, height);
    let running = automata_python::step_running_game(py)?;
    if let Some(message) = driver::take_error() {
        return Err(automata_python::engine_error(&message));
    }
    Ok(running)
}

fn element_state(down: bool) -> ElementState {
    if down {
        ElementState::Pressed
    } else {
        ElementState::Released
    }
}

/// A key went down (`down`, again on key repeat) or up, by its
/// `KeyboardEvent.code` (`"KeyW"`, `"ArrowLeft"`). Returns whether the
/// engine knows the key, so the page can leave other keys to the browser.
#[pyfunction]
fn web_key(code: &str, down: bool) -> bool {
    let Some(key) = KeyCode::from_code(code) else {
        return false;
    };
    driver::queue_input(InputEvent::Key {
        key,
        state: element_state(down),
    });
    true
}

/// The held modifiers, as the page's bits: 1 Shift, 2 Ctrl, 4 Alt, 8 Meta.
#[pyfunction]
fn web_modifiers(bits: u8) {
    let held = |bit: u8| bits & bit != 0;
    let state = ModifiersState::empty()
        .with(ModifiersState::SHIFT, held(1))
        .with(ModifiersState::CONTROL, held(2))
        .with(ModifiersState::ALT, held(4))
        .with(ModifiersState::SUPER, held(8));
    driver::queue_input(InputEvent::Modifiers(state));
}

/// A mouse button went down or up, by `MouseEvent.button` (0 left, 1
/// middle, 2 right, 3 back, 4 forward).
#[pyfunction]
fn web_mouse_button(button: u16, down: bool) {
    let button = match button {
        0 => MouseButton::Left,
        1 => MouseButton::Middle,
        2 => MouseButton::Right,
        3 => MouseButton::Back,
        4 => MouseButton::Forward,
        other => MouseButton::Other(other),
    };
    driver::queue_input(InputEvent::MouseButton {
        button,
        state: element_state(down),
    });
}

/// The cursor moved to `(x, y)` canvas pixels (from the top left).
#[pyfunction]
fn web_cursor(x: f64, y: f64) {
    driver::queue_input(InputEvent::CursorMoved { x, y });
}

/// The mouse moved by `(dx, dy)` (`movementX/Y`; keeps coming while the
/// pointer is locked).
#[pyfunction]
fn web_mouse_motion(dx: f64, dy: f64) {
    driver::queue_input(InputEvent::MouseMotion { dx, dy });
}

/// The wheel turned: about one per notch, +y away from the user.
#[pyfunction]
fn web_scroll(x: f64, y: f64) {
    driver::queue_input(InputEvent::Scroll { x, y });
}

/// A finger (`id`: `PointerEvent.pointerId`) touched (phase 0), moved
/// (1), lifted (2) or was taken away (3) at `(x, y)` canvas pixels.
#[pyfunction]
fn web_touch(id: u64, phase: u8, x: f64, y: f64) {
    let phase = match phase {
        0 => TouchPhase::Started,
        1 => TouchPhase::Moved,
        2 => TouchPhase::Ended,
        _ => TouchPhase::Cancelled,
    };
    driver::queue_input(InputEvent::Touch { id, phase, x, y });
}

/// The page lost focus or was hidden: every held key and button lets go.
#[pyfunction]
fn web_focus_lost() {
    driver::queue_input(InputEvent::FocusLost);
}

/// The page is visible again: the time it was hidden isn't a frame.
#[pyfunction]
fn web_resume() {
    driver::skip_time();
}

/// The gamepads `navigator.getGamepads()` shows this frame, each as
/// `[index, id, [pressed…], [axis…]]` in the standard mapping (the page
/// converts them with `pyodide.toPy`, so they arrive as lists).
#[cfg(target_os = "emscripten")]
#[pyfunction]
fn web_gamepads(pads: Vec<Bound<'_, PyAny>>) -> PyResult<()> {
    let mut snapshots = Vec::with_capacity(pads.len());
    for pad in &pads {
        snapshots.push(automata_input::PadSnapshot {
            index: pad.get_item(0)?.extract()?,
            name: pad.get_item(1)?.extract()?,
            buttons: pad.get_item(2)?.extract()?,
            axes: pad.get_item(3)?.extract()?,
        });
    }
    driver::set_gamepads(snapshots);
    Ok(())
}

/// Whether the game wants the pointer locked (the page asks the browser
/// on the next click, and lets go when this turns false).
#[pyfunction]
fn web_cursor_locked() -> bool {
    driver::cursor_locked()
}

/// The sound mixed since the last call: interleaved stereo `float32`
/// samples (little-endian bytes) at [`web_sample_rate`] Hz.
#[pyfunction]
fn web_audio(py: Python<'_>) -> Bound<'_, PyBytes> {
    let mut samples = Vec::new();
    automata_audio::stream::take_output(&mut samples);
    let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    PyBytes::new(py, &bytes)
}

/// The sample rate [`web_audio`] mixes at.
#[pyfunction]
fn web_sample_rate() -> u32 {
    automata_audio::stream::STREAM_SAMPLE_RATE
}

/// The native extension module imported as `automata._core`.
#[allow(clippy::used_underscore_items)] // the Python module path dictates the name
#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    automata_python::install_step_driver(Some(driver::step));
    automata_python::init_module(m)?;
    m.add_function(wrap_pyfunction!(web_frame, m)?)?;
    m.add_function(wrap_pyfunction!(web_key, m)?)?;
    m.add_function(wrap_pyfunction!(web_modifiers, m)?)?;
    m.add_function(wrap_pyfunction!(web_mouse_button, m)?)?;
    m.add_function(wrap_pyfunction!(web_cursor, m)?)?;
    m.add_function(wrap_pyfunction!(web_mouse_motion, m)?)?;
    m.add_function(wrap_pyfunction!(web_scroll, m)?)?;
    m.add_function(wrap_pyfunction!(web_touch, m)?)?;
    m.add_function(wrap_pyfunction!(web_focus_lost, m)?)?;
    m.add_function(wrap_pyfunction!(web_resume, m)?)?;
    #[cfg(target_os = "emscripten")]
    m.add_function(wrap_pyfunction!(web_gamepads, m)?)?;
    m.add_function(wrap_pyfunction!(web_cursor_locked, m)?)?;
    m.add_function(wrap_pyfunction!(web_audio, m)?)?;
    m.add_function(wrap_pyfunction!(web_sample_rate, m)?)?;
    m.add("platform", "web")?;
    Ok(())
}
