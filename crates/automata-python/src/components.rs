//! Python-facing component classes: plain-data builders that are staged on
//! [`crate::scene::PyEntity`] and materialized into ECS components when the
//! scene loads.

use automata_renderer::{Camera2D, Camera3D, Environment, Tonemapping};
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

/// A built-in primitive mesh with a flat color, rendered in 3D.
///
/// This is a plain-data visual: no GPU handles cross into Python. The
/// engine's windowed loop materializes it on the first frame; in headless
/// mode it is inert. The entity's `Transform` scale sizes the primitive.
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

    /// The primitive kind: "cube" or "quad".
    #[getter]
    fn kind(&self) -> &'static str {
        self.kind
    }

    /// The flat RGBA color.
    #[getter]
    fn color(&self) -> (f32, f32, f32, f32) {
        self.inner.color.into()
    }

    fn __repr__(&self) -> String {
        format!("MeshRenderer.{}(color={:?})", self.kind, self.inner.color)
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
        };
        Self { inner, kind }
    }
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
    let typename = obj.get_type().name()?.to_string();
    Err(PyTypeError::new_err(format!(
        "unsupported component type {typename}; expected Transform, Camera3D, Camera2D, \
         RigidBody, Collider3D, Collider2D, CharacterController, CharacterVelocity, \
         MeshRenderer, DirectionalLight, or Environment"
    )))
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
