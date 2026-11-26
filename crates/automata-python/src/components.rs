//! Python-facing component classes: plain-data builders that are staged on
//! [`crate::scene::PyEntity`] and materialized into ECS components when the
//! scene loads.

use automata_renderer::{Camera2D, Camera3D, Environment, PostEffects, Tonemapping, Water};
use automata_scene::{DirectionalLight, PointLight, SpotLight, Transform};
use glam::{EulerRot, Quat, Vec2, Vec3};
use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;

/// Converts a Python 3-tuple of floats to a `Vec3`.
fn vec3_of(t: (f32, f32, f32)) -> Vec3 {
    Vec3::new(t.0, t.1, t.2)
}

/// Converts a `Vec3` to a Python-friendly 3-tuple.
fn tuple_of(v: Vec3) -> (f32, f32, f32) {
    (v.x, v.y, v.z)
}

/// Position, rotation, and scale of an entity, relative to its parent.
///
/// Rotation is exposed as Euler angles in radians, applied in XYZ order.
/// For parented entities the values are local; the engine computes the
/// world-space transform every frame.
///
/// Properties are mutable: `t.position = (1, 2, 0)`.
#[pyclass(name = "Transform", module = "automata._core")]
#[derive(Debug, Clone, Copy)]
pub struct PyTransform {
    inner: Transform,
}

#[pymethods]
impl PyTransform {
    /// Creates a transform. All parameters are optional 3-tuples.
    #[new]
    #[pyo3(signature = (position=(0.0, 0.0, 0.0), rotation_euler=(0.0, 0.0, 0.0), scale=(1.0, 1.0, 1.0)))]
    fn new(
        position: (f32, f32, f32),
        rotation_euler: (f32, f32, f32),
        scale: (f32, f32, f32),
    ) -> Self {
        let (x, y, z) = rotation_euler;
        Self {
            inner: Transform {
                position: vec3_of(position),
                rotation: Quat::from_euler(EulerRot::XYZ, x, y, z),
                scale: vec3_of(scale),
            },
        }
    }

    /// The `(x, y, z)` position relative to the parent.
    #[getter]
    fn position(&self) -> (f32, f32, f32) {
        tuple_of(self.inner.position)
    }

    #[setter]
    fn set_position(&mut self, position: (f32, f32, f32)) {
        self.inner.position = vec3_of(position);
    }

    /// The rotation as Euler angles `(x, y, z)` in radians.
    #[getter]
    fn rotation_euler(&self) -> (f32, f32, f32) {
        let (x, y, z) = self.inner.rotation.to_euler(EulerRot::XYZ);
        (x, y, z)
    }

    #[setter]
    fn set_rotation_euler(&mut self, rotation: (f32, f32, f32)) {
        let (x, y, z) = rotation;
        self.inner.rotation = Quat::from_euler(EulerRot::XYZ, x, y, z);
    }

    /// The `(x, y, z)` scale relative to the parent.
    #[getter]
    fn scale(&self) -> (f32, f32, f32) {
        tuple_of(self.inner.scale)
    }

    #[setter]
    fn set_scale(&mut self, scale: (f32, f32, f32)) {
        self.inner.scale = vec3_of(scale);
    }

    /// A detached copy of this transform.
    fn copy(&self) -> Self {
        *self
    }

    fn __repr__(&self) -> String {
        let (px, py, pz) = tuple_of(self.inner.position);
        format!("Transform(position=({px:.3}, {py:.3}, {pz:.3}))")
    }
}

impl PyTransform {
    /// The engine component value.
    pub fn inner(&self) -> Transform {
        self.inner
    }

    /// Wraps an engine transform for Python.
    pub fn from_inner(inner: Transform) -> Self {
        Self { inner }
    }
}

/// A perspective 3D camera. The first entity carrying one becomes the
/// frame's 3D camera.
#[pyclass(name = "Camera3D", module = "automata._core", frozen)]
#[derive(Debug, Clone, Copy)]
pub struct PyCamera3D {
    inner: Camera3D,
}

#[pymethods]
impl PyCamera3D {
    /// Creates a camera at `position` looking at `target`.
    #[new]
    #[pyo3(signature = (position=(0.0, 0.0, 0.0), target=(0.0, 0.0, -1.0), fov=60.0))]
    fn new(position: (f32, f32, f32), target: (f32, f32, f32), fov: f32) -> Self {
        let mut inner = Camera3D::new(vec3_of(position), vec3_of(target));
        inner.fov_y_degrees = fov;
        Self { inner }
    }

    /// The camera position in world space.
    #[getter]
    fn position(&self) -> (f32, f32, f32) {
        tuple_of(self.inner.position)
    }

    /// The point the camera looks at.
    #[getter]
    fn target(&self) -> (f32, f32, f32) {
        tuple_of(self.inner.target)
    }

    /// The vertical field of view in degrees.
    #[getter]
    fn fov(&self) -> f32 {
        self.inner.fov_y_degrees
    }
}

impl PyCamera3D {
    /// The engine component value.
    pub fn inner(&self) -> Camera3D {
        self.inner
    }

    /// Wraps an engine camera value.
    pub fn from_inner(inner: Camera3D) -> Self {
        Self { inner }
    }
}

/// The scene's directional sun: parallel light rays plus ambient fill.
///
/// Attach it to any entity; the first entity carrying one lights the frame.
/// With `shadows=True` (the default) the sun casts shadow-mapped shadows.
///
/// ```python
/// scene.add(at.Entity("sun").add(at.DirectionalLight(
///     direction=(-0.4, -1.0, -0.3), color=(1.0, 0.95, 0.85), ambient=0.35)))
/// ```
#[pyclass(name = "DirectionalLight", module = "automata._core", frozen)]
#[derive(Debug, Clone, Copy)]
pub struct PyDirectionalLight {
    inner: DirectionalLight,
}

#[pymethods]
impl PyDirectionalLight {
    /// Creates a sun shining along `direction`. `shadow_distance` is how
    /// far from the camera its shadows reach (metres;
    /// shorter is sharper) and `shadow_softness` how soft their edges are
    /// (0 sharpest, 1 default, up to 8).
    #[new]
    #[pyo3(signature = (direction=(-0.5, -1.0, -0.35), color=(1.0, 1.0, 1.0), ambient=0.3,
                        shadows=true, *, shadow_distance=120.0, shadow_softness=1.0))]
    fn new(
        direction: (f32, f32, f32),
        color: (f32, f32, f32),
        ambient: f32,
        shadows: bool,
        shadow_distance: f32,
        shadow_softness: f32,
    ) -> PyResult<Self> {
        if shadow_distance.is_nan()
            || shadow_distance <= 0.0
            || !(0.0..=8.0).contains(&shadow_softness)
        {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "shadow_distance must be above 0 and shadow_softness between 0 and 8",
            ));
        }
        Ok(Self {
            inner: DirectionalLight {
                direction: vec3_of(direction),
                color: [color.0, color.1, color.2],
                ambient,
                shadows,
                shadow_distance,
                shadow_softness,
            },
        })
    }

    /// The direction the light travels.
    #[getter]
    fn direction(&self) -> (f32, f32, f32) {
        tuple_of(self.inner.direction)
    }

    /// The light color (linear RGB).
    #[getter]
    fn color(&self) -> (f32, f32, f32) {
        let [r, g, b] = self.inner.color;
        (r, g, b)
    }

    /// The ambient light fraction.
    #[getter]
    fn ambient(&self) -> f32 {
        self.inner.ambient
    }

    /// Whether the light casts shadows.
    #[getter]
    fn shadows(&self) -> bool {
        self.inner.shadows
    }

    /// How far from the camera shadows reach (metres).
    #[getter]
    fn shadow_distance(&self) -> f32 {
        self.inner.shadow_distance
    }

    /// How soft shadow edges are (0 sharpest, 1 default).
    #[getter]
    fn shadow_softness(&self) -> f32 {
        self.inner.shadow_softness
    }

    fn __repr__(&self) -> String {
        let d = self.inner.direction;
        format!(
            "DirectionalLight(direction=({:.2}, {:.2}, {:.2}), ambient={:.2}, shadows={})",
            d.x, d.y, d.z, self.inner.ambient, self.inner.shadows
        )
    }
}

impl PyDirectionalLight {
    /// The engine component value.
    pub fn inner(&self) -> DirectionalLight {
        self.inner
    }

    /// Wraps an engine light for Python (used by scene enumeration).
    pub fn from_inner(inner: DirectionalLight) -> Self {
        Self { inner }
    }
}

/// A light placed in the scene shining every way from its entity's
/// position: a torch, a lamp, a campfire. It fades with the square of the
/// distance and reaches nothing past `range` (0 picks it from the
/// brightness). `intensity` 1 lights a surface 1 m away as brightly as a
/// white sun overhead.
///
/// ```python
/// scene.add(at.Entity("torch").add(at.Transform(position=(0, 2, -3)))
///                             .add(at.PointLight(color=(1.0, 0.6, 0.3), intensity=4.0)))
/// ```
#[pyclass(name = "PointLight", module = "automata._core", frozen)]
#[derive(Debug, Clone, Copy)]
pub struct PyPointLight {
    inner: PointLight,
}

fn check_light(intensity: f32, range: f32, radius: f32) -> PyResult<()> {
    if !(intensity >= 0.0 && range >= 0.0 && radius > 0.0) {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "intensity and range must be 0 or more and radius above 0",
        ));
    }
    Ok(())
}

#[pymethods]
impl PyPointLight {
    /// Creates a point light. `radius` is the glowing source's size
    /// (bigger is softer up close); `shadows` lets objects block it (the
    /// nearest few shadowed lights get shadows on High and up).
    #[new]
    #[pyo3(signature = (color=(1.0, 0.85, 0.65), intensity=3.0, range=0.0, *, radius=0.05,
                        shadows=true))]
    fn new(
        color: (f32, f32, f32),
        intensity: f32,
        range: f32,
        radius: f32,
        shadows: bool,
    ) -> PyResult<Self> {
        check_light(intensity, range, radius)?;
        Ok(Self {
            inner: PointLight {
                color: [color.0, color.1, color.2],
                intensity,
                range,
                radius,
                shadows,
            },
        })
    }

    /// The light color (linear RGB).
    #[getter]
    fn color(&self) -> (f32, f32, f32) {
        let [r, g, b] = self.inner.color;
        (r, g, b)
    }

    /// The brightness.
    #[getter]
    fn intensity(&self) -> f32 {
        self.inner.intensity
    }

    /// Where the light ends (metres; 0 = picked from the brightness).
    #[getter]
    fn range(&self) -> f32 {
        self.inner.range
    }

    /// The glowing source's size (metres).
    #[getter]
    fn radius(&self) -> f32 {
        self.inner.radius
    }

    /// Whether objects block it and cast shadows.
    #[getter]
    fn shadows(&self) -> bool {
        self.inner.shadows
    }

    fn __repr__(&self) -> String {
        let [r, g, b] = self.inner.color;
        format!(
            "PointLight(color=({r:.2}, {g:.2}, {b:.2}), intensity={:.2}, range={:.2}, shadows={})",
            self.inner.intensity,
            self.inner.range,
            if self.inner.shadows { "True" } else { "False" }
        )
    }
}

impl PyPointLight {
    /// The engine component value.
    pub fn inner(&self) -> PointLight {
        self.inner
    }

    /// Wraps an engine light for Python (used by scene enumeration).
    pub fn from_inner(inner: PointLight) -> Self {
        Self { inner }
    }
}

/// A light placed in the scene shining in a cone along its entity's
/// forward direction (local -Z, turned by the entity's rotation): a
/// flashlight, a headlight, a stage light. Full strength inside
/// `inner_angle` (degrees from the axis; three quarters of `outer_angle`
/// unless given), faded out by `outer_angle` (at most 80).
///
/// ```python
/// scene.add(at.Entity("headlight")
///           .add(at.Transform(position=(0, 0.7, 2), rotation_euler=(-0.1, 0, 0)))
///           .add(at.SpotLight(intensity=20.0, outer_angle=25.0)))
/// ```
#[pyclass(name = "SpotLight", module = "automata._core", frozen)]
#[derive(Debug, Clone, Copy)]
pub struct PySpotLight {
    inner: SpotLight,
}

#[pymethods]
impl PySpotLight {
    /// Creates a spotlight.
    #[new]
    #[pyo3(signature = (color=(1.0, 1.0, 1.0), intensity=8.0, outer_angle=30.0,
                        inner_angle=None, range=0.0, *, radius=0.05, shadows=true))]
    fn new(
        color: (f32, f32, f32),
        intensity: f32,
        outer_angle: f32,
        inner_angle: Option<f32>,
        range: f32,
        radius: f32,
        shadows: bool,
    ) -> PyResult<Self> {
        check_light(intensity, range, radius)?;
        let inner_angle = inner_angle.unwrap_or(outer_angle * 0.75);
        if !(outer_angle > 0.0
            && outer_angle <= automata_renderer::SpotCone::MAX_ANGLE
            && (0.0..=outer_angle).contains(&inner_angle))
        {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "outer_angle must be between 0 and 80 degrees and inner_angle between 0 and \
                 outer_angle",
            ));
        }
        Ok(Self {
            inner: SpotLight {
                color: [color.0, color.1, color.2],
                intensity,
                range,
                inner_angle,
                outer_angle,
                radius,
                shadows,
            },
        })
    }

    /// The light color (linear RGB).
    #[getter]
    fn color(&self) -> (f32, f32, f32) {
        let [r, g, b] = self.inner.color;
        (r, g, b)
    }

    /// The brightness on the cone's axis.
    #[getter]
    fn intensity(&self) -> f32 {
        self.inner.intensity
    }

    /// The cone's edge (degrees from the axis).
    #[getter]
    fn outer_angle(&self) -> f32 {
        self.inner.outer_angle
    }

    /// The full-strength core (degrees from the axis).
    #[getter]
    fn inner_angle(&self) -> f32 {
        self.inner.inner_angle
    }

    /// Where the light ends (metres; 0 = picked from the brightness).
    #[getter]
    fn range(&self) -> f32 {
        self.inner.range
    }

    /// The glowing source's size (metres).
    #[getter]
    fn radius(&self) -> f32 {
        self.inner.radius
    }

    /// Whether objects block it and cast shadows.
    #[getter]
    fn shadows(&self) -> bool {
        self.inner.shadows
    }

    fn __repr__(&self) -> String {
        format!(
            "SpotLight(intensity={:.2}, outer_angle={:.1}, inner_angle={:.1}, shadows={})",
            self.inner.intensity,
            self.inner.outer_angle,
            self.inner.inner_angle,
            if self.inner.shadows { "True" } else { "False" }
        )
    }
}

impl PySpotLight {
    /// The engine component value.
    pub fn inner(&self) -> SpotLight {
        self.inner
    }

    /// Wraps an engine light for Python (used by scene enumeration).
    pub fn from_inner(inner: SpotLight) -> Self {
        Self { inner }
    }
}

/// Scene-wide look: sky/ground ambient, background, exposure, tone mapping,
/// and distance fog. The first entity carrying one sets the frame's look.
///
/// ```python
/// scene.add(at.Entity("environment").add(at.Environment.preset("dusk")))
/// scene.add(at.Entity("environment").add(at.Environment(
///     tonemapping="aces", exposure=1.2, fog=True, fog_density=0.02)))
/// ```
#[pyclass(name = "Environment", module = "automata._core", frozen)]
#[derive(Debug, Clone, Copy)]
pub struct PyEnvironment {
    inner: Environment,
}

fn tonemapping_of(name: &str) -> PyResult<Tonemapping> {
    Ok(match name.to_ascii_lowercase().as_str() {
        "none" | "" => Tonemapping::None,
        "reinhard" => Tonemapping::Reinhard,
        "aces" => Tonemapping::Aces,
        "agx" => Tonemapping::Agx,
        other => {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "unknown tonemapping {other:?}; expected 'none', 'reinhard', 'aces', or 'agx'"
            )))
        }
    })
}

#[pymethods]
impl PyEnvironment {
    /// Creates an environment. Defaults reproduce the plain look (white
    /// ambient, no fog, no tone mapping).
    #[new]
    #[pyo3(signature = (
        sky_color=(1.0, 1.0, 1.0),
        ground_color=(1.0, 1.0, 1.0),
        background=(0.012, 0.012, 0.012),
        exposure=1.0,
        tonemapping="none",
        fog=false,
        fog_color=(0.5, 0.55, 0.6),
        fog_density=0.02,
        fog_start=0.0,
        sky=false,
        zenith_color=(0.16, 0.34, 0.78),
        sun_size=2.0,
        sun_glow=1.0,
        clouds=0.0,
        lens_flare=1.0,
        ambient_occlusion=1.0,
        fog_height=0.0,
        fog_falloff=0.0,
        fog_sun=0.0,
        god_rays=0.0,
        depth_of_field=0.0,
        focus_distance=0.0,
        focus_range=1.0,
        toon_bands=0,
        toon_softness=0.15,
        shadow_tint=(1.0, 1.0, 1.0),
        rim_light=0.0,
        painted_sky=false,
        outline=0.0,
        outline_color=(0.05, 0.04, 0.06),
        pixelate=0.0,
        palette=0,
        wind=0.0,
        wind_direction=0.0,
        post=None,
        bloom=None,
        contrast=None,
        saturation=None,
        temperature=None,
        tint=None,
        lift=None,
        gamma=None,
        gain=None,
        vignette=None,
        grain=None,
        chromatic_aberration=None,
    ))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        sky_color: (f32, f32, f32),
        ground_color: (f32, f32, f32),
        background: (f32, f32, f32),
        exposure: f32,
        tonemapping: &str,
        fog: bool,
        fog_color: (f32, f32, f32),
        fog_density: f32,
        fog_start: f32,
        sky: bool,
        zenith_color: (f32, f32, f32),
        sun_size: f32,
        sun_glow: f32,
        clouds: f32,
        lens_flare: f32,
        ambient_occlusion: f32,
        fog_height: f32,
        fog_falloff: f32,
        fog_sun: f32,
        god_rays: f32,
        depth_of_field: f32,
        focus_distance: f32,
        focus_range: f32,
        toon_bands: u32,
        toon_softness: f32,
        shadow_tint: (f32, f32, f32),
        rim_light: f32,
        painted_sky: bool,
        outline: f32,
        outline_color: (f32, f32, f32),
        pixelate: f32,
        palette: u32,
        wind: f32,
        wind_direction: f32,
        post: Option<&str>,
        bloom: Option<f32>,
        contrast: Option<f32>,
        saturation: Option<f32>,
        temperature: Option<f32>,
        tint: Option<f32>,
        lift: Option<(f32, f32, f32)>,
        gamma: Option<(f32, f32, f32)>,
        gain: Option<(f32, f32, f32)>,
        vignette: Option<f32>,
        grain: Option<f32>,
        chromatic_aberration: Option<f32>,
    ) -> PyResult<Self> {
        let rgb = |c: (f32, f32, f32)| [c.0, c.1, c.2];
        let post = PostArgs {
            bloom,
            contrast,
            saturation,
            temperature,
            tint,
            lift,
            gamma,
            gain,
            vignette,
            grain,
            chromatic_aberration,
        }
        .apply(post_preset(post, PostEffects::default())?);
        Ok(Self {
            inner: Environment {
                sky_color: rgb(sky_color),
                ground_color: rgb(ground_color),
                background: rgb(background),
                exposure,
                tonemapping: tonemapping_of(tonemapping)?,
                fog,
                fog_color: rgb(fog_color),
                fog_density,
                fog_start,
                sky,
                zenith_color: rgb(zenith_color),
                sun_size,
                sun_glow,
                clouds,
                lens_flare,
                ambient_occlusion,
                fog_height,
                fog_falloff,
                fog_sun,
                god_rays,
                depth_of_field,
                focus_distance,
                focus_range,
                toon_bands,
                toon_softness,
                shadow_tint: rgb(shadow_tint),
                rim_light,
                painted_sky,
                outline,
                outline_color: rgb(outline_color),
                pixelate,
                palette,
                wind,
                wind_direction,
                post,
            },
        })
    }

    /// A style: ``"realistic"``, ``"semi-realistic"``, ``"botw"``,
    /// ``"a short hike"`` or ``"nier"`` (case, spaces and dashes don't
    /// matter). Applied to ``base`` (the default environment if omitted):
    /// it sets tone mapping, exposure, the lens, the height fog's shape,
    /// shading, outlines, pixelation and post effects, and keeps the
    /// base's colours, fog colour and density, sky and clouds.
    #[staticmethod]
    #[pyo3(signature = (name, base=None))]
    fn style(name: &str, base: Option<&Self>) -> PyResult<Self> {
        base.map_or_else(Environment::default, |b| b.inner)
            .with_style(name)
            .map(|inner| Self { inner })
            .ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err(format!(
                    "unknown style {name:?}; expected one of {:?}",
                    Environment::STYLES.map(|(n, _)| n)
                ))
            })
    }

    /// A copy of this environment in the style ``name`` (see
    /// :meth:`style`).
    fn with_style(&self, name: &str) -> PyResult<Self> {
        Self::style(name, Some(self))
    }

    /// The style names, in the editor's order.
    #[staticmethod]
    fn styles() -> Vec<String> {
        Environment::STYLES
            .iter()
            .map(|(name, _)| (*name).to_owned())
            .collect()
    }

    /// The style this environment is exactly, or ``"Custom"`` once any of
    /// a style's knobs has changed.
    #[getter]
    fn style_name(&self) -> &'static str {
        self.inner.style_name()
    }

    /// Toon shading: the light in this many flat bands (``0`` = smooth).
    #[getter]
    fn toon_bands(&self) -> u32 {
        self.inner.toon_bands
    }

    /// How soft the edges between toon bands are, ``0`` (crisp) to ``1``.
    #[getter]
    fn toon_softness(&self) -> f32 {
        self.inner.toon_softness
    }

    /// The colour of shade: ambient light out of the sun is multiplied by
    /// it (white = none).
    #[getter]
    fn shadow_tint(&self) -> (f32, f32, f32) {
        let [r, g, b] = self.inner.shadow_tint;
        (r, g, b)
    }

    /// Light along objects' edges, from the sky and a sun behind them
    /// (``0`` = none).
    #[getter]
    fn rim_light(&self) -> f32 {
        self.inner.rim_light
    }

    /// A painted sky: banded gradient, flat clouds, a flat sun disk.
    #[getter]
    fn painted_sky(&self) -> bool {
        self.inner.painted_sky
    }

    /// Outline width in pixels on a 720-pixel-tall screen (Medium graphics
    /// and up; ``0`` = none).
    #[getter]
    fn outline(&self) -> f32 {
        self.inner.outline
    }

    /// The outlines' colour.
    #[getter]
    fn outline_color(&self) -> (f32, f32, f32) {
        let [r, g, b] = self.inner.outline_color;
        (r, g, b)
    }

    /// Pixel art: the size of a pixel on a 720-pixel-tall screen (Medium
    /// graphics and up; ``0`` or ``1`` = off).
    #[getter]
    fn pixelate(&self) -> f32 {
        self.inner.pixelate
    }

    /// A limited palette: levels per colour channel, dithered (Medium
    /// graphics and up; ``0`` = full colour).
    #[getter]
    fn palette(&self) -> u32 {
        self.inner.palette
    }

    /// A copy with post effects: start from the named look ``post``
    /// (``"neutral"``, ``"bleached"``, ``"vivid"``, ``"film"``, ``"noir"``;
    /// ``None`` keeps the current ones) and change any field by keyword.
    #[pyo3(signature = (
        post=None,
        *,
        bloom=None,
        contrast=None,
        saturation=None,
        temperature=None,
        tint=None,
        lift=None,
        gamma=None,
        gain=None,
        vignette=None,
        grain=None,
        chromatic_aberration=None,
    ))]
    #[allow(clippy::too_many_arguments)]
    fn with_post(
        &self,
        post: Option<&str>,
        bloom: Option<f32>,
        contrast: Option<f32>,
        saturation: Option<f32>,
        temperature: Option<f32>,
        tint: Option<f32>,
        lift: Option<(f32, f32, f32)>,
        gamma: Option<(f32, f32, f32)>,
        gain: Option<(f32, f32, f32)>,
        vignette: Option<f32>,
        grain: Option<f32>,
        chromatic_aberration: Option<f32>,
    ) -> PyResult<Self> {
        let post = PostArgs {
            bloom,
            contrast,
            saturation,
            temperature,
            tint,
            lift,
            gamma,
            gain,
            vignette,
            grain,
            chromatic_aberration,
        }
        .apply(post_preset(post, self.inner.post)?);
        Ok(Self {
            inner: Environment { post, ..self.inner },
        })
    }

    /// The names of the post-effect looks, for ``post=``.
    #[staticmethod]
    fn post_presets() -> Vec<String> {
        PostEffects::PRESETS
            .iter()
            .map(|(name, _)| name.to_ascii_lowercase())
            .collect()
    }

    /// The top of the height fog's layer (world Y): full density below,
    /// thinning above by ``fog_falloff``.
    #[getter]
    fn fog_height(&self) -> f32 {
        self.inner.fog_height
    }

    /// How fast fog thins above ``fog_height``, per metre (``0`` = plain
    /// distance fog).
    #[getter]
    fn fog_falloff(&self) -> f32 {
        self.inner.fog_falloff
    }

    /// How much the sun lights the fog (``0`` = none).
    #[getter]
    fn fog_sun(&self) -> f32 {
        self.inner.fog_sun
    }

    /// Light shafts from the sun (Beautiful graphics; ``0`` = off).
    #[getter]
    fn god_rays(&self) -> f32 {
        self.inner.god_rays
    }

    /// Depth-of-field blur strength (Beautiful graphics; ``0`` = off).
    #[getter]
    fn depth_of_field(&self) -> f32 {
        self.inner.depth_of_field
    }

    /// The distance in focus in metres (``0`` = the camera's target).
    #[getter]
    fn focus_distance(&self) -> f32 {
        self.inner.focus_distance
    }

    /// How deep the sharp zone around the focus is, in metres.
    #[getter]
    fn focus_range(&self) -> f32 {
        self.inner.focus_range
    }

    /// Glow around bright things (High and Beautiful graphics; ``0`` = off).
    #[getter]
    fn bloom(&self) -> f32 {
        self.inner.post.bloom
    }

    /// Contrast around mid grey (``1`` = unchanged).
    #[getter]
    fn contrast(&self) -> f32 {
        self.inner.post.contrast
    }

    /// Colour saturation (``0`` = black and white, ``1`` = unchanged).
    #[getter]
    fn saturation(&self) -> f32 {
        self.inner.post.saturation
    }

    /// White balance, ``-1`` (cool) to ``1`` (warm).
    #[getter]
    fn temperature(&self) -> f32 {
        self.inner.post.temperature
    }

    /// White balance, ``-1`` (green) to ``1`` (magenta).
    #[getter]
    fn tint(&self) -> f32 {
        self.inner.post.tint
    }

    /// Added to the shadows per channel (``0`` = none).
    #[getter]
    fn lift(&self) -> (f32, f32, f32) {
        let [r, g, b] = self.inner.post.lift;
        (r, g, b)
    }

    /// Midtone power per channel (``1`` = unchanged, above 1 brightens).
    #[getter]
    fn gamma(&self) -> (f32, f32, f32) {
        let [r, g, b] = self.inner.post.gamma;
        (r, g, b)
    }

    /// Highlight multiplier per channel (``1`` = unchanged).
    #[getter]
    fn gain(&self) -> (f32, f32, f32) {
        let [r, g, b] = self.inner.post.gain;
        (r, g, b)
    }

    /// Darkened corners, ``0`` (none) to ``1``.
    #[getter]
    fn vignette(&self) -> f32 {
        self.inner.post.vignette
    }

    /// Animated film grain, ``0`` (none) to ``1``.
    #[getter]
    fn grain(&self) -> f32 {
        self.inner.post.grain
    }

    /// Colour fringes toward the frame's edges, ``0`` (none) to ``1``.
    #[getter]
    fn chromatic_aberration(&self) -> f32 {
        self.inner.post.chromatic_aberration
    }

    /// A named look: ``"neutral"``, ``"daylight"``, ``"dusk"``, or ``"night"``.
    #[staticmethod]
    fn preset(name: &str) -> PyResult<Self> {
        let inner = match name.to_ascii_lowercase().as_str() {
            "neutral" | "default" => Environment::default(),
            "daylight" | "day" => Environment::daylight(),
            "dusk" => Environment::dusk(),
            "night" => Environment::night(),
            other => {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "unknown preset {other:?}; expected 'neutral', 'daylight', 'dusk', or 'night'"
                )))
            }
        };
        Ok(Self { inner })
    }

    /// Ambient light from above (linear RGB).
    #[getter]
    fn sky_color(&self) -> (f32, f32, f32) {
        let [r, g, b] = self.inner.sky_color;
        (r, g, b)
    }

    /// Ambient light from below (linear RGB).
    #[getter]
    fn ground_color(&self) -> (f32, f32, f32) {
        let [r, g, b] = self.inner.ground_color;
        (r, g, b)
    }

    /// The clear color behind all geometry (linear RGB).
    #[getter]
    fn background(&self) -> (f32, f32, f32) {
        let [r, g, b] = self.inner.background;
        (r, g, b)
    }

    /// Brightness multiplier before tone mapping.
    #[getter]
    fn exposure(&self) -> f32 {
        self.inner.exposure
    }

    /// The tone mapping curve: ``"none"``, ``"reinhard"``, ``"aces"``, ``"agx"``.
    #[getter]
    fn tonemapping(&self) -> &'static str {
        match self.inner.tonemapping {
            Tonemapping::None => "none",
            Tonemapping::Reinhard => "reinhard",
            Tonemapping::Aces => "aces",
            Tonemapping::Agx => "agx",
        }
    }

    /// Whether distance fog is on.
    #[getter]
    fn fog(&self) -> bool {
        self.inner.fog
    }

    /// Fog color (linear RGB).
    #[getter]
    fn fog_color(&self) -> (f32, f32, f32) {
        let [r, g, b] = self.inner.fog_color;
        (r, g, b)
    }

    /// Exponential fog density per world unit.
    #[getter]
    fn fog_density(&self) -> f32 {
        self.inner.fog_density
    }

    /// How hard the wind blows, ``0`` (still) to ``1`` (a gale): grass
    /// sways and spring bones (hair, cloth, leaves) stream with it.
    #[getter]
    fn wind(&self) -> f32 {
        self.inner.wind
    }

    /// Which way the wind blows, in degrees around +Y (``0`` = toward +X,
    /// ``90`` = toward +Z), like water's ``wave_direction``.
    #[getter]
    fn wind_direction(&self) -> f32 {
        self.inner.wind_direction
    }

    /// Distance from the camera where fog starts.
    #[getter]
    fn fog_start(&self) -> f32 {
        self.inner.fog_start
    }

    /// Whether a sky dome with a sun is drawn (``background`` is then the
    /// horizon color).
    #[getter]
    fn sky(&self) -> bool {
        self.inner.sky
    }

    /// The sky's color straight up (linear RGB).
    #[getter]
    fn zenith_color(&self) -> (f32, f32, f32) {
        let [r, g, b] = self.inner.zenith_color;
        (r, g, b)
    }

    /// The sun disk's apparent diameter, degrees.
    #[getter]
    fn sun_size(&self) -> f32 {
        self.inner.sun_size
    }

    /// Brightness of the sun's halo.
    #[getter]
    fn sun_glow(&self) -> f32 {
        self.inner.sun_glow
    }

    /// Cloud cover, 0 (clear) to 1 (overcast).
    #[getter]
    fn clouds(&self) -> f32 {
        self.inner.clouds
    }

    /// Lens flare strength when looking at the sun (Beautiful graphics).
    #[getter]
    fn lens_flare(&self) -> f32 {
        self.inner.lens_flare
    }

    /// Ambient occlusion strength: how much creases and the ground under
    /// objects darken (Beautiful graphics; ``0`` = off).
    #[getter]
    fn ambient_occlusion(&self) -> f32 {
        self.inner.ambient_occlusion
    }

    fn __repr__(&self) -> String {
        format!(
            "Environment(tonemapping={:?}, exposure={:.2}, fog={})",
            self.tonemapping(),
            self.inner.exposure,
            if self.inner.fog { "True" } else { "False" }
        )
    }
}

/// Post-effect fields given by keyword (None: keep).
struct PostArgs {
    bloom: Option<f32>,
    contrast: Option<f32>,
    saturation: Option<f32>,
    temperature: Option<f32>,
    tint: Option<f32>,
    lift: Option<(f32, f32, f32)>,
    gamma: Option<(f32, f32, f32)>,
    gain: Option<(f32, f32, f32)>,
    vignette: Option<f32>,
    grain: Option<f32>,
    chromatic_aberration: Option<f32>,
}

impl PostArgs {
    fn apply(self, base: PostEffects) -> PostEffects {
        let rgb = |c: Option<(f32, f32, f32)>, d: [f32; 3]| c.map_or(d, |c| [c.0, c.1, c.2]);
        PostEffects {
            bloom: self.bloom.unwrap_or(base.bloom),
            contrast: self.contrast.unwrap_or(base.contrast),
            saturation: self.saturation.unwrap_or(base.saturation),
            temperature: self.temperature.unwrap_or(base.temperature),
            tint: self.tint.unwrap_or(base.tint),
            lift: rgb(self.lift, base.lift),
            gamma: rgb(self.gamma, base.gamma),
            gain: rgb(self.gain, base.gain),
            vignette: self.vignette.unwrap_or(base.vignette),
            grain: self.grain.unwrap_or(base.grain),
            chromatic_aberration: self
                .chromatic_aberration
                .unwrap_or(base.chromatic_aberration),
        }
    }
}

/// The named post look, or `current` for None.
fn post_preset(name: Option<&str>, current: PostEffects) -> PyResult<PostEffects> {
    let Some(name) = name else {
        return Ok(current);
    };
    PostEffects::preset(name).ok_or_else(|| {
        pyo3::exceptions::PyValueError::new_err(format!(
            "unknown post look {name:?}; expected 'neutral', 'bleached', 'vivid', 'film', or 'noir'"
        ))
    })
}

impl PyEnvironment {
    /// The engine component value.
    pub fn inner(&self) -> Environment {
        self.inner
    }

    /// Wraps an engine environment for Python.
    pub fn from_inner(inner: Environment) -> Self {
        Self { inner }
    }
}

/// An animated water surface, centred on its entity's transform: Gerstner
/// waves, sky reflections and sun glints on every preset; refraction,
/// depth-based color and shoreline foam on ``"beautiful"`` graphics.
/// Under it: caustics, and a blue-green view when the camera dives in.
///
/// ```python
/// scene.add(at.Entity("lake").add(at.Transform(position=(0, -0.5, 0)))
///                            .add(at.Water.preset("pond")))
/// scene.add(at.Entity("sea").add(at.Transform()).add(at.Water(
///     size=(300, 300), wave_height=0.6, wave_length=18.0)))
/// ```
#[pyclass(name = "Water", module = "automata._core", frozen)]
#[derive(Debug, Clone, Copy)]
pub struct PyWater {
    inner: Water,
}

#[pymethods]
impl PyWater {
    /// Creates a water surface; unspecified settings use the defaults.
    #[new]
    #[pyo3(signature = (
        size=None,
        deep_color=None,
        shallow_color=None,
        clarity=None,
        wave_height=None,
        wave_length=None,
        wave_speed=None,
        wave_direction=None,
        foam=None,
        density=None,
        drag=None,
        endless=None,
        caustics=None,
        wind=None,
    ))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        size: Option<(f32, f32)>,
        deep_color: Option<(f32, f32, f32)>,
        shallow_color: Option<(f32, f32, f32)>,
        clarity: Option<f32>,
        wave_height: Option<f32>,
        wave_length: Option<f32>,
        wave_speed: Option<f32>,
        wave_direction: Option<f32>,
        foam: Option<f32>,
        density: Option<f32>,
        drag: Option<f32>,
        endless: Option<bool>,
        caustics: Option<f32>,
        wind: Option<f32>,
    ) -> Self {
        let d = Water::default();
        let rgb = |c: (f32, f32, f32)| [c.0, c.1, c.2];
        Self {
            inner: Water {
                size: size.map_or(d.size, |(w, l)| [w, l]),
                deep_color: deep_color.map_or(d.deep_color, rgb),
                shallow_color: shallow_color.map_or(d.shallow_color, rgb),
                clarity: clarity.unwrap_or(d.clarity),
                wave_height: wave_height.unwrap_or(d.wave_height),
                wave_length: wave_length.unwrap_or(d.wave_length),
                wave_speed: wave_speed.unwrap_or(d.wave_speed),
                wave_direction: wave_direction.unwrap_or(d.wave_direction),
                foam: foam.unwrap_or(d.foam),
                density: density.unwrap_or(d.density),
                drag: drag.unwrap_or(d.drag),
                endless: endless.unwrap_or(d.endless),
                caustics: caustics.unwrap_or(d.caustics),
                wind: wind.unwrap_or(d.wind),
            },
        }
    }

    /// A named surface: ``"pond"`` (calm, clear) or ``"ocean"`` (big swell).
    #[staticmethod]
    fn preset(name: &str) -> PyResult<Self> {
        let inner = match name.to_ascii_lowercase().as_str() {
            "pond" | "lake" => Water::pond(),
            "ocean" | "sea" => Water::ocean(),
            "default" => Water::default(),
            other => {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "unknown water preset {other:?}; expected 'pond' or 'ocean'"
                )))
            }
        };
        Ok(Self { inner })
    }

    /// Width (X) and length (Z) in world units.
    #[getter]
    fn size(&self) -> (f32, f32) {
        (self.inner.size[0], self.inner.size[1])
    }

    /// Color of deep water (linear RGB).
    #[getter]
    fn deep_color(&self) -> (f32, f32, f32) {
        let [r, g, b] = self.inner.deep_color;
        (r, g, b)
    }

    /// Tint of shallow water and wave crests (linear RGB).
    #[getter]
    fn shallow_color(&self) -> (f32, f32, f32) {
        let [r, g, b] = self.inner.shallow_color;
        (r, g, b)
    }

    /// How deep you can see into the water (world units).
    #[getter]
    fn clarity(&self) -> f32 {
        self.inner.clarity
    }

    /// Height of the main waves (world units).
    #[getter]
    fn wave_height(&self) -> f32 {
        self.inner.wave_height
    }

    /// Wavelength of the main swell (world units).
    #[getter]
    fn wave_length(&self) -> f32 {
        self.inner.wave_length
    }

    /// Wave speed multiplier.
    #[getter]
    fn wave_speed(&self) -> f32 {
        self.inner.wave_speed
    }

    /// Direction the waves travel, in degrees around +Y.
    #[getter]
    fn wave_direction(&self) -> f32 {
        self.inner.wave_direction
    }

    /// Shoreline foam amount (0..1).
    #[getter]
    fn foam(&self) -> f32 {
        self.inner.foam
    }

    /// Buoyancy: the water's density, against colliders' ``density``
    /// (default ``2.0``; colliders default to ``1.0``, so they float half
    /// under). Lighter than this floats, with that fraction under the
    /// surface; denser sinks, slowed by the water.
    #[getter]
    fn density(&self) -> f32 {
        self.inner.density
    }

    /// How much the water slows what moves and spins in it.
    #[getter]
    fn drag(&self) -> f32 {
        self.inner.drag
    }

    /// Whether the surface carries on to the horizon past ``size``.
    #[getter]
    fn endless(&self) -> bool {
        self.inner.endless
    }

    /// How strongly sunlight through the waves dances on what's under the
    /// water (caustics), 0..2 (default 1; Medium graphics and up).
    #[getter]
    fn caustics(&self) -> f32 {
        self.inner.caustics
    }

    /// Wind over the water, 0..1 (default 0): from 0.5 up it blows spray
    /// off the wave crests.
    #[getter]
    fn wind(&self) -> f32 {
        self.inner.wind
    }

    fn __repr__(&self) -> String {
        format!(
            "Water(size=({:.1}, {:.1}), wave_height={:.2})",
            self.inner.size[0], self.inner.size[1], self.inner.wave_height
        )
    }
}

impl PyWater {
    /// The engine component value.
    pub fn inner(&self) -> Water {
        self.inner
    }

    /// Wraps an engine water surface for Python.
    pub fn from_inner(inner: Water) -> Self {
        Self { inner }
    }
}

/// An orthographic 2D camera. The first entity carrying one becomes the
/// frame's 2D camera.
#[pyclass(name = "Camera2D", module = "automata._core", frozen)]
#[derive(Debug, Clone, Copy)]
pub struct PyCamera2D {
    inner: Camera2D,
}

#[pymethods]
impl PyCamera2D {
    /// Creates a 2D camera centered at `center` whose vertical view spans
    /// `2 * half_height` world units.
    #[new]
    #[pyo3(signature = (center=(0.0, 0.0), half_height=100.0))]
    fn new(center: (f32, f32), half_height: f32) -> Self {
        Self {
            inner: Camera2D::new(Vec2::new(center.0, center.1), half_height),
        }
    }

    /// The center of the view in world units.
    #[getter]
    fn center(&self) -> (f32, f32) {
        (self.inner.center.x, self.inner.center.y)
    }

    /// Half of the vertical view extent, in world units.
    #[getter]
    fn half_height(&self) -> f32 {
        self.inner.half_height
    }
}

impl PyCamera2D {
    /// The engine component value.
    pub fn inner(&self) -> Camera2D {
        self.inner
    }

    /// Wraps an engine camera for Python (used by scene enumeration).
    pub fn from_inner(inner: Camera2D) -> Self {
        Self { inner }
    }
}

/// Rejects negative or non-finite draw distances.
fn check_draw_distance(value: f32) -> PyResult<()> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(pyo3::exceptions::PyValueError::new_err(format!(
            "draw_distance must be 0 (any distance) or a positive number of metres, not {value}"
        )))
    }
}

/// A mesh rendered in 3D: a built-in primitive with a flat color, or a
/// model file (`MeshRenderer.model("models/tree.glb")`).
///
/// This is a plain-data visual: no GPU handles cross into Python. The
/// engine's windowed loop materializes it on the first frame; in headless
/// mode it is inert. The entity's `Transform` scale sizes it.
#[pyclass(name = "MeshRenderer", module = "automata._core", frozen)]
#[derive(Debug, Clone)]
pub struct PyMeshRenderer {
    inner: automata_scene::MeshVisual,
    kind: &'static str,
}

#[pymethods]
impl PyMeshRenderer {
    /// A cube centered on the entity, `size` units on a side before scaling.
    ///
    /// The keyword-only surface arguments are shared by every constructor:
    /// `roughness` (0 = mirror-smooth, 1 = chalky; default 0.6),
    /// `metallic` (0 = plastic/wood/stone, 1 = bare metal; default 0),
    /// `emissive` (RGB light the surface gives off),
    /// `emissive_strength` (its multiplier; above 1 it glows and blooms) and
    /// `glow_light` (how much the glow lights nearby surfaces: 1 like a
    /// real glowing object of that size, 0 not at all) and `glow_shadows`
    /// (whether objects block that light; on High and up).
    ///
    /// `material` names a material file (`"materials/brick.mat.ron"`,
    /// made in Oberon or by hand) to draw with instead of `color` and the
    /// surface arguments: its colour, surface and texture maps, shared with
    /// every entity that uses the same file. Relative paths resolve like
    /// model paths. Raises `FileNotFoundError` when the file is missing
    /// and `ValueError` when it isn't a material.
    ///
    /// `draw_distance` (metres; 0 = any distance) hides it, shadow included,
    /// when the camera is farther away, dithering it out over the last
    /// tenth. Model files' `_LOD1`, `_LOD2`… levels apply either way.
    #[classmethod]
    #[pyo3(signature = (size=1.0, color=(1.0, 1.0, 1.0, 1.0), *, roughness=None, metallic=None,
                        emissive=(0.0, 0.0, 0.0), emissive_strength=1.0, glow_light=1.0,
                        glow_shadows=true, draw_distance=0.0, material=None))]
    #[allow(clippy::too_many_arguments)] // Python keyword arguments
    fn cube(
        cls: &Bound<'_, pyo3::types::PyType>,
        size: f32,
        color: (f32, f32, f32, f32),
        roughness: Option<f32>,
        metallic: Option<f32>,
        emissive: (f32, f32, f32),
        emissive_strength: f32,
        glow_light: f32,
        glow_shadows: bool,
        draw_distance: f32,
        material: Option<&str>,
    ) -> PyResult<Self> {
        check_draw_distance(draw_distance)?;
        let surface = surface_from(
            roughness,
            metallic,
            emissive,
            emissive_strength,
            glow_light,
            glow_shadows,
        )?;
        Ok(Self {
            inner: automata_scene::MeshVisual::cube(size, color.into())
                .with_surface(surface)
                .with_draw_distance(draw_distance)
                .with_material(&material_from(cls.py(), material)?),
            kind: "cube",
        })
    }

    /// A smooth sphere centered on the entity, `size` units across before
    /// scaling. Surface arguments as for `cube`.
    #[classmethod]
    #[pyo3(signature = (size=1.0, color=(1.0, 1.0, 1.0, 1.0), *, roughness=None, metallic=None,
                        emissive=(0.0, 0.0, 0.0), emissive_strength=1.0, glow_light=1.0,
                        glow_shadows=true, draw_distance=0.0, material=None))]
    #[allow(clippy::too_many_arguments)] // Python keyword arguments
    fn sphere(
        cls: &Bound<'_, pyo3::types::PyType>,
        size: f32,
        color: (f32, f32, f32, f32),
        roughness: Option<f32>,
        metallic: Option<f32>,
        emissive: (f32, f32, f32),
        emissive_strength: f32,
        glow_light: f32,
        glow_shadows: bool,
        draw_distance: f32,
        material: Option<&str>,
    ) -> PyResult<Self> {
        check_draw_distance(draw_distance)?;
        let surface = surface_from(
            roughness,
            metallic,
            emissive,
            emissive_strength,
            glow_light,
            glow_shadows,
        )?;
        Ok(Self {
            inner: automata_scene::MeshVisual::sphere(size, color.into())
                .with_surface(surface)
                .with_draw_distance(draw_distance)
                .with_material(&material_from(cls.py(), material)?),
            kind: "sphere",
        })
    }

    /// A quad centered on the entity, facing +Z, `size` units on a side
    /// before scaling. Surface arguments as for `cube`.
    #[classmethod]
    #[pyo3(signature = (size=1.0, color=(1.0, 1.0, 1.0, 1.0), *, roughness=None, metallic=None,
                        emissive=(0.0, 0.0, 0.0), emissive_strength=1.0, glow_light=1.0,
                        glow_shadows=true, draw_distance=0.0, material=None))]
    #[allow(clippy::too_many_arguments)] // Python keyword arguments
    fn quad(
        cls: &Bound<'_, pyo3::types::PyType>,
        size: f32,
        color: (f32, f32, f32, f32),
        roughness: Option<f32>,
        metallic: Option<f32>,
        emissive: (f32, f32, f32),
        emissive_strength: f32,
        glow_light: f32,
        glow_shadows: bool,
        draw_distance: f32,
        material: Option<&str>,
    ) -> PyResult<Self> {
        check_draw_distance(draw_distance)?;
        let surface = surface_from(
            roughness,
            metallic,
            emissive,
            emissive_strength,
            glow_light,
            glow_shadows,
        )?;
        Ok(Self {
            inner: automata_scene::MeshVisual::quad(size, color.into())
                .with_surface(surface)
                .with_draw_distance(draw_distance)
                .with_material(&material_from(cls.py(), material)?),
            kind: "quad",
        })
    }

    /// A model file: glTF 2.0 (`.glb`, `.gltf`) or Wavefront `.obj`. Every
    /// part keeps its own colors and textures; `color` tints them all
    /// (white keeps the file's look).
    ///
    /// Relative paths resolve against the asset root: the open scene's
    /// folder (set by `Scene.load`), else the folder of the script you ran,
    /// else the working directory. See `set_asset_root`.
    ///
    /// Surface arguments as for `cube`, except that unset `roughness` and
    /// `metallic` keep each part's values from the file, and `emissive`
    /// adds to the file's own glow.
    ///
    /// Raises `FileNotFoundError` when the file is missing and `ValueError`
    /// when it isn't a readable model.
    #[classmethod]
    #[pyo3(signature = (path, color=(1.0, 1.0, 1.0, 1.0), *, roughness=None, metallic=None,
                        emissive=(0.0, 0.0, 0.0), emissive_strength=1.0, glow_light=1.0,
                        glow_shadows=true, draw_distance=0.0, material=None))]
    #[allow(clippy::too_many_arguments)] // Python keyword arguments
    fn model(
        cls: &Bound<'_, pyo3::types::PyType>,
        path: &str,
        color: (f32, f32, f32, f32),
        roughness: Option<f32>,
        metallic: Option<f32>,
        emissive: (f32, f32, f32),
        emissive_strength: f32,
        glow_light: f32,
        glow_shadows: bool,
        draw_distance: f32,
        material: Option<&str>,
    ) -> PyResult<Self> {
        check_draw_distance(draw_distance)?;
        let surface = surface_from(
            roughness,
            metallic,
            emissive,
            emissive_strength,
            glow_light,
            glow_shadows,
        )?;
        default_asset_root(cls.py());
        let inner =
            automata_scene::MeshVisual::try_model(path, color.into()).map_err(|e| match e {
                automata_assets::ModelError::NotFound(_) => {
                    pyo3::exceptions::PyFileNotFoundError::new_err(e.to_string())
                }
                _ => pyo3::exceptions::PyValueError::new_err(e.to_string()),
            })?;
        Ok(Self {
            inner: inner
                .with_surface(surface)
                .with_draw_distance(draw_distance)
                .with_material(&material_from(cls.py(), material)?),
            kind: "model",
        })
    }

    /// The kind: "cube", "sphere", "quad", "custom" or "model".
    #[getter]
    fn kind(&self) -> &'static str {
        self.kind
    }

    /// The flat RGBA color (for models, the tint).
    #[getter]
    fn color(&self) -> (f32, f32, f32, f32) {
        self.inner.color.into()
    }

    /// The roughness override (0..1), or `None` for the default / the
    /// model's own.
    #[getter]
    fn roughness(&self) -> Option<f32> {
        self.inner.surface.roughness
    }

    /// The metalness override (0..1), or `None` for the default / the
    /// model's own.
    #[getter]
    fn metallic(&self) -> Option<f32> {
        self.inner.surface.metallic
    }

    /// The emitted RGB color (before `emissive_strength`).
    #[getter]
    fn emissive(&self) -> (f32, f32, f32) {
        self.inner.surface.emissive.into()
    }

    /// The multiplier on `emissive`.
    #[getter]
    fn emissive_strength(&self) -> f32 {
        self.inner.surface.emissive_strength
    }

    /// How much the glow lights nearby surfaces (1 = like a real glowing
    /// object of this size).
    #[getter]
    fn glow_light(&self) -> f32 {
        self.inner.surface.glow_light
    }

    /// Whether objects block the glow's light, casting shadows (on High
    /// and up).
    #[getter]
    fn glow_shadows(&self) -> bool {
        self.inner.surface.glow_shadows
    }

    /// Beyond this camera distance (metres) it isn't drawn; 0 = any
    /// distance.
    #[getter]
    fn draw_distance(&self) -> f32 {
        self.inner.draw_distance
    }

    /// The material file it draws with, or `None`.
    #[getter]
    fn material(&self) -> Option<String> {
        (!self.inner.material.is_empty()).then(|| self.inner.material.clone())
    }

    /// The model file, for `MeshRenderer.model`; otherwise `None`.
    #[getter]
    fn path(&self) -> Option<String> {
        match &self.inner.kind {
            automata_scene::MeshVisualKind::Model { path } => Some(path.clone()),
            _ => None,
        }
    }

    /// The model's bounding box as `((min_x, min_y, min_z), (max_x, max_y,
    /// max_z))` in model units, or `None` for primitives and models that
    /// failed to load.
    #[getter]
    #[allow(clippy::type_complexity)]
    fn bounds(&self) -> Option<((f32, f32, f32), (f32, f32, f32))> {
        self.inner
            .model
            .as_ref()
            .map(|m| (m.min.into(), m.max.into()))
    }

    fn __repr__(&self) -> String {
        match &self.inner.kind {
            automata_scene::MeshVisualKind::Model { path } => {
                format!("MeshRenderer.model({path:?}, color={:?})", self.inner.color)
            }
            _ => format!("MeshRenderer.{}(color={:?})", self.kind, self.inner.color),
        }
    }
}

impl PyMeshRenderer {
    /// The engine component value.
    pub(crate) fn inner(&self) -> automata_scene::MeshVisual {
        self.inner.clone()
    }

    /// Wraps an engine mesh visual for Python, deriving the kind label from
    /// the visual's shape (used by scene enumeration).
    pub fn from_inner(inner: automata_scene::MeshVisual) -> Self {
        let kind = match inner.kind {
            automata_scene::MeshVisualKind::Cube { .. } => "cube",
            automata_scene::MeshVisualKind::Quad { .. } => "quad",
            automata_scene::MeshVisualKind::Sphere { .. } => "sphere",
            automata_scene::MeshVisualKind::Custom => "custom",
            automata_scene::MeshVisualKind::Model { .. } => "model",
        };
        Self { inner, kind }
    }
}

/// Checks a `material=` argument: a material file that loads (relative
/// paths against the asset root). `None` is no material.
fn material_from(py: Python<'_>, material: Option<&str>) -> PyResult<String> {
    let Some(path) = material.map(str::trim).filter(|m| !m.is_empty()) else {
        return Ok(String::new());
    };
    if !automata_assets::is_material_path(std::path::Path::new(path)) {
        return Err(pyo3::exceptions::PyValueError::new_err(format!(
            "material={path:?}: a material is a .mat.ron file, like \"materials/brick.mat.ron\""
        )));
    }
    default_asset_root(py);
    match automata_assets::load_material(path) {
        Ok(_) => Ok(path.replace('\\', "/")),
        Err(e @ automata_assets::MaterialError::NotFound(_)) => Err(
            pyo3::exceptions::PyFileNotFoundError::new_err(e.to_string()),
        ),
        Err(e) => Err(pyo3::exceptions::PyValueError::new_err(e.to_string())),
    }
}

/// Validates the keyword-only surface arguments of `MeshRenderer`.
fn surface_from(
    roughness: Option<f32>,
    metallic: Option<f32>,
    emissive: (f32, f32, f32),
    emissive_strength: f32,
    glow_light: f32,
    glow_shadows: bool,
) -> PyResult<automata_scene::Surface> {
    for (name, v) in [("roughness", roughness), ("metallic", metallic)] {
        if let Some(v) = v {
            if !(0.0..=1.0).contains(&v) {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "{name} must be between 0 and 1, got {v}"
                )));
            }
        }
    }
    let emissive: [f32; 3] = emissive.into();
    if emissive.iter().any(|c| *c < 0.0) || emissive_strength < 0.0 || glow_light < 0.0 {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "emissive, emissive_strength and glow_light can't be negative",
        ));
    }
    Ok(automata_scene::Surface {
        roughness,
        metallic,
        emissive,
        emissive_strength,
        glow_light,
        glow_shadows,
    })
}

/// When no asset root is set, uses the folder of the script being run
/// (`__main__.__file__`), so `python game.py` finds `models/` beside it.
pub(crate) fn default_asset_root(py: Python<'_>) {
    if automata_assets::root().is_some() {
        return;
    }
    let main_file = py
        .import("sys")
        .and_then(|sys| sys.getattr("modules"))
        .and_then(|m| m.get_item("__main__"))
        .and_then(|main| main.getattr("__file__"))
        .and_then(|f| f.extract::<String>());
    if let Ok(file) = main_file {
        if let Some(dir) = std::path::Path::new(&file).parent() {
            if !dir.as_os_str().is_empty() {
                automata_assets::set_root(Some(dir));
            }
        }
    }
}

/// Sets the folder relative model paths resolve against. `None` clears it
/// (the working directory is used).
#[pyfunction]
#[pyo3(signature = (path))]
pub fn set_asset_root(path: Option<&str>) {
    automata_assets::set_root(path.map(std::path::Path::new));
}

/// The folder relative model paths resolve against, or `None`.
#[pyfunction]
pub fn asset_root() -> Option<String> {
    automata_assets::root().map(|p| p.to_string_lossy().into_owned())
}

/// A component staged on a Python-side entity, ready to be inserted into
/// the ECS world.
#[derive(Debug, Clone)]
pub enum StagedComponent {
    /// Local transform.
    Transform(Transform),
    /// Perspective camera.
    Camera3D(Camera3D),
    /// Orthographic camera.
    Camera2D(Camera2D),
    /// Physics rigid body.
    RigidBody(automata_physics::RigidBody),
    /// 3D physics collider.
    Collider3D(automata_physics::Collider3D),
    /// 2D physics collider.
    Collider2D(automata_physics::Collider2D),
    /// Character controller.
    CharacterController(automata_physics::CharacterController),
    /// Character velocity.
    CharacterVelocity(automata_physics::CharacterVelocity),
    /// Primitive mesh visual.
    MeshVisual(automata_scene::MeshVisual),
    /// Directional sun light.
    DirectionalLight(DirectionalLight),
    /// Placed light shining every way.
    PointLight(PointLight),
    /// Placed light shining in a cone.
    SpotLight(SpotLight),
    /// Scene look (ambient, fog, tone mapping).
    Environment(Environment),
    /// Animated water surface.
    Water(Water),
    /// A sound the entity plays.
    AudioSource(automata_core::AudioSource),
    /// Makes particles.
    ParticleEmitter(automata_scene::ParticleEmitter),
    /// Which clip an animated model plays.
    Animation(automata_scene::Animation),
    /// A field of grass.
    Grass(automata_scene::Grass),
    /// Draws the entity's model at many spots on the ground.
    Scatter(automata_scene::Scatter),
    /// Holds the entity's body to another's (or the world).
    Joint(automata_physics::Joint),
    /// Follows a bone of another entity (the transform is its offset).
    Attach(automata_core::scene_file::AttachData),
    /// Finds its own way around the navigation mesh.
    NavAgent(automata_nav::NavAgent),
    /// The scene's navigation mesh settings.
    Navigation(automata_nav::Navigation),
    /// A hand-placed shortcut for nav agents.
    NavLink(automata_nav::NavLink),
}

/// Extracts a staged component from an arbitrary Python object, raising a
/// `TypeError` listing the supported types when it matches none.
pub fn extract_component(obj: &Bound<'_, PyAny>) -> PyResult<StagedComponent> {
    if let Ok(t) = obj.cast::<PyTransform>() {
        return Ok(StagedComponent::Transform(t.borrow().inner()));
    }
    if let Ok(c) = obj.cast::<PyCamera3D>() {
        return Ok(StagedComponent::Camera3D(c.get().inner()));
    }
    if let Ok(c) = obj.cast::<PyCamera2D>() {
        return Ok(StagedComponent::Camera2D(c.get().inner()));
    }
    if let Ok(c) = obj.cast::<crate::physics_components::PyRigidBody>() {
        return Ok(StagedComponent::RigidBody(c.get().inner()));
    }
    if let Ok(c) = obj.cast::<crate::physics_components::PyCollider3D>() {
        return Ok(StagedComponent::Collider3D(c.get().inner()));
    }
    if let Ok(c) = obj.cast::<crate::physics_components::PyCollider2D>() {
        return Ok(StagedComponent::Collider2D(c.get().inner()));
    }
    if let Ok(c) = obj.cast::<crate::physics_components::PyCharacterController>() {
        return Ok(StagedComponent::CharacterController(c.get().inner()));
    }
    if let Ok(c) = obj.cast::<crate::physics_components::PyCharacterVelocity>() {
        return Ok(StagedComponent::CharacterVelocity(c.borrow().inner()));
    }
    if let Ok(m) = obj.cast::<PyMeshRenderer>() {
        return Ok(StagedComponent::MeshVisual(m.get().inner()));
    }
    if let Ok(l) = obj.cast::<PyDirectionalLight>() {
        return Ok(StagedComponent::DirectionalLight(l.get().inner()));
    }
    if let Ok(l) = obj.cast::<PyPointLight>() {
        return Ok(StagedComponent::PointLight(l.get().inner()));
    }
    if let Ok(l) = obj.cast::<PySpotLight>() {
        return Ok(StagedComponent::SpotLight(l.get().inner()));
    }
    if let Ok(e) = obj.cast::<PyEnvironment>() {
        return Ok(StagedComponent::Environment(e.get().inner()));
    }
    if let Ok(w) = obj.cast::<PyWater>() {
        return Ok(StagedComponent::Water(w.get().inner()));
    }
    if let Ok(a) = obj.cast::<PyAudioSource>() {
        return Ok(StagedComponent::AudioSource(a.get().inner.clone()));
    }
    if let Ok(e) = obj.cast::<crate::particles::PyParticleEmitter>() {
        return Ok(StagedComponent::ParticleEmitter(e.get().inner()));
    }
    if let Ok(a) = obj.cast::<crate::animation::PyAnimation>() {
        return Ok(StagedComponent::Animation(a.get().inner()));
    }
    if let Ok(g) = obj.cast::<crate::foliage::PyGrass>() {
        return Ok(StagedComponent::Grass(g.get().inner()));
    }
    if let Ok(s) = obj.cast::<crate::foliage::PyScatter>() {
        return Ok(StagedComponent::Scatter(s.get().inner()));
    }
    if let Ok(j) = obj.cast::<crate::joints::PyJoint>() {
        return Ok(StagedComponent::Joint(j.get().inner()));
    }
    if let Ok(a) = obj.cast::<crate::animation::PyAttach>() {
        return Ok(StagedComponent::Attach(a.get().inner.clone()));
    }
    if let Ok(a) = obj.cast::<crate::nav::PyNavAgent>() {
        return Ok(StagedComponent::NavAgent(a.get().inner.clone()));
    }
    if let Ok(n) = obj.cast::<crate::nav::PyNavigation>() {
        return Ok(StagedComponent::Navigation(n.get().inner.clone()));
    }
    if let Ok(l) = obj.cast::<crate::nav::PyNavLink>() {
        return Ok(StagedComponent::NavLink(l.get().inner.clone()));
    }
    let typename = obj.get_type().name()?.to_string();
    Err(PyTypeError::new_err(format!(
        "unsupported component type {typename}; expected Transform, Camera3D, Camera2D, \
         RigidBody, Collider3D, Collider2D, CharacterController, CharacterVelocity, \
         MeshRenderer, DirectionalLight, PointLight, SpotLight, Environment, Water, \
         AudioSource, ParticleEmitter, Animation, Grass, Scatter, Joint, Attach, \
         NavAgent, Navigation, or NavLink"
    )))
}

/// A sound attached to an entity: what Oberon's Inspector calls an Audio
/// Source. `clip` is a sound file (`.ogg`, `.wav`, `.mp3`, `.flac`),
/// relative to the scene's folder. Behaviour scripts start it with
/// `entity.audio.play()`; `play_on_start` starts it with the scene.
/// `spatial` sources fade out with distance from the camera (silent at
/// `range` metres) and pan left/right.
///
/// ```python
/// at.Entity("Radio").add(at.Transform()).add(
///     at.AudioSource("sounds/music.ogg", looping=True, play_on_start=True))
/// ```
#[pyclass(name = "AudioSource", module = "automata._core", frozen)]
#[derive(Debug, Clone)]
pub struct PyAudioSource {
    inner: automata_core::AudioSource,
}

#[pymethods]
impl PyAudioSource {
    /// Creates an audio source.
    #[new]
    #[pyo3(signature = (clip, volume=1.0, pitch=1.0, looping=false, play_on_start=false, spatial=false, range=20.0))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        clip: &str,
        volume: f32,
        pitch: f32,
        looping: bool,
        play_on_start: bool,
        spatial: bool,
        range: f32,
    ) -> PyResult<Self> {
        if !(volume.is_finite() && volume >= 0.0) {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "volume must be a non-negative number",
            ));
        }
        if !(pitch.is_finite() && pitch > 0.0) {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "pitch must be a positive number",
            ));
        }
        Ok(Self {
            inner: automata_core::AudioSource {
                clip: clip.replace('\\', "/"),
                volume,
                pitch,
                looping,
                play_on_start,
                spatial,
                range: range.max(0.1),
            },
        })
    }

    /// The sound file.
    #[getter]
    fn clip(&self) -> String {
        self.inner.clip.clone()
    }

    /// Linear volume (1.0 = as recorded).
    #[getter]
    fn volume(&self) -> f32 {
        self.inner.volume
    }

    /// Speed and pitch multiplier.
    #[getter]
    fn pitch(&self) -> f32 {
        self.inner.pitch
    }

    /// Whether it starts over at the end.
    #[getter]
    fn looping(&self) -> bool {
        self.inner.looping
    }

    /// Whether it starts with the scene.
    #[getter]
    fn play_on_start(&self) -> bool {
        self.inner.play_on_start
    }

    /// Whether it fades with distance from the camera.
    #[getter]
    fn spatial(&self) -> bool {
        self.inner.spatial
    }

    /// The distance at which a spatial source falls silent.
    #[getter]
    fn range(&self) -> f32 {
        self.inner.range
    }

    fn __repr__(&self) -> String {
        format!(
            "AudioSource({:?}, volume={:.2}, looping={}, play_on_start={}, spatial={})",
            self.inner.clip,
            self.inner.volume,
            if self.inner.looping { "True" } else { "False" },
            if self.inner.play_on_start {
                "True"
            } else {
                "False"
            },
            if self.inner.spatial { "True" } else { "False" },
        )
    }
}

impl PyAudioSource {
    /// Wraps an engine component.
    #[must_use]
    pub fn from_inner(inner: automata_core::AudioSource) -> Self {
        Self { inner }
    }

    /// The engine component.
    #[must_use]
    pub fn inner(&self) -> automata_core::AudioSource {
        self.inner.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_defaults_to_identity() {
        let t = PyTransform::new((0.0, 0.0, 0.0), (0.0, 0.0, 0.0), (1.0, 1.0, 1.0));
        assert_eq!(t.inner(), Transform::IDENTITY);
    }

    #[test]
    fn transform_euler_round_trips() {
        let t = PyTransform::new(
            (1.0, 2.0, 3.0),
            (0.0, std::f32::consts::PI, 0.0),
            (2.0, 2.0, 2.0),
        );
        assert_eq!(t.position(), (1.0, 2.0, 3.0));
        // The euler round-trip may decompose π-rotations differently across
        // glam versions; verify the quaternion itself instead.
        let (rx, ry, rz) = t.rotation_euler();
        let q = glam::Quat::from_euler(glam::EulerRot::XYZ, rx, ry, rz);
        let orig = glam::Quat::from_rotation_y(std::f32::consts::PI);
        assert!(q.abs_diff_eq(orig, 1e-4) || q.abs_diff_eq(-orig, 1e-4));
        assert_eq!(t.scale(), (2.0, 2.0, 2.0));
    }

    #[test]
    fn camera3d_holds_its_parameters() {
        let c = PyCamera3D::new((0.0, 1.0, 4.0), (0.0, 0.0, 0.0), 75.0);
        assert_eq!(c.position(), (0.0, 1.0, 4.0));
        assert_eq!(c.target(), (0.0, 0.0, 0.0));
        assert_eq!(c.fov(), 75.0);
    }

    #[test]
    fn camera2d_defaults() {
        let c = PyCamera2D::new((5.0, 6.0), 50.0);
        assert_eq!(c.center(), (5.0, 6.0));
        assert_eq!(c.half_height(), 50.0);
    }
}
