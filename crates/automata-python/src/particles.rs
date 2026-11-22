//! `at.ParticleEmitter`: the particle emitter component for Python.

use automata_scene::{blend_label, ParticleEmitter, ParticleField};
use pyo3::exceptions::{PyAttributeError, PyKeyError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict};

/// Makes particles: fire, smoke, sparks, dust, splashes, magic, motes.
///
/// Start from a preset and change what you like, or set fields from
/// scratch (any field below, by keyword):
///
/// ```python
/// fire = at.ParticleEmitter.preset("fire")
/// embers = at.ParticleEmitter.preset("sparks", rate=6.0, one_shot=False)
/// mist = at.ParticleEmitter(rate=4, sprite="puff", lit=True, size=(1, 3),
///                           color=(1, 1, 1, 0.3), color_end=(1, 1, 1, 0))
/// scene.add(at.Entity("campfire").add(at.Transform()).add(fire))
/// ```
///
/// Fields: `emitting`, `rate` (per second), `burst` (at start),
/// `duration` (s, 0 = forever), `one_shot` (despawn when done),
/// `max_particles`, `lifetime` (min, max s), `shape` ("point", "sphere",
/// "box", "cone"), `shape_size` (radius or half extents, m), `cone_angle`
/// (degrees from up), `speed` (min, max m/s), `gravity` (m/s² down;
/// negative rises), `drag`, `size` (at birth, at death, m), `size_spread`,
/// `color` and `color_end` (RGBA; over 1 glows), `fade_in` and `fade_out`
/// (shares of life), `spin` (rad/s), `stretch` (s of travel), `blend`
/// ("alpha", "additive"), `lit`, `soft` (m), `sprite` ("dot", "spark",
/// "star", "flame", "puff"), `texture` (image file), `sheet` (columns,
/// rows), `world_space`, `follow_camera` (fill the air around the camera:
/// the emitter's box repeats through space, for dust motes or snow).
#[pyclass(name = "ParticleEmitter", module = "automata._core", frozen)]
#[derive(Debug, Clone)]
pub struct PyParticleEmitter {
    inner: ParticleEmitter,
}

/// A Python value as a [`ParticleField`].
fn field_value(v: &Bound<'_, PyAny>) -> PyResult<ParticleField> {
    if let Ok(b) = v.cast::<PyBool>() {
        return Ok(ParticleField::Bool(b.is_true()));
    }
    if v.is_none() {
        return Ok(ParticleField::Text(String::new()));
    }
    if let Ok(s) = v.extract::<String>() {
        return Ok(ParticleField::Text(s));
    }
    if let Ok(x) = v.extract::<f32>() {
        return Ok(ParticleField::Number(x));
    }
    if let Ok(xs) = v.extract::<Vec<f32>>() {
        return Ok(ParticleField::Numbers(xs));
    }
    Err(PyTypeError::new_err(format!(
        "expected a number, tuple, bool or name, got {}",
        v.get_type().name()?
    )))
}

/// Sets field `name` of `e` from a Python value.
fn set_field(e: &mut ParticleEmitter, name: &str, v: &Bound<'_, PyAny>) -> PyResult<()> {
    if !ParticleEmitter::FIELDS.contains(&name) {
        return Err(PyKeyError::new_err(format!(
            "ParticleEmitter has no field {name:?}"
        )));
    }
    e.set_field(name, &field_value(v)?)
        .map_err(PyValueError::new_err)
}

fn apply(e: &mut ParticleEmitter, kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<()> {
    if let Some(kwargs) = kwargs {
        for (k, v) in kwargs.iter() {
            set_field(e, &k.extract::<String>()?, &v)?;
        }
    }
    Ok(())
}

#[pymethods]
impl PyParticleEmitter {
    /// An emitter with the given fields (the rest default: 20 white dots a
    /// second rising in a 25° cone).
    #[new]
    #[pyo3(signature = (**fields))]
    fn new(fields: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
        let mut inner = ParticleEmitter::default();
        apply(&mut inner, fields)?;
        Ok(Self { inner })
    }

    /// A built-in look: "fire", "smoke", "sparks", "dust", "splash" or
    /// "magic" (any case), with `fields` changed.
    #[staticmethod]
    #[pyo3(signature = (name, **fields))]
    fn preset(name: &str, fields: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
        let mut inner = ParticleEmitter::preset(name).ok_or_else(|| {
            PyValueError::new_err(format!(
                "unknown particle preset {name:?}; expected one of {}",
                automata_scene::PARTICLE_PRESETS.join(", ").to_lowercase()
            ))
        })?;
        apply(&mut inner, fields)?;
        Ok(Self { inner })
    }

    /// The built-in presets' names.
    #[staticmethod]
    fn presets() -> Vec<String> {
        automata_scene::PARTICLE_PRESETS
            .iter()
            .map(|n| n.to_lowercase())
            .collect()
    }

    /// A copy with `fields` changed.
    #[pyo3(signature = (**fields))]
    fn replace(&self, fields: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
        let mut inner = self.inner.clone();
        apply(&mut inner, fields)?;
        Ok(Self { inner })
    }

    /// The preset this is exactly (lowercase), or None.
    #[getter(preset_name)]
    fn preset_name(&self) -> Option<String> {
        self.inner.preset_name().map(str::to_lowercase)
    }

    fn __getattr__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        let e = &self.inner;
        let t2 = |v: [f32; 2]| (v[0], v[1]);
        let t4 = |v: [f32; 4]| (v[0], v[1], v[2], v[3]);
        let lower = |s: &str| s.to_lowercase();
        Ok(match name {
            "emitting" => e.emitting.into_pyobject(py)?.to_owned().into_any().unbind(),
            "rate" => e.rate.into_pyobject(py)?.into_any().unbind(),
            "burst" => e.burst.into_pyobject(py)?.into_any().unbind(),
            "duration" => e.duration.into_pyobject(py)?.into_any().unbind(),
            "one_shot" => e.one_shot.into_pyobject(py)?.to_owned().into_any().unbind(),
            "max_particles" => e.max_particles.into_pyobject(py)?.into_any().unbind(),
            "lifetime" => t2(e.lifetime).into_pyobject(py)?.into_any().unbind(),
            "shape" => lower(e.shape.label())
                .into_pyobject(py)?
                .into_any()
                .unbind(),
            "shape_size" => {
                let [x, y, z] = e.shape_size;
                (x, y, z).into_pyobject(py)?.into_any().unbind()
            }
            "cone_angle" => e.cone_angle.into_pyobject(py)?.into_any().unbind(),
            "speed" => t2(e.speed).into_pyobject(py)?.into_any().unbind(),
            "gravity" => e.gravity.into_pyobject(py)?.into_any().unbind(),
            "drag" => e.drag.into_pyobject(py)?.into_any().unbind(),
            "size" => t2(e.size).into_pyobject(py)?.into_any().unbind(),
            "size_spread" => e.size_spread.into_pyobject(py)?.into_any().unbind(),
            "color" => t4(e.color).into_pyobject(py)?.into_any().unbind(),
            "color_end" => t4(e.color_end).into_pyobject(py)?.into_any().unbind(),
            "fade_in" => e.fade_in.into_pyobject(py)?.into_any().unbind(),
            "fade_out" => e.fade_out.into_pyobject(py)?.into_any().unbind(),
            "spin" => e.spin.into_pyobject(py)?.into_any().unbind(),
            "stretch" => e.stretch.into_pyobject(py)?.into_any().unbind(),
            "blend" => lower(blend_label(e.blend))
                .into_pyobject(py)?
                .into_any()
                .unbind(),
            "lit" => e.lit.into_pyobject(py)?.to_owned().into_any().unbind(),
            "soft" => e.soft.into_pyobject(py)?.into_any().unbind(),
            "sprite" => lower(e.sprite.label())
                .into_pyobject(py)?
                .into_any()
                .unbind(),
            "texture" => e.texture.clone().into_pyobject(py)?.into_any().unbind(),
            "sheet" => (e.sheet[0], e.sheet[1])
                .into_pyobject(py)?
                .into_any()
                .unbind(),
            "world_space" => e
                .world_space
                .into_pyobject(py)?
                .to_owned()
                .into_any()
                .unbind(),
            "follow_camera" => e
                .follow_camera
                .into_pyobject(py)?
                .to_owned()
                .into_any()
                .unbind(),
            _ => {
                return Err(PyAttributeError::new_err(format!(
                    "'ParticleEmitter' object has no attribute {name:?}"
                )))
            }
        })
    }

    #[allow(clippy::unused_self)] // Python calls it on the instance
    fn __dir__(&self) -> Vec<&'static str> {
        let mut names = ParticleEmitter::FIELDS.to_vec();
        names.extend(["replace", "preset_name", "preset", "presets"]);
        names
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<Self>()
            .is_ok_and(|o| o.get().inner == self.inner)
    }

    fn __repr__(&self) -> String {
        match self.inner.preset_name() {
            Some(name) => format!("ParticleEmitter.preset({:?})", name.to_lowercase()),
            None => format!(
                "ParticleEmitter(rate={}, sprite={:?}, blend={:?})",
                self.inner.rate,
                self.inner.sprite.label().to_lowercase(),
                blend_label(self.inner.blend).to_lowercase()
            ),
        }
    }
}

impl PyParticleEmitter {
    /// The engine component value.
    pub fn inner(&self) -> ParticleEmitter {
        self.inner.clone()
    }

    /// Wraps an engine emitter for Python (used by scene enumeration).
    pub fn from_inner(inner: ParticleEmitter) -> Self {
        Self { inner }
    }
}

/// An emitter from a preset name or an `at.ParticleEmitter`.
pub fn emitter_from(obj: &Bound<'_, PyAny>) -> PyResult<ParticleEmitter> {
    if let Ok(e) = obj.cast::<PyParticleEmitter>() {
        return Ok(e.get().inner());
    }
    let name: String = obj.extract()?;
    ParticleEmitter::preset(&name).ok_or_else(|| {
        PyValueError::new_err(format!(
            "unknown particle preset {name:?}; expected one of {}",
            automata_scene::PARTICLE_PRESETS.join(", ").to_lowercase()
        ))
    })
}
