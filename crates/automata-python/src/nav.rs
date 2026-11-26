//! Python bindings for navigation: the `NavAgent`, `Navigation` and
//! `NavLink` components, and the path queries behind `ctx.nav`.
//!
//! ```python
//! at.Entity("Robot").add(at.Transform()).add(at.NavAgent(speed=3.0))
//! at.Entity("Navigation").add(at.Navigation(file="level.nav.ron"))
//! ```

use automata_nav::{NavAgent, NavLink, NavSettings, Navigation};
use glam::Vec3;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

fn finite(name: &str, v: f32, min: f32) -> PyResult<f32> {
    if v.is_finite() && v >= min {
        Ok(v)
    } else {
        Err(PyValueError::new_err(format!(
            "{name} must be a finite number of at least {min}, not {v}"
        )))
    }
}

/// An entity that finds its own way around the navigation mesh (what
/// Oberon's Inspector calls a Nav Agent). Scripts send it somewhere with
/// `entity.nav.go(target)`; it walks the shortest way, steering round
/// corners and other agents and taking jump links. With a
/// `CharacterController` it moves through the controller (and falls);
/// otherwise it glides `hover` metres above the mesh.
///
/// ```python
/// at.Entity("Robot").add(at.Transform()).add(at.NavAgent(speed=3.0, radius=0.4))
/// ```
#[pyclass(name = "NavAgent", module = "automata._core", frozen)]
#[derive(Debug, Clone)]
pub struct PyNavAgent {
    pub(crate) inner: NavAgent,
}

#[pymethods]
impl PyNavAgent {
    /// `speed` is the top speed (m/s), `acceleration` how quickly it gets
    /// there and turns (m/s²), `radius` the room it takes among other
    /// agents (m), `hover` how high a non-character floats above the
    /// mesh (m), `stop_distance` how near counts as arrived (m); `avoid`
    /// steers clear of other agents and `turn` faces where it's going.
    #[new]
    #[pyo3(signature = (speed=3.5, acceleration=12.0, radius=0.4, hover=0.0, stop_distance=0.1, avoid=true, turn=true))]
    #[allow(clippy::fn_params_excessive_bools)] // the Python keywords
    fn new(
        speed: f32,
        acceleration: f32,
        radius: f32,
        hover: f32,
        stop_distance: f32,
        avoid: bool,
        turn: bool,
    ) -> PyResult<Self> {
        Ok(Self {
            inner: NavAgent {
                speed: finite("speed", speed, 0.0)?,
                acceleration: finite("acceleration", acceleration, 0.0)?,
                radius: finite("radius", radius, 0.0)?,
                hover: finite("hover", hover, f32::MIN)?,
                stop_distance: finite("stop_distance", stop_distance, 0.0)?,
                avoid,
                turn,
                ..NavAgent::default()
            },
        })
    }

    /// Top speed (m/s).
    #[getter]
    fn speed(&self) -> f32 {
        self.inner.speed
    }

    /// How quickly it speeds up, slows down and turns (m/s²).
    #[getter]
    fn acceleration(&self) -> f32 {
        self.inner.acceleration
    }

    /// The room it takes among other agents (m).
    #[getter]
    fn radius(&self) -> f32 {
        self.inner.radius
    }

    /// How high a non-character floats above the mesh (m).
    #[getter]
    fn hover(&self) -> f32 {
        self.inner.hover
    }

    /// How near the target counts as arrived (m).
    #[getter]
    fn stop_distance(&self) -> f32 {
        self.inner.stop_distance
    }

    /// Whether it steers clear of other agents.
    #[getter]
    fn avoid(&self) -> bool {
        self.inner.avoid
    }

    /// Whether it turns to face where it's going.
    #[getter]
    fn turn(&self) -> bool {
        self.inner.turn
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<Self>()
            .is_ok_and(|o| o.get().inner == self.inner)
    }

    fn __repr__(&self) -> String {
        format!(
            "NavAgent(speed={}, radius={}, hover={})",
            self.inner.speed, self.inner.radius, self.inner.hover
        )
    }
}

/// The scene's navigation mesh settings, on one entity (Oberon names it
/// "Navigation"): how the walkable floor is worked out from the static
/// colliders, and `file`, where Oberon's Bake button saved it (relative
/// to the scene). A saved mesh that no longer matches the level, or none
/// at all, is baked when the game starts.
///
/// ```python
/// at.Entity("Navigation").add(at.Navigation(agent_radius=0.4, jump_distance=0))
/// ```
#[pyclass(name = "Navigation", module = "automata._core", frozen)]
#[derive(Debug, Clone)]
pub struct PyNavigation {
    pub(crate) inner: Navigation,
}

#[pymethods]
impl PyNavigation {
    /// `agent_radius` keeps the mesh that far from walls, `agent_height`
    /// is the headroom needed, `max_climb` the highest step walked up,
    /// `max_slope` the steepest walkable slope (degrees),
    /// `jump_distance`/`jump_height` the gaps and ledges jumped (0 for
    /// none), `drop_height` the deepest drop jumped down, `cell_size`
    /// the detail (smaller is finer but slower), `min_area` the smallest
    /// island kept (m²).
    #[new]
    #[pyo3(signature = (file="", agent_radius=0.35, agent_height=1.8, max_climb=0.4, max_slope=45.0, jump_distance=2.5, jump_height=1.2, drop_height=3.0, cell_size=0.2, min_area=0.5))]
    #[allow(clippy::too_many_arguments)] // the Python keywords
    fn new(
        file: &str,
        agent_radius: f32,
        agent_height: f32,
        max_climb: f32,
        max_slope: f32,
        jump_distance: f32,
        jump_height: f32,
        drop_height: f32,
        cell_size: f32,
        min_area: f32,
    ) -> PyResult<Self> {
        let settings = NavSettings {
            cell_size: finite("cell_size", cell_size, 0.01)?,
            agent_radius: finite("agent_radius", agent_radius, 0.0)?,
            agent_height: finite("agent_height", agent_height, 0.0)?,
            max_climb: finite("max_climb", max_climb, 0.0)?,
            max_slope: finite("max_slope", max_slope, 0.0)?,
            jump_distance: finite("jump_distance", jump_distance, 0.0)?,
            jump_height: finite("jump_height", jump_height, 0.0)?,
            drop_height: finite("drop_height", drop_height, 0.0)?,
            min_area: finite("min_area", min_area, 0.0)?,
            ..NavSettings::default()
        };
        Ok(Self {
            inner: Navigation {
                settings,
                file: file.to_string(),
            },
        })
    }

    /// Where the baked mesh is saved, relative to the scene ("" if never).
    #[getter]
    fn file(&self) -> String {
        self.inner.file.clone()
    }

    /// How far the mesh keeps from walls (m).
    #[getter]
    fn agent_radius(&self) -> f32 {
        self.inner.settings.agent_radius
    }

    /// The headroom an agent needs (m).
    #[getter]
    fn agent_height(&self) -> f32 {
        self.inner.settings.agent_height
    }

    /// The highest step walked up (m).
    #[getter]
    fn max_climb(&self) -> f32 {
        self.inner.settings.max_climb
    }

    /// The steepest walkable slope (degrees).
    #[getter]
    fn max_slope(&self) -> f32 {
        self.inner.settings.max_slope
    }

    /// The widest gap jumped (m; 0: no jump links).
    #[getter]
    fn jump_distance(&self) -> f32 {
        self.inner.settings.jump_distance
    }

    /// The deepest drop jumped down (m).
    #[getter]
    fn drop_height(&self) -> f32 {
        self.inner.settings.drop_height
    }

    /// The size of the cells the floor is worked out in (m).
    #[getter]
    fn cell_size(&self) -> f32 {
        self.inner.settings.cell_size
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<Self>()
            .is_ok_and(|o| o.get().inner == self.inner)
    }

    fn __repr__(&self) -> String {
        format!(
            "Navigation(file={:?}, agent_radius={})",
            self.inner.file, self.inner.settings.agent_radius
        )
    }
}

/// A hand-placed shortcut for nav agents: from this entity to the entity
/// named `to` (a jump the bake didn't find, a ladder, a teleporter), and
/// back too when `two_way`.
///
/// ```python
/// at.Entity("LedgeTop").add(at.Transform(position=(4, 3, 0))).add(at.NavLink("LedgeFoot"))
/// ```
#[pyclass(name = "NavLink", module = "automata._core", frozen)]
#[derive(Debug, Clone)]
pub struct PyNavLink {
    pub(crate) inner: NavLink,
}

#[pymethods]
impl PyNavLink {
    /// Links this entity to the one named `to`.
    #[new]
    #[pyo3(signature = (to, two_way=true))]
    fn new(to: &str, two_way: bool) -> Self {
        Self {
            inner: NavLink {
                to: to.to_string(),
                two_way,
            },
        }
    }

    /// The name of the entity at the other end.
    #[getter]
    fn to(&self) -> String {
        self.inner.to.clone()
    }

    /// Whether agents may take it both ways.
    #[getter]
    fn two_way(&self) -> bool {
        self.inner.two_way
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<Self>()
            .is_ok_and(|o| o.get().inner == self.inner)
    }

    fn __repr__(&self) -> String {
        format!(
            "NavLink({:?}, two_way={})",
            self.inner.to, self.inner.two_way
        )
    }
}

/// A path as Python sees it: `(x, y, z, link)` corners, `link` marking
/// the corners where a jump link starts.
pub(crate) fn path_tuples(path: &[automata_nav::PathPoint]) -> Vec<(f32, f32, f32, bool)> {
    path.iter()
        .map(|p| (p.position.x, p.position.y, p.position.z, p.link))
        .collect()
}

/// An agent's start-of-frame state, for `entity.nav`.
pub(crate) fn agent_state<'py>(
    py: Python<'py>,
    agent: &NavAgent,
) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
    use pyo3::types::PyDictMethods;
    let d = pyo3::types::PyDict::new(py);
    d.set_item("spd", agent.speed)?;
    d.set_item("stop", agent.stop_distance)?;
    d.set_item("hov", agent.hover)?;
    match &agent.target {
        Some(automata_nav::NavTarget::Point(p)) => d.set_item("t", (p.x, p.y, p.z))?,
        Some(automata_nav::NavTarget::Entity(n)) => d.set_item("t", n)?,
        None => d.set_item("t", py.None())?,
    }
    let s = &agent.state;
    d.set_item("arr", s.arrived)?;
    d.set_item("jmp", s.jumping)?;
    d.set_item("vel", (s.velocity.x, s.velocity.y, s.velocity.z))?;
    let rest = s.path.get(s.next..).unwrap_or_default();
    d.set_item("path", path_tuples(rest))?;
    Ok(d)
}

/// Reads a target: a point, a name, or `None`.
pub(crate) fn extract_target(obj: &Bound<'_, PyAny>) -> PyResult<Option<automata_nav::NavTarget>> {
    if obj.is_none() {
        return Ok(None);
    }
    if let Ok(name) = obj.extract::<String>() {
        return Ok(Some(automata_nav::NavTarget::Entity(name)));
    }
    let (x, y, z): (f32, f32, f32) = obj.extract().map_err(|_| {
        pyo3::exceptions::PyTypeError::new_err(
            "a nav target is a point (x, y, z), an entity name, or None",
        )
    })?;
    if !(x.is_finite() && y.is_finite() && z.is_finite()) {
        return Err(PyValueError::new_err("a nav target must be finite"));
    }
    Ok(Some(automata_nav::NavTarget::Point(Vec3::new(x, y, z))))
}
