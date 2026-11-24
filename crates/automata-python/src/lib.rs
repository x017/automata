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

mod actions;
mod animation;
mod attributes;
mod audio;
mod components;
mod context;
mod error;
mod events;
mod foliage;
mod game;
mod gamepads;
mod joints;
mod keys;
mod particles;
mod physics;
mod physics_components;
mod scene;
mod screen;
mod serialize;

use pyo3::prelude::*;

/// The native extension module imported as `automata._core`.
#[allow(clippy::used_underscore_items)] // the Python module path dictates the name
#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;

    m.add("Error", m.py().get_type::<error::AutomataError>())?;

    m.add_class::<game::PyGame>()?;
    m.add_class::<scene::PyScene>()?;
    m.add_class::<attributes::PyAttributes>()?;
    m.add_class::<attributes::PyPrefab>()?;
    m.add_class::<scene::PyEntity>()?;
    m.add_class::<scene::SceneView>()?;
    m.add_class::<scene::EntityView>()?;
    m.add_class::<scene::TransformRef>()?;
    m.add_class::<components::PyTransform>()?;
    m.add_class::<components::PyCamera3D>()?;
    m.add_class::<components::PyCamera2D>()?;
    m.add_class::<context::FrameContext>()?;
    m.add_class::<context::FrameTime>()?;
    m.add_class::<context::PyInput>()?;
    m.add_class::<context::PyModifiers>()?;
    m.add_class::<context::PyTouch>()?;
    m.add_class::<context::PyResources>()?;
    m.add_class::<events::PyEvents>()?;
    m.add_class::<audio::PyAudio>()?;
    m.add_class::<screen::PyScreen>()?;
    m.add_class::<gamepads::PyGamepads>()?;
    m.add_class::<gamepads::PyGamepad>()?;
    m.add_class::<gamepads::PyGamepadEvent>()?;
    m.add_class::<actions::PyActionMap>()?;
    m.add_class::<actions::PyBinding>()?;
    m.add_class::<actions::PyAxisBinding>()?;
    m.add_class::<physics::PyPhysics>()?;
    m.add_class::<physics::PyPhysics2D>()?;
    m.add_class::<physics_components::PyRigidBody>()?;
    m.add_class::<physics_components::PyCollider3D>()?;
    m.add_class::<physics_components::PyCollider2D>()?;
    m.add_class::<physics_components::PyCharacterController>()?;
    m.add_class::<physics_components::PyCharacterVelocity>()?;
    m.add_class::<joints::PyJoint>()?;
    m.add_class::<joints::PyJointMotor>()?;
    m.add_class::<joints::PyJointRef>()?;
    m.add_class::<components::PyMeshRenderer>()?;
    m.add_class::<components::PyDirectionalLight>()?;
    m.add_class::<components::PyPointLight>()?;
    m.add_class::<components::PySpotLight>()?;
    m.add_class::<particles::PyParticleEmitter>()?;
    m.add_class::<components::PyEnvironment>()?;
    m.add_class::<components::PyWater>()?;
    m.add_class::<foliage::PyGrass>()?;
    m.add_class::<foliage::PyScatter>()?;
    m.add_class::<components::PyAudioSource>()?;
    m.add_class::<animation::PyAnimation>()?;
    m.add_class::<animation::PyAnimationRef>()?;
    m.add_function(wrap_pyfunction!(components::set_asset_root, m)?)?;
    m.add_function(wrap_pyfunction!(components::asset_root, m)?)?;
    Ok(())
}
