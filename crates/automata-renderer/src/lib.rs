//! AUTOMATA rendering system.
//!
//! Phase 2 delivered the GPU device, surface, and frame presentation.
//! Phase 3 adds the rendering foundation on top: shaders, vertex/index
//! buffers, textures, materials, cameras, and 2D/3D drawing via
//! [`Renderer`] + [`DrawList`].
//!
//! The crate is built on [`wgpu`] and never exposes `unsafe` code. Public
//! API types that originate in `wgpu` (such as [`Color`]) are re-exported so
//! that simple consumers do not need a direct `wgpu` dependency.

mod camera;
mod context;
mod draw;
mod environment;
mod error;
mod light;
mod material;
mod mesh;
mod pipeline;
mod quality;
mod renderer;
pub mod text;
mod texture;

pub use camera::{Camera2D, Camera3D};
pub use context::RenderContext;
pub use draw::{DrawList, Sprite};
pub use environment::{Environment, Tonemapping};
pub use error::{Error, Result};
pub use light::Sun;
pub use material::Material;
pub use mesh::{Mesh, MeshData, Vertex};
pub use quality::GraphicsQuality;
pub use renderer::Renderer;
pub use text::{TextAlign, TextAtlas, TextStyle};
pub use texture::Texture;

pub use wgpu::Color;
pub use wgpu::Texture as RenderTarget;

/// Maximum number of sprite vertices in one batch (quads × 4).
pub(crate) const SPRITE_VERTEX_CAPACITY: usize = 16_384;
