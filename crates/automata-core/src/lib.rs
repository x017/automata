//! Core runtime of the AUTOMATA game engine.
//!
//! `automata-core` hosts the engine's foundation: application configuration,
//! logging, timekeeping, error types, the windowed/headless runtime, and the
//! main-loop lifecycle. All other subsystems (rendering, scenes, physics,
//! audio, assets, and the Python bindings) build on top of this crate and are
//! introduced as separate crates as their boundaries become meaningful.
//!
//! See `docs/architecture.md` for the layered design and `build.md` for the
//! full project specification.

pub mod app;
pub mod audio_source;
pub mod config;
mod driver;
pub mod error;
pub mod logging;
pub mod prefab;
pub mod scene_file;
pub mod time;
pub mod window;

pub use app::{AppContext, AppState, Application, SharedAudio, WorldState};
pub use audio_source::{AudioCommand, AudioSource, AudioSourcePlayer};
pub use config::EngineConfig;
pub use error::{Error, Result};
pub use prefab::PrefabLink;
pub use scene_file::{
    AttributeValue, Attributes, EntityData, MeshVisualData, PrimitiveData, SceneFile,
    SceneFileError, Script, TransformData, SCENE_FILE_VERSION,
};
pub use time::Time;
pub use window::{WindowConfig, WindowMode};

pub use automata_input::InputState;
