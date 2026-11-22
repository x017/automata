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
    /// How far from the camera shadows reach (metres); they fade out
    /// beyond. Shorter is sharper. (Medium's single shadow map only
    /// follows it when that covers less than the whole scene.)
    pub shadow_distance: f32,
    /// Shadow edge softness: 1 is the default slight blur, 0 as sharp as
    /// the shadow map allows, higher softer (up to 8).
    pub shadow_softness: f32,
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
            shadow_distance: DEFAULT_SHADOW_DISTANCE,
            shadow_softness: 1.0,
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
    /// Whether objects block this light. The quality preset decides how many
    /// lights a frame shadows ([`GraphicsQuality::point_shadows`](crate::GraphicsQuality::point_shadows));
    /// the brightest and nearest get them first.
    pub shadows: bool,
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
            shadows: true,
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
            shadows: true,
        }
    }

    /// The shaders' layout: position + range, intensity + radius, and the
    /// shadow (its slot or -1, near and far planes).
    pub(crate) fn packed(&self, shadow: Option<(usize, &PointShadow)>) -> [f32; 12] {
        let [r, g, b] = self.intensity.map(|c| c.max(0.0));
        let p = self.position;
        #[allow(clippy::cast_precision_loss)] // a handful of slots
        let (slot, near, far) =
            shadow.map_or((-1.0, 0.0, 0.0), |(slot, s)| (slot as f32, s.near, s.far));
        [
            p.x,
            p.y,
            p.z,
            self.effective_range(),
            r,
            g,
            b,
            self.radius.max(0.01),
            slot,
            near,
            far,
            0.0,
        ]
    }
}

/// The most point lights a frame can shadow (the shaders' face array holds
/// six per light).
pub const MAX_POINT_SHADOWS: usize = 4;

/// Each face of a point light's shadow cube sees a little more than 90°,
/// so soft-shadow taps near a face's edge stay inside it.
pub(crate) const POINT_SHADOW_FOV_DEGREES: f32 = 96.0;

/// The six views a point light's shadow renders: +X, -X, +Y, -Y, +Z, -Z
/// (the order the shaders pick faces in).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PointShadow {
    pub faces: [glam::Mat4; 6],
    pub near: f32,
    pub far: f32,
}

impl PointShadow {
    /// The shadow views of `light`. The near plane sits inside the glowing
    /// body (which is left out of its own shadow), the far plane at the
    /// light's range.
    pub fn for_light(light: &PointLight) -> Self {
        use glam::camera::rh::{proj::directx, view};
        let near = (light.radius * 0.5).clamp(0.02, 0.5);
        let far = light.effective_range().max(near * 2.0);
        let proj = directx::perspective(POINT_SHADOW_FOV_DEGREES.to_radians(), 1.0, near, far);
        let p = light.position;
        let face = |dir: Vec3, up: Vec3| proj * view::look_at_mat4(p, p + dir, up);
        Self {
            faces: [
                face(Vec3::X, Vec3::Y),
                face(Vec3::NEG_X, Vec3::Y),
                face(Vec3::Y, Vec3::Z),
                face(Vec3::NEG_Y, Vec3::Z),
                face(Vec3::Z, Vec3::Y),
                face(Vec3::NEG_Z, Vec3::Y),
            ],
            near,
            far,
        }
    }

    /// Whether a sphere can cast into face `face`: within the light's
    /// range and inside the face's (slightly wider than 90°) pyramid.
    pub fn face_sees(face: usize, light: Vec3, center: Vec3, radius: f32, range: f32) -> bool {
        let d = center - light;
        if d.length() > range + radius {
            return false;
        }
        let (forward, a, b) = match face {
            0 => (d.x, d.y, d.z),
            1 => (-d.x, d.y, d.z),
            2 => (d.y, d.x, d.z),
            3 => (-d.y, d.x, d.z),
            4 => (d.z, d.x, d.y),
            _ => (-d.z, d.x, d.y),
        };
        let t = (POINT_SHADOW_FOV_DEGREES * 0.5).to_radians().tan();
        // Distance of the center inside each side plane, scaled by the
        // plane normal's length.
        let slack = radius * (1.0 + t * t).sqrt();
        forward * t - a.abs() >= -slack && forward * t - b.abs() >= -slack
    }
}

/// A point light as a frame shades it: the light, the mesh draw it glows
/// from (left out of its shadow), and its shadow slot and views.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FramePoint {
    pub light: PointLight,
    pub source: Option<usize>,
    pub shadow: Option<(usize, PointShadow)>,
}

/// How far the ambient sky light leans toward the sky dome's colour.
const SKY_TINT: f32 = 0.35;

/// The ambient light from above: the environment's sky colour, and with
/// the sky dome on, leaning toward the zenith's hue at the same
/// brightness. Shade is lit by the sky alone, so shaded sides turn
/// faintly sky-coloured instead of grey (the look of most stylised
/// outdoor games).
fn sky_ambient(env: &Environment) -> [f32; 3] {
    let sky = env.sky_color;
    if !env.sky {
        return sky;
    }
    let luma = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let zenith = env.zenith_color.map(|c| c.max(0.0));
    let l = luma(zenith);
    if l <= 1e-4 {
        return sky;
    }
    std::array::from_fn(|i| sky[i] * (1.0 + SKY_TINT * (zenith[i] / l - 1.0)))
}

/// Shadow penumbra width at softness 1 (metres).
const SOFT_SHADOW_RADIUS: f32 = 0.05;

/// [`Sun::shadow_distance`]'s default (metres).
pub const DEFAULT_SHADOW_DISTANCE: f32 = 120.0;

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
    pub points: [[f32; 12]; MAX_POINT_LIGHTS],
    /// The shadowed point lights' face view-projections, six per slot.
    pub point_faces: [[f32; 16]; MAX_POINT_SHADOWS * 6],
    /// x: one shadow texel in UV, y: soft filtering (Beautiful),
    /// z: tan(face FOV / 2).
    pub point_shadow: [f32; 4],
}

impl LightUniform {
    /// Packs the sun (`sun.shadows` gates shadow sampling), the
    /// environment, the shadow cascades (`map_size` texels per side), the
    /// frame and its point lights (the first [`MAX_POINT_LIGHTS`]) with
    /// their shadows (`point_map_size` texels per face side).
    #[must_use]
    pub fn new(
        sun: &Sun,
        env: &Environment,
        shadows: &ShadowCascades,
        map_size: u32,
        frame: &FrameView,
        point_lights: &[FramePoint],
        point_map_size: u32,
    ) -> Self {
        let mut points = [[0.0; 12]; MAX_POINT_LIGHTS];
        let mut point_faces = [[0.0; 16]; MAX_POINT_SHADOWS * 6];
        for (slot, p) in points.iter_mut().zip(point_lights) {
            let shadow = p.shadow.filter(|(s, _)| *s < MAX_POINT_SHADOWS);
            *slot = p.light.packed(shadow.as_ref().map(|(s, v)| (*s, v)));
            if let Some((s, view)) = shadow {
                for (face, m) in view.faces.iter().enumerate() {
                    point_faces[s * 6 + face] = m.to_cols_array();
                }
            }
        }
        let point_count = point_lights.len().min(MAX_POINT_LIGHTS) as f32;
        let point_shadow = [
            1.0 / point_map_size.max(1) as f32,
            f32::from(u8::from(frame.beautiful)),
            (POINT_SHADOW_FOV_DEGREES * 0.5).to_radians().tan(),
            0.0,
        ];
        let direction = sun.normalized_direction();
        let density = if env.fog {
            env.fog_density.max(0.0)
        } else {
            0.0
        };
        let [sr, sg, sb] = sky_ambient(env);
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
            direction: [
                direction.x,
                direction.y,
                direction.z,
                SOFT_SHADOW_RADIUS * sun.shadow_softness.clamp(0.0, 8.0),
            ],
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
            point_faces,
            point_shadow,
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
    fn sky_dome_tints_the_ambient_toward_the_zenith_at_the_same_brightness() {
        let luma = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let flat = Environment {
            sky: false,
            ..Environment::daylight()
        };
        assert_eq!(sky_ambient(&flat), flat.sky_color);
        let dome = Environment::daylight();
        let tinted = sky_ambient(&dome);
        assert!((luma(tinted) - luma(dome.sky_color)).abs() < 0.02);
        let blue = |c: [f32; 3]| c[2] / c[0];
        assert!(blue(tinted) > blue(dome.sky_color) * 1.2, "{tinted:?}");
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
            256,
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
        let packed = light.packed(None);
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
            &[
                FramePoint {
                    light,
                    source: None,
                    shadow: None,
                },
                FramePoint {
                    light: dark,
                    source: None,
                    shadow: Some((1, PointShadow::for_light(&dark))),
                },
            ],
            256,
        );
        assert_eq!(u.fog_params[2], 2.0);
        assert_eq!(u.points[0], packed);
        assert!(
            (u.points[0][8] + 1.0).abs() < f32::EPSILON,
            "no shadow slot"
        );
        assert!((u.points[1][8] - 1.0).abs() < f32::EPSILON, "shadow slot 1");
        assert_eq!(
            u.point_faces[6],
            PointShadow::for_light(&dark).faces[0].to_cols_array()
        );
    }

    #[test]
    fn point_shadow_faces_cover_every_direction_once_picked_by_major_axis() {
        let light = PointLight {
            range: 10.0,
            ..PointLight::new(Vec3::new(1.0, 2.0, 3.0), [1.0; 3])
        };
        let s = PointShadow::for_light(&light);
        // A point along each axis lands in the middle of its face, in
        // front of the near plane.
        for (face, dir) in [
            Vec3::X,
            Vec3::NEG_X,
            Vec3::Y,
            Vec3::NEG_Y,
            Vec3::Z,
            Vec3::NEG_Z,
        ]
        .into_iter()
        .enumerate()
        {
            let clip = s.faces[face]
                * (light.position + dir * 4.0 + dir.any_orthonormal_vector() * 0.5).extend(1.0);
            let ndc = clip.truncate() / clip.w;
            assert!(ndc.x.abs() < 0.5 && ndc.y.abs() < 0.5, "face {face}: {ndc}");
            assert!(ndc.z > 0.0 && ndc.z < 1.0, "face {face} depth {}", ndc.z);
            assert!(PointShadow::face_sees(
                face,
                light.position,
                light.position + dir * 4.0,
                0.1,
                10.0
            ));
            // The opposite face never sees it.
            assert!(!PointShadow::face_sees(
                face ^ 1,
                light.position,
                light.position + dir * 4.0,
                0.1,
                10.0
            ));
        }
        // Out of range: no face.
        assert!(!PointShadow::face_sees(
            0,
            light.position,
            light.position + Vec3::X * 12.0,
            0.5,
            10.0
        ));
        // A diagonal direction falls inside the face its major axis picks.
        let diag = Vec3::new(1.0, 0.95, -0.9).normalize() * 3.0;
        let clip = s.faces[0] * (light.position + diag).extend(1.0);
        let ndc = clip.truncate() / clip.w;
        assert!(ndc.x.abs() <= 1.0 && ndc.y.abs() <= 1.0, "{ndc}");
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
        let u = LightUniform::new(
            &Sun::default(),
            &env,
            &scene_shadows(),
            2048,
            &frame,
            &[],
            256,
        );
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
            256,
        );
        assert!(
            off.fog[3].abs() < f32::EPSILON,
            "fog disabled packs density 0"
        );
    }
}
