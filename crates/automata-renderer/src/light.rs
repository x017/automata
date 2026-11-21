//! Directional sunlight: data shared by the draw list, the scene bridge,
//! and the mesh shader's lighting model.

use glam::Vec3;

use crate::shadow::{ShadowCascades, MAX_CASCADES};
use crate::Environment;

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
/// Soft shadow penumbra width on the Beautiful preset (metres).
const SOFT_SHADOW_RADIUS: f32 = 0.05;

/// What the light uniform needs to know about the frame being drawn.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FrameView {
    pub camera: crate::Camera3D,
    pub viewport: (u32, u32),
    /// The clear color (linear RGB): the horizon in reflections.
    pub background: [f32; 3],
    /// Animation time (seconds).
    pub time: f32,
    /// Beautiful preset: PBR lighting, soft shadows, HDR output.
    pub beautiful: bool,
}

impl Default for FrameView {
    fn default() -> Self {
        Self {
            camera: crate::Camera3D::default(),
            viewport: (1, 1),
            background: [0.0; 3],
            time: 0.0,
            beautiful: false,
        }
    }
}

/// The lit shaders' light uniform (`LightUniform` in common.wgsl).
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct LightUniform {
    pub direction: [f32; 4],
    pub color_ambient: [f32; 4],
    pub cascades: [[f32; 16]; MAX_CASCADES],
    pub splits: [f32; 4],
    pub cascade_texel: [f32; 4],
    pub cascade_bias: [f32; 4],
    pub params: [f32; 4],
    pub sky_exposure: [f32; 4],
    pub ground_tonemap: [f32; 4],
    pub fog: [f32; 4],
    pub fog_params: [f32; 4],
    pub camera_forward: [f32; 4],
    pub background: [f32; 4],
    pub screen: [f32; 4],
    pub inv_view_proj: [f32; 16],
    pub sky_zenith: [f32; 4],
    pub sky_sun: [f32; 4],
}

impl LightUniform {
    /// Packs the sun (`sun.shadows` gates shadow sampling), the
    /// environment, the shadow cascades (`map_size` texels per side) and
    /// the frame.
    #[must_use]
    pub fn new(
        sun: &Sun,
        env: &Environment,
        shadows: &ShadowCascades,
        map_size: u32,
        frame: &FrameView,
    ) -> Self {
        let direction = sun.normalized_direction();
        let density = if env.fog {
            env.fog_density.max(0.0)
        } else {
            0.0
        };
        let [sr, sg, sb] = env.sky_color;
        let [gr, gg, gb] = env.ground_color;
        let [fr, fg, fb] = env.fog_color;
        let [br, bg, bb] = frame.background;
        let size = map_size.max(1) as f32;
        let mut cascades = [[0.0; 16]; MAX_CASCADES];
        let mut texel = [1.0; 4];
        let mut bias = [0.0; 4];
        for i in 0..MAX_CASCADES {
            cascades[i] = shadows.view_proj[i].to_cols_array();
            texel[i] = 2.0 * shadows.half_extent[i] / size;
            // About a texel of world-space depth, in the map's NDC units.
            bias[i] = texel[i] / shadows.depth_range[i].max(1e-3);
        }
        let cam = frame.camera;
        let inv_view_proj = cam.view_proj(frame.viewport).inverse().to_cols_array();
        let [zr, zg, zb] = env.zenith_color;
        // The disk's edge as the cosine of its angular radius.
        let sun_cos = (env.sun_size.clamp(0.05, 20.0) * 0.5).to_radians().cos();
        let forward = (cam.target - cam.position)
            .try_normalize()
            .unwrap_or(Vec3::NEG_Z);
        Self {
            direction: [direction.x, direction.y, direction.z, SOFT_SHADOW_RADIUS],
            color_ambient: [sun.color[0], sun.color[1], sun.color[2], sun.ambient],
            cascades,
            splits: shadows.splits,
            cascade_texel: texel,
            cascade_bias: bias,
            params: [
                f32::from(u8::from(sun.shadows)),
                1.0 / size,
                shadows.count as f32,
                f32::from(u8::from(frame.beautiful)),
            ],
            sky_exposure: [sr, sg, sb, env.exposure.max(0.0)],
            ground_tonemap: [gr, gg, gb, env.tonemapping.index()],
            fog: [fr, fg, fb, density],
            fog_params: [env.fog_start.max(0.0), frame.time, 0.0, 0.0],
            camera_forward: [forward.x, forward.y, forward.z, 0.0],
            background: [br, bg, bb, 1.0],
            screen: [
                frame.viewport.0.max(1) as f32,
                frame.viewport.1.max(1) as f32,
                cam.near,
                cam.far,
            ],
            inv_view_proj,
            sky_zenith: [zr, zg, zb, f32::from(u8::from(env.sky))],
            sky_sun: [
                sun_cos,
                env.sun_glow.max(0.0),
                env.clouds.clamp(0.0, 1.0),
                env.lens_flare.max(0.0),
            ],
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

    fn scene_shadows() -> ShadowCascades {
        ShadowCascades::fit_scene(
            crate::shadow::Sphere {
                center: Vec3::ZERO,
                radius: 10.0,
            },
            Vec3::NEG_Y,
            2048,
        )
    }

    #[test]
    fn uniform_packs_shadow_flag_and_texel() {
        let sun = Sun {
            shadows: true,
            ..Sun::new(Vec3::NEG_Y)
        };
        let u = LightUniform::new(
            &sun,
            &Environment::default(),
            &scene_shadows(),
            2048,
            &FrameView::default(),
        );
        assert_eq!(u.params[0], 1.0);
        assert!((u.params[1] - 1.0 / 2048.0).abs() < f32::EPSILON);
        assert_eq!(u.params[2], 1.0, "one cascade");
        assert_eq!(u.params[3], 0.0, "forward path");
        assert_eq!(u.direction[1], -1.0);
        assert!(u.cascade_texel[0] > 0.0 && u.cascade_bias[0] > 0.0);
    }

    #[test]
    fn uniform_size_is_16_byte_aligned() {
        assert_eq!(std::mem::size_of::<LightUniform>() % 16, 0);
    }

    #[test]
    fn uniform_packs_environment_and_frame() {
        let env = Environment {
            fog: true,
            fog_density: 0.5,
            exposure: 2.0,
            tonemapping: crate::Tonemapping::Aces,
            ..Environment::default()
        };
        let frame = FrameView {
            viewport: (640, 480),
            time: 3.0,
            beautiful: true,
            ..FrameView::default()
        };
        let u = LightUniform::new(&Sun::default(), &env, &scene_shadows(), 2048, &frame);
        assert!((u.fog[3] - 0.5).abs() < f32::EPSILON);
        assert!((u.sky_exposure[3] - 2.0).abs() < f32::EPSILON);
        assert!((u.ground_tonemap[3] - 2.0).abs() < f32::EPSILON);
        assert_eq!(u.params[3], 1.0);
        assert_eq!(u.fog_params[1], 3.0);
        assert_eq!(&u.screen[..2], &[640.0, 480.0]);
        let off = LightUniform::new(
            &Sun::default(),
            &Environment::default(),
            &scene_shadows(),
            2048,
            &FrameView::default(),
        );
        assert!(
            off.fog[3].abs() < f32::EPSILON,
            "fog disabled packs density 0"
        );
    }
}
