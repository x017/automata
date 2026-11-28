//! Python bindings for ragdolls: the `Ragdoll` component.
//!
//! ```python
//! at.Entity("Runner").add(at.MeshRenderer.model("runner.glb")).add(at.Animation("Idle")).add(at.Ragdoll())
//! ```
//!
//! Behaviour scripts switch it with `entity.ragdoll.on()` / `.off()` and
//! push it with `entity.ragdoll.push(...)`.

use automata_scene::{Ragdoll, RagdollStatus};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

fn finite(name: &str, v: f32, min: f32, max: f32) -> PyResult<f32> {
    if v.is_finite() && (min..=max).contains(&v) {
        Ok(v)
    } else {
        Err(PyValueError::new_err(format!(
            "{name} must be a number from {min} to {max}, not {v}"
        )))
    }
}

/// An animated model that can go limp (what Oberon's Inspector calls a
/// Ragdoll): physics throws its skeleton about, one capsule per big bone
/// joined at the knees, elbows, hips and so on, worked out from the
/// model. Switching it off blends back to the animation over
/// `blend_time` seconds, the entity standing where the hips came down.
///
/// `active` starts it limp, `mass` is the whole body's (kg), `stiffness`
/// (0 to 1) how hard the joints hold the pose it had when it went limp.
///
/// ```python
/// at.Entity("Robot").add(at.MeshRenderer.model("robot.glb")).add(at.Animation("Idle")).add(at.Ragdoll(mass=80))
/// ```
#[pyclass(name = "Ragdoll", module = "automata._core", frozen)]
#[derive(Debug, Clone)]
pub struct PyRagdoll {
    pub(crate) inner: Ragdoll,
}

#[pymethods]
impl PyRagdoll {
    #[new]
    #[pyo3(signature = (active=false, mass=70.0, stiffness=0.0, blend_time=0.5))]
    fn new(active: bool, mass: f32, stiffness: f32, blend_time: f32) -> PyResult<Self> {
        Ok(Self {
            inner: Ragdoll {
                active,
                mass: finite("mass", mass, 1.0, 10_000.0)?,
                stiffness: finite("stiffness", stiffness, 0.0, 1.0)?,
                blend_time: finite("blend_time", blend_time, 0.0, 10.0)?,
            },
        })
    }

    /// Whether it starts limp.
    #[getter]
    fn active(&self) -> bool {
        self.inner.active
    }

    /// The whole body's mass (kg).
    #[getter]
    fn mass(&self) -> f32 {
        self.inner.mass
    }

    /// How hard the joints hold their pose (0 limp, 1 stiff).
    #[getter]
    fn stiffness(&self) -> f32 {
        self.inner.stiffness
    }

    /// Seconds the animation takes to take over again.
    #[getter]
    fn blend_time(&self) -> f32 {
        self.inner.blend_time
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<Self>()
            .is_ok_and(|o| o.get().inner == self.inner)
    }

    fn __repr__(&self) -> String {
        format!(
            "Ragdoll(active={}, mass={}, stiffness={}, blend_time={})",
            if self.inner.active { "True" } else { "False" },
            self.inner.mass,
            self.inner.stiffness,
            self.inner.blend_time
        )
    }
}

/// A ragdoll's state as the dict behaviour scripts read (`on`, `mass`,
/// `stiff`, `blend`, `limp`, `w` the weight over the animation, `rest`
/// limp and still, `hips` where the hips are or `None`).
pub(crate) fn status_dict<'py>(py: Python<'py>, s: &RagdollStatus) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("on", s.ragdoll.active)?;
    d.set_item("mass", s.ragdoll.mass)?;
    d.set_item("stiff", s.ragdoll.stiffness)?;
    d.set_item("blend", s.ragdoll.blend_time)?;
    d.set_item("limp", s.limp)?;
    d.set_item("w", s.weight)?;
    d.set_item("rest", s.resting)?;
    d.set_item("hips", s.hips.map(|h| (h.x, h.y, h.z)))?;
    Ok(d)
}
