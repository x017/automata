//! Directional sunlight: data shared by the draw list, the scene bridge,
//! and the mesh shader's lighting model.

use glam::Vec3;

/// The default direction light travels: mostly downward, slightly tilted
/// (matches the pre-lighting-model hardcoded shader light).
pub(crate) const DEFAULT_SUN_DIRECTION: Vec3 = Vec3::new(-0.5, -1.0, -0.35);

/// The default ambient light fraction.
pub(crate) const DEFAULT_AMBIENT: f32 = 0.3;

/// A directional sun: parallel light rays plus flat ambient fill.
///
/// Set on a frame via [`DrawList::set_sun`](crate::DrawList::set_sun);
/// when a frame has no sun, the renderer falls back to
/// [`Sun::default`] with shadows disabled.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sun {
    /// The direction the light travels (normalized on use).
    pub direction: Vec3,
    /// Light color (linear RGB).
    pub color: [f32; 3],
    /// Ambient light fraction, 0.0..=1.0: how much of the material color
    /// shows on unlit faces.
    pub ambient: f32,
    /// Whether this sun casts shadows (rendered via shadow mapping).
    pub shadows: bool,
}

impl Sun {
    /// Creates a sun shining along `direction`.
    #[must_use]
    pub fn new(direction: Vec3) -> Self {
        Self {
            direction,
            ..Self::default()
        }
    }

    /// The normalized direction the light travels.
    #[must_use]
    pub fn normalized_direction(&self) -> Vec3 {
        self.direction
            .try_normalize()
            .unwrap_or(DEFAULT_SUN_DIRECTION.normalize())
    }
}

impl Default for Sun {
    fn default() -> Self {
        Self {
            direction: DEFAULT_SUN_DIRECTION,
            color: [1.0, 1.0, 1.0],
            ambient: DEFAULT_AMBIENT,
            shadows: false,
        }
    }
}

/// GPU mirror of the light bind group's uniform buffer.
///
/// Layout (WGSL): a `vec4` direction, a `vec4` color (alpha = ambient),
/// the light-space view-projection matrix, and a `vec4` of params
/// (x = shadow flag, y = shadow-map texel size).
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct LightUniform {
    pub direction: [f32; 4],
    pub color_ambient: [f32; 4],
    pub light_view_proj: [[f32; 4]; 4],
    pub params: [f32; 4],
}

impl LightUniform {
    /// Packs a sun plus its light-space transform for the shader.
    #[must_use]
    pub fn new(sun: &Sun, light_view_proj: glam::Mat4, shadow_texel: f32) -> Self {
        let direction = sun.normalized_direction();
        Self {
            direction: [direction.x, direction.y, direction.z, 0.0],
            color_ambient: [sun.color[0], sun.color[1], sun.color[2], sun.ambient],
            light_view_proj: light_view_proj.to_cols_array_2d(),
            params: [f32::from(u8::from(sun.shadows)), shadow_texel, 0.0, 0.0],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_sun_matches_legacy_look() {
        let sun = Sun::default();
        assert_eq!(sun.direction, DEFAULT_SUN_DIRECTION);
        assert_eq!(sun.color, [1.0, 1.0, 1.0]);
        assert_eq!(sun.ambient, DEFAULT_AMBIENT);
        assert!(!sun.shadows);
    }

    #[test]
    fn normalized_direction_handles_zero() {
        let sun = Sun::new(Vec3::ZERO);
        assert!((sun.normalized_direction().length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn uniform_packs_shadow_flag_and_texel() {
        let sun = Sun {
            shadows: true,
            ..Sun::new(Vec3::NEG_Y)
        };
        let uniform = LightUniform::new(&sun, glam::Mat4::IDENTITY, 1.0 / 2048.0);
        assert_eq!(uniform.params[0], 1.0);
        assert!((uniform.params[1] - 1.0 / 2048.0).abs() < f32::EPSILON);
        assert_eq!(uniform.direction[1], -1.0);
    }
}
