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

mod driver;

use pyo3::prelude::*;

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

/// The native extension module imported as `automata._core`.
#[allow(clippy::used_underscore_items)] // the Python module path dictates the name
#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    automata_python::install_step_driver(Some(driver::step));
    automata_python::init_module(m)?;
    m.add_function(wrap_pyfunction!(web_frame, m)?)?;
    m.add("platform", "web")?;
    Ok(())
}
