//! Custom entity attributes and prefabs on the Python side.
//!
//! Attributes are the named values Oberon's inspector edits (health, speed,
//! team color …). Python sees them as a dict-like [`PyAttributes`] mapping
//! on both staged entities (`Entity.attributes`) and live ones
//! (`EntityView.attributes`, writes buffered like transforms).
//!
//! A [`PyPrefab`] is a reusable entity template stored in the same RON
//! format Oberon writes for `.prefab` files.

use std::collections::BTreeMap;

use automata_core::scene_file::AttributeValue;
use pyo3::exceptions::{PyIOError, PyKeyError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyString};
use pyo3::IntoPyObjectExt;

use crate::components::StagedComponent;
use crate::scene::{PyEntity, StagedEntity, WriteQueue};

/// The attribute map type shared by staged and live entities.
pub type AttributeMap = BTreeMap<String, AttributeValue>;

/// Converts an attribute value into a Python object: `bool`, `int`,
/// `float`, `str`, or a 3/4-tuple of floats for vectors and colors.
pub fn value_to_py(py: Python<'_>, value: &AttributeValue) -> PyResult<Py<PyAny>> {
    match value {
        AttributeValue::Bool(v) => v.into_py_any(py),
        AttributeValue::Int(v) => v.into_py_any(py),
        AttributeValue::Float(v) => v.into_py_any(py),
        AttributeValue::Text(v) => v.into_py_any(py),
        AttributeValue::Vec3([x, y, z]) => (*x, *y, *z).into_py_any(py),
        AttributeValue::Color([r, g, b, a]) => (*r, *g, *b, *a).into_py_any(py),
    }
}

/// Converts a Python value into an attribute value.
///
/// `bool` → Bool, `int` → Int, `float` → Float, `str` → Text, a 3-sequence
/// of numbers → Vec3, a 4-sequence → Color. Anything else raises
/// `TypeError`.
pub fn value_from_py(obj: &Bound<'_, PyAny>) -> PyResult<AttributeValue> {
    // bool first: Python's bool is a subclass of int.
    if obj.is_instance_of::<PyBool>() {
        return Ok(AttributeValue::Bool(obj.extract()?));
    }
    if obj.is_instance_of::<PyInt>() {
        return Ok(AttributeValue::Int(obj.extract()?));
    }
    if obj.is_instance_of::<PyFloat>() {
        return Ok(AttributeValue::Float(obj.extract()?));
    }
    if obj.is_instance_of::<PyString>() {
        return Ok(AttributeValue::Text(obj.extract()?));
    }
    if let Ok(values) = obj.extract::<Vec<f32>>() {
        match values.as_slice() {
            [x, y, z] => return Ok(AttributeValue::Vec3([*x, *y, *z])),
            [r, g, b, a] => return Ok(AttributeValue::Color([*r, *g, *b, *a])),
            _ => {}
        }
    }
    Err(PyTypeError::new_err(format!(
        "attribute values must be bool, int, float, str, or a 3/4-tuple of numbers, \
         not {}",
        obj.get_type().name()?
    )))
}

/// Converts a Python mapping into an attribute map.
pub fn map_from_py(obj: &Bound<'_, PyAny>) -> PyResult<AttributeMap> {
    if let Ok(attrs) = obj.cast::<PyAttributes>() {
        return Ok(attrs.borrow().read(obj.py()));
    }
    let dict = obj
        .cast::<PyDict>()
        .map_err(|_| PyTypeError::new_err("attributes must be a dict"))?;
    let mut map = AttributeMap::new();
    for (key, value) in dict.iter() {
        let key: String = key
            .extract()
            .map_err(|_| PyTypeError::new_err("attribute names must be strings"))?;
        map.insert(key, value_from_py(&value)?);
    }
    Ok(map)
}

/// Where an attribute mapping reads and writes.
#[derive(Debug)]
enum Sink {
    /// A staged (not yet spawned) entity: edits change it directly.
    Staged(Py<PyEntity>),
    /// A live entity during the update callback: reads see the
    /// start-of-frame values plus this frame's writes; writes are buffered.
    Live {
        id: u64,
        base: AttributeMap,
        writes: WriteQueue,
    },
}

/// An entity's custom attributes, used like a dict:
///
/// ```python
/// hp = entity.attributes.get("health", 100)
/// entity.attributes["health"] = hp - 10
/// "team" in entity.attributes
/// ```
///
/// Values are `bool`, `int`, `float`, `str`, or 3/4-tuples (vectors and
/// colors) — the same types Oberon's inspector edits.
#[pyclass(name = "Attributes", unsendable, module = "automata._core")]
#[derive(Debug)]
pub struct PyAttributes {
    sink: Sink,
}

impl PyAttributes {
    /// Attributes of a staged entity.
    pub fn staged(entity: Py<PyEntity>) -> Self {
        Self {
            sink: Sink::Staged(entity),
        }
    }

    /// Attributes of a live entity, with buffered writes.
    pub fn live(id: u64, base: AttributeMap, writes: WriteQueue) -> Self {
        Self {
            sink: Sink::Live { id, base, writes },
        }
    }

    /// The current values.
    fn read(&self, py: Python<'_>) -> AttributeMap {
        match &self.sink {
            Sink::Staged(entity) => entity.borrow(py).staged().attributes.clone(),
            Sink::Live { id, base, writes } => writes
                .borrow()
                .attributes
                .get(id)
                .cloned()
                .unwrap_or_else(|| base.clone()),
        }
    }

    /// Applies `edit` to the values.
    fn write(&self, py: Python<'_>, edit: impl FnOnce(&mut AttributeMap)) {
        match &self.sink {
            Sink::Staged(entity) => edit(&mut entity.borrow_mut(py).staged_mut().attributes),
            Sink::Live { id, base, writes } => {
                let mut writes = writes.borrow_mut();
                let map = writes.attributes.entry(*id).or_insert_with(|| base.clone());
                edit(map);
            }
        }
    }
}

#[pymethods]
impl PyAttributes {
    fn __getitem__(&self, py: Python<'_>, key: &str) -> PyResult<Py<PyAny>> {
        match self.read(py).get(key) {
            Some(value) => value_to_py(py, value),
            None => Err(PyKeyError::new_err(key.to_string())),
        }
    }

    fn __setitem__(&self, py: Python<'_>, key: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let value = value_from_py(value)?;
        self.write(py, |map| {
            map.insert(key.to_string(), value);
        });
        Ok(())
    }

    fn __delitem__(&self, py: Python<'_>, key: &str) -> PyResult<()> {
        if !self.read(py).contains_key(key) {
            return Err(PyKeyError::new_err(key.to_string()));
        }
        self.write(py, |map| {
            map.remove(key);
        });
        Ok(())
    }

    fn __contains__(&self, py: Python<'_>, key: &str) -> bool {
        self.read(py).contains_key(key)
    }

    fn __len__(&self, py: Python<'_>) -> usize {
        self.read(py).len()
    }

    fn __iter__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let keys = pyo3::types::PyList::new(py, self.read(py).into_keys())?;
        Ok(keys.as_any().try_iter()?.unbind().into_any())
    }

    /// The value for `key`, or `default` when missing.
    #[pyo3(signature = (key, default=None))]
    fn get(&self, py: Python<'_>, key: &str, default: Option<Py<PyAny>>) -> PyResult<Py<PyAny>> {
        match self.read(py).get(key) {
            Some(value) => value_to_py(py, value),
            None => Ok(default.unwrap_or_else(|| py.None())),
        }
    }

    /// The attribute names, sorted.
    fn keys(&self, py: Python<'_>) -> Vec<String> {
        self.read(py).into_keys().collect()
    }

    /// The values, in name order.
    fn values(&self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        self.read(py).values().map(|v| value_to_py(py, v)).collect()
    }

    /// `(name, value)` pairs, in name order.
    fn items(&self, py: Python<'_>) -> PyResult<Vec<(String, Py<PyAny>)>> {
        self.read(py)
            .iter()
            .map(|(k, v)| Ok((k.clone(), value_to_py(py, v)?)))
            .collect()
    }

    /// Sets several attributes from a dict.
    fn update(&self, py: Python<'_>, values: &Bound<'_, PyAny>) -> PyResult<()> {
        let incoming = map_from_py(values)?;
        self.write(py, |map| map.extend(incoming));
        Ok(())
    }

    /// Removes every attribute.
    fn clear(&self, py: Python<'_>) {
        self.write(py, BTreeMap::clear);
    }

    /// A plain `dict` copy.
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new(py);
        for (k, v) in self.read(py) {
            dict.set_item(k, value_to_py(py, &v)?)?;
        }
        Ok(dict)
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let body: Vec<String> = self
            .read(py)
            .iter()
            .map(|(k, v)| format!("{k:?}: {}", repr_value(v)))
            .collect();
        format!("Attributes({{{}}})", body.join(", "))
    }
}

/// A Python-style repr of an attribute value.
fn repr_value(value: &AttributeValue) -> String {
    match value {
        AttributeValue::Bool(true) => "True".into(),
        AttributeValue::Bool(false) => "False".into(),
        AttributeValue::Int(v) => v.to_string(),
        AttributeValue::Float(v) => format!("{v:?}"),
        AttributeValue::Text(v) => format!("{v:?}"),
        AttributeValue::Vec3([x, y, z]) => format!("({x:?}, {y:?}, {z:?})"),
        AttributeValue::Color([r, g, b, a]) => format!("({r:?}, {g:?}, {b:?}, {a:?})"),
    }
}

/// A reusable entity template (Oberon's `.prefab` files).
///
/// ```python
/// crate = at.Prefab.load("prefabs/crate.prefab")
/// scene.add(crate.instantiate(position=(2, 0, 0)))   # staged
/// ctx.scene.spawn(crate, position=(0, 5, 0))          # at runtime
/// ```
///
/// Build one from code with `Prefab(entity)` and write it with `save`.
#[pyclass(name = "Prefab", module = "automata._core", subclass)]
#[derive(Debug, Clone, Default)]
pub struct PyPrefab {
    entities: Vec<StagedEntity>,
}

#[pymethods]
impl PyPrefab {
    /// A prefab from one `Entity` or a list of them.
    #[new]
    #[pyo3(signature = (entities=None))]
    fn new(entities: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let Some(entities) = entities else {
            return Ok(Self::default());
        };
        if let Ok(entity) = entities.cast::<PyEntity>() {
            return Ok(Self {
                entities: vec![entity.borrow().staged().clone()],
            });
        }
        let mut staged = Vec::new();
        for item in entities.try_iter()? {
            let item = item?;
            let entity = item
                .cast::<PyEntity>()
                .map_err(|_| PyTypeError::new_err("Prefab takes an Entity or a list of them"))?;
            staged.push(entity.borrow().staged().clone());
        }
        Ok(Self { entities: staged })
    }

    /// Parses a prefab from RON text (the scene-file format).
    #[classmethod]
    fn from_ron(_cls: &Bound<'_, pyo3::types::PyType>, text: &str) -> PyResult<Self> {
        let entities = crate::serialize::scene_from_ron(text)
            .map_err(|err| PyValueError::new_err(err.to_string()))?;
        Ok(Self { entities })
    }

    /// Loads a `.prefab` file.
    #[classmethod]
    fn load(cls: &Bound<'_, pyo3::types::PyType>, path: &str) -> PyResult<Self> {
        let text =
            std::fs::read_to_string(path).map_err(|err| PyIOError::new_err(err.to_string()))?;
        Self::from_ron(cls, &text)
    }

    /// Serializes the prefab to RON text.
    fn to_ron(&self) -> PyResult<String> {
        crate::serialize::scene_to_ron(&self.entities)
            .map_err(|err| PyValueError::new_err(err.to_string()))
    }

    /// Writes the prefab to `path`.
    fn save(&self, path: &str) -> PyResult<()> {
        std::fs::write(path, self.to_ron()?).map_err(|err| PyIOError::new_err(err.to_string()))
    }

    /// Fresh copies of the template's entities.
    fn entities(&self) -> Vec<PyEntity> {
        self.entities
            .iter()
            .cloned()
            .map(PyEntity::from_staged)
            .collect()
    }

    fn __len__(&self) -> usize {
        self.entities.len()
    }

    /// A staged copy of the prefab's root (first) entity, optionally placed
    /// at `position` and renamed. Raises `ValueError` for empty prefabs.
    #[pyo3(signature = (position=None, name=None))]
    fn instantiate(
        &self,
        position: Option<(f32, f32, f32)>,
        name: Option<String>,
    ) -> PyResult<PyEntity> {
        self.instances(position, name)
            .into_iter()
            .next()
            .map(PyEntity::from_staged)
            .ok_or_else(|| PyValueError::new_err("the prefab is empty"))
    }

    /// Staged copies of every entity, moved together so the root lands at
    /// `position`.
    #[pyo3(signature = (position=None))]
    fn instantiate_all(&self, position: Option<(f32, f32, f32)>) -> Vec<PyEntity> {
        self.instances(position, None)
            .into_iter()
            .map(PyEntity::from_staged)
            .collect()
    }

    fn __repr__(&self) -> String {
        let root = self
            .entities
            .first()
            .and_then(|e| e.name.clone())
            .unwrap_or_default();
        format!("Prefab({root:?}, {} entities)", self.entities.len())
    }
}

impl PyPrefab {
    /// Copies of the entities, offset so the root sits at `position` and
    /// the root renamed to `name`.
    pub fn instances(
        &self,
        position: Option<(f32, f32, f32)>,
        name: Option<String>,
    ) -> Vec<StagedEntity> {
        let mut out = self.entities.clone();
        if let Some((x, y, z)) = position {
            let origin = out
                .first()
                .and_then(transform_of)
                .map_or(glam::Vec3::ZERO, |t| t.position);
            let delta = glam::Vec3::new(x, y, z) - origin;
            for (i, entity) in out.iter_mut().enumerate() {
                let mut found = false;
                for component in &mut entity.components {
                    if let StagedComponent::Transform(t) = component {
                        t.position += delta;
                        found = true;
                    }
                }
                if !found && i == 0 {
                    entity.components.push(StagedComponent::Transform(
                        automata_scene::Transform::from_position(glam::Vec3::new(x, y, z)),
                    ));
                }
            }
        }
        if let (Some(name), Some(root)) = (name, out.first_mut()) {
            root.name = Some(name);
        }
        out
    }
}

/// The staged transform of an entity, if any.
fn transform_of(entity: &StagedEntity) -> Option<automata_scene::Transform> {
    entity.components.iter().find_map(|c| match c {
        StagedComponent::Transform(t) => Some(*t),
        _ => None,
    })
}
