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

use automata_core::{Attributes, TransformData};
use automata_physics::{
    BodyType, CharacterController, CharacterVelocity, PhysicsWorld3D, RigidBody,
};
use automata_renderer::Camera3D;
use automata_scene::{Entity, Name, Scene, Transform};
use glam::Vec3;

use crate::bridge::{EntityState, EntityWrite};
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
}

impl PlaySession {
    /// Starts a session from the edited document.
    #[must_use]
    pub fn start(doc: &Document) -> Self {
        let display = doc.play_copy();
        let mut scene = Scene::new();
        display.to_scene_file().apply_to_world(scene.world_mut());

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
        }
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
        for w in writes {
            let Some(&(i, entity)) = self
                .map
                .iter()
                .find(|(i, _)| self.doc.entities.get(*i).is_some_and(|e| e.id == w.id))
            else {
                continue;
            };
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
                    BridgeMsg::Frame(w) => play.apply_script_writes(&w),
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
                    BridgeMsg::Frame(w) => play.apply_script_writes(&w),
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
