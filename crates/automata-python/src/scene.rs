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
use std::rc::Rc;

use automata_scene::{EntityId, Name, Scene, Transform};
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;

use crate::components::{extract_component, PyTransform, StagedComponent};

/// A staged entity: a name plus the components to insert.
#[derive(Debug, Clone, Default)]
pub struct StagedEntity {
    /// Optional name for later lookup.
    pub name: Option<String>,
    /// Components to insert for the entity.
    pub components: Vec<StagedComponent>,
}

/// A game object builder. Attach components with [`Entity.add`], then place
/// it in a scene with [`Scene.add`].
#[pyclass(name = "Entity", module = "automata._core")]
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
                components: Vec::new(),
            },
        }
    }

    /// The entity's name, if any.
    #[getter]
    fn name(&self) -> Option<String> {
        self.staged.name.clone()
    }

    /// Attaches a component (`Transform`, `Camera3D`, `Camera2D`).
    fn add(&mut self, component: &Bound<'_, PyAny>) -> PyResult<()> {
        self.staged.components.push(extract_component(component)?);
        Ok(())
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
/// per-entity FFI traffic at startup.
#[pyclass(name = "Scene", module = "automata._core")]
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

    /// Stages an entity. The builder is copied; later edits to it do not
    /// affect the scene.
    fn add(&mut self, entity: &PyEntity) {
        self.entities.push(entity.staged.clone());
    }

    /// The number of staged entities.
    #[getter]
    fn entity_count(&self) -> usize {
        self.entities.len()
    }

    fn __len__(&self) -> usize {
        self.entities.len()
    }
}

impl PyScene {
    /// The staged entities.
    pub fn entities(&self) -> &[StagedEntity] {
        &self.entities
    }
}

/// Replaces the contents of an ECS scene with staged entities.
pub fn load_staged(scene: &mut Scene, staged: &[StagedEntity]) {
    scene.world_mut().clear();
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
        }
    }
    id
}

/// A buffered scene write. Commands are recorded during the Python update
/// callback and applied by the engine immediately after it returns.
#[derive(Debug)]
pub enum SceneCommand {
    /// Replace an entity's local transform.
    SetTransform {
        /// Target entity.
        entity: EntityId,
        /// New local transform.
        transform: Transform,
    },
    /// Spawn a staged entity.
    Spawn(StagedEntity),
    /// Despawn an entity and its children.
    Despawn(EntityId),
}

/// Shared command buffer between the per-frame views and the game loop.
pub type CommandQueue = Rc<RefCell<Vec<SceneCommand>>>;

/// Applies buffered commands to the ECS scene.
pub fn apply_commands(scene: &mut Scene, commands: &CommandQueue) {
    let drained: Vec<SceneCommand> = commands.borrow_mut().drain(..).collect();
    for command in drained {
        match command {
            SceneCommand::SetTransform { entity, transform } => {
                if let Ok(mut current) = scene.world_mut().get::<&mut Transform>(entity) {
                    *current = transform;
                }
            }
            SceneCommand::Spawn(staged) => {
                spawn_staged(scene, &staged);
            }
            SceneCommand::Despawn(entity) => {
                if scene.world().contains(entity) {
                    scene.despawn_recursive(entity);
                }
            }
        }
    }
}

/// One entity's start-of-frame state, as seen by the update callback.
#[derive(Debug, Clone)]
struct EntitySnapshot {
    id: EntityId,
    name: Option<String>,
    transform: Option<Transform>,
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
    commands: CommandQueue,
}

impl SceneView {
    /// Builds a view over the scene's current state.
    pub fn snapshot(scene: &Scene, commands: CommandQueue) -> Self {
        let snapshot = scene
            .world()
            .query::<(EntityId, Option<&Name>, Option<&Transform>)>()
            .iter()
            .map(|(id, name, transform)| EntitySnapshot {
                id,
                name: name.map(|n| n.0.clone()),
                transform: transform.copied(),
            })
            .collect();
        Self { snapshot, commands }
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
            .map(|e| EntityView::from_snapshot(e, Rc::clone(&self.commands)))
    }

    /// The names of all named entities at the start of the frame.
    fn names(&self) -> Vec<String> {
        self.snapshot
            .iter()
            .filter_map(|e| e.name.clone())
            .collect()
    }

    /// Spawns a staged entity (applied after the callback returns; look it
    /// up by name from the next frame onward).
    fn spawn(&self, entity: &PyEntity) {
        self.commands
            .borrow_mut()
            .push(SceneCommand::Spawn(entity.staged.clone()));
    }

    /// Despawns an entity by id (applied after the callback returns).
    fn despawn(&self, id: u64) -> PyResult<()> {
        let entity = entity_from_bits(id)?;
        self.commands
            .borrow_mut()
            .push(SceneCommand::Despawn(entity));
        Ok(())
    }
}

/// A handle to one entity inside [`SceneView`].
#[pyclass(name = "EntityView", unsendable, module = "automata._core")]
#[derive(Debug, Clone)]
pub struct EntityView {
    id: u64,
    name: Option<String>,
    transform: Option<Transform>,
    commands: CommandQueue,
}

impl EntityView {
    fn from_snapshot(snapshot: &EntitySnapshot, commands: CommandQueue) -> Self {
        Self {
            id: snapshot.id.to_bits().get(),
            name: snapshot.name.clone(),
            transform: snapshot.transform,
            commands,
        }
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
    #[getter]
    fn transform(&self) -> Option<PyTransform> {
        self.transform.map(PyTransform::from_inner)
    }

    /// Queues a new local transform for the entity (applied after the
    /// callback returns).
    #[setter]
    fn set_transform(&self, transform: &Bound<'_, PyAny>) -> PyResult<()> {
        let entity = entity_from_bits(self.id)?;
        let transform = transform
            .cast::<PyTransform>()
            .map_err(|_| PyTypeError::new_err("expected a Transform"))?
            .get()
            .inner();
        self.commands
            .borrow_mut()
            .push(SceneCommand::SetTransform { entity, transform });
        Ok(())
    }

    /// Queues the entity (and its children) for despawn after the callback.
    fn despawn(&self) -> PyResult<()> {
        let entity = entity_from_bits(self.id)?;
        self.commands
            .borrow_mut()
            .push(SceneCommand::Despawn(entity));
        Ok(())
    }

    fn __repr__(&self) -> String {
        match &self.name {
            Some(name) => format!("EntityView(id={}, name={name:?})", self.id),
            None => format!("EntityView(id={})", self.id),
        }
    }
}

/// Reconstructs an entity id from its bit representation.
fn entity_from_bits(bits: u64) -> PyResult<EntityId> {
    EntityId::from_bits(bits)
        .ok_or_else(|| PyValueError::new_err(format!("invalid entity id {bits}")))
}
