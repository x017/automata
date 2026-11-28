//! Python bindings for physics joints.
//!
//! ```python
//! door = Entity("Door").add(RigidBody.dynamic()).add(Collider3D.cuboid(1, 2, 0.1))
//! door.add(Joint.hinge(axis=(0, 1, 0), connected="Frame", anchor=(-0.5, 0, 0),
//!                      limits=(0, 1.6)))
//! ```

use automata_physics::{Joint, JointKind, JointMotor};
use glam::Vec3;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyType};

/// A joint motor: `JointMotor(speed=2.0)` turns at a speed,
/// `JointMotor(target=1.2)` pulls to an angle (or slider position).
#[pyclass(name = "JointMotor", module = "automata._core", frozen)]
#[derive(Debug, Clone, Copy)]
pub struct PyJointMotor {
    inner: JointMotor,
}

#[pymethods]
impl PyJointMotor {
    #[new]
    #[pyo3(signature = (speed = 0.0, *, target = None, stiffness = 400.0, damping = 80.0, max_force = 0.0))]
    fn new(speed: f32, target: Option<f32>, stiffness: f32, damping: f32, max_force: f32) -> Self {
        Self {
            inner: JointMotor {
                speed,
                target,
                stiffness,
                damping,
                max_force,
            },
        }
    }

    /// Radians (or metres) per second it drives at.
    #[getter]
    fn speed(&self) -> f32 {
        self.inner.speed
    }

    /// The angle / position it pulls to, or ``None`` to just turn.
    #[getter]
    fn target(&self) -> Option<f32> {
        self.inner.target
    }

    /// How hard it pulls to its target.
    #[getter]
    fn stiffness(&self) -> f32 {
        self.inner.stiffness
    }

    /// How much it resists overshooting (and, without a target, how hard it
    /// holds its speed).
    #[getter]
    fn damping(&self) -> f32 {
        self.inner.damping
    }

    /// The most force it can use (``0`` = no limit).
    #[getter]
    fn max_force(&self) -> f32 {
        self.inner.max_force
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<Self>()
            .is_ok_and(|o| o.get().inner == self.inner)
    }

    fn __repr__(&self) -> String {
        match self.inner.target {
            Some(t) => format!("JointMotor(target={t})"),
            None => format!("JointMotor(speed={})", self.inner.speed),
        }
    }
}

/// A joint holding this entity's body to another entity's (named by
/// ``connected``) or to the world. See ``docs/physics.md``.
#[pyclass(name = "Joint", module = "automata._core", frozen)]
#[derive(Debug, Clone)]
pub struct PyJoint {
    inner: Joint,
}

impl PyJoint {
    pub(crate) fn inner(&self) -> Joint {
        self.inner.clone()
    }

    /// Wraps an engine joint for Python (used by scene enumeration).
    pub fn from_inner(inner: Joint) -> Self {
        Self { inner }
    }
}

fn vec3(t: (f32, f32, f32)) -> Vec3 {
    Vec3::new(t.0, t.1, t.2)
}

fn tuple(v: Vec3) -> (f32, f32, f32) {
    (v.x, v.y, v.z)
}

/// Applies `Joint(...)` / `Joint.hinge(...)` keyword arguments.
fn apply_options(joint: &mut Joint, options: Option<&Bound<'_, PyDict>>) -> PyResult<()> {
    let Some(options) = options else {
        return Ok(());
    };
    for (key, value) in options.iter() {
        let key: String = key.extract()?;
        match key.as_str() {
            "connected" => joint.connected = value.extract::<Option<String>>()?,
            "anchor" => joint.anchor = vec3(value.extract()?),
            "connected_anchor" => {
                joint.connected_anchor = value.extract::<Option<(f32, f32, f32)>>()?.map(vec3);
            }
            "axis" => joint.axis = vec3(value.extract()?),
            "limits" => {
                joint.limits = value
                    .extract::<Option<(f32, f32)>>()?
                    .map(|(a, b)| [a.min(b), a.max(b)]);
            }
            "length" => joint.length = value.extract()?,
            "stiffness" => joint.stiffness = value.extract()?,
            "damping" => joint.damping = value.extract()?,
            "motor" => {
                joint.motor = if value.is_none() {
                    None
                } else if let Ok(m) = value.cast::<PyJointMotor>() {
                    Some(m.get().inner)
                } else {
                    // A bare number is a speed.
                    Some(JointMotor::speed(value.extract().map_err(|_| {
                        PyTypeError::new_err("motor must be a JointMotor, a speed, or None")
                    })?))
                };
            }
            "collide_connected" => joint.collide_connected = value.extract()?,
            "break_force" => joint.break_force = value.extract()?,
            "enabled" => joint.enabled = value.extract()?,
            other => {
                return Err(PyTypeError::new_err(format!(
                    "Joint got an unexpected keyword argument {other:?}; expected connected, \
                     anchor, connected_anchor, axis, limits, length, stiffness, damping, \
                     motor, collide_connected, break_force or enabled"
                )))
            }
        }
    }
    Ok(())
}

fn with_options(mut joint: Joint, options: Option<&Bound<'_, PyDict>>) -> PyResult<PyJoint> {
    apply_options(&mut joint, options)?;
    Ok(PyJoint { inner: joint })
}

#[pymethods]
impl PyJoint {
    /// `Joint("hinge" | "ball" | "slider" | "fixed" | "rope" | "spring", **options)`.
    #[new]
    #[pyo3(signature = (kind = "hinge", **options))]
    fn new(kind: &str, options: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
        let kind = JointKind::from_key(kind).ok_or_else(|| {
            PyValueError::new_err(format!(
                "unknown joint kind {kind:?}; expected hinge, ball, slider, fixed, rope or spring"
            ))
        })?;
        with_options(Joint::new(kind), options)
    }

    /// Turns around `axis` (in the entity's space) like a door or a wheel.
    #[classmethod]
    #[pyo3(signature = (axis = (0.0, 1.0, 0.0), **options))]
    fn hinge(
        _cls: &Bound<'_, PyType>,
        axis: (f32, f32, f32),
        options: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
        with_options(Joint::hinge(vec3(axis)), options)
    }

    /// Swivels freely around a point, like a shoulder or a chain link.
    #[classmethod]
    #[pyo3(signature = (**options))]
    fn ball(_cls: &Bound<'_, PyType>, options: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
        with_options(Joint::ball(), options)
    }

    /// Slides along `axis` without turning, like a drawer or a lift.
    #[classmethod]
    #[pyo3(signature = (axis = (1.0, 0.0, 0.0), **options))]
    fn slider(
        _cls: &Bound<'_, PyType>,
        axis: (f32, f32, f32),
        options: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
        with_options(Joint::slider(vec3(axis)), options)
    }

    /// Welds the two bodies together where they are.
    #[classmethod]
    #[pyo3(signature = (**options))]
    fn fixed(_cls: &Bound<'_, PyType>, options: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
        with_options(Joint::fixed(), options)
    }

    /// Keeps the anchors at most `length` apart (``0`` = as far as they
    /// start); slack when closer.
    #[classmethod]
    #[pyo3(signature = (length = 0.0, **options))]
    fn rope(
        _cls: &Bound<'_, PyType>,
        length: f32,
        options: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
        with_options(Joint::rope(length), options)
    }

    /// Pulls the anchors towards `length` apart, bouncing.
    #[classmethod]
    #[pyo3(signature = (length = 0.0, stiffness = 30.0, damping = 2.0, **options))]
    fn spring(
        _cls: &Bound<'_, PyType>,
        length: f32,
        stiffness: f32,
        damping: f32,
        options: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
        with_options(Joint::spring(length, stiffness, damping), options)
    }

    /// A copy with some options changed: `joint.replace(enabled=False)`.
    #[pyo3(signature = (**options))]
    fn replace(&self, options: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
        with_options(self.inner.clone(), options)
    }

    /// ``"hinge"``, ``"ball"``, ``"slider"``, ``"fixed"``, ``"rope"`` or ``"spring"``.
    #[getter]
    fn kind(&self) -> &'static str {
        self.inner.kind.key()
    }

    /// The entity it joins to, by name (``None`` = the world).
    #[getter]
    fn connected(&self) -> Option<String> {
        self.inner.connected.clone()
    }

    /// Where it holds this entity, in the entity's space.
    #[getter]
    fn anchor(&self) -> (f32, f32, f32) {
        tuple(self.inner.anchor)
    }

    /// Where it holds the other side (in that entity's space, or the
    /// world's), or ``None`` to work it out from where things start.
    #[getter]
    fn connected_anchor(&self) -> Option<(f32, f32, f32)> {
        self.inner.connected_anchor.map(tuple)
    }

    /// The hinge / slide axis in the entity's space.
    #[getter]
    fn axis(&self) -> (f32, f32, f32) {
        tuple(self.inner.axis)
    }

    /// ``(min, max)`` angle (radians) or slide (metres) from the start.
    #[getter]
    fn limits(&self) -> Option<(f32, f32)> {
        self.inner.limits.map(|[a, b]| (a, b))
    }

    /// The rope / spring length (``0`` = the starting distance).
    #[getter]
    fn length(&self) -> f32 {
        self.inner.length
    }

    /// The spring's stiffness.
    #[getter]
    fn stiffness(&self) -> f32 {
        self.inner.stiffness
    }

    /// The spring's damping.
    #[getter]
    fn damping(&self) -> f32 {
        self.inner.damping
    }

    /// The hinge / slider motor, if any.
    #[getter]
    fn motor(&self) -> Option<PyJointMotor> {
        self.inner.motor.map(|inner| PyJointMotor { inner })
    }

    /// Whether the two joined bodies still bump into each other.
    #[getter]
    fn collide_connected(&self) -> bool {
        self.inner.collide_connected
    }

    /// Newtons that snap it (``0`` = unbreakable).
    #[getter]
    fn break_force(&self) -> f32 {
        self.inner.break_force
    }

    /// Whether it holds at all.
    #[getter]
    fn enabled(&self) -> bool {
        self.inner.enabled
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<Self>()
            .is_ok_and(|o| o.get().inner == self.inner)
    }

    fn __repr__(&self) -> String {
        match &self.inner.connected {
            Some(name) => format!("Joint.{}(connected={name:?})", self.inner.kind.key()),
            None => format!("Joint.{}()", self.inner.kind.key()),
        }
    }
}

/// `entity.joint` in a game's update: the entity's joint and how it's
/// holding. Setting a field changes the joint after the callback returns.
#[pyclass(name = "JointRef", module = "automata._core", unsendable)]
#[derive(Debug, Clone)]
pub struct PyJointRef {
    id: u64,
    joint: Joint,
    state: automata_physics::JointState,
    writes: crate::scene::WriteQueue,
}

impl PyJointRef {
    pub(crate) fn new(
        id: u64,
        joint: Joint,
        state: automata_physics::JointState,
        writes: crate::scene::WriteQueue,
    ) -> Self {
        Self {
            id,
            joint,
            state,
            writes,
        }
    }

    fn push(&mut self, edit: automata_physics::JointEdit) {
        edit.apply(&mut self.joint);
        self.writes.borrow_mut().joints.push((self.id, edit));
    }
}

#[pymethods]
impl PyJointRef {
    /// ``"hinge"``, ``"ball"``, ``"slider"``, ``"fixed"``, ``"rope"`` or ``"spring"``.
    #[getter]
    fn kind(&self) -> &'static str {
        self.joint.kind.key()
    }

    /// The joint component (a copy, with this frame's changes).
    #[getter]
    fn component(&self) -> PyJoint {
        PyJoint::from_inner(self.joint.clone())
    }

    /// The entity it holds on to, by name (``None`` = the world).
    #[getter]
    fn connected(&self) -> Option<String> {
        self.joint.connected.clone()
    }

    #[setter]
    fn set_connected(&mut self, name: Option<String>) {
        self.push(automata_physics::JointEdit {
            connected: Some(name),
            ..Default::default()
        });
    }

    /// A hinge's angle (radians), a slider's position (metres), or how far
    /// apart the anchors are.
    #[getter]
    fn value(&self) -> f32 {
        self.state.value
    }

    /// A hinge's angle in radians from where it started (same as ``value``).
    #[getter]
    fn angle(&self) -> f32 {
        self.state.value
    }

    /// Whether it's holding two things together right now.
    #[getter]
    fn attached(&self) -> bool {
        self.state.attached
    }

    /// Whether it snapped past its break force.
    #[getter]
    fn broken(&self) -> bool {
        self.state.broken
    }

    /// The force it held with last step, in newtons.
    #[getter]
    fn force(&self) -> f32 {
        self.state.force
    }

    /// Whether it holds at all; set ``False`` to let go.
    #[getter]
    fn enabled(&self) -> bool {
        self.joint.enabled
    }

    #[setter]
    fn set_enabled(&mut self, on: bool) {
        self.push(automata_physics::JointEdit {
            enabled: Some(on),
            ..Default::default()
        });
    }

    /// The motor's speed, or ``None`` without a motor. Setting it drives at
    /// that speed (``None`` removes the motor).
    #[getter]
    fn motor_speed(&self) -> Option<f32> {
        self.joint.motor.map(|m| m.speed)
    }

    #[setter]
    fn set_motor_speed(&mut self, speed: Option<f32>) {
        self.push(automata_physics::JointEdit {
            motor: Some(speed.map(|s| (s, None))),
            ..Default::default()
        });
    }

    /// The angle / position the motor pulls to, or ``None``. Setting it
    /// pulls there (``None`` removes the motor).
    #[getter]
    fn motor_target(&self) -> Option<f32> {
        self.joint.motor.and_then(|m| m.target)
    }

    #[setter]
    fn set_motor_target(&mut self, target: Option<f32>) {
        let speed = self.joint.motor.map_or(0.0, |m| m.speed);
        self.push(automata_physics::JointEdit {
            motor: Some(target.map(|t| (speed, Some(t)))),
            ..Default::default()
        });
    }

    /// ``(min, max)`` angle (radians) or slide (metres), or ``None``.
    #[getter]
    fn limits(&self) -> Option<(f32, f32)> {
        self.joint.limits.map(|[a, b]| (a, b))
    }

    #[setter]
    fn set_limits(&mut self, limits: Option<(f32, f32)>) {
        self.push(automata_physics::JointEdit {
            limits: Some(limits.map(|(a, b)| [a, b])),
            ..Default::default()
        });
    }

    /// Newtons that snap it (``0`` = unbreakable).
    #[getter]
    fn break_force(&self) -> f32 {
        self.joint.break_force
    }

    #[setter]
    fn set_break_force(&mut self, newtons: f32) {
        self.push(automata_physics::JointEdit {
            break_force: Some(newtons),
            ..Default::default()
        });
    }

    fn __repr__(&self) -> String {
        format!(
            "JointRef({}, value={:.3}, attached={})",
            self.joint.kind.key(),
            self.state.value,
            if self.state.attached { "True" } else { "False" }
        )
    }
}
