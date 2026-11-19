//! Python-facing component classes: plain-data builders that are staged on
//! [`crate::scene::PyEntity`] and materialized into ECS components when the
//! scene loads.

use automata_renderer::{Camera2D, Camera3D};
use automata_scene::Transform;
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
    let typename = obj.get_type().name()?.to_string();
    Err(PyTypeError::new_err(format!(
        "unsupported component type {typename}; expected Transform, Camera3D, or Camera2D"
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
        let (_, ry, _) = t.rotation_euler();
        assert!((ry.abs() - std::f32::consts::PI).abs() < 1e-5);
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
