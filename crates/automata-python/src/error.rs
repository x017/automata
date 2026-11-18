//! Error types bridging Rust engine failures into Python exceptions.

use pyo3::exceptions::PyException;
use pyo3::PyErr;

pyo3::create_exception!(
    automata._core,
    AutomataError,
    PyException,
    "An error raised by the AUTOMATA engine runtime."
);

/// Converts a core engine error into the Python `automata.Error` exception.
pub fn core_error(err: automata_core::Error) -> pyo3::PyErr {
    AutomataError::new_err(err.to_string())
}
