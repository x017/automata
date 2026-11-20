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

use std::collections::HashMap;

use automata_core::TransformData;
use automata_physics::{
    BodyType, CharacterController, CharacterVelocity, PhysicsWorld3D, RigidBody,
};
use automata_scene::{Entity, Name, Scene, Transform};
use glam::Vec3;

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
        }
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
}
