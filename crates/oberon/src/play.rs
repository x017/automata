//! Play mode: simulate the scene without touching the edited document.
//!
//! Pressing Play snapshots the document into a [`PlaySession`]: a live ECS
//! world (the same one the game runtime builds from a scene file) plus a 3D
//! physics world. Each frame the session steps physics at a fixed rate and
//! copies the simulated transforms into its own display copy of the document,
//! so the hierarchy, inspector and overlays show the running state. Stop drops
//! the session; the edited document was never modified, so nothing needs to be
//! restored.

#![forbid(unsafe_code)]

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use automata_audio::AudioEngine;
use automata_core::audio_source::{AudioCommand, AudioSource, AudioSourcePlayer};
use automata_core::{Attributes, TransformData};
use automata_physics::{
    BodyType, CharacterController, CharacterVelocity, PhysicsWorld3D, RigidBody, WorldShape,
};
use automata_renderer::Camera3D;
use automata_scene::{Entity, Name, Scene, Transform};
use glam::Vec3;

use crate::bridge::{
    AttachWrite, AudioWrite, ColliderShape, EntityState, EntityWrite, FrameInput, FrameReply,
    HudItem, MeshTris,
};
use crate::doc::Document;

/// The fixed physics timestep (seconds).
pub const TIMESTEP: f32 = 1.0 / 60.0;

/// The most physics steps run in one frame (avoids a spiral of death after a
/// long stall, e.g. while the window is being dragged).
const MAX_STEPS: u32 = 5;

/// Player walking speed (m/s); sprinting doubles it.
const WALK_SPEED: f32 = 5.0;
/// Upward speed of a jump (m/s).
const JUMP_SPEED: f32 = 7.0;
/// Gravity applied to characters (m/s²). Kinematic characters are not
/// affected by the physics world's gravity, so the session integrates it.
const CHARACTER_GRAVITY: f32 = 20.0;

/// The player's controls for this frame (set by the app from the keyboard).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PlayInput {
    /// Forward (+1) / back (-1), relative to the camera.
    pub forward: f32,
    /// Right (+1) / left (-1), relative to the camera.
    pub right: f32,
    /// Jump held.
    pub jump: bool,
    /// Sprint held.
    pub sprint: bool,
    /// The view camera's yaw (radians): movement is camera-relative.
    pub camera_yaw: f32,
}

/// A character-controlled entity and its vertical speed.
#[derive(Debug, Clone, Copy)]
struct Character {
    entity: Entity,
    vertical: f32,
    /// A 2D (side-view) character: walks along X, jumps along Y in the
    /// 2D physics world.
    two_d: bool,
}

/// A running simulation of a document.
pub struct PlaySession {
    /// The document as simulated (shown by the UI while playing).
    pub doc: Document,
    scene: Scene,
    physics: PhysicsWorld3D,
    /// Sprites' and tilemaps' world (entities with a 2D collider).
    physics2d: automata_physics::PhysicsWorld2D,
    /// `(entity index in doc, ECS entity)` for every entity with a transform.
    map: Vec<(usize, Entity)>,
    accumulator: f32,
    /// Whether the simulation is paused.
    pub paused: bool,
    /// Simulated seconds since Play.
    pub elapsed: f32,
    /// Physics steps run since Play.
    pub steps: u64,
    /// The player's controls (applied to every character controller).
    pub input: PlayInput,
    characters: Vec<Character>,
    /// Entities with a behaviour script: Python drives them (characters
    /// with a script skip the built-in WASD controller).
    scripted: HashSet<Entity>,
    /// Plays the scene's Audio Sources.
    sources: AudioSourcePlayer,
    /// Sound commands from scripts, applied by [`Self::update_audio`].
    audio_commands: Vec<AudioCommand>,
    /// What scripts drew on the HUD in their latest frame.
    pub hud: Vec<HudItem>,
    /// A cursor lock request from a script, taken by the app.
    pub lock_request: Option<bool>,
    /// Collision triangles of mesh / convex-hull colliders by shape key,
    /// read out of the physics world once (scripts raycast against them).
    mesh_tris: std::cell::RefCell<HashMap<u64, std::sync::Arc<MeshTris>>>,
    /// The graphics preset the game's settings ask for
    /// (`ctx.settings.graphics`); Play renders at it instead of the
    /// editor's preset.
    pub graphics: Option<automata_renderer::GraphicsQuality>,
    /// The master volume the game's settings ask for.
    volume: Option<f32>,
    /// The master volume last given to the audio engine.
    applied_volume: Option<f32>,
    /// The scene's navigation mesh (loaded or baked at Play), if any.
    nav: Option<std::sync::Arc<automata_nav::NavMesh>>,
    /// What getting the navigation mesh ready had to say (for the
    /// Console), taken by the app.
    pub nav_warning: Option<String>,
}

/// The document as simulated (a play copy) and its ECS scene, with
/// model paths pinned to the document's folder.
fn simulated_scene(doc: &Document) -> (Document, Scene) {
    let display = doc.play_copy();
    let mut scene = Scene::new();
    let mut file = display.to_scene_file();
    // Mesh / hull colliders load their model by path: pin it to the
    // document's folder so it doesn't hang on the global asset root.
    for entity in &mut file.entities {
        if let Some(
            automata_physics::Shape3D::Mesh { model }
            | automata_physics::Shape3D::ConvexHull { model },
        ) = entity.collider_3d.as_mut().map(|c| &mut c.shape)
        {
            *model = doc.resolve(model).to_string_lossy().into_owned();
        }
        // Models too, so play needn't rely on the global asset root.
        if let Some(automata_core::PrimitiveData::Model { path }) =
            entity.mesh_visual.as_mut().map(|m| &mut m.primitive)
        {
            *path = doc.resolve(path).to_string_lossy().into_owned();
        }
    }
    file.apply_to_world(scene.world_mut());
    (display, scene)
}

impl PlaySession {
    /// Starts a session from the edited document.
    #[must_use]
    pub fn start(doc: &Document) -> Self {
        let (display, mut scene) = simulated_scene(doc);

        // Map document entries to ECS entities by name (the scene file keeps
        // names; duplicates are matched in order of appearance).
        let mut by_name: HashMap<String, Vec<Entity>> = HashMap::new();
        for (entity, name) in &mut scene.world().query::<(Entity, &Name)>() {
            by_name.entry(name.0.clone()).or_default().push(entity);
        }
        for list in by_name.values_mut() {
            // hecs iterates in archetype order; sort by id so duplicates map
            // in spawn order.
            list.sort_by_key(|e| e.id());
            list.reverse(); // pop() takes the earliest
        }
        let mut map = Vec::new();
        for (i, entry) in display.entities.iter().enumerate() {
            if entry.data.transform.is_none() {
                continue;
            }
            if let Some(entity) = by_name.get_mut(entry.name()).and_then(Vec::pop) {
                map.push((i, entity));
            }
        }

        let scripted = map
            .iter()
            .filter(|(i, _)| display.entities[*i].data.script.is_some())
            .map(|&(_, entity)| entity)
            .collect();

        // Nav agents with a controller walk themselves, not by the input.
        let characters = scene
            .world()
            .query::<(Entity, &CharacterController)>()
            .without::<&automata_nav::NavAgent>()
            .iter()
            .map(|(entity, _)| Character {
                entity,
                vertical: 0.0,
                two_d: scene
                    .world()
                    .get::<&automata_physics::Collider2D>(entity)
                    .is_ok(),
            })
            .collect();

        let mut physics = PhysicsWorld3D::new();
        physics.sync_from_scene(scene.world_mut());
        let mut physics2d = automata_physics::PhysicsWorld2D::new();
        physics2d.sync_from_scene(scene.world_mut());
        let prepared = automata_nav::scene::prepare(scene.world(), &physics, Some(&doc.base_dir()));
        let (nav, nav_warning) = prepared.map_or((None, None), |p| {
            (p.mesh.map(std::sync::Arc::new), p.warning)
        });
        Self {
            doc: display,
            scene,
            physics,
            physics2d,
            map,
            accumulator: 0.0,
            paused: false,
            elapsed: 0.0,
            steps: 0,
            input: PlayInput::default(),
            characters,
            scripted,
            sources: AudioSourcePlayer::new(),
            audio_commands: Vec::new(),
            hud: Vec::new(),
            lock_request: None,
            mesh_tris: std::cell::RefCell::default(),
            graphics: None,
            volume: None,
            applied_volume: None,
            nav,
            nav_warning,
        }
    }

    /// The navigation mesh agents walk in this session, if any.
    #[must_use]
    pub fn nav(&self) -> Option<&std::sync::Arc<automata_nav::NavMesh>> {
        self.nav.as_ref()
    }

    /// Where each nav agent is heading: `(position, corners ahead)`, for
    /// the overlay.
    #[must_use]
    pub fn agent_paths(&self) -> Vec<(Vec3, Vec<Vec3>)> {
        self.scene
            .world()
            .query::<(&Transform, &automata_nav::NavAgent)>()
            .iter()
            .filter(|(_, a)| !a.state.arrived)
            .map(|(t, a)| {
                let s = &a.state;
                let ahead = s.path.get(s.next..).unwrap_or_default();
                (t.position, ahead.iter().map(|p| p.position).collect())
            })
            .collect()
    }

    /// Bakes `doc`'s navigation mesh with its Navigation's settings (the
    /// defaults without one) from its static colliders.
    ///
    /// # Errors
    ///
    /// When there's no floor to walk on, or the level is too big.
    pub fn bake_nav(doc: &Document) -> Result<automata_nav::NavMesh, automata_nav::BakeError> {
        let (_, mut scene) = simulated_scene(doc);
        let mut physics = PhysicsWorld3D::new();
        physics.sync_from_scene(scene.world_mut());
        let settings = automata_nav::scene::navigation(scene.world())
            .unwrap_or_default()
            .settings
            .sanitized();
        automata_nav::scene::bake_scene(scene.world(), &physics, settings)
    }

    /// Bakes `doc`'s navigation mesh and saves it next to the scene as
    /// `<scene>.nav.ron`, pointing entity `id`'s Navigation at it (one
    /// undo step). Returns the file and the mesh.
    ///
    /// # Errors
    ///
    /// A message for the Console: the scene isn't saved yet, there's no
    /// floor, or the file can't be written.
    pub fn bake_nav_to_file(
        doc: &mut Document,
        id: u64,
    ) -> Result<(PathBuf, automata_nav::NavMesh), String> {
        let Some(scene_path) = doc.path.clone() else {
            return Err("Save the scene first: the navigation mesh is saved next to it".into());
        };
        let mesh = Self::bake_nav(doc).map_err(|e| format!("Couldn't bake: {e}"))?;
        let stem = scene_path
            .file_stem()
            .and_then(|s| s.to_str())
            .map_or("level", |s| s.trim_end_matches(".scene"));
        let name = format!("{stem}.nav.ron");
        let path = doc.base_dir().join(&name);
        let text = mesh
            .to_ron()
            .map_err(|e| format!("Couldn't save the mesh: {e}"))?;
        std::fs::write(&path, text)
            .map_err(|e| format!("Couldn't save {}: {e}", path.display()))?;
        doc.modify(id, false, |d| {
            if let Some(n) = &mut d.navigation {
                n.file.clone_from(&name);
            }
        });
        Ok((path, mesh))
    }

    /// The document id of a simulated entity.
    fn doc_id(&self, entity: Entity) -> Option<u64> {
        self.map
            .iter()
            .find(|(_, e)| *e == entity)
            .and_then(|&(i, _)| self.doc.entities.get(i))
            .map(|e| e.id)
    }

    /// Mouse-independent frame data for scripts: solid colliders (for
    /// raycasts) and which Audio Sources are playing.
    #[must_use]
    pub fn frame_input(&self) -> FrameInput {
        let to4 = |q: glam::Quat| [q.x, q.y, q.z, q.w];
        let snapshot = self.physics.collider_snapshot();
        let mut meshes: Vec<(u64, std::sync::Arc<MeshTris>)> = Vec::new();
        let mut cache = self.mesh_tris.borrow_mut();
        let colliders = snapshot
            .shapes()
            .into_iter()
            .filter(|(_, _, sensor)| !*sensor)
            .filter_map(|(entity, shape, _)| {
                let id = self.doc_id(entity)?;
                Some(match shape {
                    WorldShape::Cuboid {
                        center,
                        rotation,
                        half_extents,
                    } => ColliderShape::Cuboid {
                        id,
                        center: center.to_array(),
                        rotation: to4(rotation),
                        half: half_extents.to_array(),
                    },
                    WorldShape::Ball { center, radius } => ColliderShape::Ball {
                        id,
                        center: center.to_array(),
                        radius,
                    },
                    WorldShape::Capsule {
                        center,
                        rotation,
                        half_height,
                        radius,
                    } => ColliderShape::Capsule {
                        id,
                        center: center.to_array(),
                        rotation: to4(rotation),
                        half_height,
                        radius,
                    },
                    WorldShape::Mesh {
                        center,
                        rotation,
                        mesh,
                    } => {
                        let tris = match cache.entry(mesh) {
                            std::collections::hash_map::Entry::Occupied(e) => e.get().clone(),
                            std::collections::hash_map::Entry::Vacant(e) => {
                                let (vertices, triangles) = snapshot.mesh_triangles(mesh)?;
                                // Terrains travel as their grid alone.
                                let grid = snapshot.mesh_grid(mesh);
                                let tris = MeshTris {
                                    vertices: vertices.iter().map(glam::Vec3::to_array).collect(),
                                    triangles: if grid.is_some() {
                                        Vec::new()
                                    } else {
                                        triangles
                                    },
                                    grid,
                                };
                                e.insert(std::sync::Arc::new(tris)).clone()
                            }
                        };
                        if !meshes.iter().any(|(k, _)| *k == mesh) {
                            meshes.push((mesh, tris));
                        }
                        ColliderShape::Mesh {
                            id,
                            center: center.to_array(),
                            rotation: to4(rotation),
                            mesh,
                        }
                    }
                })
            })
            .collect();
        drop(cache);
        let playing = self
            .map
            .iter()
            .filter(|(_, e)| self.sources.is_playing(*e))
            .filter_map(|&(i, _)| self.doc.entities.get(i).map(|e| e.id))
            .collect();
        let water = self
            .physics
            .water_surfaces()
            .iter()
            .map(|w| w.pack().to_vec())
            .collect();
        FrameInput {
            colliders,
            meshes,
            playing,
            water,
            nav: self.nav.clone(),
            ..FrameInput::default()
        }
    }

    /// Applies a whole runner reply: entity writes, sounds, splashes, HUD,
    /// cursor.
    pub fn apply_frame(&mut self, reply: &FrameReply) {
        self.apply_script_writes(&reply.writes);
        for s in &reply.sounds {
            self.audio_commands.push(AudioCommand::OneShot {
                path: s.path.clone(),
                volume: s.volume,
                pitch: s.pitch,
                position: s.position.map(Vec3::from),
            });
        }
        for &(position, strength) in &reply.splashes {
            self.physics.splash(Vec3::from(position), strength);
        }
        for b in &reply.bursts {
            automata_scene::spawn_burst(
                self.scene.world_mut(),
                b.emitter.clone(),
                Vec3::from(b.position),
                b.direction.map(Vec3::from),
                b.count,
            );
        }
        for &(id, count) in &reply.emits {
            let entity = self
                .map
                .iter()
                .find(|(i, _)| self.doc.entities.get(*i).is_some_and(|e| e.id == id))
                .map(|&(_, entity)| entity);
            if let Some(entity) = entity {
                automata_scene::burst(self.scene.world_mut(), entity, count);
            }
        }
        self.hud.clone_from(&reply.hud);
        if reply.lock.is_some() {
            self.lock_request = reply.lock;
        }
        if let Some(volume) = reply.settings.volume {
            self.volume = Some(volume);
        }
        if reply.settings.graphics.is_some() {
            self.graphics = reply.settings.graphics;
        }
    }

    /// Runs the scene's audio for this frame: `play_on_start` sources,
    /// script requests and distance fading, all muffled while the scene
    /// camera is under water. `root` is the scene folder. Returns problems
    /// for the Console (missing clips, once each).
    pub fn update_audio(
        &mut self,
        engine: &mut AudioEngine,
        root: &std::path::Path,
    ) -> Vec<String> {
        if self.paused {
            return Vec::new();
        }
        engine.set_underwater(self.camera_under_water());
        if self.volume != self.applied_volume {
            if let Some(volume) = self.volume {
                engine.set_master_volume(f64::from(volume), 0.0);
            }
            self.applied_volume = self.volume;
        }
        let commands = std::mem::take(&mut self.audio_commands);
        self.sources
            .update(self.scene.world(), engine, Some(root), commands)
    }

    /// Whether the scene camera is below the water surface over it.
    #[must_use]
    pub fn camera_under_water(&self) -> bool {
        self.scene_camera().is_some_and(|cam| {
            let p = cam.position;
            self.physics.water_height(p.x, p.z).is_some_and(|h| p.y < h)
        })
    }

    /// Stops every sound this session started (Stop / pause), and lifts
    /// the underwater muffle.
    pub fn stop_audio(&mut self, engine: &mut AudioEngine) {
        self.audio_commands.clear();
        self.sources.stop_all(engine);
        engine.set_underwater(false);
        if self.applied_volume.take().is_some() {
            engine.set_master_volume(1.0, 0.0);
        }
    }

    /// Pauses or resumes this session's sounds.
    pub fn pause_audio(&self, engine: &mut AudioEngine, paused: bool) {
        if paused {
            self.sources.pause_all(engine);
        } else {
            self.sources.resume_all(engine);
        }
    }

    /// Number of sounds this session is playing.
    #[must_use]
    pub fn voice_count(&self) -> usize {
        self.sources.voice_count()
    }

    /// Removes a simulated entity (a script despawned it).
    fn despawn(&mut self, entity: Entity) {
        let Some(pos) = self.map.iter().position(|(_, e)| *e == entity) else {
            return;
        };
        let (index, _) = self.map.remove(pos);
        for (i, _) in &mut self.map {
            if *i > index {
                *i -= 1;
            }
        }
        self.doc.entities.remove(index);
        self.scripted.remove(&entity);
        self.characters.retain(|c| c.entity != entity);
        let _ = self.scene.world_mut().despawn(entity);
    }

    /// Number of entities with a behaviour script.
    #[must_use]
    pub fn script_count(&self) -> usize {
        self.scripted.len()
    }

    /// Every simulated entity's state for the script runner; `resolve`
    /// turns scene-relative script paths into absolute ones.
    #[must_use]
    pub fn script_states(&self, resolve: impl Fn(&str) -> PathBuf) -> Vec<EntityState> {
        let world = self.scene.world();
        self.map
            .iter()
            .filter_map(|&(i, entity)| {
                let entry = self.doc.entities.get(i)?;
                let t = TransformData::from(*world.get::<&Transform>(entity).ok()?);
                let character = world.get::<&CharacterController>(entity).is_ok();
                let velocity = character.then(|| {
                    world
                        .get::<&CharacterVelocity>(entity)
                        .map_or([0.0; 3], |v| v.velocity.to_array())
                });
                Some(EntityState {
                    id: entry.id,
                    name: entry.name().to_string(),
                    script: entry.data.script.as_ref().map(|s| resolve(&s.path)),
                    position: t.position,
                    rotation: t.rotation_euler,
                    // The display document keeps the authored (or
                    // script-set) scale; physics never changes it.
                    scale: entry.data.transform.map_or(t.scale, |d| d.scale),
                    velocity,
                    grounded: character
                        && (self.physics.grounded(entity) || self.physics2d.grounded(entity)),
                    attributes: entry.data.attributes.clone(),
                    camera: world.get::<&Camera3D>(entity).ok().map(|c| {
                        let (p, t) = (c.position, c.target);
                        [p.x, p.y, p.z, t.x, t.y, t.z, c.fov_y_degrees]
                    }),
                    has_audio: world.get::<&AudioSource>(entity).is_ok(),
                    anim: automata_scene::animation_state(world, entity),
                    joint: world.get::<&automata_physics::Joint>(entity).ok().map(|j| {
                        let state = world
                            .get::<&automata_physics::JointState>(entity)
                            .map(|s| *s)
                            .unwrap_or_default();
                        ((*j).clone(), state)
                    }),
                    attached: world
                        .get::<&automata_scene::Attachment>(entity)
                        .ok()
                        .map(|a| (a.to.clone(), a.bone.clone())),
                    nav: world
                        .get::<&automata_nav::NavAgent>(entity)
                        .ok()
                        .map(|a| (*a).clone()),
                    rag: automata_scene::ragdoll_status(world, entity),
                    spr: world
                        .get::<&automata_scene::Sprite2D>(entity)
                        .ok()
                        .map(|s| (*s).clone()),
                })
            })
            .collect()
    }

    /// The animation graphs running, for the graph editor: `(entity index
    /// in the document, graph path as written, its playback)` for every
    /// entity with an Animation whose graph runs.
    #[must_use]
    pub fn anim_graphs(&self) -> Vec<(usize, String, automata_scene::GraphState)> {
        let world = self.scene.world();
        self.map
            .iter()
            .filter(|&&(i, _)| {
                self.doc
                    .entities
                    .get(i)
                    .is_some_and(|e| e.data.animation.is_some())
            })
            .filter_map(|&(i, entity)| {
                let s = automata_scene::animation_state(world, entity)?;
                Some((i, s.graph_path?, s.graph?))
            })
            .collect()
    }

    /// Applies what scripts changed (transforms, character velocities,
    /// attributes) to the simulation and the display document.
    pub fn apply_script_writes(&mut self, writes: &[EntityWrite]) {
        if writes.is_empty() {
            return;
        }
        let mut despawned = Vec::new();
        for w in writes {
            let Some(&(i, entity)) = self
                .map
                .iter()
                .find(|(i, _)| self.doc.entities.get(*i).is_some_and(|e| e.id == w.id))
            else {
                continue;
            };
            match w.audio {
                Some(AudioWrite::Play) => self.audio_commands.push(AudioCommand::Play(entity)),
                Some(AudioWrite::Stop) => self.audio_commands.push(AudioCommand::Stop(entity)),
                None => {}
            }
            if w.despawn {
                despawned.push(entity);
                continue;
            }
            let world = self.scene.world_mut();
            for command in &w.anim {
                automata_scene::command_animation(world, entity, command);
            }
            if let Some(edit) = &w.joint {
                if let Ok(mut joint) = world.get::<&mut automata_physics::Joint>(entity) {
                    edit.apply(&mut joint);
                }
            }
            match &w.attach {
                Some(AttachWrite::To { to, bone, offset }) => {
                    let a = automata_scene::Attachment::new(to, bone, Transform::from(*offset));
                    let _ = world.insert_one(entity, a);
                }
                Some(AttachWrite::Detach) => {
                    let _ = world.remove_one::<automata_scene::Attachment>(entity);
                }
                None => {}
            }
            if let Some(edit) = &w.nav {
                if let Ok(mut agent) = world.get::<&mut automata_nav::NavAgent>(entity) {
                    edit.apply(&mut agent);
                }
            }
            if let Some(edit) = &w.rag {
                edit.clone().apply(world, entity);
            }
            if let Some(edit) = &w.spr {
                if let Ok(mut sprite) = world.get::<&mut automata_scene::Sprite2D>(entity) {
                    edit.apply(&mut sprite);
                }
            }
            if w.position.is_some() || w.rotation.is_some() || w.scale.is_some() {
                if let Ok(mut t) = world.get::<&mut Transform>(entity) {
                    let mut data = TransformData::from(*t);
                    if let Some(p) = w.position {
                        data.position = p;
                    }
                    if let Some(r) = w.rotation {
                        data.rotation_euler = r;
                    }
                    if let Some(s) = w.scale {
                        data.scale = s;
                    }
                    *t = Transform::from(data);
                }
                if let (Some(s), Some(slot)) =
                    (w.scale, self.doc.entities[i].data.transform.as_mut())
                {
                    slot.scale = s;
                }
            }
            if let Some(v) = w.velocity {
                let _ = world.insert_one(entity, CharacterVelocity::new(Vec3::from(v)));
            }
            if let Some(attributes) = &w.attributes {
                let _ = world.insert_one(entity, Attributes(attributes.clone()));
                self.doc.entities[i].data.attributes.clone_from(attributes);
            }
            if let Some(c) = w.camera {
                if let Ok(mut cam) = world.get::<&mut Camera3D>(entity) {
                    cam.position = Vec3::new(c[0], c[1], c[2]);
                    cam.target = Vec3::new(c[3], c[4], c[5]);
                    cam.fov_y_degrees = c[6].clamp(1.0, 179.0);
                    self.doc.entities[i].data.camera_3d = Some(*cam);
                }
            }
        }
        for entity in despawned {
            self.despawn(entity);
        }
        self.write_back();
    }

    /// The clock water waves run on in this session (the physics clock).
    #[must_use]
    pub fn water_time(&self) -> f32 {
        self.physics.water_time()
    }

    /// Keeps the ripple simulation around what a camera at `eye` looking at
    /// `target` sees (the camera Play draws through).
    pub fn set_ripple_view(&mut self, eye: Vec3, target: Vec3) {
        self.physics.set_ripple_view(eye, target);
    }

    /// The ripples to draw, or `None` while the water is undisturbed.
    pub fn ripples(&mut self) -> Option<automata_renderer::RippleMap> {
        self.physics.ripples()
    }

    /// Makes a splash in the water at `position` (see
    /// `PhysicsWorld3D::splash`).
    pub fn splash(&mut self, position: Vec3, strength: f32) {
        self.physics.splash(position, strength);
    }

    /// The scene's camera (the first entity with one), as the simulation
    /// currently has it (scripts may move it). Play mode renders through it.
    #[must_use]
    pub fn scene_camera(&self) -> Option<Camera3D> {
        let world = self.scene.world();
        let cam = world.query::<&Camera3D>().iter().next().copied();
        // A 2D scene is seen through its 2D camera (as an ortho one).
        cam.or_else(|| {
            automata_scene::scene_camera_2d(world).map(|c| automata_scene::ortho_camera_for(&c))
        })
    }

    /// Number of player-controllable characters.
    #[must_use]
    pub fn character_count(&self) -> usize {
        self.characters.len()
    }

    /// Number of dynamic (moving, gravity-affected) bodies.
    #[must_use]
    pub fn dynamic_count(&self) -> usize {
        self.scene
            .world()
            .query::<&RigidBody>()
            .iter()
            .filter(|b| b.enabled && b.body_type == BodyType::Dynamic)
            .count()
    }

    /// The first character's position (the camera follows it).
    #[must_use]
    pub fn player_position(&self) -> Option<Vec3> {
        let c = self.characters.first()?;
        let t = self.scene.world().get::<&Transform>(c.entity).ok()?;
        Some(t.position)
    }

    /// Number of simulated rigid bodies.
    #[must_use]
    pub fn body_count(&self) -> usize {
        self.physics.body_count()
    }

    /// Advances the simulation by `dt` real seconds (no-op while paused).
    pub fn update(&mut self, dt: f32) {
        if self.paused {
            return;
        }
        // Animation runs on real time, so it stays smooth between physics
        // steps. Feet stand on the colliders.
        let physics = &self.physics;
        let ground = |entity: Entity, from: Vec3, max: f32| {
            let filter = automata_physics::RayFilter {
                exclude_sensors: true,
                exclude_entity: Some(entity),
                ..automata_physics::RayFilter::default()
            };
            physics
                .cast_ray(from, Vec3::NEG_Y, max, &filter)
                .map(|hit| (hit.point, hit.normal))
        };
        automata_scene::animate_on(self.scene.world_mut(), dt.clamp(0.0, 0.25), Some(&ground));
        automata_scene::animate_sprites(self.scene.world_mut(), dt.clamp(0.0, 0.25));
        self.accumulator += dt.clamp(0.0, 0.25);
        let mut steps = 0;
        while self.accumulator >= TIMESTEP && steps < MAX_STEPS {
            self.accumulator -= TIMESTEP;
            steps += 1;
            self.step_once();
        }
        if steps == MAX_STEPS {
            self.accumulator = 0.0;
        }
        if steps > 0 {
            self.write_back();
            #[allow(clippy::cast_precision_loss)] // a handful of steps
            automata_scene::simulate_particles(self.scene.world_mut(), steps as f32 * TIMESTEP);
            // The models moved: what's attached to them follows.
            automata_scene::attach_to_bones(self.scene.world_mut());
        }
        // 2D cameras chase their targets where they ended up.
        automata_scene::follow_cameras_2d(self.scene.world_mut(), dt.clamp(0.0, 0.25));
        automata_scene::propagate_transforms(self.scene.world_mut());
    }

    /// Live particles in the play world (every emitter's, bursts included).
    #[must_use]
    pub fn particle_count(&self) -> usize {
        self.scene
            .world()
            .query::<&automata_scene::ParticleState>()
            .iter()
            .map(automata_scene::ParticleState::len)
            .sum()
    }

    /// Runs exactly one physics step (used by tests and frame stepping).
    pub fn step_once(&mut self) {
        let world = self.scene.world_mut();
        self.physics.sync_from_scene(world);
        self.drive_characters();
        let world = self.scene.world_mut();
        let physics = &self.physics;
        let grounded = |e: Entity| physics.grounded(e);
        automata_nav::step_agents(
            world,
            &automata_nav::AgentWorld {
                nav: self.nav.as_deref(),
                dt: TIMESTEP,
                grounded: &grounded,
            },
        );
        self.physics.sync_from_scene(world);
        self.physics.move_characters(world, TIMESTEP);
        let _tick = self.physics.step(world, TIMESTEP);
        self.physics2d.sync_from_scene(world);
        self.physics2d.move_characters(world, TIMESTEP);
        let _tick = self.physics2d.step(world, TIMESTEP);
        self.elapsed += TIMESTEP;
        self.steps += 1;
    }

    /// Turns the input into each character's desired velocity.
    fn drive_characters(&mut self) {
        let input = self.input;
        // Camera-relative axes on the ground plane (the eye sits at
        // +(sin yaw, _, cos yaw) from its target, looking back toward it).
        let (sy, cy) = input.camera_yaw.sin_cos();
        let forward = Vec3::new(-sy, 0.0, -cy);
        let right = Vec3::new(cy, 0.0, -sy);
        let mut wish = forward * input.forward + right * input.right;
        if wish.length_squared() > 1.0 {
            wish = wish.normalize();
        }
        let speed = if input.sprint {
            WALK_SPEED * 2.0
        } else {
            WALK_SPEED
        };
        let world = self.scene.world_mut();
        for c in &mut self.characters {
            if self.scripted.contains(&c.entity) {
                continue; // its script sets the velocity
            }
            let grounded = if c.two_d {
                self.physics2d.grounded(c.entity)
            } else {
                self.physics.grounded(c.entity)
            };
            c.vertical = if grounded && input.jump {
                JUMP_SPEED
            } else if grounded && c.vertical <= 0.0 {
                -2.0 // keep in contact with the ground
            } else {
                (c.vertical - CHARACTER_GRAVITY * TIMESTEP).max(-50.0)
            };
            // Side view: left/right walks along X (forward/back does
            // nothing), whichever way the camera looks.
            let along = if c.two_d {
                Vec3::X * input.right.clamp(-1.0, 1.0)
            } else {
                wish
            };
            let velocity = CharacterVelocity::new(along * speed + Vec3::Y * c.vertical);
            let _ = world.insert_one(c.entity, velocity);
        }
    }

    /// Copies simulated transforms into the display document and refreshes
    /// the world's global transforms for rendering.
    pub fn write_back(&mut self) {
        automata_scene::propagate_transforms(self.scene.world_mut());
        let world = self.scene.world();
        for &(i, entity) in &self.map {
            let Ok(t) = world.get::<&Transform>(entity) else {
                continue;
            };
            let sim = TransformData::from(*t);
            let slot = self.doc.entities.get_mut(i);
            if let Some(slot) = slot.and_then(|e| e.data.transform.as_mut()) {
                // Keep the authored scale (physics never changes it) to avoid
                // float drift from the round trip.
                *slot = TransformData {
                    scale: slot.scale,
                    ..sim
                };
            }
        }
    }

    /// The simulated ECS world's scene (rendered by the viewport).
    pub fn scene_mut(&mut self) -> &mut Scene {
        &mut self.scene
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::doc::CreateKind;

    /// Differences between an `expect.json` value and an observed one
    /// (numbers within 1e-3; booleans must be booleans).
    fn json_mismatches(
        expected: &crate::json::Json,
        actual: &crate::json::Json,
        path: &str,
    ) -> Vec<String> {
        use crate::json::Json;
        let differ = || {
            vec![format!(
                "{path}: expected {}, got {}",
                expected.dump(),
                actual.dump()
            )]
        };
        match (expected, actual) {
            (Json::Bool(a), Json::Bool(b)) if a == b => Vec::new(),
            (Json::Int(_) | Json::Float(_), Json::Int(_) | Json::Float(_)) => {
                let (a, b) = (expected.as_f64().unwrap(), actual.as_f64().unwrap());
                if (a - b).abs() <= 1e-3 {
                    Vec::new()
                } else {
                    differ()
                }
            }
            (Json::Str(a), Json::Str(b)) if a == b => Vec::new(),
            (Json::Arr(a), Json::Arr(b)) if a.len() == b.len() => a
                .iter()
                .zip(b)
                .enumerate()
                .flat_map(|(i, (e, g))| json_mismatches(e, g, &format!("{path}[{i}]")))
                .collect(),
            _ => differ(),
        }
    }

    /// Runs one `tests/script_conformance/` folder through Oberon's Play
    /// loop and the real bridge; returns what didn't match `expect.json`.
    fn run_conformance(python: &str, dir: &std::path::Path) -> Vec<String> {
        use crate::bridge::{attr_json, BridgeMsg, ScriptBridge};
        use crate::json::Json;
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        let expect =
            Json::parse(&std::fs::read_to_string(dir.join("expect.json")).unwrap()).unwrap();
        let frames = u64::try_from(expect.get("frames").and_then(Json::as_i64).unwrap()).unwrap();
        let doc = Document::load(&dir.join("scene.ron")).unwrap();
        let mut play = PlaySession::start(&doc);
        let runtime =
            std::env::temp_dir().join(format!("oberon_conformance_{name}_{}", std::process::id()));
        let mut bridge = ScriptBridge::start(python, &runtime, dir).unwrap();
        let mut problems = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            for msg in bridge.poll() {
                match msg {
                    BridgeMsg::Frame(w) => play.apply_frame(&w),
                    BridgeMsg::Error { msg, traceback, .. } => {
                        problems.push(format!("{name}: script error {msg}\n{traceback}"));
                    }
                    _ => {}
                }
            }
            if bridge.answered >= frames || std::time::Instant::now() > deadline {
                break;
            }
            let mut input = play.frame_input();
            input.screen = [1280, 720];
            let states = play.script_states(|rel| doc.resolve(rel));
            if bridge.send_frame_with(TIMESTEP, play.elapsed, &[], &states, &input) {
                play.update(TIMESTEP);
            } else {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        let answered = bridge.answered;
        drop(bridge.stop());
        let _ = std::fs::remove_dir_all(&runtime);
        if answered < frames {
            problems.push(format!("{name}: only {answered} of {frames} frames ran"));
            return problems;
        }
        let states = play.script_states(|rel| doc.resolve(rel));
        for (entity, want) in expect.get("entities").and_then(Json::as_obj).unwrap() {
            let got = states.iter().find(|s| &s.name == entity);
            let (want, got) = match (want, got) {
                (Json::Null, None) => continue,
                (Json::Null, Some(_)) => {
                    problems.push(format!("{name}: {entity}: expected gone, still there"));
                    continue;
                }
                (_, None) => {
                    problems.push(format!("{name}: {entity}: missing"));
                    continue;
                }
                (want, Some(got)) => (want, got),
            };
            for (key, value) in want.get("attributes").and_then(Json::as_obj).unwrap_or(&[]) {
                let path = format!("{name}: {entity}.attributes[{key:?}]");
                match got.attributes.get(key) {
                    Some(v) => problems.extend(json_mismatches(value, &attr_json(v), &path)),
                    None => problems.push(format!("{path}: missing")),
                }
            }
            for key in want
                .get("attributes_absent")
                .and_then(Json::as_arr)
                .unwrap_or(&[])
            {
                let key = key.as_str().unwrap();
                if got.attributes.contains_key(key) {
                    problems.push(format!(
                        "{name}: {entity}.attributes[{key:?}]: expected absent"
                    ));
                }
            }
            let fields = [
                ("position", Some(Json::floats(&got.position))),
                ("rotation", Some(Json::floats(&got.rotation))),
                ("scale", Some(Json::floats(&got.scale))),
                ("camera", got.camera.map(|c| Json::floats(&c))),
            ];
            for (key, actual) in fields {
                if let Some(value) = want.get(key) {
                    let path = format!("{name}: {entity}.{key}");
                    match actual {
                        Some(actual) => problems.extend(json_mismatches(value, &actual, &path)),
                        None => problems.push(format!("{path}: missing")),
                    }
                }
            }
        }
        problems
    }

    #[test]
    fn a_physics_cube_falls_onto_the_ground_and_the_document_is_untouched() {
        let mut doc = Document::starter();
        let id = doc.create(CreateKind::PhysicsCube);
        let idx = doc.index_of(id).unwrap();
        let start_y = doc.entities[idx].data.transform.unwrap().position[1];

        let mut play = PlaySession::start(&doc);
        assert!(play.body_count() >= 2, "ground + cube are simulated");
        for _ in 0..240 {
            play.update(TIMESTEP);
        }
        let y = play.doc.entities[idx].data.transform.unwrap().position[1];
        assert!(y < start_y - 1.0, "the cube fell (from {start_y} to {y})");
        assert!(y > 0.0, "the ground stopped it (y = {y})");
        // The edited document is unchanged.
        assert_eq!(
            doc.entities[idx].data.transform.unwrap().position[1],
            start_y
        );
    }

    #[test]
    fn the_player_walks_with_the_input_and_lands_on_the_ground() {
        let doc = Document::starter();
        let idx = doc
            .entities
            .iter()
            .position(|e| e.name() == "Player")
            .unwrap();
        let start = doc.entities[idx].data.transform.unwrap().position;
        let mut play = PlaySession::start(&doc);
        assert_eq!(play.character_count(), 1);
        assert!(play.dynamic_count() >= 3, "the crates are dynamic");
        // Camera yaw 0 looks down -Z: forward is -Z.
        play.input = PlayInput {
            forward: 1.0,
            ..PlayInput::default()
        };
        for _ in 0..60 {
            play.update(TIMESTEP);
        }
        let end = play.doc.entities[idx].data.transform.unwrap().position;
        assert!(
            end[2] < start[2] - 2.0,
            "walked forward: {start:?} -> {end:?}"
        );
        assert!(end[1] > 0.5 && end[1] < 1.5, "stays on the ground: {end:?}");
        let pos = play.player_position().unwrap();
        assert!((pos.z - end[2]).abs() < 1e-4);
    }

    #[test]
    fn the_navigation_mesh_bakes_and_saves_next_to_the_scene() {
        let dir = std::env::temp_dir().join(format!("oberon_nav_bake_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut doc = Document::starter();
        let id = doc.create(CreateKind::Empty);
        assert!(doc.add_component(id, crate::doc::ComponentKind::Navigation));
        assert!(
            PlaySession::bake_nav_to_file(&mut doc, id).is_err(),
            "an unsaved scene has nowhere to keep the mesh"
        );
        doc.save(Some(&dir.join("arena.scene.ron"))).unwrap();
        let (path, mesh) = PlaySession::bake_nav_to_file(&mut doc, id).unwrap();
        assert_eq!(path, dir.join("arena.nav.ron"));
        assert!(
            mesh.area() > 50.0,
            "the ground is walkable: {} m²",
            mesh.area()
        );
        let i = doc.index_of(id).unwrap();
        assert_eq!(
            doc.entities[i].data.navigation.as_ref().unwrap().file,
            "arena.nav.ron"
        );
        let loaded = automata_nav::NavMesh::load(&path).unwrap();
        assert!((loaded.area() - mesh.area()).abs() < 1e-3);
        // Playing uses the saved mesh.
        let play = PlaySession::start(&doc);
        assert!(
            play.nav().is_some() && play.nav_warning.is_none(),
            "{:?}",
            play.nav_warning
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nav_agents_walk_to_their_targets_in_play_mode() {
        let mut doc = Document::starter();
        let id = doc.create(CreateKind::Empty);
        assert!(doc.add_component(id, crate::doc::ComponentKind::NavAgent));
        let i = doc.index_of(id).unwrap();
        doc.modify(id, false, |d| {
            d.transform.as_mut().unwrap().position = [-3.0, 0.0, 3.0];
        });
        let mut play = PlaySession::start(&doc);
        assert!(play.nav().is_some(), "baked when play starts");
        let goal = glam::Vec3::new(3.0, 0.0, 3.0);
        for agent in play
            .scene
            .world_mut()
            .query_mut::<&mut automata_nav::NavAgent>()
        {
            agent.target = Some(automata_nav::NavTarget::Point(goal));
        }
        play.update(TIMESTEP);
        assert_eq!(play.agent_paths().len(), 1, "it has somewhere to go");
        for _ in 0..240 {
            play.update(TIMESTEP);
        }
        let end = glam::Vec3::from(play.doc.entities[i].data.transform.unwrap().position);
        assert!(end.distance(goal) < 0.3, "it got there: {end}");
        assert!(play.agent_paths().is_empty(), "and stopped");
    }

    #[test]
    fn joints_hold_in_play_mode_and_scripts_can_drive_them() {
        use automata_physics::{Joint, JointMotor};
        let mut doc = Document::new();
        let i = doc.add_entity_named("Hook");
        let hook = doc.entities[i].id;
        doc.modify(hook, false, |d| {
            d.transform = Some(TransformData {
                position: [0.0, 5.0, 0.0],
                ..TransformData::default()
            });
        });
        let i = doc.add_entity_named("Lamp");
        let lamp = doc.entities[i].id;
        doc.modify(lamp, false, |d| {
            d.transform = Some(TransformData {
                position: [1.5, 5.0, 0.0],
                ..TransformData::default()
            });
            crate::doc::ComponentKind::Joint.add_to(d);
            d.joint = Some(
                Joint::ball()
                    .connected_to("Hook")
                    .anchor(glam::Vec3::new(-1.5, 0.0, 0.0)),
            );
        });
        let i = doc.add_entity_named("Wheel");
        let wheel = doc.entities[i].id;
        doc.modify(wheel, false, |d| {
            d.transform = Some(TransformData {
                position: [6.0, 1.0, 0.0],
                ..TransformData::default()
            });
            crate::doc::ComponentKind::Joint.add_to(d);
            d.joint = Some(Joint::hinge(glam::Vec3::Z).motor(JointMotor::speed(2.0)));
        });
        let mut play = PlaySession::start(&doc);
        let pos = |play: &PlaySession, id: u64| {
            let e = play.doc.entities.iter().find(|e| e.id == id).unwrap();
            glam::Vec3::from(e.data.transform.unwrap().position)
        };
        let mut lowest = f32::MAX;
        for _ in 0..90 {
            play.update(TIMESTEP);
            let lamp_pos = pos(&play, lamp);
            lowest = lowest.min(lamp_pos.y);
            assert!(
                (lamp_pos.distance(glam::Vec3::new(0.0, 5.0, 0.0)) - 1.5).abs() < 0.05,
                "the lamp swings on its 1.5 m cord: {lamp_pos}"
            );
        }
        assert!(lowest < 3.6, "and swung through the bottom ({lowest})");
        let states = play.script_states(|p| PathBuf::from(p));
        let lamp_state = states.iter().find(|s| s.id == lamp).unwrap();
        let (joint, state) = lamp_state.joint.as_ref().expect("scripts see the joint");
        assert_eq!(joint.connected.as_deref(), Some("Hook"));
        assert!(state.attached && state.force > 0.0, "{state:?}");
        // A script lets go of the lamp.
        play.apply_script_writes(&[crate::bridge::EntityWrite {
            id: lamp,
            joint: Some(automata_physics::JointEdit {
                enabled: Some(false),
                ..Default::default()
            }),
            ..Default::default()
        }]);
        let before = pos(&play, lamp).y;
        for _ in 0..30 {
            play.update(TIMESTEP);
        }
        assert!(pos(&play, lamp).y < before - 0.5, "it fell once let go");
        assert!(
            pos(&play, wheel).distance(glam::Vec3::new(6.0, 1.0, 0.0)) < 0.05,
            "axle holds"
        );
    }

    #[test]
    fn pausing_freezes_the_simulation() {
        let mut doc = Document::starter();
        doc.create(CreateKind::PhysicsCube);
        let mut play = PlaySession::start(&doc);
        play.paused = true;
        play.update(1.0);
        assert_eq!(play.steps, 0);
    }

    /// The CC0 robot from the Robot Plaza example (14 clips).
    pub(crate) const ROBOT: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/oberon/robot_plaza/models/robot.glb"
    );

    #[test]
    fn animated_models_play_on_start_and_take_script_requests() {
        let mut doc = Document::new();
        let id = doc.create_model(ROBOT, "Robot", [0.0; 3]);
        let data = &mut doc.entities[0].data;
        assert_eq!(
            data.animation.as_ref().map(|a| a.clip.as_str()),
            Some("Idle"),
            "dropped-in animated models get an Animation"
        );
        let mut play = PlaySession::start(&doc);
        play.update(0.2);
        play.update(0.2);
        let state = |play: &PlaySession| {
            let states = play.script_states(|p| PathBuf::from(p));
            states
                .into_iter()
                .find(|s| s.id == id)
                .unwrap()
                .anim
                .unwrap()
        };
        let s = state(&play);
        assert_eq!(s.clip.as_deref(), Some("Idle"));
        assert!(s.playing && (s.time - 0.4).abs() < 1e-4, "{s:?}");
        assert_eq!(s.clips.len(), 14);

        play.apply_script_writes(&[EntityWrite {
            id,
            anim: vec![automata_scene::AnimationCommand::Play {
                clip: "Wave".into(),
                options: automata_scene::PlayOptions {
                    looping: false,
                    fade: 0.25,
                    ..automata_scene::PlayOptions::default()
                },
            }],
            ..EntityWrite::default()
        }]);
        play.update(0.1);
        let s = state(&play);
        assert_eq!(s.clip.as_deref(), Some("Wave"));
        assert!(!s.looping && (s.time - 0.1).abs() < 1e-4, "{s:?}");
        for _ in 0..30 {
            play.update(0.1);
        }
        assert!(!state(&play).playing, "a one-shot ends");
    }

    /// A 2D player lands on a tilemap's floor and walks along it with
    /// the arrow keys; its sprite's clip plays, scripts switch it, and
    /// the 2D camera follows.
    #[test]
    fn a_2d_player_walks_on_a_tilemap_and_the_camera_follows() {
        let mut doc = Document::new();
        let _map = doc.create(CreateKind::Tilemap);
        let cam = doc.create(CreateKind::Camera2D);
        let player = doc.create(CreateKind::Player2D);
        {
            let i = doc.index_of(player).unwrap();
            let sprite = doc.entities[i].data.sprite.as_mut().unwrap();
            sprite.columns = 4;
            sprite.clips = vec![
                automata_scene::SpriteClip::new("idle", 0, 1, 10.0),
                automata_scene::SpriteClip::new("run", 2, 3, 10.0),
            ];
            sprite.play = "idle".into();
        }
        assert!(crate::doc::scene_is_2d(&doc));
        let mut play = PlaySession::start(&doc);
        assert!(
            play.scene_camera().is_some_and(|c| c.ortho > 0.0),
            "seen through the 2D camera"
        );
        let pos = |play: &PlaySession, id: u64| {
            let i = play.doc.index_of(id).unwrap();
            play.doc.entities[i].data.transform.unwrap().position
        };
        for _ in 0..60 {
            play.update(1.0 / 60.0);
        }
        let landed = pos(&play, player);
        // The floor row's top is at y = -1; the collider is 1 tall.
        assert!(
            (landed[1] - (-0.5)).abs() < 0.15,
            "stands on the floor: {landed:?}"
        );
        let state = |play: &PlaySession| {
            play.script_states(|p| PathBuf::from(p))
                .into_iter()
                .find(|s| s.id == player)
                .unwrap()
        };
        let s = state(&play);
        assert!(s.grounded, "grounded in the 2D world");
        assert_eq!(s.spr.as_ref().unwrap().playing, "idle");
        play.input.right = 1.0;
        for _ in 0..60 {
            play.update(1.0 / 60.0);
        }
        let walked = pos(&play, player);
        assert!(
            walked[0] > landed[0] + 2.0,
            "walks right: {landed:?} -> {walked:?}"
        );
        assert!(
            (walked[1] - landed[1]).abs() < 0.15,
            "along the floor: {walked:?}"
        );
        let cam_at = pos(&play, cam);
        assert!(cam_at[0] > 1.0, "the camera follows: {cam_at:?}");
        play.apply_script_writes(&[EntityWrite {
            id: player,
            spr: Some(automata_scene::SpriteEdit {
                play: Some(("run".into(), false)),
                flip_x: Some(true),
                ..automata_scene::SpriteEdit::default()
            }),
            ..EntityWrite::default()
        }]);
        play.update(0.25);
        let spr = state(&play).spr.unwrap();
        assert_eq!(spr.playing, "run");
        assert!(spr.flip_x);
        assert!((2..=3).contains(&spr.frame), "a run frame: {}", spr.frame);
    }

    /// The 2D platformer example plays in Oberon: holding right and
    /// hopping, the player runs along the tilemap (its clips following),
    /// the camera keeps up, and the HUD counts coins.
    #[test]
    fn the_platformer_example_plays() {
        use crate::bridge::{BridgeMsg, HudKind, ScriptBridge};
        let python = crate::bridge::default_python();
        if std::process::Command::new(python)
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let game = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/oberon/platformer");
        let doc = Document::load(&game.join("level.ron")).unwrap();
        assert!(crate::doc::scene_is_2d(&doc), "opens in the 2D view");
        let state = std::env::temp_dir().join(format!("oberon_platformer_{}", std::process::id()));
        let mut play = PlaySession::start(&doc);
        play.update(TIMESTEP);
        let mut bridge = ScriptBridge::start(python, &state, &game).unwrap();
        let player = |play: &PlaySession| {
            play.script_states(|p| PathBuf::from(p))
                .into_iter()
                .find(|s| s.name == "Player")
                .unwrap()
        };
        let start = player(&play).position;
        let mut errors = Vec::new();
        let mut clips = HashSet::new();
        let mut hud = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while play.elapsed < 3.0 && std::time::Instant::now() < deadline {
            for msg in bridge.poll() {
                match msg {
                    BridgeMsg::Frame(w) => play.apply_frame(&w),
                    BridgeMsg::Error { msg, traceback, .. } => {
                        errors.push(format!("{msg}\n{traceback}"));
                    }
                    _ => {}
                }
            }
            if let Some(spr) = player(&play).spr {
                clips.insert(spr.playing.clone());
            }
            // Run right, jumping every 0.8 s (holding it 0.3 s).
            let hop = play.elapsed % 0.8 < 0.3;
            let keys: &[&str] = if hop { &["d", "space"] } else { &["d"] };
            let states = play.script_states(|rel| doc.resolve(rel));
            if bridge.send_frame_with(TIMESTEP, play.elapsed, keys, &states, &play.frame_input()) {
                play.update(TIMESTEP);
                hud.clone_from(&play.hud);
            } else {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        drop(bridge.stop());
        let _ = std::fs::remove_dir_all(&state);
        assert!(errors.is_empty(), "{errors:?}");
        let end = player(&play).position;
        assert!(end[0] > start[0] + 8.0, "ran right: {start:?} -> {end:?}");
        assert!(end[1] > 1.5, "still on the level: {end:?}");
        for clip in ["run", "jump", "fall"] {
            assert!(clips.contains(clip), "{clip} played: {clips:?}");
        }
        let cam = play
            .doc
            .entities
            .iter()
            .find(|e| e.name() == "Camera")
            .and_then(|e| e.data.transform)
            .unwrap()
            .position;
        assert!(cam[0] > 8.8, "the camera follows: {cam:?}");
        let labels: Vec<&str> = hud
            .iter()
            .filter_map(|h| match &h.kind {
                HudKind::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            labels.iter().any(|t| t.ends_with("/ 11")),
            "the coin count: {labels:?}"
        );
    }

    #[test]
    fn ragdolls_fall_in_play_mode_and_scripts_stand_them_back_up() {
        let mut doc = Document::starter();
        let id = doc.create_model(ROBOT, "Robot", [0.0, 0.0, 6.0]);
        assert!(doc.add_component(id, crate::doc::ComponentKind::Ragdoll));
        let i = doc.index_of(id).unwrap();
        let stand = doc.entities[i].data.transform.unwrap().position;
        let mut play = PlaySession::start(&doc);
        let rag = |play: &PlaySession| {
            play.script_states(|p| PathBuf::from(p))
                .into_iter()
                .find(|s| s.id == id)
                .unwrap()
                .rag
                .unwrap()
        };
        play.update(0.1);
        assert!(!rag(&play).limp, "it starts animated");
        play.apply_script_writes(&[EntityWrite {
            id,
            rag: Some(automata_scene::RagdollEdit {
                active: Some(true),
                pushes: vec![automata_scene::RagdollPush {
                    bone: None,
                    impulse: glam::Vec3::new(150.0, 0.0, 0.0),
                }],
                ..automata_scene::RagdollEdit::default()
            }),
            ..EntityWrite::default()
        }]);
        for _ in 0..100 {
            play.update(0.1);
            if rag(&play).resting {
                break;
            }
        }
        let down = rag(&play);
        assert!(down.limp && down.resting, "it fell and lay still: {down:?}");
        let hips = down.hips.unwrap();
        assert!(
            hips.y < stand[1] - 0.3,
            "on the floor: {hips} (stood at {stand:?})"
        );
        play.apply_script_writes(&[EntityWrite {
            id,
            rag: Some(automata_scene::RagdollEdit {
                active: Some(false),
                ..automata_scene::RagdollEdit::default()
            }),
            ..EntityWrite::default()
        }]);
        for _ in 0..10 {
            play.update(0.1);
        }
        let up = rag(&play);
        assert!(
            !up.limp && up.weight < 1e-3,
            "back in its animation: {up:?}"
        );
        let at = play.doc.entities[i].data.transform.unwrap().position;
        assert!(
            (at[1] - stand[1]).abs() < 0.05,
            "standing as tall as before: {at:?}"
        );
        assert!(at[0] > stand[0] + 0.3, "where it fell: {at:?}");
    }

    fn player_index(doc: &Document) -> usize {
        doc.entities
            .iter()
            .position(|e| e.name() == "Player")
            .unwrap()
    }

    #[test]
    fn scripted_entities_are_reported_and_take_script_writes() {
        let mut doc = Document::starter();
        let idx = player_index(&doc);
        let id = doc.entities[idx].id;
        doc.entities[idx].data.script = Some(automata_core::Script {
            path: "scripts/player.py".into(),
        });
        doc.entities[idx]
            .data
            .attributes
            .insert("hp".into(), automata_core::AttributeValue::Int(3));
        let mut play = PlaySession::start(&doc);
        assert_eq!(play.script_count(), 1);

        let states = play.script_states(|rel| PathBuf::from("/game").join(rel));
        let me = states.iter().find(|s| s.id == id).unwrap();
        assert_eq!(me.script, Some(PathBuf::from("/game/scripts/player.py")));
        assert_eq!(me.attributes["hp"], automata_core::AttributeValue::Int(3));
        assert!(me.velocity.is_some(), "characters report their velocity");
        assert!(
            states.iter().any(|s| s.script.is_none()),
            "other entities come along"
        );

        // A scripted character ignores the built-in WASD controller.
        play.input = PlayInput {
            forward: 1.0,
            ..PlayInput::default()
        };
        let start = play.doc.entities[idx].data.transform.unwrap().position;
        let mut attrs = std::collections::BTreeMap::new();
        attrs.insert("hp".to_string(), automata_core::AttributeValue::Int(2));
        play.apply_script_writes(&[EntityWrite {
            id,
            velocity: Some([2.0, 0.0, 0.0]),
            attributes: Some(attrs),
            scale: Some([2.0, 2.0, 2.0]),
            ..EntityWrite::default()
        }]);
        for _ in 0..30 {
            play.update(TIMESTEP);
        }
        let end = play.doc.entities[idx].data.transform.unwrap();
        assert!(
            end.position[0] > start[0] + 0.5,
            "moved along +X by the script: {end:?}"
        );
        assert!(
            (end.position[2] - start[2]).abs() < 0.1,
            "not by WASD: {end:?}"
        );
        assert_eq!(end.scale, [2.0, 2.0, 2.0]);
        assert_eq!(
            play.doc.entities[idx].data.attributes["hp"],
            automata_core::AttributeValue::Int(2)
        );
        // The edited document is untouched.
        assert_eq!(
            doc.entities[idx].data.attributes["hp"],
            automata_core::AttributeValue::Int(3)
        );
    }

    /// The script conformance suite (`tests/script_conformance/`): every
    /// scenario runs through Oberon's Play loop and JSON bridge and must
    /// end as its `expect.json` says. The engine's runner is held to the
    /// same files by `crates/automata-python/tests/test_script_conformance.py`
    /// (skipped without python3).
    #[test]
    fn script_conformance_scenarios_match_expectations() {
        let python = crate::bridge::default_python();
        if std::process::Command::new(python)
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/script_conformance");
        let mut dirs: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.join("expect.json").is_file())
            .collect();
        dirs.sort();
        assert!(dirs.len() >= 7, "{dirs:?}");
        let problems: Vec<String> = dirs
            .iter()
            .flat_map(|d| run_conformance(python, d))
            .collect();
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    /// The finished tutorial project plays in Oberon: every script loads and
    /// runs without errors, the intro flies the camera rail, and then the
    /// Ponytail Park through the real script bridge: footsteps (animation
    /// events from the import settings) reach the player script's
    /// `*:animation:footstep` subscriber, which kicks up dust, and pressing
    /// 3 leaps her 2.4 m forward by root motion (skipped without python3).
    #[test]
    fn ponytail_park_hears_footsteps_and_leaps_by_root_motion() {
        use crate::bridge::{BridgeMsg, ScriptBridge};
        let python = crate::bridge::default_python();
        if std::process::Command::new(python)
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let game = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/oberon/ponytail_park");
        let doc = Document::load(&game.join("level.ron")).unwrap();
        let mut play = PlaySession::start(&doc);
        play.update(TIMESTEP); // as the editor does before the first frame
        let runtime = std::env::temp_dir().join(format!("oberon_ponytail_{}", std::process::id()));
        let mut bridge = ScriptBridge::start(python, &runtime, &game).unwrap();
        let runner = |play: &PlaySession| {
            let states = play.script_states(|rel| doc.resolve(rel));
            states.into_iter().find(|s| s.name == "Runner").unwrap()
        };
        let start = Vec3::from(runner(&play).position);
        let (mut errors, mut most_dust, mut pressed) = (Vec::new(), 0, None);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        // The jogger sets off after 1 s; leap at 2.5 s and land by 3.8 s.
        while play.elapsed < 4.0 && std::time::Instant::now() < deadline {
            for msg in bridge.poll() {
                match msg {
                    BridgeMsg::Frame(w) => play.apply_frame(&w),
                    BridgeMsg::Error { msg, .. } => errors.push(msg),
                    _ => {}
                }
            }
            let states = play.script_states(|rel| doc.resolve(rel));
            let keys: &[&str] = if play.elapsed >= 2.5 && pressed.is_none() {
                &["3"]
            } else {
                &[]
            };
            if bridge.send_frame(TIMESTEP, play.elapsed, keys, &states) {
                if !keys.is_empty() {
                    pressed = Some(play.elapsed);
                }
                play.update(TIMESTEP);
                most_dust = most_dust.max(play.particle_count());
            } else {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        drop(bridge.stop());
        let _ = std::fs::remove_dir_all(&runtime);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(play.elapsed >= 4.0, "the runner kept up: {}", play.elapsed);
        assert!(most_dust > 0, "footsteps kick up dust");
        let end = runner(&play);
        let travelled = start - Vec3::from(end.position);
        // She faces -Z: the leap carries her 2.4 m that way, and no further.
        assert!(
            (travelled.z - 2.4).abs() < 0.1 && travelled.x.abs() < 0.01,
            "leapt {travelled} (pressed at {pressed:?})"
        );
        assert_eq!(end.anim.unwrap().clip.as_deref(), Some("Idle"));
        let landed = runner(&play).anim.unwrap().events;
        assert!(landed
            .iter()
            .any(|e| e.name == "land" && e.clip == "emotes:Leap"));
    }

    /// camera settles in behind the player (skipped without python3).
    #[test]
    fn the_coin_run_tutorial_project_plays() {
        use crate::bridge::{BridgeMsg, ScriptBridge};
        let python = crate::bridge::default_python();
        if std::process::Command::new(python)
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let level = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/oberon/coin_run/level.ron");
        let doc = Document::load(&level).unwrap();
        let mut play = PlaySession::start(&doc);
        let runtime = std::env::temp_dir().join(format!("oberon_coin_run_{}", std::process::id()));
        let mut bridge = ScriptBridge::start(python, &runtime, &doc.base_dir()).unwrap();
        let mut errors = Vec::new();
        let mut intro_distance = 0.0_f32;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        // 7 s of play: the 5 s flyover, the 1 s blend, and a second of follow.
        while play.elapsed < 7.0 && std::time::Instant::now() < deadline {
            for msg in bridge.poll() {
                match msg {
                    BridgeMsg::Frame(w) => play.apply_frame(&w),
                    BridgeMsg::Error { msg, .. } => errors.push(msg),
                    _ => {}
                }
            }
            let states = play.script_states(|rel| doc.resolve(rel));
            if bridge.send_frame(TIMESTEP, play.elapsed, &[], &states) {
                play.update(TIMESTEP);
                if (play.elapsed - 2.5).abs() < TIMESTEP {
                    let (cam, player) = (
                        play.scene_camera().unwrap(),
                        play.player_position().unwrap(),
                    );
                    intro_distance = cam.position.distance(player);
                }
            } else {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        drop(bridge.stop());
        let _ = std::fs::remove_dir_all(&runtime);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(play.elapsed >= 7.0, "the runner kept up: {}", play.elapsed);
        assert!(
            intro_distance > 10.0,
            "mid-intro the camera is out on the rail: {intro_distance}"
        );
        let (cam, player) = (
            play.scene_camera().unwrap(),
            play.player_position().unwrap(),
        );
        let distance = cam.position.distance(player);
        assert!(
            (6.0..12.0).contains(&distance),
            "then it follows the player: {distance}"
        );
        assert!(
            cam.target.distance(player) < 2.0,
            "looking at them: {:?} vs {player:?}",
            cam.target
        );
    }

    /// `ctx.ui` through the real runner: the cursor Oberon sends lands on
    /// a button, the click reaches the script and not the game, and the
    /// button is drawn into the HUD (skipped without python3).
    #[test]
    fn script_ui_buttons_click_through_the_bridge() {
        use crate::bridge::{BridgeMsg, ScriptBridge};
        use automata_core::AttributeValue as A;
        let python = crate::bridge::default_python();
        if std::process::Command::new(python)
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let dir = std::env::temp_dir().join(format!("oberon_play_ui_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("menu.py"),
            "def update(entity, ctx):\n\
             \x20   a = entity.attributes\n\
             \x20   if ctx.ui.button('Play', 100, 100, 200, 50):\n\
             \x20       a['clicks'] = a.get('clicks', 0) + 1\n\
             \x20   if ctx.input.mouse_just_pressed('left'):\n\
             \x20       a['fired'] = a.get('fired', 0) + 1\n\
             \x20   a['pos'] = str(ctx.input.mouse_position)\n",
        )
        .unwrap();
        let mut doc = Document::new();
        doc.path = Some(dir.join("level.ron"));
        let i = doc.add_entity_named("Menu");
        let menu = doc.entities[i].id;
        doc.modify(menu, false, |d| {
            d.script = Some(automata_core::Script {
                path: "menu.py".into(),
            });
        });
        let mut play = PlaySession::start(&doc);
        let mut bridge = ScriptBridge::start(python, &dir.join(".oberon"), &dir).unwrap();
        let mut errors = Vec::new();
        let mut sent = 0;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while bridge.answered < 8 && std::time::Instant::now() < deadline {
            for msg in bridge.poll() {
                match msg {
                    BridgeMsg::Frame(w) => play.apply_frame(&w),
                    BridgeMsg::Error { msg, traceback, .. } => {
                        errors.push(format!("{msg}\n{traceback}"));
                    }
                    _ => {}
                }
            }
            let mut input = play.frame_input();
            input.screen = [800, 600];
            input.mouse_pos = Some([150.0, 120.0]);
            // Held on frames 3 and 4: one press, one release.
            if (3..5).contains(&sent) {
                input.mouse_buttons = vec!["left"];
            }
            let states = play.script_states(|rel| doc.resolve(rel));
            if bridge.send_frame_with(TIMESTEP, play.elapsed, &[], &states, &input) {
                sent += 1;
                play.update(TIMESTEP);
            } else {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        drop(bridge.stop());
        let _ = std::fs::remove_dir_all(&dir);
        assert!(errors.is_empty(), "{errors:?}");
        let e = play.doc.entities.iter().find(|e| e.id == menu).unwrap();
        let attr = |k: &str| e.data.attributes.get(k).cloned();
        assert_eq!(attr("clicks"), Some(A::Int(1)));
        assert_eq!(attr("fired"), None, "the game never saw the click");
        assert_eq!(attr("pos"), Some(A::Text("(150.0, 120.0)".into())));
        assert!(
            play.hud.len() >= 2,
            "the button's box and text: {:?}",
            play.hud
        );
    }

    /// FPS building blocks through the real runner: raycasts against the
    /// session's colliders, mouse input, cursor lock, the HUD, Audio Source
    /// playback and one-shots, and despawning (skipped without python3).
    #[test]
    fn scripts_raycast_play_sounds_draw_a_hud_and_despawn() {
        use crate::bridge::{BridgeMsg, ScriptBridge};
        use crate::doc::ComponentKind;
        use automata_core::AttributeValue as A;
        let python = crate::bridge::default_python();
        if std::process::Command::new(python)
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let dir = std::env::temp_dir().join(format!("oberon_play_fps_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sounds")).unwrap();
        crate::sounds::tests::write_test_wav(&dir.join("sounds/beep.wav"), 22_050, 2.0);
        std::fs::write(
            dir.join("shooter.py"),
            "def start(entity, ctx):\n\
             \x20   ctx.input.lock_mouse()\n\
             \n\
             def update(entity, ctx):\n\
             \x20   hit = ctx.physics.raycast((20, 1, 0), (0, 0, -1), 50, ignore=entity)\n\
             \x20   entity.attributes['hit'] = hit.entity.name if hit and hit.entity else 'none'\n\
             \x20   entity.attributes['dist'] = hit.distance if hit else -1.0\n\
             \x20   entity.attributes['dx'] = ctx.input.mouse_delta[0]\n\
             \x20   entity.attributes['click'] = ctx.input.mouse_pressed('left')\n\
             \x20   entity.attributes['locked'] = ctx.input.mouse_locked\n\
             \x20   ctx.hud.rect(ctx.hud.center[0], ctx.hud.center[1], 4, 4)\n\
             \x20   n = entity.attributes.get('n', 0) + 1\n\
             \x20   entity.attributes['n'] = n\n\
             \x20   if n == 3:\n\
             \x20       entity.audio.play()\n\
             \x20       ctx.audio.play('sounds/beep.wav', volume=0.5, position=entity)\n\
             \x20   if n == 6 and hit and hit.entity:\n\
             \x20       hit.entity.despawn()\n",
        )
        .unwrap();

        let mut doc = Document::starter();
        doc.path = Some(dir.join("level.ron"));
        let target = doc.create(CreateKind::Cube);
        doc.rename_entity(target, "Target");
        doc.modify(target, false, |d| {
            if let Some(t) = &mut d.transform {
                t.position = [20.0, 1.0, -5.0];
            }
        });
        doc.add_component(target, ComponentKind::Collider);
        let i = doc.add_entity_named("Shooter");
        let shooter = doc.entities[i].id;
        doc.modify(shooter, false, |d| {
            d.script = Some(automata_core::Script {
                path: "shooter.py".into(),
            });
            d.audio_source = Some(AudioSource::new("sounds/beep.wav"));
        });

        let mut play = PlaySession::start(&doc);
        play.update(TIMESTEP); // colliders exist after the first step
        let mut bridge = ScriptBridge::start(python, &dir.join(".oberon"), &dir).unwrap();
        let mut engine = AudioEngine::new();
        let mut errors = Vec::new();
        let mut most_voices = 0;
        let mut lock = None;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while bridge.answered < 12 && std::time::Instant::now() < deadline {
            for msg in bridge.poll() {
                match msg {
                    BridgeMsg::Frame(w) => {
                        play.apply_frame(&w);
                        lock = lock.or(play.lock_request.take());
                    }
                    BridgeMsg::Error { msg, traceback, .. } => {
                        errors.push(format!("{msg}\n{traceback}"));
                    }
                    _ => {}
                }
            }
            let problems = play.update_audio(&mut engine, &dir);
            assert!(problems.is_empty(), "{problems:?}");
            most_voices = most_voices.max(play.voice_count());
            let mut input = play.frame_input();
            input.mouse_delta = [3.0, 0.0];
            input.mouse_buttons = vec!["left"];
            input.mouse_locked = lock == Some(true);
            input.screen = [800, 600];
            let states = play.script_states(|rel| doc.resolve(rel));
            if bridge.send_frame_with(TIMESTEP, play.elapsed, &[], &states, &input) {
                play.update(TIMESTEP);
            } else {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        drop(bridge.stop());
        let _ = std::fs::remove_dir_all(&dir);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(lock, Some(true), "start() captured the mouse");
        assert_eq!(most_voices, 2, "the source and the one-shot played");
        assert_eq!(play.hud.len(), 1);
        assert!((play.hud[0].x - 400.0).abs() < 1e-3, "{:?}", play.hud[0]);
        let shooter = play
            .doc
            .entities
            .iter()
            .find(|e| e.name() == "Shooter")
            .unwrap();
        let attr = |k: &str| shooter.data.attributes.get(k).cloned();
        assert_eq!(attr("dx"), Some(A::Float(3.0)));
        assert_eq!(attr("click"), Some(A::Bool(true)));
        assert_eq!(attr("locked"), Some(A::Bool(true)));
        // The target was hit at its near face (z = -4.5) until despawned.
        assert!(
            !play.doc.entities.iter().any(|e| e.name() == "Target"),
            "the hit entity was despawned: {:?} {:?}",
            shooter.data.attributes,
            play.frame_input().colliders
        );
        assert_eq!(
            attr("hit"),
            Some(A::Text("none".into())),
            "nothing left to hit"
        );
    }

    /// Scripts in Oberon Play raycast against mesh colliders' real
    /// triangles (sent once over the bridge), at the entity's scale
    /// (skipped without python3).
    #[test]
    fn scripts_raycast_against_mesh_colliders() {
        use crate::bridge::{BridgeMsg, ScriptBridge};
        use crate::doc::ComponentKind;
        use automata_core::AttributeValue as A;
        let python = crate::bridge::default_python();
        if std::process::Command::new(python)
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let ramp = crate::doc::tests::ramp_model("play");
        let dir = std::env::temp_dir().join(format!("oberon_play_mesh_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("probe.py"),
            "def update(entity, ctx):\n\
             \x20   for name, x in (('a', 0.5), ('b', 1.5), ('c', 10.5)):\n\
             \x20       hit = ctx.physics.raycast((x, 10, 1), (0, -1, 0), 50)\n\
             \x20       entity.attributes[name] = hit.point[1] if hit else -1.0\n\
             \x20       entity.attributes[name + 'n'] = hit.normal[1] if hit else 0.0\n",
        )
        .unwrap();
        let mut doc = Document::starter();
        doc.path = Some(dir.join("level.ron"));
        // Two ramps sharing one model and scale: one mesh crosses the bridge.
        for (name, x) in [("Ramp", 0.0), ("Ramp 2", 10.0)] {
            let id = doc.create(CreateKind::Cube);
            doc.rename_entity(id, name);
            assert!(doc.set_model(id, &ramp));
            doc.modify(id, false, |d| {
                if let Some(t) = &mut d.transform {
                    t.position = [x, 0.0, 0.0];
                    t.scale = [2.0; 3];
                }
            });
            assert!(doc.add_component(id, ComponentKind::Collider));
        }
        let i = doc.add_entity_named("Probe");
        let probe = doc.entities[i].id;
        doc.modify(probe, false, |d| {
            d.script = Some(automata_core::Script {
                path: "probe.py".into(),
            });
        });

        let mut play = PlaySession::start(&doc);
        play.update(TIMESTEP);
        let input = play.frame_input();
        assert_eq!(input.meshes.len(), 1, "both ramps share one mesh");
        assert_eq!(input.meshes[0].1.triangles.len(), 8);
        let mut bridge = ScriptBridge::start(python, &dir.join(".oberon"), &dir).unwrap();
        let mut errors = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while bridge.answered < 4 && std::time::Instant::now() < deadline {
            for msg in bridge.poll() {
                match msg {
                    BridgeMsg::Frame(w) => play.apply_frame(&w),
                    BridgeMsg::Error { msg, traceback, .. } => {
                        errors.push(format!("{msg}\n{traceback}"));
                    }
                    _ => {}
                }
            }
            let input = play.frame_input();
            let states = play.script_states(|rel| doc.resolve(rel));
            if bridge.send_frame_with(TIMESTEP, play.elapsed, &[], &states, &input) {
                play.update(TIMESTEP);
            } else {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        drop(bridge.stop());
        let _ = std::fs::remove_dir_all(&dir);
        assert!(errors.is_empty(), "{errors:?}");
        let probe = play
            .doc
            .entities
            .iter()
            .find(|e| e.name() == "Probe")
            .unwrap();
        let float = |k: &str| match probe.data.attributes.get(k) {
            Some(A::Float(v)) => *v,
            other => panic!("{k}: {other:?} in {:?}", probe.data.attributes),
        };
        // The slope is y = x over 0..2 (scale 2): not the box top at 2.
        assert!((float("a") - 0.5).abs() < 1e-3, "{}", float("a"));
        assert!((float("b") - 1.5).abs() < 1e-3, "{}", float("b"));
        assert!(
            (float("c") - 0.5).abs() < 1e-3,
            "second ramp: {}",
            float("c")
        );
        // The slope's normal leans 45° and faces up, toward the ray.
        assert!((float("an") - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-3);
    }

    /// A scene camera under the water muffles the sound; Stop lifts it.
    #[test]
    fn sound_is_muffled_while_the_scene_camera_is_under_water() {
        let mut doc = Document::starter();
        let i = doc.add_entity_named("Sea");
        let sea = doc.entities[i].id;
        doc.modify(sea, false, |d| {
            d.water = Some(automata_renderer::Water {
                size: [20.0, 20.0],
                wave_height: 0.0,
                ..automata_renderer::Water::default()
            });
        });
        let i = doc.add_entity_named("Eye");
        let eye = doc.entities[i].id;
        let mut engine = AudioEngine::new();
        let root = std::env::temp_dir();
        for (y, under) in [(-1.0, true), (2.0, false)] {
            doc.modify(eye, false, |d| {
                d.camera_3d = Some(Camera3D::new(Vec3::new(1.0, y, 1.0), Vec3::ZERO));
            });
            let mut play = PlaySession::start(&doc);
            play.update(TIMESTEP);
            assert_eq!(play.camera_under_water(), under, "camera at y = {y}");
            play.update_audio(&mut engine, &root);
            assert_eq!(engine.is_underwater(), under, "camera at y = {y}");
            play.stop_audio(&mut engine);
            assert!(!engine.is_underwater());
        }
    }

    /// `ctx.settings` volume and graphics reach the session, the master
    /// volume is set once per change, and Stop puts it back.
    #[test]
    fn player_settings_apply_while_playing() {
        use crate::bridge::{FrameReply, PlayerSettings};
        use automata_renderer::GraphicsQuality as Q;
        let doc = Document::starter();
        let mut play = PlaySession::start(&doc);
        let mut engine = AudioEngine::new();
        let root = std::env::temp_dir();
        assert_eq!(play.graphics, None);
        play.apply_frame(&FrameReply {
            settings: PlayerSettings {
                volume: Some(0.3),
                graphics: Some(Q::Low),
            },
            ..FrameReply::default()
        });
        assert_eq!(play.graphics, Some(Q::Low));
        play.update_audio(&mut engine, &root);
        assert_eq!(play.applied_volume, Some(0.3));
        // A reply without settings changes nothing.
        play.apply_frame(&FrameReply::default());
        assert_eq!((play.graphics, play.volume), (Some(Q::Low), Some(0.3)));
        play.stop_audio(&mut engine);
        assert_eq!(play.applied_volume, None);
    }

    /// Water in play mode: a crate floats at the surface, and scripts read
    /// the surface height and splash the water through the real bridge.
    #[test]
    fn crates_float_and_scripts_read_the_water_height() {
        use crate::bridge::{BridgeMsg, ScriptBridge};
        use crate::doc::ComponentKind;
        use automata_core::AttributeValue as A;
        let dir = std::env::temp_dir().join(format!("oberon_play_water_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("probe.py"),
            "def update(entity, ctx):\n\
             \x20   h = ctx.physics.water_height(1.0, 2.0)\n\
             \x20   entity.attributes['in'] = h if h is not None else -99.0\n\
             \x20   entity.attributes['out'] = ctx.physics.water_height(40, 0) is None\n\
             \x20   entity.attributes['same'] = ctx.water.height(1.0, 2.0) == h\n\
             \x20   if not entity.attributes.get('splashed'):\n\
             \x20       ctx.water.splash((-5.0, 0.5, -5.0), 2.0)\n\
             \x20       entity.attributes['splashed'] = True\n",
        )
        .unwrap();
        let mut doc = Document::starter();
        doc.path = Some(dir.join("level.ron"));
        let i = doc.add_entity_named("Sea");
        let sea = doc.entities[i].id;
        doc.modify(sea, false, |d| {
            if let Some(t) = &mut d.transform {
                t.position = [0.0, 0.5, 0.0];
            }
            d.water = Some(automata_renderer::Water {
                size: [20.0, 20.0],
                wave_height: 0.0,
                ..automata_renderer::Water::default()
            });
        });
        let id = doc.create(CreateKind::Cube);
        doc.rename_entity(id, "Crate");
        doc.modify(id, false, |d| {
            if let Some(t) = &mut d.transform {
                t.position = [3.0, 3.0, 3.0];
            }
        });
        assert!(doc.add_component(id, ComponentKind::RigidBody));
        assert!(doc.add_component(id, ComponentKind::Collider));
        let i = doc.add_entity_named("Probe");
        let probe = doc.entities[i].id;
        doc.modify(probe, false, |d| {
            d.script = Some(automata_core::Script {
                path: "probe.py".into(),
            });
        });

        let mut play = PlaySession::start(&doc);
        for _ in 0..600 {
            play.update(TIMESTEP);
        }
        let y = |play: &PlaySession, name: &str| {
            let e = play.doc.entities.iter().find(|e| e.name() == name).unwrap();
            e.data.transform.as_ref().unwrap().position[1]
        };
        // Collider density 1 in water of density 2: half under.
        let crate_y = y(&play, "Crate");
        assert!(
            (crate_y - 0.5).abs() < 0.08,
            "crate should float at the surface, y = {crate_y}"
        );
        assert!(
            play.water_time() > 9.9,
            "the wave clock is the physics clock"
        );
        assert_eq!(play.frame_input().water.len(), 1);

        let python = crate::bridge::default_python();
        if std::process::Command::new(python)
            .arg("--version")
            .output()
            .is_err()
        {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let mut bridge = ScriptBridge::start(python, &dir.join(".oberon"), &dir).unwrap();
        let mut errors = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while bridge.answered < 3 && std::time::Instant::now() < deadline {
            for msg in bridge.poll() {
                match msg {
                    BridgeMsg::Frame(w) => play.apply_frame(&w),
                    BridgeMsg::Error { msg, traceback, .. } => {
                        errors.push(format!("{msg}\n{traceback}"));
                    }
                    _ => {}
                }
            }
            let input = play.frame_input();
            let states = play.script_states(|rel| doc.resolve(rel));
            if bridge.send_frame_with(TIMESTEP, play.elapsed, &[], &states, &input) {
                play.update(TIMESTEP);
            } else {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        drop(bridge.stop());
        let _ = std::fs::remove_dir_all(&dir);
        assert!(errors.is_empty(), "{errors:?}");
        let probe = play
            .doc
            .entities
            .iter()
            .find(|e| e.name() == "Probe")
            .unwrap();
        match probe.data.attributes.get("in") {
            Some(A::Float(h)) => assert!((h - 0.5).abs() < 1e-4, "water height {h}"),
            other => panic!("in: {other:?}"),
        }
        assert_eq!(probe.data.attributes.get("out"), Some(&A::Bool(true)));
        assert_eq!(probe.data.attributes.get("same"), Some(&A::Bool(true)));
        // The splash dug into the water where the script asked.
        let dip = (-8..=8)
            .flat_map(|i| (-8..=8).map(move |j| (i, j)))
            .map(|(i, j)| {
                let (dx, dz) = (i as f32 * 0.25, j as f32 * 0.25);
                play.physics.ripple_height(-5.0 + dx, -5.0 + dz).abs()
            })
            .fold(0.0, f32::max);
        assert!(dip > 0.02, "splash ripples {dip}");
        assert!(play.ripples().is_some());
    }

    /// The shipped FPS example, played through the real bridge: the player
    /// captures the mouse, turns toward a drone with the mouse, holds the
    /// trigger and hits it; the HUD, the held blaster and the blaster's
    /// sound all come through (skipped without python3).
    #[test]
    #[allow(clippy::too_many_lines)]
    fn the_fps_example_plays_in_oberon() {
        use crate::bridge::{BridgeMsg, HudKind, ScriptBridge};
        use automata_core::AttributeValue as A;
        let python = crate::bridge::default_python();
        if std::process::Command::new(python)
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let game =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/oberon/fps");
        let doc = Document::load(&game.join("level.ron")).unwrap();
        let state = std::env::temp_dir().join(format!("oberon_fps_example_{}", std::process::id()));
        let mut play = PlaySession::start(&doc);
        play.update(TIMESTEP);
        let mut bridge = ScriptBridge::start(python, &state, &game).unwrap();
        let mut engine = AudioEngine::new();
        let mut errors = Vec::new();
        let mut lock = None;
        let mut most_voices = 0;
        let mut drone_health = 100;
        let mut most_sparks = 0;
        let mut frames = 0;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while bridge.answered < 40 && std::time::Instant::now() < deadline {
            for msg in bridge.poll() {
                match msg {
                    BridgeMsg::Frame(w) => {
                        play.apply_frame(&w);
                        lock = lock.or(play.lock_request.take());
                    }
                    BridgeMsg::Error { msg, traceback, .. } => {
                        errors.push(format!("{msg}\n{traceback}"));
                    }
                    _ => {}
                }
            }
            let problems = play.update_audio(&mut engine, &game);
            assert!(problems.is_empty(), "{problems:?}");
            most_voices = most_voices.max(play.voice_count());
            most_sparks = most_sparks.max(play.particle_count());
            if let Some(e) = play.doc.entities.iter().find(|e| e.name() == "Enemy 1") {
                if let Some(A::Int(h)) = e.data.attributes.get("health") {
                    drone_health = drone_health.min(*h);
                }
            } else {
                drone_health = drone_health.min(0);
            }
            let mut input = play.frame_input();
            input.screen = [1280, 720];
            input.mouse_locked = lock == Some(true);
            // One mouse move turns toward Enemy 1 at (-3.5, 2.5, -6): yaw
            // atan2(3.5, 6) left, pitch about 0.2 up (700 px per radian).
            if frames == 0 {
                input.mouse_delta = [-0.528 * 700.0, -0.2 * 700.0];
            }
            if frames >= 2 {
                input.mouse_buttons = vec!["left"];
            }
            let states = play.script_states(|rel| doc.resolve(rel));
            if bridge.send_frame_with(TIMESTEP, play.elapsed, &[], &states, &input) {
                play.update(TIMESTEP);
                frames += 1;
            } else {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        drop(bridge.stop());
        let _ = std::fs::remove_dir_all(&state);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(lock, Some(true), "the player captures the mouse");
        assert!(most_voices >= 1, "the blaster sounded");
        assert!(drone_health < 100, "a volley hit Enemy 1");
        let chasing = play
            .scene
            .world()
            .query::<&automata_nav::NavAgent>()
            .iter()
            .any(|a| a.target.is_some());
        assert!(chasing || drone_health <= 0, "the shot drone gives chase");
        assert!(most_sparks >= 10, "hits throw sparks: {most_sparks}");
        let images: Vec<&str> = play
            .hud
            .iter()
            .filter_map(|h| match &h.kind {
                HudKind::Image(p) => Some(p.as_str()),
                _ => None,
            })
            .collect();
        assert!(images.contains(&"sprites/crosshair.png"), "{images:?}");
        let labels: Vec<&str> = play
            .hud
            .iter()
            .filter_map(|h| match &h.kind {
                HudKind::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            labels.iter().any(|t| t.ends_with('%')),
            "the health label: {labels:?}"
        );
        let pips = play
            .hud
            .iter()
            .filter(|h| h.kind == HudKind::Rect && h.size == Some([14.0, 14.0]))
            .count();
        assert!((3..=4).contains(&pips), "a pip per drone: {pips}");
        let find = |name: &str| {
            play.doc
                .entities
                .iter()
                .find(|e| e.name() == name)
                .and_then(|e| e.data.transform)
                .map(|t| glam::Vec3::from(t.position))
                .unwrap()
        };
        let player = find("Player");
        assert!((0.5..1.3).contains(&player.y), "on the platform: {player}");
        assert!(
            find("Blaster").distance(player) < 1.5,
            "the blaster is held: {}",
            find("Blaster")
        );
    }

    /// The whole loop with a real interpreter: Oberon state → Python script
    /// → writes applied to the simulation (skipped without python3).
    #[test]
    fn a_python_script_drives_the_player_end_to_end() {
        use crate::bridge::{BridgeMsg, ScriptBridge};
        let python = crate::bridge::default_python();
        if std::process::Command::new(python)
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let dir = std::env::temp_dir().join(format!("oberon_play_bridge_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("player.py");
        std::fs::write(&script, crate::project::script_template(&script)).unwrap();

        let mut doc = Document::starter();
        doc.path = Some(dir.join("level.ron"));
        let idx = player_index(&doc);
        doc.entities[idx].data.script = Some(automata_core::Script {
            path: "player.py".into(),
        });
        let start = doc.entities[idx].data.transform.unwrap().position;
        let mut play = PlaySession::start(&doc);
        let mut bridge = ScriptBridge::start(python, &dir.join(".oberon"), &dir).unwrap();
        let mut errors = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while bridge.answered < 40 && std::time::Instant::now() < deadline {
            for msg in bridge.poll() {
                match msg {
                    BridgeMsg::Frame(w) => play.apply_frame(&w),
                    BridgeMsg::Error { msg, .. } => errors.push(msg),
                    _ => {}
                }
            }
            let states = play.script_states(|rel| doc.resolve(rel));
            // Hold "d": the template walks right (+X).
            bridge.send_frame(TIMESTEP, play.elapsed, &["d"], &states);
            play.update(TIMESTEP);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        drop(bridge.stop());
        let _ = std::fs::remove_dir_all(&dir);
        assert!(errors.is_empty(), "{errors:?}");
        let end = play.doc.entities[idx].data.transform.unwrap().position;
        assert!(
            end[0] > start[0] + 1.0,
            "the script walked the player: {start:?} -> {end:?}"
        );
        assert!(
            end[1] > 0.5 && end[1] < 1.5,
            "and kept it on the ground: {end:?}"
        );
    }
}
