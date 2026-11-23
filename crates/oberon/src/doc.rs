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

/// The local bounding box `(center, size)` of an entity's mesh, before the
/// transform's scale: primitives from their size, models from the loaded
/// file. `None` without a mesh (or for a model that failed to load).
#[must_use]
pub fn mesh_bounds(data: &EntityData) -> Option<(glam::Vec3, glam::Vec3)> {
    use glam::Vec3;
    Some(match &data.mesh_visual.as_ref()?.primitive {
        PrimitiveData::Cube { size } | PrimitiveData::Sphere { size } => {
            (Vec3::ZERO, Vec3::splat(*size))
        }
        PrimitiveData::Quad { size } => (Vec3::ZERO, Vec3::new(*size, *size, 0.02)),
        PrimitiveData::Custom { mesh } => {
            let mut lo = Vec3::splat(f32::INFINITY);
            let mut hi = Vec3::splat(f32::NEG_INFINITY);
            for v in &mesh.vertices {
                lo = lo.min(Vec3::from(v.position));
                hi = hi.max(Vec3::from(v.position));
            }
            if !lo.is_finite() {
                return None;
            }
            ((lo + hi) * 0.5, hi - lo)
        }
        PrimitiveData::Model { path } => {
            let model = automata_assets::load(path).ok()?;
            (model.center(), model.size())
        }
    })
}

/// A box collider fitted to the entity's visible mesh (physics ignores
/// scale, so the collider carries the world size and offset). Entities
/// without a mesh get a 1 m cube.
#[must_use]
pub fn fitted_box(data: &EntityData) -> automata_physics::Collider3D {
    let (center, size) = mesh_bounds(data).unwrap_or((glam::Vec3::ZERO, glam::Vec3::ONE));
    let scale = glam::Vec3::from(data.transform.map_or([1.0; 3], |t| t.scale));
    let ext = (size * scale.abs()).max(glam::Vec3::splat(0.01));
    let offset = center * scale;
    automata_physics::Collider3D::cuboid(ext.x, ext.y, ext.z).offset(offset.to_array())
}

/// The model file an entity shows, if its mesh is one.
#[must_use]
pub fn model_path(data: &EntityData) -> Option<&str> {
    match &data.mesh_visual.as_ref()?.primitive {
        PrimitiveData::Model { path } => Some(path),
        _ => None,
    }
}

/// The rig (bones and clips) of the animated model an entity shows.
#[must_use]
pub fn model_rig(data: &EntityData) -> Option<std::sync::Arc<automata_assets::Rig>> {
    automata_assets::load(model_path(data)?).ok()?.rig.clone()
}

/// The Animation a new or freshly swapped animated model gets: its idle
/// clip (or first), playing on start. Entities without one get an empty
/// clip.
#[must_use]
pub fn default_animation(data: &EntityData) -> automata_scene::Animation {
    let clip = model_rig(data)
        .and_then(|rig| automata_scene::Animation::default_clip(&rig).map(str::to_string))
        .unwrap_or_default();
    automata_scene::Animation::new(clip)
}

/// Gives an entity whose model has clips an Animation (or, if it has one
/// naming a clip the model lacks, points it at the default clip).
fn fit_animation(data: &mut EntityData) {
    let Some(rig) = model_rig(data).filter(|r| !r.clips.is_empty()) else {
        return;
    };
    let fresh = default_animation(data);
    match &mut data.animation {
        Some(a) if rig.clip(&a.clip).is_none() => a.clip = fresh.clip,
        Some(_) => {}
        None => data.animation = Some(fresh),
    }
}

/// The collider Add Collider gives an entity: a box fitted to its mesh,
/// unless it shows a model that a box fits badly. Then static scenery gets
/// the model's triangles (Mesh) and moving bodies its Convex Hull (see
/// [`automata_physics::auto_model_shape`]).
#[must_use]
pub fn fitted_collider(data: &EntityData) -> automata_physics::Collider3D {
    if let Some(PrimitiveData::Sphere { size }) = data.mesh_visual.as_ref().map(|m| &m.primitive) {
        let scale = glam::Vec3::from(data.transform.map_or([1.0; 3], |t| t.scale));
        return automata_physics::Collider3D::ball(
            (size * 0.5 * scale.abs().max_element()).max(0.005),
        );
    }
    let body = data.rigid_body.as_ref().map(|b| b.body_type);
    model_path(data)
        .and_then(|path| automata_physics::auto_model_shape(path, body))
        .map_or_else(|| fitted_box(data), automata_physics::Collider3D::new)
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
    /// A light shining every way from a point (a lamp, a torch).
    PointLight,
    /// A light shining in a cone (a flashlight, a headlight).
    SpotLight,
    /// A perspective camera.
    Camera,
    /// Scene look settings (sky, fog, exposure, tone mapping).
    Environment,
    /// An animated water surface.
    Water,
    /// A particle emitter (a campfire's flames to start with).
    Particles,
    /// A cube with a dynamic rigid body and a box collider.
    PhysicsCube,
    /// A 1 m sphere.
    Sphere,
    /// A playable character: WASD / arrows to move, Space to jump in Play
    /// mode.
    Player,
}

impl CreateKind {
    /// Every kind, in menu order.
    pub const ALL: [Self; 13] = [
        Self::Empty,
        Self::Cube,
        Self::Sphere,
        Self::Plane,
        Self::Light,
        Self::PointLight,
        Self::SpotLight,
        Self::Camera,
        Self::Environment,
        Self::Water,
        Self::Particles,
        Self::PhysicsCube,
        Self::Player,
    ];

    /// The menu label (also the base name of the new entity).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Empty => "Empty",
            Self::Cube => "Cube",
            Self::Sphere => "Sphere",
            Self::Plane => "Plane",
            Self::Light => "Directional Light",
            Self::PointLight => "Point Light",
            Self::SpotLight => "Spot Light",
            Self::Camera => "Camera",
            Self::Environment => "Environment",
            Self::Water => "Water",
            Self::Particles => "Particles",
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
    /// Which clip an animated model plays.
    Animation,
    /// Directional light.
    Light,
    /// Light shining every way from the entity.
    PointLight,
    /// Light shining in a cone along the entity's forward axis.
    SpotLight,
    /// Perspective camera.
    Camera,
    /// Scene look: ambient, fog, exposure, tone mapping.
    Environment,
    /// Animated water surface.
    Water,
    /// Particle emitter: fire, smoke, sparks, dust …
    Particles,
    /// Rigid body.
    RigidBody,
    /// 3D collider.
    Collider,
    /// Character controller (player movement in Play mode).
    Character,
    /// A sound the entity plays.
    AudioSource,
}

impl ComponentKind {
    /// Every addable component, in menu order.
    pub const ALL: [Self; 13] = [
        Self::Mesh,
        Self::Animation,
        Self::Light,
        Self::PointLight,
        Self::SpotLight,
        Self::Camera,
        Self::Environment,
        Self::Water,
        Self::Particles,
        Self::RigidBody,
        Self::Collider,
        Self::Character,
        Self::AudioSource,
    ];

    /// The display name.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Mesh => "Mesh Renderer",
            Self::Animation => "Animation",
            Self::Light => "Directional Light",
            Self::PointLight => "Point Light",
            Self::SpotLight => "Spot Light",
            Self::Camera => "Camera",
            Self::Environment => "Environment",
            Self::Water => "Water",
            Self::Particles => "Particle Emitter",
            Self::RigidBody => "Rigidbody",
            Self::Collider => "Collider",
            Self::Character => "Character Controller",
            Self::AudioSource => "Audio Source",
        }
    }

    /// Whether `data` has this component.
    #[must_use]
    pub fn present_in(self, data: &EntityData) -> bool {
        match self {
            Self::Mesh => data.mesh_visual.is_some(),
            Self::Animation => data.animation.is_some(),
            Self::Light => data.directional_light.is_some(),
            Self::PointLight => data.point_light.is_some(),
            Self::SpotLight => data.spot_light.is_some(),
            Self::Camera => data.camera_3d.is_some(),
            Self::Environment => data.environment.is_some(),
            Self::Water => data.water.is_some(),
            Self::Particles => data.particle_emitter.is_some(),
            Self::RigidBody => data.rigid_body.is_some(),
            Self::Collider => data.collider_3d.is_some(),
            Self::Character => data.character_controller.is_some(),
            Self::AudioSource => data.audio_source.is_some(),
        }
    }

    /// Adds a default instance of this component to `data` (no-op if present).
    pub fn add_to(self, data: &mut EntityData) {
        match self {
            Self::Mesh => {
                data.mesh_visual.get_or_insert(MeshVisualData {
                    primitive: PrimitiveData::Cube { size: 1.0 },
                    color: [0.85, 0.87, 0.92, 1.0],
                    surface: automata_scene::Surface::default(),
                });
            }
            Self::Animation => {
                if data.animation.is_none() {
                    data.animation = Some(default_animation(data));
                }
            }
            Self::Light => {
                data.directional_light
                    .get_or_insert_with(automata_scene::DirectionalLight::default);
            }
            Self::PointLight => {
                data.point_light
                    .get_or_insert_with(automata_scene::PointLight::default);
            }
            Self::SpotLight => {
                data.spot_light
                    .get_or_insert_with(automata_scene::SpotLight::default);
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
            Self::Water => {
                data.water
                    .get_or_insert_with(automata_renderer::Water::pond);
            }
            Self::Particles => {
                data.particle_emitter.get_or_insert_with(|| {
                    automata_scene::ParticleEmitter::preset("fire").unwrap_or_default()
                });
            }
            Self::RigidBody => {
                data.rigid_body
                    .get_or_insert_with(automata_physics::RigidBody::default);
            }
            Self::Collider => {
                if data.collider_3d.is_none() {
                    data.collider_3d = Some(fitted_collider(data));
                }
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
            Self::AudioSource => {
                data.audio_source
                    .get_or_insert_with(automata_core::AudioSource::default);
            }
        }
    }

    /// Copies this component's values from `src` to `dst` (adding it to
    /// `dst` if missing; removing it if `src` lacks it).
    pub fn copy_between(self, src: &EntityData, dst: &mut EntityData) {
        match self {
            Self::Mesh => dst.mesh_visual.clone_from(&src.mesh_visual),
            Self::Animation => dst.animation.clone_from(&src.animation),
            Self::Light => dst.directional_light = src.directional_light,
            Self::PointLight => dst.point_light = src.point_light,
            Self::SpotLight => dst.spot_light = src.spot_light,
            Self::Camera => dst.camera_3d = src.camera_3d,
            Self::Environment => dst.environment = src.environment,
            Self::Water => dst.water = src.water,
            Self::Particles => dst.particle_emitter.clone_from(&src.particle_emitter),
            Self::RigidBody => dst.rigid_body.clone_from(&src.rigid_body),
            Self::Collider => dst.collider_3d.clone_from(&src.collider_3d),
            Self::Character => {
                dst.character_controller
                    .clone_from(&src.character_controller);
                dst.character_velocity = src.character_velocity;
            }
            Self::AudioSource => dst.audio_source.clone_from(&src.audio_source),
        }
    }

    /// Removes this component from `data`.
    pub fn remove_from(self, data: &mut EntityData) {
        match self {
            Self::Mesh => data.mesh_visual = None,
            Self::Animation => data.animation = None,
            Self::Light => data.directional_light = None,
            Self::PointLight => data.point_light = None,
            Self::SpotLight => data.spot_light = None,
            Self::Camera => data.camera_3d = None,
            Self::Environment => data.environment = None,
            Self::Water => data.water = None,
            Self::Particles => data.particle_emitter = None,
            Self::RigidBody => data.rigid_body = None,
            Self::Collider => data.collider_3d = None,
            Self::Character => {
                data.character_controller = None;
                data.character_velocity = None;
            }
            Self::AudioSource => data.audio_source = None,
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
    /// A glowing lantern ringed by crates: glow light and its shadows.
    Lantern,
}

impl TestScene {
    /// Every test, in panel order.
    pub const ALL: [Self; 5] = [
        Self::ColorChart,
        Self::Shadows,
        Self::Depth,
        Self::Stress,
        Self::Lantern,
    ];

    /// The button label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::ColorChart => "Color chart",
            Self::Shadows => "Shadows",
            Self::Depth => "Fog depth",
            Self::Stress => "400 cubes",
            Self::Lantern => "Lantern",
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
            Self::Lantern => {
                "A glowing lantern among crates: glow light and its shadows (High and up)"
            }
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
            surface: automata_scene::Surface::default(),
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
        TestScene::Lantern => {
            let center = [0.0f32, 0.0, 18.0];
            out.push((
                "Test Floor".into(),
                test_box(
                    [center[0], -0.05, center[2]],
                    [10.0, 0.1, 10.0],
                    [0.55, 0.5, 0.45, 1.0],
                ),
            ));
            for i in 0..6 {
                let a = i as f32 * std::f32::consts::TAU / 6.0 + 0.3;
                let h = if i % 2 == 0 { 1.4 } else { 0.5 };
                let pos = [
                    center[0] + 1.6 * a.cos(),
                    h / 2.0,
                    center[2] + 1.6 * a.sin(),
                ];
                out.push((
                    "Test Crate".into(),
                    test_box(pos, [0.5, h, 0.5], [0.6, 0.45, 0.3, 1.0]),
                ));
            }
            let mut lantern = test_box([center[0], 0.6, center[2]], [1.0; 3], [0.1, 0.1, 0.1, 1.0]);
            if let Some(m) = &mut lantern.mesh_visual {
                m.primitive = PrimitiveData::Sphere { size: 0.3 };
                m.surface.emissive = [1.0, 0.6, 0.25];
                m.surface.emissive_strength = 8.0;
            }
            out.push(("Test Lantern".into(), lantern));
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
            ..Default::default()
        });
        // The sky with its sun, haze and clouds, on the sun itself (a fresh
        // scene shouldn't look like a void).
        doc.entities[sun].data.environment = Some(automata_renderer::Environment::daylight());
        let ground = doc.add_entity_named("Ground");
        doc.entities[ground].data.transform = Some(TransformData {
            position: [0.0, -0.1, 0.0],
            scale: [20.0, 0.2, 20.0],
            ..Default::default()
        });
        doc.entities[ground].data.mesh_visual = Some(MeshVisualData {
            primitive: PrimitiveData::Cube { size: 1.0 },
            color: [0.16, 0.17, 0.20, 1.0],
            surface: automata_scene::Surface::default(),
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
            surface: automata_scene::Surface::default(),
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
            surface: automata_scene::Surface::default(),
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

    /// Creates an entity showing the model at `rel` (scene-relative), named
    /// `name`, standing on `at` (its lowest point at `at.y`), and selects it.
    /// One undo step.
    pub fn create_model(&mut self, rel: &str, name: &str, at: [f32; 3]) -> u64 {
        let mut data = EntityData {
            name: Some(self.unique_name(name)),
            transform: Some(TransformData::default()),
            mesh_visual: Some(MeshVisualData {
                primitive: PrimitiveData::Model {
                    path: rel.replace('\\', "/"),
                },
                color: [1.0; 4],
                surface: automata_scene::Surface::default(),
            }),
            ..Default::default()
        };
        fit_animation(&mut data);
        let lift = mesh_bounds(&data).map_or(0.0, |(c, s)| -(c.y - s.y * 0.5));
        if let Some(t) = &mut data.transform {
            t.position = [at[0], at[1] + lift, at[2]];
        }
        self.push_undo();
        let id = self.alloc_id();
        self.entities.push(EntityEntry { id, data });
        self.selected = Some(id);
        self.touch();
        id
    }

    /// Points entity `id`'s mesh at the model `rel` (adding a mesh if it has
    /// none; a primitive's color becomes the model's white tint).
    pub fn set_model(&mut self, id: u64, rel: &str) -> bool {
        let rel = rel.replace('\\', "/");
        self.modify(id, false, |d| {
            let color = match &d.mesh_visual {
                Some(m) if matches!(m.primitive, PrimitiveData::Model { .. }) => m.color,
                _ => [1.0; 4],
            };
            d.mesh_visual = Some(MeshVisualData {
                primitive: PrimitiveData::Model { path: rel.clone() },
                color,
                surface: automata_scene::Surface::default(),
            });
            fit_animation(d);
        })
    }

    /// Creates an entity at `at` with an Audio Source playing `rel` on start
    /// (3D, so it fades with distance), named `name`, and selects it. One
    /// undo step.
    pub fn create_sound(&mut self, rel: &str, name: &str, at: [f32; 3]) -> u64 {
        let data = EntityData {
            name: Some(self.unique_name(name)),
            transform: Some(TransformData {
                position: at,
                ..TransformData::default()
            }),
            audio_source: Some(automata_core::AudioSource {
                play_on_start: true,
                spatial: true,
                ..automata_core::AudioSource::new(rel.replace('\\', "/"))
            }),
            ..Default::default()
        };
        self.push_undo();
        let id = self.alloc_id();
        self.entities.push(EntityEntry { id, data });
        self.selected = Some(id);
        self.touch();
        id
    }

    /// Gives entity `id` an Audio Source playing `rel` (keeping the
    /// source's other settings when it already has one).
    pub fn set_audio_clip(&mut self, id: u64, rel: &str) -> bool {
        let rel = rel.replace('\\', "/");
        self.modify(id, false, |d| {
            d.audio_source
                .get_or_insert_with(automata_core::AudioSource::default)
                .clip
                .clone_from(&rel);
        })
    }

    /// Refits entity `id`'s collider to its mesh, keeping the collider's
    /// material settings and its kind: a box is refitted to the bounds, a
    /// Mesh or Convex Hull takes the entity's current model, and a sphere
    /// or capsule is replaced by the automatic choice (see
    /// [`fitted_collider`]). Returns true when it changed anything.
    pub fn fit_collider(&mut self, id: u64) -> bool {
        self.refit_collider(id, false)
    }

    /// [`fit_collider`](Self::fit_collider) after the entity's mesh was
    /// swapped: a box was chosen for the old mesh, so it's chosen afresh.
    pub fn fit_collider_to_new_mesh(&mut self, id: u64) -> bool {
        self.refit_collider(id, true)
    }

    fn refit_collider(&mut self, id: u64, rechoose_box: bool) -> bool {
        use automata_physics::Shape3D;
        let has = self
            .index_of(id)
            .is_some_and(|i| self.entities[i].data.collider_3d.is_some());
        has && self.modify(id, false, |d| {
            let model = model_path(d).map(str::to_string);
            let keep_box = !rechoose_box
                && matches!(
                    d.collider_3d.as_ref().map(|c| &c.shape),
                    Some(Shape3D::Cuboid { .. })
                );
            let fitted = if keep_box {
                fitted_box(d)
            } else {
                fitted_collider(d)
            };
            if let Some(c) = &mut d.collider_3d {
                match (&mut c.shape, model) {
                    (
                        Shape3D::Mesh { model: current } | Shape3D::ConvexHull { model: current },
                        Some(model),
                    ) => {
                        *current = model;
                        c.offset = [0.0; 3];
                    }
                    _ => {
                        c.shape = fitted.shape;
                        c.offset = fitted.offset;
                    }
                }
            }
        })
    }

    /// Creates a camera entity looking through `view` (one undo step), so a
    /// new camera starts out framing what the user is looking at.
    pub fn create_camera_at(&mut self, view: automata_renderer::Camera3D) -> u64 {
        let id = self.create(CreateKind::Camera);
        if let Some(i) = self.index_of(id) {
            let data = &mut self.entities[i].data;
            if let Some(cam) = &mut data.camera_3d {
                cam.position = view.position;
                cam.target = view.target;
            }
            if let Some(t) = &mut data.transform {
                t.position = view.position.to_array();
            }
            self.touch();
        }
        id
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
            CreateKind::Sphere => {
                if let Some(t) = &mut data.transform {
                    t.position = [0.0, 0.5, 0.0];
                }
                data.mesh_visual = Some(MeshVisualData {
                    primitive: PrimitiveData::Sphere { size: 1.0 },
                    color: [1.0, 1.0, 1.0, 1.0],
                    surface: automata_scene::Surface::default(),
                });
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
                    surface: automata_scene::Surface::default(),
                });
            }
            CreateKind::Light => {
                if let Some(t) = &mut data.transform {
                    t.position = [0.0, 5.0, 0.0];
                }
                ComponentKind::Light.add_to(&mut data);
            }
            CreateKind::PointLight => {
                if let Some(t) = &mut data.transform {
                    t.position = [0.0, 2.0, 0.0];
                }
                ComponentKind::PointLight.add_to(&mut data);
            }
            CreateKind::SpotLight => {
                if let Some(t) = &mut data.transform {
                    // Forward (-Z) tipped down 60°: a lamp lighting the
                    // ground in front of it.
                    t.position = [0.0, 3.0, 2.0];
                    t.rotation_euler = [-60f32.to_radians(), 0.0, 0.0];
                }
                ComponentKind::SpotLight.add_to(&mut data);
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
            CreateKind::Water => {
                ComponentKind::Water.add_to(&mut data);
            }
            CreateKind::Particles => {
                ComponentKind::Particles.add_to(&mut data);
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
                    surface: automata_scene::Surface::default(),
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

    /// The directory scene-relative paths are resolved against, always
    /// absolute (a scene opened as `oberon examples/fps/level.ron` must
    /// not hand the script runner, which runs inside this folder, paths
    /// relative to Oberon's own working directory).
    #[must_use]
    pub fn base_dir(&self) -> PathBuf {
        let dir = self
            .path
            .as_ref()
            .and_then(|p| p.parent())
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(
                || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                Path::to_path_buf,
            );
        std::path::absolute(&dir).unwrap_or(dir)
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

    /// After a file or folder moved on disk from `from` to `to`, points
    /// every script, model and sound reference at the new place (files
    /// inside a moved folder included). One undo step; returns how many
    /// entities changed.
    pub fn retarget_paths(&mut self, from: &Path, to: &Path) -> usize {
        let old = self.relative(from);
        let new = self.relative(to);
        let moved = |path: &str| -> Option<String> {
            let rel = path.replace('\\', "/");
            if rel == old {
                Some(new.clone())
            } else {
                rel.strip_prefix(&format!("{old}/"))
                    .map(|rest| format!("{new}/{rest}"))
            }
        };
        let mut changes = Vec::new();
        for (i, e) in self.entities.iter().enumerate() {
            let mut data = e.data.clone();
            let mut changed = false;
            if let Some(script) = &mut data.script {
                if let Some(p) = moved(&script.path) {
                    script.path = p;
                    changed = true;
                }
            }
            if let Some(MeshVisualData {
                primitive: PrimitiveData::Model { path },
                ..
            }) = &mut data.mesh_visual
            {
                if let Some(p) = moved(path) {
                    *path = p;
                    changed = true;
                }
            }
            if let Some(audio) = &mut data.audio_source {
                if let Some(p) = moved(&audio.clip) {
                    audio.clip = p;
                    changed = true;
                }
            }
            if changed {
                changes.push((i, data));
            }
        }
        if changes.is_empty() {
            return 0;
        }
        self.push_undo();
        let n = changes.len();
        for (i, data) in changes {
            self.entities[i].data = data;
        }
        self.touch();
        n
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
pub(crate) mod tests {

    #[test]
    fn moved_files_keep_their_references() {
        let root = std::env::temp_dir().join(format!("oberon-retarget-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("scripts")).unwrap();
        std::fs::write(root.join("scripts/coin.py"), "").unwrap();
        let mut doc = Document::new();
        doc.path = Some(root.join("level.ron"));
        let id = doc.create(CreateKind::Cube);
        doc.modify(id, false, |d| {
            d.script = Some(automata_core::Script {
                path: "scripts/coin.py".into(),
            });
        });
        // The folder is renamed on disk, then the scene follows.
        std::fs::rename(root.join("scripts"), root.join("code")).unwrap();
        assert_eq!(
            doc.retarget_paths(&root.join("scripts"), &root.join("code")),
            1
        );
        let i = doc.index_of(id).unwrap();
        assert_eq!(
            doc.entities[i].data.script.as_ref().unwrap().path,
            "code/coin.py"
        );
        // One undo step brings the old reference back.
        doc.undo();
        let i = doc.index_of(id).unwrap();
        assert_eq!(
            doc.entities[i].data.script.as_ref().unwrap().path,
            "scripts/coin.py"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// A closed ramp (a wedge rising along +X, `y = x` over a 1 m
    /// footprint: half its bounding box), written to a temp file. Returns
    /// its absolute path.
    pub(crate) fn ramp_model(tag: &str) -> String {
        let dir = std::env::temp_dir().join(format!("oberon-ramp-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ramp.obj");
        std::fs::write(
            &path,
            "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 0 1\nv 1 0 1\nv 1 1 1\n\
             f 1 3 2\nf 4 5 6\nf 1 2 5\nf 1 5 4\nf 2 3 6\nf 2 6 5\nf 1 4 6\nf 1 6 3\n",
        )
        .unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn relative_scene_paths_resolve_to_absolute_ones() {
        // `oberon examples/fps/level.ron` from the repository root.
        let mut doc = Document::new();
        doc.path = Some(PathBuf::from("examples/fps/level.ron"));
        let base = doc.base_dir();
        assert!(base.is_absolute(), "{}", base.display());
        assert!(base.ends_with("examples/fps"));
        let script = doc.resolve("scripts/player.py");
        assert!(script.is_absolute());
        assert!(script.ends_with("examples/fps/scripts/player.py"));
    }

    #[test]
    fn models_a_box_fits_badly_get_mesh_or_hull_colliders() {
        use crate::fields::{choice_label, choice_options, set_choice, Choice};
        use automata_physics::{BodyType, RigidBody, Shape3D};
        let path = ramp_model("auto");
        let mut doc = Document::new();
        let id = doc.create(CreateKind::Cube);
        assert!(doc.set_model(id, &path));
        let data = |doc: &Document| doc.entities[doc.index_of(id).unwrap()].data.clone();

        // Static scenery: the ramp's own triangles.
        assert!(doc.add_component(id, ComponentKind::Collider));
        let c = data(&doc).collider_3d.unwrap();
        assert_eq!(
            c.shape,
            Shape3D::Mesh {
                model: path.clone()
            }
        );
        assert_eq!(c.offset, [0.0; 3]);
        assert_eq!(
            choice_label(&data(&doc), Choice::ColliderShape),
            Some("Mesh")
        );

        // A dynamic body moves, so it gets the convex hull; the Inspector
        // flags a mesh left on it.
        doc.modify(id, false, |d| d.rigid_body = Some(RigidBody::dynamic()));
        let shape = data(&doc).collider_3d.unwrap().shape;
        assert!(automata_physics::collider_problem(&shape, Some(BodyType::Dynamic)).is_some());
        doc.modify(id, false, |d| d.collider_3d = None);
        assert!(doc.add_component(id, ComponentKind::Collider));
        assert_eq!(
            data(&doc).collider_3d.unwrap().shape,
            Shape3D::ConvexHull {
                model: path.clone()
            }
        );

        // Fit keeps the kind; the dropdown offers every shape and Box
        // comes back fitted to the bounds.
        // (Refitting to the same model changes nothing.)
        assert!(!doc.fit_collider(id));
        assert_eq!(
            choice_label(&data(&doc), Choice::ColliderShape),
            Some("Convex Hull")
        );
        assert_eq!(
            choice_options(&data(&doc), Choice::ColliderShape),
            ["Convex Hull", "Box", "Sphere", "Capsule", "Mesh"]
        );
        let mut d = data(&doc);
        assert!(set_choice(&mut d, Choice::ColliderShape, "Box"));
        let c = d.collider_3d.unwrap();
        let Shape3D::Cuboid { extents } = c.shape else {
            panic!("box expected: {c:?}");
        };
        assert!((extents - glam::Vec3::ONE).length() < 1e-5, "{extents}");
        assert!((glam::Vec3::from(c.offset) - glam::Vec3::splat(0.5)).length() < 1e-5);
        // A box picked by hand survives Fit; swapping the mesh re-chooses.
        doc.modify(id, false, |dd| dd.collider_3d = Some(c.clone()));
        assert!(!doc.fit_collider(id));
        assert!(matches!(
            data(&doc).collider_3d.unwrap().shape,
            Shape3D::Cuboid { .. }
        ));
        assert!(doc.fit_collider_to_new_mesh(id));
        assert_eq!(
            data(&doc).collider_3d.unwrap().shape,
            Shape3D::ConvexHull {
                model: path.clone()
            }
        );

        // A plain cube boxes well and never offers model shapes.
        let cube = doc.create(CreateKind::Cube);
        assert!(doc.add_component(cube, ComponentKind::Collider));
        let cube_data = doc.entities[doc.index_of(cube).unwrap()].data.clone();
        assert_eq!(
            choice_options(&cube_data, Choice::ColliderShape),
            ["Box", "Sphere", "Capsule"]
        );
    }

    /// A triangle model spanning x 0..1, y 1..3 (not centred on its
    /// origin), written to a temp file. Returns its absolute path.
    fn offset_model(tag: &str) -> String {
        let dir = std::env::temp_dir().join(format!("oberon-doc-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("spire.obj");
        std::fs::write(&path, "v 0 1 0\nv 1 1 0\nv 0 3 0\nf 1 2 3\n").unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn models_are_created_standing_on_the_drop_point() {
        let path = offset_model("create");
        let mut doc = Document::new();
        let id = doc.create_model(&path, "Spire", [2.0, 0.0, -1.0]);
        let e = &doc.entities[doc.index_of(id).unwrap()];
        assert_eq!(e.name(), "Spire");
        assert_eq!(doc.selected, Some(id));
        // The lowest point (local y = 1) lands on y = 0.
        let p = e.data.transform.unwrap().position;
        assert!(
            (p[0] - 2.0).abs() < 1e-5 && (p[1] + 1.0).abs() < 1e-5,
            "{p:?}"
        );
        let (center, size) = mesh_bounds(&e.data).unwrap();
        assert!((center - glam::Vec3::new(0.5, 2.0, 0.0)).length() < 1e-5);
        assert!((size - glam::Vec3::new(1.0, 2.0, 0.0)).length() < 1e-5);
        // A second one gets a fresh name; one undo removes it.
        let id2 = doc.create_model(&path, "Spire", [0.0; 3]);
        assert_eq!(doc.entities[doc.index_of(id2).unwrap()].name(), "Spire 2");
        assert!(doc.undo());
        assert!(doc.index_of(id2).is_none());
    }

    #[test]
    fn colliders_fit_the_model_bounds_and_follow_a_swap() {
        let path = offset_model("fit");
        let mut doc = Document::new();
        let id = doc.create(CreateKind::Cube);
        doc.modify(id, false, |d| {
            if let Some(t) = &mut d.transform {
                t.scale = [2.0, 2.0, 2.0];
            }
        });
        doc.modify(id, false, |d| ComponentKind::Collider.add_to(d));
        let collider = |doc: &Document| {
            doc.entities[doc.index_of(id).unwrap()]
                .data
                .collider_3d
                .clone()
        };
        // A cube of size 1 at scale 2: a 2 m box, centred.
        let c = collider(&doc).unwrap();
        assert!(
            matches!(c.shape, automata_physics::Shape3D::Cuboid { extents } if (extents.x - 2.0).abs() < 1e-5),
            "{c:?}"
        );
        assert_eq!(c.offset, [0.0; 3]);

        // Swap the mesh for the model: tint resets to white, then refit.
        assert!(doc.set_model(id, &path));
        let mesh = doc.entities[doc.index_of(id).unwrap()]
            .data
            .mesh_visual
            .clone()
            .unwrap();
        assert_eq!(mesh.color, [1.0; 4]);
        assert!(matches!(mesh.primitive, PrimitiveData::Model { .. }));
        assert!(doc.fit_collider(id));
        let c = collider(&doc).unwrap();
        // Bounds centre (0.5, 2, 0) × scale 2 = offset (1, 4, 0).
        assert!(
            (c.offset[0] - 1.0).abs() < 1e-5 && (c.offset[1] - 4.0).abs() < 1e-5,
            "{c:?}"
        );
        let automata_physics::Shape3D::Cuboid { extents } = c.shape else {
            panic!("box collider expected: {c:?}");
        };
        // Size (1, 2, ~0) × 2 → full extents (2, 4, 0.01 floor).
        assert!((extents.x - 2.0).abs() < 1e-5 && (extents.y - 4.0).abs() < 1e-5);
        assert!(extents.z > 0.0);
        // Without a collider there is nothing to fit.
        let bare = doc.create(CreateKind::Cube);
        assert!(!doc.fit_collider(bare));
    }

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
        assert_eq!(doc.spawn_test(TestScene::Lantern), 8);
        assert_eq!(doc.clear_tests(), 400 + 7 + 8);
        assert_eq!(doc.entities.len(), before);
        assert!(doc.undo());
        assert_eq!(doc.entities.len(), before + 415);
    }

    #[test]
    fn new_cameras_frame_the_current_view_in_one_undo_step() {
        let mut doc = Document::new();
        let view = automata_renderer::Camera3D::new(
            glam::Vec3::new(4.0, 5.0, 6.0),
            glam::Vec3::new(1.0, 0.0, 0.0),
        );
        let id = doc.create_camera_at(view);
        let data = &doc.entities[doc.index_of(id).unwrap()].data;
        assert_eq!(data.camera_3d.unwrap().position, view.position);
        assert_eq!(data.camera_3d.unwrap().target, view.target);
        assert_eq!(data.transform.unwrap().position, [4.0, 5.0, 6.0]);
        assert!(doc.undo());
        assert!(doc.entities.is_empty());
    }

    #[test]
    fn created_lights_light_the_scene_and_the_spot_aims_down_ahead() {
        let mut doc = Document::new();
        let point = doc.create(CreateKind::PointLight);
        let spot = doc.create(CreateKind::SpotLight);
        let data = |id| doc.entities[doc.index_of(id).unwrap()].data.clone();
        assert!(ComponentKind::PointLight.present_in(&data(point)));
        assert!(ComponentKind::SpotLight.present_in(&data(spot)));
        assert_eq!(data(point).name.as_deref(), Some("Point Light"));
        let mut world = automata_scene::SceneWorld::new();
        doc.to_scene_file().apply_to_world(&mut world);
        let mut draw = automata_renderer::DrawList::new();
        automata_scene::record_draw_list(&world, &mut draw);
        let lights = draw.point_lights();
        assert_eq!(lights.len(), 2);
        let cone = lights.iter().find_map(|l| l.spot).unwrap();
        let dir = cone.axis();
        assert!(dir.y < -0.8 && dir.z < -0.4, "down and forward: {dir}");
        // Copy and remove round out the component.
        let mut bare = EntityData::default();
        ComponentKind::SpotLight.copy_between(&data(spot), &mut bare);
        assert_eq!(bare.spot_light, data(spot).spot_light);
        ComponentKind::SpotLight.remove_from(&mut bare);
        assert!(bare.spot_light.is_none());
    }

    #[test]
    fn created_particles_burn_in_the_world() {
        let mut doc = Document::new();
        let id = doc.create(CreateKind::Particles);
        let data = doc.entities[doc.index_of(id).unwrap()].data.clone();
        assert!(ComponentKind::Particles.present_in(&data));
        assert_eq!(data.name.as_deref(), Some("Particles"));
        assert_eq!(
            data.particle_emitter
                .as_ref()
                .and_then(automata_scene::ParticleEmitter::preset_name),
            Some("Fire")
        );
        let mut world = automata_scene::SceneWorld::new();
        doc.to_scene_file().apply_to_world(&mut world);
        for _ in 0..30 {
            automata_scene::simulate_particles(&mut world, 1.0 / 30.0);
        }
        let live: usize = world
            .query::<&automata_scene::ParticleState>()
            .iter()
            .map(automata_scene::ParticleState::len)
            .sum();
        assert!(live > 10, "a second of fire: {live}");
        let mut bare = EntityData::default();
        ComponentKind::Particles.copy_between(&data, &mut bare);
        assert_eq!(bare.particle_emitter, data.particle_emitter);
        ComponentKind::Particles.remove_from(&mut bare);
        assert!(bare.particle_emitter.is_none());
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
