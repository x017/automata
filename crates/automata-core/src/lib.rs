//! Core runtime of the AUTOMATA game engine.
//!
//! `automata-core` hosts the engine's foundation: application configuration,
//! logging, timekeeping, error types, and the main-loop lifecycle. All other
//! subsystems (rendering, scenes, input, physics, audio, assets, and the
//! Python bindings) build on top of this crate and are introduced as separate
//! crates as their boundaries become meaningful.
//!
//! See `docs/architecture.md` for the layered design and `build.md` for the
//! full project specification.

pub mod app;
pub mod config;
pub mod error;
pub mod logging;
pub mod time;

pub use app::{AppContext, AppState, Application};
pub use config::EngineConfig;
pub use error::{Error, Result};
pub use time::Time;
