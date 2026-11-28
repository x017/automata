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
    /// The prefab file this entity is an instance of, if any (kept so a
    /// saved scene stays linked; the game itself ignores it).
    pub prefab: Option<automata_core::PrefabLink>,
}

/// A game object builder. Attach components with [`Entity.add`], then place
/// it in a scene with [`Scene.add`]. Methods chain:
/// `Entity("cube").add(Transform())`.
/// Refuses entity names with `:`, the separator in script keys.
fn check_name(name: Option<&str>) -> PyResult<()> {
    match name {
        Some(n) if n.contains(':') => Err(pyo3::exceptions::PyValueError::new_err(format!(
            "entity names can't contain ':' ({n:?}): it separates the parts of a key \
             like \"Player:animation:clip\""
        ))),
        _ => Ok(()),
    }
}

#[pyclass(name = "Entity", module = "automata._core", subclass)]
#[derive(Debug, Clone, Default)]
pub struct PyEntity {
    staged: StagedEntity,
}

#[pymethods]
impl PyEntity {
    /// Creates an entity with an optional name for later lookup. Names
    /// can't contain `:`, which separates the parts of a script key
    /// (`"Player:animation:clip"`).
    #[new]
    #[pyo3(signature = (name=None))]
    fn new(name: Option<String>) -> PyResult<Self> {
        check_name(name.as_deref())?;
        Ok(Self {
            staged: StagedEntity {
                name,
                ..StagedEntity::default()
            },
        })
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

    /// The prefab file this entity is an instance of (relative to the
    /// asset root), or `None`. Instances from `Prefab.load(...)` are
    /// linked: a saved scene keeps following the file, as in Oberon. Set
    /// `None` to unlink, or a path to link.
    #[getter]
    fn prefab(&self) -> Option<String> {
        self.staged.prefab.as_ref().map(|l| l.path.clone())
    }

    #[setter]
    fn set_prefab(&mut self, path: Option<String>) {
        self.staged.prefab = path.map(automata_core::PrefabLink::new);
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
    fn set_name(&mut self, name: Option<String>) -> PyResult<()> {
        check_name(name.as_deref())?;
        self.staged.name = name;
        Ok(())
    }

    /// Attaches a component (`Transform`, `MeshRenderer`, a light, …),
    /// replacing one of the same kind. Returns the entity so calls can
    /// chain.
    fn add<'py>(
        mut slf: PyRefMut<'py, Self>,
        component: &Bound<'_, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        // One of each kind: adding a second Transform (say) replaces the
        // first, so a copied entity can be re-dressed.
        let component = extract_component(component)?;
        let kind = std::mem::discriminant(&component);
        let staged = &mut slf.staged.components;
        match staged
            .iter_mut()
            .find(|c| std::mem::discriminant(*c) == kind)
        {
            Some(existing) => *existing = component,
            None => staged.push(component),
        }
        Ok(slf)
    }

    /// The staged components as fresh Python objects, for editors that need
    /// to inspect a scene's contents (Oberon). Each call rebuilds the list.
    #[allow(clippy::too_many_lines)] // one step per kind of component
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
                    StagedComponent::PointLight(v) => {
                        Py::new(py, crate::components::PyPointLight::from_inner(*v))?.into_any()
                    }
                    StagedComponent::SpotLight(v) => {
                        Py::new(py, crate::components::PySpotLight::from_inner(*v))?.into_any()
                    }
                    StagedComponent::Environment(v) => {
                        Py::new(py, crate::components::PyEnvironment::from_inner(*v))?.into_any()
                    }
                    StagedComponent::Water(v) => {
                        Py::new(py, crate::components::PyWater::from_inner(*v))?.into_any()
                    }
                    StagedComponent::PostShader(v) => {
                        Py::new(py, crate::components::PyPostShader::from_inner(v.clone()))?
                            .into_any()
                    }
                    StagedComponent::AudioSource(v) => {
                        Py::new(py, crate::components::PyAudioSource::from_inner(v.clone()))?
                            .into_any()
                    }
                    StagedComponent::ParticleEmitter(v) => Py::new(
                        py,
                        crate::particles::PyParticleEmitter::from_inner(v.clone()),
                    )?
                    .into_any(),
                    StagedComponent::Animation(v) => {
                        Py::new(py, crate::animation::PyAnimation::from_inner(v.clone()))?
                            .into_any()
                    }
                    StagedComponent::Grass(v) => {
                        Py::new(py, crate::foliage::PyGrass::from_inner(v.clone()))?.into_any()
                    }
                    StagedComponent::Scatter(v) => {
                        Py::new(py, crate::foliage::PyScatter::from_inner(v.clone()))?.into_any()
                    }
                    StagedComponent::Terrain(v) => {
                        Py::new(py, crate::terrain::PyTerrain::from_inner(v.clone()))?.into_any()
                    }
                    StagedComponent::Joint(v) => {
                        Py::new(py, crate::joints::PyJoint::from_inner(v.clone()))?.into_any()
                    }
                    StagedComponent::Attach(v) => {
                        Py::new(py, crate::animation::PyAttach { inner: v.clone() })?.into_any()
                    }
                    StagedComponent::NavAgent(v) => {
                        Py::new(py, crate::nav::PyNavAgent { inner: v.clone() })?.into_any()
                    }
                    StagedComponent::Navigation(v) => {
                        Py::new(py, crate::nav::PyNavigation { inner: v.clone() })?.into_any()
                    }
                    StagedComponent::NavLink(v) => {
                        Py::new(py, crate::nav::PyNavLink { inner: v.clone() })?.into_any()
                    }
                    StagedComponent::Ragdoll(v) => {
                        Py::new(py, crate::ragdoll::PyRagdoll { inner: v.clone() })?.into_any()
                    }
                    StagedComponent::Sprite(v) => {
                        Py::new(py, crate::sprite2d::PySprite { inner: v.clone() })?.into_any()
                    }
                    StagedComponent::Tilemap(v) => {
                        Py::new(py, crate::sprite2d::PyTilemap { inner: v.clone() })?.into_any()
                    }
                    StagedComponent::CameraFollow2D(v) => {
                        Py::new(py, crate::sprite2d::PyCameraFollow2D { inner: v.clone() })?
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
        // Model paths in the scene are relative to its folder (and so are
        // prefab links, whose instances follow their prefab files).
        if let Ok(abs) = std::path::absolute(path) {
            automata_assets::set_root(abs.parent());
        }
        let _ = cls;
        let entities =
            crate::serialize::scene_from_file_text(&text, &crate::serialize::link_root())
                .map_err(|err| pyo3::exceptions::PyValueError::new_err(err.to_string()))?;
        Ok(Self { entities })
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
#[allow(clippy::too_many_lines)] // one step per kind of component
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
            StagedComponent::PointLight(c) => {
                let _ = scene.world_mut().insert_one(id, *c);
            }
            StagedComponent::SpotLight(c) => {
                let _ = scene.world_mut().insert_one(id, *c);
            }
            StagedComponent::ParticleEmitter(c) => {
                let _ = scene.world_mut().insert_one(id, c.clone());
            }
            StagedComponent::Environment(c) => drop(scene.world_mut().insert_one(id, *c)),
            StagedComponent::Water(c) => drop(scene.world_mut().insert_one(id, *c)),
            StagedComponent::PostShader(c) => drop(scene.world_mut().insert_one(id, c.clone())),
            StagedComponent::AudioSource(c) => {
                let _ = scene.world_mut().insert_one(id, c.clone());
            }
            StagedComponent::Animation(c) => {
                let _ = scene.world_mut().insert_one(id, c.clone());
            }
            StagedComponent::Grass(c) => drop(scene.world_mut().insert_one(id, c.clone())),
            StagedComponent::Scatter(c) => drop(scene.world_mut().insert_one(id, c.clone())),
            StagedComponent::Terrain(c) => drop(scene.world_mut().insert_one(id, c.clone())),
            StagedComponent::Joint(c) => {
                let _ = scene.world_mut().insert_one(id, c.clone());
            }
            StagedComponent::Attach(a) => {
                let offset = staged
                    .components
                    .iter()
                    .find_map(|c| match c {
                        StagedComponent::Transform(t) => Some(*t),
                        _ => None,
                    })
                    .unwrap_or_default();
                let attachment = automata_scene::Attachment::new(&a.to, &a.bone, offset);
                let _ = scene.world_mut().insert_one(id, attachment);
            }
            StagedComponent::NavAgent(c) => {
                let _ = scene.world_mut().insert_one(id, c.clone());
            }
            StagedComponent::Navigation(c) => {
                let _ = scene.world_mut().insert_one(id, c.clone());
            }
            StagedComponent::NavLink(c) => {
                let _ = scene.world_mut().insert_one(id, c.clone());
            }
            StagedComponent::Ragdoll(c) => {
                let _ = scene.world_mut().insert_one(id, c.sanitized());
            }
            StagedComponent::Sprite(c) => drop(scene.world_mut().insert_one(id, c.clone())),
            StagedComponent::Tilemap(c) => drop(scene.world_mut().insert_one(id, c.clone())),
            StagedComponent::CameraFollow2D(c) => {
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
    /// One-off particle effects to spawn: emitter, position, direction,
    /// particle count.
    pub bursts: Vec<(
        automata_scene::ParticleEmitter,
        glam::Vec3,
        Option<glam::Vec3>,
        Option<u32>,
    )>,
    /// Extra particles from entities' own emitters.
    pub emitter_bursts: Vec<(EntityId, u32)>,
    /// Animation requests (play, stop, seek, speed), in call order.
    pub animations: Vec<(u64, automata_scene::AnimationCommand)>,
    /// Changes to entities' joints, in call order.
    pub joints: Vec<(u64, automata_physics::JointEdit)>,
    /// Attaching entities to bones (`None` detaches), in call order.
    pub attachments: Vec<(u64, Option<automata_scene::Attachment>)>,
    /// Changes to nav agents (targets, speeds), in call order.
    pub navs: Vec<(u64, automata_nav::NavEdit)>,
    /// Ragdolls switched on or off, retuned and pushed, in call order.
    pub ragdolls: Vec<(u64, automata_scene::RagdollEdit)>,
    /// Sprite clips played and sprites flipped or retinted, in call order.
    pub sprites: Vec<(u64, automata_scene::SpriteEdit)>,
}

/// Shared write buffer between the per-frame views and the game loop.
pub type WriteQueue = Rc<RefCell<FrameWrites>>;

/// Applies buffered writes to the ECS scene.
#[allow(clippy::too_many_lines)] // one step per kind of component
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
    for (emitter, position, direction, count) in drained.bursts {
        automata_scene::spawn_burst(scene.world_mut(), emitter, position, direction, count);
    }
    for (entity, count) in drained.emitter_bursts {
        automata_scene::burst(scene.world_mut(), entity, count);
    }
    for (bits, command) in drained.animations {
        if let Some(entity) = EntityId::from_bits(bits) {
            automata_scene::command_animation(scene.world_mut(), entity, &command);
        }
    }
    for (bits, edit) in drained.joints {
        if let Some(entity) = EntityId::from_bits(bits) {
            if let Ok(mut joint) = scene
                .world_mut()
                .get::<&mut automata_physics::Joint>(entity)
            {
                edit.apply(&mut joint);
            }
        }
    }
    for (bits, attachment) in drained.attachments {
        if let Some(entity) = EntityId::from_bits(bits) {
            let world = scene.world_mut();
            match attachment {
                Some(a) if world.contains(entity) => {
                    if world.get::<&Transform>(entity).is_err() {
                        let _ = world.insert_one(entity, Transform::IDENTITY);
                    }
                    let _ = world.insert_one(entity, a);
                }
                Some(_) => {}
                None => {
                    let _ = world.remove_one::<automata_scene::Attachment>(entity);
                }
            }
        }
    }
    for (bits, edit) in drained.navs {
        if let Some(entity) = EntityId::from_bits(bits) {
            if let Ok(mut agent) = scene.world_mut().get::<&mut automata_nav::NavAgent>(entity) {
                edit.apply(&mut agent);
            }
        }
    }
    for (bits, edit) in drained.ragdolls {
        if let Some(entity) = EntityId::from_bits(bits) {
            edit.apply(scene.world_mut(), entity);
        }
    }
    for (bits, edit) in drained.sprites {
        if let Some(entity) = EntityId::from_bits(bits) {
            if let Ok(mut sprite) = scene
                .world_mut()
                .get::<&mut automata_scene::Sprite2D>(entity)
            {
                edit.apply(&mut sprite);
            }
        }
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
    animation: Option<automata_scene::AnimationState>,
    joint: Option<(automata_physics::Joint, automata_physics::JointState)>,
    attached: Option<(String, String)>,
    nav_agent: Option<automata_nav::NavAgent>,
    ragdoll: Option<automata_scene::RagdollStatus>,
    sprite: Option<automata_scene::Sprite2D>,
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
        let world = scene.world();
        let snapshot = world
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
                        animation: automata_scene::animation_state(world, id),
                        joint: world.get::<&automata_physics::Joint>(id).ok().map(|j| {
                            let state = world
                                .get::<&automata_physics::JointState>(id)
                                .map(|s| *s)
                                .unwrap_or_default();
                            ((*j).clone(), state)
                        }),
                        attached: world
                            .get::<&automata_scene::Attachment>(id)
                            .ok()
                            .map(|a| (a.to.clone(), a.bone.clone())),
                        nav_agent: world
                            .get::<&automata_nav::NavAgent>(id)
                            .ok()
                            .map(|a| (*a).clone()),
                        ragdoll: automata_scene::ragdoll_status(world, id),
                        sprite: world
                            .get::<&automata_scene::Sprite2D>(id)
                            .ok()
                            .map(|s| (*s).clone()),
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

    /// A one-off particle effect at `position` (applied after the
    /// callback returns): `emitter` is a preset name ("sparks") or an
    /// `at.ParticleEmitter`, its up axis turned to `direction`, making
    /// `count` particles (default: its own `burst`, or a second of its
    /// `rate`). It cleans up after
    /// itself.
    #[pyo3(signature = (emitter, position, direction=None, count=None))]
    fn burst(
        &self,
        emitter: &Bound<'_, PyAny>,
        position: (f32, f32, f32),
        direction: Option<(f32, f32, f32)>,
        count: Option<u32>,
    ) -> PyResult<()> {
        let emitter = crate::particles::emitter_from(emitter)?;
        self.writes.borrow_mut().bursts.push((
            emitter,
            position.into(),
            direction.map(Into::into),
            count,
        ));
        Ok(())
    }

    /// Makes `count` extra particles from entity `id`'s own emitter next
    /// frame (nothing if it has none).
    fn burst_emitter(&self, id: u64, count: u32) -> PyResult<()> {
        let entity = entity_from_bits(id)?;
        self.writes
            .borrow_mut()
            .emitter_bursts
            .push((entity, count));
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
    animation: Option<automata_scene::AnimationState>,
    joint: Option<(automata_physics::Joint, automata_physics::JointState)>,
    attached: Option<(String, String)>,
    nav_agent: Option<automata_nav::NavAgent>,
    ragdoll: Option<automata_scene::RagdollStatus>,
    sprite: Option<automata_scene::Sprite2D>,
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
            animation: snapshot.animation.clone(),
            joint: snapshot.joint.clone(),
            attached: snapshot.attached.clone(),
            nav_agent: snapshot.nav_agent.clone(),
            ragdoll: snapshot.ragdoll.clone(),
            sprite: snapshot.sprite.clone(),
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

    /// Queues a whole new attribute map (a dict), applied after the
    /// callback returns.
    #[setter]
    fn set_attributes(&self, values: &Bound<'_, PyAny>) -> PyResult<()> {
        let map = map_from_py(values)?;
        self.writes.borrow_mut().attributes.insert(self.id, map);
        Ok(())
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

    /// The entity's joint (`angle`, `broken`, `force`, `enabled`,
    /// `motor_speed`, `motor_target`, ...), or `None` without a Joint.
    #[getter]
    fn joint(&self) -> Option<crate::joints::PyJointRef> {
        self.joint.clone().map(|(joint, state)| {
            crate::joints::PyJointRef::new(self.id, joint, state, Rc::clone(&self.writes))
        })
    }

    /// The entity's animation (`play`, `stop`, `clip`, `time`, ...), or
    /// `None` when its mesh isn't an animated model.
    #[getter]
    fn animation(&self) -> Option<crate::animation::PyAnimationRef> {
        self.animation.clone().map(|state| {
            crate::animation::PyAnimationRef::new(self.id, state, Rc::clone(&self.writes))
        })
    }

    /// Attaches the entity to `bone` of the entity named `to` (no bone:
    /// the entity itself), sitting at `offset` (a `Transform`, default
    /// none) in the bone's space: a sword picked up into a hand. Applied
    /// after the callback returns; from then on it follows the bone.
    #[pyo3(signature = (to, bone="", offset=None))]
    fn attach(&self, to: &str, bone: &str, offset: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
        let offset = offset
            .map(extract_transform)
            .transpose()?
            .unwrap_or_default();
        self.writes.borrow_mut().attachments.push((
            self.id,
            Some(automata_scene::Attachment::new(to, bone, offset)),
        ));
        Ok(())
    }

    /// Stops following a bone, staying where it is (applied after the
    /// callback returns).
    fn detach(&self) {
        self.writes.borrow_mut().attachments.push((self.id, None));
    }

    /// What the entity follows: `(entity name, bone)`, or `None`.
    #[getter]
    fn attached_to(&self) -> Option<(String, String)> {
        self.attached.clone()
    }

    /// The nav agent's start-of-frame state as a dict (`t` target, `spd`
    /// speed, `stop` stop distance, `arr` arrived, `jmp` jumping, `vel`
    /// velocity, `path` the corners still ahead), or `None` without a
    /// `NavAgent`. Behaviour scripts read it through `entity.nav`.
    #[getter]
    fn nav_state<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, pyo3::types::PyDict>>> {
        self.nav_agent
            .as_ref()
            .map(|a| crate::nav::agent_state(py, a))
            .transpose()
    }

    /// Changes the nav agent (applied after the callback returns):
    /// `go` sends it to a point, an entity's name, or (`None`) nowhere;
    /// `speed`, `stop_distance` and `hover` change those. Arguments left out stay
    /// as they are. Behaviour scripts use `entity.nav.go(...)`.
    #[pyo3(signature = (**changes))]
    fn nav_edit(&self, changes: Option<&Bound<'_, pyo3::types::PyDict>>) -> PyResult<()> {
        let mut edit = automata_nav::NavEdit::default();
        if let Some(changes) = changes {
            for (key, value) in changes.iter() {
                match key.extract::<String>()?.as_str() {
                    "go" => edit.target = Some(crate::nav::extract_target(&value)?),
                    "speed" => edit.speed = Some(value.extract()?),
                    "stop_distance" => edit.stop_distance = Some(value.extract()?),
                    "hover" => edit.hover = Some(value.extract()?),
                    other => {
                        return Err(pyo3::exceptions::PyTypeError::new_err(format!(
                            "nav_edit() got an unexpected keyword {other:?}"
                        )))
                    }
                }
            }
        }
        self.writes.borrow_mut().navs.push((self.id, edit));
        Ok(())
    }

    /// The ragdoll's start-of-frame state as a dict (`on` switched on,
    /// `mass`, `stiff` stiffness, `blend` blend-back time, `limp` physics
    /// has it, `w` its weight over the animation, `rest` limp and still,
    /// `hips` where the hips are or `None`), or `None` without a
    /// `Ragdoll`. Behaviour scripts read it through `entity.ragdoll`.
    #[getter]
    fn ragdoll_state<'py>(
        &self,
        py: Python<'py>,
    ) -> PyResult<Option<Bound<'py, pyo3::types::PyDict>>> {
        self.ragdoll
            .as_ref()
            .map(|s| crate::ragdoll::status_dict(py, s))
            .transpose()
    }

    /// Changes the ragdoll (applied after the callback returns): `on`
    /// switches it, `mass` / `stiffness` / `blend_time` retune it, `push`
    /// is `(impulse, bone)` (bone `None`: the whole body) for the next
    /// physics step. Behaviour scripts use `entity.ragdoll.on()` and the
    /// like.
    #[pyo3(signature = (**changes))]
    fn ragdoll_edit(&self, changes: Option<&Bound<'_, pyo3::types::PyDict>>) -> PyResult<()> {
        let mut edit = automata_scene::RagdollEdit::default();
        if let Some(changes) = changes {
            for (key, value) in changes.iter() {
                match key.extract::<String>()?.as_str() {
                    "on" => edit.active = Some(value.extract()?),
                    "mass" => edit.mass = Some(value.extract()?),
                    "stiffness" => edit.stiffness = Some(value.extract()?),
                    "blend_time" => edit.blend_time = Some(value.extract()?),
                    "push" => {
                        let (impulse, bone): ((f32, f32, f32), Option<String>) = value.extract()?;
                        edit.pushes.push(automata_scene::RagdollPush {
                            bone,
                            impulse: glam::Vec3::new(impulse.0, impulse.1, impulse.2),
                        });
                    }
                    other => {
                        return Err(pyo3::exceptions::PyTypeError::new_err(format!(
                            "ragdoll_edit() got an unexpected keyword {other:?}"
                        )))
                    }
                }
            }
        }
        self.writes.borrow_mut().ragdolls.push((self.id, edit));
        Ok(())
    }

    /// The sprite's start-of-frame state as a dict (`clip` playing or
    /// `None`, `frame`, `done` a one-shot clip ended, `clips` names, `fx` /
    /// `fy` flips, `layer`, `color`, `image`, `size`), or `None` without a
    /// `Sprite`. Behaviour scripts read it through `entity.sprite`.
    #[getter]
    fn sprite_state<'py>(
        &self,
        py: Python<'py>,
    ) -> PyResult<Option<Bound<'py, pyo3::types::PyDict>>> {
        self.sprite
            .as_ref()
            .map(|s| crate::sprite2d::state_dict(py, s))
            .transpose()
    }

    /// Changes the sprite (applied after the callback returns): `play` is
    /// `(clip, restart)`, `stop`, `frame`, `flip_x`, `flip_y`, `color`,
    /// `layer`, `image`, `size`. Behaviour scripts use
    /// `entity.sprite.play("run")` and the like.
    #[pyo3(signature = (**changes))]
    fn sprite_edit(&self, changes: Option<&Bound<'_, pyo3::types::PyDict>>) -> PyResult<()> {
        let edit = crate::sprite2d::edit_from(changes)?;
        self.writes.borrow_mut().sprites.push((self.id, edit));
        Ok(())
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
