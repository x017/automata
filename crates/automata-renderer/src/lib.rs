//! AUTOMATA rendering system.
//!
//! Phase 2 scope: GPU device and surface management plus frame presentation
//! (clearing the window). Geometry, materials, cameras, and lighting arrive
//! in Phase 3.
//!
//! The crate is built on [`wgpu`] and never exposes `unsafe` code. Public
//! API types that originate in `wgpu` (such as [`Color`]) are re-exported so
//! that simple consumers do not need a direct `wgpu` dependency.

mod context;
mod error;

pub use context::RenderContext;
pub use error::{Error, Result};

pub use wgpu::Color;
