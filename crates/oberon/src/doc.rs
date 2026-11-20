//! The Oberon scene document: the editor's model of the scene being edited.
//!
//! A document is a [`SceneFile`] (the engine's serde RON data) plus editor
//! state: stable per-entity ids, a selection, dirty tracking, and an undo
//! stack of serialized snapshots. All editing goes through methods that
//! record undo and mark the document dirty, so the UI never touches the
//! file data directly.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use automata_core::{EntityData, MeshVisualData, PrimitiveData, SceneFile, TransformData};

/// The maximum number of undo snapshots retained.
const UNDO_LIMIT: usize = 100;

/// One entity as the editor sees it: a stable id plus its data.
#[derive(Debug, Clone)]
pub struct EntityEntry {
    /// Stable editor id (survives list rebuilds; not the ECS id).
    pub id: u64,
    /// The entity's editable data.
    pub data: EntityData,
}

impl EntityEntry {
    /// The display name (or a placeholder for unnamed entities).
    #[must_use]
    pub fn name(&self) -> &str {
        self.data.name.as_deref().unwrap_or("(unnamed)")
    }
}

/// An editable scene document.
#[derive(Debug)]
pub struct Document {
    /// The entities, in scene order.
    pub entities: Vec<EntityEntry>,
    next_id: u64,
    /// The on-disk path, if the document has been saved.
    pub path: Option<PathBuf>,
    /// The selected entity id, if any.
    pub selected: Option<u64>,
    dirty: bool,
    undo: Vec<String>,
    redo: Vec<String>,
    /// Monotonic counter bumped on every structural edit. The viewport
    /// compares it to rebuild the GPU world only when the scene changed,
    /// instead of clearing + respawning the ECS world (and re-uploading every
    /// mesh) every frame — which is what drove memory use up without bound.
    change_counter: u64,
    /// True while a move gesture is in progress (coalesces nudge undo).
    mid_gesture: bool,
}

/// The kinds of entity the Create menu can spawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateKind {
    /// An empty entity with only a transform.
    Empty,
    /// A unit cube with a mesh renderer.
    Cube,
    /// A flat 10×10 plane facing +Y.
    Plane,
    /// A directional (sun) light.
    Light,
    /// A perspective camera.
    Camera,
    /// A cube with a dynamic rigid body and a box collider.
    PhysicsCube,
}

impl CreateKind {
    /// Every kind, in menu order.
    pub const ALL: [Self; 6] = [
        Self::Empty,
        Self::Cube,
        Self::Plane,
        Self::Light,
        Self::Camera,
        Self::PhysicsCube,
    ];

    /// The menu label (also the base name of the new entity).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Empty => "Empty",
            Self::Cube => "Cube",
            Self::Plane => "Plane",
            Self::Light => "Directional Light",
            Self::Camera => "Camera",
            Self::PhysicsCube => "Physics Cube",
        }
    }
}

/// Components that can be added to / removed from an entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComponentKind {
    /// Primitive mesh visual.
    Mesh,
    /// Directional light.
    Light,
    /// Perspective camera.
    Camera,
    /// Rigid body.
    RigidBody,
    /// 3D collider.
    Collider,
}

impl ComponentKind {
    /// Every addable component, in menu order.
    pub const ALL: [Self; 5] = [
        Self::Mesh,
        Self::Light,
        Self::Camera,
        Self::RigidBody,
        Self::Collider,
    ];

    /// The display name.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Mesh => "Mesh Renderer",
            Self::Light => "Directional Light",
            Self::Camera => "Camera",
            Self::RigidBody => "Rigidbody",
            Self::Collider => "Collider",
        }
    }

    /// Whether `data` has this component.
    #[must_use]
    pub fn present_in(self, data: &EntityData) -> bool {
        match self {
            Self::Mesh => data.mesh_visual.is_some(),
            Self::Light => data.directional_light.is_some(),
            Self::Camera => data.camera_3d.is_some(),
            Self::RigidBody => data.rigid_body.is_some(),
            Self::Collider => data.collider_3d.is_some(),
        }
    }

    /// Adds a default instance of this component to `data` (no-op if present).
    pub fn add_to(self, data: &mut EntityData) {
        match self {
            Self::Mesh => {
                data.mesh_visual.get_or_insert(MeshVisualData {
                    primitive: PrimitiveData::Cube { size: 1.0 },
                    color: [0.85, 0.87, 0.92, 1.0],
                });
            }
            Self::Light => {
                data.directional_light
                    .get_or_insert_with(automata_scene::DirectionalLight::default);
            }
            Self::Camera => {
                data.camera_3d.get_or_insert_with(|| {
                    automata_renderer::Camera3D::new(
                        glam::Vec3::new(0.0, 2.0, 6.0),
                        glam::Vec3::ZERO,
                    )
                });
            }
            Self::RigidBody => {
                data.rigid_body
                    .get_or_insert_with(automata_physics::RigidBody::default);
            }
            Self::Collider => {
                // Fit the box to the visible mesh (physics ignores scale, so
                // the collider carries the world size).
                let size = match data.mesh_visual.as_ref().map(|m| &m.primitive) {
                    Some(PrimitiveData::Cube { size }) => *size,
                    _ => 1.0,
                };
                let scale = data.transform.map_or([1.0; 3], |t| t.scale);
                let ext = |i: usize| (size * scale[i].abs()).max(0.01);
                data.collider_3d.get_or_insert_with(|| {
                    automata_physics::Collider3D::cuboid(ext(0), ext(1), ext(2))
                });
            }
        }
    }

    /// Removes this component from `data`.
    pub fn remove_from(self, data: &mut EntityData) {
        match self {
            Self::Mesh => data.mesh_visual = None,
            Self::Light => data.directional_light = None,
            Self::Camera => data.camera_3d = None,
            Self::RigidBody => data.rigid_body = None,
            Self::Collider => data.collider_3d = None,
        }
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    /// An empty document with one default entity selected.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entities: Vec::new(),
            next_id: 1,
            path: None,
            selected: None,
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            change_counter: 0,
            mid_gesture: false,
        }
    }

    /// A starter scene (a sun, a ground plane, and a cube) for a fresh file.
    #[must_use]
    pub fn starter() -> Self {
        let mut doc = Self::new();
        let sun = doc.add_entity_named("Sun");
        doc.entities[sun].data.transform = Some(TransformData {
            position: [0.0, 6.0, 0.0],
            ..Default::default()
        });
        doc.entities[sun].data.directional_light = Some(automata_scene::DirectionalLight {
            direction: glam::Vec3::new(-0.4, -1.0, -0.3),
            color: [1.0, 0.96, 0.88],
            ambient: 0.35,
            shadows: true,
        });
        let ground = doc.add_entity_named("Ground");
        doc.entities[ground].data.transform = Some(TransformData {
            position: [0.0, -0.1, 0.0],
            scale: [20.0, 0.2, 20.0],
            ..Default::default()
        });
        doc.entities[ground].data.mesh_visual = Some(MeshVisualData {
            primitive: PrimitiveData::Cube { size: 1.0 },
            color: [0.16, 0.17, 0.20, 1.0],
        });
        // A static collider so physics objects land on the ground in play mode.
        doc.entities[ground].data.rigid_body = Some(automata_physics::RigidBody::fixed());
        doc.entities[ground].data.collider_3d =
            Some(automata_physics::Collider3D::cuboid(20.0, 0.2, 20.0));
        let cube = doc.add_entity_named("Cube");
        doc.entities[cube].data.transform = Some(TransformData {
            position: [0.0, 0.5, 0.0],
            ..Default::default()
        });
        doc.entities[cube].data.mesh_visual = Some(MeshVisualData {
            primitive: PrimitiveData::Cube { size: 1.0 },
            color: [0.62, 0.78, 1.0, 1.0],
        });
        doc.dirty = false;
        doc.undo.clear();
        doc
    }

    /// The number of entities.
    #[must_use]
    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    /// Whether the document has unsaved changes.
    #[must_use]
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// A counter bumped on every structural edit (add/remove/component
    /// change/undo/redo). Consumers that cache a derived scene (the viewport's
    /// GPU world) compare it to know when to rebuild, so an unchanged frame
    /// re-renders without touching the ECS world or re-uploading GPU meshes.
    #[must_use]
    pub fn change_counter(&self) -> u64 {
        self.change_counter
    }

    /// Records a structural edit: bumps the change counter and marks dirty.
    fn touch(&mut self) {
        self.change_counter += 1;
        self.dirty = true;
    }

    /// Finds an entity's index by id.
    #[must_use]
    pub fn index_of(&self, id: u64) -> Option<usize> {
        self.entities.iter().position(|e| e.id == id)
    }

    /// The selected entity's index, if it still exists.
    #[must_use]
    pub fn selected_index(&self) -> Option<usize> {
        self.selected.and_then(|id| self.index_of(id))
    }

    /// Selects an entity by id (`None` clears the selection).
    pub fn select(&mut self, id: Option<u64>) {
        self.selected = id;
    }

    /// A copy of the document for play mode: the same entities, ids,
    /// selection, path and dirty flag, without the undo history.
    #[must_use]
    pub fn play_copy(&self) -> Self {
        Self {
            entities: self.entities.clone(),
            next_id: self.next_id,
            path: self.path.clone(),
            selected: self.selected,
            dirty: self.dirty,
            undo: Vec::new(),
            redo: Vec::new(),
            change_counter: self.change_counter,
            mid_gesture: false,
        }
    }

    fn snapshot(&self) -> String {
        self.to_scene_file().to_ron().unwrap_or_default()
    }

    fn push_undo(&mut self) {
        // A discrete edit ends any in-progress move gesture, so the next
        // nudge snapshots fresh.
        self.mid_gesture = false;
        self.undo.push(self.snapshot());
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn unique_name(&self, base: &str) -> String {
        let existing: Vec<&str> = self.entities.iter().map(EntityEntry::name).collect();
        if !existing.contains(&base) {
            return base.to_string();
        }
        let mut i = 2;
        while existing.contains(&format!("{base} {i}").as_str()) {
            i += 1;
        }
        format!("{base} {i}")
    }

    /// Adds an entity with a default transform and selects it. Returns its index.
    pub fn add_entity(&mut self) -> usize {
        let name = self.unique_name("Entity");
        self.add_entity_named(&name)
    }

    /// Adds a named entity. Returns its index.
    pub fn add_entity_named(&mut self, name: &str) -> usize {
        self.push_undo();
        let id = self.alloc_id();
        let data = EntityData {
            name: Some(name.to_string()),
            transform: Some(TransformData::default()),
            ..Default::default()
        };
        self.entities.push(EntityEntry { id, data });
        self.selected = Some(id);
        self.touch();
        self.entities.len() - 1
    }

    /// Deletes the entity with `id`. Returns true when it existed.
    pub fn delete_entity(&mut self, id: u64) -> bool {
        let Some(idx) = self.index_of(id) else {
            return false;
        };
        self.push_undo();
        self.entities.remove(idx);
        if self.selected == Some(id) {
            self.selected = None;
        }
        self.touch();
        true
    }

    /// Renames the entity with `id` (records undo).
    pub fn rename_entity(&mut self, id: u64, name: &str) -> bool {
        if name.is_empty() {
            return false;
        }
        let Some(idx) = self.index_of(id) else {
            return false;
        };
        self.push_undo();
        self.entities[idx].data.name = Some(name.to_string());
        self.touch();
        true
    }

    /// Adds a mesh visual to the entity (one per entity).
    pub fn add_mesh(&mut self, id: u64, cube: bool) -> bool {
        let Some(idx) = self.index_of(id) else {
            return false;
        };
        if self.entities[idx].data.mesh_visual.is_some() {
            return false;
        }
        self.push_undo();
        self.entities[idx].data.mesh_visual = Some(MeshVisualData {
            primitive: if cube {
                PrimitiveData::Cube { size: 1.0 }
            } else {
                PrimitiveData::Quad { size: 1.0 }
            },
            color: [1.0, 1.0, 1.0, 1.0],
        });
        self.touch();
        true
    }

    /// Adds a directional light to the entity (one per entity).
    pub fn add_light(&mut self, id: u64) -> bool {
        let Some(idx) = self.index_of(id) else {
            return false;
        };
        if self.entities[idx].data.directional_light.is_some() {
            return false;
        }
        self.push_undo();
        self.entities[idx].data.directional_light =
            Some(automata_scene::DirectionalLight::default());
        self.touch();
        true
    }

    /// Adds a 3D camera to the entity (one per entity).
    pub fn add_camera_3d(&mut self, id: u64) -> bool {
        let Some(idx) = self.index_of(id) else {
            return false;
        };
        if self.entities[idx].data.camera_3d.is_some() {
            return false;
        }
        self.push_undo();
        self.entities[idx].data.camera_3d = Some(automata_renderer::Camera3D::new(
            glam::Vec3::new(0.0, 2.0, 6.0),
            glam::Vec3::ZERO,
        ));
        self.touch();
        true
    }

    /// Removes the mesh visual from the entity.
    pub fn remove_mesh(&mut self, id: u64) -> bool {
        let Some(idx) = self.index_of(id) else {
            return false;
        };
        if self.entities[idx].data.mesh_visual.is_none() {
            return false;
        }
        self.push_undo();
        self.entities[idx].data.mesh_visual = None;
        self.touch();
        true
    }

    /// Removes the directional light from the entity.
    pub fn remove_light(&mut self, id: u64) -> bool {
        let Some(idx) = self.index_of(id) else {
            return false;
        };
        if self.entities[idx].data.directional_light.is_none() {
            return false;
        }
        self.push_undo();
        self.entities[idx].data.directional_light = None;
        self.touch();
        true
    }

    /// Applies an edit to one entity's data through `f`, with undo.
    ///
    /// When `gesture` is true the edit is part of a continuous interaction
    /// (dragging a gizmo or scrubbing a field): only the first edit of the
    /// gesture records an undo snapshot, until [`Document::end_gesture`].
    /// Edits that leave the data unchanged record nothing. Returns true when
    /// the entity changed.
    pub fn modify(&mut self, id: u64, gesture: bool, f: impl FnOnce(&mut EntityData)) -> bool {
        let Some(idx) = self.index_of(id) else {
            return false;
        };
        let mut edited = self.entities[idx].data.clone();
        f(&mut edited);
        if edited == self.entities[idx].data {
            return false;
        }
        if gesture {
            if !self.mid_gesture {
                self.push_undo();
                self.mid_gesture = true;
            }
        } else {
            self.push_undo();
        }
        self.entities[idx].data = edited;
        self.touch();
        true
    }

    /// Ends a continuous edit gesture: the next gesture edit records a fresh
    /// undo snapshot.
    pub fn end_gesture(&mut self) {
        self.mid_gesture = false;
    }

    /// Creates a new entity of `kind`, selects it, and returns its id.
    pub fn create(&mut self, kind: CreateKind) -> u64 {
        let name = self.unique_name(kind.label());
        let mut data = EntityData {
            name: Some(name),
            transform: Some(TransformData::default()),
            ..Default::default()
        };
        match kind {
            CreateKind::Empty => {}
            CreateKind::Cube => {
                if let Some(t) = &mut data.transform {
                    t.position = [0.0, 0.5, 0.0];
                }
                ComponentKind::Mesh.add_to(&mut data);
            }
            CreateKind::Plane => {
                if let Some(t) = &mut data.transform {
                    // A quad faces +Z; tip it over to face +Y.
                    t.rotation_euler = [-std::f32::consts::FRAC_PI_2, 0.0, 0.0];
                    t.position = [0.0, 0.01, 0.0];
                }
                data.mesh_visual = Some(MeshVisualData {
                    primitive: PrimitiveData::Quad { size: 10.0 },
                    color: [0.55, 0.58, 0.64, 1.0],
                });
            }
            CreateKind::Light => {
                if let Some(t) = &mut data.transform {
                    t.position = [0.0, 5.0, 0.0];
                }
                ComponentKind::Light.add_to(&mut data);
            }
            CreateKind::Camera => {
                if let Some(t) = &mut data.transform {
                    t.position = [0.0, 2.0, 6.0];
                }
                ComponentKind::Camera.add_to(&mut data);
            }
            CreateKind::PhysicsCube => {
                if let Some(t) = &mut data.transform {
                    t.position = [0.0, 4.0, 0.0];
                }
                ComponentKind::Mesh.add_to(&mut data);
                ComponentKind::RigidBody.add_to(&mut data);
                ComponentKind::Collider.add_to(&mut data);
            }
        }
        self.push_undo();
        let id = self.alloc_id();
        self.entities.push(EntityEntry { id, data });
        self.selected = Some(id);
        self.touch();
        id
    }

    /// Duplicates the entity with `id` (placed right after it, selected).
    /// Returns the new entity's id.
    pub fn duplicate_entity(&mut self, id: u64) -> Option<u64> {
        let idx = self.index_of(id)?;
        let mut data = self.entities[idx].data.clone();
        let base = self.entities[idx].name().to_string();
        data.name = Some(self.unique_name(&base));
        self.push_undo();
        let new_id = self.alloc_id();
        self.entities
            .insert(idx + 1, EntityEntry { id: new_id, data });
        self.selected = Some(new_id);
        self.touch();
        Some(new_id)
    }

    /// Adds a default `kind` component to the entity. Returns true if added.
    pub fn add_component(&mut self, id: u64, kind: ComponentKind) -> bool {
        self.modify(id, false, |d| kind.add_to(d))
    }

    /// Removes the `kind` component from the entity. Returns true if removed.
    pub fn remove_component(&mut self, id: u64, kind: ComponentKind) -> bool {
        self.modify(id, false, |d| kind.remove_from(d))
    }

    /// Resets the entity's transform to identity. Returns true if changed.
    pub fn reset_transform(&mut self, id: u64) -> bool {
        self.modify(id, false, |d| d.transform = Some(TransformData::default()))
    }

    /// Whether undo is available.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether redo is available.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Translates an entity by `delta` in world space. The first nudge after
    /// any other edit pushes one undo snapshot (capturing the pre-move state);
    /// subsequent nudges in the same gesture coalesce into that entry, so held
    /// arrow keys don't flood the undo stack. Returns true when moved.
    pub fn nudge_entity(&mut self, id: u64, delta: [f32; 3]) -> bool {
        let Some(idx) = self.index_of(id) else {
            return false;
        };
        if self.entities[idx].data.transform.is_none() {
            return false;
        }
        // Coalesce: only snapshot on the first nudge of a gesture.
        if !self.mid_gesture {
            self.push_undo();
            self.mid_gesture = true;
        }
        let t = self.entities[idx].data.transform.as_mut().unwrap();
        t.position[0] += delta[0];
        t.position[1] += delta[1];
        t.position[2] += delta[2];
        self.touch();
        true
    }

    /// Undoes the last edit, restoring the prior snapshot.
    pub fn undo(&mut self) -> bool {
        let Some(ron) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.snapshot());
        self.restore(&ron);
        self.touch();
        true
    }

    /// Redoes the last undone edit.
    pub fn redo(&mut self) -> bool {
        let Some(ron) = self.redo.pop() else {
            return false;
        };
        self.undo.push(self.snapshot());
        self.restore(&ron);
        self.touch();
        true
    }

    /// Replaces the document contents from a RON snapshot, keeping ids stable
    /// by name so the selection survives.
    fn restore(&mut self, ron: &str) {
        let Ok(file) = SceneFile::from_ron(ron) else {
            return;
        };
        let old_ids: Vec<(String, u64)> = self
            .entities
            .iter()
            .map(|e| (e.name().to_string(), e.id))
            .collect();
        let mut used: Vec<u64> = Vec::new();
        let mut entities = Vec::with_capacity(file.entities.len());
        for (i, data) in file.entities.into_iter().enumerate() {
            let name = data.name.clone().unwrap_or_else(|| "(unnamed)".to_string());
            // Match by name first; else keep the id at the same position
            // (an undone rename); else allocate a fresh id.
            let by_name = old_ids
                .iter()
                .find(|(n, id)| *n == name && !used.contains(id))
                .map(|(_, id)| *id);
            let by_index = old_ids
                .get(i)
                .map(|(_, id)| *id)
                .filter(|id| !used.contains(id));
            let id = match by_name.or(by_index) {
                Some(id) => id,
                None => {
                    self.next_id += 1;
                    self.next_id - 1
                }
            };
            used.push(id);
            entities.push(EntityEntry { id, data });
        }
        self.entities = entities;
        self.next_id = self
            .entities
            .iter()
            .map(|e| e.id)
            .max()
            .unwrap_or(0)
            .max(self.next_id)
            + 1;
        if self.selected.and_then(|id| self.index_of(id)).is_none() {
            self.selected = self.entities.first().map(|e| e.id);
        }
    }

    /// Builds the engine [`SceneFile`] from the document.
    #[must_use]
    pub fn to_scene_file(&self) -> SceneFile {
        SceneFile {
            version: automata_core::SCENE_FILE_VERSION,
            entities: self.entities.iter().map(|e| e.data.clone()).collect(),
        }
    }

    /// Serializes the document to RON text.
    pub fn to_ron(&self) -> Result<String, automata_core::SceneFileError> {
        self.to_scene_file().to_ron()
    }

    /// Saves the document to `path` (or its current path).
    pub fn save(&mut self, path: Option<&Path>) -> std::io::Result<PathBuf> {
        let target = match path {
            Some(p) => p.to_path_buf(),
            None => self.path.clone().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "no scene path")
            })?,
        };
        std::fs::write(&target, self.to_ron().unwrap_or_default())?;
        self.path = Some(target.clone());
        self.dirty = false;
        Ok(target)
    }

    /// Loads a document from a RON scene file.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        Self::from_ron(text.as_str())
            .map_err(|e| e.to_string())
            .map(|mut doc| {
                doc.path = Some(path.to_path_buf());
                doc
            })
    }

    /// Parses a document from RON text.
    pub fn from_ron(text: &str) -> Result<Self, automata_core::SceneFileError> {
        let file = SceneFile::from_ron(text)?;
        let mut doc = Self::new();
        doc.entities = file
            .entities
            .into_iter()
            .map(|mut data| {
                if data.transform.is_none() {
                    data.transform = Some(TransformData::default());
                }
                let id = doc.alloc_id();
                EntityEntry { id, data }
            })
            .collect();
        doc.dirty = false;
        doc.selected = doc.entities.first().map(|e| e.id);
        Ok(doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_document_is_empty_and_clean() {
        let doc = Document::new();
        assert_eq!(doc.entity_count(), 0);
        assert!(!doc.dirty());
        assert!(doc.selected.is_none());
    }

    #[test]
    fn add_entity_selects_and_marks_dirty() {
        let mut doc = Document::new();
        let idx = doc.add_entity();
        assert_eq!(doc.entity_count(), 1);
        assert!(doc.dirty());
        assert_eq!(doc.selected, Some(doc.entities[idx].id));
        assert!(doc.entities[idx].data.transform.is_some());
    }

    #[test]
    fn add_mesh_guard_duplicate() {
        let mut doc = Document::new();
        let idx = doc.add_entity();
        let id = doc.entities[idx].id;
        assert!(doc.add_mesh(id, true));
        assert!(!doc.add_mesh(id, true)); // one mesh per entity
    }

    #[test]
    fn delete_removes_and_clears_selection() {
        let mut doc = Document::new();
        let idx = doc.add_entity();
        let id = doc.entities[idx].id;
        assert!(doc.delete_entity(id));
        assert_eq!(doc.entity_count(), 0);
        assert!(doc.selected.is_none());
    }

    #[test]
    fn undo_redo_restores_state() {
        let mut doc = Document::new();
        doc.add_entity();
        assert_eq!(doc.entity_count(), 1);
        assert!(doc.undo());
        assert_eq!(doc.entity_count(), 0);
        assert!(doc.redo());
        assert_eq!(doc.entity_count(), 1);
    }

    #[test]
    fn save_load_round_trip() {
        let mut doc = Document::new();
        let idx = doc.add_entity_named("Cube");
        let id = doc.entities[idx].id;
        doc.add_mesh(id, true);
        let ron = doc.to_ron().expect("serialize");
        let loaded = Document::from_ron(&ron).expect("parse");
        assert_eq!(loaded.entity_count(), 1);
        assert_eq!(loaded.entities[0].name(), "Cube");
        assert!(loaded.entities[0].data.mesh_visual.is_some());
        assert!(loaded.entities[0].data.transform.is_some());
    }

    #[test]
    fn starter_scene_has_sun_ground_cube() {
        let doc = Document::starter();
        assert_eq!(doc.entity_count(), 3);
        assert!(!doc.dirty());
        let names: Vec<&str> = doc.entities.iter().map(super::EntityEntry::name).collect();
        assert!(names.contains(&"Sun") && names.contains(&"Ground") && names.contains(&"Cube"));
    }

    #[test]
    fn change_counter_bumps_on_edit() {
        let mut doc = Document::new();
        let c0 = doc.change_counter();
        doc.add_entity();
        assert!(doc.change_counter() > c0);
    }

    #[test]
    fn nudge_moves_entity_and_coalesces_undo() {
        let mut doc = Document::starter();
        let id = doc.entities.iter().find(|e| e.name() == "Cube").unwrap().id;
        let before = doc.entities[doc.index_of(id).unwrap()]
            .data
            .transform
            .unwrap()
            .position;
        let c0 = doc.change_counter();
        let undo0 = doc.undo.len();

        // A gesture of several nudges = one undo entry.
        doc.nudge_entity(id, [1.0, 0.0, 0.0]);
        doc.nudge_entity(id, [1.0, 0.0, 0.0]);
        doc.nudge_entity(id, [0.0, 2.0, 0.0]);

        let after = doc.entities[doc.index_of(id).unwrap()]
            .data
            .transform
            .unwrap()
            .position;
        assert_eq!(after, [before[0] + 2.0, before[1] + 2.0, before[2]]);
        assert!(doc.change_counter() > c0);
        assert_eq!(doc.undo.len(), undo0 + 1, "gesture coalesced to one undo");

        // Undo restores the pre-gesture position.
        assert!(doc.undo());
        let restored = doc.entities[doc.index_of(id).unwrap()]
            .data
            .transform
            .unwrap()
            .position;
        assert_eq!(restored, before);
    }
}
