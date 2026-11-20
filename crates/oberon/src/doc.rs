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
            position: [0.0, -0.5, 0.0],
            scale: [12.0, 0.2, 12.0],
            ..Default::default()
        });
        doc.entities[ground].data.mesh_visual = Some(MeshVisualData {
            primitive: PrimitiveData::Cube { size: 1.0 },
            color: [0.16, 0.17, 0.20, 1.0],
        });
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
        self.entities = file
            .entities
            .into_iter()
            .map(|data| {
                let name = data.name.clone().unwrap_or_default();
                let id = old_ids.iter().find(|(n, _)| *n == name).map_or_else(
                    || {
                        self.next_id += 1;
                        self.next_id - 1
                    },
                    |(_, id)| *id,
                );
                EntityEntry { id, data }
            })
            .collect();
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
