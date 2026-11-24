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

mod batch;
mod camera;
mod composite;
mod context;
mod draw;
mod environment;
mod error;
mod grass;
mod light;
mod material;
mod mesh;
mod particles;
mod pipeline;
mod post;
mod quality;
mod renderer;
mod shadow;
mod ssao;
mod style;
pub mod text;
mod texture;
mod water;

pub use camera::{Camera2D, Camera3D};
pub use context::{probe_gpu, GpuSummary, RenderContext};
pub use draw::{DrawList, MeshLod, Sprite};
pub use environment::{Environment, PostEffects, Tonemapping};
pub use error::{Error, Result};
pub use grass::{
    blade_mesh as grass_blade_mesh, drawn_blades as grass_drawn_blades, keep_at as grass_keep_at,
    GrassBlade, GrassChunk, GrassField, GrassLook, GrassPusher, MAX_GRASS_PUSHERS,
};
pub use light::{
    PointLight, SpotCone, Sun, DEFAULT_SHADOW_DISTANCE, MAX_POINT_LIGHTS, MAX_POINT_SHADOWS,
};
pub use material::{Material, MaterialParams, MaterialTextures, DEFAULT_ROUGHNESS};
pub use mesh::{Mesh, MeshData, SkinVertex, Vertex};
pub use particles::{
    sheet_cell, ParticleBatch, ParticleBlend, ParticleInstance, ParticleSprite,
    MAX_PARTICLES_PER_FRAME,
};
pub use quality::GraphicsQuality;
pub use renderer::Renderer;
pub use text::{Faces, FontId, TextAlign, TextAtlas, TextStyle};
pub use texture::Texture;
pub use water::{RippleMap, Water, WaterSurface, RIPPLE_FIELD_SIZE};

pub use wgpu::Color;
pub use wgpu::Texture as RenderTarget;

/// Maximum number of sprite vertices in one batch (quads × 4).
pub(crate) const SPRITE_VERTEX_CAPACITY: usize = 16_384;

/// The most joints a skinned draw can use (the size of the shaders' joint
/// palette).
pub const MAX_JOINTS: usize = 128;
