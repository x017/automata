//! AUTOMATA on Android: the engine's Python module, `automata._core`, with
//! an SDL3 window driver.
//!
//! python-for-android's SDL3 launcher starts Python on SDL's main thread and
//! runs the game's `main.py`. Importing `automata` loads this library,
//! which sends the engine's logs to logcat, installs [`driver::run`] as
//! the window driver, and fills the module exactly like the desktop
//! extension. `game.run()` then opens an SDL window instead of a winit one;
//! scripts don't change.

mod driver;
mod keys;
mod log;

use pyo3::prelude::*;

/// The native extension module imported as `automata._core`.
#[allow(clippy::used_underscore_items)] // the Python module path dictates the name
#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    log::init();
    if automata_core::platform::install_driver(driver::run) {
        tracing::info!("SDL3 window driver installed");
    }
    automata_python::init_module(m)?;
    m.add("platform", "android")?;
    Ok(())
}
