//! Python-facing component classes: plain-data builders that are staged on
//! [`crate::scene::PyEntity`] and materialized into ECS components when the
//! scene loads.

use automata_renderer::{Camera2D, Camera3D, Environment, Tonemapping, Water};
use automata_scene::{DirectionalLight, Transform};
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
    /// Creates a sun shining along `direction`.
    #[new]
    #[pyo3(signature = (direction=(-0.5, -1.0, -0.35), color=(1.0, 1.0, 1.0), ambient=0.3, shadows=true))]
    fn new(
        direction: (f32, f32, f32),
        color: (f32, f32, f32),
        ambient: f32,
        shadows: bool,
    ) -> Self {
        Self {
            inner: DirectionalLight {
                direction: vec3_of(direction),
                color: [color.0, color.1, color.2],
                ambient,
                shadows,
            },
        }
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
    ) -> PyResult<Self> {
        let rgb = |c: (f32, f32, f32)| [c.0, c.1, c.2];
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
            },
        })
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

    /// Distance from the camera where fog starts.
    #[getter]
    fn fog_start(&self) -> f32 {
        self.inner.fog_start
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
    #[classmethod]
    #[pyo3(signature = (size=1.0, color=(1.0, 1.0, 1.0, 1.0)))]
    fn cube(_cls: &Bound<'_, pyo3::types::PyType>, size: f32, color: (f32, f32, f32, f32)) -> Self {
        Self {
            inner: automata_scene::MeshVisual::cube(size, color.into()),
            kind: "cube",
        }
    }

    /// A quad centered on the entity, facing +Z, `size` units on a side
    /// before scaling.
    #[classmethod]
    #[pyo3(signature = (size=1.0, color=(1.0, 1.0, 1.0, 1.0)))]
    fn quad(_cls: &Bound<'_, pyo3::types::PyType>, size: f32, color: (f32, f32, f32, f32)) -> Self {
        Self {
            inner: automata_scene::MeshVisual::quad(size, color.into()),
            kind: "quad",
        }
    }

    /// A model file: glTF 2.0 (`.glb`, `.gltf`) or Wavefront `.obj`. Every
    /// part keeps its own colors and textures; `color` tints them all
    /// (white keeps the file's look).
    ///
    /// Relative paths resolve against the asset root: the open scene's
    /// folder (set by `Scene.load`), else the folder of the script you ran,
    /// else the working directory. See `set_asset_root`.
    ///
    /// Raises `FileNotFoundError` when the file is missing and `ValueError`
    /// when it isn't a readable model.
    #[classmethod]
    #[pyo3(signature = (path, color=(1.0, 1.0, 1.0, 1.0)))]
    fn model(
        cls: &Bound<'_, pyo3::types::PyType>,
        path: &str,
        color: (f32, f32, f32, f32),
    ) -> PyResult<Self> {
        default_asset_root(cls.py());
        let inner =
            automata_scene::MeshVisual::try_model(path, color.into()).map_err(|e| match e {
                automata_assets::ModelError::NotFound(_) => {
                    pyo3::exceptions::PyFileNotFoundError::new_err(e.to_string())
                }
                _ => pyo3::exceptions::PyValueError::new_err(e.to_string()),
            })?;
        Ok(Self {
            inner,
            kind: "model",
        })
    }

    /// The kind: "cube", "quad", "custom" or "model".
    #[getter]
    fn kind(&self) -> &'static str {
        self.kind
    }

    /// The flat RGBA color (for models, the tint).
    #[getter]
    fn color(&self) -> (f32, f32, f32, f32) {
        self.inner.color.into()
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
            automata_scene::MeshVisualKind::Custom => "custom",
            automata_scene::MeshVisualKind::Model { .. } => "model",
        };
        Self { inner, kind }
    }
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
    /// Scene look (ambient, fog, tone mapping).
    Environment(Environment),
    /// Animated water surface.
    Water(Water),
    /// A sound the entity plays.
    AudioSource(automata_core::AudioSource),
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
    if let Ok(e) = obj.cast::<PyEnvironment>() {
        return Ok(StagedComponent::Environment(e.get().inner()));
    }
    if let Ok(w) = obj.cast::<PyWater>() {
        return Ok(StagedComponent::Water(w.get().inner()));
    }
    if let Ok(a) = obj.cast::<PyAudioSource>() {
        return Ok(StagedComponent::AudioSource(a.get().inner.clone()));
    }
    let typename = obj.get_type().name()?.to_string();
    Err(PyTypeError::new_err(format!(
        "unsupported component type {typename}; expected Transform, Camera3D, Camera2D, \
         RigidBody, Collider3D, Collider2D, CharacterController, CharacterVelocity, \
         MeshRenderer, DirectionalLight, Environment, Water, or AudioSource"
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
