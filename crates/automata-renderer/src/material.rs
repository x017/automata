//! Materials: the surface appearance of rendered meshes.

use std::sync::Arc;

use crate::texture::Texture;

/// A mesh material: a base color (times an optional texture) plus the
/// physically based surface parameters every quality preset reads.
///
/// Cheap to clone: the underlying GPU resources are reference-counted.
/// Created via [`Renderer::create_material`](crate::Renderer::create_material)
/// or [`Renderer::create_pbr_material`](crate::Renderer::create_pbr_material).
#[derive(Debug, Clone)]
pub struct Material {
    pub(crate) inner: Arc<MaterialInner>,
}

impl Material {
    /// The material's base color (multiplied with the texture, if any).
    #[must_use]
    pub fn color(&self) -> [f32; 4] {
        self.inner.params.color
    }

    /// The material's texture (a 1×1 white fallback when none was given).
    #[must_use]
    pub fn texture(&self) -> &Texture {
        &self.inner.texture
    }

    /// The surface parameters it was created with.
    #[must_use]
    pub fn params(&self) -> &MaterialParams {
        &self.inner.params
    }
}

/// The surface parameters of a [`Material`], in glTF's metallic-roughness
/// model. The defaults match the engine's original look: a slightly rough,
/// non-metal surface that doesn't glow.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialParams {
    /// Base color, linear RGBA (multiplied with the base color texture).
    pub color: [f32; 4],
    /// Microsurface roughness, 0 (mirror) to 1 (chalk). Multiplied with the
    /// metallic-roughness texture's green channel.
    pub roughness: f32,
    /// 0 for dielectrics (plastic, wood, stone), 1 for bare metal.
    /// Multiplied with the metallic-roughness texture's blue channel.
    pub metallic: f32,
    /// Light the surface emits, linear RGB; values above 1 glow (and bloom
    /// on Beautiful). Multiplied with the emissive texture.
    pub emissive: [f32; 3],
    /// How strongly the normal map bends the surface (glTF `scale`).
    pub normal_scale: f32,
}

impl Default for MaterialParams {
    fn default() -> Self {
        Self {
            color: [1.0; 4],
            roughness: DEFAULT_ROUGHNESS,
            metallic: 0.0,
            emissive: [0.0; 3],
            normal_scale: 1.0,
        }
    }
}

impl MaterialParams {
    /// Plain parameters with this base color.
    #[must_use]
    pub fn with_color(color: [f32; 4]) -> Self {
        Self {
            color,
            ..Self::default()
        }
    }
}

/// The roughness a material gets when nothing says otherwise.
pub const DEFAULT_ROUGHNESS: f32 = 0.6;

/// The textures of a [`Material`]. Each is optional; a missing map reads
/// as "no change" (white for color, metallic-roughness and emissive; flat
/// for the normal map).
#[derive(Debug, Clone, Default)]
pub struct MaterialTextures {
    /// Base color (sRGB), multiplied with `color`.
    pub base: Option<Texture>,
    /// Tangent-space normal map (linear, OpenGL convention: +Y = green up,
    /// as glTF uses). Tangents come from the UVs on the fly, so meshes
    /// don't need a tangent attribute.
    pub normal: Option<Texture>,
    /// glTF metallic-roughness map (linear): G = roughness, B = metallic.
    pub metallic_roughness: Option<Texture>,
    /// Emissive color (sRGB), multiplied with `emissive`.
    pub emissive: Option<Texture>,
}

/// The material uniform (`MaterialData` in mesh.wgsl).
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct MaterialUniform {
    pub color: [f32; 4],
    /// RGB emitted light; A unused.
    pub emissive: [f32; 4],
    /// Roughness, metallic, normal scale, and which maps are bound
    /// (`MAP_*` bits, as a float).
    pub params: [f32; 4],
}

/// `MaterialUniform::params.w` bits (mesh.wgsl's `MAP_*`).
pub(crate) const MAP_NORMAL: u32 = 1;
pub(crate) const MAP_METAL_ROUGH: u32 = 2;
pub(crate) const MAP_EMISSIVE: u32 = 4;

impl MaterialUniform {
    pub(crate) fn new(p: &MaterialParams, maps: u32) -> Self {
        Self {
            color: p.color,
            emissive: [p.emissive[0], p.emissive[1], p.emissive[2], 0.0],
            params: [
                // Perfect mirrors alias and blow out the GGX peak.
                p.roughness.clamp(0.04, 1.0),
                p.metallic.clamp(0.0, 1.0),
                p.normal_scale,
                #[allow(clippy::cast_precision_loss)] // three bits
                {
                    maps as f32
                },
            ],
        }
    }
}

/// GPU-resident material data.
#[derive(Debug)]
pub(crate) struct MaterialInner {
    pub bind_group: wgpu::BindGroup,
    pub params: MaterialParams,
    pub texture: Texture,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_uniform_matches_the_shader_layout_and_clamps() {
        assert_eq!(std::mem::size_of::<MaterialUniform>(), 48);
        let u = MaterialUniform::new(
            &MaterialParams {
                roughness: 0.0,
                metallic: 2.0,
                emissive: [3.0, 2.0, 1.0],
                ..MaterialParams::default()
            },
            MAP_NORMAL | MAP_EMISSIVE,
        );
        assert!(u.params[0] > 0.0, "roughness keeps a floor");
        assert!((u.params[1] - 1.0).abs() < f32::EPSILON);
        assert_eq!(u.emissive, [3.0, 2.0, 1.0, 0.0]);
        assert!((u.params[3] - 5.0).abs() < f32::EPSILON);
        let plain = MaterialUniform::new(&MaterialParams::default(), 0);
        assert!((plain.params[0] - DEFAULT_ROUGHNESS).abs() < f32::EPSILON);
        assert!(plain.params[3].abs() < f32::EPSILON);
    }
}
