//! Python-side scene staging and the per-frame scene view.
//!
//! [`PyScene`] and [`PyEntity`] are plain-data builders: a game assembles
//! them in Python, then [`crate::game::PyGame::load_scene`] materializes
//! them into the ECS world.
//!
//! During the update callback, games see a [`SceneView`]: a start-of-frame
//! snapshot for reads, plus a command buffer for writes (applied by the
//! engine right after the callback returns). This keeps the Python/Rust
//! ownership story simple — no live borrows cross the language boundary.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use automata_core::scene_file::{Attributes, Script};
use automata_physics::CharacterVelocity;
use automata_renderer::Camera3D;
use automata_scene::{EntityId, Name, Scene, Transform};
use pyo3::exceptions::{PyKeyError, PyTypeError, PyValueError};
use pyo3::prelude::*;

use crate::attributes::{map_from_py, AttributeMap, PyAttributes, PyPrefab};
use crate::components::{extract_component, PyCamera3D, PyTransform, StagedComponent};

/// A staged entity: a name plus the components to insert.
#[derive(Debug, Clone, Default)]
pub struct StagedEntity {
    /// Optional name for later lookup.
    pub name: Option<String>,
    /// Components to insert for the entity.
    pub components: Vec<StagedComponent>,
    /// Custom attributes (Oberon's inspector "Attributes" section).
    pub attributes: AttributeMap,
    /// Path of the behaviour script attached in Oberon, if any.
    pub script: Option<String>,
}

/// A game object builder. Attach components with [`Entity.add`], then place
/// it in a scene with [`Scene.add`]. Methods chain:
/// `Entity("cube").add(Transform())`.
#[pyclass(name = "Entity", module = "automata._core", subclass)]
#[derive(Debug, Clone, Default)]
pub struct PyEntity {
    staged: StagedEntity,
}

#[pymethods]
impl PyEntity {
    /// Creates an entity with an optional name for later lookup.
    #[new]
    #[pyo3(signature = (name=None))]
    fn new(name: Option<String>) -> Self {
        Self {
            staged: StagedEntity {
                name,
                ..StagedEntity::default()
            },
        }
    }

    /// The entity's custom attributes, used like a dict
    /// (`entity.attributes["health"] = 100`). Assign a dict to replace them
    /// all.
    #[getter]
    fn attributes(slf: &Bound<'_, Self>) -> PyAttributes {
        PyAttributes::staged(slf.clone().unbind())
    }

    #[setter]
    fn set_attributes(&mut self, values: &Bound<'_, PyAny>) -> PyResult<()> {
        self.staged.attributes = map_from_py(values)?;
        Ok(())
    }

    /// Path of the attached behaviour script (`.py`, relative to the scene
    /// file), or `None`.
    #[getter]
    fn script(&self) -> Option<String> {
        self.staged.script.clone()
    }

    #[setter]
    fn set_script(&mut self, path: Option<String>) {
        self.staged.script = path;
    }

    /// The entity's name, if any.
    #[getter]
    fn name(&self) -> Option<String> {
        self.staged.name.clone()
    }

    /// Sets the entity's name. Staging is plain data, so renaming before
    /// the entity is spawned is always safe. (Also used by the Python
    /// `Entity` wrapper so subclass constructors can pass the name through
    /// `super().__init__(name)`.)
    #[setter]
    fn set_name(&mut self, name: Option<String>) {
        self.staged.name = name;
    }

    /// Attaches a component (`Transform`, `Camera3D`, `Camera2D`). Returns
    /// the entity so calls can chain.
    fn add<'py>(
        mut slf: PyRefMut<'py, Self>,
        component: &Bound<'_, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        slf.staged.components.push(extract_component(component)?);
        Ok(slf)
    }

    /// The staged components as fresh Python objects, for editors that need
    /// to inspect a scene's contents (Oberon). Each call rebuilds the list.
    fn components(&self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        use crate::components::{
            PyCamera2D, PyCamera3D, PyDirectionalLight, PyMeshRenderer, PyTransform,
        };
        use crate::physics_components::{
            PyCharacterController, PyCharacterVelocity, PyCollider2D, PyCollider3D, PyRigidBody,
        };
        self.staged
            .components
            .iter()
            .map(|c| {
                Ok(match c {
                    StagedComponent::Transform(v) => {
                        Py::new(py, PyTransform::from_inner(*v))?.into_any()
                    }
                    StagedComponent::Camera3D(v) => {
                        Py::new(py, PyCamera3D::from_inner(*v))?.into_any()
                    }
                    StagedComponent::Camera2D(v) => {
                        Py::new(py, PyCamera2D::from_inner(*v))?.into_any()
                    }
                    StagedComponent::RigidBody(v) => {
                        Py::new(py, PyRigidBody::from_inner(v.clone()))?.into_any()
                    }
                    StagedComponent::Collider3D(v) => {
                        Py::new(py, PyCollider3D::from_inner(v.clone()))?.into_any()
                    }
                    StagedComponent::Collider2D(v) => {
                        Py::new(py, PyCollider2D::from_inner(v.clone()))?.into_any()
                    }
                    StagedComponent::CharacterController(v) => {
                        Py::new(py, PyCharacterController::from_inner(v.clone()))?.into_any()
                    }
                    StagedComponent::CharacterVelocity(v) => {
                        Py::new(py, PyCharacterVelocity::from_inner(*v))?.into_any()
                    }
                    StagedComponent::MeshVisual(v) => {
                        Py::new(py, PyMeshRenderer::from_inner(v.clone()))?.into_any()
                    }
                    StagedComponent::DirectionalLight(v) => {
                        Py::new(py, PyDirectionalLight::from_inner(*v))?.into_any()
                    }
                    StagedComponent::Environment(v) => {
                        Py::new(py, crate::components::PyEnvironment::from_inner(*v))?.into_any()
                    }
                    StagedComponent::AudioSource(v) => {
                        Py::new(py, crate::components::PyAudioSource::from_inner(v.clone()))?
                            .into_any()
                    }
                })
            })
            .collect()
    }

    fn __repr__(&self) -> String {
        match &self.staged.name {
            Some(name) => format!(
                "Entity({name:?}, {} components)",
                self.staged.components.len()
            ),
            None => format!("Entity({} components)", self.staged.components.len()),
        }
    }
}

/// A collection of entities, ready to load into a game.
///
/// Staging a scene in Python and loading it in one shot avoids
/// per-entity FFI traffic at startup. Methods chain: `Scene().add(a).add(b)`.
#[pyclass(name = "Scene", module = "automata._core", subclass)]
#[derive(Debug, Clone, Default)]
pub struct PyScene {
    entities: Vec<StagedEntity>,
}

#[pymethods]
impl PyScene {
    /// Creates an empty scene.
    #[new]
    fn new() -> Self {
        Self::default()
    }

    /// Stages an entity (the builder is copied; later edits to it do not
    /// affect the scene), or every entity of a `Prefab`. Returns the scene
    /// so calls can chain.
    fn add<'py>(
        mut slf: PyRefMut<'py, Self>,
        entity: &Bound<'_, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let staged = staged_from_py(entity, None)?;
        slf.entities.extend(staged);
        Ok(slf)
    }

    /// The number of staged entities.
    #[getter]
    fn entity_count(&self) -> usize {
        self.entities.len()
    }

    fn __len__(&self) -> usize {
        self.entities.len()
    }

    /// The staged entities as fresh `Entity` objects, for editors that need
    /// to walk a loaded scene (Oberon). Order matches the scene file.
    fn entities(&self) -> Vec<PyEntity> {
        self.entities
            .iter()
            .map(|staged| PyEntity {
                staged: staged.clone(),
            })
            .collect()
    }

    /// Finds the first staged entity named `name` (a copy), or `None`.
    fn find(&self, name: &str) -> Option<PyEntity> {
        self.entities
            .iter()
            .find(|e| e.name.as_deref() == Some(name))
            .cloned()
            .map(PyEntity::from_staged)
    }

    /// Serializes the staged scene to RON text (the scene-file format).
    fn to_ron(&self) -> PyResult<String> {
        crate::serialize::scene_to_ron(&self.entities)
            .map_err(|err| pyo3::exceptions::PyValueError::new_err(err.to_string()))
    }

    /// Writes the staged scene to `path` as a RON scene file.
    fn save(&self, path: &str) -> PyResult<()> {
        let text = self.to_ron()?;
        std::fs::write(path, text)
            .map_err(|err| pyo3::exceptions::PyIOError::new_err(err.to_string()))
    }

    /// Parses a staged scene from RON text.
    ///
    /// Raises `ValueError` for malformed files or files written by a newer
    /// engine version.
    #[classmethod]
    fn from_ron(_cls: &Bound<'_, pyo3::types::PyType>, text: &str) -> PyResult<Self> {
        let entities = crate::serialize::scene_from_ron(text)
            .map_err(|err| pyo3::exceptions::PyValueError::new_err(err.to_string()))?;
        Ok(Self { entities })
    }

    /// Loads a staged scene from a RON scene file at `path`.
    ///
    /// Raises `ValueError` for malformed files and `IOError` for unreadable
    /// paths.
    #[classmethod]
    fn load(cls: &Bound<'_, pyo3::types::PyType>, path: &str) -> PyResult<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|err| pyo3::exceptions::PyIOError::new_err(err.to_string()))?;
        // Model paths in the scene are relative to its folder.
        if let Ok(abs) = std::path::absolute(path) {
            automata_assets::set_root(abs.parent());
        }
        Self::from_ron(cls, &text)
    }
}

impl PyEntity {
    /// Wraps staged data in a Python-visible builder.
    pub fn from_staged(staged: StagedEntity) -> Self {
        Self { staged }
    }

    /// The staged data.
    pub fn staged(&self) -> &StagedEntity {
        &self.staged
    }

    /// The staged data, mutably.
    pub fn staged_mut(&mut self) -> &mut StagedEntity {
        &mut self.staged
    }
}

/// Staged entities from an `Entity` or a `Prefab` argument, the prefab
/// optionally placed at `position`.
fn staged_from_py(
    obj: &Bound<'_, PyAny>,
    position: Option<(f32, f32, f32)>,
) -> PyResult<Vec<StagedEntity>> {
    if let Ok(prefab) = obj.cast::<PyPrefab>() {
        return Ok(prefab.borrow().instances(position, None));
    }
    let entity = obj
        .cast::<PyEntity>()
        .map_err(|_| PyTypeError::new_err("expected an Entity or a Prefab"))?;
    let mut staged = entity.borrow().staged.clone();
    if let Some((x, y, z)) = position {
        let at = glam::Vec3::new(x, y, z);
        let mut found = false;
        for component in &mut staged.components {
            if let StagedComponent::Transform(t) = component {
                t.position = at;
                found = true;
            }
        }
        if !found {
            staged
                .components
                .push(StagedComponent::Transform(Transform::from_position(at)));
        }
    }
    Ok(vec![staged])
}

impl PyScene {
    /// The staged entities (Rust-side accessor; the Python-visible method
    /// is [`PyScene::entities`]).
    pub fn staged_entities(&self) -> &[StagedEntity] {
        &self.entities
    }
}

/// Replaces the contents of an ECS scene with staged entities.
pub fn load_staged(scene: &mut Scene, staged: &[StagedEntity]) {
    // Despawn one by one rather than `clear()`: clearing resets the entity
    // allocator, so the new scene would reuse the old entities' exact ids
    // and systems keyed by entity (physics bodies) would mistake them for
    // the old ones. Despawning bumps each id's generation.
    let old: Vec<EntityId> = scene.world().iter().map(|e| e.entity()).collect();
    for entity in old {
        let _ = scene.world_mut().despawn(entity);
    }
    for entity in staged {
        spawn_staged(scene, entity);
    }
}

/// Spawns one staged entity into the ECS scene.
fn spawn_staged(scene: &mut Scene, staged: &StagedEntity) -> EntityId {
    let id = scene.spawn(());
    if let Some(name) = &staged.name {
        let _ = scene.world_mut().insert_one(id, Name::new(name.clone()));
    }
    for component in &staged.components {
        match component {
            StagedComponent::Transform(t) => {
                let _ = scene.world_mut().insert_one(id, *t);
            }
            StagedComponent::Camera3D(c) => {
                let _ = scene.world_mut().insert_one(id, *c);
            }
            StagedComponent::Camera2D(c) => {
                let _ = scene.world_mut().insert_one(id, *c);
            }
            StagedComponent::RigidBody(c) => {
                let _ = scene.world_mut().insert_one(id, c.clone());
            }
            StagedComponent::Collider3D(c) => {
                let _ = scene.world_mut().insert_one(id, c.clone());
            }
            StagedComponent::Collider2D(c) => {
                let _ = scene.world_mut().insert_one(id, c.clone());
            }
            StagedComponent::CharacterController(c) => {
                let _ = scene.world_mut().insert_one(id, c.clone());
            }
            StagedComponent::CharacterVelocity(c) => {
                let _ = scene.world_mut().insert_one(id, *c);
            }
            StagedComponent::MeshVisual(c) => {
                let _ = scene.world_mut().insert_one(id, c.clone());
            }
            StagedComponent::DirectionalLight(c) => {
                let _ = scene.world_mut().insert_one(id, *c);
            }
            StagedComponent::Environment(c) => {
                let _ = scene.world_mut().insert_one(id, *c);
            }
            StagedComponent::AudioSource(c) => {
                let _ = scene.world_mut().insert_one(id, c.clone());
            }
        }
    }
    if !staged.attributes.is_empty() {
        let _ = scene
            .world_mut()
            .insert_one(id, Attributes(staged.attributes.clone()));
    }
    if let Some(path) = &staged.script {
        let _ = scene
            .world_mut()
            .insert_one(id, Script { path: path.clone() });
    }
    id
}

/// Buffered scene writes for one frame. Recorded during the Python update
/// callback and applied by the engine immediately after it returns.
#[derive(Debug, Default)]
pub struct FrameWrites {
    /// Pending local transforms, keyed by entity bits. Attribute-level
    /// writes compose: the entry starts as the entity's snapshot transform
    /// and each write updates one field.
    pub transforms: HashMap<u64, Transform>,
    /// Pending character-controller velocities, keyed by entity bits.
    pub character_velocities: HashMap<u64, CharacterVelocity>,
    /// Pending 3D camera replacements, keyed by entity bits.
    pub cameras_3d: HashMap<u64, Camera3D>,
    /// Pending attribute maps (whole-map replacements), keyed by entity
    /// bits.
    pub attributes: HashMap<u64, AttributeMap>,
    /// Entities to spawn.
    pub spawns: Vec<StagedEntity>,
    /// Entities to despawn (recursively).
    pub despawns: Vec<EntityId>,
    /// A whole scene to switch to. Replaces the world before this frame's
    /// spawns are applied; writes aimed at the old scene's entities are
    /// dropped.
    pub load: Option<Vec<StagedEntity>>,
}

/// Shared write buffer between the per-frame views and the game loop.
pub type WriteQueue = Rc<RefCell<FrameWrites>>;

/// Applies buffered writes to the ECS scene.
pub fn apply_writes(scene: &mut Scene, writes: &WriteQueue) {
    let drained = {
        let mut writes = writes.borrow_mut();
        std::mem::take(&mut *writes)
    };
    if let Some(staged) = drained.load {
        load_staged(scene, &staged);
        for staged in drained.spawns {
            spawn_staged(scene, &staged);
        }
        return;
    }
    for (bits, transform) in drained.transforms {
        if let Some(entity) = EntityId::from_bits(bits) {
            if let Ok(mut current) = scene.world_mut().get::<&mut Transform>(entity) {
                *current = transform;
            }
        }
    }
    for (bits, velocity) in drained.character_velocities {
        if let Some(entity) = EntityId::from_bits(bits) {
            let _ = scene.world_mut().insert_one(entity, velocity);
        }
    }
    for (bits, camera) in drained.cameras_3d {
        if let Some(entity) = EntityId::from_bits(bits) {
            let _ = scene.world_mut().insert_one(entity, camera);
        }
    }
    for (bits, map) in drained.attributes {
        if let Some(entity) = EntityId::from_bits(bits) {
            if scene.world().contains(entity) {
                let _ = scene.world_mut().insert_one(entity, Attributes(map));
            }
        }
    }
    for staged in drained.spawns {
        spawn_staged(scene, &staged);
    }
    for entity in drained.despawns {
        if scene.world().contains(entity) {
            scene.despawn_recursive(entity);
        }
    }
}

/// One entity's start-of-frame state, as seen by the update callback.
#[derive(Debug, Clone)]
struct EntitySnapshot {
    id: EntityId,
    name: Option<String>,
    transform: Option<Transform>,
    character_velocity: Option<CharacterVelocity>,
    camera_3d: Option<Camera3D>,
    attributes: AttributeMap,
    script: Option<String>,
    audio_source: Option<automata_core::AudioSource>,
}

/// Read access to the scene during the update callback.
///
/// Reads observe the world as it was at the start of the frame. Writes
/// (`spawn`, `despawn`, entity `transform` assignments) are buffered and
/// applied right after the callback returns — they are visible from the
/// next frame onward.
#[pyclass(name = "SceneView", unsendable, module = "automata._core")]
#[derive(Debug, Clone)]
pub struct SceneView {
    snapshot: Vec<EntitySnapshot>,
    writes: WriteQueue,
}

impl SceneView {
    /// Builds a view over the scene's current state.
    pub fn snapshot(scene: &Scene, writes: WriteQueue) -> Self {
        let snapshot = scene
            .world()
            .query::<(
                EntityId,
                Option<&Name>,
                Option<&Transform>,
                Option<&CharacterVelocity>,
                Option<&Camera3D>,
                Option<&Attributes>,
                Option<&Script>,
                Option<&automata_core::AudioSource>,
            )>()
            .iter()
            .map(
                |(id, name, transform, velocity, camera, attributes, script, audio)| {
                    EntitySnapshot {
                        id,
                        name: name.map(|n| n.0.clone()),
                        transform: transform.copied(),
                        character_velocity: velocity.copied(),
                        camera_3d: camera.copied(),
                        attributes: attributes.map(|a| a.0.clone()).unwrap_or_default(),
                        script: script.map(|s| s.path.clone()),
                        audio_source: audio.cloned(),
                    }
                },
            )
            .collect();
        Self { snapshot, writes }
    }
}

#[pymethods]
impl SceneView {
    /// The number of live entities at the start of the frame.
    #[getter]
    fn entity_count(&self) -> usize {
        self.snapshot.len()
    }

    /// Finds the first entity with the given name, or `None`.
    fn find(&self, name: &str) -> Option<EntityView> {
        self.snapshot
            .iter()
            .find(|e| e.name.as_deref() == Some(name))
            .map(|e| EntityView::from_snapshot(e, Rc::clone(&self.writes)))
    }

    /// `scene["name"]` — like [`find`] but raises `KeyError` when missing.
    fn __getitem__(&self, name: &str) -> PyResult<EntityView> {
        self.find(name)
            .ok_or_else(|| PyKeyError::new_err(format!("no entity named {name:?}")))
    }

    /// Iterates over all entities alive at the start of the frame.
    fn __iter__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let views: Vec<EntityView> = self
            .snapshot
            .iter()
            .map(|e| EntityView::from_snapshot(e, Rc::clone(&self.writes)))
            .collect();
        let list = pyo3::types::PyList::new(py, views)?;
        Ok(list.as_any().try_iter()?.unbind().into_any())
    }

    /// The names of all named entities at the start of the frame.
    fn names(&self) -> Vec<String> {
        self.snapshot
            .iter()
            .filter_map(|e| e.name.clone())
            .collect()
    }

    /// Spawns a staged entity, or every entity of a `Prefab` (applied
    /// after the callback returns; look it up by name from the next frame
    /// onward). `position` places the entity / the prefab's root.
    #[pyo3(signature = (entity, position=None))]
    fn spawn(&self, entity: &Bound<'_, PyAny>, position: Option<(f32, f32, f32)>) -> PyResult<()> {
        let staged = staged_from_py(entity, position)?;
        self.writes.borrow_mut().spawns.extend(staged);
        Ok(())
    }

    /// Despawns an entity by id (applied after the callback returns).
    fn despawn(&self, id: u64) -> PyResult<()> {
        let entity = entity_from_bits(id)?;
        self.writes.borrow_mut().despawns.push(entity);
        Ok(())
    }

    /// Switches to another scene: every current entity goes and the staged
    /// `Scene`'s entities replace them, after the callback returns. Other
    /// writes to this frame's entities are dropped; entities spawned this
    /// frame land in the new scene. `Game.load_scene` calls this while the
    /// game is running.
    fn load(&self, scene: &PyScene) {
        self.writes.borrow_mut().load = Some(scene.staged_entities().to_vec());
    }
}

/// A handle to one entity inside [`SceneView`].
#[pyclass(name = "EntityView", unsendable, module = "automata._core")]
#[derive(Debug, Clone)]
pub struct EntityView {
    id: u64,
    name: Option<String>,
    transform: Option<Transform>,
    character_velocity: Option<CharacterVelocity>,
    camera_3d: Option<Camera3D>,
    attributes: AttributeMap,
    script: Option<String>,
    audio_source: Option<automata_core::AudioSource>,
    writes: WriteQueue,
}

impl EntityView {
    fn from_snapshot(snapshot: &EntitySnapshot, writes: WriteQueue) -> Self {
        Self {
            id: snapshot.id.to_bits().get(),
            name: snapshot.name.clone(),
            transform: snapshot.transform,
            character_velocity: snapshot.character_velocity,
            camera_3d: snapshot.camera_3d,
            attributes: snapshot.attributes.clone(),
            script: snapshot.script.clone(),
            audio_source: snapshot.audio_source.clone(),
            writes,
        }
    }

    /// The entity's bit representation (for the physics views).
    pub(crate) fn id_bits(&self) -> u64 {
        self.id
    }
}

#[pymethods]
impl EntityView {
    /// The entity's stable id (valid while the entity is alive).
    #[getter]
    fn id(&self) -> u64 {
        self.id
    }

    /// The entity's name, if any.
    #[getter]
    fn name(&self) -> Option<String> {
        self.name.clone()
    }

    /// The entity's local transform at the start of the frame, if any.
    ///
    /// The returned [`TransformRef`] is live: attribute writes on it buffer
    /// engine commands (`view.transform.position = (1, 0, 0)`).
    #[getter]
    fn transform(&self) -> Option<TransformRef> {
        self.transform
            .map(|t| TransformRef::new(self.id, t, Rc::clone(&self.writes)))
    }

    /// Queues a new local transform for the entity (applied after the
    /// callback returns). Accepts a `Transform` or a `TransformRef`.
    #[setter]
    fn set_transform(&self, transform: &Bound<'_, PyAny>) -> PyResult<()> {
        let transform = extract_transform(transform)?;
        self.writes
            .borrow_mut()
            .transforms
            .insert(self.id, transform);
        Ok(())
    }

    /// The entity's character-controller velocity at the start of the
    /// frame, or `None` when the entity has no `CharacterVelocity`.
    #[getter]
    fn character_velocity(&self) -> Option<(f32, f32, f32)> {
        self.character_velocity
            .map(|v| (v.velocity.x, v.velocity.y, v.velocity.z))
    }

    /// Queues a character-controller velocity for the entity (applied after
    /// the callback returns). Accepts a `(vx, vy, vz)` tuple or a
    /// `CharacterVelocity`. This is the per-frame drive input for kinematic
    /// characters: set it every frame from the action map.
    #[setter]
    fn set_character_velocity(&self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let velocity = if let Ok(tuple) = value.extract::<(f32, f32, f32)>() {
            CharacterVelocity::new(glam::Vec3::new(tuple.0, tuple.1, tuple.2))
        } else if let Ok(c) = value.cast::<crate::physics_components::PyCharacterVelocity>() {
            c.borrow().inner()
        } else {
            let typename = value.get_type().name()?.to_string();
            return Err(PyTypeError::new_err(format!(
                "expected a (vx, vy, vz) tuple or CharacterVelocity, got {typename}"
            )));
        };
        self.writes
            .borrow_mut()
            .character_velocities
            .insert(self.id, velocity);
        Ok(())
    }

    /// The entity's 3D camera at the start of the frame, or `None` when
    /// the entity has no `Camera3D`.
    #[getter]
    fn camera_3d(&self) -> Option<PyCamera3D> {
        self.camera_3d.map(PyCamera3D::from_inner)
    }

    /// Queues a new 3D camera for the entity (applied after the callback
    /// returns). Accepts a `Camera3D`; this is how follow cameras move:
    /// set it every frame from the followed entity's position.
    #[setter]
    fn set_camera_3d(&self, camera: &Bound<'_, PyAny>) -> PyResult<()> {
        let camera = if let Ok(c) = camera.cast::<PyCamera3D>() {
            c.borrow().inner()
        } else {
            let typename = camera.get_type().name()?.to_string();
            return Err(PyTypeError::new_err(format!(
                "expected a Camera3D, got {typename}"
            )));
        };
        self.writes.borrow_mut().cameras_3d.insert(self.id, camera);
        Ok(())
    }

    /// The entity's custom attributes, used like a dict. Reads see the
    /// start-of-frame values plus this frame's writes; writes apply after
    /// the callback returns.
    #[getter]
    fn attributes(&self) -> PyAttributes {
        PyAttributes::live(self.id, self.attributes.clone(), Rc::clone(&self.writes))
    }

    /// Path of the behaviour script attached in Oberon, or `None`.
    #[getter]
    fn script(&self) -> Option<String> {
        self.script.clone()
    }

    /// The entity's `AudioSource`, or `None`. Play it with
    /// `ctx.audio.play_source(view)`.
    #[getter]
    fn audio_source(&self) -> Option<crate::components::PyAudioSource> {
        self.audio_source
            .clone()
            .map(crate::components::PyAudioSource::from_inner)
    }

    /// Queues the entity (and its children) for despawn after the callback.
    fn despawn(&self) -> PyResult<()> {
        let entity = entity_from_bits(self.id)?;
        self.writes.borrow_mut().despawns.push(entity);
        Ok(())
    }

    fn __repr__(&self) -> String {
        match &self.name {
            Some(name) => format!("EntityView(id={}, name={name:?})", self.id),
            None => format!("EntityView(id={})", self.id),
        }
    }
}

/// Extracts an engine transform from a `Transform` or `TransformRef`.
fn extract_transform(obj: &Bound<'_, PyAny>) -> PyResult<Transform> {
    if let Ok(t) = obj.cast::<PyTransform>() {
        return Ok(t.borrow().inner());
    }
    if let Ok(t) = obj.cast::<TransformRef>() {
        return Ok(t.borrow().current());
    }
    let typename = obj.get_type().name()?.to_string();
    Err(PyTypeError::new_err(format!(
        "expected a Transform or TransformRef, got {typename}"
    )))
}

/// Reconstructs an entity id from its bit representation.
pub(crate) fn entity_from_bits(bits: u64) -> PyResult<EntityId> {
    EntityId::from_bits(bits)
        .ok_or_else(|| PyValueError::new_err(format!("invalid entity id {bits}")))
}

/// A live handle to an entity's local transform.
///
/// Reads see pending writes made earlier in the same frame, falling back to
/// the start-of-frame value; attribute writes
/// (`tref.position = (1, 0, 0)`) compose field-by-field into a pending
/// transform that the engine applies right after the update callback
/// returns.
#[pyclass(name = "TransformRef", unsendable, module = "automata._core")]
#[derive(Debug, Clone)]
pub struct TransformRef {
    id: u64,
    base: Transform,
    writes: WriteQueue,
}

impl TransformRef {
    fn new(id: u64, base: Transform, writes: WriteQueue) -> Self {
        Self { id, base, writes }
    }

    /// The pending transform if a write already targeted this entity this
    /// frame, else the start-of-frame value.
    fn current(&self) -> Transform {
        self.writes
            .borrow()
            .transforms
            .get(&self.id)
            .copied()
            .unwrap_or(self.base)
    }

    /// Updates one field of the pending transform.
    fn update(&mut self, f: impl FnOnce(&mut Transform)) {
        let mut writes = self.writes.borrow_mut();
        let entry = writes.transforms.entry(self.id).or_insert(self.base);
        f(entry);
    }
}

#[pymethods]
impl TransformRef {
    /// The `(x, y, z)` position relative to the parent.
    #[getter]
    fn position(&self) -> (f32, f32, f32) {
        let p = self.current().position;
        (p.x, p.y, p.z)
    }

    #[setter]
    fn set_position(&mut self, position: (f32, f32, f32)) {
        self.update(|t| t.position = glam::Vec3::new(position.0, position.1, position.2));
    }

    /// The rotation as Euler angles `(x, y, z)` in radians.
    #[getter]
    fn rotation_euler(&self) -> (f32, f32, f32) {
        self.current().rotation.to_euler(glam::EulerRot::XYZ)
    }

    #[setter]
    fn set_rotation_euler(&mut self, rotation: (f32, f32, f32)) {
        self.update(|t| {
            t.rotation =
                glam::Quat::from_euler(glam::EulerRot::XYZ, rotation.0, rotation.1, rotation.2);
        });
    }

    /// The `(x, y, z)` scale relative to the parent.
    #[getter]
    fn scale(&self) -> (f32, f32, f32) {
        let s = self.current().scale;
        (s.x, s.y, s.z)
    }

    #[setter]
    fn set_scale(&mut self, scale: (f32, f32, f32)) {
        self.update(|t| t.scale = glam::Vec3::new(scale.0, scale.1, scale.2));
    }

    /// Replaces the whole transform (applied after the callback returns).
    fn set(&mut self, transform: &Bound<'_, PyAny>) -> PyResult<()> {
        let value = extract_transform(transform)?;
        self.update(|t| *t = value);
        Ok(())
    }

    /// A detached [`Transform`] copy of the current value.
    fn copy(&self) -> PyTransform {
        PyTransform::from_inner(self.current())
    }

    fn __repr__(&self) -> String {
        let p = self.current().position;
        format!(
            "TransformRef(id={}, position=({:.3}, {:.3}, {:.3}))",
            self.id, p.x, p.y, p.z
        )
    }
}
