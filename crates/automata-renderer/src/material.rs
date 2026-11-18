//! Materials: the surface appearance of rendered meshes.

use std::sync::Arc;

use crate::texture::Texture;

/// A basic mesh material: a base color multiplied with an optional texture.
///
/// Cheap to clone: the underlying GPU resources are reference-counted.
/// Created via [`Renderer::create_material`](crate::Renderer::create_material).
///
/// Phase 3 keeps materials deliberately minimal; richer material models
/// (lighting parameters, normal maps, custom shaders) arrive with later
/// phases.
#[derive(Debug, Clone)]
pub struct Material {
    pub(crate) inner: Arc<MaterialInner>,
}

impl Material {
    /// The material's base color (multiplied with the texture, if any).
    #[must_use]
    pub fn color(&self) -> [f32; 4] {
        self.inner.color
    }

    /// The material's texture (a 1×1 white fallback when none was given).
    #[must_use]
    pub fn texture(&self) -> &Texture {
        &self.inner.texture
    }
}

/// GPU-resident material data.
#[derive(Debug)]
pub(crate) struct MaterialInner {
    pub bind_group: wgpu::BindGroup,
    pub color: [f32; 4],
    pub texture: Texture,
}
