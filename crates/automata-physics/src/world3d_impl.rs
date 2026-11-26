//! 3D physics world implementation (`Rapier3D`).

use std::collections::HashMap;

use automata_scene::{CharacterMotion, Entity, SceneWorld, Transform};
use glam::{Quat, Vec3};
use r3::control::{CharacterAutostep, CharacterLength, KinematicCharacterController};
use r3::dynamics::{
    CCDSolver, ImpulseJointSet, IntegrationParameters, IslandManager, MultibodyJointSet,
    RigidBodyBuilder, RigidBodySet,
};
use r3::geometry::{
    BroadPhaseBvh, ColliderBuilder, ColliderHandle, ColliderSet, CollisionEvent, NarrowPhase,
};
use r3::math::{Pose, Vector};
use r3::pipeline::{
    ActiveEvents, ChannelEventCollector, PhysicsPipeline, QueryFilter, QueryFilterFlags,
};
use rapier3d as r3;
use tracing::trace;

#[path = "water.rs"]
mod water;

#[path = "joints3d.rs"]
mod joints;

#[path = "terrain3d.rs"]
mod terrain;

#[path = "ragdoll3d.rs"]
mod ragdoll;

use crate::components::{
    BodyType, CharacterController, CharacterVelocity, Collider3D, RigidBody, Shape3D,
};
use crate::events::{ContactForce, PhysicsEvent};
use crate::model_shape::build_model_shape;
use crate::world::{
    entity_to_user_data, user_data_to_entity, CharacterMove, PhysicsTick, RayFilter, RayHit,
};

/// The 3D physics world. Owns the Rapier sets; you own the ECS scene.
///
/// The world is deliberately *not* exposed raw to game scripts: scripts
/// interact with it through context methods (`ctx.physics.cast_ray(...)`,
/// `ctx.physics.apply_impulse(...)`) instead.
pub struct PhysicsWorld3D {
    gravity: Vec3,
    pipeline: PhysicsPipeline,
    islands: IslandManager,
    broad_phase: BroadPhaseBvh,
    narrow_phase: NarrowPhase,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    ccd: CCDSolver,
    soft_bodies: r3::dynamics::SoftBodySet,
    body_entities: HashMap<r3::dynamics::RigidBodyHandle, Entity>,
    entity_bodies: HashMap<Entity, r3::dynamics::RigidBodyHandle>,
    entity_colliders: HashMap<Entity, ColliderHandle>,
    /// Entities with a collider but no [`RigidBody`]: simulated as fixed
    /// bodies that follow their [`Transform`] (scripts may move them).
    static_colliders: HashMap<Entity, Transform>,
    controllers: HashMap<Entity, KinematicCharacterController>,
    grounded: HashMap<Entity, bool>,
    last_dt: f32,
    /// Built mesh / convex-hull shapes, shared by every collider that uses
    /// the same model at the same scale.
    model_shapes: HashMap<ModelShapeKey, r3::geometry::SharedShape>,
    /// Simulated time (sum of steps): the clock the water's waves run on.
    sim_time: f64,
    /// The scene's water surfaces at `sim_time` (see `water.rs`).
    water: Vec<automata_renderer::WaterSurface>,
    /// Ripples around the camera (see `ripples.rs`).
    ripples: crate::ripples::RippleField,
    /// Each body's dent in the ripples, as last dug.
    dents: HashMap<r3::dynamics::RigidBodyHandle, crate::ripples::Dent>,
    /// Each water surface's wind: strength 0..1 and the direction it
    /// blows (radians around +Y, as `Water::wave_direction`).
    winds: Vec<(f32, f32)>,
    /// Bodies that hit the water this step (see `water.rs`).
    splashes: Vec<crate::world::WaterSplash>,
    /// Crest spray owed so far (fractional bursts) and its random state.
    spray_carry: f32,
    spray_rng: u64,
    /// Splash and spray particles on (the default).
    water_particles: bool,
    /// Each character's last move straight down (m/s): characters are
    /// teleported, so their bodies carry no velocity for the water.
    character_sinking: HashMap<r3::dynamics::RigidBodyHandle, f32>,
    /// Every entity's [`crate::Joint`] as built (see `joints3d.rs`).
    joints: HashMap<Entity, joints::JointSlot>,
    /// The fixed body joints to the world hang from.
    ground: Option<r3::dynamics::RigidBodyHandle>,
    /// Joints already warned about (no body, no such partner).
    joint_warned: std::collections::HashSet<Entity>,
    /// Each colliding terrain's heightfield body.
    terrains: HashMap<Entity, terrain::TerrainBody>,
    /// Each limp entity's ragdoll bodies (see `ragdoll3d.rs`).
    ragdolls: HashMap<Entity, ragdoll::RagdollBodies>,
    /// Ragdoll layouts by model (`None`: the model can't have one).
    ragdoll_layouts: HashMap<usize, Option<std::sync::Arc<automata_scene::RagdollLayout>>>,
}

/// What a built model shape depends on: the loaded model (by identity, so
/// a reloaded file builds afresh), hull or triangles, and the scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ModelShapeKey {
    model: usize,
    hull: bool,
    scale: [u32; 3],
}

impl std::fmt::Debug for PhysicsWorld3D {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhysicsWorld3D")
            .field("bodies", &self.entity_bodies.len())
            .field("colliders", &self.entity_colliders.len())
            .field("controllers", &self.controllers.len())
            .finish_non_exhaustive()
    }
}

impl Default for PhysicsWorld3D {
    fn default() -> Self {
        Self::new()
    }
}

/// Gives every character a [`CharacterMotion`], so root motion reaches
/// its controller (and collides) instead of moving its transform.
fn mark_characters(scene: &mut SceneWorld) {
    let unmarked: Vec<Entity> = scene
        .query::<hecs::Without<(hecs::Entity, &CharacterController), &CharacterMotion>>()
        .iter()
        .map(|(e, _)| e)
        .collect();
    for entity in unmarked {
        let _ = scene.insert_one(entity, CharacterMotion::default());
    }
}

impl PhysicsWorld3D {
    /// An empty 3D world with Earth-like gravity along -Y (`-9.81` m/s²).
    #[must_use]
    pub fn new() -> Self {
        Self::with_gravity(Vec3::new(0.0, -9.81, 0.0))
    }

    /// An empty 3D world with the given gravity (in m/s²).
    #[must_use]
    pub fn with_gravity(gravity: Vec3) -> Self {
        Self {
            gravity,
            pipeline: PhysicsPipeline::new(),
            islands: IslandManager::new(),
            broad_phase: BroadPhaseBvh::new(),
            narrow_phase: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd: CCDSolver::new(),
            soft_bodies: r3::dynamics::SoftBodySet::new(),
            body_entities: HashMap::new(),
            entity_bodies: HashMap::new(),
            entity_colliders: HashMap::new(),
            static_colliders: HashMap::new(),
            controllers: HashMap::new(),
            grounded: HashMap::new(),
            last_dt: 1.0 / 60.0,
            model_shapes: HashMap::new(),
            sim_time: 0.0,
            water: Vec::new(),
            ripples: crate::ripples::RippleField::new(),
            dents: HashMap::new(),
            winds: Vec::new(),
            splashes: Vec::new(),
            spray_carry: 0.0,
            spray_rng: 0x9e37_79b9_7f4a_7c15,
            water_particles: true,
            character_sinking: HashMap::new(),
            joints: HashMap::new(),
            ground: None,
            joint_warned: std::collections::HashSet::new(),
            terrains: HashMap::new(),
            ragdolls: HashMap::new(),
            ragdoll_layouts: HashMap::new(),
        }
    }

    /// The current gravity vector.
    #[must_use]
    pub fn gravity(&self) -> Vec3 {
        self.gravity
    }

    /// Change the gravity for subsequent steps.
    pub fn set_gravity(&mut self, gravity: Vec3) {
        self.gravity = gravity;
    }

    /// Number of rigid bodies currently tracked.
    #[must_use]
    pub fn body_count(&self) -> usize {
        self.entity_bodies.len()
    }

    /// Number of colliders currently tracked.
    #[must_use]
    pub fn collider_count(&self) -> usize {
        self.entity_colliders.len()
    }

    fn entity_for_collider(&self, handle: ColliderHandle) -> Option<Entity> {
        let collider = self.colliders.get(handle)?;
        user_data_to_entity(collider.user_data)
    }

    /// Synchronise the Rapier world with the ECS scene: spawn bodies for new
    /// [`RigidBody`] entities, update kinematic targets, remove despawned
    /// entities. Called each fixed tick before [`step`](Self::step).
    ///
    /// An entity with a [`Collider3D`] but no [`RigidBody`] is static
    /// scenery (walls, floors, props): it blocks bodies and characters and
    /// is hit by ray casts, and it follows its [`Transform`] when a script
    /// moves it.
    pub fn sync_from_scene(&mut self, scene: &mut SceneWorld) {
        self.spawn_missing(scene);
        self.update_existing(scene);
        self.despawn_removed(scene);
        self.sync_terrains(scene);
        mark_characters(scene);
        self.sync_joints(scene);
        self.sync_ragdolls(scene);
    }

    fn spawn_missing(&mut self, scene: &mut SceneWorld) {
        let mut to_spawn = Vec::new();
        let mut bodies_query = scene.query::<(hecs::Entity, &RigidBody, Option<&Transform>)>();
        for (entity, body, transform) in &mut bodies_query {
            // 3D owns entities with a 3D collider or none; a 2D collider
            // routes the entity to the 2D world instead.
            if !body.enabled
                || self.entity_bodies.contains_key(&entity)
                || scene.get::<&crate::Collider2D>(entity).is_ok()
            {
                continue;
            }
            let collider = scene.get::<&Collider3D>(entity).ok().map(|c| (*c).clone());
            to_spawn.push((entity, body.clone(), transform.copied(), collider));
        }

        let mut statics = Vec::new();
        let mut static_query = scene
            .query::<hecs::Without<(hecs::Entity, &Collider3D, Option<&Transform>), &RigidBody>>();
        for (entity, collider, transform) in &mut static_query {
            if !self.entity_bodies.contains_key(&entity) {
                statics.push((entity, collider.clone(), transform.copied()));
            }
        }
        drop(static_query);
        drop(bodies_query);
        for (entity, collider, transform) in statics {
            let t = transform.unwrap_or_default();
            let builder = RigidBodyBuilder::fixed()
                .translation(Vector::new(t.position.x, t.position.y, t.position.z))
                .rotation(t.rotation.to_scaled_axis());
            let handle = self.bodies.insert(builder.build());
            self.entity_bodies.insert(entity, handle);
            self.body_entities.insert(handle, entity);
            self.static_colliders.insert(entity, t);
            self.attach_collider(entity, handle, &collider, t.scale, false);
            trace!(?entity, "physics: spawned static collider");
        }

        for (entity, body, transform, collider) in to_spawn {
            let mut builder = match body.body_type {
                BodyType::Dynamic => RigidBodyBuilder::dynamic(),
                BodyType::Static => RigidBodyBuilder::fixed(),
                BodyType::KinematicVelocity => RigidBodyBuilder::kinematic_velocity_based(),
                BodyType::KinematicPosition => RigidBodyBuilder::kinematic_position_based(),
            }
            .gravity_scale(body.gravity_scale)
            .linear_damping(body.linear_damping)
            .angular_damping(body.angular_damping);

            if let Some(t) = transform {
                // The builder takes a scaled axis (axis × angle), not the
                // quaternion's vector part.
                builder = builder
                    .translation(Vector::new(t.position.x, t.position.y, t.position.z))
                    .rotation(t.rotation.to_scaled_axis());
            }

            let handle = self.bodies.insert(builder.build());
            self.entity_bodies.insert(entity, handle);
            self.body_entities.insert(handle, entity);
            trace!(?entity, "physics: spawned 3D body");

            if let Some(c) = collider {
                let scale = transform.map_or(Vec3::ONE, |t| t.scale);
                let moving = body.body_type == BodyType::Dynamic;
                self.attach_collider(entity, handle, &c, scale, moving);
            }
        }
    }

    /// Builds (or reuses) the shape for a mesh or convex-hull collider.
    /// `moving` (a dynamic body) turns a triangle mesh into its hull: a
    /// triangle soup has no volume, so the body would get no mass or
    /// inertia. Kinematic bodies keep their triangles (moving platforms).
    fn model_shape(
        &mut self,
        shape: &Shape3D,
        scale: Vec3,
        moving: bool,
    ) -> Option<r3::geometry::SharedShape> {
        let (path, hull) = match shape {
            Shape3D::Mesh { model } => (model, moving),
            Shape3D::ConvexHull { model } => (model, true),
            _ => return None,
        };
        if moving && matches!(shape, Shape3D::Mesh { .. }) {
            tracing::warn!(
                "mesh collider '{path}' is on a dynamic body; using its convex hull \
                 (mesh colliders are for static and kinematic bodies)"
            );
        }
        let model = match automata_assets::load(path) {
            Ok(model) => model,
            Err(e) => {
                tracing::warn!("collider: {e}");
                return None;
            }
        };
        let key = ModelShapeKey {
            model: std::sync::Arc::as_ptr(&model) as usize,
            hull,
            scale: scale.to_array().map(f32::to_bits),
        };
        if let Some(shape) = self.model_shapes.get(&key) {
            return Some(shape.clone());
        }
        let built = build_model_shape(&model, scale, hull);
        if built.is_none() {
            tracing::warn!("collider: '{path}' has no usable triangles");
        }
        let built = built?;
        self.model_shapes.insert(key, built.clone());
        Some(built)
    }

    fn attach_collider(
        &mut self,
        entity: Entity,
        body: r3::dynamics::RigidBodyHandle,
        collider: &Collider3D,
        scale: Vec3,
        moving: bool,
    ) {
        let builder = match &collider.shape {
            Shape3D::Ball { radius } => ColliderBuilder::ball(*radius),
            Shape3D::Cuboid { extents } => {
                ColliderBuilder::cuboid(extents.x / 2.0, extents.y / 2.0, extents.z / 2.0)
            }
            Shape3D::Capsule {
                half_height,
                radius,
            } => ColliderBuilder::capsule_y(*half_height, *radius),
            shape @ (Shape3D::Mesh { .. } | Shape3D::ConvexHull { .. }) => {
                let Some(built) = self.model_shape(shape, scale, moving) else {
                    return;
                };
                ColliderBuilder::new(built)
            }
        };
        let mut builder = builder
            .density(collider.density)
            .friction(collider.friction)
            .restitution(collider.restitution)
            .sensor(collider.sensor)
            .translation(Vector::new(
                collider.offset[0],
                collider.offset[1],
                collider.offset[2],
            ))
            .active_events(ActiveEvents::COLLISION_EVENTS)
            .user_data(entity_to_user_data(entity));

        if !collider.enabled {
            builder = builder.enabled(false);
        }
        let handle = self
            .colliders
            .insert_with_parent(builder.build(), body, &mut self.bodies);
        self.entity_colliders.insert(entity, handle);
    }

    fn update_existing(&mut self, scene: &mut SceneWorld) {
        // Static colliders follow their transform when it changes.
        let mut moved = Vec::new();
        for (entity, last) in &self.static_colliders {
            if let Ok(t) = scene.get::<&Transform>(*entity) {
                if t.position != last.position || t.rotation != last.rotation {
                    moved.push((*entity, *t));
                }
            }
        }
        for (entity, t) in moved {
            self.static_colliders.insert(entity, t);
            if let Some(rb) = self
                .entity_bodies
                .get(&entity)
                .and_then(|h| self.bodies.get_mut(*h))
            {
                rb.set_position(
                    Pose::from_parts(
                        Vector::new(t.position.x, t.position.y, t.position.z),
                        t.rotation,
                    ),
                    true,
                );
            }
        }
        let mut updates = Vec::new();
        let mut bodies_query = scene.query::<(hecs::Entity, &RigidBody, Option<&Transform>)>();
        for (entity, body, transform) in &mut bodies_query {
            if !body.enabled {
                continue;
            }
            let Some(&handle) = self.entity_bodies.get(&entity) else {
                continue;
            };
            updates.push((body.clone(), transform.copied(), handle));
        }

        for (body, transform, handle) in updates {
            let Some(rb) = self.bodies.get_mut(handle) else {
                continue;
            };
            rb.set_gravity_scale(body.gravity_scale, true);
            rb.set_linear_damping(body.linear_damping);
            rb.set_angular_damping(body.angular_damping);
            if let (BodyType::KinematicPosition, Some(t)) = (body.body_type, transform) {
                rb.set_next_kinematic_translation(Vector::new(
                    t.position.x,
                    t.position.y,
                    t.position.z,
                ));
                rb.set_next_kinematic_rotation(Quat::from_xyzw(
                    t.rotation.x,
                    t.rotation.y,
                    t.rotation.z,
                    t.rotation.w,
                ));
            }
        }
    }

    fn despawn_removed(&mut self, scene: &mut SceneWorld) {
        let mut to_remove = Vec::new();
        for entity in self.entity_bodies.keys() {
            let has_body = scene.get::<&RigidBody>(*entity).is_ok();
            let keep = if self.static_colliders.contains_key(entity) {
                // Gone, lost its collider, or became a real body (respawned
                // as one next sync).
                !has_body && scene.get::<&Collider3D>(*entity).is_ok()
            } else {
                has_body
            };
            if !keep {
                to_remove.push(*entity);
            }
        }
        for entity in to_remove {
            self.remove_entity(entity);
        }
    }

    fn remove_entity(&mut self, entity: Entity) {
        if let Some(body) = self.entity_bodies.remove(&entity) {
            self.body_entities.remove(&body);
            self.bodies.remove(
                body,
                &mut self.islands,
                &mut self.colliders,
                &mut self.impulse_joints,
                &mut self.multibody_joints,
                &mut self.soft_bodies,
                true,
            );
        }
        self.entity_colliders.remove(&entity);
        self.static_colliders.remove(&entity);
        self.controllers.remove(&entity);
        self.grounded.remove(&entity);
        trace!(?entity, "physics: removed 3D body");
    }

    /// Move all character controllers by their [`CharacterVelocity`] * `dt`,
    /// resolving collisions. Call after [`sync_from_scene`](Self::sync_from_scene)
    /// and before [`step`](Self::step).
    pub fn move_characters(&mut self, scene: &mut SceneWorld, dt: f32) {
        // Sync controller configs.
        let mut configs = Vec::new();
        for (entity, config) in &mut scene.query::<(hecs::Entity, &CharacterController)>() {
            configs.push((entity, config.clone()));
        }
        for (entity, config) in configs {
            self.controllers
                .entry(entity)
                .or_insert_with(|| controller_from_config(&config));
        }

        // Velocity for this step, plus any root motion from the animator.
        mark_characters(scene);
        let mut moves = Vec::new();
        for (entity, velocity, motion) in &mut scene.query::<(
            hecs::Entity,
            Option<&CharacterVelocity>,
            Option<&mut CharacterMotion>,
        )>() {
            if !self.controllers.contains_key(&entity) {
                continue;
            }
            let root = motion.map_or(Vec3::ZERO, CharacterMotion::take);
            // A limp character lies where its ragdoll fell.
            if self.ragdolls.contains_key(&entity) {
                continue;
            }
            if velocity.is_some() || root != Vec3::ZERO {
                moves.push((
                    entity,
                    velocity.map_or(Vec3::ZERO, |v| v.velocity) * dt + root,
                ));
            }
        }
        for (entity, desired) in moves {
            let mv = self.move_character(scene, entity, desired);
            self.grounded.insert(entity, mv.grounded);
        }
    }

    /// Move a character controller entity by `desired` displacement,
    /// resolving collisions. Returns the actual displacement applied.
    pub fn move_character(
        &mut self,
        scene: &mut SceneWorld,
        entity: Entity,
        desired: Vec3,
    ) -> CharacterMove {
        let Some(controller) = self.controllers.get(&entity).copied() else {
            return CharacterMove::default();
        };
        let Some(&body_handle) = self.entity_bodies.get(&entity) else {
            return CharacterMove::default();
        };
        let Some(&collider_handle) = self.entity_colliders.get(&entity) else {
            return CharacterMove::default();
        };
        let Some(collider) = self.colliders.get(collider_handle) else {
            return CharacterMove::default();
        };
        let character_shape = collider.shared_shape().clone();
        let character_pos = *collider.position();

        let query = self.broad_phase.as_query_pipeline(
            self.narrow_phase.query_dispatcher(),
            &self.bodies,
            &self.colliders,
            QueryFilter::default().exclude_rigid_body(body_handle),
        );
        let mut collisions = Vec::new();
        let movement = controller.move_shape(
            self.integration_dt(),
            &query,
            &*character_shape,
            &character_pos,
            Vector::new(desired.x, desired.y, desired.z),
            |collision| collisions.push(collision),
        );

        let actual = Vec3::new(
            movement.translation.x,
            movement.translation.y,
            movement.translation.z,
        );

        if let Ok(mut transform) = scene.get::<&mut Transform>(entity) {
            transform.position += actual;
        }
        self.character_sinking
            .insert(body_handle, -actual.y / self.integration_dt().max(1e-6));
        if let Some(body) = self.bodies.get_mut(body_handle) {
            // Teleport immediately: `set_next_kinematic_translation` only
            // affects KinematicPositionBased bodies, but characters may be
            // KinematicVelocityBased — those must follow the resolved move
            // too, or the collider stays behind while the scene transform
            // walks on.
            let pos = body.translation();
            body.set_translation(
                Vector::new(pos.x + actual.x, pos.y + actual.y, pos.z + actual.z),
                false,
            );
        }

        CharacterMove {
            translation: actual,
            grounded: movement.grounded,
            sliding: movement.is_sliding_down_slope,
            collisions: collisions
                .iter()
                .filter_map(|c| self.entity_for_collider(c.handle))
                .collect(),
        }
    }

    fn integration_dt(&self) -> f32 {
        // move_character runs between sync and step; use the last dt the
        // world was stepped with (default 1/60 before the first step).
        self.last_dt
    }

    /// Step the simulation by `dt` seconds and write transforms back.
    pub fn step(&mut self, scene: &mut SceneWorld, dt: f32) -> PhysicsTick {
        self.last_dt = dt;
        let params = IntegrationParameters {
            dt,
            ..IntegrationParameters::default()
        };

        let (collision_send, collision_recv) = std::sync::mpsc::channel();
        let (force_send, force_recv) = std::sync::mpsc::channel();
        let (tear_send, _tear_recv) = std::sync::mpsc::channel();
        let events = ChannelEventCollector::new(collision_send, force_send, tear_send);

        // Water: buoyancy and drag as impulses, before integration.
        self.collect_water(scene);
        self.apply_water(dt);
        self.step_ripples(dt);
        self.drive_ragdolls(dt);
        let splashes = self.water_effects(scene, dt);
        self.sim_time += f64::from(dt);

        self.pipeline.step(
            Vector::new(self.gravity.x, self.gravity.y, self.gravity.z),
            &params,
            &mut self.islands,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.soft_bodies,
            &mut self.ccd,
            &(),
            &events,
        );

        let mut tick = PhysicsTick {
            splashes,
            ..PhysicsTick::default()
        };
        while let Ok(event) = collision_recv.try_recv() {
            let mapped = match event {
                CollisionEvent::Started(a, b, _) => {
                    match (self.entity_for_collider(a), self.entity_for_collider(b)) {
                        (Some(entity_a), Some(entity_b)) => {
                            Some(PhysicsEvent::CollisionStarted { entity_a, entity_b })
                        }
                        _ => None,
                    }
                }
                CollisionEvent::Stopped(a, b, _) => {
                    match (self.entity_for_collider(a), self.entity_for_collider(b)) {
                        (Some(entity_a), Some(entity_b)) => {
                            Some(PhysicsEvent::CollisionStopped { entity_a, entity_b })
                        }
                        _ => None,
                    }
                }
            };
            if let Some(mapped) = mapped {
                tick.collisions.push(mapped);
            }
        }
        while let Ok(event) = force_recv.try_recv() {
            if let (Some(entity_a), Some(entity_b)) = (
                self.entity_for_collider(event.collider1),
                self.entity_for_collider(event.collider2),
            ) {
                tick.contact_forces.push(ContactForce {
                    entity_a,
                    entity_b,
                    total_force: Vec3::new(
                        event.total_force.x,
                        event.total_force.y,
                        event.total_force.z,
                    ),
                    max_force_magnitude: event.max_force_magnitude,
                });
            }
        }

        tick.broken_joints = self.update_joints(scene, dt);
        self.write_back_transforms(scene);
        self.report_ragdolls(scene);
        tick
    }

    fn write_back_transforms(&mut self, scene: &mut SceneWorld) {
        for (handle, entity) in &self.body_entities {
            let Some(body) = self.bodies.get(*handle) else {
                continue;
            };
            if !body.is_dynamic() {
                continue;
            }
            let pos = body.translation();
            if let Ok(mut transform) = scene.get::<&mut Transform>(*entity) {
                transform.position = Vec3::new(pos.x, pos.y, pos.z);
                let rot = body.rotation();
                transform.rotation = Quat::from_xyzw(rot.x, rot.y, rot.z, rot.w);
            }
        }
    }

    /// Whether `entity`'s body is asleep: at rest, skipped by the solver
    /// (and by water) until something touches or pushes it. False for
    /// entities without a body.
    #[must_use]
    pub fn is_sleeping(&self, entity: Entity) -> bool {
        self.entity_bodies
            .get(&entity)
            .and_then(|h| self.bodies.get(*h))
            .is_some_and(r3::dynamics::RigidBody::is_sleeping)
    }

    /// Current linear velocity of an entity's body (zero if none).
    #[must_use]
    pub fn velocity(&self, entity: Entity) -> Vec3 {
        self.entity_bodies
            .get(&entity)
            .and_then(|h| self.bodies.get(*h))
            .map_or(Vec3::ZERO, |b| {
                Vec3::new(b.linvel().x, b.linvel().y, b.linvel().z)
            })
    }

    /// Apply an impulse to a dynamic body's centre of mass.
    pub fn apply_impulse(&mut self, entity: Entity, impulse: Vec3) {
        if let Some(&handle) = self.entity_bodies.get(&entity) {
            if let Some(body) = self.bodies.get_mut(handle) {
                body.apply_impulse(Vector::new(impulse.x, impulse.y, impulse.z), true);
            }
        }
    }

    /// Apply an impulse at a world `point` (off-centre pushes also spin
    /// the body: shoving a door's free edge, kicking a crate's corner).
    pub fn apply_impulse_at(&mut self, entity: Entity, impulse: Vec3, point: Vec3) {
        if let Some(&handle) = self.entity_bodies.get(&entity) {
            if let Some(body) = self.bodies.get_mut(handle) {
                body.apply_impulse_at_point(impulse, point, true);
            }
        }
    }

    /// Current angular velocity of an entity's body in radians per second
    /// around each world axis (zero if none).
    #[must_use]
    pub fn angular_velocity(&self, entity: Entity) -> Vec3 {
        self.entity_bodies
            .get(&entity)
            .and_then(|h| self.bodies.get(*h))
            .map_or(Vec3::ZERO, rapier3d::dynamics::RigidBody::angvel)
    }

    /// Apply a force to a dynamic body (cleared each step).
    pub fn apply_force(&mut self, entity: Entity, force: Vec3) {
        if let Some(&handle) = self.entity_bodies.get(&entity) {
            if let Some(body) = self.bodies.get_mut(handle) {
                body.add_force(Vector::new(force.x, force.y, force.z), true);
            }
        }
    }

    /// Set a kinematic-velocity body's velocity.
    pub fn set_velocity(&mut self, entity: Entity, velocity: Vec3) {
        if let Some(&handle) = self.entity_bodies.get(&entity) {
            if let Some(body) = self.bodies.get_mut(handle) {
                body.set_linvel(Vector::new(velocity.x, velocity.y, velocity.z), true);
            }
        }
    }

    /// True if the entity's character controller was grounded after the last
    /// [`move_characters`](Self::move_characters) call.
    #[must_use]
    pub fn grounded(&self, entity: Entity) -> bool {
        self.grounded.get(&entity).copied().unwrap_or(false)
    }

    /// The world-space triangles of every solid collider that doesn't move
    /// on its own: colliders without a body and on fixed bodies, sensors
    /// left out, as `(vertices, triangles)`. What a navigation mesh is
    /// baked from; `skip` leaves out more entities (the agents
    /// themselves). Balls and capsules become coarse triangle shells.
    #[must_use]
    pub fn static_geometry(&self, skip: &dyn Fn(Entity) -> bool) -> (Vec<Vec3>, Vec<[u32; 3]>) {
        let mut vertices = Vec::new();
        let mut triangles = Vec::new();
        for (_, c) in self.colliders.iter() {
            if !c.is_enabled() || c.is_sensor() {
                continue;
            }
            let fixed = c
                .parent()
                .and_then(|h| self.bodies.get(h))
                .is_none_or(r3::dynamics::RigidBody::is_fixed);
            if !fixed || user_data_to_entity(c.user_data).is_some_and(skip) {
                continue;
            }
            shape_triangles(c.shape(), c.position(), &mut vertices, &mut triangles);
        }
        (vertices, triangles)
    }

    /// Cast a ray and return the closest hit, if any.
    #[must_use]
    pub fn cast_ray(
        &self,
        origin: Vec3,
        direction: Vec3,
        max_toi: f32,
        filter: &RayFilter,
    ) -> Option<RayHit> {
        let mut query_filter = QueryFilter::default();
        if filter.exclude_sensors {
            query_filter.flags |= QueryFilterFlags::EXCLUDE_SENSORS;
        }
        let predicate = |handle, _: &_| filter.exclude_entity != self.entity_for_collider(handle);
        query_filter = query_filter.predicate(&predicate);
        let query = self.broad_phase.as_query_pipeline(
            self.narrow_phase.query_dispatcher(),
            &self.bodies,
            &self.colliders,
            query_filter,
        );
        let ray = r3::geometry::Ray::new(
            Vector::new(origin.x, origin.y, origin.z),
            Vector::new(direction.x, direction.y, direction.z),
        );
        query
            .cast_ray_and_get_normal(&ray, max_toi, filter.solid)
            .and_then(|(handle, hit)| {
                let p = ray.point_at(hit.time_of_impact);
                Some(RayHit {
                    entity: self.entity_for_collider(handle)?,
                    point: Vec3::new(p.x, p.y, p.z),
                    normal: Vec3::new(hit.normal.x, hit.normal.y, hit.normal.z),
                    distance: hit.time_of_impact,
                })
            })
    }

    /// Every enabled collider's shape and world pose, for answering raycasts
    /// later without the world (see [`ColliderSnapshot`]).
    #[must_use]
    pub fn collider_snapshot(&self) -> ColliderSnapshot {
        let colliders = self
            .colliders
            .iter()
            .filter(|(_, c)| c.is_enabled())
            .filter_map(|(_, c)| {
                Some(SnapshotCollider {
                    entity: user_data_to_entity(c.user_data)?,
                    shape: c.shared_shape().clone(),
                    pose: *c.position(),
                    sensor: c.is_sensor(),
                })
            })
            .collect();
        ColliderSnapshot {
            colliders,
            tree: std::sync::OnceLock::new(),
        }
    }

    /// Shape-cast: sweep a shape from `origin` along `direction` and report
    /// the first impact.
    #[must_use]
    pub fn cast_shape(
        &self,
        shape: Shape3D,
        origin: Vec3,
        direction: Vec3,
        max_toi: f32,
        filter: &RayFilter,
    ) -> Option<RayHit> {
        let mut query_filter = QueryFilter::default();
        if filter.exclude_sensors {
            query_filter.flags |= QueryFilterFlags::EXCLUDE_SENSORS;
        }
        let query = self.broad_phase.as_query_pipeline(
            self.narrow_phase.query_dispatcher(),
            &self.bodies,
            &self.colliders,
            query_filter,
        );
        let shape: r3::geometry::SharedShape = match shape {
            Shape3D::Ball { radius } => r3::geometry::SharedShape::ball(radius),
            Shape3D::Cuboid { extents } => {
                r3::geometry::SharedShape::cuboid(extents.x / 2.0, extents.y / 2.0, extents.z / 2.0)
            }
            Shape3D::Capsule {
                half_height,
                radius,
            } => r3::geometry::SharedShape::capsule_y(half_height, radius),
            // Sweeping a triangle soup is ill-defined; sweep its hull.
            Shape3D::Mesh { model } | Shape3D::ConvexHull { model } => {
                let model = automata_assets::load(&model).ok()?;
                build_model_shape(&model, Vec3::ONE, true)?
            }
        };
        let pos = Pose::from_translation(Vector::new(origin.x, origin.y, origin.z));
        let vel = Vector::new(direction.x, direction.y, direction.z);
        query
            .cast_shape(
                &pos,
                vel,
                &*shape,
                r3::parry::query::ShapeCastOptions {
                    max_time_of_impact: max_toi,
                    target_distance: 0.0,
                    stop_at_penetration: false,
                    compute_impact_geometry_on_penetration: true,
                },
            )
            .and_then(|(handle, toi)| {
                Some(RayHit {
                    entity: self.entity_for_collider(handle)?,
                    point: Vec3::new(toi.witness1.x, toi.witness1.y, toi.witness1.z),
                    normal: Vec3::new(toi.normal1.x, toi.normal1.y, toi.normal1.z),
                    distance: toi.time_of_impact,
                })
            })
    }
}

fn controller_from_config(config: &CharacterController) -> KinematicCharacterController {
    KinematicCharacterController {
        up: Vector::Y,
        offset: CharacterLength::Absolute(config.offset),
        slide: config.slide,
        autostep: config.autostep_height.map(|h| CharacterAutostep {
            max_height: CharacterLength::Absolute(h),
            min_width: CharacterLength::Absolute(0.1),
            include_dynamic_bodies: true,
        }),
        max_slope_climb_angle: config.max_slope_climb_angle,
        min_slope_slide_angle: config.min_slope_slide_angle,
        snap_to_ground: config.snap_to_ground.map(CharacterLength::Absolute),
        normal_nudge_factor: 0.01,
    }
}

/// One collider in a [`ColliderSnapshot`].
#[derive(Clone)]
struct SnapshotCollider {
    entity: Entity,
    shape: r3::geometry::SharedShape,
    pose: Pose,
    sensor: bool,
}

/// The colliders of a [`PhysicsWorld3D`] frozen at one moment: their
/// shapes and world poses. Raycasts against it answer immediately, so
/// game code can ask "what's under the crosshair?" in the middle of a
/// frame without touching the live world.
#[derive(Clone, Default)]
pub struct ColliderSnapshot {
    colliders: Vec<SnapshotCollider>,
    /// A bounding-volume tree over `colliders`, built by the first ray cast
    /// once there are enough colliders for it to pay off (scripts in big
    /// scenes cast thousands of rays a frame).
    tree: std::sync::OnceLock<r3::parry::partitioning::Bvh>,
}

/// Colliders below this count are simply all tested.
const SNAPSHOT_TREE_FROM: usize = 32;

impl std::fmt::Debug for ColliderSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ColliderSnapshot")
            .field("colliders", &self.colliders.len())
            .field("tree", &self.tree.get().is_some())
            .finish()
    }
}

/// A collider's shape in world space, as plain numbers (for tools and
/// script runners that do their own ray tests).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WorldShape {
    /// A box: centre, rotation, half extents.
    Cuboid {
        /// World-space centre.
        center: Vec3,
        /// World rotation.
        rotation: Quat,
        /// Half the size along each local axis.
        half_extents: Vec3,
    },
    /// A sphere.
    Ball {
        /// World-space centre.
        center: Vec3,
        /// Radius.
        radius: f32,
    },
    /// A capsule along its local Y axis.
    Capsule {
        /// World-space centre.
        center: Vec3,
        /// World rotation.
        rotation: Quat,
        /// Half the length of the straight part.
        half_height: f32,
        /// Radius.
        radius: f32,
    },
    /// Triangles (a mesh collider, or a convex hull's faces), in the
    /// collider's local space; fetch them once per `mesh` key with
    /// [`ColliderSnapshot::mesh_triangles`].
    Mesh {
        /// World-space origin of the triangles.
        center: Vec3,
        /// World rotation.
        rotation: Quat,
        /// Identifies the shared geometry: colliders built from the same
        /// model at the same scale share a key.
        mesh: u64,
    },
}

/// Appends `shape`'s surface at `pose` as world-space triangles (see
/// [`PhysicsWorld3D::static_geometry`]).
fn shape_triangles(
    shape: &dyn r3::geometry::Shape,
    pose: &Pose,
    vertices: &mut Vec<Vec3>,
    triangles: &mut Vec<[u32; 3]>,
) {
    let (local, tris) = if let Some(compound) = shape.as_compound() {
        for (sub, part) in compound.shapes() {
            shape_triangles(part.as_ref(), &(*pose * *sub), vertices, triangles);
        }
        return;
    } else if let Some(b) = shape.as_cuboid() {
        b.to_trimesh()
    } else if let Some(b) = shape.as_round_cuboid() {
        r3::geometry::Cuboid::new(b.inner_shape.half_extents + Vec3::splat(b.border_radius))
            .to_trimesh()
    } else if let Some(t) = shape.as_trimesh() {
        (t.vertices().to_vec(), t.indices().to_vec())
    } else if let Some(h) = shape.as_convex_polyhedron() {
        h.to_trimesh()
    } else if let Some(h) = shape.as_heightfield() {
        h.to_trimesh()
    } else if let Some(b) = shape.as_ball() {
        b.to_trimesh(12, 6)
    } else if let Some(c) = shape.as_capsule() {
        c.to_trimesh(12, 4)
    } else if let Some(c) = shape.as_cylinder() {
        c.to_trimesh(12)
    } else if let Some(c) = shape.as_cone() {
        c.to_trimesh(12)
    } else {
        return;
    };
    let Ok(base) = u32::try_from(vertices.len()) else {
        return;
    };
    vertices.extend(local.into_iter().map(|v| pose.transform_point(v)));
    triangles.extend(tris.into_iter().map(|t| t.map(|i| i + base)));
}

/// A heightfield's triangles in its local space, vertices row by row
/// (rows along Z, columns along X), each cell split the way parry splits
/// it (see [`ColliderSnapshot::mesh_grid`]).
fn grid_triangles(h: &r3::parry::shape::HeightField) -> (Vec<Vec3>, Vec<[u32; 3]>) {
    let heights = h.heights();
    let (rows, cols) = (heights.nrows(), heights.ncols());
    let scale = h.scale();
    let step = |n: usize| 1.0 / (n.max(2) - 1) as f32;
    let (sx, sz) = (step(cols), step(rows));
    let mut vertices = Vec::with_capacity(rows * cols);
    for z in 0..rows {
        for x in 0..cols {
            vertices.push(Vec3::new(
                (x as f32 * sx - 0.5) * scale.x,
                heights[(z, x)] * scale.y,
                (z as f32 * sz - 0.5) * scale.z,
            ));
        }
    }
    let at = |x: usize, z: usize| u32::try_from(z * cols + x).unwrap_or(u32::MAX);
    let mut triangles = Vec::with_capacity(2 * rows.saturating_sub(1) * cols.saturating_sub(1));
    for z in 0..rows.saturating_sub(1) {
        for x in 0..cols.saturating_sub(1) {
            let (p00, p01, p10, p11) = (at(x, z), at(x, z + 1), at(x + 1, z), at(x + 1, z + 1));
            triangles.push([p00, p01, p11]);
            triangles.push([p00, p11, p10]);
        }
    }
    (vertices, triangles)
}

/// The key of a shape's shared geometry (its allocation address, stable
/// for as long as any collider holds the shape).
fn shape_key(shape: &r3::geometry::SharedShape) -> u64 {
    std::sync::Arc::as_ptr(&shape.0).cast::<u8>() as usize as u64
}

impl ColliderSnapshot {
    /// The closest hit along a ray (`direction` need not be normalized),
    /// honouring the filter's excluded entity and sensor setting.
    #[must_use]
    pub fn cast_ray(
        &self,
        origin: Vec3,
        direction: Vec3,
        max_distance: f32,
        filter: &RayFilter,
    ) -> Option<RayHit> {
        let len = direction.length();
        if len <= f32::EPSILON || !len.is_finite() {
            return None;
        }
        let dir = direction / len;
        let ray = r3::geometry::Ray::new(origin, dir);
        let skip = |c: &SnapshotCollider| {
            (filter.exclude_sensors && c.sensor) || filter.exclude_entity == Some(c.entity)
        };
        let hit_at = |c: &SnapshotCollider, limit: f32| {
            c.shape
                .cast_ray_and_get_normal(&c.pose, &ray, limit, filter.solid)
                .map(|hit| RayHit {
                    entity: c.entity,
                    point: origin + dir * hit.time_of_impact,
                    normal: hit.normal,
                    distance: hit.time_of_impact,
                })
        };
        if self.colliders.len() >= SNAPSHOT_TREE_FROM {
            let tree = self.tree.get_or_init(|| {
                let leaves: Vec<_> = self
                    .colliders
                    .iter()
                    .map(|c| c.shape.compute_aabb(&c.pose))
                    .collect();
                r3::parry::partitioning::Bvh::from_leaves(
                    r3::parry::partitioning::BvhBuildStrategy::default(),
                    &leaves,
                )
            });
            let (index, toi) = tree.cast_ray(&ray, max_distance, |i, limit| {
                let c = &self.colliders[i as usize];
                if skip(c) {
                    return None;
                }
                c.shape.cast_ray(&c.pose, &ray, limit, filter.solid)
            })?;
            // The tree reports the distance; ask the winner for its normal.
            let c = &self.colliders[index as usize];
            return hit_at(c, toi + 1e-4);
        }
        let mut best: Option<RayHit> = None;
        for c in &self.colliders {
            if skip(c) {
                continue;
            }
            let limit = best.as_ref().map_or(max_distance, |b| b.distance);
            if let Some(hit) = hit_at(c, limit) {
                best = Some(hit);
            }
        }
        best
    }

    /// The triangles behind a [`WorldShape::Mesh`] key, in the collider's
    /// local space: `(vertices, triangles)`.
    #[must_use]
    pub fn mesh_triangles(&self, mesh: u64) -> Option<(Vec<Vec3>, Vec<[u32; 3]>)> {
        let shape = &self
            .colliders
            .iter()
            .find(|c| shape_key(&c.shape) == mesh)?
            .shape;
        if let Some(t) = shape.as_trimesh() {
            Some((t.vertices().to_vec(), t.indices().to_vec()))
        } else if let Some(h) = shape.as_heightfield() {
            Some(grid_triangles(h))
        } else {
            Some(shape.as_convex_polyhedron()?.to_trimesh())
        }
    }

    /// For a [`WorldShape::Mesh`] that is a heightfield (a terrain): its
    /// grid's `[columns, rows]`. Its [`Self::mesh_triangles`] vertices are
    /// then laid out row by row (rows along Z, columns along X), so ray
    /// tests can walk the grid instead of testing every triangle.
    #[must_use]
    pub fn mesh_grid(&self, mesh: u64) -> Option<[u32; 2]> {
        let h = self
            .colliders
            .iter()
            .find(|c| shape_key(&c.shape) == mesh)?
            .shape
            .as_heightfield()?;
        let heights = h.heights();
        Some([
            u32::try_from(heights.ncols()).ok()?,
            u32::try_from(heights.nrows()).ok()?,
        ])
    }

    /// Every collider as `(entity, world shape, sensor)`. Shapes other
    /// than boxes, balls, capsules, meshes, convex hulls and heightfields
    /// (terrains, as meshes) are skipped.
    #[must_use]
    pub fn shapes(&self) -> Vec<(Entity, WorldShape, bool)> {
        self.colliders
            .iter()
            .filter_map(|c| {
                let center = c.pose.translation;
                let rotation = c.pose.rotation;
                let shape = if let Some(b) = c.shape.as_cuboid() {
                    WorldShape::Cuboid {
                        center,
                        rotation,
                        half_extents: b.half_extents,
                    }
                } else if let Some(b) = c.shape.as_ball() {
                    WorldShape::Ball {
                        center,
                        radius: b.radius,
                    }
                } else if let Some(cap) = c.shape.as_capsule() {
                    WorldShape::Capsule {
                        center,
                        rotation,
                        half_height: cap.half_height(),
                        radius: cap.radius,
                    }
                } else if c.shape.as_trimesh().is_some()
                    || c.shape.as_convex_polyhedron().is_some()
                    || c.shape.as_heightfield().is_some()
                {
                    WorldShape::Mesh {
                        center,
                        rotation,
                        mesh: shape_key(&c.shape),
                    }
                } else {
                    return None;
                };
                Some((c.entity, shape, c.sensor))
            })
            .collect()
    }

    /// Number of colliders.
    #[must_use]
    pub fn len(&self) -> usize {
        self.colliders.len()
    }

    /// Whether there are no colliders.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.colliders.is_empty()
    }
}
