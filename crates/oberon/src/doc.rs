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
    /// Scene look settings (sky, fog, exposure, tone mapping).
    Environment,
    /// A cube with a dynamic rigid body and a box collider.
    PhysicsCube,
    /// A playable character: WASD / arrows to move, Space to jump in Play
    /// mode.
    Player,
}

impl CreateKind {
    /// Every kind, in menu order.
    pub const ALL: [Self; 8] = [
        Self::Empty,
        Self::Cube,
        Self::Plane,
        Self::Light,
        Self::Camera,
        Self::Environment,
        Self::PhysicsCube,
        Self::Player,
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
            Self::Environment => "Environment",
            Self::PhysicsCube => "Physics Cube",
            Self::Player => "Player",
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
    /// Scene look: ambient, fog, exposure, tone mapping.
    Environment,
    /// Rigid body.
    RigidBody,
    /// 3D collider.
    Collider,
    /// Character controller (player movement in Play mode).
    Character,
}

impl ComponentKind {
    /// Every addable component, in menu order.
    pub const ALL: [Self; 7] = [
        Self::Mesh,
        Self::Light,
        Self::Camera,
        Self::Environment,
        Self::RigidBody,
        Self::Collider,
        Self::Character,
    ];

    /// The display name.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Mesh => "Mesh Renderer",
            Self::Light => "Directional Light",
            Self::Camera => "Camera",
            Self::Environment => "Environment",
            Self::RigidBody => "Rigidbody",
            Self::Collider => "Collider",
            Self::Character => "Character Controller",
        }
    }

    /// Whether `data` has this component.
    #[must_use]
    pub fn present_in(self, data: &EntityData) -> bool {
        match self {
            Self::Mesh => data.mesh_visual.is_some(),
            Self::Light => data.directional_light.is_some(),
            Self::Camera => data.camera_3d.is_some(),
            Self::Environment => data.environment.is_some(),
            Self::RigidBody => data.rigid_body.is_some(),
            Self::Collider => data.collider_3d.is_some(),
            Self::Character => data.character_controller.is_some(),
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
            Self::Environment => {
                data.environment
                    .get_or_insert_with(automata_renderer::Environment::daylight);
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
            Self::Character => {
                // A character needs a kinematic body and a collider.
                if data
                    .rigid_body
                    .as_ref()
                    .is_none_or(|b| b.body_type == automata_physics::BodyType::Dynamic)
                {
                    data.rigid_body = Some(automata_physics::RigidBody::kinematic_position());
                }
                Self::Collider.add_to(data);
                data.character_controller.get_or_insert_with(|| {
                    automata_physics::CharacterController::new().snap_to_ground(0.3)
                });
                data.character_velocity
                    .get_or_insert_with(automata_physics::CharacterVelocity::default);
            }
        }
    }

    /// Copies this component's values from `src` to `dst` (adding it to
    /// `dst` if missing; removing it if `src` lacks it).
    pub fn copy_between(self, src: &EntityData, dst: &mut EntityData) {
        match self {
            Self::Mesh => dst.mesh_visual.clone_from(&src.mesh_visual),
            Self::Light => dst.directional_light = src.directional_light,
            Self::Camera => dst.camera_3d = src.camera_3d,
            Self::Environment => dst.environment = src.environment,
            Self::RigidBody => dst.rigid_body.clone_from(&src.rigid_body),
            Self::Collider => dst.collider_3d.clone_from(&src.collider_3d),
            Self::Character => {
                dst.character_controller
                    .clone_from(&src.character_controller);
                dst.character_velocity = src.character_velocity;
            }
        }
    }

    /// Removes this component from `data`.
    pub fn remove_from(self, data: &mut EntityData) {
        match self {
            Self::Mesh => data.mesh_visual = None,
            Self::Light => data.directional_light = None,
            Self::Camera => data.camera_3d = None,
            Self::Environment => data.environment = None,
            Self::RigidBody => data.rigid_body = None,
            Self::Collider => data.collider_3d = None,
            Self::Character => {
                data.character_controller = None;
                data.character_velocity = None;
            }
        }
    }
}

/// Built-in arrangements for checking the renderer (Graphics panel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TestScene {
    /// A grid of cubes sweeping hue and brightness: color, tone mapping,
    /// and exposure response.
    ColorChart,
    /// Pillars and a floating slab on a floor: shadow shape, bias, and
    /// softness.
    Shadows,
    /// A row of pillars receding 150 units: distance fog and far clipping.
    Depth,
    /// A 20×20 field of cubes (400 draws): frame-time stress.
    Stress,
}

impl TestScene {
    /// Every test, in panel order.
    pub const ALL: [Self; 4] = [Self::ColorChart, Self::Shadows, Self::Depth, Self::Stress];

    /// The button label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::ColorChart => "Color chart",
            Self::Shadows => "Shadows",
            Self::Depth => "Fog depth",
            Self::Stress => "400 cubes",
        }
    }

    /// One line on what the test is for.
    #[must_use]
    pub fn hint(self) -> &'static str {
        match self {
            Self::ColorChart => "Hue and brightness sweep: compare tone mapping curves",
            Self::Shadows => "Pillars and a floating slab: check shadow shape and acne",
            Self::Depth => "Pillars out to 150 m: tune fog density and start",
            Self::Stress => "A 20×20 field of cubes: watch the frame time",
        }
    }
}

fn hsv(h: f32, s: f32, v: f32) -> [f32; 4] {
    let h = h.rem_euclid(1.0) * 6.0;
    let i = h.floor();
    let f = h - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    let (r, g, b) = match i as u8 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    [r, g, b, 1.0]
}

fn test_box(pos: [f32; 3], scale: [f32; 3], color: [f32; 4]) -> EntityData {
    EntityData {
        transform: Some(TransformData {
            position: pos,
            scale,
            ..TransformData::default()
        }),
        mesh_visual: Some(MeshVisualData {
            primitive: PrimitiveData::Cube { size: 1.0 },
            color,
        }),
        ..Default::default()
    }
}

fn test_scene_entities(scene: TestScene) -> Vec<(String, EntityData)> {
    let mut out = Vec::new();
    match scene {
        TestScene::ColorChart => {
            for row in 0..4 {
                for col in 0..8 {
                    let value = 1.0 - row as f32 * 0.22;
                    let sat = if col == 7 { 0.0 } else { 0.85 };
                    let c = hsv(col as f32 / 7.0, sat, value);
                    let pos = [col as f32 * 1.3 - 4.55, 0.5 + (3 - row) as f32 * 1.3, -6.0];
                    out.push(("Test Chart".into(), test_box(pos, [1.0; 3], c)));
                }
            }
        }
        TestScene::Shadows => {
            out.push((
                "Test Floor".into(),
                test_box([0.0, -0.05, 8.0], [14.0, 0.1, 10.0], [0.7, 0.7, 0.72, 1.0]),
            ));
            for (i, h) in [0.5f32, 1.5, 3.0, 1.0, 2.2].into_iter().enumerate() {
                let x = i as f32 * 2.2 - 4.4;
                out.push((
                    "Test Pillar".into(),
                    test_box([x, h / 2.0, 7.0], [0.6, h, 0.6], [0.9, 0.9, 0.9, 1.0]),
                ));
            }
            out.push((
                "Test Slab".into(),
                test_box([0.0, 2.6, 10.0], [4.0, 0.2, 2.0], [0.85, 0.5, 0.3, 1.0]),
            ));
        }
        TestScene::Depth => {
            for i in 0..12 {
                let z = -8.0 - i as f32 * 13.0;
                let x = if i % 2 == 0 { -3.0 } else { 3.0 };
                out.push((
                    "Test Depth".into(),
                    test_box([x, 2.0, z], [1.0, 4.0, 1.0], [0.95, 0.95, 0.95, 1.0]),
                ));
            }
        }
        TestScene::Stress => {
            for i in 0..20 {
                for j in 0..20 {
                    // A cheap deterministic hash for varied heights.
                    let hsh = ((i * 73 + j * 151) % 97) as f32 / 97.0;
                    let h = 0.3 + hsh * 2.0;
                    let pos = [i as f32 * 0.9 - 8.55, h / 2.0, j as f32 * 0.9 - 30.0];
                    let c = hsv(0.55 + hsh * 0.15, 0.35, 0.9);
                    out.push(("Test Stress".into(), test_box(pos, [0.6, h, 0.6], c)));
                }
            }
        }
    }
    out
}

/// Keeps a camera attached to its entity: when an edit moves the entity's
/// position but leaves the camera alone, the camera's eye and look-at
/// point move by the same offset (so dragging a camera entity with the
/// Move tool drags the view it renders).
fn follow_camera(before: &EntityData, after: &mut EntityData) {
    let (Some(old_t), Some(new_t)) = (before.transform, after.transform) else {
        return;
    };
    if after.camera_3d != before.camera_3d {
        return;
    }
    let delta = glam::Vec3::from(new_t.position) - glam::Vec3::from(old_t.position);
    if delta == glam::Vec3::ZERO {
        return;
    }
    if let Some(cam) = &mut after.camera_3d {
        cam.position += delta;
        cam.target += delta;
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
        // Something to play with: a player and a stack of crates.
        let player = doc.create(CreateKind::Player);
        if let Some(t) = doc.transform_mut(player) {
            t.position = [-3.0, 0.9, 2.0];
        }
        for (i, pos) in [[2.5, 0.5, -1.0], [2.5, 1.5, -1.0], [2.6, 2.5, -0.9]]
            .into_iter()
            .enumerate()
        {
            let id = doc.create(CreateKind::PhysicsCube);
            if let Some(idx) = doc.index_of(id) {
                let e = &mut doc.entities[idx].data;
                e.name = Some(format!("Crate {}", i + 1));
                if let Some(t) = &mut e.transform {
                    t.position = pos;
                }
                if let Some(m) = &mut e.mesh_visual {
                    m.color = [0.78, 0.62, 0.40, 1.0];
                }
            }
        }
        doc.selected = None;
        doc.dirty = false;
        doc.undo.clear();
        doc
    }

    fn transform_mut(&mut self, id: u64) -> Option<&mut TransformData> {
        let idx = self.index_of(id)?;
        self.entities[idx].data.transform.as_mut()
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
        follow_camera(&self.entities[idx].data, &mut edited);
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
            CreateKind::Environment => {
                ComponentKind::Environment.add_to(&mut data);
            }
            CreateKind::PhysicsCube => {
                if let Some(t) = &mut data.transform {
                    t.position = [0.0, 4.0, 0.0];
                }
                ComponentKind::Mesh.add_to(&mut data);
                ComponentKind::RigidBody.add_to(&mut data);
                ComponentKind::Collider.add_to(&mut data);
            }
            CreateKind::Player => {
                if let Some(t) = &mut data.transform {
                    t.position = [0.0, 0.9, 0.0];
                    t.scale = [0.8, 1.6, 0.8];
                }
                data.mesh_visual = Some(MeshVisualData {
                    primitive: PrimitiveData::Cube { size: 1.0 },
                    color: [1.0, 0.56, 0.24, 1.0],
                });
                ComponentKind::Character.add_to(&mut data);
            }
        }
        self.push_undo();
        let id = self.alloc_id();
        self.entities.push(EntityEntry { id, data });
        self.selected = Some(id);
        self.touch();
        id
    }

    /// Adds a graphics test arrangement (one undo step). Every entity is
    /// named `Test …` so [`Document::clear_tests`] can remove them again.
    /// Returns how many entities were added.
    pub fn spawn_test(&mut self, scene: TestScene) -> usize {
        let items = test_scene_entities(scene);
        if items.is_empty() {
            return 0;
        }
        self.push_undo();
        let n = items.len();
        let mut taken: std::collections::HashSet<String> =
            self.entities.iter().map(|e| e.name().to_string()).collect();
        let mut counters: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for (base, mut data) in items {
            let counter = counters.entry(base.clone()).or_insert(0);
            let name = loop {
                *counter += 1;
                let candidate = format!("{base} {counter}");
                if !taken.contains(&candidate) {
                    break candidate;
                }
            };
            taken.insert(name.clone());
            data.name = Some(name);
            let id = self.alloc_id();
            self.entities.push(EntityEntry { id, data });
        }
        self.selected = None;
        self.touch();
        n
    }

    /// Removes every entity whose name starts with `Test ` (one undo
    /// step). Returns how many were removed.
    pub fn clear_tests(&mut self) -> usize {
        let before = self.entities.len();
        if !self.entities.iter().any(|e| e.name().starts_with("Test ")) {
            return 0;
        }
        self.push_undo();
        self.entities.retain(|e| !e.name().starts_with("Test "));
        if self.selected.is_some_and(|id| self.index_of(id).is_none()) {
            self.selected = None;
        }
        self.touch();
        before - self.entities.len()
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

    /// Resolves a scene-relative path (scripts, prefabs) against the scene
    /// file's directory, or the working directory for unsaved scenes.
    #[must_use]
    pub fn resolve(&self, rel: &str) -> PathBuf {
        let p = Path::new(rel);
        if p.is_absolute() {
            return p.to_path_buf();
        }
        self.base_dir().join(p)
    }

    /// The directory scene-relative paths are resolved against.
    #[must_use]
    pub fn base_dir(&self) -> PathBuf {
        self.path
            .as_ref()
            .and_then(|p| p.parent())
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(
                || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                Path::to_path_buf,
            )
    }

    /// `path` relative to [`Document::base_dir`] when inside it (forward
    /// slashes, so scene files stay portable), else absolute.
    #[must_use]
    pub fn relative(&self, path: &Path) -> String {
        let base = self.base_dir();
        let abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().map_or_else(|_| path.to_path_buf(), |cwd| cwd.join(path))
        };
        let base = std::fs::canonicalize(&base).unwrap_or(base);
        let abs_c = abs
            .parent()
            .and_then(|parent| std::fs::canonicalize(parent).ok())
            .and_then(|parent| abs.file_name().map(|n| parent.join(n)))
            .unwrap_or(abs);
        match abs_c.strip_prefix(&base) {
            Ok(rel) => rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/"),
            Err(_) => abs_c.display().to_string(),
        }
    }

    /// Resets a component to its defaults. Returns true if changed.
    pub fn reset_component(&mut self, id: u64, kind: ComponentKind) -> bool {
        self.modify(id, false, |d| {
            kind.remove_from(d);
            kind.add_to(d);
        })
    }

    /// Copies the `kind` component's values from `source` onto entity `id`.
    pub fn paste_component(&mut self, id: u64, kind: ComponentKind, source: &EntityData) -> bool {
        self.modify(id, false, |d| kind.copy_between(source, d))
    }

    /// Adds copies of `entities` (unique names, offset slightly so pastes
    /// are visible), selecting the last. Returns the new ids.
    pub fn paste_entities(&mut self, entities: &[EntityData], offset: [f32; 3]) -> Vec<u64> {
        if entities.is_empty() {
            return Vec::new();
        }
        self.push_undo();
        let mut ids = Vec::with_capacity(entities.len());
        for data in entities {
            let mut data = data.clone();
            let base = data.name.clone().unwrap_or_else(|| "Entity".to_string());
            data.name = Some(self.unique_name(&base));
            if let Some(t) = &mut data.transform {
                for (axis, d) in t.position.iter_mut().zip(offset) {
                    *axis += d;
                }
            }
            let id = self.alloc_id();
            self.entities.push(EntityEntry { id, data });
            ids.push(id);
        }
        self.selected = ids.last().copied();
        self.touch();
        ids
    }

    /// The prefab file for entity `id`: a one-entity scene file.
    #[must_use]
    pub fn prefab_of(&self, id: u64) -> Option<SceneFile> {
        let idx = self.index_of(id)?;
        Some(SceneFile {
            version: automata_core::SCENE_FILE_VERSION,
            entities: vec![self.entities[idx].data.clone()],
        })
    }

    /// Instantiates a prefab file's entities with their first entity placed
    /// at `at` (others keep their relative offsets). Returns the new ids.
    pub fn instantiate(&mut self, prefab: &SceneFile, at: [f32; 3]) -> Vec<u64> {
        let origin = prefab
            .entities
            .first()
            .and_then(|e| e.transform)
            .map_or([0.0; 3], |t| t.position);
        let offset = [at[0] - origin[0], at[1] - origin[1], at[2] - origin[2]];
        self.paste_entities(&prefab.entities, offset)
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
    #[test]
    fn test_scenes_add_and_clear_in_single_undo_steps() {
        let mut doc = Document::starter();
        let before = doc.entities.len();
        let n = doc.spawn_test(TestScene::Stress);
        assert_eq!(n, 400);
        assert_eq!(doc.entities.len(), before + 400);
        let names: std::collections::HashSet<&str> =
            doc.entities.iter().map(EntityEntry::name).collect();
        assert_eq!(names.len(), doc.entities.len(), "names stay unique");
        doc.spawn_test(TestScene::Shadows);
        assert_eq!(doc.clear_tests(), 400 + 7);
        assert_eq!(doc.entities.len(), before);
        assert!(doc.undo());
        assert_eq!(doc.entities.len(), before + 407);
    }

    #[test]
    fn moving_a_camera_entity_moves_its_view() {
        let mut doc = Document::new();
        let id = doc.create(CreateKind::Camera);
        let cam = doc.entities[doc.index_of(id).unwrap()]
            .data
            .camera_3d
            .unwrap();
        doc.modify(id, false, |d| {
            if let Some(t) = &mut d.transform {
                t.position[0] += 2.0;
            }
        });
        let moved = doc.entities[doc.index_of(id).unwrap()]
            .data
            .camera_3d
            .unwrap();
        assert!((moved.position.x - cam.position.x - 2.0).abs() < 1e-5);
        assert!((moved.target.x - cam.target.x - 2.0).abs() < 1e-5);
        // Editing the camera itself does not drag the transform along.
        doc.modify(id, false, |d| {
            if let Some(c) = &mut d.camera_3d {
                c.fov_y_degrees = 40.0;
            }
        });
        let t = doc.entities[doc.index_of(id).unwrap()]
            .data
            .transform
            .unwrap();
        assert!((t.position[0] - 2.0).abs() < 1e-5);
    }

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
    fn starter_scene_is_playable() {
        let doc = Document::starter();
        assert_eq!(doc.entity_count(), 7);
        assert!(!doc.dirty());
        assert!(!doc.can_undo());
        let names: Vec<&str> = doc.entities.iter().map(super::EntityEntry::name).collect();
        for name in [
            "Sun", "Ground", "Cube", "Player", "Crate 1", "Crate 2", "Crate 3",
        ] {
            assert!(names.contains(&name), "{name} missing from {names:?}");
        }
        let player = &doc.entities[names.iter().position(|n| *n == "Player").unwrap()].data;
        assert!(player.character_controller.is_some());
        assert!(player.rigid_body.is_some() && player.collider_3d.is_some());
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

    fn cube(doc: &mut Document, name: &str) -> u64 {
        let idx = doc.add_entity();
        let id = doc.entities[idx].id;
        doc.add_mesh(id, true);
        doc.entities[idx].data.name = Some(name.to_string());
        id
    }

    #[test]
    fn paths_resolve_against_the_scene_directory() {
        let dir = std::env::temp_dir().join("oberon_doc_paths");
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        let mut doc = Document::new();
        doc.path = Some(dir.join("level.ron"));
        assert_eq!(doc.base_dir(), dir);
        assert_eq!(doc.resolve("scripts/a.py"), dir.join("scripts/a.py"));
        assert_eq!(
            doc.relative(&dir.join("scripts").join("a.py")),
            "scripts/a.py"
        );
        let outside = std::env::temp_dir().join("elsewhere.py");
        assert!(Path::new(&doc.relative(&outside)).is_absolute());
        assert_eq!(doc.resolve(&outside.display().to_string()), outside);
    }

    #[test]
    fn paste_entities_offsets_renames_and_undoes() {
        let mut doc = Document::new();
        let id = cube(&mut doc, "Box");
        let data = doc.entities[doc.index_of(id).unwrap()].data.clone();
        let ids = doc.paste_entities(std::slice::from_ref(&data), [1.0, 0.0, 0.0]);
        assert_eq!(ids.len(), 1);
        assert_eq!(doc.selected, Some(ids[0]));
        let pasted = &doc.entities[doc.index_of(ids[0]).unwrap()].data;
        assert_ne!(pasted.name, data.name, "pasted copies get unique names");
        let (a, b) = (
            data.transform.unwrap().position,
            pasted.transform.unwrap().position,
        );
        assert!((b[0] - a[0] - 1.0).abs() < 1e-6);
        assert!(doc.undo());
        assert_eq!(doc.entity_count(), 1);
        assert_eq!(doc.paste_entities(&[], [0.0; 3]), Vec::<u64>::new());
    }

    #[test]
    fn prefabs_round_trip_and_instantiate_at_a_point() {
        let mut doc = Document::new();
        let id = cube(&mut doc, "Crate");
        doc.entities[0]
            .data
            .attributes
            .insert("health".into(), automata_core::AttributeValue::Int(5));
        let prefab = doc.prefab_of(id).unwrap();
        assert_eq!(prefab.entities.len(), 1);
        let text = prefab.to_ron().unwrap();
        let back = SceneFile::from_ron(&text).unwrap();
        let mut other = Document::new();
        let ids = other.instantiate(&back, [3.0, 1.0, -2.0]);
        let e = &other.entities[other.index_of(ids[0]).unwrap()].data;
        assert_eq!(e.transform.unwrap().position, [3.0, 1.0, -2.0]);
        assert_eq!(
            e.attributes.get("health"),
            Some(&automata_core::AttributeValue::Int(5))
        );
        assert!(e.mesh_visual.is_some());
        assert!(doc.prefab_of(999_999).is_none());
    }

    #[test]
    fn reset_and_paste_component_values() {
        let mut doc = Document::new();
        let a = cube(&mut doc, "A");
        let b = cube(&mut doc, "B");
        let ia = doc.index_of(a).unwrap();
        doc.entities[ia].data.mesh_visual.as_mut().unwrap().color = [0.1, 0.2, 0.3, 1.0];
        let source = doc.entities[ia].data.clone();
        assert!(doc.paste_component(b, ComponentKind::Mesh, &source));
        let ib = doc.index_of(b).unwrap();
        assert_eq!(
            doc.entities[ib].data.mesh_visual.as_ref().unwrap().color,
            [0.1, 0.2, 0.3, 1.0]
        );
        assert!(doc.reset_component(a, ComponentKind::Mesh));
        let ia = doc.index_of(a).unwrap();
        assert_ne!(
            doc.entities[ia].data.mesh_visual.as_ref().unwrap().color,
            [0.1, 0.2, 0.3, 1.0]
        );
    }
}
