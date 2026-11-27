//! The Oberon scene document: the editor's model of the scene being edited.
//!
//! A document is a [`SceneFile`] (the engine's serde RON data) plus editor
//! state: stable per-entity ids, a selection, dirty tracking, and an undo
//! stack of serialized snapshots. All editing goes through methods that
//! record undo and mark the document dirty, so the UI never touches the
//! file data directly.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use automata_core::prefab::{self, PrefabLink};
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
    /// The selected entity id, if any: the primary selection, which the
    /// Inspector shows and the gizmo sits on.
    pub selected: Option<u64>,
    /// The other selected entities (multi-select), oldest first.
    also_selected: Vec<u64>,
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
    /// While edits are batched into one undo step: whether it was taken.
    batch: Option<bool>,
    /// Each prefab link's entity as last synced (resolved through
    /// variants), keyed by path and index.
    prefab_bases: HashMap<(String, u32), EntityData>,
    /// Prefab problems (unreadable files) for the app to show, once each.
    pub notes: Vec<String>,
    /// Where the bones attached entities follow are in the world (position
    /// and rotation), by entity id, as the viewport last posed them: an
    /// attached entity's transform is its offset from that.
    attach_frames: HashMap<u64, glam::Mat4>,
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

/// Euler angles for `t` turned `angle` radians about the world Y axis.
fn turn_about_y(t: &TransformData, angle: f32) -> [f32; 3] {
    if angle == 0.0 {
        return t.rotation_euler;
    }
    let [x, y, z] = t.rotation_euler;
    let q =
        glam::Quat::from_rotation_y(angle) * glam::Quat::from_euler(glam::EulerRot::XYZ, x, y, z);
    let (ex, ey, ez) = q.to_euler(glam::EulerRot::XYZ);
    [ex, ey, ez]
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
    /// A 10 m field of grass to paint more onto.
    Grass,
    /// A 64 m flat terrain to sculpt and paint.
    Terrain,
    /// A particle emitter (a campfire's flames to start with).
    Particles,
    /// A cube with a dynamic rigid body and a box collider.
    PhysicsCube,
    /// A 1 m sphere.
    Sphere,
    /// A playable character: WASD / arrows to move, Space to jump in Play
    /// mode.
    Player,
    /// A 2D picture to set an image on.
    Sprite,
    /// A 16×9 tile grid with a floor painted, solid for 2D physics.
    Tilemap,
    /// A 2D camera following the entity called "Player".
    Camera2D,
    /// A 2D player: a sprite with a 2D collider and a character
    /// controller (arrows / A, D to run, Space to jump in Play mode).
    Player2D,
}

impl CreateKind {
    /// Every kind, in menu order.
    pub const ALL: [Self; 19] = [
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
        Self::Grass,
        Self::Terrain,
        Self::Particles,
        Self::PhysicsCube,
        Self::Player,
        Self::Sprite,
        Self::Tilemap,
        Self::Camera2D,
        Self::Player2D,
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
            Self::Grass => "Grass",
            Self::Terrain => "Terrain",
            Self::Particles => "Particles",
            Self::PhysicsCube => "Physics Cube",
            Self::Player => "Player",
            Self::Sprite => "Sprite",
            Self::Tilemap => "Tilemap",
            Self::Camera2D => "Camera 2D",
            Self::Player2D => "2D Player",
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
    /// A field of grass blades.
    Grass,
    /// Draws the entity's model at many spots on the ground.
    Scatter,
    /// A heightmap ground, sculpted and painted with brushes.
    Terrain,
    /// Particle emitter: fire, smoke, sparks, dust …
    Particles,
    /// Rigid body.
    RigidBody,
    /// 3D collider.
    Collider,
    /// Character controller (player movement in Play mode).
    Character,
    /// Holds the entity's body to another entity's (or to the world).
    Joint,
    /// A sound the entity plays.
    AudioSource,
    /// Follows a bone of another entity's model (a sword in a hand).
    Attach,
    /// Lets an animated model go limp and fall under physics.
    Ragdoll,
    /// Finds its own way around the navigation mesh.
    NavAgent,
    /// The scene's navigation mesh: how it's baked, and the Bake button.
    Navigation,
    /// A hand-placed shortcut for nav agents to another entity.
    NavLink,
    /// A 2D picture (or a frame of a sprite sheet) with animation clips.
    Sprite,
    /// A grid of tiles painted from a tileset; solid for 2D physics.
    Tilemap,
    /// A 2D (orthographic) camera looking down -Z.
    Camera2D,
    /// Moves the entity's 2D camera after a target, smoothly.
    CameraFollow2D,
    /// A 2D collider (box, circle or capsule in the XY plane).
    Collider2D,
}

impl ComponentKind {
    /// Every addable component, in menu order.
    pub const ALL: [Self; 27] = [
        Self::Mesh,
        Self::Animation,
        Self::Light,
        Self::PointLight,
        Self::SpotLight,
        Self::Camera,
        Self::Environment,
        Self::Water,
        Self::Grass,
        Self::Scatter,
        Self::Terrain,
        Self::Particles,
        Self::RigidBody,
        Self::Collider,
        Self::Character,
        Self::Joint,
        Self::AudioSource,
        Self::Attach,
        Self::Ragdoll,
        Self::NavAgent,
        Self::Navigation,
        Self::NavLink,
        Self::Sprite,
        Self::Tilemap,
        Self::Camera2D,
        Self::CameraFollow2D,
        Self::Collider2D,
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
            Self::Grass => "Grass",
            Self::Scatter => "Scatter",
            Self::Terrain => "Terrain",
            Self::Particles => "Particle Emitter",
            Self::RigidBody => "Rigidbody",
            Self::Collider => "Collider",
            Self::Character => "Character Controller",
            Self::Joint => "Joint",
            Self::AudioSource => "Audio Source",
            Self::Attach => "Attach To Bone",
            Self::Ragdoll => "Ragdoll",
            Self::NavAgent => "Nav Agent",
            Self::Navigation => "Navigation",
            Self::NavLink => "Nav Link",
            Self::Sprite => "Sprite",
            Self::Tilemap => "Tilemap",
            Self::Camera2D => "Camera 2D",
            Self::CameraFollow2D => "Follow Camera 2D",
            Self::Collider2D => "Collider 2D",
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
            Self::Grass => data.grass.is_some(),
            Self::Scatter => data.scatter.is_some(),
            Self::Terrain => data.terrain.is_some(),
            Self::Particles => data.particle_emitter.is_some(),
            Self::RigidBody => data.rigid_body.is_some(),
            Self::Collider => data.collider_3d.is_some(),
            Self::Character => data.character_controller.is_some(),
            Self::Joint => data.joint.is_some(),
            Self::AudioSource => data.audio_source.is_some(),
            Self::Attach => data.attach.is_some(),
            Self::Ragdoll => data.ragdoll.is_some(),
            Self::NavAgent => data.nav_agent.is_some(),
            Self::Navigation => data.navigation.is_some(),
            Self::NavLink => data.nav_link.is_some(),
            Self::Sprite => data.sprite.is_some(),
            Self::Tilemap => data.tilemap.is_some(),
            Self::Camera2D => data.camera_2d.is_some(),
            Self::CameraFollow2D => data.camera_follow_2d.is_some(),
            Self::Collider2D => data.collider_2d.is_some(),
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
                    draw_distance: 0.0,
                    material: String::new(),
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
            Self::Grass => {
                data.grass
                    .get_or_insert_with(|| automata_scene::Grass::area(10.0, 10.0));
            }
            Self::Scatter => {
                data.scatter
                    .get_or_insert_with(|| automata_scene::Scatter::area(10.0, 10.0));
            }
            Self::Terrain => {
                data.terrain
                    .get_or_insert_with(automata_scene::Terrain::default);
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
                if is_2d(data) {
                    Self::Collider2D.add_to(data);
                } else {
                    Self::Collider.add_to(data);
                }
                data.character_controller.get_or_insert_with(|| {
                    automata_physics::CharacterController::new().snap_to_ground(0.3)
                });
                data.character_velocity
                    .get_or_insert_with(automata_physics::CharacterVelocity::default);
            }
            Self::Joint => {
                // A joint holds a body: give it a dynamic one and a collider.
                if data.rigid_body.is_none() {
                    data.rigid_body = Some(automata_physics::RigidBody::dynamic());
                }
                Self::Collider.add_to(data);
                data.joint
                    .get_or_insert_with(automata_physics::Joint::default);
            }
            Self::AudioSource => {
                data.audio_source
                    .get_or_insert_with(automata_core::AudioSource::default);
            }
            Self::Attach => {
                data.attach
                    .get_or_insert_with(automata_core::scene_file::AttachData::default);
            }
            Self::Ragdoll => {
                data.ragdoll
                    .get_or_insert_with(automata_scene::Ragdoll::default);
            }
            Self::NavAgent => {
                data.nav_agent
                    .get_or_insert_with(automata_nav::NavAgent::default);
            }
            Self::Navigation => {
                data.navigation
                    .get_or_insert_with(automata_nav::Navigation::default);
            }
            Self::NavLink => {
                data.nav_link
                    .get_or_insert_with(automata_nav::NavLink::default);
            }
            Self::Sprite => {
                data.sprite
                    .get_or_insert_with(automata_scene::Sprite2D::default);
            }
            Self::Tilemap => {
                data.tilemap
                    .get_or_insert_with(automata_scene::Tilemap::default);
            }
            Self::Camera2D => {
                data.camera_2d
                    .get_or_insert_with(|| automata_renderer::Camera2D::new(glam::Vec2::ZERO, 5.0));
            }
            Self::CameraFollow2D => {
                Self::Camera2D.add_to(data);
                data.camera_follow_2d
                    .get_or_insert_with(automata_scene::CameraFollow2D::default);
            }
            Self::Collider2D => {
                if data.collider_2d.is_none() {
                    // Fits the sprite (a little narrower, so it doesn't
                    // snag on walls by its transparent edges).
                    let [w, h] = data.sprite.as_ref().map_or([1.0, 1.0], |s| s.size);
                    data.collider_2d = Some(automata_physics::Collider2D::cuboid(
                        (w * 0.8).max(0.05),
                        h.max(0.05),
                    ));
                }
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
            Self::Grass => dst.grass.clone_from(&src.grass),
            Self::Scatter => dst.scatter.clone_from(&src.scatter),
            Self::Terrain => dst.terrain.clone_from(&src.terrain),
            Self::Particles => dst.particle_emitter.clone_from(&src.particle_emitter),
            Self::RigidBody => dst.rigid_body.clone_from(&src.rigid_body),
            Self::Collider => dst.collider_3d.clone_from(&src.collider_3d),
            Self::Character => {
                dst.character_controller
                    .clone_from(&src.character_controller);
                dst.character_velocity = src.character_velocity;
            }
            Self::Joint => dst.joint.clone_from(&src.joint),
            Self::AudioSource => dst.audio_source.clone_from(&src.audio_source),
            Self::Attach => dst.attach.clone_from(&src.attach),
            Self::Ragdoll => dst.ragdoll.clone_from(&src.ragdoll),
            Self::NavAgent => dst.nav_agent.clone_from(&src.nav_agent),
            Self::Navigation => dst.navigation.clone_from(&src.navigation),
            Self::NavLink => dst.nav_link.clone_from(&src.nav_link),
            Self::Sprite => dst.sprite.clone_from(&src.sprite),
            Self::Tilemap => dst.tilemap.clone_from(&src.tilemap),
            Self::Camera2D => dst.camera_2d = src.camera_2d,
            Self::CameraFollow2D => dst.camera_follow_2d.clone_from(&src.camera_follow_2d),
            Self::Collider2D => dst.collider_2d.clone_from(&src.collider_2d),
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
            Self::Grass => data.grass = None,
            Self::Scatter => data.scatter = None,
            Self::Terrain => data.terrain = None,
            Self::Particles => data.particle_emitter = None,
            Self::RigidBody => data.rigid_body = None,
            Self::Collider => data.collider_3d = None,
            Self::Character => {
                data.character_controller = None;
                data.character_velocity = None;
            }
            Self::Joint => data.joint = None,
            Self::AudioSource => data.audio_source = None,
            Self::Attach => data.attach = None,
            Self::Ragdoll => data.ragdoll = None,
            Self::NavAgent => data.nav_agent = None,
            Self::Navigation => data.navigation = None,
            Self::NavLink => data.nav_link = None,
            Self::Sprite => data.sprite = None,
            Self::Tilemap => data.tilemap = None,
            Self::Camera2D => {
                data.camera_2d = None;
                data.camera_follow_2d = None;
            }
            Self::CameraFollow2D => data.camera_follow_2d = None,
            Self::Collider2D => data.collider_2d = None,
        }
    }
}

/// Whether the scene is a 2D one: 2D entities and no meshes or terrain
/// (opening it starts in the flat view).
#[must_use]
pub fn scene_is_2d(doc: &Document) -> bool {
    doc.entities.iter().any(|e| is_2d(&e.data))
        && !doc
            .entities
            .iter()
            .any(|e| e.data.mesh_visual.is_some() || e.data.terrain.is_some())
}

/// Whether an entity belongs to the 2D world: it shows a sprite or a
/// tilemap, or has a 2D collider or camera (and no 3D collider).
#[must_use]
pub fn is_2d(data: &EntityData) -> bool {
    data.collider_3d.is_none()
        && (data.sprite.is_some()
            || data.tilemap.is_some()
            || data.collider_2d.is_some()
            || data.camera_2d.is_some())
}

/// What a brush stroke does: paint or erase foliage, or sculpt or paint a
/// terrain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BrushTool {
    /// Paints Grass (or a Scatter's copies) on.
    Paint,
    /// Cuts it away.
    Erase,
    /// Raises the terrain.
    Raise,
    /// Lowers it.
    Lower,
    /// Evens out its bumps.
    Smooth,
    /// Levels it to where the stroke began.
    Flatten,
    /// Roughens it.
    Noise,
    /// Paints one of its four layers (0..3).
    Layer(u8),
    /// Paints a tilemap's cells with this value (0 erases; n > 0 is
    /// tile n-1, n < 0 auto-tile -n-1).
    Tile(i32),
}

impl BrushTool {
    /// The terrain sculpting tools, in Inspector order.
    pub const SCULPT: [Self; 5] = [
        Self::Raise,
        Self::Lower,
        Self::Smooth,
        Self::Flatten,
        Self::Noise,
    ];

    /// Whether it works on a Terrain (rather than Grass or a Scatter).
    #[must_use]
    pub fn is_terrain(self) -> bool {
        !matches!(self, Self::Paint | Self::Erase | Self::Tile(_))
    }

    /// Whether it paints a Tilemap's cells.
    #[must_use]
    pub fn is_tile(self) -> bool {
        matches!(self, Self::Tile(_))
    }

    /// Its button label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Paint => "Paint",
            Self::Erase | Self::Tile(0) => "Erase",
            Self::Raise => "Raise",
            Self::Lower => "Lower",
            Self::Smooth => "Smooth",
            Self::Flatten => "Flatten",
            Self::Noise => "Noise",
            Self::Layer(i) => ["Grass", "Rock", "Sand", "Dirt"][usize::from(i.min(3))],
            Self::Tile(_) => "Tile",
        }
    }
}

/// One terrain brush dab at world `point` (see [`Document::brush`]): how
/// far each tool goes per dab scales with the brush, so strokes feel the
/// same at any size.
fn terrain_dab(
    terrain: &mut automata_scene::Terrain,
    model: glam::Mat4,
    tool: BrushTool,
    point: glam::Vec3,
    radius: f32,
    strength: f32,
    anchor: glam::Vec3,
) {
    use automata_scene::Sculpt;
    let local = model.inverse();
    let p = local.transform_point3(point);
    let (scale, _, _) = model.to_scale_rotation_translation();
    let spread = f32::midpoint(scale.x.abs(), scale.z.abs()).max(1e-3);
    let up = scale.y.abs().max(1e-3);
    let strength = strength.clamp(0.0, 1.0);
    // How far a full-strength dab moves the ground at its centre: 2% of
    // the (world) radius, so a drag five brushes long lifts about half a
    // radius, whatever the brush size. In the entity's space.
    let lift = radius * 0.02 * strength / up;
    let radius = radius / spread;
    let centre = glam::Vec2::new(p.x, p.z);
    match tool {
        BrushTool::Raise => {
            terrain.sculpt(Sculpt::Raise, centre, radius, lift);
        }
        BrushTool::Lower => {
            terrain.sculpt(Sculpt::Lower, centre, radius, lift);
        }
        BrushTool::Smooth => {
            terrain.sculpt(Sculpt::Smooth, centre, radius, strength * 0.6);
        }
        BrushTool::Flatten => {
            let level = local.transform_point3(anchor).y;
            terrain.sculpt(Sculpt::Flatten(level), centre, radius, strength * 0.5);
        }
        BrushTool::Noise => {
            terrain.sculpt(Sculpt::Noise, centre, radius, lift * 0.5);
        }
        BrushTool::Layer(i) => {
            terrain.paint(usize::from(i.min(3)), centre, radius, strength);
        }
        BrushTool::Paint | BrushTool::Erase | BrushTool::Tile(_) => {}
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
            draw_distance: 0.0,
            material: String::new(),
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
            also_selected: Vec::new(),
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            change_counter: 0,
            mid_gesture: false,
            batch: None,
            prefab_bases: HashMap::new(),
            notes: Vec::new(),
            attach_frames: HashMap::new(),
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
            draw_distance: 0.0,
            material: String::new(),
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
            draw_distance: 0.0,
            material: String::new(),
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

    /// Notes where the bones attached entities follow are (see
    /// [`Self::attach_frame`]); the viewport calls this after posing.
    pub fn set_attach_frames(&mut self, frames: HashMap<u64, glam::Mat4>) {
        self.attach_frames = frames;
    }

    /// The world frame an entity's transform is relative to: the bone it
    /// follows when attached (identity until the viewport has posed it),
    /// else the world.
    #[must_use]
    pub fn attach_frame(&self, e: &EntityEntry) -> glam::Mat4 {
        if e.data.attach.is_none() {
            return glam::Mat4::IDENTITY;
        }
        self.attach_frames
            .get(&e.id)
            .copied()
            .unwrap_or(glam::Mat4::IDENTITY)
    }

    /// An entity's model matrix in the world (its transform, through the
    /// bone it follows when attached).
    #[must_use]
    pub fn world_matrix(&self, e: &EntityEntry) -> Option<glam::Mat4> {
        let t = e.data.transform?;
        Some(self.attach_frame(e) * crate::view::model_matrix(&t))
    }

    /// An entity's transform in the world (see [`Self::world_matrix`]).
    #[must_use]
    pub fn world_transform(&self, e: &EntityEntry) -> Option<TransformData> {
        let t = e.data.transform?;
        if e.data.attach.is_none() {
            return Some(t);
        }
        let m = self.world_matrix(e)?;
        Some(TransformData::from(
            automata_scene::Transform::from_matrix_lossy(m),
        ))
    }

    /// The transform that puts entity `e` at `world` (undoes
    /// [`Self::world_transform`]).
    #[must_use]
    pub fn local_from_world(&self, e: &EntityEntry, world: TransformData) -> TransformData {
        if e.data.attach.is_none() {
            return world;
        }
        let frame = self.attach_frame(e);
        let m = crate::view::model_matrix(&world);
        TransformData::from(automata_scene::offset_for(frame, m))
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
        self.also_selected.clear();
    }

    /// Every selected entity's id, the primary last.
    #[must_use]
    pub fn selection(&self) -> Vec<u64> {
        let Some(primary) = self.selected else {
            return Vec::new();
        };
        let mut ids: Vec<u64> = self
            .also_selected
            .iter()
            .copied()
            .filter(|&id| id != primary && self.index_of(id).is_some())
            .collect();
        ids.push(primary);
        ids
    }

    /// How many entities are selected.
    #[must_use]
    pub fn selection_len(&self) -> usize {
        self.selection().len()
    }

    /// Whether `id` is selected (primary or not).
    #[must_use]
    pub fn is_selected(&self, id: u64) -> bool {
        self.selected == Some(id) || (self.selected.is_some() && self.also_selected.contains(&id))
    }

    /// Ctrl+click: adds `id` to the selection (as the primary), or takes it
    /// out if it was in.
    pub fn toggle_selected(&mut self, id: u64) {
        if self.is_selected(id) {
            if self.selected == Some(id) {
                self.selected = self.also_selected.pop();
            } else {
                self.also_selected.retain(|&x| x != id);
            }
        } else {
            if let Some(old) = self.selected {
                self.also_selected.push(old);
            }
            self.selected = Some(id);
        }
    }

    /// Shift+click: selects every entity from the primary to `id` in scene
    /// order, `id` becoming the primary.
    pub fn select_range(&mut self, id: u64) {
        let (Some(from), Some(to)) = (self.selected_index(), self.index_of(id)) else {
            self.select(Some(id));
            return;
        };
        let (lo, hi) = (from.min(to), from.max(to));
        let ids: Vec<u64> = self.entities[lo..=hi].iter().map(|e| e.id).collect();
        let mut all = self.also_selected.clone();
        for x in ids {
            if x != id && !all.contains(&x) {
                all.push(x);
            }
        }
        self.also_selected = all;
        if let Some(old) = self.selected.filter(|&old| old != id) {
            if !self.also_selected.contains(&old) {
                self.also_selected.push(old);
            }
        }
        self.selected = Some(id);
        self.also_selected.retain(|&x| x != id);
    }

    /// Selects exactly `ids`, the last one primary.
    pub fn select_many(&mut self, ids: &[u64]) {
        let mut seen = std::collections::HashSet::new();
        let mut ids: Vec<u64> = ids
            .iter()
            .copied()
            .filter(|&id| self.index_of(id).is_some() && seen.insert(id))
            .collect();
        self.selected = ids.pop();
        self.also_selected = ids;
    }

    /// Starts merging every undo snapshot into one until
    /// [`Document::end_batch`], so an edit fanned out over a selection
    /// undoes in one step.
    pub fn begin_batch(&mut self) {
        self.batch = Some(false);
    }

    /// Ends [`Document::begin_batch`].
    pub fn end_batch(&mut self) {
        self.batch = None;
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
            also_selected: self.also_selected.clone(),
            dirty: self.dirty,
            undo: Vec::new(),
            redo: Vec::new(),
            change_counter: self.change_counter,
            mid_gesture: false,
            batch: None,
            prefab_bases: self.prefab_bases.clone(),
            notes: Vec::new(),
            attach_frames: HashMap::new(),
        }
    }

    fn snapshot(&self) -> String {
        SceneFile {
            version: automata_core::SCENE_FILE_VERSION,
            entities: self.entities.iter().map(|e| e.data.clone()).collect(),
        }
        .to_ron()
        .unwrap_or_default()
    }

    fn push_undo(&mut self) {
        match self.batch {
            Some(true) => return, // the batch's one step is taken
            Some(false) => self.batch = Some(true),
            None => {}
        }
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
        self.select(Some(id));
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
        if self.is_selected(id) {
            self.toggle_selected(id);
        }
        self.touch();
        true
    }

    /// Renames the entity with `id` (records undo). Empty names and names
    /// with `:` (the separator in script keys) are refused.
    pub fn rename_entity(&mut self, id: u64, name: &str) -> bool {
        if name.is_empty() || name.contains(':') {
            return false;
        }
        let Some(idx) = self.index_of(id) else {
            return false;
        };
        self.push_undo();
        let old = self.entities[idx].data.name.replace(name.to_string());
        // Joints that named it follow the new name.
        if let Some(old) = old.filter(|o| o != name) {
            for entry in &mut self.entities {
                if let Some(j) = &mut entry.data.joint {
                    if j.connected.as_deref() == Some(old.as_str()) {
                        j.connected = Some(name.to_string());
                    }
                }
                // So do attachments.
                if let Some(a) = &mut entry.data.attach {
                    if a.to == old {
                        a.to = name.to_string();
                    }
                }
            }
        }
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
            draw_distance: 0.0,
            material: String::new(),
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

    /// Lays a brush dab (world-space `point` on a surface facing
    /// `normal`, `radius` metres, `strength` 0..1) with `tool` on entity
    /// `id`, in the entity's own space: foliage tools paint or erase its
    /// Grass, or else its Scatter; terrain tools sculpt or paint its
    /// Terrain (Flatten levels to the height of `anchor`, where the
    /// stroke began). Part of a stroke: one undo step per stroke.
    #[allow(clippy::too_many_arguments)]
    pub fn brush(
        &mut self,
        id: u64,
        tool: BrushTool,
        point: [f32; 3],
        normal: [f32; 3],
        radius: f32,
        strength: f32,
        anchor: [f32; 3],
    ) -> bool {
        self.modify(id, true, |d| {
            let model = d
                .transform
                .map_or(glam::Mat4::IDENTITY, |t| crate::view::model_matrix(&t));
            let det = model.determinant();
            if !det.is_finite() || det.abs() < 1e-9 {
                return;
            }
            let local = model.inverse();
            if let BrushTool::Tile(value) = tool {
                if let Some(map) = &mut d.tilemap {
                    // `radius` is the brush's side in cells, centred on
                    // the cell under the point.
                    let p = local.transform_point3(glam::Vec3::from(point));
                    let (cx, cy) = map.cell_at(p.x, p.y);
                    #[allow(clippy::cast_possible_truncation)]
                    let side = (radius.round() as i64).clamp(1, 16);
                    let lo = (side - 1) / 2;
                    map.fill(
                        (cx - lo, cy - lo),
                        (cx - lo + side - 1, cy - lo + side - 1),
                        value,
                    );
                }
                return;
            }
            if tool.is_terrain() {
                if let Some(terrain) = &mut d.terrain {
                    terrain_dab(
                        terrain,
                        model,
                        tool,
                        point.into(),
                        radius,
                        strength,
                        anchor.into(),
                    );
                }
                return;
            }
            let strength = if tool == BrushTool::Erase {
                -strength.abs()
            } else {
                strength.abs()
            };
            let normal = (glam::Mat3::from_mat4(model).transpose() * glam::Vec3::from(normal))
                .try_normalize()
                .unwrap_or(glam::Vec3::Y);
            // The radius in the entity's units (its ground is X and Z).
            let (scale, _, _) = model.to_scale_rotation_translation();
            let spread = f32::midpoint(scale.x.abs(), scale.z.abs()).max(1e-3);
            let dab = automata_scene::Dab {
                position: local.transform_point3(glam::Vec3::from(point)).to_array(),
                normal: normal.to_array(),
                radius: radius / spread,
                strength: strength.clamp(-1.0, 1.0),
            };
            if let Some(grass) = &mut d.grass {
                grass.strokes.push(dab);
            } else if let Some(scatter) = &mut d.scatter {
                scatter.strokes.push(dab);
            }
        })
    }

    /// Paints entity `id`'s Terrain by its shape: sand along the scene's
    /// water (its first Water's height, else y = 0) up to `beach` metres
    /// above it, rock on slopes over `cliff` degrees, dirt below those,
    /// grass elsewhere. One undo step.
    pub fn auto_paint_terrain(&mut self, id: u64, beach: f32, cliff: f32) -> bool {
        let water = self
            .entities
            .iter()
            .find(|e| e.data.water.is_some())
            .and_then(|e| self.world_matrix(e))
            .map_or(0.0, |m| m.w_axis.y);
        let Some(model) = self.index_of(id).map(|i| {
            self.world_matrix(&self.entities[i])
                .unwrap_or(glam::Mat4::IDENTITY)
        }) else {
            return false;
        };
        // The water's height in the terrain's space (straight down).
        let level = model
            .inverse()
            .transform_point3(glam::Vec3::new(model.w_axis.x, water, model.w_axis.z))
            .y;
        self.modify(id, false, |d| {
            if let Some(t) = &mut d.terrain {
                t.auto_paint(level, beach, cliff);
            }
        })
    }

    /// Replaces entity `id`'s Terrain heights with a greyscale image
    /// (black at 0, white at a quarter of the terrain's size, in its
    /// space). One undo step.
    ///
    /// # Errors
    /// When the image can't be read.
    pub fn import_heightmap(&mut self, id: u64, image: &[u8]) -> Result<(), String> {
        let Some(terrain) = self
            .index_of(id)
            .and_then(|i| self.entities[i].data.terrain.clone())
        else {
            return Err(String::from("the entity has no Terrain"));
        };
        let mut loaded = terrain;
        let high = loaded.size * 0.25;
        loaded.load_heightmap(image, 0.0, high)?;
        self.modify(id, false, |d| {
            if let Some(t) = &mut d.terrain {
                *t = loaded;
            }
        });
        Ok(())
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
                draw_distance: 0.0,
                material: String::new(),
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
        self.select(Some(id));
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
                draw_distance: 0.0,
                material: String::new(),
            });
            fit_animation(d);
        })
    }

    /// Puts the material file `rel` (relative to the scene folder; empty
    /// clears it) on the entity's Mesh Renderer. False when the entity has
    /// no Mesh Renderer or already uses it.
    pub fn set_material(&mut self, id: u64, rel: &str) -> bool {
        let rel = rel.replace('\\', "/");
        let has_mesh = self
            .index_of(id)
            .is_some_and(|i| self.entities[i].data.mesh_visual.is_some());
        has_mesh
            && self.modify(id, false, |d| {
                if let Some(m) = &mut d.mesh_visual {
                    m.material.clone_from(&rel);
                }
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
        self.select(Some(id));
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
                    draw_distance: 0.0,
                    material: String::new(),
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
                    draw_distance: 0.0,
                    material: String::new(),
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
            CreateKind::Grass => {
                ComponentKind::Grass.add_to(&mut data);
            }
            CreateKind::Terrain => {
                ComponentKind::Terrain.add_to(&mut data);
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
                    draw_distance: 0.0,
                    material: String::new(),
                });
                ComponentKind::Character.add_to(&mut data);
            }
            CreateKind::Sprite => {
                ComponentKind::Sprite.add_to(&mut data);
            }
            CreateKind::Tilemap => {
                if let Some(t) = &mut data.transform {
                    // The grid grows right and up from its corner: centre
                    // it on the origin, its floor just below.
                    t.position = [-8.0, -2.0, 0.0];
                }
                let mut map = automata_scene::Tilemap::default();
                map.fill((0, 0), (15, 0), 1);
                data.tilemap = Some(map);
            }
            CreateKind::Camera2D => {
                ComponentKind::CameraFollow2D.add_to(&mut data);
            }
            CreateKind::Player2D => {
                if let Some(t) = &mut data.transform {
                    t.position = [0.0, 0.0, 0.0];
                }
                data.name = Some(self.unique_name("Player"));
                data.sprite = Some(automata_scene::Sprite2D {
                    color: [1.0, 0.56, 0.24, 1.0],
                    ..automata_scene::Sprite2D::default()
                });
                ComponentKind::Character.add_to(&mut data);
            }
        }
        self.push_undo();
        let id = self.alloc_id();
        self.entities.push(EntityEntry { id, data });
        self.select(Some(id));
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
        self.select(None);
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
            self.select(None);
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
        self.select(Some(new_id));
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
            self.select(self.entities.first().map(|e| e.id));
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
    /// every script, model, material and sound reference at the new place (files
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
            if let Some(mesh) = data.mesh_visual.as_mut().filter(|m| !m.material.is_empty()) {
                if let Some(p) = moved(&mesh.material) {
                    mesh.material = p;
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
        self.select(ids.last().copied());
        self.touch();
        ids
    }

    /// The prefab file for entity `id`: a one-entity scene file.
    #[must_use]
    pub fn prefab_of(&self, id: u64) -> Option<SceneFile> {
        let idx = self.index_of(id)?;
        let mut data = self.entities[idx].data.clone();
        data.prefab = None; // a new prefab of its own
        Some(SceneFile {
            version: automata_core::SCENE_FILE_VERSION,
            entities: vec![data],
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

    /// Duplicates every entity in `ids` (each placed after the last one,
    /// in scene order) as one undo step, selecting the copies. Returns the
    /// new ids.
    pub fn duplicate_many(&mut self, ids: &[u64]) -> Vec<u64> {
        let mut order: Vec<usize> = ids.iter().filter_map(|&id| self.index_of(id)).collect();
        order.sort_unstable();
        order.dedup();
        if order.is_empty() {
            return Vec::new();
        }
        self.push_undo();
        let mut new_ids = Vec::with_capacity(order.len());
        let first_free = order[order.len() - 1] + 1;
        for (insert_at, idx) in (first_free..).zip(order) {
            let mut data = self.entities[idx].data.clone();
            let base = self.entities[idx].name().to_string();
            data.name = Some(self.unique_name(&base));
            let new_id = self.alloc_id();
            self.entities
                .insert(insert_at, EntityEntry { id: new_id, data });
            new_ids.push(new_id);
        }
        self.select_many(&new_ids);
        self.touch();
        new_ids
    }

    /// Copies entity `id` `count` times along a line: copy *k* sits at
    /// `k × step` from the original, turned `k × turn_y` radians about Y.
    /// One undo step; the whole row (original and copies) is selected, so
    /// it moves as one. Returns the copies' ids.
    pub fn duplicate_along(
        &mut self,
        id: u64,
        count: usize,
        step: [f32; 3],
        turn_y: f32,
    ) -> Vec<u64> {
        let Some(idx) = self.index_of(id) else {
            return Vec::new();
        };
        if count == 0 {
            return Vec::new();
        }
        self.push_undo();
        let source = self.entities[idx].data.clone();
        let base = source.name.clone().unwrap_or_else(|| "Entity".to_string());
        let mut ids = Vec::with_capacity(count);
        for k in 1..=count {
            let mut data = source.clone();
            data.name = Some(self.unique_name(&base));
            if let Some(t) = &mut data.transform {
                #[allow(clippy::cast_precision_loss)] // copy counts are small
                let kf = k as f32;
                for (p, d) in t.position.iter_mut().zip(step) {
                    *p += d * kf;
                }
                t.rotation_euler = turn_about_y(t, turn_y * kf);
            }
            let new_id = self.alloc_id();
            self.entities
                .insert(idx + k, EntityEntry { id: new_id, data });
            ids.push(new_id);
        }
        let mut all = vec![id];
        all.extend(&ids);
        self.select_many(&all);
        self.touch();
        ids
    }

    // ----- prefab links ----------------------------------------------------

    /// The prefab entity `link` follows, as last synced (resolved through
    /// variants).
    #[must_use]
    pub fn prefab_base(&self, link: &PrefabLink) -> Option<&EntityData> {
        self.prefab_bases.get(&(link.path.clone(), link.index))
    }

    /// The fields entity `id` overrides on its prefab (`None` when it isn't
    /// linked, or its prefab couldn't be read).
    #[must_use]
    pub fn prefab_overrides(&self, id: u64) -> Option<Vec<String>> {
        let data = &self.entities[self.index_of(id)?].data;
        let base = self.prefab_base(data.prefab.as_ref()?)?;
        Some(prefab::overrides(base, data))
    }

    /// Every entity linked to the prefab file `path` (as the scene names it).
    #[must_use]
    pub fn instances_of(&self, path: &str) -> Vec<u64> {
        self.entities
            .iter()
            .filter(|e| e.data.prefab.as_ref().is_some_and(|l| l.path == path))
            .map(|e| e.id)
            .collect()
    }

    /// Reads (or re-reads) the prefab a link follows, keeping the result
    /// for [`Document::prefab_base`]. A failure is noted once.
    fn load_prefab_base(&mut self, link: &PrefabLink) -> Option<EntityData> {
        match prefab::load_base(link, &self.base_dir()) {
            Ok(base) => {
                self.prefab_bases
                    .insert((link.path.clone(), link.index), base.clone());
                Some(base)
            }
            Err(e) => {
                let note = format!("Prefab {e}");
                if !self.notes.contains(&note) {
                    self.notes.push(note);
                }
                None
            }
        }
    }

    /// Re-syncs every prefab instance with its prefab file as saved in the
    /// scene (each keeps its stored overrides). Used after loading.
    fn sync_prefabs_from_files(&mut self) {
        let links: Vec<PrefabLink> = self.distinct_links();
        for link in links {
            let Some(base) = self.load_prefab_base(&link) else {
                continue;
            };
            for e in &mut self.entities {
                let Some(own) = e.data.prefab.clone() else {
                    continue;
                };
                if own.path != link.path || own.index != link.index {
                    continue;
                }
                match prefab::apply(&base, &e.data, &own.overrides) {
                    Ok(synced) => e.data = synced,
                    Err(err) => self.notes.push(format!("{}: {err}", e.name())),
                }
            }
        }
    }

    fn distinct_links(&self) -> Vec<PrefabLink> {
        let mut links: Vec<PrefabLink> = Vec::new();
        for e in &self.entities {
            if let Some(l) = &e.data.prefab {
                if !links.iter().any(|x| x.path == l.path && x.index == l.index) {
                    links.push(PrefabLink {
                        overrides: Vec::new(),
                        ..l.clone()
                    });
                }
            }
        }
        links
    }

    /// Re-reads every linked prefab file and re-syncs the instances of any
    /// that changed, keeping each instance's overrides. Not an undo step
    /// (the change came from the file). Returns how many entities changed.
    pub fn poll_prefabs(&mut self) -> usize {
        let mut changed = 0;
        for link in self.distinct_links() {
            let old = self.prefab_base(&link).cloned();
            let Some(new) = self.load_prefab_base(&link) else {
                continue;
            };
            let Some(old) = old else {
                continue; // first read: nothing to compare with
            };
            if old == new {
                continue;
            }
            for e in &mut self.entities {
                let Some(own) = &e.data.prefab else {
                    continue;
                };
                if own.path != link.path || own.index != link.index {
                    continue;
                }
                let ov = prefab::overrides(&old, &e.data);
                if let Ok(synced) = prefab::apply(&new, &e.data, &ov) {
                    if synced != e.data {
                        e.data = synced;
                        changed += 1;
                    }
                }
            }
        }
        if changed > 0 {
            self.change_counter += 1;
        }
        changed
    }

    /// Links entity `id` to the prefab file `rel` (scene-relative), which
    /// it matches now (after Save as Prefab).
    pub fn link_prefab(&mut self, id: u64, rel: &str) -> bool {
        let link = PrefabLink::new(rel);
        let changed = self.modify(id, false, |d| d.prefab = Some(link.clone()));
        if changed {
            self.load_prefab_base(&link);
        }
        changed
    }

    /// Takes entity `id` off its prefab: it keeps its current values.
    pub fn unlink_prefab(&mut self, id: u64) -> bool {
        self.modify(id, false, |d| d.prefab = None)
    }

    /// Puts the prefab's values back on entity `id` for `paths` (every
    /// override when `None`). One undo step.
    pub fn revert_overrides(&mut self, id: u64, paths: Option<&[String]>) -> bool {
        let Some(idx) = self.index_of(id) else {
            return false;
        };
        let Some(link) = self.entities[idx].data.prefab.clone() else {
            return false;
        };
        let Some(base) = self
            .prefab_base(&link)
            .cloned()
            .or_else(|| self.load_prefab_base(&link))
        else {
            return false;
        };
        let all = prefab::overrides(&base, &self.entities[idx].data);
        let paths = paths.map_or(all, <[String]>::to_vec);
        match prefab::copy_paths(&base, &self.entities[idx].data, &paths) {
            Ok(reverted) => self.modify(id, false, |d| *d = reverted),
            Err(e) => {
                self.notes.push(format!("Revert failed: {e}"));
                false
            }
        }
    }

    /// Writes entity `id`'s overrides into its prefab file (into the
    /// variant's own overrides, for a variant), then re-syncs every
    /// instance. Returns the prefab file and how many fields were applied.
    ///
    /// # Errors
    ///
    /// When the entity isn't linked, or the prefab file can't be read or
    /// written.
    pub fn apply_to_prefab(&mut self, id: u64) -> Result<(PathBuf, usize), String> {
        let idx = self.index_of(id).ok_or("no such entity")?;
        let instance = self.entities[idx].data.clone();
        let link = instance
            .prefab
            .clone()
            .ok_or_else(|| format!("{} isn't a prefab instance", self.entities[idx].name()))?;
        let root = self.base_dir();
        let base = prefab::load_base(&link, &root)?;
        let ov = prefab::overrides(&base, &instance);
        let file_path = prefab::resolve_path(&root, &link.path);
        if ov.is_empty() {
            return Ok((file_path, 0));
        }
        let text =
            std::fs::read_to_string(&file_path).map_err(|e| format!("{}: {e}", link.path))?;
        let mut file = SceneFile::from_ron(&text).map_err(|e| format!("{}: {e}", link.path))?;
        let slot = file
            .entities
            .get_mut(link.index as usize)
            .ok_or_else(|| format!("{} has no entity {}", link.path, link.index))?;
        // The prefab's entity with the instance's values where it differs.
        let mut updated = prefab::copy_paths(&instance, &base, &ov)?;
        updated.prefab = match slot.prefab.clone() {
            // A variant keeps following its base: store what differs.
            Some(base_link) => {
                let grand = prefab::load_base(&base_link, &root)?;
                Some(PrefabLink {
                    overrides: prefab::overrides(&grand, &updated),
                    ..base_link
                })
            }
            None => None,
        };
        *slot = updated;
        let text = file.to_ron().map_err(|e| e.to_string())?;
        std::fs::write(&file_path, text).map_err(|e| format!("{}: {e}", file_path.display()))?;
        self.poll_prefabs();
        Ok((file_path, ov.len()))
    }

    /// Writes a variant of the prefab at `prefab_path`: a new prefab that
    /// follows it and can override its fields. Returns the new file.
    ///
    /// # Errors
    ///
    /// When the prefab can't be read or the variant can't be written.
    pub fn create_variant(&self, prefab_path: &Path) -> Result<PathBuf, String> {
        let rel = self.relative(prefab_path);
        let link = PrefabLink::new(rel.clone());
        let base = prefab::load_base(&link, &self.base_dir())?;
        let stem = prefab_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("prefab");
        let dir = prefab_path.parent().unwrap_or_else(|| Path::new("."));
        let mut target = dir.join(format!("{stem}_variant.prefab"));
        let mut n = 2;
        while target.exists() {
            target = dir.join(format!("{stem}_variant_{n}.prefab"));
            n += 1;
        }
        let mut entity = base;
        let name = entity.name.clone().unwrap_or_else(|| stem.to_string());
        entity.name = Some(format!("{name} Variant"));
        entity.prefab = Some(link);
        let file = SceneFile {
            version: automata_core::SCENE_FILE_VERSION,
            entities: vec![entity],
        };
        let text = file.to_ron().map_err(|e| e.to_string())?;
        std::fs::write(&target, text).map_err(|e| format!("{}: {e}", target.display()))?;
        Ok(target)
    }

    /// Adds linked instances of the prefab file `rel` (already read as
    /// `prefab`), its first entity placed at `at`. Returns the new ids.
    pub fn instantiate_linked(&mut self, rel: &str, prefab: &SceneFile, at: [f32; 3]) -> Vec<u64> {
        let mut file = prefab.clone();
        // Variants arrive resolved; each copy links to its entity.
        let _ = file.resolve_prefabs(&self.base_dir());
        for (i, e) in file.entities.iter_mut().enumerate() {
            e.prefab = Some(PrefabLink {
                path: rel.to_string(),
                index: u32::try_from(i).unwrap_or(0),
                overrides: Vec::new(),
            });
        }
        let ids = self.instantiate(&file, at);
        for link in self.distinct_links() {
            if link.path == rel {
                self.load_prefab_base(&link);
            }
        }
        ids
    }

    /// Builds the engine [`SceneFile`] from the document, each prefab
    /// instance's overrides listed as they are now.
    #[must_use]
    pub fn to_scene_file(&self) -> SceneFile {
        SceneFile {
            version: automata_core::SCENE_FILE_VERSION,
            entities: self
                .entities
                .iter()
                .map(|e| {
                    let mut data = e.data.clone();
                    let base = data.prefab.as_ref().and_then(|l| self.prefab_base(l));
                    if let Some(base) = base {
                        let ov = prefab::overrides(base, &data);
                        if let Some(link) = &mut data.prefab {
                            link.overrides = ov;
                        }
                    }
                    data
                })
                .collect(),
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
                // Instances follow their prefab files, which may have
                // changed since the scene was saved.
                doc.sync_prefabs_from_files();
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
    fn names_with_colons_are_refused() {
        let mut doc = Document::new();
        doc.add_entity_named("Player");
        let id = doc.entities[0].id;
        assert!(
            !doc.rename_entity(id, "Player:1"),
            "':' separates key parts"
        );
        assert!(!doc.rename_entity(id, ""));
        assert!(doc.rename_entity(id, "Player 1"));
        assert_eq!(doc.entities[0].name(), "Player 1");
    }

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

    #[test]
    fn materials_go_on_mesh_renderers_and_follow_moves() {
        let root = std::env::temp_dir().join(format!("oberon-matdoc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("materials")).unwrap();
        std::fs::write(root.join("materials/brick.mat.ron"), "()").unwrap();
        let mut doc = Document::new();
        doc.path = Some(root.join("level.ron"));
        let cube = doc.create(CreateKind::Cube);
        let empty = doc.create(CreateKind::Empty);
        assert!(doc.set_material(cube, "materials/brick.mat.ron"));
        assert!(
            !doc.set_material(cube, "materials/brick.mat.ron"),
            "no change"
        );
        assert!(
            !doc.set_material(empty, "materials/brick.mat.ron"),
            "no mesh"
        );
        let mesh = |doc: &Document| {
            let i = doc.index_of(cube).unwrap();
            doc.entities[i].data.mesh_visual.clone().unwrap()
        };
        assert_eq!(mesh(&doc).material, "materials/brick.mat.ron");
        // Changing the shape keeps the material.
        let i = doc.index_of(cube).unwrap();
        assert!(crate::fields::set_choice(
            &mut doc.entities[i].data,
            crate::fields::Choice::MeshPrimitive,
            "Sphere"
        ));
        assert_eq!(mesh(&doc).material, "materials/brick.mat.ron");
        // Renaming the folder retargets the scene.
        std::fs::rename(root.join("materials"), root.join("mats")).unwrap();
        assert_eq!(
            doc.retarget_paths(&root.join("materials"), &root.join("mats")),
            1
        );
        assert_eq!(mesh(&doc).material, "mats/brick.mat.ron");
        assert!(doc.set_material(cube, ""));
        assert_eq!(mesh(&doc).material, "");
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
    fn terrains_sculpt_paint_and_import_in_the_entitys_space() {
        let mut doc = Document::new();
        let id = doc.create(CreateKind::Terrain);
        let data = |doc: &Document| doc.entities[doc.index_of(id).unwrap()].data.clone();
        assert!(ComponentKind::Terrain.present_in(&data(&doc)));
        // Stretched ×2 across and lifted 3 m: brushes work in its space.
        doc.modify(id, false, |d| {
            let t = d.transform.as_mut().unwrap();
            t.position = [10.0, 3.0, 0.0];
            t.scale = [2.0, 1.0, 2.0];
        });
        let up = [0.0, 1.0, 0.0];
        let at = [14.0, 3.0, 0.0]; // local (2, 0, 0)
        for _ in 0..20 {
            assert!(doc.brush(id, BrushTool::Raise, at, up, 10.0, 1.0, at));
        }
        doc.end_gesture();
        let t = data(&doc).terrain.unwrap();
        let peak = t.height_at(2.0, 0.0);
        assert!(peak > 1.0, "raised: {peak}");
        assert!(t.height_at(-20.0, 0.0).abs() < 1e-4, "out of reach");
        // One stroke, one undo step.
        assert!(doc.undo());
        assert!(data(&doc).terrain.unwrap().height_at(2.0, 0.0).abs() < 1e-4);
        assert!(doc.redo());
        // Flatten levels to where the stroke began (world y 3 = local 0).
        let start = [14.0, 3.0 + peak, 0.0];
        for _ in 0..12 {
            doc.brush(
                id,
                BrushTool::Flatten,
                start,
                up,
                10.0,
                1.0,
                [30.0, 3.0, 0.0],
            );
        }
        doc.end_gesture();
        assert!(data(&doc).terrain.unwrap().height_at(2.0, 0.0) < peak * 0.2);
        // Painting puts a layer down; Paint (foliage) leaves it alone.
        doc.brush(id, BrushTool::Layer(1), at, up, 6.0, 1.0, at);
        doc.brush(id, BrushTool::Paint, at, up, 6.0, 1.0, at);
        doc.end_gesture();
        assert!(data(&doc).terrain.unwrap().weights_at(2.0, 0.0)[1] > 0.9);

        // Auto Paint reads the Water's height: a beach along it.
        let water = doc.create(CreateKind::Water);
        doc.modify(water, false, |d| {
            d.transform.as_mut().unwrap().position[1] = 4.0;
        });
        doc.modify(id, false, |d| {
            d.terrain.as_mut().unwrap().set_heights_with(|x, _| x * 0.1);
        });
        assert!(doc.auto_paint_terrain(id, 1.0, 40.0));
        let t = data(&doc).terrain.unwrap();
        // Local x = 10 is 1 m up: world 4 m, the waterline.
        assert!(t.weights_at(10.0, 0.0)[2] > 0.5, "sand at the waterline");
        assert!(t.weights_at(-25.0, 0.0)[2] > 0.5, "a sandy seabed under it");
        assert!(t.weights_at(28.0, 0.0)[0] > 0.5, "grass above the beach");

        // A heightmap: a greyscale image, black left, white right.
        let mut png = Vec::new();
        let pixels = [[0u8; 4], [255; 4]].concat().repeat(2);
        image::GrayImage::from_raw(8, 2, pixels)
            .unwrap()
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        assert!(doc.import_heightmap(id, &png).is_ok());
        let t = data(&doc).terrain.unwrap();
        assert!(t.height_at(-30.0, 0.0).abs() < 0.1);
        assert!(
            (t.height_at(30.0, 0.0) - 16.0).abs() < 0.1,
            "a quarter of 64 m"
        );
        assert!(doc.import_heightmap(id, b"not an image").is_err());
        assert!(doc.import_heightmap(water, &png).is_err());
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
    fn brush_strokes_paint_grass_in_the_entitys_space_as_one_undo_step() {
        let mut doc = Document::new();
        let id = doc.create(CreateKind::Grass);
        let data = doc.entities[doc.index_of(id).unwrap()].data.clone();
        assert!(ComponentKind::Grass.present_in(&data));
        assert_eq!(data.grass.as_ref().unwrap().area, [10.0, 10.0]);
        doc.modify(id, false, |d| {
            if let Some(t) = &mut d.transform {
                t.position = [10.0, 1.0, 0.0];
                t.scale = [2.0, 1.0, 2.0];
            }
        });
        assert!(doc.brush(
            id,
            BrushTool::Paint,
            [14.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            3.0,
            1.0,
            [0.0; 3]
        ));
        assert!(doc.brush(
            id,
            BrushTool::Erase,
            [16.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            3.0,
            0.5,
            [0.0; 3]
        ));
        doc.end_gesture();
        let grass = doc.entities[doc.index_of(id).unwrap()]
            .data
            .grass
            .clone()
            .unwrap();
        assert_eq!(grass.strokes.len(), 2);
        let dab = grass.strokes[0];
        assert_eq!(dab.position, [2.0, 0.0, 0.0]);
        assert!((dab.radius - 1.5).abs() < 1e-5);
        assert!((grass.strokes[1].strength + 0.5).abs() < 1e-5);
        // It grows where it was painted.
        let mut world = automata_scene::SceneWorld::new();
        doc.to_scene_file().apply_to_world(&mut world);
        let mut draw = automata_renderer::DrawList::new();
        automata_scene::record_draw_list(&world, &mut draw);
        assert_eq!(draw.grass_fields().len(), 1);
        // The whole stroke undoes at once.
        assert!(doc.undo());
        let after = doc.entities[doc.index_of(id).unwrap()].data.clone();
        assert_eq!(after.grass.unwrap().strokes.len(), 0);
        // Without Grass, a Scatter takes the dab; with neither, nothing.
        let cube = doc.create(CreateKind::Cube);
        assert!(!doc.brush(
            cube,
            BrushTool::Paint,
            [0.0; 3],
            [0.0, 1.0, 0.0],
            1.0,
            1.0,
            [0.0; 3]
        ));
        let at = doc.index_of(cube).unwrap();
        ComponentKind::Scatter.add_to(&mut doc.entities[at].data);
        assert!(doc.brush(
            cube,
            BrushTool::Paint,
            [0.0; 3],
            [0.0, 1.0, 0.0],
            1.0,
            1.0,
            [0.0; 3]
        ));
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

    #[test]
    fn multi_select_by_ctrl_shift_and_lists() {
        let mut doc = Document::new();
        let ids: Vec<u64> = (0..5).map(|i| cube(&mut doc, &format!("C{i}"))).collect();
        doc.select(Some(ids[1]));
        doc.toggle_selected(ids[3]);
        assert_eq!(doc.selection(), vec![ids[1], ids[3]], "primary last");
        assert!(doc.is_selected(ids[1]) && doc.is_selected(ids[3]));
        assert!(!doc.is_selected(ids[2]));
        // Ctrl+click again takes it out; the previous primary leads.
        doc.toggle_selected(ids[3]);
        assert_eq!(doc.selection(), vec![ids[1]]);
        assert_eq!(doc.selected, Some(ids[1]));
        // Shift+click: the run from the primary, the clicked one leading.
        doc.select_range(ids[4]);
        assert_eq!(doc.selection_len(), 4);
        assert_eq!(doc.selected, Some(ids[4]));
        assert!(!doc.is_selected(ids[0]));
        // A plain select drops the rest.
        doc.select(Some(ids[0]));
        assert_eq!(doc.selection(), vec![ids[0]]);
        // Lists skip repeats and missing ids.
        doc.select_many(&[ids[2], ids[0], ids[2], 99_999]);
        assert_eq!(doc.selection(), vec![ids[2], ids[0]]);
        // Deleting a selected entity leaves the rest selected.
        assert!(doc.delete_entity(ids[0]));
        assert_eq!(doc.selection(), vec![ids[2]]);
        assert!(doc.delete_entity(ids[2]));
        assert_eq!(doc.selection(), Vec::<u64>::new());
    }

    #[test]
    fn batched_edits_and_group_duplicates_undo_in_one_step() {
        let mut doc = Document::new();
        let a = cube(&mut doc, "A");
        let b = cube(&mut doc, "B");
        doc.begin_batch();
        doc.modify(a, false, |d| d.name = Some("A2".into()));
        doc.modify(b, false, |d| d.name = Some("B2".into()));
        doc.end_batch();
        assert!(doc.undo());
        let names: Vec<&str> = doc.entities.iter().map(EntityEntry::name).collect();
        assert_eq!(names, ["A", "B"], "one undo reverts the whole batch");

        let copies = doc.duplicate_many(&[a, b]);
        assert_eq!(copies.len(), 2);
        assert_eq!(doc.entity_count(), 4);
        assert_eq!(doc.selection(), copies, "the copies are selected");
        assert!(doc.undo());
        assert_eq!(doc.entity_count(), 2);
    }

    #[test]
    fn duplicate_along_spaces_and_turns_the_copies() {
        let mut doc = Document::new();
        let a = cube(&mut doc, "Post");
        let i = doc.index_of(a).unwrap();
        doc.entities[i].data.transform.as_mut().unwrap().position = [1.0, 0.0, 0.0];
        let line = doc.duplicate_along(a, 3, [2.0, 0.0, 0.5], std::f32::consts::FRAC_PI_2);
        assert_eq!(line.len(), 3);
        for (k, id) in line.iter().enumerate() {
            let t = doc.entities[doc.index_of(*id).unwrap()]
                .data
                .transform
                .unwrap();
            #[allow(clippy::cast_precision_loss)]
            let k = (k + 1) as f32;
            assert!((t.position[0] - (1.0 + 2.0 * k)).abs() < 1e-5);
            assert!((t.position[2] - 0.5 * k).abs() < 1e-5);
            let turned = crate::view::rotation_of(&t) * glam::Vec3::X;
            let want = glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2 * k) * glam::Vec3::X;
            assert!(
                turned.abs_diff_eq(want, 1e-4),
                "copy {k}: {turned} vs {want}"
            );
        }
        let mut row = vec![a];
        row.extend(&line);
        assert_eq!(doc.selection(), row, "the whole row is selected");
        assert_eq!(
            doc.duplicate_along(a, 0, [1.0, 0.0, 0.0], 0.0),
            Vec::<u64>::new()
        );
        assert!(doc.undo());
        assert_eq!(doc.entity_count(), 1, "one undo removes the whole line");
    }

    #[test]
    fn prefab_instances_follow_the_file_and_keep_overrides() {
        let dir = std::env::temp_dir().join(format!("oberon-prefab-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut doc = Document::new();
        doc.path = Some(dir.join("level.ron"));
        let a = cube(&mut doc, "Crate");
        let file = doc.prefab_of(a).unwrap();
        let prefab_path = dir.join("crate.prefab");
        std::fs::write(&prefab_path, file.to_ron().unwrap()).unwrap();
        assert!(doc.link_prefab(a, "crate.prefab"));
        assert_eq!(doc.prefab_overrides(a), Some(Vec::new()));

        // A second instance elsewhere: where it stands isn't an override.
        let b = doc.instantiate_linked("crate.prefab", &file, [4.0, 0.0, 0.0])[0];
        assert_eq!(doc.prefab_overrides(b), Some(Vec::new()));
        assert_eq!(doc.instances_of("crate.prefab"), vec![a, b]);
        let red = [1.0, 0.0, 0.0, 1.0];
        doc.modify(b, false, |d| d.mesh_visual.as_mut().unwrap().color = red);
        assert_eq!(doc.prefab_overrides(b).unwrap(), ["mesh_visual.color"]);
        let mesh = |doc: &Document, id: u64| {
            doc.entities[doc.index_of(id).unwrap()]
                .data
                .mesh_visual
                .clone()
                .unwrap()
        };

        // The file changes: both instances follow, B keeps its colour.
        let mut edited = file.clone();
        edited.entities[0]
            .mesh_visual
            .as_mut()
            .unwrap()
            .draw_distance = 77.0;
        std::fs::write(&prefab_path, edited.to_ron().unwrap()).unwrap();
        assert_eq!(doc.poll_prefabs(), 2);
        assert_eq!(doc.poll_prefabs(), 0, "nothing new the second time");
        assert!((mesh(&doc, a).draw_distance - 77.0).abs() < 1e-6);
        assert!((mesh(&doc, b).draw_distance - 77.0).abs() < 1e-6);
        assert_eq!(mesh(&doc, b).color, red);
        assert_eq!(
            doc.entities[doc.index_of(b).unwrap()]
                .data
                .transform
                .unwrap()
                .position[0],
            4.0
        );

        // The scene file lists B's override; saving and loading keeps it.
        let scene = doc.to_scene_file();
        let link = scene.entities[1].prefab.clone().unwrap();
        assert_eq!(link.overrides, ["mesh_visual.color"]);
        let saved = doc.save(None).unwrap();
        let loaded = Document::load(&saved).unwrap();
        assert_eq!(
            loaded.entities[1].data.prefab.as_ref().unwrap().path,
            "crate.prefab"
        );
        assert_eq!(mesh(&loaded, loaded.entities[1].id).color, red);

        // Revert one field.
        assert!(doc.revert_overrides(b, Some(&["mesh_visual.color".to_string()])));
        assert_eq!(doc.prefab_overrides(b), Some(Vec::new()));
        assert_ne!(mesh(&doc, b).color, red);

        // Apply A's change to the file: B (no override) picks it up.
        let green = [0.0, 1.0, 0.0, 1.0];
        doc.modify(a, false, |d| d.mesh_visual.as_mut().unwrap().color = green);
        let (written, n) = doc.apply_to_prefab(a).unwrap();
        assert_eq!((written, n), (prefab_path.clone(), 1));
        assert_eq!(mesh(&doc, b).color, green);
        assert_eq!(doc.prefab_overrides(a), Some(Vec::new()));

        // A variant follows its parent and adds its own overrides.
        let variant = doc.create_variant(&prefab_path).unwrap();
        assert_eq!(variant, dir.join("crate_variant.prefab"));
        assert_eq!(
            doc.create_variant(&prefab_path).unwrap(),
            dir.join("crate_variant_2.prefab")
        );
        let v = doc.instantiate_linked(
            "crate_variant.prefab",
            &SceneFile::from_ron(&std::fs::read_to_string(&variant).unwrap()).unwrap(),
            [8.0, 0.0, 0.0],
        )[0];
        assert_eq!(mesh(&doc, v).color, green);
        let blue = [0.0, 0.0, 1.0, 1.0];
        doc.modify(v, false, |d| d.mesh_visual.as_mut().unwrap().color = blue);
        doc.apply_to_prefab(v).unwrap();
        // The variant now overrides the colour; the parent is untouched.
        assert_eq!(mesh(&doc, a).color, green);
        let mut parent =
            SceneFile::from_ron(&std::fs::read_to_string(&prefab_path).unwrap()).unwrap();
        let _ = parent.resolve_prefabs(&dir);
        assert_eq!(
            parent.entities[0].mesh_visual.as_ref().unwrap().color,
            green
        );
        let mut child = SceneFile::from_ron(&std::fs::read_to_string(&variant).unwrap()).unwrap();
        assert_eq!(child.resolve_prefabs(&dir), Vec::<String>::new());
        assert_eq!(child.entities[0].mesh_visual.as_ref().unwrap().color, blue);
        assert!(
            (child.entities[0]
                .mesh_visual
                .as_ref()
                .unwrap()
                .draw_distance
                - 77.0)
                .abs()
                < 1e-6
        );

        // Unlink: values stay, the link goes.
        assert!(doc.unlink_prefab(b));
        assert!(doc.entities[doc.index_of(b).unwrap()].data.prefab.is_none());
        assert_eq!(mesh(&doc, b).color, green);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
