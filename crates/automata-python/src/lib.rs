//! Python bindings for the AUTOMATA engine.
//!
//! This crate builds the native `_core` extension module behind the
//! `automata` Python package (see `python/automata/__init__.py`). The
//! Python API is documented in `docs/api.md`.
//!
//! Ownership rules across the boundary:
//!
//! - `Game` owns the engine. `Game.run(update)` borrows
//!   the game for the whole loop; nested `run` calls raise.
//! - `Scene`/`Entity` are plain-data staging objects; `load_scene` copies
//!   them into the ECS world.
//! - The per-frame `FrameContext` carries owned snapshots plus buffered
//!   writes (scene commands, outgoing events). No live engine borrows
//!   cross into Python, so context objects that escape the callback are
//!   stale but harmless.

mod components;
mod context;
mod error;
mod events;
mod game;
mod keys;
mod scene;

use pyo3::prelude::*;

/// The native extension module imported as `automata._core`.
#[allow(clippy::used_underscore_items)] // the Python module path dictates the name
#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;

    m.add("Error", m.py().get_type::<error::AutomataError>())?;

    m.add_class::<game::PyGame>()?;
    m.add_class::<scene::PyScene>()?;
    m.add_class::<scene::PyEntity>()?;
    m.add_class::<scene::SceneView>()?;
    m.add_class::<scene::EntityView>()?;
    m.add_class::<components::PyTransform>()?;
    m.add_class::<components::PyCamera3D>()?;
    m.add_class::<components::PyCamera2D>()?;
    m.add_class::<context::FrameContext>()?;
    m.add_class::<context::FrameTime>()?;
    m.add_class::<context::PyInput>()?;
    m.add_class::<context::PyModifiers>()?;
    m.add_class::<context::PyResources>()?;
    m.add_class::<events::PyEvents>()?;
    Ok(())
}
