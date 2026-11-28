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

/// A spotlight's cone: the light shines along `direction` only, at full
/// strength inside `inner_angle` and fading to nothing at `outer_angle`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpotCone {
    /// The direction the light shines (normalized on use).
    pub direction: Vec3,
    /// Angle from the axis (degrees) inside which the light is at full
    /// strength.
    pub inner_angle: f32,
    /// Angle from the axis (degrees) where the light has faded out: the
    /// cone's edge, up to [`SpotCone::MAX_ANGLE`].
    pub outer_angle: f32,
}

impl SpotCone {
    /// The widest cone edge (degrees from the axis).
    pub const MAX_ANGLE: f32 = 80.0;

    /// A cone along `direction` whose edge is `outer_angle` degrees from
    /// the axis, fading over its outer quarter.
    #[must_use]
    pub fn new(direction: Vec3, outer_angle: f32) -> Self {
        Self {
            direction,
            inner_angle: outer_angle * 0.75,
            outer_angle,
        }
    }

    /// The normalized axis (straight down if `direction` is zero).
    #[must_use]
    pub fn axis(&self) -> Vec3 {
        self.direction.try_normalize().unwrap_or(Vec3::NEG_Y)
    }

    /// `(inner, outer)` in radians, clamped: the edge between 1° and
    /// [`Self::MAX_ANGLE`], the full-strength core inside it.
    #[must_use]
    pub fn angles(&self) -> (f32, f32) {
        let outer = self.outer_angle.clamp(1.0, Self::MAX_ANGLE);
        let inner = self.inner_angle.clamp(0.0, outer - 0.5);
        (inner.to_radians(), outer.to_radians())
    }

    /// The shader's cone falloff `clamp(cos θ × scale + offset, 0, 1)`:
    /// 1 inside the core, 0 past the edge.
    pub(crate) fn scale_offset(&self) -> (f32, f32) {
        let (inner, outer) = self.angles();
        let scale = 1.0 / (inner.cos() - outer.cos()).max(1e-4);
        (scale, -outer.cos() * scale)
    }

    /// Half the field of view of the spot's shadow map: the cone plus a
    /// little room for filtering (radians).
    pub(crate) fn shadow_half_fov(&self) -> f32 {
        (self.angles().1 + SPOT_SHADOW_MARGIN.to_radians()).min(85f32.to_radians())
    }
}

/// The largest mesh (bounding radius, m, plus the light's own radius) a
/// light inside it treats as its fixture rather than a shadow caster.
const FIXTURE_RADIUS: f32 = 0.3;

/// How much wider than its cone a spot's shadow map looks (degrees). Must
/// match `spot_shadow_tan` in common.wgsl.
const SPOT_SHADOW_MARGIN: f32 = 4.0;

/// Light radiating from a point (or a small glowing body), fading with the
/// square of the distance: in every direction, or with a [`SpotCone`]
/// only inside it.
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
    /// Makes it a spotlight shining only inside this cone.
    pub spot: Option<SpotCone>,
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
            spot: None,
        }
    }

    /// A spotlight at `position` shining along `direction` in a cone whose
    /// edge is `outer_angle` degrees from the axis.
    #[must_use]
    pub fn spot(position: Vec3, direction: Vec3, intensity: [f32; 3], outer_angle: f32) -> Self {
        Self {
            spot: Some(SpotCone::new(direction, outer_angle)),
            ..Self::new(position, intensity)
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
            spot: None,
        }
    }

    /// The shaders' layout: position + range, intensity + radius, the
    /// shadow (its first layer or -1, near and far planes) with the cone
    /// offset, and the cone axis with its scale (offset 1 and scale 0 for
    /// lights shining every way).
    pub(crate) fn packed(&self, shadow: Option<(usize, &PointShadow)>) -> [f32; 16] {
        let [r, g, b] = self.intensity.map(|c| c.max(0.0));
        let p = self.position;
        #[allow(clippy::cast_precision_loss)] // a handful of layers
        let (layer, near, far) =
            shadow.map_or((-1.0, 0.0, 0.0), |(layer, s)| (layer as f32, s.near, s.far));
        let (axis, scale, offset) = self.spot.map_or((Vec3::ZERO, 0.0, 1.0), |cone| {
            let (scale, offset) = cone.scale_offset();
            (cone.axis(), scale, offset)
        });
        [
            p.x,
            p.y,
            p.z,
            self.effective_range(),
            r,
            g,
            b,
            self.radius.max(0.01),
            layer,
            near,
            far,
            offset,
            axis.x,
            axis.y,
            axis.z,
            scale,
        ]
    }

    /// Whether a mesh bounded by this sphere is the light's own fixture
    /// (a bulb, a flame, a lens): small and around the light, so it is left
    /// out of the light's shadow instead of blocking all of it.
    #[must_use]
    pub fn inside_fixture(&self, center: Vec3, radius: f32) -> bool {
        radius <= FIXTURE_RADIUS + self.radius && center.distance(self.position) < radius
    }

    /// How many shadow map layers its shadow takes: six cube faces, or
    /// one for a spotlight.
    #[must_use]
    pub fn shadow_layers(&self) -> usize {
        if self.spot.is_some() {
            1
        } else {
            6
        }
    }
}

/// The most point lights a frame can shadow (the shaders' face array holds
/// six per light). A spotlight takes one face, so up to six spots share one
/// point light's room.
pub const MAX_POINT_SHADOWS: usize = 4;

/// Each face of a point light's shadow cube sees a little more than 90°,
/// so soft-shadow taps near a face's edge stay inside it.
pub(crate) const POINT_SHADOW_FOV_DEGREES: f32 = 96.0;

/// The views a light's shadow renders: a point light's six, +X, -X, +Y,
/// -Y, +Z, -Z (the order the shaders pick faces in), or a spotlight's one
/// along its cone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PointShadow {
    pub faces: [glam::Mat4; 6],
    /// How many of `faces` are used (6, or 1 for a spotlight).
    pub count: usize,
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
        let p = light.position;
        if let Some(cone) = light.spot {
            let axis = cone.axis();
            let up = if axis.y.abs() > 0.99 {
                Vec3::Z
            } else {
                Vec3::Y
            };
            let proj = directx::perspective(2.0 * cone.shadow_half_fov(), 1.0, near, far);
            return Self {
                faces: [proj * view::look_at_mat4(p, p + axis, up); 6],
                count: 1,
                near,
                far,
            };
        }
        let proj = directx::perspective(POINT_SHADOW_FOV_DEGREES.to_radians(), 1.0, near, far);
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
            count: 6,
            near,
            far,
        }
    }

    /// Whether a sphere can cast into face `face` of `light`'s shadow (a
    /// spotlight's single face: inside its view pyramid, approximated by
    /// the cone through its corners).
    pub fn light_face_sees(light: &PointLight, face: usize, center: Vec3, radius: f32) -> bool {
        let range = light.effective_range();
        let Some(cone) = light.spot else {
            return Self::face_sees(face, light.position, center, radius, range);
        };
        let d = center - light.position;
        let dist = d.length();
        if dist > range + radius {
            return false;
        }
        if dist <= radius {
            return true;
        }
        let angle = (d / dist).dot(cone.axis()).clamp(-1.0, 1.0).acos();
        let half_diagonal = (cone.shadow_half_fov().tan() * std::f32::consts::SQRT_2).atan();
        angle - (radius / dist).asin() <= half_diagonal
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
/// from (left out of its shadow), and its shadow's first layer and views.
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
    /// Depth compares per pixel per shadowed point light (see
    /// [`GraphicsQuality::point_shadow_taps`](crate::GraphicsQuality::point_shadow_taps)).
    pub point_shadow_taps: u32,
    /// The water around the camera and the waters throwing caustics.
    pub water: crate::water::WaterView,
}

impl Default for FrameView {
    fn default() -> Self {
        Self {
            camera: crate::Camera3D::default(),
            viewport: (1, 1),
            background: [0.0; 3],
            time: 0.0,
            beautiful: false,
            point_shadow_taps: 4,
            water: crate::water::WaterView::default(),
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
    /// rgb: the water around the camera's colour, a: its extinction per
    /// metre (0 = the camera isn't in water).
    pub underwater: [f32; 4],
    /// x: surface height above the camera, y: near-plane half height,
    /// z: waters throwing caustics.
    pub water_eye: [f32; 4],
    /// Caustic waters' world XZ min and max.
    pub water_rects: [[f32; 4]; crate::water::MAX_CAUSTIC_WATERS],
    /// Caustic waters' rest height, strength and clarity.
    pub water_levels: [[f32; 4]; crate::water::MAX_CAUSTIC_WATERS],
    /// Point lights and spotlights (`PointLightData` in common.wgsl); the
    /// first `fog_params.z` are used.
    pub points: [[f32; 16]; MAX_POINT_LIGHTS],
    /// The shadowed lights' face view-projections by shadow map layer: six
    /// per point light, one per spotlight.
    pub point_faces: [[f32; 16]; MAX_POINT_SHADOWS * 6],
    /// x: one shadow texel in UV, y: soft filtering (Beautiful),
    /// z: tan(face FOV / 2), w: ambient occlusion strength (Beautiful).
    pub point_shadow: [f32; 4],
    /// x: the fog layer's top (world Y), y: how fast it thins above that
    /// per metre (0 = plain distance fog), z: how much the sun lights it.
    pub fog_height: [f32; 4],
    /// x: toon bands (0 = smooth), y: band edge softness, z: rim light,
    /// w: 1 = painted sky.
    pub style: [f32; 4],
    /// rgb: the shade's tint (white = none).
    pub shadow_tint: [f32; 4],
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
        let mut points = [[0.0; 16]; MAX_POINT_LIGHTS];
        let mut point_faces = [[0.0; 16]; MAX_POINT_SHADOWS * 6];
        for (slot, p) in points.iter_mut().zip(point_lights) {
            let shadow = p
                .shadow
                .filter(|(layer, v)| layer + v.count <= MAX_POINT_SHADOWS * 6);
            *slot = p.light.packed(shadow.as_ref().map(|(l, v)| (*l, v)));
            if let Some((layer, view)) = shadow {
                for (face, m) in view.faces.iter().take(view.count).enumerate() {
                    point_faces[layer + face] = m.to_cols_array();
                }
            }
        }
        let point_count = point_lights.len().min(MAX_POINT_LIGHTS) as f32;
        let point_shadow = [
            1.0 / point_map_size.max(1) as f32,
            frame.point_shadow_taps as f32,
            (POINT_SHADOW_FOV_DEGREES * 0.5).to_radians().tan(),
            // Ambient occlusion strength (Beautiful only).
            if frame.beautiful {
                env.ambient_occlusion.clamp(0.0, 2.0)
            } else {
                0.0
            },
        ];
        let direction = sun.normalized_direction();
        let (style, shadow_tint) = style_params(env);
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
            underwater: frame.water.underwater,
            water_eye: frame.water.eye,
            water_rects: frame.water.rects,
            water_levels: frame.water.levels,
            points,
            point_faces,
            point_shadow,
            fog_height: height_fog(env),
            style,
            shadow_tint,
        }
    }
}

/// The shading style: toon bands, their softness, rim light and painted
/// sky; and the shade's tint.
fn style_params(env: &Environment) -> ([f32; 4], [f32; 4]) {
    let [r, g, b] = env.shadow_tint.map(|c| c.clamp(0.0, 2.0));
    (
        [
            f32::from(u8::try_from(env.toon_bands.min(4)).unwrap_or(4)),
            env.toon_softness.clamp(0.0, 1.0),
            env.rim_light.clamp(0.0, 2.0),
            f32::from(u8::from(env.painted_sky)),
        ],
        [r, g, b, 0.0],
    )
}

/// The height fog layer: its top, falloff per metre and sun glow.
fn height_fog(env: &Environment) -> [f32; 4] {
    [
        env.fog_height,
        env.fog_falloff.clamp(0.0, 10.0),
        env.fog_sun.clamp(0.0, 4.0),
        0.0,
    ]
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
                    shadow: Some((6, PointShadow::for_light(&dark))),
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
        assert!((u.points[1][8] - 6.0).abs() < f32::EPSILON, "layers 6..12");
        // Shining every way: no cone falloff.
        assert_eq!(&packed[11..], &[1.0, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(
            u.point_faces[6],
            PointShadow::for_light(&dark).faces[0].to_cols_array()
        );
    }

    /// common.wgsl's `point_shadow_coords` for a point light, without the
    /// normal offset: the face's (ndc x, ndc y, depth) and layer offset.
    fn shader_face_projection(d: Vec3, near: f32, far: f32) -> (Vec3, usize) {
        let a = d.abs();
        let (face, forward_axis, right, up) = if a.x >= a.y && a.x >= a.z {
            let sx = d.x.signum();
            (usize::from(d.x <= 0.0), Vec3::X * sx, Vec3::Z * sx, Vec3::Y)
        } else if a.y >= a.z {
            let sy = d.y.signum();
            (
                2 + usize::from(d.y <= 0.0),
                Vec3::Y * sy,
                Vec3::X * sy,
                Vec3::Z,
            )
        } else {
            let sz = d.z.signum();
            (
                4 + usize::from(d.z <= 0.0),
                Vec3::Z * sz,
                Vec3::X * -sz,
                Vec3::Y,
            )
        };
        let half_tan = (POINT_SHADOW_FOV_DEGREES * 0.5).to_radians().tan();
        let forward = d.dot(forward_axis);
        let x = d.dot(right) / (forward * half_tan);
        let y = d.dot(up) / (forward * half_tan);
        let depth = far * (forward - near) / ((far - near) * forward);
        (Vec3::new(x, y, depth), face)
    }

    #[test]
    fn shader_face_projection_matches_the_shadow_pass_matrices() {
        let light = PointLight {
            range: 12.0,
            radius: 0.2,
            ..PointLight::new(Vec3::new(1.0, 2.0, -3.0), [1.0; 3])
        };
        let s = PointShadow::for_light(&light);
        let mut checked = 0;
        for x in -3..=3 {
            for y in -3..=3 {
                for z in -3..=3 {
                    #[allow(clippy::cast_precision_loss)] // small integers
                    let d = Vec3::new(
                        x as f32 * 0.7 + 0.11,
                        y as f32 * 0.9 - 0.07,
                        z as f32 * 1.1 + 0.03,
                    );
                    let (analytic, face) = shader_face_projection(d, s.near, s.far);
                    let clip = s.faces[face] * (light.position + d).extend(1.0);
                    let ndc = clip.truncate() / clip.w;
                    assert!(
                        (analytic - ndc).abs().max_element() < 1e-4,
                        "{d:?} face {face}: {analytic:?} vs {ndc:?}"
                    );
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, 343);
    }

    /// common.wgsl's spot view: tan of its half field of view from the
    /// packed cone (no acos/tan), and its axes.
    fn shader_spot_view(packed: &[f32; 16]) -> (f32, Vec3, Vec3, Vec3) {
        let axis = Vec3::new(packed[12], packed[13], packed[14]);
        let cos_edge = (-packed[11] / packed[15].max(1e-6)).clamp(0.05, 1.0);
        let tan_edge = (1.0 - cos_edge * cos_edge).sqrt() / cos_edge;
        let tan_margin = 0.069_926_8;
        let denom = 1.0 - tan_edge * tan_margin;
        let tan_max = 11.430_052;
        let half_tan = if denom > 1e-3 {
            ((tan_edge + tan_margin) / denom).min(tan_max)
        } else {
            tan_max
        };
        let up0 = if axis.y.abs() > 0.99 {
            Vec3::Z
        } else {
            Vec3::Y
        };
        let right = axis.cross(up0).normalize();
        (half_tan, axis, right, right.cross(axis))
    }

    #[test]
    fn shader_spot_view_matches_the_shadow_pass_matrix() {
        for (dir, outer) in [
            (Vec3::NEG_Y, 30.0),
            (Vec3::new(1.0, -0.5, 0.3), 55.0),
            (Vec3::new(0.0, 0.2, -1.0), 80.0),
            (Vec3::Y, 10.0),
        ] {
            let spot = PointLight {
                range: 9.0,
                ..PointLight::spot(Vec3::new(0.5, 3.0, -1.0), dir, [1.0; 3], outer)
            };
            let view = PointShadow::for_light(&spot);
            let packed = spot.packed(Some((0, &view)));
            let (half_tan, axis, right, up) = shader_spot_view(&packed);
            let expected = spot.spot.unwrap().shadow_half_fov().tan();
            assert!(
                (half_tan - expected).abs() < 1e-4 * expected,
                "{outer}: {half_tan} vs {expected}"
            );
            for d in [
                axis * 2.0 + right * 0.3,
                axis * 5.0 - up * 0.8 + right * 0.5,
                axis * 0.7,
            ] {
                let forward = d.dot(axis);
                let analytic = Vec3::new(
                    d.dot(right) / (forward * half_tan),
                    d.dot(up) / (forward * half_tan),
                    view.far * (forward - view.near) / ((view.far - view.near) * forward),
                );
                let clip = view.faces[0] * (spot.position + d).extend(1.0);
                let ndc = clip.truncate() / clip.w;
                assert!(
                    (analytic - ndc).abs().max_element() < 1e-3,
                    "{outer} {d:?}: {analytic:?} vs {ndc:?}"
                );
            }
        }
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
    fn spot_cones_fade_from_core_to_edge_and_shadow_one_face() {
        let light = PointLight {
            range: 10.0,
            ..PointLight::spot(Vec3::new(0.0, 3.0, 0.0), Vec3::NEG_Y * 2.0, [5.0; 3], 30.0)
        };
        let cone = light.spot.unwrap();
        assert!((cone.inner_angle - 22.5).abs() < 1e-5);
        assert_eq!(light.shadow_layers(), 1);
        let packed = light.packed(None);
        let (offset, axis, scale) = (
            packed[11],
            Vec3::new(packed[12], packed[13], packed[14]),
            packed[15],
        );
        assert!((axis - Vec3::NEG_Y).length() < 1e-6, "normalized axis");
        // The shader's falloff: full on the axis and inside the core, none
        // past the edge, in between across the fade.
        let falloff = |deg: f32| (deg.to_radians().cos() * scale + offset).clamp(0.0, 1.0);
        assert!((falloff(0.0) - 1.0).abs() < 1e-6);
        assert!((falloff(22.0) - 1.0).abs() < 1e-6);
        assert!(falloff(26.0) > 0.1 && falloff(26.0) < 0.9);
        assert!(falloff(30.5) <= 0.0);
        // Degenerate angles are clamped into a usable cone.
        let wide = SpotCone {
            inner_angle: 120.0,
            outer_angle: 170.0,
            ..cone
        };
        let (inner, outer) = wide.angles();
        assert!(outer <= SpotCone::MAX_ANGLE.to_radians() + 1e-6 && inner < outer);

        // The shadow is a single view down the cone.
        let s = PointShadow::for_light(&light);
        assert_eq!(s.count, 1);
        let below = s.faces[0] * Vec3::new(0.0, 0.0, 0.0).extend(1.0);
        let ndc = below.truncate() / below.w;
        assert!(ndc.x.abs() < 1e-4 && ndc.y.abs() < 1e-4 && ndc.z > 0.0 && ndc.z < 1.0);
        // The cone's edge stays inside the view.
        let edge = Vec3::new(0.0, 3.0, 0.0) + Vec3::new(30f32.to_radians().tan(), -1.0, 0.0);
        let clip = s.faces[0] * edge.extend(1.0);
        assert!((clip.x / clip.w).abs() < 1.0);
        // A small mesh around the light is its fixture; a room around it
        // still casts.
        assert!(light.inside_fixture(Vec3::new(0.0, 3.05, 0.0), 0.1));
        assert!(!light.inside_fixture(Vec3::new(0.0, 3.5, 0.0), 0.1));
        assert!(!light.inside_fixture(Vec3::new(0.0, 3.0, 0.0), 5.0));
        // Culling: below it casts, above it or off to the side doesn't.
        assert!(PointShadow::light_face_sees(
            &light,
            0,
            Vec3::new(0.5, 0.0, 0.0),
            0.5
        ));
        assert!(!PointShadow::light_face_sees(
            &light,
            0,
            Vec3::new(0.0, 6.0, 0.0),
            0.5
        ));
        assert!(!PointShadow::light_face_sees(
            &light,
            0,
            Vec3::new(8.0, 2.5, 0.0),
            0.5
        ));
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
