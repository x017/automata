//! Python bindings for physics components: rigid bodies, colliders, and
//! character controllers.
//!
//! These are plain data holders. The engine reads them when the entity is
//! spawned and drives the simulation; game scripts never touch Rapier.

use automata_physics::{BodyType, CharacterMode};
use pyo3::prelude::*;
use pyo3::types::PyType;

use crate::components::PyTransform;

/// A rigid body: `RigidBody("dynamic" | "static" | "kinematic_velocity" | "kinematic_position")`.
///
/// ```python
/// ball = Entity("ball").add(RigidBody.dynamic()).add(Collider3D.ball(0.5))
/// ```
#[pyclass(name = "RigidBody", module = "automata._core", frozen)]
#[derive(Debug, Clone)]
pub struct PyRigidBody {
    inner: automata_physics::RigidBody,
}

impl PyRigidBody {
    pub(crate) fn inner(&self) -> automata_physics::RigidBody {
        self.inner.clone()
    }
}

#[pymethods]
impl PyRigidBody {
    #[new]
    #[pyo3(signature = (body_type = "dynamic", *, gravity_scale = 1.0, linear_damping = 0.0, angular_damping = 0.0, enabled = true))]
    fn new(
        body_type: &str,
        gravity_scale: f32,
        linear_damping: f32,
        angular_damping: f32,
        enabled: bool,
    ) -> PyResult<Self> {
        let body_type = parse_body_type(body_type)?;
        Ok(Self {
            inner: automata_physics::RigidBody {
                body_type,
                gravity_scale,
                linear_damping,
                angular_damping,
                enabled,
            },
        })
    }

    /// A free-moving body affected by gravity.
    #[classmethod]
    fn dynamic(_cls: &Bound<'_, PyType>) -> Self {
        Self {
            inner: automata_physics::RigidBody::dynamic(),
        }
    }

    /// An immovable body (floors, walls).
    #[classmethod]
    fn fixed(_cls: &Bound<'_, PyType>) -> Self {
        Self {
            inner: automata_physics::RigidBody::fixed(),
        }
    }

    /// A kinematic body driven by velocity (moving platforms).
    #[classmethod]
    fn kinematic_velocity(_cls: &Bound<'_, PyType>) -> Self {
        Self {
            inner: automata_physics::RigidBody::kinematic_velocity(),
        }
    }

    /// A kinematic body driven by its transform.
    #[classmethod]
    fn kinematic_position(_cls: &Bound<'_, PyType>) -> Self {
        Self {
            inner: automata_physics::RigidBody::kinematic_position(),
        }
    }

    #[getter]
    fn body_type(&self) -> &'static str {
        match self.inner.body_type {
            BodyType::Dynamic => "dynamic",
            BodyType::Static => "static",
            BodyType::KinematicVelocity => "kinematic_velocity",
            BodyType::KinematicPosition => "kinematic_position",
        }
    }

    fn __repr__(&self) -> String {
        format!("RigidBody({:?})", self.inner.body_type)
    }
}

fn parse_body_type(s: &str) -> PyResult<BodyType> {
    match s {
        "dynamic" => Ok(BodyType::Dynamic),
        "static" | "fixed" => Ok(BodyType::Static),
        "kinematic_velocity" | "kinematic" => Ok(BodyType::KinematicVelocity),
        "kinematic_position" => Ok(BodyType::KinematicPosition),
        other => Err(pyo3::exceptions::PyValueError::new_err(format!(
            "unknown body type {other:?}; expected dynamic, static, kinematic_velocity, or kinematic_position"
        ))),
    }
}

/// A 3D collider: `Collider3D.ball(r)`, `.cuboid(x, y, z)`, `.capsule(hh, r)`.
#[pyclass(name = "Collider3D", module = "automata._core", frozen)]
#[derive(Debug, Clone)]
pub struct PyCollider3D {
    inner: automata_physics::Collider3D,
}

impl PyCollider3D {
    pub(crate) fn inner(&self) -> automata_physics::Collider3D {
        self.inner.clone()
    }
}

#[pymethods]
impl PyCollider3D {
    /// A spherical collider of the given radius.
    #[classmethod]
    fn ball(_cls: &Bound<'_, PyType>, radius: f32) -> Self {
        Self {
            inner: automata_physics::Collider3D::ball(radius),
        }
    }

    /// A box collider with full extents (x, y, z).
    #[classmethod]
    fn cuboid(_cls: &Bound<'_, PyType>, x: f32, y: f32, z: f32) -> Self {
        Self {
            inner: automata_physics::Collider3D::cuboid(x, y, z),
        }
    }

    /// A capsule collider (Y-aligned) with the given half-height and radius.
    #[classmethod]
    fn capsule(_cls: &Bound<'_, PyType>, half_height: f32, radius: f32) -> Self {
        Self {
            inner: automata_physics::Collider3D::capsule(half_height, radius),
        }
    }

    /// A sensor detects overlaps without producing contact response.
    fn sensor(&self, sensor: bool) -> Self {
        Self {
            inner: self.inner.clone().sensor(sensor),
        }
    }

    /// Set the collider's density (kg/m³).
    fn density(&self, density: f32) -> Self {
        Self {
            inner: self.inner.clone().density(density),
        }
    }

    /// Set the collider's friction.
    fn friction(&self, friction: f32) -> Self {
        Self {
            inner: self.inner.clone().friction(friction),
        }
    }

    /// Set the collider's restitution (bounciness).
    fn restitution(&self, restitution: f32) -> Self {
        Self {
            inner: self.inner.clone().restitution(restitution),
        }
    }

    fn __repr__(&self) -> String {
        format!("Collider3D({:?})", self.inner.shape)
    }
}

/// A 2D collider: `Collider2D.ball(r)`, `.cuboid(x, y)`, `.capsule(hh, r)`.
#[pyclass(name = "Collider2D", module = "automata._core", frozen)]
#[derive(Debug, Clone)]
pub struct PyCollider2D {
    inner: automata_physics::Collider2D,
}

impl PyCollider2D {
    pub(crate) fn inner(&self) -> automata_physics::Collider2D {
        self.inner.clone()
    }
}

#[pymethods]
impl PyCollider2D {
    /// A circular collider of the given radius.
    #[classmethod]
    fn ball(_cls: &Bound<'_, PyType>, radius: f32) -> Self {
        Self {
            inner: automata_physics::Collider2D::ball(radius),
        }
    }

    /// A box collider with full extents (x, y).
    #[classmethod]
    fn cuboid(_cls: &Bound<'_, PyType>, x: f32, y: f32) -> Self {
        Self {
            inner: automata_physics::Collider2D::cuboid(x, y),
        }
    }

    /// A capsule collider (Y-aligned) with the given half-height and radius.
    #[classmethod]
    fn capsule(_cls: &Bound<'_, PyType>, half_height: f32, radius: f32) -> Self {
        Self {
            inner: automata_physics::Collider2D::capsule(half_height, radius),
        }
    }

    /// A sensor detects overlaps without producing contact response.
    fn sensor(&self, sensor: bool) -> Self {
        Self {
            inner: self.inner.clone().sensor(sensor),
        }
    }

    /// Set the collider's density.
    fn density(&self, density: f32) -> Self {
        Self {
            inner: self.inner.clone().density(density),
        }
    }

    /// Set the collider's friction.
    fn friction(&self, friction: f32) -> Self {
        Self {
            inner: self.inner.clone().friction(friction),
        }
    }

    /// Set the collider's restitution (bounciness).
    fn restitution(&self, restitution: f32) -> Self {
        Self {
            inner: self.inner.clone().restitution(restitution),
        }
    }

    fn __repr__(&self) -> String {
        format!("Collider2D({:?})", self.inner.shape)
    }
}

/// A kinematic character controller component.
///
/// ```python
/// player = (
///     Entity("player")
///     .add(Transform(position=(0, 1, 0)))
///     .add(RigidBody.kinematic_position())
///     .add(Collider3D.capsule(0.5, 0.4))
///     .add(CharacterController(snap_to_ground=0.4))
///     .add(CharacterVelocity((0, 0, 0)))
/// )
/// ```
#[pyclass(name = "CharacterController", module = "automata._core", frozen)]
#[derive(Debug, Clone)]
pub struct PyCharacterController {
    inner: automata_physics::CharacterController,
}

impl PyCharacterController {
    pub(crate) fn inner(&self) -> automata_physics::CharacterController {
        self.inner.clone()
    }
}

#[pymethods]
impl PyCharacterController {
    #[new]
    #[pyo3(signature = (*, mode = "normal", max_slope_climb_angle = std::f32::consts::FRAC_PI_4, min_slope_slide_angle = std::f32::consts::FRAC_PI_6, autostep = None, snap_to_ground = None, offset = 0.05, slide = true))]
    fn new(
        mode: &str,
        max_slope_climb_angle: f32,
        min_slope_slide_angle: f32,
        autostep: Option<f32>,
        snap_to_ground: Option<f32>,
        offset: f32,
        slide: bool,
    ) -> PyResult<Self> {
        let mode = match mode {
            "normal" => CharacterMode::Normal,
            "no_events" => CharacterMode::NoEvents,
            "ghost" => CharacterMode::Ghost,
            other => {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "unknown mode {other:?}; expected normal, no_events, or ghost"
                )))
            }
        };
        Ok(Self {
            inner: automata_physics::CharacterController {
                mode,
                max_slope_climb_angle,
                min_slope_slide_angle,
                autostep_height: autostep,
                snap_to_ground,
                offset,
                slide,
            },
        })
    }

    fn __repr__(&self) -> String {
        format!("CharacterController({:?})", self.inner.mode)
    }
}

/// Velocity for a character controller: `CharacterVelocity((vx, vy, vz))`.
///
/// Set a new value each frame (the engine does not clear it):
/// `player.get(CharacterVelocity).velocity = (2, 0, 0)`.
#[pyclass(name = "CharacterVelocity", module = "automata._core")]
#[derive(Debug, Clone)]
pub struct PyCharacterVelocity {
    inner: automata_physics::CharacterVelocity,
}

impl PyCharacterVelocity {
    pub(crate) fn inner(&self) -> automata_physics::CharacterVelocity {
        self.inner
    }
}

#[pymethods]
impl PyCharacterVelocity {
    #[new]
    #[pyo3(signature = (velocity = (0.0, 0.0, 0.0)))]
    fn new(velocity: (f32, f32, f32)) -> Self {
        Self {
            inner: automata_physics::CharacterVelocity::new(glam::Vec3::new(
                velocity.0, velocity.1, velocity.2,
            )),
        }
    }

    /// The current velocity tuple.
    #[getter]
    fn velocity(&self) -> (f32, f32, f32) {
        let v = self.inner.velocity;
        (v.x, v.y, v.z)
    }

    #[setter]
    fn set_velocity(&mut self, value: (f32, f32, f32)) {
        self.inner.velocity = glam::Vec3::new(value.0, value.1, value.2);
    }

    fn __repr__(&self) -> String {
        format!("CharacterVelocity({:?})", self.inner.velocity)
    }
}

// Keep PyTransform referenced so the module mirrors components.rs imports.
#[allow(dead_code)]
fn _assert_transform_used(_t: &PyTransform) {}
