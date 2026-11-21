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
    AudioWrite, ColliderShape, EntityState, EntityWrite, FrameInput, FrameReply, HudItem, MeshTris,
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
}

/// A running simulation of a document.
pub struct PlaySession {
    /// The document as simulated (shown by the UI while playing).
    pub doc: Document,
    scene: Scene,
    physics: PhysicsWorld3D,
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
}

impl PlaySession {
    /// Starts a session from the edited document.
    #[must_use]
    pub fn start(doc: &Document) -> Self {
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
        }
        file.apply_to_world(scene.world_mut());

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

        let characters = scene
            .world()
            .query::<(Entity, &CharacterController)>()
            .iter()
            .map(|(entity, _)| Character {
                entity,
                vertical: 0.0,
            })
            .collect();

        let mut physics = PhysicsWorld3D::new();
        physics.sync_from_scene(scene.world_mut());
        Self {
            doc: display,
            scene,
            physics,
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
        }
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
                                let tris = MeshTris {
                                    vertices: vertices.iter().map(glam::Vec3::to_array).collect(),
                                    triangles,
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
        FrameInput {
            colliders,
            meshes,
            playing,
            ..FrameInput::default()
        }
    }

    /// Applies a whole runner reply: entity writes, sounds, HUD, cursor.
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
        self.hud.clone_from(&reply.hud);
        if reply.lock.is_some() {
            self.lock_request = reply.lock;
        }
    }

    /// Runs the scene's audio for this frame: `play_on_start` sources,
    /// script requests and distance fading. `root` is the scene folder.
    /// Returns problems for the Console (missing clips, once each).
    pub fn update_audio(
        &mut self,
        engine: &mut AudioEngine,
        root: &std::path::Path,
    ) -> Vec<String> {
        if self.paused {
            return Vec::new();
        }
        let commands = std::mem::take(&mut self.audio_commands);
        self.sources
            .update(self.scene.world(), engine, Some(root), commands)
    }

    /// Stops every sound this session started (Stop / pause).
    pub fn stop_audio(&mut self, engine: &mut AudioEngine) {
        self.audio_commands.clear();
        self.sources.stop_all(engine);
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
                    grounded: character && self.physics.grounded(entity),
                    attributes: entry.data.attributes.clone(),
                    camera: world.get::<&Camera3D>(entity).ok().map(|c| {
                        let (p, t) = (c.position, c.target);
                        [p.x, p.y, p.z, t.x, t.y, t.z, c.fov_y_degrees]
                    }),
                    has_audio: world.get::<&AudioSource>(entity).is_ok(),
                })
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

    /// The scene's camera (the first entity with one), as the simulation
    /// currently has it (scripts may move it). Play mode renders through it.
    #[must_use]
    pub fn scene_camera(&self) -> Option<Camera3D> {
        self.scene
            .world()
            .query::<&Camera3D>()
            .iter()
            .next()
            .copied()
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
        }
    }

    /// Runs exactly one physics step (used by tests and frame stepping).
    pub fn step_once(&mut self) {
        let world = self.scene.world_mut();
        self.physics.sync_from_scene(world);
        self.drive_characters();
        let world = self.scene.world_mut();
        self.physics.move_characters(world, TIMESTEP);
        let _tick = self.physics.step(world, TIMESTEP);
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
            let grounded = self.physics.grounded(c.entity);
            c.vertical = if grounded && input.jump {
                JUMP_SPEED
            } else if grounded && c.vertical <= 0.0 {
                -2.0 // keep in contact with the ground
            } else {
                (c.vertical - CHARACTER_GRAVITY * TIMESTEP).max(-50.0)
            };
            let velocity = CharacterVelocity::new(wish * speed + Vec3::Y * c.vertical);
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
mod tests {
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
    fn pausing_freezes_the_simulation() {
        let mut doc = Document::starter();
        doc.create(CreateKind::PhysicsCube);
        let mut play = PlaySession::start(&doc);
        play.paused = true;
        play.update(1.0);
        assert_eq!(play.steps, 0);
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
