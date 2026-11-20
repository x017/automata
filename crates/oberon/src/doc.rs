//! The Oberon scene document: the editor's model of the scene being edited.
//!
//! A document is a [`SceneFile`] (the engine's serde RON data) plus editor
//! state: stable per-entity ids, a selection, dirty tracking, and an undo
//! stack of serialized snapshots. All editing goes through methods that
//! record undo and mark the document dirty, so the UI never touches the
//! file data directly.

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
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    /// An empty document with one default entity selected.
    pub fn new() -> Self {
        Self {
            entities: Vec::new(),
            next_id: 1,
            path: None,
            selected: None,
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// A starter scene (a sun, a ground plane, and a cube) for a fresh file.
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
    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    /// Whether the document has unsaved changes.
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// Finds an entity's index by id.
    pub fn index_of(&self, id: u64) -> Option<usize> {
        self.entities.iter().position(|e| e.id == id)
    }

    /// The selected entity's index, if it still exists.
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
        let existing: Vec<&str> = self.entities.iter().map(|e| e.name()).collect();
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
        self.dirty = true;
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
        self.dirty = true;
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
        self.dirty = true;
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
        self.dirty = true;
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
        self.dirty = true;
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
        self.dirty = true;
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
        self.dirty = true;
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
        self.dirty = true;
        true
    }

    /// Whether undo is available.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether redo is available.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Undoes the last edit, restoring the prior snapshot.
    pub fn undo(&mut self) -> bool {
        let Some(ron) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.snapshot());
        self.restore(&ron);
        self.dirty = true;
        true
    }

    /// Redoes the last undone edit.
    pub fn redo(&mut self) -> bool {
        let Some(ron) = self.redo.pop() else {
            return false;
        };
        self.undo.push(self.snapshot());
        self.restore(&ron);
        self.dirty = true;
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
                let id = old_ids
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, id)| *id)
                    .unwrap_or_else(|| {
                        self.next_id += 1;
                        self.next_id - 1
                    });
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
        let names: Vec<&str> = doc.entities.iter().map(|e| e.name()).collect();
        assert!(names.contains(&"Sun") && names.contains(&"Ground") && names.contains(&"Cube"));
    }
}
