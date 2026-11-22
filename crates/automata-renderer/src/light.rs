//! Directional sunlight and point lights: data shared by the draw list,
//! the scene bridge, and the lit shaders' lighting model.

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

/// The most point lights a frame shades (the size of the shaders' light
/// array). Quality presets shade fewer: see
/// [`GraphicsQuality::max_point_lights`](crate::GraphicsQuality::max_point_lights).
pub const MAX_POINT_LIGHTS: usize = 32;

/// Irradiance below which a point light counts as gone: sets the automatic
/// range (a white light of intensity 1 reaches about 16 m).
const POINT_CUTOFF: f32 = 0.004;

/// The longest automatic point light range (m).
const MAX_AUTO_RANGE: f32 = 80.0;

/// Light radiating from a point (or a small glowing body) in every
/// direction, fading with the square of the distance. It casts no shadows.
///
/// Added to a frame with [`DrawList::point_light`](crate::DrawList::point_light).
/// Glowing meshes (a material with `emissive` and `glow_light` above zero)
/// add one automatically.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointLight {
    /// Where the light sits (world space).
    pub position: Vec3,
    /// Linear RGB intensity. A white light of intensity 1 lights a surface
    /// facing it 1 m away as brightly as a white sun shining straight on.
    pub intensity: [f32; 3],
    /// The size of the glowing body (m). Closer than this the light stops
    /// getting brighter, so there is no burning hot spot.
    pub radius: f32,
    /// Where the light has faded out completely (m), smoothly. 0 picks it
    /// from the intensity (where it would be too dim to see).
    pub range: f32,
}

impl PointLight {
    /// A small white-intensity light at `position` with an automatic range.
    #[must_use]
    pub fn new(position: Vec3, intensity: [f32; 3]) -> Self {
        Self {
            position,
            intensity,
            radius: 0.05,
            range: 0.0,
        }
    }

    /// The brightest channel of the intensity.
    #[must_use]
    pub fn peak(&self) -> f32 {
        self.intensity.iter().copied().fold(0.0, f32::max)
    }

    /// The distance where the light ends: `range`, or where it would fall
    /// below a visible level.
    #[must_use]
    pub fn effective_range(&self) -> f32 {
        if self.range > 0.0 {
            return self.range;
        }
        (self.peak().max(0.0) / POINT_CUTOFF)
            .sqrt()
            .clamp(self.radius.max(0.1), MAX_AUTO_RANGE)
    }

    /// The light of a glowing body: an object `half_extents` across (half
    /// sizes of its world-space box) centered on `center`, whose surface
    /// emits `radiance` (linear RGB). Lights its surroundings as a glowing
    /// body of that surface area would (intensity = radiance × area / 4,
    /// exact for a sphere).
    #[must_use]
    pub fn from_glow(center: Vec3, half_extents: Vec3, radiance: [f32; 3]) -> Self {
        let e = half_extents.abs();
        let area = 8.0 * (e.x * e.y + e.y * e.z + e.z * e.x);
        Self {
            position: center,
            intensity: radiance.map(|c| c.max(0.0) * area * 0.25),
            radius: (area / (4.0 * std::f32::consts::PI)).sqrt(),
            range: 0.0,
        }
    }

    /// The shaders' layout: position + range, intensity + radius.
    pub(crate) fn packed(&self) -> [f32; 8] {
        let [r, g, b] = self.intensity.map(|c| c.max(0.0));
        let p = self.position;
        [
            p.x,
            p.y,
            p.z,
            self.effective_range(),
            r,
            g,
            b,
            self.radius.max(0.01),
        ]
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
    /// Point lights (`PointLightData` in common.wgsl); the first
    /// `fog_params.z` are used.
    pub points: [[f32; 8]; MAX_POINT_LIGHTS],
}

impl LightUniform {
    /// Packs the sun (`sun.shadows` gates shadow sampling), the
    /// environment, the shadow cascades (`map_size` texels per side), the
    /// frame and its point lights (the first [`MAX_POINT_LIGHTS`]).
    #[must_use]
    pub fn new(
        sun: &Sun,
        env: &Environment,
        shadows: &ShadowCascades,
        map_size: u32,
        frame: &FrameView,
        point_lights: &[PointLight],
    ) -> Self {
        let mut points = [[0.0; 8]; MAX_POINT_LIGHTS];
        for (slot, p) in points.iter_mut().zip(point_lights) {
            *slot = p.packed();
        }
        let point_count = point_lights.len().min(MAX_POINT_LIGHTS) as f32;
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
            fog_params: [env.fog_start.max(0.0), frame.time, point_count, 0.0],
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
            points,
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
            &[],
        );
        assert_eq!(u.params[0], 1.0);
        assert!((u.params[1] - 1.0 / 2048.0).abs() < f32::EPSILON);
        assert_eq!(u.params[2], 1.0, "one cascade");
        assert_eq!(u.params[3], 0.0, "forward path");
        assert_eq!(u.direction[1], -1.0);
        assert!(u.cascade_texel[0] > 0.0 && u.cascade_bias[0] > 0.0);
    }

    #[test]
    fn glowing_bodies_become_matching_point_lights() {
        // A unit sphere's worth of area (a 1 m cube is close enough to
        // check the packing) glowing at 2.
        let light = PointLight::from_glow(Vec3::Y, Vec3::splat(0.5), [2.0, 1.0, 0.0]);
        // Area 6 m², so intensity = radiance × 1.5.
        assert!((light.intensity[0] - 3.0).abs() < 1e-5);
        assert!((light.intensity[1] - 1.5).abs() < 1e-5);
        assert!((light.radius - (6.0 / (4.0 * std::f32::consts::PI)).sqrt()).abs() < 1e-5);
        // The automatic range ends where the brightest channel is too dim.
        let range = light.effective_range();
        assert!((range - (3.0_f32 / POINT_CUTOFF).sqrt()).abs() < 1e-3);
        let fixed = PointLight {
            range: 2.0,
            ..light
        };
        assert!((fixed.effective_range() - 2.0).abs() < f32::EPSILON);
        let packed = light.packed();
        assert_eq!(&packed[..3], &[0.0, 1.0, 0.0]);
        assert!((packed[3] - range).abs() < 1e-5);
        // A dark body has no reach worth shading.
        let dark = PointLight::from_glow(Vec3::ZERO, Vec3::ONE, [0.0; 3]);
        assert!(dark.peak() <= 0.0);
        let u = LightUniform::new(
            &Sun::default(),
            &Environment::default(),
            &scene_shadows(),
            2048,
            &FrameView::default(),
            &[light, dark],
        );
        assert_eq!(u.fog_params[2], 2.0);
        assert_eq!(u.points[0], packed);
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
        let u = LightUniform::new(&Sun::default(), &env, &scene_shadows(), 2048, &frame, &[]);
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
            &[],
        );
        assert!(
            off.fog[3].abs() < f32::EPSILON,
            "fog disabled packs density 0"
        );
    }
}
