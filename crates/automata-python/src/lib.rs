//! Python bindings for the AUTOMATA engine.
//!
//! This crate builds the native `_core` extension module behind the
//! `automata` Python package (see `python/automata/__init__.py`). The
//! Python-side package provides the user-facing surface documented in
//! `docs/api.md`.

use pyo3::prelude::*;

/// The native extension module imported as `automata._core`.
#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
